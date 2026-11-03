//! Zoom gesture geometry and layout validity for pipeline cells and occupancy.

use super::PipelineLayout;
use crate::geometry::{Point, Rect, point};

const MIN_SIZE: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ZoomTarget {
    Cells,
    Occupancy,
}

impl ZoomTarget {
    fn area(self, layout: &PipelineLayout) -> Rect {
        match self {
            Self::Cells => layout.cells,
            Self::Occupancy => layout.occupancy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomSelection {
    target: ZoomTarget,
    start: Point,
    current: Point,
    area: Rect,
    zoom: f32,
}

pub(super) struct ZoomPreview {
    pub area: Rect,
    pub rect: Rect,
    pub valid: bool,
}

impl ZoomSelection {
    pub(super) fn new(target: ZoomTarget, start: Point, layout: &PipelineLayout) -> Self {
        Self {
            target,
            start,
            current: start,
            area: target.area(layout),
            zoom: layout.zoom,
        }
    }

    pub(super) fn target(self) -> ZoomTarget {
        self.target
    }

    pub(super) fn move_to(&mut self, current: Point) {
        self.current = current;
    }

    pub(super) fn matches_layout(self, layout: &PipelineLayout) -> bool {
        self.area == self.target.area(layout) && self.zoom == layout.zoom
    }

    pub(super) fn preview(self) -> ZoomPreview {
        let area = self.area;
        let mut a = point(
            self.start.x.clamp(area.left(), area.right()),
            self.start.y.clamp(area.top(), area.bottom()),
        );
        let mut b = point(
            self.current.x.clamp(area.left(), area.right()),
            self.current.y.clamp(area.top(), area.bottom()),
        );
        if self.target == ZoomTarget::Occupancy {
            a.y = area.top();
            b.y = area.bottom();
        }
        let rect = Rect::from_xywh(
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs(),
            (a.y - b.y).abs(),
        );
        let min = MIN_SIZE * self.zoom;
        ZoomPreview {
            area,
            rect,
            valid: rect.width() >= min
                && (self.target == ZoomTarget::Occupancy || rect.height() >= min),
        }
    }
}
