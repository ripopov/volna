//! Compiled column predicates over borrowed raw values, independent of cell formatting.

pub(in crate::table) mod index;
mod number;

use super::{
    columns::{CellValue, TransactionColumn, attributes},
    sort::Source,
};
use anyhow::{Context, Result, ensure};
use number::{Expression, Number};
use regex::{Regex, RegexBuilder};
use std::fmt::Write;
use volna_trace::data::{transactions::AttributeValue, value_view::ValueView};

pub const MAX_QUERY_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterMode {
    #[default]
    Contains,
    Exact,
    Wildcard,
    Fuzzy,
    Regex,
    Expression,
}
impl FilterMode {
    pub const ALL: [Self; 6] = [
        Self::Contains,
        Self::Exact,
        Self::Wildcard,
        Self::Fuzzy,
        Self::Regex,
        Self::Expression,
    ];
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Contains => "Contains",
            Self::Exact => "Exact",
            Self::Wildcard => "Wildcard",
            Self::Fuzzy => "Fuzzy subsequence",
            Self::Regex => "Regular expression",
            Self::Expression => "Numeric expression",
        }
    }
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Self::Contains => "Aa",
            Self::Exact => "=",
            Self::Wildcard => "*",
            Self::Fuzzy => "~",
            Self::Regex => ".*",
            Self::Expression => ">",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Missing {
    #[default]
    Exclude,
    Include,
    Only,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ColumnFilter {
    pub query: String,
    pub mode: FilterMode,
    pub case_sensitive: bool,
    pub missing: Missing,
}
impl ColumnFilter {
    #[must_use]
    pub fn active(&self) -> bool {
        !self.query.is_empty() || self.missing == Missing::Only
    }
}

pub(in crate::table) fn same_filters(
    a: &[Option<ColumnFilter>],
    b: &[Option<ColumnFilter>],
) -> bool {
    (0..a.len().max(b.len()))
        .all(|i| a.get(i).and_then(Option::as_ref) == b.get(i).and_then(Option::as_ref))
}

#[derive(Clone)]
pub(in crate::table) enum Column {
    Transaction(TransactionColumn),
    Signal { index: usize, name: String },
    Time,
}

impl Column {
    fn title(&self) -> &str {
        match self {
            Self::Transaction(column) => column.title(),
            Self::Signal { name, .. } => name,
            Self::Time => "Time",
        }
    }

    fn is_time(&self, source: &Source) -> bool {
        match self {
            Self::Time
            | Self::Transaction(
                TransactionColumn::Begin | TransactionColumn::End | TransactionColumn::Duration,
            ) => true,
            Self::Transaction(TransactionColumn::Attribute(key)) => match source {
                Source::Generator(generator, _) => {
                    generator.attribute_catalogue().iter().any(|attribute| {
                        &attribute.key == key
                            && attribute.value_type
                                == volna_trace::data::transactions::AttributeType::Time
                    })
                }
                Source::Signals { .. } => false,
            },
            _ => false,
        }
    }
}

struct Rule {
    column: Column,
    predicate: Predicate,
    sample_hint: Option<usize>,
}

pub(in crate::table) struct Program(Vec<Rule>);
impl Program {
    pub fn compile(
        filters: &[(Column, ColumnFilter)],
        unit: &crate::trace::Unit,
        source: &Source,
    ) -> Result<Self> {
        let mut predicates = filters
            .iter()
            .map(|(column, filter)| {
                Ok(Rule {
                    column: column.clone(),
                    predicate: Predicate::compile(filter, column.is_time(source).then_some(unit))
                        .with_context(|| format!("{} filter", column.title()))?,
                    sample_hint: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        predicates.sort_by_key(|rule| !matches!(rule.predicate.matcher, Matcher::Numeric(_)));
        Ok(Self(predicates))
    }

    pub fn matches(&mut self, source: &Source, row: u64, scratch: &mut String) -> bool {
        self.0.iter_mut().all(|rule| {
            let predicate = &rule.predicate;
            match (source, &rule.column) {
                (Source::Generator(g, _), Column::Transaction(column)) => {
                    let tx = &g.transactions()[row as usize];
                    match column.value(tx) {
                        CellValue::Missing => predicate.missing(),
                        CellValue::Number(value) | CellValue::Time(value) => {
                            predicate.integer(i128::from(value))
                        }
                        CellValue::Text(value) => predicate.text(value),
                        CellValue::Attribute(value) => predicate.attribute(value, scratch),
                        CellValue::Attributes(items) => {
                            let mut attributes = attributes(items).peekable();
                            if attributes.peek().is_none() {
                                predicate.missing()
                            } else if predicate.missing == Missing::Only {
                                false
                            } else {
                                attributes.any(|a| {
                                    predicate.text(&a.key)
                                        || predicate.present_attribute(&a.value, scratch)
                                })
                            }
                        }
                    }
                }
                (Source::Signals { histories, .. }, Column::Signal { index, .. }) => {
                    let history = &histories[*index];
                    let time = source.time(row);
                    let sample = history.index_at_hint(time, rule.sample_hint.unwrap_or(0));
                    rule.sample_hint = sample;
                    predicate.value(history.value_view(sample), scratch)
                }
                (Source::Signals { .. }, Column::Time) => {
                    predicate.integer(i128::from(source.time(row)))
                }
                _ => false,
            }
        })
    }
}

enum Matcher {
    Text(Regex),
    Fuzzy(Vec<char>, bool),
    Numeric(Expression),
    Any,
}
struct Predicate {
    matcher: Matcher,
    missing: Missing,
}
impl Predicate {
    fn compile(filter: &ColumnFilter, unit: Option<&crate::trace::Unit>) -> Result<Self> {
        ensure!(
            filter.query.len() <= MAX_QUERY_BYTES,
            "Filter exceeds {MAX_QUERY_BYTES} bytes"
        );
        let matcher = if filter.missing == Missing::Only || filter.query.is_empty() {
            Matcher::Any
        } else if filter.mode == FilterMode::Expression {
            Matcher::Numeric(Expression::compile(&filter.query, unit)?)
        } else if filter.mode == FilterMode::Fuzzy {
            Matcher::Fuzzy(
                if filter.case_sensitive {
                    filter.query.chars().collect()
                } else {
                    filter.query.chars().flat_map(char::to_lowercase).collect()
                },
                filter.case_sensitive,
            )
        } else {
            let escaped = regex::escape(&filter.query);
            let pattern = match filter.mode {
                FilterMode::Contains => escaped,
                FilterMode::Exact => format!("\\A{escaped}\\z"),
                FilterMode::Wildcard => wildcard(&filter.query)?,
                FilterMode::Regex => filter.query.clone(),
                _ => unreachable!(),
            };
            Matcher::Text(
                RegexBuilder::new(&pattern)
                    .case_insensitive(!filter.case_sensitive)
                    .size_limit(1024 * 1024)
                    .dfa_size_limit(256 * 1024)
                    .build()?,
            )
        };
        Ok(Self {
            matcher,
            missing: filter.missing,
        })
    }
    fn missing(&self) -> bool {
        self.missing != Missing::Exclude
    }
    fn text(&self, value: &str) -> bool {
        if self.missing == Missing::Only {
            return false;
        }
        match &self.matcher {
            Matcher::Text(regex) => regex.is_match(value),
            Matcher::Fuzzy(query, case_sensitive) => {
                fn subsequence(query: &[char], mut input: impl Iterator<Item = char>) -> bool {
                    query.iter().all(|&needle| input.any(|ch| ch == needle))
                }
                if *case_sensitive {
                    subsequence(query, value.chars())
                } else {
                    subsequence(query, value.chars().flat_map(char::to_lowercase))
                }
            }
            Matcher::Any => true,
            Matcher::Numeric(_) => false,
        }
    }
    fn integer(&self, value: i128) -> bool {
        self.number(value, 0)
    }
    fn number(&self, value: i128, scale: i64) -> bool {
        if self.missing == Missing::Only {
            return false;
        }
        match &self.matcher {
            Matcher::Numeric(e) => e.matches(Number::new(value, scale)),
            _ => self.text(&if scale == 0 {
                value.to_string()
            } else {
                format!("{value}e{scale}")
            }),
        }
    }
    fn real(&self, value: f64) -> bool {
        if self.missing == Missing::Only {
            return false;
        }
        match &self.matcher {
            Matcher::Numeric(e) => e.matches_real(value),
            _ => self.text(&value.to_string()),
        }
    }
    fn attribute(&self, value: &AttributeValue, scratch: &mut String) -> bool {
        if matches!(value, AttributeValue::Null) {
            return self.missing();
        }
        self.missing != Missing::Only && self.present_attribute(value, scratch)
    }
    fn present_attribute(&self, value: &AttributeValue, scratch: &mut String) -> bool {
        match value {
            AttributeValue::Null => false,
            AttributeValue::Text(v) => self.text(v),
            AttributeValue::Enum { value, name } => {
                if matches!(self.matcher, Matcher::Numeric(_)) {
                    self.integer(*value)
                } else {
                    self.text(name)
                }
            }
            AttributeValue::I64(v) => self.integer(i128::from(*v)),
            AttributeValue::U64(v) | AttributeValue::Time(v) | AttributeValue::Pointer(v) => {
                self.integer(i128::from(*v))
            }
            AttributeValue::Bool(v) if matches!(self.matcher, Matcher::Numeric(_)) => {
                self.integer(i128::from(*v))
            }
            AttributeValue::Bool(v) => self.text(if *v { "true" } else { "false" }),
            AttributeValue::F64(v) => self.real(*v),
            AttributeValue::Fixed { raw, scale } => self.number(i128::from(*raw), *scale),
            AttributeValue::UFixed { raw, scale } => self.number(i128::from(*raw), *scale),
            AttributeValue::Logic {
                width,
                states,
                data,
            } => self.value(
                ValueView::Logic(volna_trace::data::value_view::LogicView::packed_lsb(
                    *width, *states, data,
                )),
                scratch,
            ),
            AttributeValue::List(values) => {
                values.iter().any(|v| self.present_attribute(v, scratch))
            }
            AttributeValue::Map(values) => values
                .iter()
                .any(|(k, v)| self.text(k) || self.present_attribute(v, scratch)),
            AttributeValue::Bytes(bytes) => {
                scratch.clear();
                for byte in bytes {
                    write!(scratch, "{byte:02x}").expect("String write");
                }
                self.text(scratch)
            }
        }
    }
    fn value(&self, value: ValueView<'_>, scratch: &mut String) -> bool {
        if self.missing == Missing::Only {
            return matches!(value, ValueView::Unavailable);
        }
        match value {
            ValueView::Unavailable => self.missing(),
            ValueView::Text(v) => self.text(v.as_ref()),
            ValueView::Real(v) => self.real(v),
            ValueView::Bytes(v) => {
                scratch.clear();
                for byte in v.as_ref() {
                    write!(scratch, "{byte:02x}").expect("String write");
                }
                self.text(scratch)
            }
            ValueView::Logic(logic) => {
                let mut numeric = Some(0u64);
                scratch.clear();
                for i in 0..logic.width {
                    let bit = logic.bit(i);
                    let ch = char::from(bit);
                    if matches!(self.matcher, Matcher::Numeric(_)) {
                        numeric = numeric
                            .and_then(|n| n.checked_mul(2)?.checked_add(u64::from(ch == '1')))
                            .filter(|_| ch == '0' || ch == '1');
                    } else {
                        scratch.push(ch);
                    }
                }
                if matches!(self.matcher, Matcher::Numeric(_)) {
                    numeric.is_some_and(|n| self.integer(i128::from(n)))
                } else {
                    self.text(scratch)
                }
            }
        }
    }
}

fn wildcard(query: &str) -> Result<String> {
    let mut pattern = String::from("\\A(?s:");
    let mut chars = query.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => pattern.push_str(".*"),
            '?' => pattern.push('.'),
            '\\' => {
                let ch = chars
                    .next()
                    .context("A wildcard escape needs a following character")?;
                pattern.push_str(&regex::escape(&ch.to_string()));
            }
            _ => pattern.push_str(&regex::escape(&ch.to_string())),
        }
    }
    pattern.push_str(")\\z");
    Ok(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn predicate(mode: FilterMode, query: &str) -> Predicate {
        Predicate::compile(
            &ColumnFilter {
                mode,
                query: query.into(),
                ..Default::default()
            },
            None,
        )
        .unwrap()
    }
    #[test]
    fn signal_scans_reuse_samples_and_report_the_declaration_name() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use volna_trace::data::{Bit, SignalHistory, SignalShape, WaveValue};
        struct History(Arc<AtomicUsize>);
        impl SignalHistory for History {
            fn shape(&self) -> SignalShape {
                SignalShape::Text
            }
            fn len(&self) -> usize {
                10_000
            }
            fn time(&self, i: usize) -> u64 {
                self.0.fetch_add(1, Ordering::Relaxed);
                i as u64
            }
            fn value(&self, _: Option<usize>) -> WaveValue {
                panic!("filter must borrow values")
            }
            fn bit(&self, _: Option<usize>) -> Bit {
                Bit::Other
            }
            fn value_view(&self, _: Option<usize>) -> ValueView<'_> {
                ValueView::Text("match".into())
            }
        }
        let reads = Arc::new(AtomicUsize::new(0));
        let source = Source::Signals {
            histories: vec![Arc::new(History(reads.clone()))],
            axis: None,
            _reservation: None,
        };
        let column = Column::Signal {
            index: 0,
            name: "cpu.request_state".into(),
        };
        let unit = crate::trace::Unit {
            timescale: -9,
            name: None,
        };
        let mut program = Program::compile(
            &[(
                column.clone(),
                ColumnFilter {
                    query: "match".into(),
                    ..Default::default()
                },
            )],
            &unit,
            &source,
        )
        .unwrap();
        let mut scratch = String::new();
        for row in 0..10_000 {
            assert!(program.matches(&source, row, &mut scratch));
        }
        assert!(
            reads.load(Ordering::Relaxed) < 80_000,
            "a sequential scan must not binary-search the entire history per row"
        );
        let error = Program::compile(
            &[(
                column,
                ColumnFilter {
                    query: "[".into(),
                    mode: FilterMode::Regex,
                    ..Default::default()
                },
            )],
            &unit,
            &source,
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("cpu.request_state"));
    }

    #[test]
    fn text_modes_are_literal_anchored_unicode_and_bounded() {
        assert!(predicate(FilterMode::Contains, "a.b").text("A.B suffix"));
        assert!(!predicate(FilterMode::Contains, "a.b").text("axb"));
        assert!(predicate(FilterMode::Wildcard, r"load\?*ö").text("LOAD?abcÖ"));
        assert!(!predicate(FilterMode::Wildcard, "load?").text("reloadX"));
        assert!(predicate(FilterMode::Fuzzy, "cLm").text("cache_load_miss"));
        assert!(!predicate(FilterMode::Exact, "load").text("load_more"));
        assert!(predicate(FilterMode::Regex, "^(load|store)$").text("store"));
        assert!(
            Predicate::compile(
                &ColumnFilter {
                    mode: FilterMode::Regex,
                    query: "[".into(),
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
    }
}
