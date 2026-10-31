//! Pixel layout of the wave panel for one frame: the three columns, the visible row range,
//! the scrollbar, and the small interactive rectangles (format badges, Markers lane chips,
//! column dividers).

use std::ops::Range;
use std::sync::Arc;

use crate::geometry::{Point, Rect, point, size};
use crate::marker::Marker;
use crate::wave::model::RowHeight;
use crate::wave::overlay::{MarkerLane, Measuring, SpanClocks, marker_lane};
use crate::wave::viewport::Viewport;

pub const MIN_COLUMN: f32 = 72.0;
const SPLITTER_TOLERANCE: f32 = 4.0;
pub const SCROLLBAR_W: f32 = 10.0;
const BADGE_W: f32 = 36.0;
pub const NAME_PAD: f32 = 12.0;
pub const INDENT: f32 = 14.0;
pub const CHEVRON_W: f32 = 16.0;

pub const TRACE_GUTTER: f32 = 20.0;

#[must_use]
pub fn indent_x(left: f32, depth: u8, zoom: f32) -> f32 {
    left + (NAME_PAD + INDENT * f32::from(depth)) * zoom
}

#[derive(Clone, Debug, Default)]
pub struct WaveLayout {
    pub bounds: Rect,
    pub header: Rect,
    pub rulers: Rect,
    pub names: Rect,
    pub name_left: f32,
    pub values: Rect,
    pub waves: Rect,
    pub shared_metadata: bool,
    pub rows: Range<usize>,
    pub visible: Arc<[u32]>,
    pub row_h: f32,
    tops: Arc<[u32]>,
    pub zoom: f32,
    pub scroll_y: f32,
    pub max_scroll: f32,
    pub scrollbar: Option<(Rect, Rect)>,
    pub badges: Vec<(usize, Rect)>,
    pub marker_lane: MarkerLane,
    pub names_split: Rect,
    pub values_split: Rect,
}

pub struct LayoutInput<'a> {
    pub bounds: Rect,
    pub metadata_width: Option<f32>,
    pub row_h: f32,
    pub header_h: f32,
    pub ruler_h: f32,
    pub zoom: f32,
    pub names_width: f32,
    pub values_width: f32,
    pub trace_gutter: bool,
    pub row_tops: Arc<[u32]>,
    pub visible: Arc<[u32]>,
    pub scroll_y: f32,
    pub markers: &'a [Marker],
    pub spans: SpanClocks<'a>,
    pub measuring: Option<Measuring>,
    pub viewport: Viewport,
}

pub fn row_tops(heights: impl IntoIterator<Item = RowHeight>) -> Arc<[u32]> {
    let mut top = 0u32;
    std::iter::once(0)
        .chain(heights.into_iter().map(|h| {
            top += u32::from(h.multiple());
            top
        }))
        .collect()
}

fn row_at_unit(tops: &[u32], unit: f32) -> usize {
    let count = tops.len().saturating_sub(1);
    let total = tops.last().copied().unwrap_or(0);
    if unit >= total as f32 {
        return count + (unit - total as f32) as usize;
    }
    tops.partition_point(|top| *top as f32 <= unit)
        .saturating_sub(1)
}

