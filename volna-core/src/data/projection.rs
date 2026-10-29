//! Bit and slice histories share raw values and index only their own transitions.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use volna_trace::data::value_view::ValueView;
use volna_trace::data::{Bit, SignalHistory, SignalShape, WaveValue};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

/// Inclusive positions in a raw MSB-first logic value, independent of HDL indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BitSlice {
    pub first: u32,
    pub last: u32,
}

impl BitSlice {
    pub fn validate(self, shape: SignalShape) -> Result<Self> {
        ensure!(shape.is_digital(), "only logic signals have bits");
        ensure!(
            self.first < shape.width() && self.last < shape.width(),
            "slice outside signal"
        );
        Ok(self)
    }

    #[must_use]
    pub fn width(self) -> u32 {
        self.first.abs_diff(self.last) + 1
    }

    #[must_use]
    pub fn shape(self) -> SignalShape {
        if self.width() == 1 {
            SignalShape::Bit
        } else {
            SignalShape::Vector {
                width: self.width(),
            }
        }
    }

    fn offset(self, i: u32) -> usize {
        (if self.first <= self.last {
            self.first + i
        } else {
            self.first - i
        }) as usize
    }

    #[must_use]
    pub fn compose(self, slice: Self) -> Self {
        Self {
            first: self.offset(slice.first) as u32,
            last: self.offset(slice.last) as u32,
        }
    }

    #[must_use]
    pub fn contains(self, offset: u32) -> bool {
        (self.first.min(self.last)..=self.first.max(self.last)).contains(&offset)
    }

    fn equal(self, a: ValueView<'_>, b: ValueView<'_>) -> bool {
        match (a, b) {
            (ValueView::Unavailable, ValueView::Unavailable) => true,
            (ValueView::Logic(a), ValueView::Logic(b)) => {
                (0..self.width()).all(|i| a.bit(self.offset(i)) == b.bit(self.offset(i)))
            }
            _ => false,
        }
    }

    fn value(self, value: ValueView<'_>) -> WaveValue {
        match value {
            ValueView::Logic(bits) => WaveValue::Bits(
                (0..self.width())
                    .map(|i| char::from(bits.bit(self.offset(i))))
                    .collect(),
            ),
            _ => WaveValue::Unavailable,
        }
    }
}

pub struct ProjectedHistory {
    source: Arc<dyn SignalHistory>,
    slice: BitSlice,
    changes: Vec<usize>,
    reservation: Reservation,
}

impl ProjectedHistory {
    pub fn build(
        source: Arc<dyn SignalHistory>,
        slice: BitSlice,
        budget: &MemoryBudget,
    ) -> Result<Self> {
        slice.validate(source.shape())?;
        let changed = |i: usize| {
            !slice.equal(
                source.value_view(i.checked_sub(1)),
                source.value_view(Some(i)),
            )
        };
        let count = (0..source.len()).filter(|&i| changed(i)).count();
        let reservation = budget.reserve_object(
            "the signal slice index",
            super::admission::bytes::<usize>(count)?,
        )?;
        let mut changes = Vec::with_capacity(count);
        changes.extend((0..source.len()).filter(|&i| changed(i)));
        Ok(Self {
            source,
            slice,
            changes,
            reservation,
        })
    }
}

impl SignalHistory for ProjectedHistory {
    fn resident_bytes(&self) -> u64 {
        self.reservation.bytes()
    }
    fn shape(&self) -> SignalShape {
        self.slice.shape()
    }
    fn len(&self) -> usize {
        self.changes.len()
    }
    fn time(&self, i: usize) -> u64 {
        self.source.time(self.changes[i])
    }
    fn value(&self, i: Option<usize>) -> WaveValue {
        self.slice
            .value(self.source.value_view(i.map(|i| self.changes[i])))
    }
    fn bit(&self, i: Option<usize>) -> Bit {
        match self.source.value_view(i.map(|i| self.changes[i])) {
            ValueView::Logic(bits) => Bit::from_ascii(bits.bit(self.slice.first as usize)),
            _ => Bit::Unavailable,
        }
    }
}

type Key = (usize, BitSlice);

struct Pending {
    source: Weak<dyn SignalHistory>,
    request: u64,
    result: Option<Result<Arc<dyn SignalHistory>, String>>,
}

#[derive(Default)]
pub(crate) struct Projections {
    entries: HashMap<Key, Pending>,
    next_request: u64,
}

impl Projections {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn sync(
        &mut self,
        retained: HashMap<Key, Arc<dyn SignalHistory>>,
        wanted: HashMap<Key, Arc<dyn SignalHistory>>,
        budget: &MemoryBudget,
    ) -> Vec<crate::session::LoadRequest> {
        self.entries.retain(|key, entry| {
            retained.get(key).is_some_and(|source| {
                entry
                    .source
                    .upgrade()
                    .is_some_and(|old| Arc::ptr_eq(&old, source))
            })
        });
        let mut requests = Vec::new();
        for (key, source) in wanted {
            if self.entries.contains_key(&key) {
                continue;
            }
            self.next_request = self
                .next_request
                .checked_add(1)
                .expect("projection request identities exhausted");
            self.entries.insert(
                key,
                Pending {
                    source: Arc::downgrade(&source),
                    request: self.next_request,
                    result: None,
                },
            );
            requests.push(crate::session::LoadRequest::Projection {
                request: self.next_request,
                source,
                slice: key.1,
                budget: budget.clone(),
            });
        }
        requests
    }

