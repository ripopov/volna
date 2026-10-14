//! Pixel layout of the pipeline panel for one frame: the header, the label column, the
//! cells area, the visible row range, and the small interactive rectangles (label divider,
//! Markers lane chips, retry).

use crate::pipeline::Activity;
use std::ops::Range;

use super::rows::RowView;
use crate::geometry::{Point, Rect, point, size};
use crate::marker::Marker;
use crate::wave::overlay::{LANE_H, MarkerLane, Measuring, SpanClocks, marker_lane};
use crate::wave::viewport::Viewport;

pub const LABEL_W_MIN: f32 = 60.0;
pub const LABEL_W_MAX: f32 = 420.0;
pub const LABEL_W_DEFAULT: f32 = 190.0;
const SPLITTER_TOLERANCE: f32 = 4.0;
const RETRY_W: f32 = 72.0;
const RETRY_H: f32 = 22.0;
pub const OVERVIEW_PX: f64 = 2.0;

#[derive(Clone, Debug, Default)]
pub struct PipelineLayout {
    pub activity_controls: Vec<(super::ActivityCommand, Rect, String)>,
    pub activity: super::activity::Activity,
    pub bounds: Rect,
    pub header: Rect,
    pub rulers: Rect,
    pub labels: Rect,
    pub cells: Rect,
    pub label_split: Rect,
    pub rows: RowView,
    pub row_range: Range<usize>,
    pub device_scale: f64,
    pub row_count: usize,
    pub zoom: f32,
    pub marker_lane: MarkerLane,
    pub retry: Option<Rect>,
}

#[derive(Clone, Copy)]
pub struct LayoutInput<'a> {
    pub bounds: Rect,
    pub metadata_width: Option<f32>,
    pub header_h: f32,
    pub ruler_h: f32,
    pub zoom: f32,
    pub label_width: f32,
    pub rows: RowView,
    pub row_count: usize,
    pub markers: &'a [Marker],
    pub spans: SpanClocks<'a>,
    pub measuring: Option<Measuring>,
    pub viewport: Viewport,
    pub failed: bool,
    pub device_scale: f64,
}

impl PipelineLayout {
    #[must_use]
    pub fn compute(input: LayoutInput<'_>) -> Self {
        let bounds = input.bounds;
        let zoom = if input.zoom.is_finite() && input.zoom > 0.0 {
            input.zoom
        } else {
            1.0
        };
        let z = |v: f32| v * zoom;
        let header_h = input.header_h;
        let labels_w = input.metadata_width.unwrap_or_else(|| {
            z(input.label_width.clamp(LABEL_W_MIN, LABEL_W_MAX))
                .min((bounds.width() - z(LABEL_W_MIN)).max(0.0))
        });
        let header = Rect::new(bounds.origin, size(bounds.width(), header_h));
        let ruler_h = input
            .ruler_h
            .clamp(0.0, (bounds.height() - header_h).max(0.0));
        let rulers = Rect::new(
            point(bounds.left(), header.bottom()),
            size(bounds.width(), ruler_h),
        );
        let lanes = if input.measuring.is_some() { 2.0 } else { 1.0 };
        let lane_h = (z(LANE_H) * lanes).min((bounds.height() - header_h - ruler_h).max(0.0));
        let lane = Rect::new(
            point(bounds.left(), rulers.bottom()),
            size(bounds.width(), lane_h),
        );
        let rows_top = lane.bottom();
        let rows_h = (bounds.bottom() - rows_top).max(0.0);
        let labels = Rect::new(point(bounds.left(), rows_top), size(labels_w, rows_h));
        let cells = Rect::new(
            point(labels.right(), rows_top),
            size((bounds.width() - labels_w).max(0.0), rows_h),
        );
        let mut rows = input.rows.zoomed(zoom);
        rows.clamp(rows_h, input.row_count, zoom);
        let first = rows.top.floor().max(0.0) as usize;
        let last = ((rows.top + f64::from(rows_h / rows.row_px)).ceil().max(0.0) as usize)
            .min(input.row_count);
        let row_range = first.min(last)..last;
        let marker_lane = marker_lane(
            lane,
            cells.left(),
            f64::from(cells.width()).max(1.0),
            input.viewport,
            input.markers,
            input.spans,
            input.measuring,
            zoom,
        );
        let retry = input.failed.then(|| {
            Rect::new(
                point(
                    cells.left() + (cells.width() - z(RETRY_W)) / 2.0,
                    rows_top + rows_h / 2.0 + z(20.0),
                ),
                size(z(RETRY_W), z(RETRY_H)),
            )
        });
        let label_split = Rect::new(
            point(labels.right() - z(SPLITTER_TOLERANCE), bounds.top()),
            size(z(2.0 * SPLITTER_TOLERANCE), bounds.height()),
        );
        Self {
            activity_controls: Vec::new(),
            activity: Activity::default(),
            bounds,
            header,
            rulers,
            labels,
            cells,
            label_split,
            rows,
            row_range,
            device_scale: input.device_scale,
            row_count: input.row_count,
            zoom,
            marker_lane,
            retry,
        }
    }

