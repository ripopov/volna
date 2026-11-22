//! Cached time-interleaved k-way merge and interval-indexed queries over one or more immutable
//! transaction generators.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::ops::Range;
use std::sync::Arc;

use super::{
    activity::Activity,
    occupancy::{RowIndex, TransactionTable, raster_source, raster_source_filtered},
    order::{RowOrder, RowSortMeta, evaluate_row_sort},
    raster::PixelMap,
    stages::{StageAnchorFilter, StageVisibility, activity_target_score, tx_overlaps_view},
};
use crate::{
    data::loaded_tracks::LoadedGenerator, scene::PresenceMask, trace::TraceId,
    wave::viewport::Viewport,
};
use volna_trace::{
    data::transactions::{Status, TrackRef, Transaction, TransactionRef},
    remote::memory::MemoryBudget,
};

const HIDDEN_ROW: u32 = u32::MAX;

#[derive(Clone, Debug)]
pub struct RowStream {
    pub source_index: usize,
    pub trace: TraceId,
    pub generator: Arc<LoadedGenerator>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RowEntry {
    stream: u32,
    ordinal: u32,
}

type Fingerprint = Vec<(usize, TraceId, usize)>;

fn stream_fingerprint(streams: &[RowStream]) -> Fingerprint {
    streams
        .iter()
        .map(|s| (s.source_index, s.trace, Arc::as_ptr(&s.generator) as usize))
        .collect()
}

struct MergedTable<'a> {
    streams: &'a [RowStream],
    entries: &'a [RowEntry],
}

impl TransactionTable for MergedTable<'_> {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn get(&self, index: usize) -> &Transaction {
        let entry = self.entries[index];
        &self.streams[entry.stream as usize].generator.transactions()[entry.ordinal as usize]
    }
}

enum FilteredIndexState {
    Ready(RowIndex),
    Refused { available_bytes: u64 },
}

struct FilteredIndex {
    visibility: StageVisibility,
    primary_lane: String,
    state: FilteredIndexState,
}

/// Cached static k-way merge or stage-ordered permutation across the active visible generators
/// of a pipeline panel.
#[derive(Default)]
pub(crate) struct MergedRows {
    fingerprint: Fingerprint,
    order: RowOrder,
    resolved_stages: Vec<String>,
    primary_lane: String,
    streams: Vec<RowStream>,
    generators: Vec<Arc<LoadedGenerator>>,
    generator_ptrs: Vec<usize>,
    entries: Vec<RowEntry>,
    sort_meta: Vec<RowSortMeta>,
    reverse: Vec<Vec<u32>>,
    reverse_monotonic: Vec<bool>,
    stream_begin_bounds: Vec<Option<(u64, u64)>>,
    index: Option<RowIndex>,
    visible_index: Option<FilteredIndex>,
    min_begin: u64,
    max_begin: u64,
    unmatched_count: usize,
    len: usize,
    version: u64,
}

impl MergedRows {
    pub(crate) fn fingerprint(&self) -> &[(usize, TraceId, usize)] {
        &self.fingerprint
    }

    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn order_matches(
        &self,
        order: &RowOrder,
        resolved_stages: &[String],
        primary_lane: &str,
    ) -> bool {
        self.order.mode == order.mode
            && self.order.unmatched == order.unmatched
            && self.resolved_stages.as_slice() == resolved_stages
            && self.primary_lane == primary_lane
    }

    pub(crate) fn matches(
        &self,
        streams: &[RowStream],
        order: &RowOrder,
        resolved_stages: &[String],
        primary_lane: &str,
    ) -> bool {
        self.order_matches(order, resolved_stages, primary_lane)
            && self.fingerprint.len() == streams.len()
            && self
                .fingerprint
                .iter()
                .zip(streams)
                .all(|(&(src, tr, ptr), s)| {
                    src == s.source_index
                        && tr == s.trace
                        && ptr == Arc::as_ptr(&s.generator) as usize
                })
    }

    pub(crate) fn clear(&mut self) {
        self.fingerprint.clear();
        self.order = RowOrder::default();
        self.resolved_stages.clear();
        self.primary_lane.clear();
        self.streams.clear();
        self.generators.clear();
        self.generator_ptrs.clear();
        self.entries.clear();
        self.sort_meta.clear();
        self.reverse.clear();
        self.reverse_monotonic.clear();
        self.stream_begin_bounds.clear();
        self.index = None;
        self.visible_index = None;
        self.min_begin = u64::MAX;
        self.max_begin = 0;
        self.unmatched_count = 0;
        self.len = 0;
        self.version = self.version.wrapping_add(1);
    }

    pub(crate) fn build(
        streams: Vec<RowStream>,
        order: &RowOrder,
        resolved_stages: &[String],
        primary_lane: &str,
        budget: Option<&MemoryBudget>,
    ) -> Self {
        Self::build_with_version(streams, order, resolved_stages, primary_lane, budget, 1)
    }

    fn build_with_version(
        streams: Vec<RowStream>,
        order: &RowOrder,
        resolved_stages: &[String],
        primary_lane: &str,
        budget: Option<&MemoryBudget>,
        version: u64,
    ) -> Self {
        let fingerprint = stream_fingerprint(&streams);
        let generators: Vec<Arc<LoadedGenerator>> =
            streams.iter().map(|s| Arc::clone(&s.generator)).collect();
        let generator_ptrs: Vec<usize> =
            generators.iter().map(|g| Arc::as_ptr(g) as usize).collect();
        let stored_order = RowOrder {
            mode: order.mode.clone(),
            unmatched: order.unmatched,
            follow_sort_key: true,
        };
        let stored_stages = resolved_stages.to_vec();
        let stored_lane = primary_lane.to_owned();

        if streams.is_empty() {
            return Self {
                fingerprint,
                order: stored_order,
                resolved_stages: stored_stages,
                primary_lane: stored_lane,
                streams,
                generators,
                generator_ptrs,
                entries: Vec::new(),
                sort_meta: Vec::new(),
                reverse: Vec::new(),
                reverse_monotonic: Vec::new(),
                stream_begin_bounds: Vec::new(),
                index: None,
                visible_index: None,
                min_begin: u64::MAX,
                max_begin: 0,
                unmatched_count: 0,
                len: 0,
                version,
            };
        }

        if streams.len() == 1 && order.is_start() {
            let txs = streams[0].generator.transactions();
            let len = txs.len();
            let bounds = txs
                .first()
                .zip(txs.last())
                .map(|(first, last)| (first.begin, last.begin));
            return Self {
                fingerprint,
                order: stored_order,
                resolved_stages: stored_stages,
                primary_lane: stored_lane,
                streams,
                generators,
                generator_ptrs,
                entries: Vec::new(),
                sort_meta: Vec::new(),
                reverse: Vec::new(),
                reverse_monotonic: vec![true],
                stream_begin_bounds: vec![bounds],
                index: None,
                visible_index: None,
                min_begin: bounds.map_or(u64::MAX, |(lo, _)| lo),
                max_begin: bounds.map_or(0, |(_, hi)| hi),
                unmatched_count: 0,
                len,
                version,
            };
        }

        if order.is_start() {
            let total: usize = streams
                .iter()
                .map(|s| s.generator.transactions().len())
                .sum();
            let mut entries = Vec::with_capacity(total);
            let mut reverse: Vec<Vec<u32>> = streams
                .iter()
                .map(|s| vec![0u32; s.generator.transactions().len()])
                .collect();
            let stream_begin_bounds: Vec<Option<(u64, u64)>> = streams
                .iter()
                .map(|s| {
                    let txs = s.generator.transactions();
                    txs.first().zip(txs.last()).map(|(f, l)| (f.begin, l.begin))
                })
                .collect();

            // Min-heap keyed by (begin, stream_idx, ordinal) for deterministic k-way merge.
            let mut heap: BinaryHeap<Reverse<(u64, u32, u32)>> =
                BinaryHeap::with_capacity(streams.len());
            for (s, stream) in streams.iter().enumerate() {
                if let Some(first) = stream.generator.transactions().first() {
                    heap.push(Reverse((first.begin, s as u32, 0)));
                }
            }
            while let Some(Reverse((_, stream_idx, ordinal))) = heap.pop() {
                let merged_row = entries.len();
                entries.push(RowEntry {
                    stream: stream_idx,
                    ordinal,
                });
                reverse[stream_idx as usize][ordinal as usize] = merged_row as u32;
                let next_ord = ordinal as usize + 1;
                let txs = streams[stream_idx as usize].generator.transactions();
                if next_ord < txs.len() {
                    heap.push(Reverse((txs[next_ord].begin, stream_idx, next_ord as u32)));
                }
            }

            let min_begin = entries.first().map_or(u64::MAX, |e| {
                streams[e.stream as usize].generator.transactions()[e.ordinal as usize].begin
            });
            let max_begin = entries.last().map_or(0, |e| {
                streams[e.stream as usize].generator.transactions()[e.ordinal as usize].begin
            });
            let index = if entries.is_empty() {
                None
            } else {
                let table = MergedTable {
                    streams: &streams,
                    entries: &entries,
                };
                RowIndex::build_sync(&table, budget)
            };
            let len = entries.len();
            let reverse_monotonic = vec![true; streams.len()];
            return Self {
                fingerprint,
                order: stored_order,
                resolved_stages: stored_stages,
                primary_lane: stored_lane,
                streams,
                generators,
                generator_ptrs,
                entries,
                sort_meta: Vec::new(),
                reverse,
                reverse_monotonic,
                stream_begin_bounds,
                index,
                visible_index: None,
                min_begin,
                max_begin,
                unmatched_count: 0,
                len,
                version,
            };
        }

        // General non-Start RowOrder permutation across 1 or more streams.
        #[derive(Clone, Copy)]
        struct Staged {
            sort_key: u64,
            unmatched: u8,
            secondary_key: u64,
            tx_end: u64,
            stream: u32,
            ordinal: u32,
            sort_stage: u32,
        }

        let total: usize = streams
            .iter()
            .map(|s| s.generator.transactions().len())
            .sum();
        let mut staged = Vec::with_capacity(total);
        let mut reverse: Vec<Vec<u32>> = streams
            .iter()
            .map(|s| vec![HIDDEN_ROW; s.generator.transactions().len()])
            .collect();
        let mut stream_begin_bounds: Vec<Option<(u64, u64)>> = vec![None; streams.len()];
        let mut min_begin = u64::MAX;
        let mut max_begin = 0u64;
        let mut unmatched_count = 0usize;

        for (s, stream) in streams.iter().enumerate() {
            let mut s_min = u64::MAX;
            let mut s_max = 0u64;
            let mut s_any = false;
            for (ord, tx) in stream.generator.transactions().iter().enumerate() {
                let Some(ev) = evaluate_row_sort(tx, order, resolved_stages, primary_lane) else {
                    unmatched_count += 1;
                    continue;
                };
                if !ev.matched {
                    unmatched_count += 1;
                }
                s_min = s_min.min(tx.begin);
                s_max = s_max.max(tx.begin);
                s_any = true;
                min_begin = min_begin.min(tx.begin);
                max_begin = max_begin.max(tx.begin);
                staged.push(Staged {
                    sort_key: ev.sort_key,
                    unmatched: u8::from(!ev.matched),
                    secondary_key: ev.secondary_key,
                    tx_end: tx.end,
                    stream: s as u32,
                    ordinal: ord as u32,
                    sort_stage: ev.sort_stage,
                });
            }
            if s_any {
                stream_begin_bounds[s] = Some((s_min, s_max));
            }
        }

        // `(stream, ordinal)` is unique across all rows, making unstable sort deterministic.
        staged.sort_unstable_by_key(|r| {
            (
                r.sort_key,
                r.unmatched,
                r.secondary_key,
                r.tx_end,
                r.stream,
                r.ordinal,
            )
        });

        let mut entries = Vec::with_capacity(staged.len());
        let mut sort_meta = Vec::with_capacity(staged.len());
        for (merged_row, row) in staged.into_iter().enumerate() {
            entries.push(RowEntry {
                stream: row.stream,
                ordinal: row.ordinal,
            });
            sort_meta.push(RowSortMeta {
                sort_key: row.sort_key,
                sort_stage: row.sort_stage,
                matched: row.unmatched == 0,
            });
            reverse[row.stream as usize][row.ordinal as usize] = merged_row as u32;
        }

        let reverse_monotonic: Vec<bool> = reverse
            .iter()
            .map(|rev| {
                let mut prev: Option<u32> = None;
                for &row in rev {
                    if row == HIDDEN_ROW || prev.is_some_and(|p| row <= p) {
                        return false;
                    }
                    prev = Some(row);
                }
                true
            })
            .collect();

        let index = if entries.is_empty() {
            None
        } else {
            let table = MergedTable {
                streams: &streams,
                entries: &entries,
            };
            RowIndex::build_sync(&table, budget)
        };
        let len = entries.len();
        Self {
            fingerprint,
            order: stored_order,
            resolved_stages: stored_stages,
            primary_lane: stored_lane,
            streams,
            generators,
            generator_ptrs,
            entries,
            sort_meta,
            reverse,
            reverse_monotonic,
            stream_begin_bounds,
            index,
            visible_index: None,
            min_begin,
            max_begin,
            unmatched_count,
            len,
            version,
        }
    }

