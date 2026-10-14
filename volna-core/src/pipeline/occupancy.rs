//! A bounded row hierarchy over immutable records.

use super::{activity::Activity, raster::PixelMap};
use crate::{scene::PresenceMask, wave::viewport::Viewport};
use std::ops::Range;
use volna_trace::{
    data::transactions::{Transaction, TxStatus},
    remote::memory::{MemoryBudget, Reservation},
};

const BLOCK: usize = 16;
const FRAGMENTS: usize = 32;
const INDEX_LIMIT: usize = 64 * 1024 * 1024;
const INCOMPLETE: u8 = u8::MAX;

#[derive(Clone, Copy, Debug)]
struct Span {
    begin: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    begin: u64,
    end: u64,
    min_end: u64,
    max_end: u64,
    union: u32,
    len: u8,
    aborted: bool,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            begin: u64::MAX,
            end: 0,
            min_end: u64::MAX,
            max_end: 0,
            union: 0,
            len: 0,
            aborted: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct RowIndex {
    nodes: Vec<Node>,
    spans: Vec<Span>,
    leaves: usize,
    reservation: Option<Reservation>,
}

impl RowIndex {
    pub(crate) async fn build<F: std::future::Future<Output = ()>>(
        records: &[Transaction],
        budget: Option<&MemoryBudget>,
        checkpoint: &mut impl FnMut() -> F,
    ) -> Option<Self> {
        if records.is_empty() {
            return None;
        }
        let leaves = records.len().div_ceil(BLOCK).checked_next_power_of_two()?;
        let count = leaves.checked_mul(2)?;
        let node_bytes = count.checked_mul(std::mem::size_of::<Node>())?;
        let worst = count
            .checked_mul(FRAGMENTS * std::mem::size_of::<Span>())?
            .checked_add(node_bytes)?
            .min(INDEX_LIMIT);
        let limit = budget.map_or(worst as u64, |b| {
            (worst as u64)
                .min(b.limit().saturating_sub(b.used()))
                .min(b.object_limit())
        }) as usize;
        if node_bytes > limit {
            return None;
        }
        let reservation = match budget {
            Some(b) => Some(
                b.reserve_object("the pipeline row index", limit as u64)
                    .ok()?,
            ),
            None => None,
        };
        let mut nodes = Vec::new();
        nodes.try_reserve_exact(count).ok()?;
        for _ in 0..count {
            checkpoint().await;
            nodes.push(Node::default());
        }
        let mut result = Self {
            nodes,
            spans: Vec::new(),
            leaves,
            reservation,
        };
        let max_spans = (limit - node_bytes) / std::mem::size_of::<Span>();
        let mut union = Vec::with_capacity(FRAGMENTS + 1);
        for (leaf, rows) in records.chunks(BLOCK).enumerate() {
            let mut node = Node::default();
            union.clear();
            let mut complete = true;
            for tx in rows {
                checkpoint().await;
                node.min_end = node.min_end.min(tx.end);
                node.max_end = node.max_end.max(tx.end);
                node.aborted |= tx.status == TxStatus::Aborted;
                for span in geometry(tx) {
                    checkpoint().await;
                    node.begin = node.begin.min(span.begin);
                    node.end = node.end.max(span.end);
                    if complete {
                        insert(&mut union, span);
                        complete = union.len() <= FRAGMENTS;
                    }
                }
            }
            result.save(&mut node, complete.then_some(union.as_slice()), max_spans);
            result.nodes[leaves + leaf] = node;
        }
        for i in (1..leaves).rev() {
            checkpoint().await;
            let a = result.nodes[i * 2];
            let b = result.nodes[i * 2 + 1];
            let mut node = Node {
                begin: a.begin.min(b.begin),
                end: a.end.max(b.end),
                min_end: a.min_end.min(b.min_end),
                max_end: a.max_end.max(b.max_end),
                aborted: a.aborted || b.aborted,
                ..Node::default()
            };
            union.clear();
            let mut complete = a.len != INCOMPLETE && b.len != INCOMPLETE;
            if complete {
                for child in [a, b] {
                    for &span in result.summary(child) {
                        checkpoint().await;
                        insert(&mut union, span);
                        if union.len() > FRAGMENTS {
                            complete = false;
                            break;
                        }
                    }
                    if !complete {
                        break;
                    }
                }
            }
            result.save(&mut node, complete.then_some(union.as_slice()), max_spans);
            result.nodes[i] = node;
        }
        if let Some(r) = &mut result.reservation {
            let bytes = node_bytes + result.spans.capacity() * std::mem::size_of::<Span>();
            r.shrink(r.bytes() - bytes as u64)
                .expect("bounded index allocation");
        }
        Some(result)
    }

