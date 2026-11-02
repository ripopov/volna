//! Transaction lanes: a generator shown as one row of the waveform panel, each record a bar
//! over its lifetime.

use crate::data::loaded_tracks::LoadedGenerator;
use crate::data::transaction_index::TransactionIndex;
use crate::document::{Document, TrackLoadState};
use crate::pipeline::{PipelineModel, TrackSource};
use crate::trace::{TraceId, Traced};
use crate::wave::model::RowHeight;
use crate::wave::viewport::Viewport;
use volna_trace::data::transactions::{Status, TrackRef, Transaction};

pub const RIBBON_H: f32 = 8.0;
pub const OCCUPANCY_ROWS: f32 = 2.0;

#[must_use]
pub fn occupancy_control(
    names: crate::geometry::Rect,
    top: f32,
    row_h: f32,
    zoom: f32,
) -> crate::geometry::Rect {
    let width = if names.width() >= 160.0 * zoom {
        88.0
    } else {
        24.0
    } * zoom;
    crate::geometry::Rect::from_xywh(
        names.right() - width - 6.0 * zoom,
        top + 3.0 * zoom,
        width,
        row_h - 6.0 * zoom,
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct TxLane {
    pub source: TrackSource,
    pub name: String,
    pub scope: String,
    pub height: RowHeight,
    pub auto_height: bool,
    pub occupancy: bool,
    pub first_sub_row: usize,
    pub tint: Option<super::Tint>,
}

pub enum LaneData<'a> {
    Ready(&'a LoadedGenerator),
    Loading,
    Failed(&'a str),
    Unresolved,
    Unavailable,
}

#[derive(Clone, Copy, Debug)]
pub struct RowScrollbar {
    pub track: crate::geometry::Rect,
    pub thumb: crate::geometry::Rect,
    pub maximum: usize,
}

impl RowScrollbar {
    pub(crate) fn first_at(&self, y: f32, grab: f32) -> usize {
        let travel = self.track.height() - self.thumb.height();
        let fraction = ((y - grab - self.track.top()) / travel).clamp(0.0, 1.0);
        (fraction * self.maximum as f32).round() as usize
    }
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
            occupancy: false,
            first_sub_row: 0,
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
            occupancy: false,
            first_sub_row: 0,
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

    #[must_use]
    pub fn summary_height(&self, doc: &Document, row_h: f32, zoom: f32) -> f32 {
        if self.generator(doc).is_some_and(|g| g.points().is_some()) {
            return 0.0;
        }
        if self.occupancy {
            OCCUPANCY_ROWS * row_h
        } else if self
            .generator(doc)
            .is_some_and(|g| g.depth() > capacity(self.height))
        {
            RIBBON_H * zoom
        } else {
            0.0
        }
    }

    #[must_use]
    pub fn layout_height(&self, doc: &Document, row_h: f32, zoom: f32) -> f64 {
        f64::from(self.height.multiple()) * f64::from(row_h)
            + f64::from(self.summary_height(doc, row_h, zoom))
    }

    #[must_use]
    pub fn visible_rows(&self) -> std::ops::Range<usize> {
        self.first_sub_row..self.first_sub_row + capacity(self.height)
    }

    #[must_use]
    pub fn scrollbar(
        &self,
        doc: &Document,
        layout: &super::layout::WaveLayout,
        position: usize,
    ) -> Option<RowScrollbar> {
        use crate::geometry::Rect;
        let generator = self.generator(doc).filter(|g| g.points().is_none())?;
        let rows = capacity(self.height);
        let maximum = generator.depth().saturating_sub(rows);
        let width = super::layout::SCROLLBAR_W * layout.zoom;
        if maximum == 0 || layout.values.width() < 2.0 * width {
            return None;
        }
        let track = Rect::from_xywh(
            layout.values_split.left() - width,
            layout.row_y(position) + 2.0 * layout.zoom,
            width,
            rows as f32 * layout.row_h - 4.0 * layout.zoom,
        );
        if track.height() <= 0.0 {
            return None;
        }
        let minimum = (18.0 * layout.zoom).min(track.height() / 2.0);
        let height = (track.height() * rows as f32 / generator.depth() as f32).max(minimum);
        let offset = self.first_sub_row.min(maximum) as f32 / maximum as f32;
        let thumb = Rect::from_xywh(
            track.left() + 2.0 * layout.zoom,
            track.top() + (track.height() - height) * offset,
            track.width() - 4.0 * layout.zoom,
            height,
        );
        Some(RowScrollbar {
            track,
            thumb,
            maximum,
        })
    }

    pub fn fit_height(&mut self, doc: &Document) -> bool {
        if !self.auto_height {
            return false;
        }
        let Some(depth) = self.generator(doc).map(LoadedGenerator::depth) else {
            return false;
        };
        self.auto_height = false;
        let height = if self.generator(doc).is_some_and(|g| g.points().is_some()) {
            RowHeight::PRESETS[4]
        } else {
            default_height(depth)
        };
        let changed = height != self.height;
        self.height = height;
        changed
    }
}

#[must_use]
pub fn capacity(height: RowHeight) -> usize {
    usize::from(height.multiple())
}

#[must_use]
pub fn default_height(depth: usize) -> RowHeight {
    RowHeight::try_from(depth.clamp(1, 4) as u8).unwrap()
}

#[must_use]
pub fn hidden_rows(depth: usize, height: RowHeight) -> usize {
    depth.saturating_sub(capacity(height))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneGeometry {
    pub top: f32,
    pub height: f32,
    pub sub_h: f32,
    pub capacity: usize,
}

impl LaneGeometry {
    #[must_use]
    pub fn new(top: f32, height: f32, row_h: f32, rows: RowHeight) -> Self {
        Self {
            top,
            height,
            sub_h: row_h,
            capacity: capacity(rows),
        }
    }
    #[must_use]
    pub fn sub_top(&self, sub: usize) -> f32 {
        self.top + self.sub_h * sub as f32
    }
    #[must_use]
    pub fn sub_at(&self, y: f32) -> Option<usize> {
        let offset = y - self.top;
        if offset < 0.0 || self.sub_h <= 0.0 {
            return None;
        }
        let sub = (offset / self.sub_h) as usize;
        (sub < self.capacity).then_some(sub)
    }
    #[must_use]
    pub fn occupancy_top(&self) -> f32 {
        self.sub_top(self.capacity)
    }
}

#[must_use]
pub fn hit(index: &TransactionIndex, sub: usize, time: f64, tolerance: f64) -> Option<usize> {
    let row = index.row(sub)?;
    let records = index.source().transactions();
    if let Some(i) = row.at(time.max(0.0).floor() as u64) {
        return Some(i);
    }
    row.window(time - tolerance, time + tolerance)
        .iter()
        .copied()
        .min_by(|&a, &b| {
            let distance = |i: usize| {
                (records[i].begin as f64 - time)
                    .abs()
                    .min((records[i].end as f64 - time).abs())
            };
            distance(a).total_cmp(&distance(b))
        })
}

#[must_use]
fn detail_records<'a>(
    index: &'a TransactionIndex,
    sub: usize,
    viewport: &Viewport,
) -> Option<&'a [usize]> {
    let row = index.row(sub)?;
    let candidates = row.window(viewport.start, viewport.end);
    (candidates.len() <= 64).then_some(candidates)
}

#[derive(Default)]
pub(crate) struct DetailCache {
    exact: std::collections::HashMap<(Traced<TrackRef>, usize), bool>,
}

pub(crate) struct RenderPlan<'a> {
    pub index: &'a TransactionIndex,
    pub details: Vec<Option<&'a [usize]>>,
}

impl<'a> RenderPlan<'a> {
    pub fn new(
        lane: &TxLane,
        index: &'a TransactionIndex,
        viewport: &Viewport,
        cache: &mut DetailCache,
    ) -> Self {
        if cache.exact.len() > 512 {
            cache.exact.clear();
        }
        let details = (lane.first_sub_row..lane.first_sub_row + capacity(lane.height))
            .map(|sub| {
                let candidates = detail_records(index, sub, viewport);
                let Some(track) = lane.track() else {
                    return candidates;
                };
                let exact = cache.exact.entry((track, sub)).or_insert(true);
                *exact = candidates.is_some_and(|records| *exact || records.len() <= 48);
                exact.then_some(candidates).flatten()
            })
            .collect();
        Self { index, details }
    }
}

pub(crate) enum Edges<'a> {
    Points(&'a [Transaction]),
    Transactions(&'a TransactionIndex),
}

impl Edges<'_> {
    pub fn next_boundary(&self, time: u64) -> Option<u64> {
        match self {
            Self::Points(records) => records
                .get(records.partition_point(|tx| tx.begin <= time))
                .map(|tx| tx.begin),
            Self::Transactions(index) => index.next_boundary(time),
        }
    }
    pub fn prev_boundary(&self, time: u64) -> Option<u64> {
        match self {
            Self::Points(records) => records
                .partition_point(|tx| tx.begin < time)
                .checked_sub(1)
                .map(|i| records[i].begin),
            Self::Transactions(index) => index.prev_boundary(time),
        }
    }
    pub fn nearest(&self, time: f64, tolerance: f64) -> Option<u64> {
        let tick = time.max(0.0).floor() as u64;
        [
            self.prev_boundary(tick.saturating_add(1)),
            self.next_boundary(tick.saturating_sub(1)),
        ]
        .into_iter()
        .flatten()
        .filter(|&t| (t as f64 - time).abs() <= tolerance)
        .min_by(|&a, &b| (a as f64 - time).abs().total_cmp(&(b as f64 - time).abs()))
    }
}

pub(crate) fn edges<'a>(lane: &TxLane, doc: &'a Document) -> Option<Edges<'a>> {
    let source = lane.generator(doc)?;
    if source.points().is_some() {
        return Some(Edges::Points(source.transactions()));
    }
    match doc.transaction_index(lane.track()?)? {
        crate::analysis::AnalysisLoad::Ready(index) => Some(Edges::Transactions(index)),
        _ => None,
    }
}

