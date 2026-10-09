//! Independent numeric group members on one amplitude scale. Each member
//! uses the analog renderer and its own min/max summary; values are never
//! composed, and an undefined member cannot interrupt another line.

use std::collections::HashMap;

use super::analog::{Analog, AnalogDraw, AnalogRange};
use super::model::DisplayedSignal;
use super::viewport::Viewport;
use crate::Document;
use crate::data::NumericKind;
use volna_trace::data::SignalShape;

/// Unsaved range caches and easing for one group. History identity and
/// numeric interpretation invalidate cached whole-trace extents.
#[derive(Clone, Debug)]
pub(crate) struct Ranges {
    members: HashMap<(usize, NumericKind), Analog>,
    pub scale: Analog,
}

impl Default for Ranges {
    fn default() -> Self {
        Self {
            members: HashMap::new(),
            scale: Analog::new(AnalogDraw::Step, AnalogRange::Trace),
        }
    }
}

impl Ranges {
    pub fn update(
        &mut self,
        signals: &[&DisplayedSignal],
        range: AnalogRange,
        doc: &Document,
        vp: &Viewport,
    ) {
        let mut wanted = std::collections::HashSet::new();
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for signal in signals {
            let (Some(history), Some(kind)) = (&signal.history, signal.numeric_kind()) else {
                continue;
            };
            let key = (super::analog::history_identity(history), kind);
            wanted.insert(key);
            let a = self
                .members
                .entry(key)
                .or_insert_with(|| Analog::new(AnalogDraw::Step, range));
            a.range = range;
            let series = super::analog::Series::of(doc, signal.source.signal(), history, kind);
            if let Some(bounds) =
                a.raw_target(&series, signal.translator.as_ref(), signal.shape, vp)
            {
                a.target = bounds;
            }
            let bounds = if range == AnalogRange::Type {
                kind.limits(signal.shape).or(a.target)
            } else {
                a.target
            };
            if let Some((low, high)) = bounds {
                lo = lo.min(low);
                hi = hi.max(high);
            }
        }
        self.members.retain(|key, _| wanted.contains(key));
        let bounds = if lo <= hi { (lo, hi) } else { (0.0, 1.0) };
        let target = if range == AnalogRange::Type {
            super::analog::padded(bounds, SignalShape::Real)
        } else {
            super::stack::clean_range(bounds)
        };
        self.scale.target = Some(target);
        if self.scale.shown.is_none() {
            self.scale.shown = Some(target);
        }
    }
}

/// Convert independent polylines to solid segments clipped to the plot bounds.
pub fn strokes(
    runs: &[Vec<crate::geometry::Point>],
    bounds: crate::geometry::Rect,
) -> Vec<[crate::geometry::Point; 2]> {
    runs.iter()
        .flat_map(|run| run.windows(2))
        .filter_map(|pair| clip_segment(pair[0], pair[1], bounds).map(|(a, b)| [a, b]))
        .collect()
}

/// Liang–Barsky clipping in f64 also handles large finite sample coordinates.
fn clip_segment(
    a: crate::geometry::Point,
    b: crate::geometry::Point,
    r: crate::geometry::Rect,
) -> Option<(crate::geometry::Point, crate::geometry::Point)> {
    use crate::geometry::point;
    if ![a.x, a.y, b.x, b.y].iter().all(|v| v.is_finite()) {
        return None;
    }
    let (x, y) = (f64::from(a.x), f64::from(a.y));
    let (dx, dy) = (f64::from(b.x) - x, f64::from(b.y) - y);
    let (mut from, mut to) = (0.0f64, 1.0f64);
    let (mut first_edge, mut last_edge) = (None, None);
    for (p, q, axis, edge) in [
        (-dx, x - f64::from(r.left()), 0, r.left()),
        (dx, f64::from(r.right()) - x, 0, r.right()),
        (-dy, y - f64::from(r.top()), 1, r.top()),
        (dy, f64::from(r.bottom()) - y, 1, r.bottom()),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 && q / p > from {
            from = q / p;
            first_edge = Some((axis, edge));
        } else if p > 0.0 && q / p < to {
            to = q / p;
            last_edge = Some((axis, edge));
        }
    }
    if from > to {
        return None;
    }
    let at = |f: f64, edge: Option<(usize, f32)>| {
        // Anchor interpolation at the nearer endpoint. Retain the exact
        // clipping edge when the ratio rounds to 0 or 1 for a distant point.
        let mut coordinates = if f <= 0.5 {
            [(x + f * dx) as f32, (y + f * dy) as f32]
        } else {
            [
                (f64::from(b.x) - (1.0 - f) * dx) as f32,
                (f64::from(b.y) - (1.0 - f) * dy) as f32,
            ]
        };
        if let Some((axis, value)) = edge {
            coordinates[axis] = value;
        }
        point(
            coordinates[0].clamp(r.left(), r.right()),
            coordinates[1].clamp(r.top(), r.bottom()),
        )
    };
    Some((at(from, first_edge), at(to, last_edge)))
}
