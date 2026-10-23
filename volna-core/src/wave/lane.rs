//! Transaction lanes: a generator shown as one row of the waveform panel, each record a bar
//! over its lifetime.

use crate::data::loaded_tracks::LoadedGenerator;
use crate::document::{Document, TrackLoadState};
use crate::pipeline::{PipelineModel, TrackSource};
use crate::trace::{TraceId, Traced};
use crate::wave::model::RowHeight;
use crate::wave::viewport::Viewport;
use volna_trace::data::transactions::{Status, TrackRef, Transaction};

pub const DENSITY_PX: f64 = 4.0;
const SUB_ROW: f32 = 2.0 / 3.0;
const LANE_PAD: f32 = 1.0 / 8.0;
const DEFAULT_MAX: RowHeight = RowHeight::PRESETS[3];

#[derive(Clone, Debug, PartialEq)]
pub struct TxLane {
    pub source: TrackSource,
    pub name: String,
    pub scope: String,
    pub height: RowHeight,
    pub auto_height: bool,
    pub tint: Option<super::Tint>,
}

pub enum LaneData<'a> {
    Ready(&'a LoadedGenerator),
    Loading,
    Failed(&'a str),
    Unresolved,
    Unavailable,
}

impl TxLane {
    #[must_use]
    pub fn new(doc: &Document, track: Traced<TrackRef>) -> Option<Self> {
        let path = doc
            .session(track.trace)?
            .tracks()
            .iter()
            .find(|t| t.id == track.item)?
            .path
            .clone();
        let (name, scope) = path.split_last()?;
        let mut lane = Self {
            name: name.clone(),
            scope: scope.join("."),
            source: TrackSource::Resolved {
                track,
                path: path.clone(),
            },
            height: RowHeight::DEFAULT,
            auto_height: true,
            tint: None,
        };
        lane.fit_height(doc);
        Some(lane)
    }

    #[must_use]
    pub fn unresolved(trace: TraceId, path: Vec<String>, height: RowHeight) -> Self {
        let name = path.last().cloned().unwrap_or_default();
        let scope = path[..path.len().saturating_sub(1)].join(".");
        Self {
            source: TrackSource::Unresolved { trace, path },
            name,
            scope,
            height,
            auto_height: false,
            tint: None,
        }
    }

    #[must_use]
    pub fn track(&self) -> Option<Traced<TrackRef>> {
        self.source.track()
    }

    #[must_use]
    pub fn data<'a>(&self, doc: &'a Document) -> LaneData<'a> {
        let Some(track) = self.track() else {
            return LaneData::Unresolved;
        };
        match doc.track(track) {
            Some(TrackLoadState::Ready(loaded)) => loaded
                .generators
                .first()
                .map_or(LaneData::Unavailable, |g| LaneData::Ready(g)),
            Some(TrackLoadState::Loading) => LaneData::Loading,
            Some(TrackLoadState::Failed(error)) => LaneData::Failed(error),
            None => LaneData::Unavailable,
        }
    }

    #[must_use]
    pub fn generator<'a>(&self, doc: &'a Document) -> Option<&'a LoadedGenerator> {
        match self.data(doc) {
            LaneData::Ready(g) => Some(g),
            _ => None,
        }
    }

    pub fn fit_height(&mut self, doc: &Document) -> bool {
        if !self.auto_height {
            return false;
        }
        let Some(depth) = self.generator(doc).map(LoadedGenerator::depth) else {
            return false;
        };
        self.auto_height = false;
        let height = default_height(depth);
        let changed = height != self.height;
        self.height = height;
        changed
    }
}

#[must_use]
pub fn capacity(height: RowHeight) -> u16 {
    let units = f32::from(height.multiple());
    (((units - 2.0 * LANE_PAD) / SUB_ROW).floor() as u16).max(1)
}

#[must_use]
pub fn default_height(depth: u16) -> RowHeight {
    RowHeight::PRESETS
        .into_iter()
        .take_while(|h| *h != DEFAULT_MAX)
        .chain([DEFAULT_MAX])
        .find(|h| capacity(*h) >= depth)
        .unwrap_or(DEFAULT_MAX)
}

#[must_use]
pub fn folded(depth: u16, height: RowHeight) -> u16 {
    depth.saturating_sub(capacity(height))
}