    fn save(&mut self, node: &mut Node, union: Option<&[Span]>, max_spans: usize) {
        node.len = INCOMPLETE;
        let Some(union) = union else {
            return;
        };
        let required = self.spans.len() + union.len();
        if required > max_spans {
            return;
        }
        if required > self.spans.capacity() {
            let capacity = required
                .max(self.spans.capacity().max(256).saturating_mul(2))
                .min(max_spans);
            if self
                .spans
                .try_reserve_exact(capacity - self.spans.len())
                .is_err()
            {
                return;
            }
        }
        node.union = self.spans.len() as u32;
        node.len = union.len() as u8;
        self.spans.extend_from_slice(union);
    }

    fn summary(&self, node: Node) -> &[Span] {
        &self.spans[node.union as usize..node.union as usize + node.len as usize]
    }

    pub(crate) fn raster(
        &self,
        records: &[Transaction],
        offset: usize,
        map: &PixelMap,
        mask: &mut PresenceMask,
    ) {
        self.visit_pixels(1, 0..self.leaves * BLOCK, records, offset, map, mask);
    }

    fn visit_pixels(
        &self,
        i: usize,
        rows: Range<usize>,
        records: &[Transaction],
        offset: usize,
        map: &PixelMap,
        mask: &mut PresenceMask,
    ) {
        let end = rows.end.min(records.len());
        if rows.start >= end {
            return;
        }
        let node = self.nodes[i];
        if node.begin > node.end {
            return;
        }
        let Some(y) = map
            .y
            .interval((offset + rows.start) as u64, (offset + end) as u64)
        else {
            return;
        };
        if map.x.interval(node.begin, node.end).is_none() && !map.x.contains(node.end) {
            return;
        }
        let full_y = map
            .y
            .inside((offset + rows.start) as u64, (offset + end) as u64, false);
        if y.len() == 1 && full_y {
            if map.x.inside(node.begin, node.end, true)
                && map.x.floor(node.begin) == map.x.floor(node.end)
            {
                let x = map.x.floor(node.begin) as usize;
                mask.fill(x..x + 1, y);
                return;
            }
            if node.len != INCOMPLETE {
                for span in self.summary(node) {
                    if let Some(x) = map.x.interval(span.begin, span.end) {
                        mask.fill(x, y.clone());
                    }
                }
                return;
            }
        }
        if i >= self.leaves {
            raster_source(&records[rows.start..end], offset + rows.start, map, mask);
        } else {
            let mid = usize::midpoint(rows.start, rows.end);
            self.visit_pixels(i * 2, rows.start..mid, records, offset, map, mask);
            self.visit_pixels(i * 2 + 1, mid..rows.end, records, offset, map, mask);
        }
    }

    pub(crate) fn raster_aborted(
        &self,
        records: &[Transaction],
        offset: usize,
        map: &PixelMap,
        mask: &mut PresenceMask,
    ) {
        self.visit_aborted(1, 0..self.leaves * BLOCK, records, offset, map, mask);
    }

    fn visit_aborted(
        &self,
        i: usize,
        rows: Range<usize>,
        records: &[Transaction],
        offset: usize,
        map: &PixelMap,
        mask: &mut PresenceMask,
    ) {
        let end = rows.end.min(records.len());
        if !self.nodes[i].aborted || rows.start >= end {
            return;
        }
        let Some(y) = map
            .y
            .interval((offset + rows.start) as u64, (offset + end) as u64)
        else {
            return;
        };
        if y.len() == 1
            && map
                .y
                .inside((offset + rows.start) as u64, (offset + end) as u64, false)
        {
            mask.fill(0..1, y);
            return;
        }
        if i >= self.leaves {
            for (row, tx) in records[rows.start..end].iter().enumerate() {
                if tx.status == TxStatus::Aborted {
                    map.aborted(rows.start + row + offset, mask);
                }
            }
        } else {
            let mid = usize::midpoint(rows.start, rows.end);
            self.visit_aborted(i * 2, rows.start..mid, records, offset, map, mask);
            self.visit_aborted(i * 2 + 1, mid..rows.end, records, offset, map, mask);
        }
    }