    pub(crate) fn rebuild(
        &mut self,
        streams: Vec<RowStream>,
        order: &RowOrder,
        resolved_stages: &[String],
        primary_lane: &str,
        budget: Option<&MemoryBudget>,
    ) {
        if self.matches(&streams, order, resolved_stages, primary_lane) {
            return;
        }
        let next_version = self.version.wrapping_add(1);
        // Release the previous RowIndex reservation before reserving for the replacement.
        self.clear();
        *self = Self::build_with_version(
            streams,
            order,
            resolved_stages,
            primary_lane,
            budget,
            next_version,
        );
    }

    pub(crate) fn clear_visible_index(&mut self) {
        self.visible_index = None;
    }

    pub(crate) fn sync_visible_index(
        &mut self,
        visibility: &StageVisibility,
        primary_lane: &str,
        budget: Option<&MemoryBudget>,
    ) {
        if visibility.is_empty() || self.len == 0 {
            self.visible_index = None;
            return;
        }
        let available_bytes = budget.map_or(u64::MAX, |b| b.limit().saturating_sub(b.used()));
        if let Some(cached) = &self.visible_index
            && cached.visibility == *visibility
            && cached.primary_lane == primary_lane
        {
            match &cached.state {
                FilteredIndexState::Ready(_) => return,
                FilteredIndexState::Refused {
                    available_bytes: refused_at,
                } if available_bytes <= *refused_at => return,
                FilteredIndexState::Refused { .. } => {}
            }
        }
        // Release any prior filtered RowIndex reservation before reserving for the replacement.
        self.visible_index = None;
        let available_after_release =
            budget.map_or(u64::MAX, |b| b.limit().saturating_sub(b.used()));
        let filter = StageAnchorFilter {
            visibility,
            primary_lane,
        };
        let index = if self.uses_direct_stream() {
            let records = self.streams[0].generator.transactions();
            RowIndex::build_filtered_sync(records, Some(filter), budget)
        } else {
            let table = MergedTable {
                streams: &self.streams,
                entries: &self.entries,
            };
            RowIndex::build_filtered_sync(&table, Some(filter), budget)
        };
        let state = match index {
            Some(index) => FilteredIndexState::Ready(index),
            None => FilteredIndexState::Refused {
                available_bytes: available_after_release,
            },
        };
        self.visible_index = Some(FilteredIndex {
            visibility: visibility.clone(),
            primary_lane: primary_lane.to_owned(),
            state,
        });
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn uses_direct_stream(&self) -> bool {
        self.streams.len() == 1 && self.order.is_start()
    }
}

enum RowStorage<'a> {
    Borrowed(&'a MergedRows),
    Owned(Box<MergedRows>),
}

impl RowStorage<'_> {
    fn as_ref(&self) -> &MergedRows {
        match self {
            Self::Borrowed(rows) => rows,
            Self::Owned(rows) => rows,
        }
    }
}

pub struct RowSet<'a> {
    storage: RowStorage<'a>,
}

impl<'a> RowSet<'a> {
    pub(crate) fn borrowed(merged: &'a MergedRows) -> Self {
        Self {
            storage: RowStorage::Borrowed(merged),
        }
    }

