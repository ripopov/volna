//! Saved measurements and waveform samples presented through signal columns.

use crate::data::admission::Buffer;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use volna_trace::data::{Bit, SignalHistory, SignalShape, WaveValue};
use volna_trace::remote::memory::MemoryBudget;

#[derive(Clone, Debug)]
pub struct Statistics {
    pub name: Arc<str>,
    rows: Arc<Buffer<Statistic>>,
    shift: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Statistic {
    pub start: u64,
    pub end: Option<u64>,
    pub value: Option<f64>,
}

impl Statistics {
    pub(crate) fn from_output(
        name: String,
        output: &crate::computed::DerivedOutput,
        domain: (u64, u64),
        budget: &MemoryBudget,
    ) -> anyhow::Result<Self> {
        use crate::computed::DerivedOutput;
        let mut rows = Buffer::new(budget)?;
        match output {
            DerivedOutput::Measurement(value) => rows.push(Statistic {
                start: value.bounds.0,
                end: Some(value.bounds.1),
                value: value.value,
            })?,
            DerivedOutput::IntervalTable(values) => {
                for row in values.iter().filter(|row| row.bounds.0 < row.bounds.1) {
                    rows.push(Statistic {
                        start: row.bounds.0,
                        end: Some(row.bounds.1),
                        value: row.result.as_ref().ok().and_then(|value| value.value),
                    })?;
                }
            }
            DerivedOutput::Waveform(history) => {
                for index in 0..history.len() {
                    let start = history.time(index);
                    if start < domain.0 || start > domain.1 {
                        continue;
                    }
                    rows.push(Statistic {
                        start,
                        end: None,
                        value: match history.value(Some(index)) {
                            WaveValue::Real(value) => Some(value),
                            WaveValue::Bits(value) if value == "1" => Some(1.0),
                            WaveValue::Bits(value) if value == "0" => Some(0.0),
                            _ => None,
                        },
                    })?;
                }
            }
        }
        let result = Self {
            name: name.into(),
            rows: Arc::new(rows),
            shift: 0,
        };
        result.validate()?;
        Ok(result)
    }

    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(super) fn row(&self, index: usize) -> Option<Statistic> {
        let mut row = *self.rows.get(index)?;
        row.start = self.time(row.start);
        row.end = row.end.map(|end| self.time(end));
        Some(row)
    }

    fn time(&self, value: u64) -> u64 {
        let Some(factor) = 10_u64.checked_pow(self.shift.unsigned_abs()) else {
            return if self.shift > 0 && value != 0 {
                u64::MAX
            } else {
                0
            };
        };
        if self.shift >= 0 {
            value.saturating_mul(factor)
        } else {
            value / factor + u64::from(value % factor >= factor.div_ceil(2))
        }
    }

    pub(super) fn retime(&mut self, by: crate::trace::Rescale) {
        self.shift += match by {
            crate::trace::Rescale::Finer(factor) => factor.ilog10() as i32,
            crate::trace::Rescale::Coarser(factor) => -(factor.ilog10() as i32),
        };
    }

    pub(super) fn restore(
        raw: &serde_json::value::RawValue,
        budget: &MemoryBudget,
    ) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct Saved {
            name: String,
            rows: Box<serde_json::value::RawValue>,
        }
        let saved: Saved = serde_json::from_str(raw.get())?;
        struct Rows<'a>(&'a MemoryBudget);
        impl<'de> serde::de::Visitor<'de> for Rows<'_> {
            type Value = Buffer<Statistic>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("statistics rows")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                use serde::de::Error;
                let mut rows = Buffer::new(self.0).map_err(A::Error::custom)?;
                while let Some(row) = seq.next_element()? {
                    if rows.len() == 1_000_000 {
                        return Err(A::Error::custom("Too many statistics rows."));
                    }
                    rows.push(row).map_err(A::Error::custom)?;
                }
                Ok(rows)
            }
        }
        use serde::Deserializer;
        let mut deserializer = serde_json::Deserializer::from_str(saved.rows.get());
        let rows = deserializer.deserialize_seq(Rows(budget))?;
        deserializer.end()?;
        let result = Self {
            name: saved.name.into(),
            rows: Arc::new(rows),
            shift: 0,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.rows.len() <= 1_000_000, "Too many statistics rows.");
        for (index, row) in self.rows.iter().enumerate() {
            anyhow::ensure!(
                row.end.is_none_or(|end| row.start < end) && row.value.is_none_or(f64::is_finite),
                "Invalid statistic value or interval bounds."
            );
            anyhow::ensure!(
                index == 0 || {
                    let previous = &self.rows[index - 1];
                    previous.end.is_some() == row.end.is_some()
                        && previous.start < row.start
                        && previous.end.is_none_or(|end| end <= row.start)
                },
                "Statistics must have one row type, ordered timestamps, and non-overlapping intervals."
            );
        }
        Ok(())
    }

    pub(super) fn has_intervals(&self) -> bool {
        self.rows.first().is_some_and(|row| row.end.is_some())
    }

    pub(super) fn signal_count(&self) -> usize {
        if self.has_intervals() { 2 } else { 1 }
    }

    pub(super) fn histories(self: &Arc<Self>) -> Vec<Arc<dyn SignalHistory>> {
        (0..self.signal_count())
            .map(|column| {
                Arc::new(History {
                    data: self.clone(),
                    end: column == 1,
                }) as Arc<dyn SignalHistory>
            })
            .collect()
    }
}