    pub(crate) fn activity(
        &self,
        records: &[Transaction],
        offset: usize,
        view: Viewport,
        visible: Range<f64>,
        anchor: f64,
        center: f64,
    ) -> Activity {
        let query = ActivityQuery {
            records,
            offset,
            view,
            visible,
            anchor,
            center,
        };
        let mut activity = Activity::default();
        self.count_activity(1, 0..self.leaves * BLOCK, &query, &mut activity);
        let mut best = (f64::INFINITY, f64::INFINITY, usize::MAX);
        self.nearest(1, 0..self.leaves * BLOCK, &query, &mut best);
        activity.target = (best.2 != usize::MAX).then_some(best.2);
        activity
    }

    fn count_activity(
        &self,
        i: usize,
        rows: Range<usize>,
        query: &ActivityQuery<'_>,
        out: &mut Activity,
    ) {
        let records = query.records;
        let offset = query.offset;
        let view = query.view;
        let visible = &query.visible;
        let end = rows.end.min(records.len());
        if rows.start >= end
            || records[rows.start].begin as f64 > view.end
            || (self.nodes[i].max_end as f64) < view.start
        {
            return;
        }
        let first = offset + rows.start;
        let last = offset + end;
        let zone = if last as f64 <= visible.start {
            0
        } else if first as f64 >= visible.end {
            2
        } else if first as f64 + 1.0 > visible.start && ((last - 1) as f64) < visible.end {
            1
        } else {
            3
        };
        if zone != 3
            && records[end - 1].begin as f64 <= view.end
            && self.nodes[i].min_end as f64 >= view.start
        {
            match zone {
                0 => {
                    out.above += end - rows.start;
                    out.nearest_above = Some(last - 1);
                }
                1 => out.visible += end - rows.start,
                _ => {
                    out.below += end - rows.start;
                    out.nearest_below.get_or_insert(first);
                }
            }
        } else if i >= self.leaves {
            for (row, tx) in records[rows.start..end].iter().enumerate() {
                if tx.begin as f64 > view.end || (tx.end as f64) < view.start {
                    continue;
                }
                let row = first + row;
                if row as f64 + 1.0 <= visible.start {
                    out.above += 1;
                    out.nearest_above = Some(row);
                } else if row as f64 >= visible.end {
                    out.below += 1;
                    out.nearest_below.get_or_insert(row);
                } else {
                    out.visible += 1;
                }
            }
        } else {
            let mid = usize::midpoint(rows.start, rows.end);
            self.count_activity(i * 2, rows.start..mid, query, out);
            self.count_activity(i * 2 + 1, mid..rows.end, query, out);
        }
    }

    fn nearest(
        &self,
        i: usize,
        rows: Range<usize>,
        query: &ActivityQuery<'_>,
        best: &mut (f64, f64, usize),
    ) {
        let records = query.records;
        let offset = query.offset;
        let view = query.view;
        let anchor = query.anchor;
        let center = query.center;
        let end = rows.end.min(records.len());
        if rows.start >= end
            || records[rows.start].begin as f64 > view.end
            || (self.nodes[i].max_end as f64) < view.start
        {
            return;
        }
        let distance = (records[rows.start].begin as f64 - anchor)
            .max(anchor - self.nodes[i].max_end as f64)
            .max(0.0);
        let row_distance = (offset as f64 + rows.start as f64 + 0.5 - center)
            .max(center - (offset as f64 + end as f64 - 0.5))
            .max(0.0);
        if (distance, row_distance, offset + rows.start) >= *best {
            return;
        }
        if i >= self.leaves {
            for (row, tx) in records[rows.start..end].iter().enumerate() {
                if tx.begin as f64 > view.end || (tx.end as f64) < view.start {
                    continue;
                }
                let row = offset + rows.start + row;
                let score = (
                    (tx.begin as f64 - anchor)
                        .max(anchor - tx.end as f64)
                        .max(0.0),
                    (row as f64 + 0.5 - center).abs(),
                    row,
                );
                if score < *best {
                    *best = score;
                }
            }
        } else {
            let mid = usize::midpoint(rows.start, rows.end);
            let children = if center < (offset + mid) as f64 {
                [(i * 2, rows.start..mid), (i * 2 + 1, mid..rows.end)]
            } else {
                [(i * 2 + 1, mid..rows.end), (i * 2, rows.start..mid)]
            };
            for (child, rows) in children {
                self.nearest(child, rows, query, best);
            }
        }
    }

