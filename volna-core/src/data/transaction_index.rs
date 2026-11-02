//! Prepared waveform queries; navigation never walks hidden transactions or stages.

use super::{
    admission::Storage, intervals::Intervals, loaded_tracks::LoadedGenerator, occupancy::Occupancy,
};
use std::ops::Range;
use std::sync::Arc;
use volna_trace::{
    data::transactions::{Status, Transaction},
    remote::memory::{MemoryBudget, Reservation},
};

#[derive(Clone, Copy, Debug)]
struct Span {
    begin: u64,
    end: u64,
}

#[derive(Debug, Default)]
struct RowData {
    ordinals: Vec<usize>,
    stages: Vec<Span>,
    errors: Vec<Span>,
}

pub struct TransactionRow<'a> {
    data: &'a RowData,
    records: &'a [Transaction],
}

impl<'a> TransactionRow<'a> {
    #[must_use]
    pub fn window(&self, start: f64, end: f64) -> &'a [usize] {
        let first = self
            .data
            .ordinals
            .partition_point(|&i| (self.records[i].end as f64) < start);
        let last = self
            .data
            .ordinals
            .partition_point(|&i| self.records[i].begin as f64 <= end);
        &self.data.ordinals[first.min(last)..last]
    }

    #[must_use]
    pub fn at(&self, time: u64) -> Option<usize> {
        let end = self
            .data
            .ordinals
            .partition_point(|&i| self.records[i].begin <= time);
        let i = *self.data.ordinals.get(end.checked_sub(1)?)?;
        let tx = &self.records[i];
        (time < tx.end || time == tx.begin && tx.begin == tx.end).then_some(i)
    }

    #[must_use]
    pub fn activity(&self, start: f64, end: f64) -> (bool, bool, bool) {
        let records = !self.window(start, end).is_empty();
        (
            records,
            intersects(&self.data.stages, start, end),
            intersects(&self.data.errors, start, end),
        )
    }
}

#[derive(Debug)]
pub struct TransactionIndex {
    source: Arc<LoadedGenerator>,
    pub occupancy: Occupancy,
    intervals: Intervals,
    rows: Vec<RowData>,
    boundaries: Vec<u64>,
    _reservation: Option<Reservation>,
}

impl TransactionIndex {
    #[must_use]
    pub fn active_count(&self, time: u64) -> usize {
        self.intervals.count(self.source.transactions(), time)
    }

    #[must_use]
    pub fn active_rank(&self, time: u64, ordinal: usize) -> Option<usize> {
        self.intervals
            .rank(self.source.transactions(), time, ordinal)
    }
    #[must_use]
    pub fn source(&self) -> &LoadedGenerator {
        &self.source
    }

    pub fn build(
        source: Arc<LoadedGenerator>,
        budget: Option<&MemoryBudget>,
    ) -> anyhow::Result<Self> {
        let mut storage = Storage::new("the transaction track index", size_of::<Self>(), budget)?;
        let records = source.transactions();
        let occupancy = Occupancy::build(records, &mut storage)?;
        let intervals = Intervals::build(records, &mut storage)?;
        let mut counts = storage.vec(source.depth())?;
        counts.resize(source.depth(), [0usize; 3]);
        for (ordinal, tx) in records.iter().enumerate() {
            let count = &mut counts[source.sub_row(ordinal)];
            count[0] += 1;
            count[1] = count[1]
                .checked_add(tx.stages.len().max(1))
                .ok_or_else(|| anyhow::anyhow!("stage index size overflow"))?;
            count[2] += usize::from(tx.status == Status::Error);
        }
        let mut rows = storage.vec(source.depth())?;
        for count in &counts {
            rows.push(RowData {
                ordinals: storage.vec(count[0])?,
                stages: storage.vec(count[1])?,
                errors: storage.vec(count[2])?,
            });
        }
        storage.release(counts)?;
        let mut boundaries = storage.vec(
            records
                .len()
                .checked_mul(2)
                .ok_or_else(|| anyhow::anyhow!("boundary index size overflow"))?,
        )?;
        for (ordinal, tx) in records.iter().enumerate() {
            boundaries.extend([tx.begin, tx.end]);
            let row = &mut rows[source.sub_row(ordinal)];
            row.ordinals.push(ordinal);
            if tx.status == Status::Error {
                row.errors.push(Span {
                    begin: tx.begin,
                    end: tx.end,
                });
            }
            for stage in &tx.stages {
                let begin = stage.begin.max(tx.begin);
                let end = stage.end.unwrap_or(tx.end).min(tx.end);
                if begin <= end {
                    row.stages.push(Span { begin, end });
                }
            }
            if tx.stages.is_empty() {
                row.stages.push(Span {
                    begin: tx.begin,
                    end: tx.end,
                });
            }
        }
        for row in &mut rows {
            union(&mut row.stages);
            union(&mut row.errors);
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        Ok(Self {
            source,
            occupancy,
            intervals,
            rows,
            boundaries,
            _reservation: storage.finish(),
        })
    }

    #[must_use]
    pub fn row(&self, row: usize) -> Option<TransactionRow<'_>> {
        self.rows.get(row).map(|data| TransactionRow {
            data,
            records: self.source.transactions(),
        })
    }

    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        (size_of::<Self>()
            + self.occupancy.bytes()
            + self.intervals.bytes()
            + self.rows.capacity() * size_of::<RowData>()
            + self.boundaries.capacity() * size_of::<u64>()
            + self
                .rows
                .iter()
                .map(|row| {
                    row.ordinals.capacity() * size_of::<usize>()
                        + (row.stages.capacity() + row.errors.capacity()) * size_of::<Span>()
                })
                .sum::<usize>()) as u64
    }

    #[must_use]
    pub fn next_boundary(&self, time: u64) -> Option<u64> {
        self.boundaries
            .get(self.boundaries.partition_point(|&t| t <= time))
            .copied()
    }

    #[must_use]
    pub fn prev_boundary(&self, time: u64) -> Option<u64> {
        let end = self.boundaries.partition_point(|&t| t < time);
        end.checked_sub(1).map(|i| self.boundaries[i])
    }

    /// Returns at most 128 records; the next page starts after the returned ranks.
    #[must_use]
    pub fn active_page(&self, time: u64, range: Range<usize>) -> Vec<usize> {
        self.intervals.page(self.source.transactions(), time, range)
    }
}

fn intersects(spans: &[Span], start: f64, end: f64) -> bool {
    let first = spans.partition_point(|span| (span.end as f64) < start);
    spans
        .get(first)
        .is_some_and(|span| (span.begin as f64) < end)
}

fn union(spans: &mut Vec<Span>) {
    spans.sort_unstable_by_key(|span| (span.begin, span.end));
    let mut len = 0;
    for i in 0..spans.len() {
        let span = spans[i];
        if len > 0 && span.begin <= spans[len - 1].end {
            spans[len - 1].end = spans[len - 1].end.max(span.end);
        } else {
            spans[len] = span;
            len += 1;
        }
    }
    spans.truncate(len);
}
