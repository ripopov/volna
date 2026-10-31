//! Point-record labels and severity summaries shared by waveform and detail views.

use std::ops::Range;
use volna_trace::data::transactions::{AttributeValue, Transaction};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKind {
    Log,
    Event,
}

impl PointKind {
    #[must_use]
    pub fn from_generator(kind: &str) -> Option<Self> {
        match kind {
            "LOG" => Some(Self::Log),
            "EVENT" => Some(Self::Event),
            _ => None,
        }
    }

    #[must_use]
    pub fn label(self, record: &Transaction) -> Option<&AttributeValue> {
        let key = match self {
            Self::Log => "log.message",
            Self::Event => "vtr.label",
        };
        record
            .attributes
            .iter()
            .find(|a| a.key == key)
            .map(|a| &a.value)
    }
}

pub const SEVERITIES: usize = 7;
pub type Counts = [u64; SEVERITIES];

#[must_use]
pub fn severity(record: &Transaction) -> usize {
    record
        .attributes
        .iter()
        .find_map(|a| {
            if a.key != "log.severity" {
                return None;
            }
            match &a.value {
                AttributeValue::Enum { value, .. } => {
                    usize::try_from(*value).ok().filter(|n| *n < 6)
                }
                _ => None,
            }
        })
        .unwrap_or(6)
}

#[derive(Debug)]
pub struct PointIndex {
    pub kind: PointKind,
    prefixes: Vec<Counts>,
}

impl PointIndex {
    pub(crate) async fn build<F: std::future::Future<Output = ()>>(
        records: &[Transaction],
        checkpoint: &mut impl FnMut() -> F,
    ) -> Option<Self> {
        let kind = records.first()?.attributes.iter().find_map(|a| {
            if a.key != "record.kind" {
                return None;
            }
            match &a.value {
                AttributeValue::Text(s) if s == "log" => Some(PointKind::Log),
                AttributeValue::Text(s) if s == "event" => Some(PointKind::Event),
                _ => None,
            }
        })?;
        if kind == PointKind::Event {
            return Some(Self {
                kind,
                prefixes: Vec::new(),
            });
        }
        let mut prefixes = Vec::with_capacity(records.len() + 1);
        let mut counts = [0; SEVERITIES];
        prefixes.push(counts);
        for record in records {
            checkpoint().await;
            counts[severity(record)] += 1;
            prefixes.push(counts);
        }
        Some(Self { kind, prefixes })
    }

    #[must_use]
    pub fn counts(&self, records: Range<usize>) -> Counts {
        if self.kind == PointKind::Event {
            let mut counts = [0; SEVERITIES];
            counts[6] = records.len() as u64;
            return counts;
        }
        std::array::from_fn(|i| self.prefixes[records.end][i] - self.prefixes[records.start][i])
    }

    pub(crate) fn bytes(&self) -> u64 {
        (self.prefixes.capacity() * std::mem::size_of::<Counts>()) as u64
    }
}
