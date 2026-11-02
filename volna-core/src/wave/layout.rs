//! Pixel layout of the wave panel for one frame: the columns, the visible row range,
//! the scrollbar, and the small interactive rectangles (format badges, Markers lane chips,
//! column dividers).

use std::ops::Range;
use std::sync::Arc;

use crate::geometry::{Point, Rect, point, size};
use crate::marker::Marker;
use crate::wave::overlay::{MarkerLane, Measuring, SpanClocks, marker_lane};
use crate::wave::viewport::Viewport;

pub const MIN_COLUMN: f32 = 72.0;
const SPLITTER_TOLERANCE: f32 = 4.0;
pub const SCROLLBAR_W: f32 = 10.0;
const BADGE_W: f32 = 44.0;
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
    pub gutter: Rect,
    pub rulers: Rect,
    pub names: Rect,
    pub name_left: f32,
    pub scopes: Rect,
    pub values: Rect,
    pub waves: Rect,
    pub shared_metadata: bool,
    pub rows: Range<usize>,
    pub visible: Arc<[u32]>,
    pub row_h: f32,
    tops: Arc<[f64]>,
    pub zoom: f32,
    pub scroll_y: f32,
    pub max_scroll: f32,
    pub scrollbar: Option<(Rect, Rect)>,
    pub badges: Vec<(usize, Rect)>,
    pub format_summaries: Vec<(usize, Rect)>,
    pub marker_lane: MarkerLane,
    pub names_split: Rect,
    pub scopes_split: Rect,
    pub values_split: Rect,
}

pub struct LayoutInput<'a> {
    pub bounds: Rect,
    pub margins: crate::panels::TimelineMargins,
    pub row_h: f32,
    pub header_h: f32,
    pub ruler_h: f32,
    pub zoom: f32,
    pub names_width: f32,
    pub scopes_width: f32,
    pub values_width: f32,
    pub trace_gutter: bool,
    pub row_tops: Arc<[f64]>,
    pub visible: Arc<[u32]>,
    pub scroll_y: f32,
    pub markers: &'a [Marker],
    pub spans: SpanClocks<'a>,
    pub measuring: Option<Measuring>,
    pub viewport: Viewport,
}

pub fn row_tops(heights: impl IntoIterator<Item = f64>) -> Arc<[f64]> {
    let mut top = 0.0;
    std::iter::once(0.0)
        .chain(heights.into_iter().map(|height| {
            top += height;
            top
        }))
        .collect()
}

fn row_at_offset(tops: &[f64], offset: f64, row_h: f32) -> usize {
    let count = tops.len().saturating_sub(1);
    let total = tops.last().copied().unwrap_or(0.0);
    if offset >= total {
        return count + ((offset - total) / f64::from(row_h)) as usize;
    }
    tops.partition_point(|top| *top <= offset).saturating_sub(1)
}