#[must_use]
pub fn label(tx: &Transaction) -> String {
    let label = PipelineModel::label(tx);
    if label.is_empty() {
        format!("#{}", tx.id.0)
    } else {
        crate::data::text::truncate_ref(label, crate::data::text::PREVIEW_BYTES)
    }
}

#[derive(Default)]
pub struct LaneReadout {
    pub text: String,
    pub failed: bool,
    pub truncated: bool,
}

#[must_use]
pub fn row_values(
    index: &TransactionIndex,
    cursor: u64,
    height: RowHeight,
    first: usize,
    limit: usize,
) -> Vec<LaneReadout> {
    use crate::data::text::truncate;
    (first..first + capacity(height))
        .map(|sub| {
            let tx = index
                .row(sub)
                .and_then(|row| row.at(cursor))
                .map(|i| &index.source().transactions()[i]);
            tx.map_or_else(LaneReadout::default, |tx| {
                let text = label(tx);
                LaneReadout {
                    truncated: text.len() > limit,
                    text: truncate(text, limit),
                    failed: tx.status == Status::Error,
                }
            })
        })
        .collect()
}

#[must_use]
pub fn value_text(
    generator: &LoadedGenerator,
    index: Option<&TransactionIndex>,
    cursor: u64,
    limit: usize,
) -> LaneReadout {
    use crate::data::text::{attribute_value_min_bytes, format_attribute, truncate};
    if let Some(index) = generator.points() {
        let records = generator.transactions();
        let first = records.partition_point(|r| r.begin < cursor);
        let end = records.partition_point(|r| r.begin <= cursor);
        let counts = index.counts(first..end);
        let mut truncated = false;
        let text = match end - first {
            0 => "–".into(),
            1 => index.kind.label(&records[first]).map_or_else(
                || format!("#{}", records[first].id.0),
                |value| {
                    truncated = attribute_value_min_bytes(value) > limit;
                    format_attribute(value, limit)
                },
            ),
            count => format!(
                "{count} {}",
                if index.kind == crate::data::points::PointKind::Log {
                    "messages"
                } else {
                    "events"
                }
            ),
        };
        return LaneReadout {
            truncated: truncated || text.len() > limit,
            text: truncate(text, limit),
            failed: counts[4] + counts[5] > 0,
        };
    }
    let Some(index) = index else {
        return LaneReadout::default();
    };
    let (count, failed) = index.occupancy.at(cursor);
    let text = format!("{count} active");
    LaneReadout {
        truncated: text.len() > limit,
        text: truncate(text, limit),
        failed,
    }
}