    pub(crate) fn scale_times(&mut self, scale: u64) {
        for node in &mut self.nodes {
            if node.begin <= node.end {
                node.begin = node.begin.saturating_mul(scale);
                node.end = node.end.saturating_mul(scale);
            }
            node.min_end = node.min_end.saturating_mul(scale);
            node.max_end = node.max_end.saturating_mul(scale);
        }
        for span in &mut self.spans {
            span.begin = span.begin.saturating_mul(scale);
            span.end = span.end.saturating_mul(scale);
        }
    }
}

struct ActivityQuery<'a> {
    records: &'a [Transaction],
    offset: usize,
    view: Viewport,
    visible: Range<f64>,
    anchor: f64,
    center: f64,
}

fn geometry(tx: &Transaction) -> impl Iterator<Item = Span> + '_ {
    tx.stages
        .iter()
        .map(|s| Span {
            begin: s.begin,
            end: s.end.unwrap_or(tx.end).max(s.begin),
        })
        .chain(tx.stages.is_empty().then_some(Span {
            begin: tx.begin,
            end: tx.end,
        }))
}

fn insert(union: &mut Vec<Span>, span: Span) {
    let i = union.partition_point(|s| (s.begin, s.end) <= (span.begin, span.end));
    union.insert(i, span);
    let mut len = 0;
    for i in 0..union.len() {
        let span = union[i];
        if len > 0
            && (span.begin < union[len - 1].end
                || span.begin == union[len - 1].end && span.end > span.begin
                || span.begin == union[len - 1].begin && span.end == union[len - 1].end)
        {
            union[len - 1].end = union[len - 1].end.max(span.end);
        } else {
            union[len] = span;
            len += 1;
        }
    }
    union.truncate(len);
}

