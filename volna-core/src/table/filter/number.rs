//! Decimal comparisons keep integer and fixed-point trace values exact.

use crate::trace::Unit;
use anyhow::{Result, bail, ensure};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug)]
pub(super) struct Number {
    coefficient: i128,
    exponent: i32,
}

impl Number {
    pub fn new(coefficient: i128, exponent: i32) -> Self {
        Self {
            coefficient,
            exponent,
        }
    }

    fn parse(input: &str, unit: Option<&Unit>) -> Result<Self> {
        let input = input.trim();
        ensure!(!input.is_empty(), "Enter a number after the operator");
        let units = [
            ("ticks", None),
            ("tick", None),
            ("fs", Some(-15)),
            ("ps", Some(-12)),
            ("ns", Some(-9)),
            ("us", Some(-6)),
            ("µs", Some(-6)),
            ("ms", Some(-3)),
            ("s", Some(0)),
        ];
        let mut number = input;
        let mut exponent = 0i32;
        for (suffix, scale) in units {
            if let Some(value) = input.strip_suffix(suffix) {
                let base =
                    unit.ok_or_else(|| anyhow::anyhow!("Time units require a time column"))?;
                ensure!(
                    scale.is_none() || base.name.is_none(),
                    "Recording uses named time units; use raw ticks"
                );
                number = value.trim();
                exponent = scale.map_or(0, |scale| scale - i32::from(base.timescale));
                break;
            }
        }
        let (negative, unsigned) = number
            .strip_prefix('-')
            .map_or((false, number), |n| (true, n));
        if let Some(hex) = unsigned
            .strip_prefix("0x")
            .or_else(|| unsigned.strip_prefix("0X"))
        {
            let value = i128::from_str_radix(hex, 16)?;
            return Ok(Self::new(if negative { -value } else { value }, exponent));
        }
        let (number, power) = number
            .split_once(['e', 'E'])
            .map_or(Ok((number, 0)), |(n, p)| {
                Ok::<_, anyhow::Error>((n, p.parse::<i32>()?))
            })?;
        exponent = exponent
            .checked_add(power)
            .ok_or_else(|| anyhow::anyhow!("Exponent is too large"))?;
        let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
        ensure!(
            fraction.bytes().all(|b| b.is_ascii_digit()),
            "Invalid decimal fraction"
        );
        let coefficient = format!("{whole}{fraction}").parse::<i128>()?;
        exponent = exponent
            .checked_sub(i32::try_from(fraction.len())?)
            .ok_or_else(|| anyhow::anyhow!("Exponent is too small"))?;
        Ok(Self::new(coefficient, exponent))
    }

    fn compare(self, other: Self) -> Ordering {
        if self.exponent == other.exponent {
            return self.coefficient.cmp(&other.coefficient);
        }
        let sign = self.coefficient.signum().cmp(&other.coefficient.signum());
        if sign != Ordering::Equal || self.coefficient == 0 {
            return sign;
        }
        let (mut a, mut b) = (
            self.coefficient.unsigned_abs(),
            other.coefficient.unsigned_abs(),
        );
        let (da, db) = (a.ilog10(), b.ilog10());
        let magnitude = (i64::from(self.exponent) + i64::from(da))
            .cmp(&(i64::from(other.exponent) + i64::from(db)));
        let order = if magnitude == Ordering::Equal {
            let (mut pa, mut pb) = (10u128.pow(da), 10u128.pow(db));
            loop {
                let order = (a / pa).cmp(&(b / pb));
                if order != Ordering::Equal || (pa == 1 && pb == 1) {
                    break order;
                }
                a %= pa;
                b %= pb;
                if pa > 1 {
                    pa /= 10;
                } else {
                    a *= 10;
                }
                if pb > 1 {
                    pb /= 10;
                } else {
                    b *= 10;
                }
            }
        } else {
            magnitude
        };
        if self.coefficient < 0 {
            order.reverse()
        } else {
            order
        }
    }

    fn real(self) -> f64 {
        if self.coefficient == 0 {
            0.0
        } else {
            self.coefficient as f64 * 10f64.powi(self.exponent)
        }
    }
}

