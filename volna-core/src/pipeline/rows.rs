//! The vertical axis of a pipeline panel: which fractional row sits at the top edge of the
//! cells area and how tall a row is.

use crate::nav::Lerp;

const EDGE_SPACE: f64 = 0.2;
pub const ROW_PX_MIN: f32 = 1e-6;
pub const ROW_PX_MAX: f32 = 48.0;
pub const ROW_PX_DEFAULT: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RowView {
    pub top: f64,
    pub row_px: f32,
}

impl Default for RowView {
    fn default() -> Self {
        Self {
            top: 0.0,
            row_px: ROW_PX_DEFAULT,
        }
    }
}

impl RowView {
    pub fn zoomed(self, zoom: f32) -> Self {
        Self {
            top: self.top,
            row_px: self.row_px * zoom,
        }
    }

    pub fn unzoomed(self, zoom: f32) -> Self {
        Self {
            top: self.top,
            row_px: self.row_px / zoom,
        }
    }

    pub fn row_at(&self, y: f32) -> f64 {
        self.top + f64::from(y) / f64::from(self.row_px)
    }

    pub fn y_of(&self, row: f64) -> f32 {
        ((row - self.top) * f64::from(self.row_px)) as f32
    }

    pub fn zoom_about(&mut self, y: f32, factor: f64, height: f32, rows: usize, zoom: f32) {
        let anchor = self.row_at(y);
        let row_px = (f64::from(self.row_px) * factor)
            .clamp(f64::from(ROW_PX_MIN * zoom), f64::from(ROW_PX_MAX * zoom));
        self.row_px = row_px as f32;
        self.top = anchor - f64::from(y) / row_px;
        self.clamp(height, rows, zoom);
    }

    pub fn pan_px(&mut self, dy: f32, height: f32, rows: usize, zoom: f32) {
        self.top += f64::from(dy) / f64::from(self.row_px);
        self.clamp(height, rows, zoom);
    }

    pub fn clamp(&mut self, height: f32, rows: usize, zoom: f32) {
        if !self.row_px.is_finite() {
            self.row_px = ROW_PX_DEFAULT * zoom;
        }
        self.row_px = self.row_px.clamp(ROW_PX_MIN * zoom, ROW_PX_MAX * zoom);
        let visible = f64::from((height / self.row_px).max(1.0));
        let lo = -visible * EDGE_SPACE;
        let hi = (rows as f64 - visible * (1.0 - EDGE_SPACE)).max(0.0);
        if !self.top.is_finite() {
            self.top = 0.0;
        }
        self.top = self.top.clamp(lo, hi);
    }
}

impl Lerp for RowView {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        Self {
            top: self.top + (to.top - self.top) * t,
            row_px: (1.0
                / (1.0 / f64::from(self.row_px)
                    + (1.0 / f64::from(to.row_px) - 1.0 / f64::from(self.row_px)) * t))
                as f32,
        }
    }

    fn approx_eq(&self, other: &Self) -> bool {
        (self.top - other.top).abs() < 1e-6 && (self.row_px - other.row_px).abs() < 1e-6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_row_under_the_pointer() {
        let mut v = RowView {
            top: 10.0,
            row_px: 20.0,
        };
        let before = v.row_at(100.0);
        v.zoom_about(100.0, 2.0, 400.0, 1000, 1.0);
        assert_eq!(v.row_px, 40.0);
        assert!((v.row_at(100.0) - before).abs() < 1e-9);
        v.zoom_about(0.0, 1000.0, 400.0, 1000, 1.0);
        assert_eq!(v.row_px, ROW_PX_MAX);
        v.zoom_about(0.0, 1e-12, 400.0, 1000, 1.0);
        assert_eq!(v.row_px, ROW_PX_MIN);
        v.row_px = 20.0;
        v.zoom_about(0.0, 1000.0, 400.0, 1000, 2.0);
        assert_eq!(v.row_px, ROW_PX_MAX * 2.0);
    }

    #[test]
    fn clamp_allows_edge_space_only() {
        let mut v = RowView {
            top: -1000.0,
            row_px: 20.0,
        };
        v.clamp(400.0, 100, 1.0);
        assert_eq!(v.top, -4.0);
        v.top = 1e9;
        v.clamp(400.0, 100, 1.0);
        assert_eq!(v.top, 100.0 - 16.0);
        v.clamp(400.0, 5, 1.0);
        assert_eq!(v.top, 0.0);
        v.top = -1000.0;
        v.clamp(400.0, 5, 1.0);
        assert_eq!(v.top, -4.0);
        v.pan_px(40.0, 400.0, 100, 1.0);
        assert_eq!(v.top, -2.0);
        assert_eq!(v.y_of(-2.0), 0.0);
        assert_eq!(v.y_of(3.0), 100.0);
        let z = v.zoomed(2.0);
        assert_eq!(z.row_px, 40.0);
        assert_eq!(z.unzoomed(2.0), v);
    }
}