impl WaveLayout {
    #[must_use]
    pub fn compute(input: LayoutInput<'_>) -> WaveLayout {
        let bounds = input.margins.content(input.bounds);
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
        let available = (total_w - z(160.0)).max((2.0 * min_column).min(total_w));
        let metadata_w = input
            .margins
            .metadata_width
            .map(|w| w.max(0.0).min(total_w));
        let budget = metadata_w.unwrap_or(available);
        let min = min_column.min(budget / 2.0);
        let scope_min = min_column.min((budget - 2.0 * min).max(0.0));
        let names_w = z(input.names_width).clamp(min, (budget - scope_min - min).max(min));
        let scopes_w =
            z(input.scopes_width).clamp(scope_min, (budget - names_w - min).max(scope_min));
        let values_w = if let Some(metadata_w) = metadata_w {
            (metadata_w - names_w - scopes_w).max(0.0)
        } else {
            z(input.values_width).clamp(min, (budget - names_w - scopes_w).max(min))
        };

        let header = Rect::new(bounds.origin, size(bounds.width(), header_h));
        let ruler_h = input
            .ruler_h
            .clamp(0.0, (bounds.height() - header_h).max(0.0));
        let rulers = Rect::new(
            point(bounds.left(), header.bottom()),
            size(bounds.width(), ruler_h),
        );
        let metadata_w = names_w + scopes_w + values_w;
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
        let scopes = Rect::new(point(bounds.left(), rows_top), size(scopes_w, rows_h));
        let names = Rect::new(point(scopes.right(), rows_top), size(names_w, rows_h));
        let values = Rect::new(point(names.right(), rows_top), size(values_w, rows_h));
        let waves = Rect::new(point(waves_left, rows_top), size(waves_w, rows_h));

        let tops = input.row_tops;
        let item_count = tops.len().saturating_sub(1);
        let content_h = tops.last().copied().unwrap_or(0.0) as f32;
        let max_scroll = (content_h - rows_h).max(0.0);
        let scroll_y = if input.scroll_y.is_finite() {
            input.scroll_y.clamp(0.0, max_scroll)
        } else {
            0.0
        };

        let (first, last) = if row_h > 0.0 && item_count > 0 {
            let bottom = f64::from(scroll_y) + f64::from(rows_h);
            (
                row_at_offset(&tops, f64::from(scroll_y), row_h),
                tops[..item_count].partition_point(|top| *top < bottom),
            )
        } else {
            (0, 0)
        };
        let rows = first.min(last)..last;

        let mut badges = Vec::new();
        for ix in rows.clone() {
            let y = rows_top + (tops[ix] - f64::from(scroll_y)) as f32;
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
            shared_metadata: input.margins.metadata_width.is_some(),
            bounds: input.bounds,
            header,
            gutter: input.margins.gutter(input.bounds),
            rulers,
            names,
            name_left: names.left()
                + if input.trace_gutter {
                    z(TRACE_GUTTER)
                } else {
                    0.0
                },
            values,
            scopes,
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
            format_summaries: Vec::new(),
            marker_lane,
            names_split: split_zone(names.right()),
            scopes_split: split_zone(scopes.right()),
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
        let offset = f64::from(y) - f64::from(self.names.top()) + f64::from(self.scroll_y);
        (offset >= 0.0).then(|| row_at_offset(&self.tops, offset, self.row_h))
    }

    #[must_use]
    pub fn row_y(&self, ix: usize) -> f32 {
        let count = self.tops.len().saturating_sub(1);
        let total = self.tops.last().copied().unwrap_or(0.0);
        let offset = match self.tops.get(ix) {
            Some(top) => *top,
            None => total + (ix - count) as f64 * f64::from(self.row_h),
        };
        self.names.top() + (offset - f64::from(self.scroll_y)) as f32
    }

    #[must_use]
    pub fn row_height(&self, ix: usize) -> f32 {
        match (self.tops.get(ix), self.tops.get(ix + 1)) {
            (Some(top), Some(bottom)) => (bottom - top) as f32,
            _ => self.row_h,
        }
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
    pub fn format_at(&self, p: Point) -> Option<(usize, Rect)> {
        self.badge_at(p).or_else(|| {
            self.format_summaries
                .iter()
                .copied()
                .find(|(_, rect)| rect.contains(p))
        })
    }

    #[must_use]
    pub fn near_split(&self, p: Point) -> bool {
        self.names_split.contains(p)
            || self.scopes_split.contains(p)
            || self.values_split.contains(p)
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
    use crate::wave::model::RowHeight;
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
            margins: Default::default(),
            bounds: Rect::from_xywh(0.0, 0.0, 2000.0, 300.0),
            row_h: 24.0 * zoom,
            header_h: 32.0 * zoom,
            ruler_h: 0.0,
            zoom,
            names_width: 220.0,
            scopes_width: 160.0,
            values_width: 120.0,
            trace_gutter: false,
            row_tops: row_tops(std::iter::repeat_n(f64::from(24.0 * zoom), items)),
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
            assert_eq!(l.scopes.width(), 160.0 * zoom);
            assert_eq!(l.values.width(), 120.0 * zoom);
            assert_eq!(l.scopes.left(), l.bounds.left());
            assert_eq!(l.names.left(), l.scopes.right());
            assert_eq!(l.values.left(), l.names.right());
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
    fn large_scrolled_layouts_keep_fractional_heights_and_row_hits_aligned() {
        const COUNT: usize = 100_000;
        const FIRST: usize = 90_000;
        for row_h in [24.0f32, 33.6] {
            let height = f64::from(row_h) * 4.0 + 8.0;
            let tops = row_tops(std::iter::repeat_n(height, COUNT));
            assert!((tops[COUNT] - height * COUNT as f64).abs() < 1e-6);
            let scroll = (height * FIRST as f64) as f32;
            let layout = WaveLayout::compute(LayoutInput {
                margins: Default::default(),
                bounds: Rect::from_xywh(0.0, 0.0, 1400.0, 600.0),
                row_h,
                header_h: 32.0,
                ruler_h: 0.0,
                zoom: 1.0,
                names_width: 220.0,
                scopes_width: 160.0,
                values_width: 120.0,
                trace_gutter: false,
                row_tops: tops,
                visible: (0..COUNT as u32).collect(),
                scroll_y: scroll,
                markers: &[],
                spans: SpanClocks::time_only(TimeBase::si(-9)),
                measuring: None,
                viewport: Viewport {
                    start: 0.0,
                    end: 100.0,
                },
            });
            for row in FIRST..FIRST + 4 {
                let top = layout.row_y(row);
                let expected =
                    layout.names.top() + (height * row as f64 - f64::from(scroll)) as f32;
                assert!((top - expected).abs() < 1e-4);
                assert!((f64::from(layout.row_height(row)) - height).abs() < 1e-5);
                assert_eq!(layout.row_at(top + 0.25), Some(row));
                assert_eq!(
                    layout.row_at(top + layout.row_height(row) - 0.25),
                    Some(row)
                );
            }
        }
    }

    #[test]
    fn tall_rows_shift_every_row_below_and_hit_test_to_their_full_height() {
        let heights = [1, 4, 1, 8, 2].map(|h| RowHeight::try_from(h).unwrap());
        let input = |scroll_y| LayoutInput {
            margins: Default::default(),
            bounds: Rect::from_xywh(0.0, 0.0, 2000.0, 32.0 + 24.0 * 5.0),
            row_h: 24.0,
            header_h: 32.0,
            ruler_h: 0.0,
            zoom: 1.0,
            names_width: 220.0,
            scopes_width: 160.0,
            values_width: 120.0,
            trace_gutter: false,
            row_tops: row_tops(heights.map(|h| f64::from(h.multiple()) * 24.0)),
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
