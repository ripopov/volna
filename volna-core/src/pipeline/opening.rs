//! Initial framing from pixel geometry and the recording's current time units.

use super::{PipelineModel, model::START_MARGIN_PX, rows::ROW_PX_DEFAULT};
use crate::wave::viewport::Viewport;
use volna_trace::data::transactions::Transaction;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportSize {
    pub width: f64,
    pub zoom: f32,
}

impl ViewportSize {
    pub(crate) fn is_valid(self) -> bool {
        self.width.is_finite() && self.width > 0.0 && self.zoom.is_finite() && self.zoom > 0.0
    }
}

#[derive(Default)]
pub(super) struct OpeningView {
    pub size: Option<ViewportSize>,
}

pub(super) fn fallback_time_unit(first: &Transaction) -> f64 {
    let mut durations: Vec<_> = first
        .stages
        .iter()
        .map(|stage| PipelineModel::stage_end(first, stage).saturating_sub(stage.begin))
        .filter(|&duration| duration > 0)
        .collect();
    durations.sort_unstable();
    durations
        .get(durations.len() / 2)
        .copied()
        .unwrap_or_else(|| first.end.saturating_sub(first.begin).max(1)) as f64
}

pub(super) fn viewport(begin: u64, unit: f64, size: ViewportSize, limits: (u64, u64)) -> Viewport {
    let zoom = f64::from(size.zoom);
    let side = f64::from(ROW_PX_DEFAULT) * zoom;
    let width = size.width / side * unit;
    let margin = (START_MARGIN_PX * zoom).min(size.width * 0.1);
    let start = begin as f64 - margin / side * unit;
    let mut viewport = Viewport {
        start,
        end: start + width,
    };
    viewport.clamp_preserving_width(limits);
    viewport
}
