//! Cached time-interleaved k-way merge and interval-indexed queries over one or more immutable
//! transaction generators.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::ops::Range;
use std::sync::Arc;

use super::{
    activity::Activity,
    occupancy::{RowIndex, TransactionTable, raster_source},
    raster::PixelMap,
};
use crate::{
    data::loaded_tracks::LoadedGenerator, scene::PresenceMask, trace::TraceId,
    wave::viewport::Viewport,
};
use volna_trace::{
    data::transactions::{Status, TrackRef, Transaction, TransactionRef},
    remote::memory::MemoryBudget,
};

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

/// Cached static k-way merge across the active visible generators of a pipeline panel.
#[derive(Default)]
pub(crate) struct MergedRows {
    fingerprint: Fingerprint,
    streams: Vec<RowStream>,
    generators: Vec<Arc<LoadedGenerator>>,
    generator_ptrs: Vec<usize>,
    entries: Vec<RowEntry>,
    reverse: Vec<Vec<u32>>,
    index: Option<RowIndex>,
    len: usize,
}

impl MergedRows {
    pub(crate) fn fingerprint(&self) -> &[(usize, TraceId, usize)] {
        &self.fingerprint
    }

    pub(crate) fn matches(&self, streams: &[RowStream]) -> bool {
        self.fingerprint.len() == streams.len()
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
        self.streams.clear();
        self.generators.clear();
        self.generator_ptrs.clear();
        self.entries.clear();
        self.reverse.clear();
        self.index = None;
        self.len = 0;
    }