    pub fn key(source: &Arc<dyn SignalHistory>, slice: BitSlice) -> Key {
        (Arc::as_ptr(source).cast::<()>() as usize, slice)
    }

    pub fn get(
        &self,
        source: &Arc<dyn SignalHistory>,
        slice: BitSlice,
    ) -> Option<&Result<Arc<dyn SignalHistory>, String>> {
        self.entries.get(&Self::key(source, slice))?.result.as_ref()
    }

    pub fn retry_failed(&mut self) {
        self.entries
            .retain(|_, entry| !matches!(entry.result, Some(Err(_))));
    }

    pub fn deliver(&mut self, request: u64, result: Result<Arc<dyn SignalHistory>>) -> bool {
        let Some(entry) = self
            .entries
            .values_mut()
            .find(|entry| entry.request == request && entry.result.is_none())
        else {
            return false;
        };
        entry.result = Some(result.map_err(|e| e.to_string()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_trace::data::history::VecHistory;

    fn source() -> Arc<dyn SignalHistory> {
        Arc::new(VecHistory {
            shape: SignalShape::Vector { width: 4 },
            times: vec![5, 10, 15, 20, 25, 30, 35],
            values: ["0000", "0001", "0011", "0011", "x-z1", "x-z1", "1011"]
                .map(|s| WaveValue::Bits(s.into()))
                .into(),
            initial: WaveValue::Unavailable,
        })
    }

    #[test]
    fn every_slice_matches_its_source_without_unrelated_or_redundant_edges() {
        let source = source();
        let budget = MemoryBudget::new(1024);
        for first in 0..4 {
            for last in 0..4 {
                let slice = BitSlice { first, last };
                let projected = ProjectedHistory::build(source.clone(), slice, &budget).unwrap();
                let mut expected_edges = Vec::new();
                let mut previous = WaveValue::Unavailable;
                for i in 0..source.len() {
                    let value = slice.value(source.value_view(Some(i)));
                    if value != previous {
                        expected_edges.push(source.time(i));
                    }
                    previous = value;
                }
                assert_eq!(
                    (0..projected.len())
                        .map(|i| projected.time(i))
                        .collect::<Vec<_>>(),
                    expected_edges
                );
                for time in 0..40 {
                    assert_eq!(
                        projected.value(projected.index_at(time)),
                        slice.value(source.value_view(source.index_at(time)))
                    );
                    assert_eq!(
                        projected.next_change_after(time),
                        expected_edges.iter().copied().find(|&t| t > time)
                    );
                    assert_eq!(
                        projected.prev_change_before(time),
                        expected_edges.iter().copied().rev().find(|&t| t < time)
                    );
                }
            }
        }
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn indexes_obey_memory_limits_and_release_without_retaining_source_values() {
        let source = source();
        let budget = MemoryBudget::new(1);
        let slice = BitSlice { first: 2, last: 3 };
        assert!(ProjectedHistory::build(source.clone(), slice, &budget).is_err());
        assert_eq!(budget.used(), 0);
        assert_eq!(Arc::strong_count(&source), 1);
        budget.set_limit(1024);
        let projected = ProjectedHistory::build(source.clone(), slice, &budget).unwrap();
        assert_eq!(budget.used(), projected.resident_bytes());
        assert_eq!(Arc::strong_count(&source), 2);
        drop(projected);
        assert_eq!(budget.used(), 0);
        assert_eq!(Arc::strong_count(&source), 1);
        assert!(ProjectedHistory::build(source, BitSlice { first: 4, last: 4 }, &budget).is_err());
    }

    #[test]
    fn discarded_jobs_and_retries_cannot_replace_a_newer_projection() {
        let source = source();
        let slice = BitSlice { first: 2, last: 3 };
        let key = Projections::key(&source, slice);
        let wanted = || HashMap::from([(key, source.clone())]);
        let budget = MemoryBudget::new(1024);
        let mut projections = Projections::default();
        let stale = projections.sync(wanted(), wanted(), &budget).pop().unwrap();
        assert!(
            projections
                .sync(HashMap::new(), HashMap::new(), &budget)
                .is_empty()
        );
        let current = projections.sync(wanted(), wanted(), &budget).pop().unwrap();
        let crate::session::LoadResult::Projection { request, result } = stale.perform() else {
            unreachable!()
        };
        assert!(!projections.deliver(request, result));
        let crate::session::LoadResult::Projection { request, result } =
            current.fail(anyhow::anyhow!("budget exceeded"))
        else {
            unreachable!()
        };
        assert!(projections.deliver(request, result));
        assert!(projections.get(&source, slice).unwrap().is_err());
        projections.retry_failed();
        let retry = projections.sync(wanted(), wanted(), &budget).pop().unwrap();
        let crate::session::LoadResult::Projection { request, result } = retry.perform() else {
            unreachable!()
        };
        assert!(projections.deliver(request, result));
        assert!(projections.get(&source, slice).unwrap().is_ok());
    }
}
