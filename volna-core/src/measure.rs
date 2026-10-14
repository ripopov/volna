//! Measuring between two times in time and in each clock's cycles.

use std::fmt;

use vtr::ClockTimeline;

use crate::clock::Clock;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cycles {
    Whole(i64),
    Fraction(f64),
    Loading,
}

impl fmt::Display for Cycles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = |negative: bool| if negative { "−" } else { "" };
        match *self {
            Self::Whole(n) => write!(f, "{}{}", sign(n < 0), n.unsigned_abs()),
            Self::Fraction(x) => {
                let x = (x * 10.0).round() / 10.0;
                write!(f, "{}{:.1}", sign(x < 0.0), x.abs())
            }
            Self::Loading => f.write_str("…"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub edges: u64,
    pub fraction: f64,
}

#[must_use]
pub fn position(timeline: &ClockTimeline, t: u64) -> Position {
    match timeline.cycle_at(t) {
        None => Position {
            edges: 0,
            fraction: 0.0,
        },
        Some(at) => Position {
            edges: at.cycle + 1,
            fraction: if at.stopped { 0.0 } else { at.fraction },
        },
    }
}

#[must_use]
pub fn cycles(timeline: &ClockTimeline, from: u64, to: u64) -> Cycles {
    let (a, b) = (position(timeline, from), position(timeline, to));
    let whole = b.edges as i64 - a.edges as i64;
    if a.fraction == 0.0 && b.fraction == 0.0 {
        Cycles::Whole(whole)
    } else {
        Cycles::Fraction(whole as f64 + (b.fraction - a.fraction))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClockCount {
    pub name: String,
    pub cycles: Cycles,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    pub from: u64,
    pub to: u64,
    pub clocks: Vec<ClockCount>,
}

impl Measurement {
    #[must_use]
    pub fn dt(&self) -> i128 {
        i128::from(self.to) - i128::from(self.from)
    }
}

#[must_use]
pub fn measure(clocks: &[&Clock], from: u64, to: u64) -> Measurement {
    Measurement {
        from,
        to,
        clocks: clocks
            .iter()
            .map(|clock| ClockCount {
                name: clock.name.clone(),
                cycles: clock
                    .timeline()
                    .map_or(Cycles::Loading, |t| cycles(t, from, to)),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline(stretches: &[(u64, u64, u64)], open: bool) -> ClockTimeline {
        ClockTimeline::new(stretches.to_vec(), open).unwrap()
    }

    fn edges(t: &ClockTimeline) -> Vec<u64> {
        t.stretches()
            .iter()
            .flat_map(|s| {
                let step = s.period.max(1);
                (s.begin..=s.end).step_by(step as usize)
            })
            .collect()
    }

    #[test]
    fn counting_follows_a_frequency_change() {
        let t = timeline(&[(0, 90, 10), (100, 200, 4)], false);
        assert_eq!(cycles(&t, 30, 104), Cycles::Whole(8));
        assert_eq!(cycles(&t, 35, 106), Cycles::Fraction(8.0));
        assert_eq!(cycles(&t, 35, 108), Cycles::Fraction(8.5));
        assert_eq!(cycles(&t, 108, 35), Cycles::Fraction(-8.5));
        assert_ne!(73.0 / 10.0, 8.5);
        assert_ne!(73.0 / 4.0, 8.5);
    }

    #[test]
    fn a_stopped_clock_adds_nothing_and_before_the_first_edge_counts_zero() {
        let t = timeline(&[(10, 50, 10), (1000, 1040, 10)], false);
        assert!(t.cycle_at(500).unwrap().stopped);
        assert_eq!(cycles(&t, 50, 1000), Cycles::Whole(1));
        assert_eq!(cycles(&t, 45, 500), Cycles::Fraction(0.5));
        assert_eq!(cycles(&t, 500, 700), Cycles::Whole(0));
        assert_eq!(cycles(&t, 0, 5), Cycles::Whole(0));
        assert_eq!(cycles(&t, 0, 30), Cycles::Whole(3));
        let one = timeline(&[(100, 100, 0)], false);
        assert_eq!(cycles(&one, 0, 200), Cycles::Whole(1));
        assert_eq!(cycles(&one, 100, 100), Cycles::Whole(0));
    }

    #[test]
    fn counts_equal_brute_force_edge_counting_on_random_timelines() {
        let mut seed = 11u64;
        let mut rnd = |n: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % n.max(1)
        };
        for trial in 0..500 {
            let mut raw = Vec::new();
            let mut at = rnd(50);
            for _ in 0..=rnd(5) {
                let period = [0, 1 + rnd(20)][usize::from(rnd(5) != 0)];
                let n = if period == 0 { 0 } else { rnd(30) };
                raw.push((at, at + n * period, period));
                at += n * period + 1 + [rnd(5), 50 + rnd(500)][rnd(2) as usize];
            }
            let t = timeline(&raw, rnd(2) == 0);
            let all = edges(&t);
            let end = at + 50;
            for _ in 0..40 {
                let (a, b) = (rnd(end), rnd(end));
                let (lo, hi) = (a.min(b), a.max(b));
                let between = all.iter().filter(|&&e| e > lo && e <= hi).count() as i64;
                let signed = if b >= a { between } else { -between };
                let on_edge = |x: u64| all.contains(&x) || position(&t, x).fraction == 0.0;
                match cycles(&t, a, b) {
                    Cycles::Whole(n) => {
                        assert_eq!(n, signed, "trial {trial}: {a}..{b} {raw:?}");
                    }
                    Cycles::Fraction(x) => {
                        assert!(!(on_edge(a) && on_edge(b)), "trial {trial}");
                        assert!(
                            (x - signed as f64).abs() < 1.0,
                            "trial {trial}: {a}..{b} {x} vs {signed} {raw:?}"
                        );
                    }
                    Cycles::Loading => unreachable!(),
                }
                let (pa, pb) = (position(&t, lo), position(&t, hi));
                assert_eq!((pb.edges - pa.edges) as i64, between, "trial {trial}");
            }
        }
    }

    #[test]
    fn counts_print_signed_with_one_decimal_mid_cycle() {
        assert_eq!(Cycles::Whole(80).to_string(), "80");
        assert_eq!(Cycles::Whole(-3).to_string(), "−3");
        assert_eq!(Cycles::Fraction(26.44).to_string(), "26.4");
        assert_eq!(Cycles::Fraction(-0.46).to_string(), "−0.5");
        assert_eq!(Cycles::Fraction(-0.04).to_string(), "0.0");
        assert_eq!(Cycles::Loading.to_string(), "…");
    }
}