impl WaveLayout {
    #[must_use]
    pub fn compute(input: LayoutInput<'_>) -> WaveLayout {
        let bounds = input.bounds;
        let row_h = input.row_h;
        let header_h = input.header_h;
        let zoom = if input.zoom.is_finite() && input.zoom > 0.0 {
            input.zoom
        } else {
            1.0
        };
        let z = |v: f32| v * zoom;
        let min_column = z(MIN_COLUMN);
        let total_w = bounds.width();
        let (names_w, values_w) = if let Some(metadata_w) = input.metadata_width {
            let min = min_column.min(metadata_w / 2.0);
            let names_w = z(input.names_width).clamp(min, metadata_w - min);
            (names_w, metadata_w - names_w)
        } else {
            let names_w = z(input.names_width).clamp(
                min_column,
                (total_w - 2.0 * min_column - z(160.0)).max(min_column),
            );
            let values_w = z(input.values_width)
                .clamp(min_column, (total_w - names_w - z(160.0)).max(min_column));
            (names_w, values_w)
        };

        let header = Rect::new(bounds.origin, size(bounds.width(), header_h));
        let ruler_h = input
            .ruler_h
            .clamp(0.0, (bounds.height() - header_h).max(0.0));
        let rulers = Rect::new(
            point(bounds.left(), header.bottom()),
            size(bounds.width(), ruler_h),
        );
        let metadata_w = input.metadata_width.unwrap_or(names_w + values_w);
        let waves_left = bounds.left() + metadata_w;
        let waves_w = (bounds.width() - metadata_w).max(0.0);
        let marker_lane = marker_lane(
            Rect::new(
                point(bounds.left(), rulers.bottom()),
                size(bounds.width(), bounds.bottom() - rulers.bottom()),
            ),
            waves_left,
            f64::from(waves_w).max(1.0),
            input.viewport,
            input.markers,
            input.spans,
            input.measuring,
            zoom,
        );
        let rows_top = marker_lane.strip().bottom();
        let rows_h = (bounds.bottom() - rows_top).max(0.0);
        let names = Rect::new(point(bounds.left(), rows_top), size(names_w, rows_h));
        let values = Rect::new(point(names.right(), rows_top), size(values_w, rows_h));
        let waves = Rect::new(point(waves_left, rows_top), size(waves_w, rows_h));

        let tops = input.row_tops;
        let item_count = tops.len().saturating_sub(1);
        let content_h = row_h * tops.last().copied().unwrap_or(0) as f32;
        let max_scroll = (content_h - rows_h).max(0.0);
        let scroll_y = if input.scroll_y.is_finite() {
            input.scroll_y.clamp(0.0, max_scroll)
        } else {
            0.0
        };

        let (first, last) = if row_h > 0.0 && item_count > 0 {
            let bottom = (scroll_y + rows_h) / row_h;
            (
                row_at_unit(&tops, scroll_y / row_h),
                tops[..item_count].partition_point(|top| (*top as f32) < bottom),
            )
        } else {
            (0, 0)
        };
        let rows = first.min(last)..last;

        let mut badges = Vec::new();
        for ix in rows.clone() {
            let y = rows_top + row_h * tops[ix] as f32 - scroll_y;
            let b = Rect::new(
                point(values.right() - z(BADGE_W + 6.0), y + z(4.0)),
                size(z(BADGE_W), row_h - z(8.0)),
            );
            if let Some(&entry) = input.visible.get(ix) {
                badges.push((entry as usize, b));
            }
        }

        let scrollbar = if max_scroll > 0.0 && rows_h > 0.0 {
            let track = Rect::new(
                point(waves.right() - z(SCROLLBAR_W), rows_top),
                size(z(SCROLLBAR_W), rows_h),
            );
            let ratio = rows_h / content_h;
            let thumb_h = (rows_h * ratio).max(z(24.0)).min(rows_h);
            let travel = rows_h - thumb_h;
            let top = rows_top + travel * (scroll_y / max_scroll).clamp(0.0, 1.0);
            let thumb = Rect::new(
                point(track.left() + z(2.0), top),
                size(z(SCROLLBAR_W - 4.0), thumb_h),
            );
            Some((track, thumb))
        } else {
            None
        };

        let split_zone = |x: f32| {
            Rect::new(
                point(x - z(SPLITTER_TOLERANCE), bounds.top()),
                size(z(2.0 * SPLITTER_TOLERANCE), bounds.height()),
            )
        };

        WaveLayout {
            shared_metadata: input.metadata_width.is_some(),
            bounds,
            header,
            rulers,
            names,
            name_left: names.left()
                + if input.trace_gutter {
                    z(TRACE_GUTTER)
                } else {
                    0.0
                },
            values,
            waves,
            rows,
            visible: input.visible,
            row_h,
            tops,
            zoom,
            scroll_y,
            max_scroll,
            scrollbar,
            badges,
            marker_lane,
            names_split: split_zone(names.right()),
            values_split: split_zone(values.right()),
        }
    }

    #[must_use]
    pub fn row_at(&self, y: f32) -> Option<usize> {
        if y < self.names.top() {
            return None;
        }
        if self.row_h <= 0.0 {
            return None;
        }
        let unit = (y - self.names.top() + self.scroll_y) / self.row_h;
        (unit >= 0.0).then(|| row_at_unit(&self.tops, unit))
    }

