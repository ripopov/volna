//! Source-aligned time bins for bounded digital waveform sampling.

use crate::wave::viewport::Viewport;
use volna_trace::data::SignalHistory;

// Power-of-two tick bins keep density decisions independent of viewport position.
// Integer boundaries also avoid rounding a transition into an adjacent bin.
pub(super) struct SamplingGrid {
    first: i128,
    step: i128,
    pub(super) pixel: f32,
    pub(super) left: f32,
    pub(super) len: usize,
}

impl SamplingGrid {
    pub(super) fn new(vp: &Viewport, width: f64) -> Self {
        let unit = vp.width() / width;
        // Subtracting panned endpoints can move an exact LOD boundary by a few ulps.
        let uncertainty = 4.0 * f64::EPSILON * vp.start.abs().max(vp.end.abs()) / width;
        let shift = (unit + uncertainty).log2().floor().clamp(0.0, 63.0) as u32;
        let step = 1_i128 << shift;
        let first = (vp.start.floor() as i128).div_euclid(step) * step;
        let relative = (vp.start.floor() as i128 - first) as f64 + (vp.start - vp.start.floor());
        let pixel = step as f64 / unit;
        Self {
            first,
            step,
            pixel: pixel as f32,
            left: (-relative / unit) as f32,
            len: ((relative + vp.width()) / step as f64).ceil().max(0.0) as usize,
        }
    }

    pub(super) fn with_padding(mut self, pixels: f32) -> Self {
        let cells = (pixels / self.pixel).ceil() as usize;
        self.first -= cells as i128 * self.step;
        self.left -= cells as f32 * self.pixel;
        self.len += 2 * cells;
        self
    }

    pub(super) fn index_before(
        &self,
        h: &dyn SignalHistory,
        boundary: usize,
        hint: Option<usize>,
    ) -> Option<usize> {
        let tick = self.first + boundary as i128 * self.step - 1;
        if tick < 0 {
            return None;
        }
        let tick = tick.min(i128::from(u64::MAX)) as u64;
        match hint {
            Some(hint) => h.index_at_hint(tick, hint),
            None => h.index_at(tick),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_trace::data::history::VecHistory;
    use volna_trace::data::{SignalShape, WaveValue};

    #[test]
    fn late_bin_boundaries_are_exact() {
        let origin = 1_u64 << 60;
        let history = VecHistory {
            shape: SignalShape::Bit,
            times: vec![origin + 31, origin + 32, origin + 33],
            values: vec![
                WaveValue::Bits("1".into()),
                WaveValue::Bits("0".into()),
                WaveValue::Bits("1".into()),
            ],
            initial: WaveValue::Unavailable,
        };
        let vp = Viewport {
            start: origin as f64,
            end: origin as f64 + 4096.0,
        };
        let grid = SamplingGrid::new(&vp, 512.0);
        assert_eq!(grid.index_before(&history, 4, None), Some(0));
        assert_eq!(grid.index_before(&history, 5, None), Some(2));
        let panned = SamplingGrid::new(
            &Viewport {
                start: vp.start + 256.0,
                end: vp.end + 256.0,
            },
            512.0,
        );
        assert_eq!(panned.first, grid.first + 256);
        assert_eq!(panned.step, grid.step);
    }

    #[test]
    fn sampling_work_stays_bounded_by_view_width() {
        for (start, span) in [
            (-123.25, 4005.0),
            (0.0, 0.25),
            (0.0, 1e12),
            ((1_u64 << 60) as f64, 4096.0),
        ] {
            let grid = SamplingGrid::new(
                &Viewport {
                    start,
                    end: start + span,
                },
                500.25,
            );
            assert!(grid.len <= 2 * 501 + 1, "{start}, {span}");
            assert_eq!(grid.first.rem_euclid(grid.step), 0);
            assert!(grid.left <= 0.0);
            assert!(grid.left + grid.len as f32 * grid.pixel >= 500.25 - 0.001);
        }
    }
}