struct History {
    data: Arc<Statistics>,
    end: bool,
}
impl SignalHistory for History {
    fn shape(&self) -> SignalShape {
        if self.end {
            SignalShape::Vector { width: 64 }
        } else {
            SignalShape::Real
        }
    }
    fn len(&self) -> usize {
        self.data.len()
    }
    fn time(&self, index: usize) -> u64 {
        self.data.row(index).expect("statistics row").start
    }
    fn value(&self, index: Option<usize>) -> WaveValue {
        let Some(row) = index.and_then(|index| self.data.row(index)) else {
            return WaveValue::Unavailable;
        };
        if self.end {
            row.end.map_or(WaveValue::Unavailable, |end| {
                WaveValue::Bits(format!("{end:064b}"))
            })
        } else {
            row.value.map_or(WaveValue::Unavailable, WaveValue::Real)
        }
    }
    fn bit(&self, _: Option<usize>) -> Bit {
        Bit::Unavailable
    }
}

impl Serialize for Statistics {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        struct Rows<'a>(&'a Statistics);
        impl Serialize for Rows<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_seq(
                    (0..self.0.len()).map(|index| self.0.row(index).expect("statistics row")),
                )
            }
        }
        let mut state = serializer.serialize_struct("Statistics", 2)?;
        state.serialize_field("name", self.name.as_ref())?;
        state.serialize_field("rows", &Rows(self))?;
        state.end()
    }
}

impl PartialEq for Statistics {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.len() == other.len()
            && (0..self.len()).all(|index| self.row(index) == other.row(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Document,
        data::translator::Translators,
        table::{TableModel, TableSource},
        wave::model::Link,
    };

    fn saved() -> Box<serde_json::value::RawValue> {
        serde_json::value::RawValue::from_string(r#"{"name":"maximum","rows":[{"start":0,"end":3,"value":2.0},{"start":3,"end":10,"value":8.0}]}"#.into()).unwrap()
    }

    // A user closes a table while a shared snapshot remains available; the saved values stay accounted until their final owner closes.
    #[test]
    fn detached_table_keeps_snapshot_admission_until_the_last_owner_closes() {
        let budget = MemoryBudget::new(1 << 20);
        let data = Arc::new(Statistics::restore(&saved(), &budget).unwrap());
        let retained = budget.used();
        assert!(retained > 0);
        let mut doc = Document::new();
        let mut table = TableModel::new(
            TableSource::Statistics(data.clone()),
            Link::default(),
            MemoryBudget::new(8 << 20),
            Translators::builtin(),
        );
        table.attach(&mut doc, &Default::default()).unwrap();
        assert_eq!(table.len(), 2);
        table.detach(&mut doc);
        assert_eq!(budget.used(), retained);
        drop(table);
        assert_eq!(data.row(1).unwrap().value, Some(8.0));
        assert_eq!(budget.used(), retained);
        drop(data);
        assert_eq!(budget.used(), 0);
    }

    // A user reopens statistics with insufficient memory, then retries; restore refuses without retaining a partially loaded snapshot.
    #[test]
    fn restore_refusal_releases_storage_and_allows_retry() {
        let budget = MemoryBudget::new(1);
        assert!(
            Statistics::restore(&saved(), &budget)
                .unwrap_err()
                .to_string()
                .contains("memory budget")
        );
        assert_eq!(budget.used(), 0);
        budget.set_limit(1 << 20);
        let data = Statistics::restore(&saved(), &budget).unwrap();
        assert_eq!(data.row(1).unwrap().start, 3);
        drop(data);
        assert_eq!(budget.used(), 0);
    }

    // A user changes one view's time scale and saves it; timestamps convert without changing a second view of the same snapshot.
    #[test]
    fn retimed_snapshot_saves_converted_timestamps_without_copying_rows() {
        let budget = MemoryBudget::new(1 << 20);
        let original = Statistics::restore(&saved(), &budget).unwrap();
        let retained = budget.used();
        budget.set_limit(retained);
        let mut scaled = original.clone();
        scaled.retime(crate::trace::Rescale::Finer(1000));
        assert_eq!(scaled.row(1).unwrap().start, 3000);
        assert_eq!(original.row(1).unwrap().start, 3);
        let json = serde_json::to_value(&scaled).unwrap();
        assert_eq!(json["rows"][1]["end"], 10000);
        assert_eq!(budget.used(), retained);
    }
}