pub(crate) fn raster_source(
    records: &[Transaction],
    offset: usize,
    map: &PixelMap,
    mask: &mut PresenceMask,
) {
    for (row, tx) in records.iter().enumerate() {
        let Some(y) = map
            .y
            .interval((offset + row) as u64, (offset + row + 1) as u64)
        else {
            continue;
        };
        for span in geometry(tx) {
            if let Some(x) = map.x.interval(span.begin, span.end) {
                mask.fill(x, y.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::Rect,
        pipeline::{raster::oracle, rows::RowView},
    };
    use volna_trace::data::transactions::{TrackRef, TransactionRef, TransactionStage, TxKind};

    fn tx(id: u64, begin: u64, end: u64, spans: &[(u64, Option<u64>)]) -> Transaction {
        Transaction {
            id: TransactionRef(id),
            generator: TrackRef(1),
            begin,
            end,
            status: if id.is_multiple_of(7) {
                TxStatus::Aborted
            } else {
                TxStatus::Open
            },
            kind: TxKind::Unspecified,
            parent: None,
            attributes: vec![],
            events: vec![],
            stages: spans
                .iter()
                .enumerate()
                .map(|(i, &(begin, end))| TransactionStage {
                    name: "stage".into(),
                    lane: (i % 2).to_string(),
                    begin,
                    end,
                    attributes: vec![],
                })
                .collect(),
        }
    }
    fn build(records: &[Transaction], budget: Option<&MemoryBudget>) -> Option<RowIndex> {
        let mut checkpoint = || std::future::ready(());
        let mut future = std::pin::pin!(RowIndex::build(records, budget, &mut checkpoint));
        match std::future::Future::poll(
            future.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop()),
        ) {
            std::task::Poll::Ready(index) => index,
            std::task::Poll::Pending => unreachable!(),
        }
    }
    #[test]
    fn hierarchy_and_source_match_inverse_footprints_across_scales_clips_and_phases() {
        let mut seed = 0x37ab_cdef_u64;
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let records: Vec<_> = (0..83)
            .map(|row| {
                let spans: Vec<_> = (0..row % 7)
                    .map(|i| {
                        let a = (random() % 200) * 10;
                        (
                            a,
                            if i % 5 == 0 {
                                None
                            } else {
                                Some(a + random() % 9)
                            },
                        )
                    })
                    .collect();
                tx(row, row * 2, row * 2 + 20, &spans)
            })
            .collect();
        let index = build(&records, None).unwrap();
        assert!(index.nodes.iter().any(|n| n.len == INCOMPLETE));
        for case in 0..120 {
            let scale = [1.0, 1.25, 1.5, 2.0][case % 4];
            let ui_zoom = [0.75, 1.0, 1.5][case % 3];
            let map = PixelMap::new(
                Rect::from_xywh(0.37, 0.23, 23.0, 9.0),
                Viewport {
                    start: (random() % 220) as f64 * 5.0 - 20.75,
                    end: (random() % 100 + 240) as f64 * 5.0 + 0.125,
                },
                RowView {
                    top: (random() % 95) as f64 - 10.25,
                    row_px: [0.03, 0.13, 0.5, 1.3][case % 4] * ui_zoom,
                },
                scale,
            );
            let expected = oracle(&records, 7, &map);
            let mut actual = map.mask();
            index.raster(&records, 7, &map, &mut actual);
            assert_eq!(actual, expected, "hierarchy case {case}");
            let mut actual = map.mask();
            raster_source(&records, 7, &map, &mut actual);
            assert_eq!(actual, expected, "fallback case {case}");
            let mut aborted = map.aborted_mask();
            index.raster_aborted(&records, 7, &map, &mut aborted);
            let mut expected_aborted = map.aborted_mask();
            for (row, tx) in records.iter().enumerate() {
                if tx.status == TxStatus::Aborted {
                    map.aborted(row + 7, &mut expected_aborted);
                }
            }
            assert_eq!(aborted, expected_aborted);
        }
    }

    #[test]
    fn terminal_points_gaps_outliers_and_late_ticks_survive_aggregation() {
        for origin in [0, 1u64 << 60, u64::MAX - 32768] {
            let records = vec![
                tx(
                    0,
                    origin,
                    origin + 4,
                    &[
                        (origin + 10, Some(origin + 30)),
                        (origin + 50, Some(origin + 70)),
                        (origin + 70, Some(origin + 70)),
                    ],
                ),
                tx(1, origin + 1, origin + 92, &[(origin + 90, None)]),
            ];
            let index = build(&records, None).unwrap();
            let map = PixelMap::new(
                Rect::from_xywh(0.0, 0.0, 16.0, 2.0),
                Viewport {
                    start: origin as f64,
                    end: origin as f64 + 32768.0,
                },
                RowView {
                    top: 0.0,
                    row_px: 0.5,
                },
                1.0,
            );
            let mut actual = map.mask();
            index.raster(&records, 0, &map, &mut actual);
            assert_eq!(actual, oracle(&records, 0, &map));
        }
        let records = vec![tx(
            0,
            0,
            80,
            &[
                (10, Some(30)),
                (50, Some(70)),
                (70, Some(70)),
                (80, Some(80)),
            ],
        )];
        let map = PixelMap::new(
            Rect::from_xywh(0.0, 0.0, 8.0, 1.0),
            Viewport {
                start: 0.0,
                end: 80.0,
            },
            RowView {
                top: 0.0,
                row_px: 1.0,
            },
            1.0,
        );
        let mut actual = map.mask();
        build(&records, None)
            .unwrap()
            .raster(&records, 0, &map, &mut actual);
        assert_eq!(
            (0..8).map(|x| actual.contains(x, 0)).collect::<Vec<_>>(),
            [false, true, true, false, false, true, true, true]
        );
    }

    #[test]
    fn lifetime_counts_and_nearest_targets_match_a_scan() {
        let records: Vec<_> = (0..1003)
            .map(|row| tx(row, row * 3, row * 3 + (row * 37) % 800, &[]))
            .collect();
        let index = build(&records, None).unwrap();
        for case in 0..200 {
            let view = Viewport {
                start: f64::from(case * 13) + 0.25,
                end: f64::from(case * 13 + case % 53) + 0.75,
            };
            let visible = f64::from(case * 7) + 0.13..f64::from(case * 7 + 70) + 0.87;
            let anchor = view.start + view.width() * 0.5;
            let center = (visible.start + visible.end) * 0.5;
            let actual = index.activity(&records, 11, view, visible.clone(), anchor, center);
            let mut expected = Activity::default();
            let mut best = (f64::INFINITY, f64::INFINITY, usize::MAX);
            for (row, tx) in records.iter().enumerate() {
                if (tx.end as f64) < view.start || tx.begin as f64 > view.end {
                    continue;
                }
                let row = row + 11;
                if row as f64 + 1.0 <= visible.start {
                    expected.above += 1;
                    expected.nearest_above = Some(row);
                } else if row as f64 >= visible.end {
                    expected.below += 1;
                    expected.nearest_below.get_or_insert(row);
                } else {
                    expected.visible += 1;
                }
                let score = (
                    (tx.begin as f64 - anchor)
                        .max(anchor - tx.end as f64)
                        .max(0.0),
                    (row as f64 + 0.5 - center).abs(),
                    row,
                );
                if score < best {
                    best = score;
                    expected.target = Some(row);
                }
            }
            assert_eq!(
                (
                    actual.above,
                    actual.visible,
                    actual.below,
                    actual.nearest_above,
                    actual.nearest_below,
                    actual.target
                ),
                (
                    expected.above,
                    expected.visible,
                    expected.below,
                    expected.nearest_above,
                    expected.nearest_below,
                    expected.target
                ),
                "case {case}"
            );
        }
    }

    #[test]
    fn optional_admission_and_missing_unions_preserve_exact_output() {
        let records: Vec<_> = (0..100)
            .map(|row| tx(row, row, row + 10, &[(row, Some(row + 2))]))
            .collect();
        let budget = MemoryBudget::new(0);
        assert!(build(&records, Some(&budget)).is_none());
        let bytes =
            records.len().div_ceil(BLOCK).next_power_of_two() * 2 * std::mem::size_of::<Node>();
        budget.set_limit(bytes as u64);
        let index = build(&records, Some(&budget)).unwrap();
        assert!(index.spans.is_empty());
        assert_eq!(budget.used(), bytes as u64);
        let map = PixelMap::new(
            Rect::from_xywh(0.37, 0.23, 23.0, 9.0),
            Viewport {
                start: 0.0,
                end: 120.0,
            },
            RowView {
                top: 0.0,
                row_px: 0.05,
            },
            1.5,
        );
        let mut actual = map.mask();
        index.raster(&records, 0, &map, &mut actual);
        assert_eq!(actual, oracle(&records, 0, &map));
        drop(index);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn cancelling_index_construction_releases_its_private_reservation() {
        use std::{
            future::{Future, poll_fn},
            task::{Context, Poll, Waker},
        };
        let records: Vec<_> = (0..1000).map(|row| tx(row, row, row + 10, &[])).collect();
        let budget = MemoryBudget::new(INDEX_LIMIT as u64);
        let mut checkpoint = || poll_fn(|_| Poll::<()>::Pending);
        let mut future = Box::pin(RowIndex::build(&records, Some(&budget), &mut checkpoint));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert!(budget.used() > 0);
        drop(future);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn placement_scales_geometry_and_lifetimes_with_the_source() {
        let mut records = vec![tx(0, 1, 4, &[(0, Some(2)), (9, None)]), tx(1, 3, 8, &[])];
        let mut index = build(&records, None).unwrap();
        index.scale_times(1000);
        for tx in &mut records {
            tx.begin *= 1000;
            tx.end *= 1000;
            for stage in &mut tx.stages {
                stage.begin *= 1000;
                stage.end = stage.end.map(|e| e * 1000);
            }
        }
        let map = PixelMap::new(
            Rect::from_xywh(0.37, 0.23, 23.0, 9.0),
            Viewport {
                start: 0.0,
                end: 10000.0,
            },
            RowView {
                top: 0.0,
                row_px: 0.13,
            },
            1.5,
        );
        let mut actual = map.mask();
        index.raster(&records, 0, &map, &mut actual);
        assert_eq!(actual, oracle(&records, 0, &map));
        let fresh = build(&records, None).unwrap();
        let activity = index.activity(
            &records,
            0,
            Viewport {
                start: 2000.0,
                end: 5000.0,
            },
            0.0..1.0,
            3000.0,
            0.5,
        );
        let expected = fresh.activity(
            &records,
            0,
            Viewport {
                start: 2000.0,
                end: 5000.0,
            },
            0.0..1.0,
            3000.0,
            0.5,
        );
        assert_eq!(
            (
                activity.above,
                activity.visible,
                activity.below,
                activity.target
            ),
            (
                expected.above,
                expected.visible,
                expected.below,
                expected.target
            )
        );
    }
}