    pub(crate) fn build(streams: Vec<RowStream>, budget: Option<&MemoryBudget>) -> Self {
        let fingerprint = stream_fingerprint(&streams);
        let generators: Vec<Arc<LoadedGenerator>> =
            streams.iter().map(|s| Arc::clone(&s.generator)).collect();
        let generator_ptrs: Vec<usize> =
            generators.iter().map(|g| Arc::as_ptr(g) as usize).collect();
        if streams.is_empty() {
            return Self {
                fingerprint,
                streams,
                generators,
                generator_ptrs,
                entries: Vec::new(),
                reverse: Vec::new(),
                index: None,
                len: 0,
            };
        }
        if streams.len() == 1 {
            let len = streams[0].generator.transactions().len();
            return Self {
                fingerprint,
                streams,
                generators,
                generator_ptrs,
                entries: Vec::new(),
                reverse: Vec::new(),
                index: None,
                len,
            };
        }

        let total: usize = streams
            .iter()
            .map(|s| s.generator.transactions().len())
            .sum();
        let mut entries = Vec::with_capacity(total);
        let mut reverse: Vec<Vec<u32>> = streams
            .iter()
            .map(|s| vec![0u32; s.generator.transactions().len()])
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
            streams,
            generators,
            generator_ptrs,
            entries,
            reverse,
            index,
            len,
        }
    }

    pub(crate) fn rebuild(&mut self, streams: Vec<RowStream>, budget: Option<&MemoryBudget>) {
        if self.matches(&streams) {
            return;
        }
        // Release the previous RowIndex reservation before reserving for the replacement.
        self.clear();
        *self = Self::build(streams, budget);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn is_single_stream(&self) -> bool {
        self.streams.len() == 1
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
        let streams = generators
            .iter()
            .enumerate()
            .map(|(i, g)| RowStream {
                source_index: i,
                trace: TraceId::A,
                generator: Arc::clone(g),
            })
            .collect();
        Self::owned(MergedRows::build(streams, None))
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
        if inner.is_single_stream() {
            Some(ordinal)
        } else {
            Some(inner.reverse[s][ordinal] as usize)
        }
    }

    /// Returns `(source_index, &Transaction)` for the given merged row.
    #[must_use]
    pub fn get(&self, row: usize) -> Option<(usize, &Transaction)> {
        let inner = self.storage.as_ref();
        if row >= inner.len {
            return None;
        }
        if inner.is_single_stream() {
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
        if inner.is_single_stream() {
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
    pub(super) fn presence(&self, map: &PixelMap) -> PresenceMask {
        let inner = self.storage.as_ref();
        let mut mask = map.mask();
        if inner.len == 0 {
            return mask;
        }
        if inner.is_single_stream() {
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

    pub(super) fn aborted_presence(&self, map: &PixelMap) -> PresenceMask {
        let inner = self.storage.as_ref();
        let mut mask = map.aborted_mask();
        if inner.len == 0 {
            return mask;
        }
        if inner.is_single_stream() {
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

    pub(super) fn activity(&self, view: Viewport, visible: Range<f64>, anchor: f64) -> Activity {
        let inner = self.storage.as_ref();
        if inner.len == 0 {
            return Activity::default();
        }
        let center = visible.start.midpoint(visible.end);
        let earlier = self
            .get(0)
            .is_some_and(|(_, tx)| (tx.begin as f64) < view.start);
        let later = self
            .get(inner.len - 1)
            .is_some_and(|(_, tx)| (tx.begin as f64) > view.end);

        if inner.is_single_stream() {
            let g = &inner.streams[0].generator;
            let records = g.transactions();
            if let Some(index) = &g.row_index {
                let mut result = index.activity(records, 0, view, visible, anchor, center);
                result.earlier = earlier;
                result.later = later;
                return result;
            }
            let mut result = Activity {
                earlier,
                later,
                ..Activity::default()
            };
            let mut best = (f64::INFINITY, f64::INFINITY, usize::MAX);
            g.visit_window_ordinals(
                view.start.max(0.0).floor() as u64,
                view.end.max(0.0).ceil() as u64,
                |row, tx| {
                    record_activity(row, tx, view, &visible, anchor, &mut result, &mut best);
                    true
                },
            )
            .expect("ordered viewport");
            result.target = (best.2 != usize::MAX).then_some(best.2);
            return result;
        }

        let table = MergedTable {
            streams: &inner.streams,
            entries: &inner.entries,
        };
        if let Some(index) = &inner.index {
            let mut result = index.activity(&table, 0, view, visible, anchor, center);
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
        let mut best = (f64::INFINITY, f64::INFINITY, usize::MAX);
        let start_u64 = view.start.max(0.0).floor() as u64;
        let end_u64 = view.end.max(0.0).ceil() as u64;
        for (s, stream) in inner.streams.iter().enumerate() {
            let rev = &inner.reverse[s];
            stream
                .generator
                .visit_window_ordinals(start_u64, end_u64, |ordinal, tx| {
                    let row = rev[ordinal] as usize;
                    record_activity(row, tx, view, &visible, anchor, &mut result, &mut best);
                    true
                })
                .expect("ordered viewport");
        }
        result.target = (best.2 != usize::MAX).then_some(best.2);
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
    ) -> Activity {
        let inner = self.storage.as_ref();
        if inner.len == 0 {
            return Activity::default();
        }
        if inner.is_single_stream() {
            return if inner.streams[0].source_index == source_index {
                self.activity(view, visible, anchor)
            } else {
                Activity::default()
            };
        }
        let center = visible.start.midpoint(visible.end);
        let mut combined = Activity::default();
        let mut best_target: Option<(f64, f64, usize)> = None;
        let start_u64 = view.start.max(0.0).floor() as u64;
        let end_u64 = view.end.max(0.0).ceil() as u64;
        for (s, stream) in inner.streams.iter().enumerate() {
            if stream.source_index != source_index {
                continue;
            }
            let g = &stream.generator;
            let records = g.transactions();
            if records.is_empty() {
                continue;
            }
            combined.earlier |= (records[0].begin as f64) < view.start;
            combined.later |= (records[records.len() - 1].begin as f64) > view.end;
            let rev = &inner.reverse[s];
            if let Some(index) = &g.row_index {
                let part = index.activity_mapped(
                    records,
                    |ord| rev[ord] as usize,
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
                    let dist = (tx.begin as f64 - anchor)
                        .max(anchor - tx.end as f64)
                        .max(0.0);
                    let score = (dist, (row as f64 + 0.5 - center).abs(), row);
                    if best_target.is_none_or(|prev| score < prev) {
                        best_target = Some(score);
                    }
                }
            } else {
                let mut best = best_target.unwrap_or((f64::INFINITY, f64::INFINITY, usize::MAX));
                g.visit_window_ordinals(start_u64, end_u64, |ordinal, tx| {
                    let row = rev[ordinal] as usize;
                    record_activity(row, tx, view, &visible, anchor, &mut combined, &mut best);
                    true
                })
                .expect("ordered viewport");
                if best.2 != usize::MAX {
                    best_target = Some(best);
                }
            }
        }
        combined.target = best_target.map(|(_, _, row)| row);
        combined
    }
}

fn record_activity(
    row: usize,
    tx: &Transaction,
    view: Viewport,
    visible: &Range<f64>,
    anchor: f64,
    result: &mut Activity,
    best: &mut (f64, f64, usize),
) {
    if (tx.end as f64) < view.start || (tx.begin as f64) > view.end {
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
    let distance = (tx.begin as f64 - anchor)
        .max(anchor - tx.end as f64)
        .max(0.0);
    let score = (distance, (row as f64 + 0.5 - center).abs(), row);
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
        let indexed = MergedRows::build(streams.clone(), None);
        assert!(indexed.index.is_some());
        let zero_budget = MemoryBudget::new(0);
        let unindexed = MergedRows::build(streams, Some(&zero_budget));
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
                let mut best = (f64::INFINITY, f64::INFINITY, usize::MAX);
                for (row, tx) in merged_records.iter().enumerate() {
                    record_activity(row, tx, view, &visible, anchor, &mut expected, &mut best);
                }
                expected.target = (best.2 != usize::MAX).then_some(best.2);
                assert_eq!(
                    fields(rows.activity(view, visible.clone(), anchor)),
                    fields(expected)
                );

                // Solo (Dim) per-source activity in merged row coordinates must also match oracle.
                for target_src in 0..generators.len() {
                    let mut src_expected = Activity::default();
                    let mut src_best = (f64::INFINITY, f64::INFINITY, usize::MAX);
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
                            view,
                            &visible,
                            anchor,
                            &mut src_expected,
                            &mut src_best,
                        );
                    }
                    src_expected.earlier = first_begin.is_some_and(|b| (b as f64) < view.start);
                    src_expected.later = last_begin.is_some_and(|b| (b as f64) > view.end);
                    src_expected.target = (src_best.2 != usize::MAX).then_some(src_best.2);
                    assert_eq!(
                        fields(rows.activity_for_source(target_src, view, visible.clone(), anchor)),
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
        let probe = MemoryBudget::new(1024 * 1024);
        let initial = MergedRows::build(s01.clone(), Some(&probe));
        assert!(initial.index.is_some());
        let needed = probe.used();
        drop(initial);
        assert_eq!(probe.used(), 0);

        // Give budget room for only ONE RowIndex (1.5x needed < 2x needed).
        let tight = MemoryBudget::new(needed + needed / 2);
        let mut merged = MergedRows::build(s01, Some(&tight));
        assert!(merged.index.is_some());
        merged.rebuild(s02, Some(&tight));
        assert!(
            merged.index.is_some(),
            "rebuild should free old RowIndex reservation before building replacement"
        );
        merged.clear();
        assert_eq!(tight.used(), 0);
    }
}