    #[must_use]
    pub fn row_at(&self, y: f32) -> Option<usize> {
        if y < self.cells.top() || y >= self.cells.bottom() {
            return None;
        }
        let row = self.rows.row_at(y - self.cells.top()).floor();
        (row >= 0.0 && (row as usize) < self.row_count).then_some(row as usize)
    }

    #[must_use]
    pub fn is_overview(&self) -> bool {
        f64::from(self.rows.row_px) * self.device_scale < OVERVIEW_PX
    }

    #[must_use]
    pub fn row_y(&self, ix: usize) -> f32 {
        self.cells.top() + self.rows.y_of(ix as f64)
    }

    #[must_use]
    pub fn cells_width_f64(&self) -> f64 {
        f64::from(self.cells.width()).max(1.0)
    }

    #[must_use]
    pub fn ruler_at(&self, p: Point, count: usize) -> Option<usize> {
        crate::wave::layout::ruler_row(self.rulers, p, count)
    }

    #[must_use]
    pub fn time_at(&self, viewport: &Viewport, x: f32) -> f64 {
        viewport.time_at(f64::from(x - self.cells.left()), self.cells_width_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wave::timeline::TimeBase;

    fn layout(zoom: f32, row_px: f32, rows: usize) -> PipelineLayout {
        PipelineLayout::compute(LayoutInput {
            metadata_width: None,
            bounds: Rect::from_xywh(0.0, 0.0, 1000.0, 332.0),
            header_h: 32.0 * zoom,
            ruler_h: 0.0,
            zoom,
            label_width: 190.0,
            rows: RowView { top: 3.0, row_px },
            row_count: rows,
            markers: &[Marker {
                id: crate::marker::MarkerId::new(1).unwrap(),
                time: 50,
                label: None,
            }],
            spans: SpanClocks::time_only(TimeBase::si(-9)),
            measuring: None,
            viewport: Viewport {
                start: 0.0,
                end: 100.0,
            },
            failed: false,
            device_scale: 1.0,
        })
    }

    #[test]
    fn rectangles_scale_with_zoom_and_preserve_source_rows() {
        for zoom in [1.0f32, 2.0] {
            let l = layout(zoom, 20.0, 1000);
            assert_eq!(l.labels.width(), 190.0 * zoom);
            assert_eq!(l.cells.left(), 190.0 * zoom);
            assert_eq!(l.rows.row_px, 20.0 * zoom);
            assert!(!l.is_overview());
            assert_eq!(l.row_range.start, 3);
            let visible = ((332.0 - (32.0 + LANE_H) * zoom) / (20.0 * zoom)).ceil() as usize;
            assert_eq!(l.row_range.end, 3 + visible);
            assert_eq!(l.row_at(l.cells.top() + 20.0 * zoom * 1.5), Some(4));
            assert_eq!(l.row_at(l.cells.top() - 1.0), None);
            assert_eq!(l.marker_lane.chips.len(), 1);
            assert_eq!(l.marker_lane.band.bottom(), l.cells.top());
            assert_eq!(l.label_split.width(), 8.0 * zoom);
        }
        let dense = layout(1.0, 0.5, 100_000);
        assert!(dense.is_overview());
        assert_eq!(dense.row_range.start, 3);
        let few = layout(1.0, 20.0, 5);
        assert_eq!(few.row_range, 0..5);
        assert_eq!(few.rows.top, 0.0, "rows that fit start at the top edge");
    }
}