#[must_use]
pub fn shown_sub_row(sub_row: u16, height: RowHeight) -> u16 {
    sub_row.min(capacity(height) - 1)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneGeometry {
    pub top: f32,
    pub height: f32,
    pub sub_h: f32,
    pub pad: f32,
    pub capacity: u16,
}

impl LaneGeometry {
    #[must_use]
    pub fn new(top: f32, height: f32, row_h: f32, rows: RowHeight) -> Self {
        Self {
            top,
            height,
            sub_h: row_h * SUB_ROW,
            pad: row_h * LANE_PAD,
            capacity: capacity(rows),
        }
    }

    #[must_use]
    pub fn sub_top(&self, sub: u16) -> f32 {
        self.top + self.pad + self.sub_h * f32::from(sub)
    }

    #[must_use]
    pub fn sub_at(&self, y: f32) -> Option<u16> {
        let offset = y - self.top - self.pad;
        if offset < 0.0 || self.sub_h <= 0.0 {
            return None;
        }
        let sub = (offset / self.sub_h) as u16;
        (sub < self.capacity).then_some(sub)
    }
}

#[must_use]
pub fn is_density(generator: &LoadedGenerator, px_per_unit: f64) -> bool {
    generator.median_lifetime() as f64 * px_per_unit < DENSITY_PX
}

#[must_use]
pub fn window(viewport: &Viewport) -> (u64, u64) {
    let start = viewport.start.max(0.0).floor() as u64;
    let end = viewport.end.max(0.0).ceil() as u64;
    (start, end.max(start))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DensityColumn {
    pub open: u32,
    pub failed: bool,
}

#[must_use]
pub fn density(
    generator: &LoadedGenerator,
    viewport: &Viewport,
    columns: usize,
) -> Vec<DensityColumn> {
    if columns == 0 {
        return Vec::new();
    }
    let mut open = vec![0i32; columns + 1];
    let mut failed = vec![0i32; columns + 1];
    let last = (columns - 1) as f64;
    let scale = viewport.px_per_unit(columns as f64);
    let column = |t: u64| ((t as f64 - viewport.start) * scale).clamp(0.0, last) as usize;
    let (start, end) = window(viewport);
    drop(generator.visit_window(start, end, |tx| {
        let (a, b) = (column(tx.begin), column(tx.end) + 1);
        open[a] += 1;
        open[b] -= 1;
        if tx.status == Status::Error {
            failed[a] += 1;
            failed[b] -= 1;
        }
        true
    }));
    let (mut n, mut f) = (0i32, 0i32);
    (0..columns)
        .map(|x| {
            n += open[x];
            f += failed[x];
            DensityColumn {
                open: n.max(0) as u32,
                failed: f > 0,
            }
        })
        .collect()
}

#[must_use]
pub fn fold_overlaps(
    generator: &LoadedGenerator,
    viewport: &Viewport,
    height: RowHeight,
) -> Vec<(u64, u64)> {
    let last = capacity(height) - 1;
    if generator.depth() <= capacity(height) {
        return Vec::new();
    }
    let (start, end) = window(viewport);
    let mut events = Vec::new();
    drop(generator.visit_window_ordinals(start, end, |ordinal, tx| {
        if generator.sub_row(ordinal) >= last && tx.end > tx.begin {
            events.push((tx.begin, 1i32));
            events.push((tx.end, -1));
        }
        true
    }));
    events.sort_unstable();
    let mut spans: Vec<(u64, u64)> = Vec::new();
    let (mut open, mut from) = (0i32, 0u64);
    for (time, delta) in events {
        let next = open + delta;
        if open < 2 && next >= 2 {
            from = time;
        } else if open >= 2 && next < 2 && time > from {
            match spans.last_mut() {
                Some(last) if last.1 >= from => last.1 = time,
                _ => spans.push((from, time)),
            }
        }
        open = next;
    }
    spans
}

#[must_use]
pub fn hit(
    generator: &LoadedGenerator,
    height: RowHeight,
    sub: u16,
    time: f64,
    tolerance: f64,
) -> Option<usize> {
    let start = (time - tolerance).max(0.0).floor() as u64;
    let end = (time + tolerance).max(0.0).ceil() as u64;
    let mut inside = None;
    let mut near: Option<(f64, usize)> = None;
    drop(generator.visit_window_ordinals(start, end, |ordinal, tx| {
        if shown_sub_row(generator.sub_row(ordinal), height) != sub {
            return true;
        }
        if tx.begin as f64 <= time && time <= tx.end as f64 {
            inside = Some(ordinal);
        }
        let d = (tx.begin as f64 - time)
            .abs()
            .min((tx.end as f64 - time).abs());
        if d <= tolerance && near.is_none_or(|(best, _)| d < best) {
            near = Some((d, ordinal));
        }
        true
    }));
    inside.or(near.map(|(_, ordinal)| ordinal))
}

#[must_use]
pub fn nearest_boundary(generator: &LoadedGenerator, time: f64, tolerance: f64) -> Option<u64> {
    let start = (time - tolerance).max(0.0).floor() as u64;
    let end = (time + tolerance).max(0.0).ceil() as u64;
    let mut best: Option<(f64, u64)> = None;
    drop(generator.visit_window(start, end, |tx| {
        for edge in [tx.begin, tx.end] {
            let d = (edge as f64 - time).abs();
            if d <= tolerance && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, edge));
            }
        }
        true
    }));
    best.map(|(_, edge)| edge)
}