    #[must_use]
    pub fn row_y(&self, ix: usize) -> f32 {
        let count = self.tops.len().saturating_sub(1);
        let total = self.tops.last().copied().unwrap_or(0);
        let unit = match self.tops.get(ix) {
            Some(top) => *top as f32,
            None => (total as usize + ix - count) as f32,
        };
        self.names.top() + self.row_h * unit - self.scroll_y
    }

    #[must_use]
    pub fn row_height(&self, ix: usize) -> f32 {
        let units = match (self.tops.get(ix), self.tops.get(ix + 1)) {
            (Some(top), Some(bottom)) => bottom - top,
            _ => 1,
        };
        self.row_h * units as f32
    }

    #[must_use]
    pub fn entry(&self, pos: usize) -> Option<usize> {
        self.visible.get(pos).map(|&i| i as usize)
    }

    #[must_use]
    pub fn position(&self, entry: usize) -> Option<usize> {
        self.visible.binary_search(&(entry as u32)).ok()
    }

    #[must_use]
    pub fn entry_at(&self, y: f32) -> Option<usize> {
        self.entry(self.row_at(y)?)
    }

    #[must_use]
    pub fn entry_span(&self, entry: usize) -> Option<(f32, f32)> {
        let pos = self.position(entry)?;
        Some((self.row_y(pos), self.row_height(pos)))
    }

    #[must_use]
    pub fn badge_at(&self, p: Point) -> Option<(usize, Rect)> {
        self.badges.iter().copied().find(|(_, b)| b.contains(p))
    }

    #[must_use]
    pub fn near_split(&self, p: Point) -> bool {
        self.names_split.contains(p) || self.values_split.contains(p)
    }

    #[must_use]
    pub fn wave_width_f64(&self) -> f64 {
        f64::from(self.waves.width()).max(1.0)
    }

    #[must_use]
    pub fn ruler_at(&self, p: Point, count: usize) -> Option<usize> {
        ruler_row(self.rulers, p, count)
    }
}

