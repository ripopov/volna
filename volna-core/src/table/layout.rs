//! Exact integer row navigation with viewport-local pixel conversion.

use crate::geometry::Rect;

pub const ROW_HEIGHT: f32 = 24.0;
pub const HEADER_HEIGHT: f32 = 40.0;
pub const FILTER_HEIGHT: f32 = 36.0;
pub const MAX_PREPARED_ROWS: u64 = 256;
pub const MIN_COLUMN_WIDTH: f32 = 48.0;
pub const MAX_COLUMN_WIDTH: f32 = 16_384.0;
pub(super) const CELL_PADDING: f32 = 7.0;

#[derive(Clone, Copy, Debug)]
pub struct ColumnHeader {
    pub title: Rect,
    pub sort_icon: Rect,
    pub sort_target: Rect,
    pub format: Option<Rect>,
}

impl ColumnHeader {
    pub const FORMAT_WIDTH: f32 = 76.0;

    #[must_use]
    pub fn minimum_width(has_format: bool) -> f32 {
        if has_format { 144.0 } else { MIN_COLUMN_WIDTH }
    }

    pub(super) fn new(rect: Rect, has_format: bool, zoom: f32) -> Self {
        let pad = 8.0 * zoom;
        let center_y = rect.top() + rect.height() * 0.5;
        let right = rect.right() - pad;
        let format = has_format.then(|| {
            Rect::from_xywh(
                right - Self::FORMAT_WIDTH * zoom,
                center_y - 13.0 * zoom,
                Self::FORMAT_WIDTH * zoom,
                26.0 * zoom,
            )
        });
        let sort_right = format.map_or(right, |format| format.left() - pad);
        let sort_icon = Rect::from_xywh(
            sort_right - 16.0 * zoom,
            center_y - 8.0 * zoom,
            16.0 * zoom,
            16.0 * zoom,
        );
        Self {
            title: Rect::from_xywh(
                rect.left() + pad,
                rect.top(),
                (sort_icon.left() - pad - rect.left() - pad).max(0.0),
                rect.height(),
            ),
            sort_icon,
            sort_target: Rect::from_xywh(
                rect.left() + 4.0 * zoom,
                rect.top(),
                (sort_right - rect.left() - 4.0 * zoom).max(0.0),
                rect.height(),
            ),
            format,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RowViewport {
    pub top: u64,
    pub subrow_px: f32,
}

impl RowViewport {
    #[must_use]
    pub fn visible_rows(height: f32, row_height: f32) -> u64 {
        (height.max(0.0) / row_height.max(1.0)).ceil() as u64
    }
    #[must_use]
    pub fn full_rows(height: f32, row_height: f32) -> u64 {
        ((height.max(0.0) / row_height.max(1.0)).floor() as u64).max(1)
    }
    pub fn clamp(&mut self, rows: u64, height: f32, row_height: f32) {
        if !self.subrow_px.is_finite() {
            self.subrow_px = 0.0;
        }
        let last_top = rows.saturating_sub(Self::full_rows(height, row_height));
        self.top = self.top.min(last_top);
        self.subrow_px = self.subrow_px.clamp(0.0, row_height.max(1.0));
        if self.top == last_top {
            self.subrow_px = 0.0;
        }
    }
    pub fn scroll(&mut self, pixels: f32, rows: u64, height: f32, row_height: f32) {
        if !pixels.is_finite() {
            return;
        }
        let total = self.subrow_px + pixels;
        let whole = (total / row_height.max(1.0)).floor();
        let remainder = total - whole * row_height.max(1.0);
        if whole < 0.0 {
            let delta = (-whole) as u64;
            if delta > self.top {
                self.top = 0;
                self.subrow_px = 0.0;
            } else {
                self.top -= delta;
                self.subrow_px = remainder;
            }
        } else {
            self.top = self.top.saturating_add(whole as u64);
            self.subrow_px = remainder;
        }
        self.clamp(rows, height, row_height);
    }
    pub fn reveal(&mut self, row: u64, rows: u64, height: f32, row_height: f32) {
        let visible = Self::full_rows(height, row_height);
        if row < self.top {
            self.top = row;
        } else if row >= self.top.saturating_add(visible) {
            self.top = row.saturating_add(1).saturating_sub(visible);
        }
        self.subrow_px = 0.0;
        self.clamp(rows, height, row_height);
    }
    #[must_use]
    pub fn row_at(&self, y: f32, rows: u64, row_height: f32) -> Option<u64> {
        if !y.is_finite() || y < 0.0 {
            return None;
        }
        let local = ((y + self.subrow_px) / row_height.max(1.0)).floor() as u64;
        self.top.checked_add(local).filter(|&row| row < rows)
    }
    #[must_use]
    pub fn y_of(&self, row: u64, row_height: f32) -> f32 {
        let delta = if row >= self.top {
            (row - self.top) as f32
        } else {
            -((self.top - row) as f32)
        };
        delta * row_height - self.subrow_px
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ColumnRect {
    pub source_index: usize,
    pub prepared_index: usize,
    pub rect: Rect,
    pub resize: Rect,
    pub header: ColumnHeader,
    pub preview_bytes: usize,
}

#[derive(Clone, Debug, Default)]
pub struct TableLayout {
    pub bounds: Rect,
    pub header: Rect,
    pub body: Rect,
    pub filter_row: Rect,
    pub gutter: Rect,
    pub vertical_bar: Rect,
    pub vertical_thumb: Rect,
    pub horizontal_bar: Rect,
    pub horizontal_thumb: Rect,
    pub status: Rect,
    pub columns: Vec<ColumnRect>,
    pub row_height: f32,
    pub horizontal: f32,
    pub content_width: f32,
}
