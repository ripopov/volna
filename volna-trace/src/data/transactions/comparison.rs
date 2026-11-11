//! Stack-safe, lexicographic comparison steps over shared attribute graphs.

use super::AttributeValue;
use std::collections::HashSet;

#[derive(Clone, Copy)]
pub enum References {
    /// Retain shared-wrapper boundaries for ordinary value equality.
    Preserve,
    /// Compare referenced content for ordering and wire consistency.
    Resolve,
}

pub enum Step<'a> {
    Value(&'a AttributeValue, &'a AttributeValue),
    Key(&'a str, &'a str),
    Length(usize, usize),
    Progress,
}

enum Work<'a> {
    Value(&'a AttributeValue, &'a AttributeValue),
    Keys(&'a str, &'a str),
    Values(&'a [AttributeValue], &'a [AttributeValue]),
    Attributes(
        &'a [(String, AttributeValue)],
        &'a [(String, AttributeValue)],
    ),
}

/// Consumers stop at the first unequal step and supply their own scalar semantics.
/// Progress steps let cooperative callers yield even while traversing only shared nodes.
pub struct Comparison<'a> {
    work: Vec<Work<'a>>,
    seen: HashSet<(usize, usize)>,
    references: References,
}

impl<'a> Comparison<'a> {
    #[must_use]
    pub fn new(a: &'a AttributeValue, b: &'a AttributeValue, references: References) -> Self {
        Self {
            work: vec![Work::Value(a, b)],
            seen: HashSet::new(),
            references,
        }
    }

    #[must_use]
    pub fn attributes(
        a: &'a [(String, AttributeValue)],
        b: &'a [(String, AttributeValue)],
        references: References,
    ) -> Self {
        Self {
            work: vec![Work::Attributes(a, b)],
            seen: HashSet::new(),
            references,
        }
    }

    fn shared(&mut self, a: &'a AttributeValue, b: &'a AttributeValue) {
        // A repeated pair has already compared equal on this depth-first path.
        if self.seen.insert((
            std::ptr::from_ref(a) as usize,
            std::ptr::from_ref(b) as usize,
        )) {
            self.work.push(Work::Value(a, b));
        }
    }
}

impl<'a> Iterator for Comparison<'a> {
    type Item = Step<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(match self.work.pop()? {
            Work::Value(a, b) => {
                if matches!(self.references, References::Resolve)
                    && (matches!(a, AttributeValue::Shared(_))
                        || matches!(b, AttributeValue::Shared(_)))
                {
                    self.shared(a.resolved(), b.resolved());
                    return Some(Step::Progress);
                }
                match (a, b) {
                    (AttributeValue::Shared(a_value), AttributeValue::Shared(b_value)) => {
                        self.shared(a_value, b_value);
                        Step::Progress
                    }
                    (AttributeValue::List(a_values), AttributeValue::List(b_values)) => {
                        self.work.push(Work::Values(a_values, b_values));
                        Step::Progress
                    }
                    (AttributeValue::Map(a_values), AttributeValue::Map(b_values)) => {
                        self.work.push(Work::Attributes(a_values, b_values));
                        Step::Progress
                    }
                    _ => Step::Value(a, b),
                }
            }
            Work::Keys(a, b) => Step::Key(a, b),
            Work::Attributes(a, b) => match (a.split_first(), b.split_first()) {
                (Some(((a_key, a_value), a_tail)), Some(((b_key, b_value), b_tail))) => {
                    self.work.push(Work::Attributes(a_tail, b_tail));
                    self.work.push(Work::Value(a_value, b_value));
                    self.work.push(Work::Keys(a_key, b_key));
                    Step::Progress
                }
                _ => Step::Length(a.len(), b.len()),
            },
            Work::Values(a, b) => match (a.split_first(), b.split_first()) {
                (Some((a_value, a_tail)), Some((b_value, b_tail))) => {
                    self.work.push(Work::Values(a_tail, b_tail));
                    self.work.push(Work::Value(a_value, b_value));
                    Step::Progress
                }
                _ => Step::Length(a.len(), b.len()),
            },
        })
    }
}