#[must_use]
pub fn ruler_row(band: Rect, p: Point, count: usize) -> Option<usize> {
    if count == 0 || !band.contains(p) {
        return None;
    }
    Some((((p.y - band.top()) / band.height() * count as f32) as usize).min(count - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wave::overlay::{CHIP_H, LANE_H};
    use crate::wave::timeline::TimeBase;

    fn marker() -> Marker {
        Marker {
            id: crate::marker::MarkerId::new(1).unwrap(),
            time: 50,
            label: None,
        }
    }

    fn layout(zoom: f32, items: usize) -> WaveLayout {
        layout_with(&[marker()], None, zoom, items)
    }

    fn layout_with(
        markers: &[Marker],
        measuring: Option<Measuring>,
        zoom: f32,
        items: usize,
    ) -> WaveLayout {
        WaveLayout::compute(LayoutInput {
            metadata_width: None,
            bounds: Rect::from_xywh(0.0, 0.0, 2000.0, 300.0),
            row_h: 24.0 * zoom,
            header_h: 32.0 * zoom,
            ruler_h: 0.0,
            zoom,
            names_width: 220.0,
            values_width: 120.0,
            trace_gutter: false,
            row_tops: row_tops(vec![RowHeight::DEFAULT; items]),
            visible: (0..items as u32).collect(),
            scroll_y: 0.0,
            markers,
            spans: SpanClocks::time_only(TimeBase::si(-9)),
            measuring,
            viewport: Viewport {
                start: 0.0,
                end: 100.0,
            },
        })
    }

    #[test]
    fn marker_and_measure_rows_only_take_space_when_present() {
        let empty = layout_with(&[], None, 1.0, 10);
        assert_eq!(empty.marker_lane.band.height(), 0.0);
        assert_eq!(empty.marker_lane.strip().height(), 0.0);
        assert_eq!(empty.names.top(), empty.rulers.bottom());

        let measuring = Some(Measuring {
            reference: 25,
            cursor: Some(75),
        });
        let measured = layout_with(&[], measuring, 1.0, 10);
        assert_eq!(measured.marker_lane.band.height(), 0.0);
        let measure = measured.marker_lane.measure.as_ref().unwrap();
        assert_eq!(measured.marker_lane.strip(), measure.band);
        assert_eq!(measured.names.top(), measure.band.bottom());

        let both = layout_with(&[marker()], measuring, 1.0, 10);
        assert_eq!(both.marker_lane.band.height(), LANE_H);
        assert_eq!(both.marker_lane.strip().height(), 2.0 * LANE_H);
        assert_eq!(both.names.top(), both.marker_lane.strip().bottom());
    }

    #[test]
    fn every_rectangle_scales_with_the_interface_zoom() {
        let base = layout(1.0, 100);
        for zoom in [0.5f32, 2.0] {
            let l = layout(zoom, 100);
            assert_eq!(l.zoom, zoom);
            assert_eq!(l.row_h, 24.0 * zoom);
            assert_eq!(l.header.height(), 32.0 * zoom);
            assert_eq!(l.names.width(), 220.0 * zoom);
            assert_eq!(l.values.width(), 120.0 * zoom);
            assert_eq!(l.names_split.width(), 8.0 * zoom);
            let (_, b) = l.badges[0];
            assert_eq!(b.width(), BADGE_W * zoom);
            assert_eq!(b.height(), (24.0 - 8.0) * zoom);
            assert_eq!(l.marker_lane.band.height(), LANE_H * zoom);
            assert_eq!(l.names.top(), (32.0 + LANE_H) * zoom);
            let chip = &l.marker_lane.chips[0];
            assert_eq!(chip.markers, 0..1);
            assert_eq!(chip.rect.height(), CHIP_H * zoom);
            assert_eq!(chip.rect.left(), l.waves.left() + l.waves.width() / 2.0);
            let (track, _) = l.scrollbar.unwrap();
            assert_eq!(track.width(), SCROLLBAR_W * zoom);
            let visible = (300.0 - (32.0 + LANE_H) * zoom) / (24.0 * zoom);
            assert_eq!(l.rows.len(), visible.ceil() as usize);
            assert_eq!(l.row_at(l.names.top() + 24.0 * zoom * 3.5), Some(3));
        }
        assert_eq!(base.zoom, 1.0);
        assert_eq!(base.names_split.width(), 8.0);
        let cramped = layout(5.0, 100);
        let (track, thumb) = cramped.scrollbar.unwrap();
        assert!(thumb.height() <= track.height());
        assert!(thumb.top() >= track.top() && thumb.bottom() <= track.bottom());
    }

    #[test]
    fn tall_rows_shift_every_row_below_and_hit_test_to_their_full_height() {
        let heights = [1, 4, 1, 8, 2].map(|h| RowHeight::try_from(h).unwrap());
        let input = |scroll_y| LayoutInput {
            metadata_width: None,
            bounds: Rect::from_xywh(0.0, 0.0, 2000.0, 32.0 + 24.0 * 5.0),
            row_h: 24.0,
            header_h: 32.0,
            ruler_h: 0.0,
            zoom: 1.0,
            names_width: 220.0,
            values_width: 120.0,
            trace_gutter: false,
            row_tops: row_tops(heights),
            visible: (0..5).collect(),
            scroll_y,
            markers: &[],
            spans: SpanClocks::time_only(TimeBase::si(-9)),
            measuring: None,
            viewport: Viewport {
                start: 0.0,
                end: 100.0,
            },
        };
        let l = WaveLayout::compute(input(0.0));
        let top = l.names.top();
        assert_eq!(
            (0..5).map(|ix| l.row_y(ix) - top).collect::<Vec<_>>(),
            [0.0, 24.0, 120.0, 144.0, 336.0]
        );
        assert_eq!(l.row_height(1), 96.0);
        assert_eq!(l.row_height(3), 192.0);
        assert_eq!(l.rows, 0..2);
        assert_eq!(l.badges.len(), 2);
        assert_eq!(l.badges[1].1.top(), top + 24.0 + 4.0);
        assert_eq!(l.row_at(top + 24.0 + 95.0), Some(1));
        assert_eq!(l.row_at(top + 120.0), Some(2));
        assert_eq!(l.max_scroll, 24.0 * 16.0 - 24.0 * 5.0);
        assert_eq!(l.row_at(top + 384.0 + 30.0 - l.scroll_y), Some(6));

        let l = WaveLayout::compute(input(24.0 * 8.0));
        assert_eq!(l.rows, 3..4);
        assert_eq!(l.row_y(3), l.names.top() - 48.0);
    }
}