    pub(crate) fn owned(merged: MergedRows) -> Self {
        Self {
            storage: RowStorage::Owned(Box::new(merged)),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_generators(generators: &[Arc<LoadedGenerator>]) -> Self {
        Self::from_generators_with_order(generators, &RowOrder::default(), &[])
    }

    #[cfg(test)]
    pub(crate) fn from_generators_with_order(
        generators: &[Arc<LoadedGenerator>],
        order: &RowOrder,
        resolved_stages: &[String],
    ) -> Self {
        let streams = generators
            .iter()
            .enumerate()
            .map(|(i, g)| RowStream {
                source_index: i,
                trace: TraceId::A,
                generator: Arc::clone(g),
            })
            .collect();
        Self::owned(MergedRows::build(
            streams,
            order,
            resolved_stages,
            "0",
            None,
        ))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.storage.as_ref().len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.storage.as_ref().len == 0
    }

    #[must_use]
    pub fn unmatched_count(&self) -> usize {
        self.storage.as_ref().unmatched_count
    }

    #[must_use]
    pub fn sort_meta(&self, row: usize) -> Option<RowSortMeta> {
        self.storage.as_ref().sort_meta.get(row).copied()
    }

    #[must_use]
    pub fn sort_key(&self, row: usize) -> Option<u64> {
        let inner = self.storage.as_ref();
        if let Some(meta) = inner.sort_meta.get(row) {
            Some(meta.sort_key)
        } else {
            self.get(row).map(|(_, tx)| tx.begin)
        }
    }

    #[must_use]
    pub fn generators(&self) -> &[Arc<LoadedGenerator>] {
        &self.storage.as_ref().generators
    }

    #[must_use]
    pub(crate) fn generator_ptrs(&self) -> &[usize] {
        &self.storage.as_ref().generator_ptrs
    }

    #[must_use]
    pub fn streams(&self) -> &[RowStream] {
        &self.storage.as_ref().streams
    }

    #[must_use]
    pub fn row_of(&self, trace: TraceId, generator: TrackRef, id: TransactionRef) -> Option<usize> {
        let inner = self.storage.as_ref();
        let (s, stream) = inner
            .streams
            .iter()
            .enumerate()
            .find(|(_, st)| st.trace == trace && st.generator.generator() == generator)?;
        let ordinal = stream.generator.transaction_ordinal(id)?;
        if inner.uses_direct_stream() {
            Some(ordinal)
        } else {
            let row = inner.reverse[s][ordinal];
            (row != HIDDEN_ROW).then_some(row as usize)
        }
    }

    /// Returns `(source_index, &Transaction)` for the given merged row.
    #[must_use]
    pub fn get(&self, row: usize) -> Option<(usize, &Transaction)> {
        let inner = self.storage.as_ref();
        if row >= inner.len {
            return None;
        }
        if inner.uses_direct_stream() {
            let st = &inner.streams[0];
            Some((st.source_index, &st.generator.transactions()[row]))
        } else {
            let entry = inner.entries[row];
            let st = &inner.streams[entry.stream as usize];
            Some((
                st.source_index,
                &st.generator.transactions()[entry.ordinal as usize],
            ))
        }
    }

    /// Returns `(source_index, trace, &Transaction)` for the given merged row.
    #[must_use]
    pub fn get_with_trace(&self, row: usize) -> Option<(usize, TraceId, &Transaction)> {
        let inner = self.storage.as_ref();
        if row >= inner.len {
            return None;
        }
        if inner.uses_direct_stream() {
            let st = &inner.streams[0];
            Some((st.source_index, st.trace, &st.generator.transactions()[row]))
        } else {
            let entry = inner.entries[row];
            let st = &inner.streams[entry.stream as usize];
            Some((
                st.source_index,
                st.trace,
                &st.generator.transactions()[entry.ordinal as usize],
            ))
        }
    }
}

impl RowSet<'_> {
    pub(super) fn sample_overlapping(
        &self,
        view: Viewport,
        limit: usize,
    ) -> Vec<(usize, &Transaction)> {
        let inner = self.storage.as_ref();
        let mut out = Vec::new();
        if inner.len == 0 || limit == 0 {
            return out;
        }
        out.reserve(limit.min(inner.len));
        if inner.uses_direct_stream() {
            let st = &inner.streams[0];
            let records = st.generator.transactions();
            if let Some(index) = &st.generator.row_index {
                index.visit_overlapping(records, view, limit, &mut |row| {
                    out.push((st.source_index, &records[row]));
                });
            } else {
                let start_u64 = view.start.max(0.0).floor() as u64;
                let end_u64 = view.end.max(0.0).ceil() as u64;
                st.generator
                    .visit_window_ordinals(start_u64, end_u64, |_, tx| {
                        if tx.begin as f64 <= view.end && tx.end as f64 >= view.start {
                            out.push((st.source_index, tx));
                        }
                        out.len() < limit
                    })
                    .expect("ordered viewport");
            }
            return out;
        }
        let table = MergedTable {
            streams: &inner.streams,
            entries: &inner.entries,
        };
        if let Some(index) = &inner.index {
            index.visit_overlapping(&table, view, limit, &mut |row| {
                let entry = inner.entries[row];
                let st = &inner.streams[entry.stream as usize];
                let tx = &st.generator.transactions()[entry.ordinal as usize];
                out.push((st.source_index, tx));
            });
            return out;
        }
        let start_u64 = view.start.max(0.0).floor() as u64;
        let end_u64 = view.end.max(0.0).ceil() as u64;
        for (s, st) in inner.streams.iter().enumerate() {
            if out.len() >= limit {
                break;
            }
            let rev = &inner.reverse[s];
            st.generator
                .visit_window_ordinals(start_u64, end_u64, |ord, tx| {
                    if rev[ord] != HIDDEN_ROW
                        && tx.begin as f64 <= view.end
                        && tx.end as f64 >= view.start
                    {
                        out.push((st.source_index, tx));
                    }
                    out.len() < limit
                })
                .expect("ordered viewport");
        }
        out
    }

    pub(super) fn presence(&self, map: &PixelMap) -> PresenceMask {
        let inner = self.storage.as_ref();
        let mut mask = map.mask();
        if inner.len == 0 {
            return mask;
        }
        if inner.uses_direct_stream() {
            let g = &inner.streams[0].generator;
            if let Some(index) = &g.row_index {
                index.raster(g.transactions(), 0, map, &mut mask);
            } else {
                raster_source(g.transactions(), 0, map, &mut mask);
            }
            return mask;
        }
        let table = MergedTable {
            streams: &inner.streams,
            entries: &inner.entries,
        };
        if let Some(index) = &inner.index {
            index.raster(&table, 0, map, &mut mask);
        } else {
            raster_source(&table, 0, map, &mut mask);
        }
        mask
    }

    pub(super) fn visible_presence(
        &self,
        map: &PixelMap,
        stage_filter: StageAnchorFilter<'_>,
    ) -> PresenceMask {
        let inner = self.storage.as_ref();
        let mut mask = map.mask();
        if inner.len == 0 {
            return mask;
        }
        let cached = inner.visible_index.as_ref().and_then(|c| {
            if c.visibility == *stage_filter.visibility
                && c.primary_lane == stage_filter.primary_lane
                && let FilteredIndexState::Ready(index) = &c.state
            {
                Some(index)
            } else {
                None
            }
        });
        if inner.uses_direct_stream() {
            let records = inner.streams[0].generator.transactions();
            if let Some(index) = cached {
                index.raster_filtered(records, 0, Some(stage_filter), map, &mut mask);
            } else {
                raster_source_filtered(records, 0, Some(stage_filter), map, &mut mask);
            }
            return mask;
        }
        let table = MergedTable {
            streams: &inner.streams,
            entries: &inner.entries,
        };
        if let Some(index) = cached {
            index.raster_filtered(&table, 0, Some(stage_filter), map, &mut mask);
        } else {
            raster_source_filtered(&table, 0, Some(stage_filter), map, &mut mask);
        }
        mask
    }

    pub(super) fn aborted_presence(&self, map: &PixelMap) -> PresenceMask {
        let inner = self.storage.as_ref();
        let mut mask = map.aborted_mask();
        if inner.len == 0 {
            return mask;
        }
        if inner.uses_direct_stream() {
            let g = &inner.streams[0].generator;
            if let Some(index) = &g.row_index {
                index.raster_aborted(g.transactions(), 0, map, &mut mask);
            } else {
                for (row, tx) in g.transactions().iter().enumerate() {
                    if tx.status == Status::Aborted {
                        map.aborted(row, &mut mask);
                    }
                }
            }
            return mask;
        }
        let table = MergedTable {
            streams: &inner.streams,
            entries: &inner.entries,
        };
        if let Some(index) = &inner.index {
            index.raster_aborted(&table, 0, map, &mut mask);
        } else {
            for row in 0..table.len() {
                if table.get(row).status == Status::Aborted {
                    map.aborted(row, &mut mask);
                }
            }
        }
        mask
    }

    pub(super) fn activity(
        &self,
        view: Viewport,
        visible: Range<f64>,
        anchor: f64,
        follow_sort_key: bool,
        stage_filter: Option<StageAnchorFilter<'_>>,
    ) -> Activity {
        let inner = self.storage.as_ref();
        if inner.len == 0 {
            return Activity::default();
        }
        let center = visible.start.midpoint(visible.end);
        let earlier = (inner.min_begin as f64) < view.start;
        let later = (inner.max_begin as f64) > view.end;
        let sort_meta =
            (follow_sort_key && !inner.sort_meta.is_empty()).then_some(inner.sort_meta.as_slice());

        if inner.uses_direct_stream() {
            let g = &inner.streams[0].generator;
            let records = g.transactions();
            if let Some(index) = &g.row_index {
                let mut result = index.activity_mapped(
                    records,
                    |i| i,
                    None,
                    stage_filter,
                    view,
                    visible,
                    anchor,
                    center,
                );
                result.earlier = earlier;
                result.later = later;
                return result;
            }
            let mut result = Activity {
                earlier,
                later,
                ..Activity::default()
            };
            let mut best = (u8::MAX, f64::INFINITY, f64::INFINITY, usize::MAX);
            if g.stages_exceed_owner() {
                for (row, tx) in records.iter().enumerate() {
                    record_activity(
                        row,
                        tx,
                        None,
                        stage_filter,
                        view,
                        &visible,
                        anchor,
                        &mut result,
                        &mut best,
                    );
                }
            } else {
                g.visit_window_ordinals(
                    view.start.max(0.0).floor() as u64,
                    view.end.max(0.0).ceil() as u64,
                    |row, tx| {
                        record_activity(
                            row,
                            tx,
                            None,
                            stage_filter,
                            view,
                            &visible,
                            anchor,
                            &mut result,
                            &mut best,
                        );
                        true
                    },
                )
                .expect("ordered viewport");
            }
            result.target = (best.3 != usize::MAX).then_some(best.3);
            return result;
        }

        let table = MergedTable {
            streams: &inner.streams,
            entries: &inner.entries,
        };
        if let Some(index) = &inner.index {
            let mut result = index.activity_mapped(
                &table,
                |i| i,
                sort_meta,
                stage_filter,
                view,
                visible,
                anchor,
                center,
            );
            result.earlier = earlier;
            result.later = later;
            return result;
        }

        // Unindexed fallback (e.g. tight memory budget): query each stream's window ordinals
        // and map ordinals back to merged rows via reverse index.
        let mut result = Activity {
            earlier,
            later,
            ..Activity::default()
        };
        let mut best = (u8::MAX, f64::INFINITY, f64::INFINITY, usize::MAX);
        let start_u64 = view.start.max(0.0).floor() as u64;
        let end_u64 = view.end.max(0.0).ceil() as u64;
        for (s, stream) in inner.streams.iter().enumerate() {
            let rev = &inner.reverse[s];
            let mut visit = |ordinal: usize, tx: &Transaction| {
                let row = rev[ordinal];
                if row == HIDDEN_ROW {
                    return true;
                }
                let row = row as usize;
                let meta = sort_meta.map(|m| m[row]);
                record_activity(
                    row,
                    tx,
                    meta,
                    stage_filter,
                    view,
                    &visible,
                    anchor,
                    &mut result,
                    &mut best,
                );
                true
            };
            if stream.generator.stages_exceed_owner() {
                for (ordinal, tx) in stream.generator.transactions().iter().enumerate() {
                    visit(ordinal, tx);
                }
            } else {
                stream
                    .generator
                    .visit_window_ordinals(start_u64, end_u64, visit)
                    .expect("ordered viewport");
            }
        }
        result.target = (best.3 != usize::MAX).then_some(best.3);
        result
    }

    /// Exact activity restricted to a single source in merged row coordinates (used in Solo/Dim
    /// mode so `FollowActivity` and offscreen reveal pills track the focused source).
    pub(super) fn activity_for_source(
        &self,
        source_index: usize,
        view: Viewport,
        visible: Range<f64>,
        anchor: f64,
        follow_sort_key: bool,
        stage_filter: Option<StageAnchorFilter<'_>>,
    ) -> Activity {
        let inner = self.storage.as_ref();
        if inner.len == 0 {
            return Activity::default();
        }
        if inner.uses_direct_stream() {
            return if inner.streams[0].source_index == source_index {
                self.activity(view, visible, anchor, follow_sort_key, stage_filter)
            } else {
                Activity::default()
            };
        }
        let center = visible.start.midpoint(visible.end);
        let sort_meta =
            (follow_sort_key && !inner.sort_meta.is_empty()).then_some(inner.sort_meta.as_slice());
        let mut combined = Activity::default();
        let mut best_target: Option<(u8, f64, f64, usize)> = None;
        let start_u64 = view.start.max(0.0).floor() as u64;
        let end_u64 = view.end.max(0.0).ceil() as u64;
        for (s, stream) in inner.streams.iter().enumerate() {
            if stream.source_index != source_index {
                continue;
            }
            let Some((min_b, max_b)) = inner.stream_begin_bounds[s] else {
                continue;
            };
            combined.earlier |= (min_b as f64) < view.start;
            combined.later |= (max_b as f64) > view.end;
            let g = &stream.generator;
            let records = g.transactions();
            let rev = &inner.reverse[s];
            if inner.reverse_monotonic[s]
                && let Some(index) = &g.row_index
            {
                let part = index.activity_mapped(
                    records,
                    |ord| rev[ord] as usize,
                    sort_meta,
                    stage_filter,
                    view,
                    visible.clone(),
                    anchor,
                    center,
                );
                combined.above += part.above;
                combined.visible += part.visible;
                combined.below += part.below;
                if let Some(a) = part.nearest_above {
                    combined.nearest_above = Some(combined.nearest_above.map_or(a, |p| p.max(a)));
                }
                if let Some(b) = part.nearest_below {
                    combined.nearest_below = Some(combined.nearest_below.map_or(b, |p| p.min(b)));
                }
                if let Some(row) = part.target
                    && let Some((_, tx)) = self.get(row)
                {
                    let score = activity_target_score(
                        row,
                        tx,
                        sort_meta.map(|m| m[row]),
                        stage_filter,
                        anchor,
                        center,
                    );
                    if best_target.is_none_or(|prev| score < prev) {
                        best_target = Some(score);
                    }
                }
            } else {
                let mut best =
                    best_target.unwrap_or((u8::MAX, f64::INFINITY, f64::INFINITY, usize::MAX));
                let mut visit = |ordinal: usize, tx: &Transaction| {
                    let row = rev[ordinal];
                    if row == HIDDEN_ROW {
                        return true;
                    }
                    let row = row as usize;
                    let meta = sort_meta.map(|m| m[row]);
                    record_activity(
                        row,
                        tx,
                        meta,
                        stage_filter,
                        view,
                        &visible,
                        anchor,
                        &mut combined,
                        &mut best,
                    );
                    true
                };
                if g.stages_exceed_owner() {
                    for (ordinal, tx) in records.iter().enumerate() {
                        visit(ordinal, tx);
                    }
                } else {
                    g.visit_window_ordinals(start_u64, end_u64, visit)
                        .expect("ordered viewport");
                }
                if best.3 != usize::MAX {
                    best_target = Some(best);
                }
            }
        }
        combined.target = best_target.map(|(_, _, _, row)| row);
        combined
    }
}

#[expect(clippy::too_many_arguments, reason = "internal activity accumulator")]
fn record_activity(
    row: usize,
    tx: &Transaction,
    meta: Option<RowSortMeta>,
    stage_filter: Option<StageAnchorFilter<'_>>,
    view: Viewport,
    visible: &Range<f64>,
    anchor: f64,
    result: &mut Activity,
    best: &mut (u8, f64, f64, usize),
) {
    if !tx_overlaps_view(tx, view) {
        return;
    }
    let center = visible.start.midpoint(visible.end);
    if row as f64 + 1.0 <= visible.start {
        result.above += 1;
        result.nearest_above = Some(result.nearest_above.map_or(row, |prev| prev.max(row)));
    } else if row as f64 >= visible.end {
        result.below += 1;
        result.nearest_below = Some(result.nearest_below.map_or(row, |prev| prev.min(row)));
    } else {
        result.visible += 1;
    }
    let score = activity_target_score(row, tx, meta, stage_filter, anchor, center);
    if score < *best {
        *best = score;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::Rect,
        pipeline::{raster::oracle, rows::RowView},
    };
    use volna_trace::data::transactions::{SpanKind, TransactionStage};

    fn make_gen(g: u32, specs: &[(u64, u64, u64, bool)]) -> Arc<LoadedGenerator> {
        let records = specs
            .iter()
            .map(|&(id, begin, end, aborted)| Transaction {
                id: TransactionRef(id),
                generator: TrackRef(g),
                begin,
                end,
                status: if aborted { Status::Aborted } else { Status::Ok },
                kind: SpanKind::Unspecified,
                parent: None,
                attributes: vec![],
                events: vec![],
                stages: vec![TransactionStage {
                    id: TransactionRef(((u64::from(g) + 1) << 48) | ((id + 1) << 16)),
                    generator: None,
                    name: "stage".into(),
                    lane: "0".into(),
                    begin,
                    end: Some(end.max(begin + 1)),
                    attributes: vec![],
                }],
            })
            .collect();
        Arc::new(LoadedGenerator::new(TrackRef(g), records, Default::default(), vec![]).unwrap())
    }

    // User scenario: multiple peer warp generators with overlapping timestamps and ties are
    // merged into one timeline; rows must be non-decreasing in begin, tie-broken deterministically
    // by source order then ordinal, and reverse-indexed for O(1) selection/reveal lookup.
    #[test]
    fn k_way_merge_orders_by_begin_breaks_ties_by_source_and_round_trips_row_of() {
        let g0 = make_gen(
            10,
            &[(1, 5, 10, false), (2, 10, 14, false), (3, 20, 25, true)],
        );
        let g1 = make_gen(
            20,
            &[(1, 2, 6, false), (2, 10, 15, true), (3, 10, 18, false)],
        );
        let g2 = make_gen(30, &[(1, 10, 12, false), (2, 15, 22, false)]);
        let set = RowSet::from_generators(&[g0, g1, g2]);
        assert_eq!(set.len(), 8);

        let order: Vec<(usize, u64, u64)> = (0..set.len())
            .map(|r| {
                let (src, tx) = set.get(r).unwrap();
                (src, tx.begin, tx.id.0)
            })
            .collect();
        assert_eq!(
            order,
            [
                (1, 2, 1),
                (0, 5, 1),
                (0, 10, 2),
                (1, 10, 2),
                (1, 10, 3),
                (2, 10, 1),
                (2, 15, 2),
                (0, 20, 3),
            ]
        );

        for r in 0..set.len() {
            let (_, trace, tx) = set.get_with_trace(r).unwrap();
            assert_eq!(set.row_of(trace, tx.generator, tx.id), Some(r));
        }
        assert_eq!(
            set.row_of(TraceId::A, TrackRef(999), TransactionRef(1)),
            None
        );
    }

    // User scenario: an engineer pans and zooms a merged multi-generator pipeline panel in both
    // overview and detailed modes (with and without RowIndex budget) and expects exact presence,
    // aborted ticks, and FollowActivity counts matching a brute-force oracle over merged rows.
    #[test]
    fn merged_queries_match_brute_force_oracle_with_and_without_row_index() {
        let generators: Vec<_> = [0usize, 19, 0, 23, 17]
            .into_iter()
            .enumerate()
            .map(|(g, count)| {
                let specs: Vec<_> = (0..count as u64)
                    .map(|row| {
                        let begin = row * 3 + (g as u64 % 3);
                        (row, begin, begin + 8 + (row % 5), row % 3 == 0)
                    })
                    .collect();
                make_gen(g as u32, &specs)
            })
            .collect();

        let streams: Vec<_> = generators
            .iter()
            .enumerate()
            .map(|(i, g)| RowStream {
                source_index: i,
                trace: TraceId::A,
                generator: Arc::clone(g),
            })
            .collect();
        let default_order = RowOrder::default();
        let indexed = MergedRows::build(streams.clone(), &default_order, &[], "0", None);
        assert!(indexed.index.is_some());
        let zero_budget = MemoryBudget::new(0);
        let unindexed = MergedRows::build(streams, &default_order, &[], "0", Some(&zero_budget));
        assert!(unindexed.index.is_none());

        let merged_records: Vec<Transaction> = (0..indexed.len)
            .map(|r| RowSet::borrowed(&indexed).get(r).unwrap().1.clone())
            .collect();
        for pair in merged_records.windows(2) {
            assert!(pair[0].begin <= pair[1].begin);
        }

        let fields = |a: Activity| {
            (
                a.above,
                a.visible,
                a.below,
                a.nearest_above,
                a.nearest_below,
                a.target,
                a.earlier,
                a.later,
            )
        };

        for rows in [RowSet::borrowed(&indexed), RowSet::borrowed(&unindexed)] {
            for top in [0.25, 17.5, 31.75] {
                let view = Viewport {
                    start: 11.25,
                    end: 52.75,
                };
                let map = PixelMap::new(
                    Rect::from_xywh(0.37, 0.23, 18.0, 4.0),
                    view,
                    RowView { top, row_px: 0.3 },
                    1.25,
                );
                assert_eq!(rows.presence(&map), oracle(&merged_records, 0, &map));

                let mut expected_aborted = map.aborted_mask();
                for (row, tx) in merged_records.iter().enumerate() {
                    if tx.status == Status::Aborted {
                        map.aborted(row, &mut expected_aborted);
                    }
                }
                assert_eq!(rows.aborted_presence(&map), expected_aborted);

                let visible = top..top + 12.0;
                let anchor = 30.0;
                let mut expected = Activity {
                    earlier: merged_records
                        .first()
                        .is_some_and(|tx| (tx.begin as f64) < view.start),
                    later: merged_records
                        .last()
                        .is_some_and(|tx| (tx.begin as f64) > view.end),
                    ..Activity::default()
                };
                let mut best = (u8::MAX, f64::INFINITY, f64::INFINITY, usize::MAX);
                for (row, tx) in merged_records.iter().enumerate() {
                    record_activity(
                        row,
                        tx,
                        None,
                        None,
                        view,
                        &visible,
                        anchor,
                        &mut expected,
                        &mut best,
                    );
                }
                expected.target = (best.3 != usize::MAX).then_some(best.3);
                assert_eq!(
                    fields(rows.activity(view, visible.clone(), anchor, false, None)),
                    fields(expected)
                );

                // Solo (Dim) per-source activity in merged row coordinates must also match oracle.
                for target_src in 0..generators.len() {
                    let mut src_expected = Activity::default();
                    let mut src_best = (u8::MAX, f64::INFINITY, f64::INFINITY, usize::MAX);
                    let mut first_begin = None;
                    let mut last_begin = None;
                    for row in 0..rows.len() {
                        let (src, tx) = rows.get(row).unwrap();
                        if src != target_src {
                            continue;
                        }
                        first_begin.get_or_insert(tx.begin);
                        last_begin = Some(tx.begin);
                        record_activity(
                            row,
                            tx,
                            None,
                            None,
                            view,
                            &visible,
                            anchor,
                            &mut src_expected,
                            &mut src_best,
                        );
                    }
                    src_expected.earlier = first_begin.is_some_and(|b| (b as f64) < view.start);
                    src_expected.later = last_begin.is_some_and(|b| (b as f64) > view.end);
                    src_expected.target = (src_best.3 != usize::MAX).then_some(src_best.3);
                    assert_eq!(
                        fields(rows.activity_for_source(
                            target_src,
                            view,
                            visible.clone(),
                            anchor,
                            false,
                            None,
                        )),
                        fields(src_expected),
                        "source {target_src} solo activity mismatch"
                    );
                }
            }
        }
    }

    // User scenario: an engineer toggles visibility on a merged panel under a tight memory budget;
    // rebuilding MergedRows must release the previous RowIndex reservation before reserving the new
    // one so the replacement RowIndex still fits within budget.
    #[test]
    fn rebuild_releases_previous_row_index_reservation_before_allocating_replacement() {
        let specs: Vec<_> = (0..64u64).map(|r| (r, r * 4, r * 4 + 10, false)).collect();
        let g0 = make_gen(1, &specs);
        let g1 = make_gen(2, &specs);
        let g2 = make_gen(3, &specs);
        let s01 = vec![
            RowStream {
                source_index: 0,
                trace: TraceId::A,
                generator: Arc::clone(&g0),
            },
            RowStream {
                source_index: 1,
                trace: TraceId::A,
                generator: Arc::clone(&g1),
            },
        ];
        let s02 = vec![
            RowStream {
                source_index: 0,
                trace: TraceId::A,
                generator: g0,
            },
            RowStream {
                source_index: 2,
                trace: TraceId::A,
                generator: g2,
            },
        ];
        let default_order = RowOrder::default();
        let probe = MemoryBudget::new(1024 * 1024);
        let initial = MergedRows::build(s01.clone(), &default_order, &[], "0", Some(&probe));
        assert!(initial.index.is_some());
        let needed = probe.used();
        drop(initial);
        assert_eq!(probe.used(), 0);

        // Give budget room for only ONE RowIndex (1.5x needed < 2x needed).
        let tight = MemoryBudget::new(needed + needed / 2);
        let mut merged = MergedRows::build(s01, &default_order, &[], "0", Some(&tight));
        assert!(merged.index.is_some());
        merged.rebuild(s02, &default_order, &[], "0", Some(&tight));
        assert!(
            merged.index.is_some(),
            "rebuild should free old RowIndex reservation before building replacement"
        );
        merged.clear();
        assert_eq!(tight.used(), 0);
    }

    struct StagedTxSpec<'a> {
        id: u64,
        begin: u64,
        end: u64,
        aborted: bool,
        stages: Vec<(&'a str, u64, u64)>,
    }

    fn make_staged_gen(g: u32, specs: &[StagedTxSpec<'_>]) -> Arc<LoadedGenerator> {
        let records = specs
            .iter()
            .map(|spec| Transaction {
                id: TransactionRef(spec.id),
                generator: TrackRef(g),
                begin: spec.begin,
                end: spec.end,
                status: if spec.aborted {
                    Status::Aborted
                } else {
                    Status::Ok
                },
                kind: SpanKind::Unspecified,
                parent: None,
                attributes: vec![],
                events: vec![],
                stages: spec
                    .stages
                    .iter()
                    .enumerate()
                    .map(|(k, &(name, sb, se))| TransactionStage {
                        id: TransactionRef(
                            ((u64::from(g) + 1) << 48) | ((spec.id + 1) << 16) | k as u64,
                        ),
                        generator: None,
                        name: name.into(),
                        lane: "0".into(),
                        begin: sb,
                        end: Some(se),
                        attributes: vec![],
                    })
                    .collect(),
            })
            .collect();
        Arc::new(LoadedGenerator::new(TrackRef(g), records, Default::default(), vec![]).unwrap())
    }

    // User scenario: two GPU warp slots each fetch a 6-instruction cache-line burst at identical
    // `tx.begin`, then interleave round-robin at `issue` and diverge into `alu` / `mem` pipes,
    // while flushed instructions abort before `issue`.
    // Requirement: `Start` order clumps by burst and suggests `issue`; `Stages(["issue"])`
    // interleaves warps by `issue.begin` and handles unmatched rows per `FallbackEnd`,
    // `FallbackStart`, and `Hide`; `FromStage("alu")` interleaves `alu` and `mem` via palette rank;
    // `End` reorders even a single-source stream by `tx.end`.
    #[test]
    fn batched_fetch_ordering_modes_unmatched_policies_and_suggestion() {
        use super::super::{
            order::{RowOrderMode, StageOrderOption, UnmatchedPolicy, detect_batch_suggestion},
            palette::StagePalette,
        };

        // Warp 0 fetches 6 instructions at t=100; Warp 1 fetches 6 instructions at t=102.
        // They interleave at `issue` (110, 111, 112, ...) except tx 5 on warp 0 and tx 5 on warp 1
        // which abort before `issue`.
        let w0_specs: Vec<StagedTxSpec<'_>> = (0..6u64)
            .map(|i| {
                if i == 5 {
                    StagedTxSpec {
                        id: i,
                        begin: 100,
                        end: 115,
                        aborted: true,
                        stages: vec![("fetch", 100, 108)],
                    }
                } else {
                    let iss = 110 + i * 2;
                    let pipe = if i % 2 == 0 { "alu" } else { "mem" };
                    let end = if pipe == "mem" { iss + 20 } else { iss + 6 };
                    StagedTxSpec {
                        id: i,
                        begin: 100,
                        end,
                        aborted: false,
                        stages: vec![
                            ("fetch", 100, 106),
                            ("issue", iss, iss + 1),
                            (pipe, iss + 1, end),
                        ],
                    }
                }
            })
            .collect();
        let w1_specs: Vec<StagedTxSpec<'_>> = (0..6u64)
            .map(|i| {
                if i == 5 {
                    StagedTxSpec {
                        id: i,
                        begin: 102,
                        end: 113,
                        aborted: true,
                        stages: vec![("fetch", 102, 109)],
                    }
                } else {
                    let iss = 111 + i * 2;
                    let pipe = if i % 2 == 0 { "mem" } else { "alu" };
                    let end = if pipe == "mem" { iss + 18 } else { iss + 5 };
                    StagedTxSpec {
                        id: i,
                        begin: 102,
                        end,
                        aborted: false,
                        stages: vec![
                            ("fetch", 102, 107),
                            ("issue", iss, iss + 1),
                            (pipe, iss + 1, end),
                        ],
                    }
                }
            })
            .collect();

        let g0 = make_staged_gen(1, &w0_specs);
        let g1 = make_staged_gen(2, &w1_specs);
        let palette = StagePalette::build(&[Arc::clone(&g0), Arc::clone(&g1)]);

        // Start order clumps all 6 warp 0 instructions before warp 1, and batch detector suggests "issue".
        let start_set = RowSet::from_generators(&[Arc::clone(&g0), Arc::clone(&g1)]);
        let start_sources: Vec<usize> = (0..start_set.len())
            .map(|r| start_set.get(r).unwrap().0)
            .collect();
        assert_eq!(start_sources, [0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1]);

        let options: Vec<StageOrderOption> = palette
            .names()
            .iter()
            .map(|name| {
                let count: u64 = [&g0, &g1]
                    .iter()
                    .filter_map(|g| {
                        g.stage_census()
                            .lanes
                            .iter()
                            .find(|l| l.lane == palette.primary_lane())
                            .and_then(|l| l.names.iter().find(|n| &n.name == name))
                            .map(|n| n.count)
                    })
                    .sum();
                StageOrderOption {
                    name: name.clone(),
                    count,
                    total_transactions: 12,
                    coverage: (count as f64 / 12.0).min(1.0) as f32,
                }
            })
            .collect();
        let samples = start_set.sample_overlapping(
            Viewport {
                start: 100.0,
                end: 140.0,
            },
            256,
        );
        assert_eq!(
            detect_batch_suggestion(&samples, &palette, &options).as_deref(),
            Some("issue")
        );

        // Order by `issue` with FallbackEnd (default): interleaves w0/w1 and places flushed rows
        // at tx.end (113, 115) alongside nearby issued instructions.
        let issue_end = RowOrder {
            mode: RowOrderMode::Stages {
                stages: vec!["issue".into()],
            },
            unmatched: UnmatchedPolicy::FallbackEnd,
            follow_sort_key: true,
        };
        let resolved = issue_end.resolved_stages(&palette);
        let issue_set = RowSet::from_generators_with_order(
            &[Arc::clone(&g0), Arc::clone(&g1)],
            &issue_end,
            resolved,
        );
        assert_eq!(issue_set.len(), 12);
        assert_eq!(issue_set.unmatched_count(), 2);
        let keys: Vec<(u64, bool, usize)> = (0..issue_set.len())
            .map(|r| {
                let meta = issue_set.sort_meta(r).unwrap();
                let (src, _) = issue_set.get(r).unwrap();
                (meta.sort_key, meta.matched, src)
            })
            .collect();
        // Verify non-decreasing sort keys and matched-before-fallback tie breaking at t=113 and t=115.
        for pair in keys.windows(2) {
            assert!(pair[0].0 <= pair[1].0);
            if pair[0].0 == pair[1].0 {
                assert!(pair[0].1 >= pair[1].1);
            }
        }
        assert_eq!(keys[0], (110, true, 0));
        assert_eq!(keys[1], (111, true, 1));
        assert_eq!(keys[2], (112, true, 0));
        assert_eq!(keys[3], (113, true, 1));
        assert_eq!(keys[4], (113, false, 1)); // flushed w1 tx5 at tx.end=113, right after matched t=113

        // FallbackStart places flushed rows at tx.begin (100, 102) ahead of issued rows (110+).
        let issue_start = RowOrder {
            unmatched: UnmatchedPolicy::FallbackStart,
            ..issue_end.clone()
        };
        let issue_start_set = RowSet::from_generators_with_order(
            &[Arc::clone(&g0), Arc::clone(&g1)],
            &issue_start,
            issue_start.resolved_stages(&palette),
        );
        assert_eq!(issue_start_set.sort_meta(0).unwrap().sort_key, 100);
        assert!(!issue_start_set.sort_meta(0).unwrap().matched);
        assert_eq!(issue_start_set.sort_meta(1).unwrap().sort_key, 102);
        assert!(!issue_start_set.sort_meta(1).unwrap().matched);

        // Hide removes the 2 flushed rows and returns None from row_of for hidden transactions.
        let issue_hide = RowOrder {
            unmatched: UnmatchedPolicy::Hide,
            ..issue_end
        };
        let issue_hide_set = RowSet::from_generators_with_order(
            &[Arc::clone(&g0), Arc::clone(&g1)],
            &issue_hide,
            issue_hide.resolved_stages(&palette),
        );
        assert_eq!(issue_hide_set.len(), 10);
        assert_eq!(issue_hide_set.unmatched_count(), 2);
        assert_eq!(
            issue_hide_set.row_of(TraceId::A, TrackRef(1), TransactionRef(5)),
            None
        );
        assert_eq!(
            issue_hide_set.row_of(TraceId::A, TrackRef(2), TransactionRef(5)),
            None
        );
        assert!(
            issue_hide_set
                .row_of(TraceId::A, TrackRef(1), TransactionRef(0))
                .is_some()
        );

        // FromStage("alu") resolves to both `alu` and `mem` (stages at or after `alu` in palette).
        let from_stage_name = palette.names()[2].clone();
        let from_order = RowOrder {
            mode: RowOrderMode::FromStage {
                stage: from_stage_name,
            },
            unmatched: UnmatchedPolicy::Hide,
            follow_sort_key: true,
        };
        let from_resolved = from_order.resolved_stages(&palette);
        assert!(from_resolved.len() >= 2);
        let from_set = RowSet::from_generators_with_order(
            &[Arc::clone(&g0), Arc::clone(&g1)],
            &from_order,
            from_resolved,
        );
        assert_eq!(from_set.len(), 10);
        for r in 0..from_set.len() {
            let meta = from_set.sort_meta(r).unwrap();
            assert!(meta.matched);
            let (_, tx) = from_set.get(r).unwrap();
            let st = &tx.stages[meta.sort_stage_index().unwrap()];
            assert!(st.name == "alu" || st.name == "mem");
        }

        // Single-source `End` order reorders long-latency `mem` instructions after shorter `alu` ones.
        let end_order = RowOrder {
            mode: RowOrderMode::End,
            ..RowOrder::default()
        };
        let single_end = RowSet::from_generators_with_order(&[Arc::clone(&g0)], &end_order, &[]);
        assert_eq!(single_end.len(), 6);
        let end_times: Vec<u64> = (0..single_end.len())
            .map(|r| single_end.get(r).unwrap().1.end)
            .collect();
        assert!(end_times.windows(2).all(|w| w[0] <= w[1]));
        // Flushed tx5 (end=115) and fast alu tx0 (end=116) precede slow mem tx1 (end=132).
        assert_eq!(single_end.get(0).unwrap().1.id, TransactionRef(5));
        assert_eq!(single_end.get(1).unwrap().1.id, TransactionRef(0));
    }

    // User scenario: an engineer reorders a merged pipeline (including out-of-order completion
    // where a single stream's reverse index is non-monotonic and rows are hidden) and navigates
    // with overview presence, FollowActivity (both lifetime and sort-key anchored), and Solo/Dim.
    // Requirement: order-agnostic `RowIndex` matches a brute-force oracle identically across all
    // viewports, both with and without memory budget.
    #[test]
    fn non_monotonic_stage_and_end_orders_match_oracle_with_and_without_row_index() {
        use super::super::order::{RowOrderMode, UnmatchedPolicy};

        // Construct two streams whose `issue` and `end` orderings are non-monotonic within each
        // stream (simulating out-of-order issue / variable latency) plus aborted unmatched rows.
        let build_stream = |g: u32, offset: u64| {
            let specs: Vec<StagedTxSpec<'_>> = (0..40u64)
                .map(|i| {
                    let begin = (i / 5) * 20 + offset;
                    let aborted = i % 7 == 0;
                    if aborted {
                        StagedTxSpec {
                            id: i,
                            begin,
                            end: begin + 12 + (i % 4),
                            aborted: true,
                            stages: vec![("fetch", begin, begin + 6)],
                        }
                    } else {
                        // Reverse within each 5-tx group so per-stream order is non-monotonic.
                        let ooo = (4 - (i % 5)) * 3;
                        let issue = begin + 6 + ooo + u64::from(g);
                        let end = issue + 5 + ((i * 7) % 19);
                        StagedTxSpec {
                            id: i,
                            begin,
                            end,
                            aborted: false,
                            stages: vec![
                                ("fetch", begin, begin + 4),
                                ("issue", issue, issue + 2),
                                ("exec", issue + 2, end),
                            ],
                        }
                    }
                })
                .collect();
            make_staged_gen(g, &specs)
        };

        let g0 = build_stream(1, 0);
        let g1 = build_stream(2, 5);
        let streams = vec![
            RowStream {
                source_index: 0,
                trace: TraceId::A,
                generator: Arc::clone(&g0),
            },
            RowStream {
                source_index: 1,
                trace: TraceId::A,
                generator: Arc::clone(&g1),
            },
        ];

        let orders = [
            (
                RowOrder {
                    mode: RowOrderMode::Stages {
                        stages: vec!["issue".into()],
                    },
                    unmatched: UnmatchedPolicy::FallbackEnd,
                    follow_sort_key: true,
                },
                vec!["issue".to_owned()],
            ),
            (
                RowOrder {
                    mode: RowOrderMode::Stages {
                        stages: vec!["issue".into()],
                    },
                    unmatched: UnmatchedPolicy::Hide,
                    follow_sort_key: true,
                },
                vec!["issue".to_owned()],
            ),
            (
                RowOrder {
                    mode: RowOrderMode::End,
                    unmatched: UnmatchedPolicy::FallbackEnd,
                    follow_sort_key: true,
                },
                vec![],
            ),
        ];

        let fields = |a: Activity| {
            (
                a.above,
                a.visible,
                a.below,
                a.nearest_above,
                a.nearest_below,
                a.target,
                a.earlier,
                a.later,
            )
        };

        let zero_budget = MemoryBudget::new(0);
        for (order, resolved) in orders {
            let indexed = MergedRows::build(streams.clone(), &order, &resolved, "0", None);
            assert!(indexed.index.is_some());
            assert!(indexed.reverse_monotonic.iter().any(|&m| !m));
            let unindexed =
                MergedRows::build(streams.clone(), &order, &resolved, "0", Some(&zero_budget));
            assert!(unindexed.index.is_none());

            let ordered_records: Vec<Transaction> = (0..indexed.len)
                .map(|r| RowSet::borrowed(&indexed).get(r).unwrap().1.clone())
                .collect();

            for follow_sort_key in [true, false] {
                for rows in [RowSet::borrowed(&indexed), RowSet::borrowed(&unindexed)] {
                    for (start, end, top) in [
                        (18.0, 58.0, 4.25),
                        (65.0, 115.0, 22.5),
                        (130.0, 175.0, 45.0),
                    ] {
                        let view = Viewport { start, end };
                        let map = PixelMap::new(
                            Rect::from_xywh(0.37, 0.23, 18.0, 4.0),
                            view,
                            RowView { top, row_px: 0.3 },
                            1.25,
                        );
                        assert_eq!(rows.presence(&map), oracle(&ordered_records, 0, &map));

                        let visible = top..top + 14.0;
                        let anchor = start + (end - start) * 0.5;

                        let mut expected = Activity::default();
                        let mut best = (u8::MAX, f64::INFINITY, f64::INFINITY, usize::MAX);
                        let mut min_b = u64::MAX;
                        let mut max_b = 0u64;
                        for (row, tx) in ordered_records.iter().enumerate() {
                            min_b = min_b.min(tx.begin);
                            max_b = max_b.max(tx.begin);
                            let meta = follow_sort_key.then(|| rows.sort_meta(row)).flatten();
                            record_activity(
                                row,
                                tx,
                                meta,
                                None,
                                view,
                                &visible,
                                anchor,
                                &mut expected,
                                &mut best,
                            );
                        }
                        expected.earlier =
                            !ordered_records.is_empty() && (min_b as f64) < view.start;
                        expected.later = !ordered_records.is_empty() && (max_b as f64) > view.end;
                        expected.target = (best.3 != usize::MAX).then_some(best.3);
                        assert_eq!(
                            fields(rows.activity(
                                view,
                                visible.clone(),
                                anchor,
                                follow_sort_key,
                                None,
                            )),
                            fields(expected),
                            "activity mismatch for {order:?} follow_sort_key={follow_sort_key}"
                        );

                        for target_src in 0..2 {
                            let mut src_expected = Activity::default();
                            let mut src_best = (u8::MAX, f64::INFINITY, f64::INFINITY, usize::MAX);
                            let mut s_min = u64::MAX;
                            let mut s_max = 0u64;
                            let mut s_any = false;
                            for row in 0..rows.len() {
                                let (src, tx) = rows.get(row).unwrap();
                                if src != target_src {
                                    continue;
                                }
                                s_min = s_min.min(tx.begin);
                                s_max = s_max.max(tx.begin);
                                s_any = true;
                                let meta = follow_sort_key.then(|| rows.sort_meta(row)).flatten();
                                record_activity(
                                    row,
                                    tx,
                                    meta,
                                    None,
                                    view,
                                    &visible,
                                    anchor,
                                    &mut src_expected,
                                    &mut src_best,
                                );
                            }
                            src_expected.earlier = s_any && (s_min as f64) < view.start;
                            src_expected.later = s_any && (s_max as f64) > view.end;
                            src_expected.target = (src_best.3 != usize::MAX).then_some(src_best.3);
                            assert_eq!(
                                fields(rows.activity_for_source(
                                    target_src,
                                    view,
                                    visible.clone(),
                                    anchor,
                                    follow_sort_key,
                                    None,
                                )),
                                fields(src_expected),
                                "solo activity mismatch for source {target_src} {order:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    // User scenario: an engineer hides early primary-lane stages (`fetch`, then `fetch` + `issue`,
    // then all primary-lane stages) on single-source, merged, Solo/Dim, and stage-ordered panels
    // (including when the sorted stage itself is hidden).
    // Requirement: `FollowActivity` anchors on the start of the first visible primary-lane stage
    // (ignoring hidden stages), falls back cleanly to normal timestamps when all stages are hidden,
    // and matches a brute-force oracle identically with and without `RowIndex`.
    #[test]
    fn hidden_primary_stages_anchor_follow_activity_matching_oracle_with_and_without_row_index() {
        use super::super::{
            order::{RowOrderMode, UnmatchedPolicy},
            stages::{StageAnchorFilter, StageVisibility},
        };

        // Construct transactions sharing batched `fetch` at t=0..100 while `issue` and `exec`
        // spread across t=100..260, plus rows that abort during `fetch` (all stages hidden once
        // `fetch` is hidden).
        let build_stream = |g: u32, phase: u64| {
            let specs: Vec<StagedTxSpec<'_>> = (0..24u64)
                .map(|i| {
                    let batch_begin = (i / 6) * 20;
                    if i % 5 == 0 {
                        StagedTxSpec {
                            id: i,
                            begin: batch_begin,
                            end: batch_begin + 40,
                            aborted: true,
                            stages: vec![("fetch", batch_begin, batch_begin + 40)],
                        }
                    } else {
                        let issue = 100 + i * 6 + phase;
                        let exec = issue + 8;
                        StagedTxSpec {
                            id: i,
                            begin: batch_begin,
                            end: exec + 12,
                            aborted: false,
                            stages: vec![
                                ("fetch", batch_begin, issue),
                                ("issue", issue, exec),
                                ("exec", exec, exec + 12),
                            ],
                        }
                    }
                })
                .collect();
            make_staged_gen(g, &specs)
        };

        let g0 = build_stream(1, 0);
        let g1 = build_stream(2, 3);
        let single_stream = vec![RowStream {
            source_index: 0,
            trace: TraceId::A,
            generator: Arc::clone(&g0),
        }];
        let merged_streams = vec![
            RowStream {
                source_index: 0,
                trace: TraceId::A,
                generator: Arc::clone(&g0),
            },
            RowStream {
                source_index: 1,
                trace: TraceId::A,
                generator: Arc::clone(&g1),
            },
        ];

        let mut hide_fetch = StageVisibility::default();
        hide_fetch.toggle("fetch".into());
        let mut hide_fetch_issue = hide_fetch.clone();
        hide_fetch_issue.toggle("issue".into());
        let mut hide_all = hide_fetch_issue.clone();
        hide_all.toggle("exec".into());

        let fields = |a: Activity| {
            (
                a.above,
                a.visible,
                a.below,
                a.nearest_above,
                a.nearest_below,
                a.target,
                a.earlier,
                a.later,
            )
        };

        let zero_budget = MemoryBudget::new(0);
        let configs = [
            (single_stream, RowOrder::default(), vec![]),
            (merged_streams.clone(), RowOrder::default(), vec![]),
            (
                merged_streams.clone(),
                RowOrder {
                    mode: RowOrderMode::Stages {
                        stages: vec!["fetch".into()],
                    },
                    unmatched: UnmatchedPolicy::FallbackEnd,
                    follow_sort_key: true,
                },
                vec!["fetch".to_owned()],
            ),
            (
                merged_streams,
                RowOrder {
                    mode: RowOrderMode::Stages {
                        stages: vec!["exec".into()],
                    },
                    unmatched: UnmatchedPolicy::FallbackEnd,
                    follow_sort_key: true,
                },
                vec!["exec".to_owned()],
            ),
        ];

        // Independent requirement-derived oracle (does not call the production scorer).
        let independent_oracle = |records: &[(usize, Transaction)],
                                  rows: &RowSet<'_>,
                                  src_filter: Option<usize>,
                                  order: &RowOrder,
                                  vis: &StageVisibility,
                                  view: Viewport,
                                  visible: &Range<f64>,
                                  anchor: f64|
         -> Activity {
            let follow_sort_key = order.follow_sort_key && !order.is_start();
            let center = visible.start.midpoint(visible.end);
            let mut out = Activity::default();
            let mut best = (f64::INFINITY, f64::INFINITY, usize::MAX);
            let mut min_b = u64::MAX;
            let mut max_b = 0u64;
            let mut any = false;
            for (row, (src, tx)) in records.iter().enumerate() {
                if src_filter.is_some_and(|want| *src != want) {
                    continue;
                }
                min_b = min_b.min(tx.begin);
                max_b = max_b.max(tx.begin);
                any = true;
                let overlaps = ((tx.begin as f64) <= view.end && (tx.end as f64) >= view.start)
                    || tx.stages.iter().any(|s| {
                        let se = s.end.unwrap_or(tx.end).max(s.begin);
                        (s.begin as f64) <= view.end && (se as f64) >= view.start
                    });
                if !overlaps {
                    continue;
                }
                if row as f64 + 1.0 <= visible.start {
                    out.above += 1;
                    out.nearest_above = Some(out.nearest_above.map_or(row, |p| p.max(row)));
                } else if row as f64 >= visible.end {
                    out.below += 1;
                    out.nearest_below = Some(out.nearest_below.map_or(row, |p| p.min(row)));
                } else {
                    out.visible += 1;
                }
                // Derive anchor timestamp purely from requirements:
                // 1) Visible matched sort stage when stage-ordered with follow_sort_key.
                // 2) Earliest visible primary-lane stage begin.
                // 3) Otherwise normal transaction timestamp (never a hidden stage timestamp).
                let anchor_t = if let Some(m) =
                    follow_sort_key.then(|| rows.sort_meta(row)).flatten()
                    && m.matched
                    && let Some(idx) = m.sort_stage_index()
                    && let Some(st) = tx.stages.get(idx)
                    && !(st.lane == "0" && vis.is_hidden(&st.name))
                {
                    m.sort_key
                } else if let Some(vis_begin) = tx
                    .stages
                    .iter()
                    .filter(|s| s.lane == "0" && !vis.is_hidden(&s.name))
                    .map(|s| s.begin)
                    .min()
                {
                    vis_begin
                } else if let Some(m) = follow_sort_key.then(|| rows.sort_meta(row)).flatten()
                    && m.sort_stage_index().is_none()
                {
                    m.sort_key
                } else {
                    tx.begin
                };
                let score = (
                    (anchor_t as f64 - anchor).abs(),
                    (row as f64 + 0.5 - center).abs(),
                    row,
                );
                if score < best {
                    best = score;
                }
            }
            out.earlier = any && (min_b as f64) < view.start;
            out.later = any && (max_b as f64) > view.end;
            out.target = (best.2 != usize::MAX).then_some(best.2);
            out
        };

        for (streams, order, resolved) in configs {
            let indexed = MergedRows::build(streams.clone(), &order, &resolved, "0", None);
            let unindexed = MergedRows::build(streams, &order, &resolved, "0", Some(&zero_budget));
            let indexed_set = RowSet::borrowed(&indexed);
            let ordered_records: Vec<(usize, Transaction)> = (0..indexed.len)
                .map(|r| {
                    let (src, tx) = indexed_set.get(r).unwrap();
                    (src, tx.clone())
                })
                .collect();

            for vis in [&hide_fetch, &hide_fetch_issue, &hide_all] {
                let filter = Some(StageAnchorFilter {
                    visibility: vis,
                    primary_lane: "0",
                });
                let follow_sort_key = order.follow_sort_key && !order.is_start();

                for rows in [RowSet::borrowed(&indexed), RowSet::borrowed(&unindexed)] {
                    for (start, end, top, anchor) in [
                        (0.0, 260.0, 0.0, 148.0),
                        (100.0, 220.0, 10.5, 195.0),
                        (0.0, 160.0, 4.0, 25.0),
                    ] {
                        let view = Viewport { start, end };
                        let visible = top..top + 8.0;

                        let expected = independent_oracle(
                            &ordered_records,
                            &rows,
                            None,
                            &order,
                            vis,
                            view,
                            &visible,
                            anchor,
                        );
                        let actual =
                            rows.activity(view, visible.clone(), anchor, follow_sort_key, filter);
                        assert_eq!(
                            fields(actual),
                            fields(expected),
                            "hidden-stage activity mismatch for order={order:?} vis={vis:?} anchor={anchor}"
                        );

                        // When only `fetch` is hidden in Start order at anchor=195, target must be
                        // a row with a visible `issue` stage near 195, never an all-hidden aborted row.
                        if order.is_start() && vis == &hide_fetch && anchor == 195.0 {
                            let target = actual.target.expect("target row expected");
                            let (_, tx) = rows.get(target).unwrap();
                            let issue = tx
                                .stages
                                .iter()
                                .find(|s| s.name == "issue")
                                .expect("target should have a visible issue stage");
                            assert!(
                                (issue.begin as i64 - 195).abs() <= 6,
                                "expected target anchored near issue=195, got issue.begin={}",
                                issue.begin
                            );
                        }

                        // Solo/Dim per-source activity with hidden stages must also match oracle.
                        let src_expected = independent_oracle(
                            &ordered_records,
                            &rows,
                            Some(0),
                            &order,
                            vis,
                            view,
                            &visible,
                            anchor,
                        );
                        assert_eq!(
                            fields(rows.activity_for_source(
                                0,
                                view,
                                visible,
                                anchor,
                                follow_sort_key,
                                filter,
                            )),
                            fields(src_expected),
                            "solo hidden-stage activity mismatch for order={order:?} vis={vis:?}"
                        );
                    }
                }
            }
        }
    }

    // User scenario: an engineer hides a noisy primary-lane stage (and later hides all stages)
    // and zooms out into overview mode on both single-source and merged panels.
    // Requirement: `visible_presence` matches a brute-force filtered oracle identically whether
    // backed by the cached filtered `RowIndex` or unindexed fallback, excludes hidden stages,
    // yields an empty mask when all stages are hidden, and releases `MemoryBudget` on clear.
    #[test]
    fn visible_presence_matches_filtered_oracle_with_and_without_cached_index() {
        use super::super::stages::{StageAnchorFilter, StageVisibility};

        let make_stream = |g: u32, offset: u64| {
            let specs: Vec<StagedTxSpec<'_>> = (0..48u64)
                .map(|i| {
                    let b = offset + i * 2;
                    StagedTxSpec {
                        id: i,
                        begin: b,
                        end: b + 120,
                        aborted: false,
                        stages: vec![("fetch", b, b + 100), ("exec", b + 100, b + 120)],
                    }
                })
                .collect();
            make_staged_gen(g, &specs)
        };

        let g0 = make_stream(1, 0);
        let g1 = make_stream(2, 1);
        let single = vec![RowStream {
            source_index: 0,
            trace: TraceId::A,
            generator: Arc::clone(&g0),
        }];
        let merged = vec![
            RowStream {
                source_index: 0,
                trace: TraceId::A,
                generator: g0,
            },
            RowStream {
                source_index: 1,
                trace: TraceId::A,
                generator: g1,
            },
        ];

        let mut hide_fetch = StageVisibility::default();
        hide_fetch.toggle("fetch".into());
        let mut hide_all = hide_fetch.clone();
        hide_all.toggle("exec".into());

        let order = RowOrder::default();
        let zero_budget = MemoryBudget::new(0);
        let roomy_budget = MemoryBudget::new(4 * 1024 * 1024);

        for streams in [single, merged] {
            let mut indexed =
                MergedRows::build(streams.clone(), &order, &[], "0", Some(&roomy_budget));
            let mut unindexed = MergedRows::build(streams, &order, &[], "0", Some(&zero_budget));

            let exec_only_records: Vec<Transaction> = {
                let set = RowSet::borrowed(&indexed);
                (0..set.len())
                    .map(|r| {
                        let mut tx = set.get(r).unwrap().1.clone();
                        tx.stages.retain(|s| s.name != "fetch");
                        tx
                    })
                    .collect()
            };

            let view = Viewport {
                start: 0.0,
                end: 240.0,
            };
            let map = PixelMap::new(
                Rect::from_xywh(0.25, 0.5, 48.0, 12.0),
                view,
                RowView {
                    top: 0.0,
                    row_px: 0.5,
                },
                1.0,
            );
            let expected_exec_only = oracle(&exec_only_records, 0, &map);

            indexed.sync_visible_index(&hide_fetch, "0", Some(&roomy_budget));
            assert!(matches!(
                indexed.visible_index.as_ref().map(|c| &c.state),
                Some(FilteredIndexState::Ready(_))
            ));
            unindexed.sync_visible_index(&hide_fetch, "0", Some(&zero_budget));
            assert!(matches!(
                unindexed.visible_index.as_ref().map(|c| &c.state),
                Some(FilteredIndexState::Refused { .. })
            ));

            // Raising budget headroom causes a previously refused filtered index to retry and build.
            let growing_budget = MemoryBudget::new(0);
            unindexed.sync_visible_index(&hide_fetch, "0", Some(&growing_budget));
            assert!(matches!(
                unindexed.visible_index.as_ref().map(|c| &c.state),
                Some(FilteredIndexState::Refused { .. })
            ));
            growing_budget.set_limit(4 * 1024 * 1024);
            unindexed.sync_visible_index(&hide_fetch, "0", Some(&growing_budget));
            assert!(matches!(
                unindexed.visible_index.as_ref().map(|c| &c.state),
                Some(FilteredIndexState::Ready(_))
            ));
            unindexed.clear_visible_index();
            assert_eq!(growing_budget.used(), 0);
            unindexed.sync_visible_index(&hide_fetch, "0", Some(&zero_budget));

            let filter = StageAnchorFilter {
                visibility: &hide_fetch,
                primary_lane: "0",
            };
            assert_eq!(
                RowSet::borrowed(&indexed).visible_presence(&map, filter),
                expected_exec_only
            );
            assert_eq!(
                RowSet::borrowed(&unindexed).visible_presence(&map, filter),
                expected_exec_only
            );

            // Hiding every primary stage yields an empty foreground mask.
            indexed.sync_visible_index(&hide_all, "0", Some(&roomy_budget));
            let all_filter = StageAnchorFilter {
                visibility: &hide_all,
                primary_lane: "0",
            };
            assert_eq!(
                RowSet::borrowed(&indexed)
                    .visible_presence(&map, all_filter)
                    .runs()
                    .count(),
                0
            );
            assert_eq!(
                RowSet::borrowed(&unindexed)
                    .visible_presence(&map, all_filter)
                    .runs()
                    .count(),
                0
            );

            indexed.clear();
            assert_eq!(roomy_budget.used(), 0);
        }
    }

    // User scenario: a trace has an overlay stage sharing the name of a primary-lane stage at an
    // earlier timestamp, AND a transaction whose visible primary-lane stage lies outside its
    // owner's [tx.begin, tx.end] lifetime.
    // Requirement: stage sort matching ignores overlay lanes and uses the primary-lane stage;
    // FollowActivity finds visible stages outside their owner's lifetime in both indexed and
    // unindexed paths.
    #[test]
    fn sort_ignores_overlay_same_name_and_activity_finds_out_of_lifetime_stages() {
        use super::super::{
            order::{RowOrderMode, UnmatchedPolicy},
            stages::{StageAnchorFilter, StageVisibility},
        };

        let records = vec![
            Transaction {
                id: TransactionRef(0),
                generator: TrackRef(1),
                begin: 100,
                end: 110,
                status: Status::Ok,
                kind: SpanKind::Unspecified,
                parent: None,
                attributes: vec![],
                events: vec![],
                stages: vec![
                    // Earlier overlay stage sharing the primary stage name "issue".
                    TransactionStage {
                        id: TransactionRef(10),
                        generator: None,
                        name: "issue".into(),
                        lane: "1".into(),
                        begin: 10,
                        end: Some(20),
                        attributes: vec![],
                    },
                    // Primary-lane "issue" lies outside owner's [100, 110] lifetime.
                    TransactionStage {
                        id: TransactionRef(11),
                        generator: None,
                        name: "issue".into(),
                        lane: "0".into(),
                        begin: 130,
                        end: Some(150),
                        attributes: vec![],
                    },
                ],
            },
            Transaction {
                id: TransactionRef(1),
                generator: TrackRef(1),
                begin: 100,
                end: 120,
                status: Status::Ok,
                kind: SpanKind::Unspecified,
                parent: None,
                attributes: vec![],
                events: vec![],
                stages: vec![TransactionStage {
                    id: TransactionRef(20),
                    generator: None,
                    name: "issue".into(),
                    lane: "0".into(),
                    begin: 105,
                    end: Some(115),
                    attributes: vec![],
                }],
            },
        ];

        let g_indexed = Arc::new(
            LoadedGenerator::new(TrackRef(1), records.clone(), Default::default(), vec![]).unwrap(),
        );
        let mut g_unindexed_raw =
            LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap();
        g_unindexed_raw.row_index = None;
        let g_unindexed = Arc::new(g_unindexed_raw);
        assert!(g_unindexed.row_index.is_none());
        assert!(g_unindexed.stages_exceed_owner());

        // Ordering by "issue" on primary lane "0" puts tx1 (issue=105) before tx0 (primary issue=130,
        // ignoring overlay "issue" at t=10).
        let order = RowOrder {
            mode: RowOrderMode::Stages {
                stages: vec!["issue".into()],
            },
            unmatched: UnmatchedPolicy::FallbackEnd,
            follow_sort_key: true,
        };
        let resolved = vec!["issue".to_owned()];
        let ordered =
            RowSet::from_generators_with_order(&[Arc::clone(&g_indexed)], &order, &resolved);
        assert_eq!(ordered.get(0).unwrap().1.id, TransactionRef(1));
        assert_eq!(ordered.sort_meta(0).unwrap().sort_key, 105);
        assert_eq!(ordered.get(1).unwrap().1.id, TransactionRef(0));
        assert_eq!(ordered.sort_meta(1).unwrap().sort_key, 130);

        // Viewport [125, 160] contains ONLY tx0's out-of-lifetime primary stage [130, 150].
        // FollowActivity must find row 0 on both indexed and unindexed single-source and merged paths.
        let mut hide_other = StageVisibility::default();
        hide_other.toggle("fetch".into());
        let filter = Some(StageAnchorFilter {
            visibility: &hide_other,
            primary_lane: "0",
        });
        let view = Viewport {
            start: 125.0,
            end: 160.0,
        };
        for generator in [g_indexed, g_unindexed] {
            let direct = RowSet::from_generators(&[generator]);
            let act = direct.activity(view, 0.0..10.0, 140.0, false, filter);
            assert_eq!(act.visible, 1);
            assert_eq!(act.target, Some(0));
        }
    }
}
