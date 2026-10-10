//! Pipeline zoom uses one size for a cycle's width and a row's height.
//! Rectangular cells are projected to the nearest square in logarithmic
//! scale space, then the requested zoom step scales that square. Both axes
//! stop together at a shared size limit.

/// Default square-cell size cap in design pixels. A taller row set by a
/// rows-only zoom raises the panel's cap up to [`super::rows::ROW_PX_MAX`].
pub const ROW_PX_CAP: f32 = 24.0;
/// Maximum cycle width for explicit box zoom, in design pixels.
pub const CYCLE_PX_MAX: f64 = 120.0;

/// Natural logarithms of pixels per time unit and row height in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomPoint {
    pub time: f64,
    pub rows: f64,
}

/// Independent axis limits used by box zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomBox {
    pub time: (f64, f64),
    pub rows: (f64, f64),
}

/// Restore the nearest feasible square, then apply `step` (the logarithm
/// of the zoom factor). `size` bounds the logarithm of the square's side.
/// `cycle_units` is the stream clock's period at the zoom anchor; time
/// units and clock cycles need not coincide.
///
/// Averaging log dimensions minimizes proportional changes to both axes:
/// an 80 × 20 cell projects to 40 × 40. Clamping both the projection and the
/// step keeps an axis at its limit from stretching the other one.
pub fn square_zoom(from: ZoomPoint, cycle_units: f64, size: (f64, f64), step: f64) -> ZoomPoint {
    if !step.is_finite() {
        return from;
    }
    let period = cycle_units.ln();
    let nearest = ((from.time + period + from.rows) * 0.5).clamp(size.0, size.1);
    let side = (nearest + step).clamp(size.0, size.1);
    ZoomPoint {
        time: side - period,
        rows: side,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(cycle_px: f64, row_px: f64, period: f64) -> ZoomPoint {
        ZoomPoint {
            time: (cycle_px / period).ln(),
            rows: row_px.ln(),
        }
    }

    fn close(a: ZoomPoint, b: ZoomPoint) -> bool {
        (a.time - b.time).abs() < 1e-9 && (a.rows - b.rows).abs() < 1e-9
    }

    #[test]
    fn rectangles_project_to_the_geometric_mean_before_the_step() {
        let size = (1.0f64.ln(), 100.0f64.ln());
        for period in [1.0, 10.0] {
            for (width, height) in [(80.0, 20.0), (20.0, 80.0)] {
                assert!(close(
                    square_zoom(at(width, height, period), period, size, 0.0),
                    at(40.0, 40.0, period)
                ));
                assert!(close(
                    square_zoom(at(width, height, period), period, size, 2.0f64.ln()),
                    at(80.0, 80.0, period)
                ));
            }
        }
    }

    #[test]
    fn both_axes_stop_at_the_shared_limits() {
        let size = (2.0f64.ln(), 24.0f64.ln());
        assert!(close(
            square_zoom(at(20.0, 20.0, 1.0), 1.0, size, 2.0f64.ln()),
            at(24.0, 24.0, 1.0)
        ));
        assert!(close(
            square_zoom(at(3.0, 3.0, 1.0), 1.0, size, -2.0f64.ln()),
            at(2.0, 2.0, 1.0)
        ));
        // Normalize an out-of-range rectangle before spending the zoom step.
        assert!(close(
            square_zoom(at(120.0, 24.0, 1.0), 1.0, size, -2.0f64.ln()),
            at(12.0, 12.0, 1.0)
        ));
    }

    #[test]
    fn square_steps_are_reversible_and_invalid_steps_do_nothing() {
        let size = (1.0f64.ln(), 100.0f64.ln());
        let start = at(10.0, 10.0, 10.0);
        let out = square_zoom(start, 10.0, size, -2.0f64.ln());
        assert!(close(square_zoom(out, 10.0, size, 2.0f64.ln()), start));
        assert_eq!(square_zoom(start, 10.0, size, f64::NAN), start);
    }
}