#[must_use]
pub fn open_at(generator: &LoadedGenerator, time: u64) -> Vec<&Transaction> {
    let mut open = Vec::new();
    drop(generator.visit_window(time, time, |tx| {
        if time < tx.end || tx.begin == tx.end {
            open.push(tx);
        }
        true
    }));
    open
}

#[must_use]
pub fn label(tx: &Transaction) -> String {
    let label = PipelineModel::label(tx);
    if label.is_empty() {
        format!("#{}", tx.id.0)
    } else {
        label
    }
}

#[must_use]
pub fn value_text(generator: &LoadedGenerator, cursor: u64) -> (String, bool) {
    let open = open_at(generator, cursor);
    let failed = open.iter().any(|tx| tx.status == Status::Error);
    let text = match open.as_slice() {
        [] => "–".to_owned(),
        [tx] => label(tx),
        many => format!(
            "{} open: {}",
            many.len(),
            many.iter()
                .map(|tx| label(tx))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    };
    (text, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use volna_trace::data::transactions::{SpanKind, TransactionRef};

    fn tx(id: u64, begin: u64, end: u64, status: Status) -> Transaction {
        Transaction {
            id: TransactionRef(id),
            generator: TrackRef(1),
            begin,
            end,
            status,
            kind: SpanKind::Producer,
            parent: None,
            attributes: vec![],
            events: vec![],
            stages: vec![],
        }
    }

    fn episode() -> LoadedGenerator {
        LoadedGenerator::new(
            TrackRef(1),
            vec![
                tx(0, 2, 20, Status::Ok),
                tx(1, 8, 26, Status::Ok),
                tx(2, 14, 40, Status::Error),
                tx(3, 22, 34, Status::Ok),
            ],
            HashMap::new(),
            vec![],
        )
        .unwrap()
    }

    #[test]
    fn heights_hold_whole_sub_rows_and_default_to_the_depth() {
        let caps: Vec<_> = RowHeight::PRESETS.into_iter().map(capacity).collect();
        assert_eq!(caps, [1, 2, 4, 5, 11]);
        assert_eq!(default_height(0).multiple(), 1);
        assert_eq!(default_height(1).multiple(), 1);
        assert_eq!(default_height(3).multiple(), 3);
        assert_eq!(default_height(5).multiple(), 4);
        assert_eq!(default_height(40).multiple(), 4, "capped at 4×");
        assert_eq!(folded(7, RowHeight::PRESETS[3]), 2);
        assert_eq!(shown_sub_row(6, RowHeight::DEFAULT), 0);
        let g = LaneGeometry::new(100.0, 24.0, 24.0, RowHeight::DEFAULT);
        assert_eq!(g.sub_at(104.0), Some(0));
        assert_eq!(g.sub_at(101.0), None);
        assert_eq!(g.sub_at(121.0), None);
    }

    #[test]
    fn density_fold_hit_and_value_queries() {
        let g = episode();
        assert_eq!(
            (0..4).map(|i| g.sub_row(i)).collect::<Vec<_>>(),
            [0, 1, 2, 0]
        );
        assert_eq!(g.depth(), 3);
        let vp = Viewport {
            start: 0.0,
            end: 60.0,
        };
        let d = density(&g, &vp, 60);
        assert_eq!(d[19].open, 3);
        assert!(d[39].failed && !d[41].failed);
        assert_eq!(d[50], DensityColumn::default());
        assert_eq!(fold_overlaps(&g, &vp, RowHeight::DEFAULT), [(8, 34)]);
        assert!(fold_overlaps(&g, &vp, RowHeight::PRESETS[2]).is_empty());
        let three = RowHeight::PRESETS[2];
        assert_eq!(hit(&g, three, 2, 19.0, 0.0), Some(2));
        assert_eq!(hit(&g, RowHeight::DEFAULT, 0, 19.0, 0.0), Some(2));
        assert_eq!(hit(&g, three, 0, 21.0, 0.0), None);
        assert_eq!(hit(&g, three, 0, 21.0, 1.5), Some(0));
        assert_eq!(nearest_boundary(&g, 21.2, 2.0), Some(22));
        assert_eq!(nearest_boundary(&g, 50.0, 2.0), None);
        assert_eq!(value_text(&g, 5), ("#0".into(), false));
        assert_eq!(value_text(&g, 36), ("#2".into(), true));
        assert_eq!(value_text(&g, 45).0, "–");
        assert!(value_text(&g, 19).0.starts_with("3 open"));
        assert!(is_density(&g, 0.1) && !is_density(&g, 1.0));
    }
}