#[derive(Clone, Copy)]
enum Operator {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
impl Operator {
    fn matches(self, ordering: Ordering) -> bool {
        match self {
            Self::Eq => ordering == Ordering::Equal,
            Self::Ne => ordering != Ordering::Equal,
            Self::Lt => ordering == Ordering::Less,
            Self::Le => ordering != Ordering::Greater,
            Self::Gt => ordering == Ordering::Greater,
            Self::Ge => ordering != Ordering::Less,
        }
    }
}

struct Term {
    operator: Operator,
    number: Number,
    real: f64,
}
impl Term {
    fn new(operator: Operator, number: Number) -> Self {
        Self {
            operator,
            number,
            real: number.real(),
        }
    }
}

pub(super) struct Expression(Vec<Vec<Term>>);
impl Expression {
    pub fn compile(input: &str, unit: Option<&Unit>) -> Result<Self> {
        let or = regex::Regex::new(r"(?i)\s+OR\s+|\|\|")?;
        let and = regex::Regex::new(r"(?i)\s+AND\s+|&&")?;
        let mut groups = Vec::new();
        for group in or.split(input) {
            let mut terms = Vec::new();
            for term in and.split(group) {
                let term = term.trim();
                if let Some((low, high)) = term.split_once("..") {
                    let units = ["ticks", "tick", "fs", "ps", "ns", "us", "µs", "ms", "s"];
                    let shared_unit = units.iter().find(|&&unit| high.trim().ends_with(unit));
                    let low = if units.iter().any(|unit| low.trim().ends_with(unit)) {
                        low.to_owned()
                    } else if let Some(unit) = shared_unit {
                        format!("{low} {unit}")
                    } else {
                        low.to_owned()
                    };
                    let low = Number::parse(&low, unit)?;
                    let high = Number::parse(high, unit)?;
                    ensure!(
                        low.compare(high) != Ordering::Greater,
                        "Range start exceeds its end"
                    );
                    terms.extend([Term::new(Operator::Ge, low), Term::new(Operator::Le, high)]);
                    continue;
                }
                let (operator, value) = [
                    (">=", Operator::Ge),
                    ("<=", Operator::Le),
                    ("!=", Operator::Ne),
                    ("==", Operator::Eq),
                    (">", Operator::Gt),
                    ("<", Operator::Lt),
                    ("=", Operator::Eq),
                ]
                .into_iter()
                .find_map(|(prefix, op)| term.strip_prefix(prefix).map(|value| (op, value)))
                .unwrap_or((Operator::Eq, term));
                if value.trim().is_empty() {
                    bail!("Enter a number after the operator");
                }
                terms.push(Term::new(operator, Number::parse(value, unit)?));
            }
            groups.push(terms);
        }
        Ok(Self(groups))
    }
    pub fn matches(&self, value: Number) -> bool {
        self.0.iter().any(|terms| {
            terms
                .iter()
                .all(|term| term.operator.matches(value.compare(term.number)))
        })
    }
    pub fn matches_real(&self, value: f64) -> bool {
        value.is_finite()
            && self.0.iter().any(|terms| {
                terms.iter().all(|term| {
                    value
                        .partial_cmp(&term.real)
                        .is_some_and(|order| term.operator.matches(order))
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_time_units_accept_ticks_without_assuming_seconds() {
        let cycles = Unit {
            timescale: 0,
            name: Some("cycles".into()),
        };
        assert!(
            Expression::compile("> 100 ticks", Some(&cycles))
                .unwrap()
                .matches(Number::new(101, 0))
        );
        assert!(
            Expression::compile("100..500 ticks", Some(&cycles))
                .unwrap()
                .matches(Number::new(100, 0))
        );
        assert!(Expression::compile("= 100 ns", Some(&cycles)).is_err());
    }

    #[test]
    fn expressions_preserve_integer_precision_and_time_units() {
        let e = Expression::compile("> 9007199254740992 AND <= 0x20000000000001 OR -4..-2", None)
            .unwrap();
        assert!(e.matches(Number::new(9_007_199_254_740_993, 0)));
        assert!(!e.matches(Number::new(9_007_199_254_740_992, 0)));
        assert!(e.matches(Number::new(-3, 0)));
        assert!(
            Expression::compile(
                "= 0.1 us",
                Some(&Unit {
                    timescale: -9,
                    name: None
                })
            )
            .unwrap()
            .matches(Number::new(100, 0))
        );
        assert!(
            Expression::compile("1e-3..2", None)
                .unwrap()
                .matches(Number::new(1, -2))
        );
        assert_eq!(
            Number::new(3, 1).compare(Number::new(30, 0)),
            Ordering::Equal
        );
        assert_eq!(
            Number::new(i128::MAX, i32::MAX).compare(Number::new(99, i32::MAX)),
            Ordering::Greater
        );
        assert!(
            Expression::compile(
                "100..500 ns",
                Some(&Unit {
                    timescale: -12,
                    name: None
                })
            )
            .unwrap()
            .matches(Number::new(100_000, 0))
        );
        assert!(
            !Expression::compile(
                "100..500 ns",
                Some(&Unit {
                    timescale: -12,
                    name: None
                })
            )
            .unwrap()
            .matches(Number::new(99_999, 0))
        );
        assert!(Expression::compile("> 2 ns", None).is_err());
        assert!(Expression::compile("2..1", None).is_err());
        assert!(Expression::compile(">=", None).is_err());
        assert!(Expression::compile("1 AND", None).is_err());
    }
}
