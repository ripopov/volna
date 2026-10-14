//! Analog drawing of numeric signal rows: the row setting, its vertical range, the block
//! min/max summary of long histories, and the geometry the painter strokes.

use std::sync::Arc;

use crate::data::{NumericKind, Translator};
use crate::geometry::{Point, point};
use crate::wave::model::RowHeight;
use crate::wave::viewport::Viewport;
use volna_trace::data::value_view::ValueView;
use volna_trace::data::{Bit, SignalHistory, SignalShape};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalogDraw {
    Step,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalogRange {
    Trace,
    Window,
    Type,
}

impl AnalogDraw {
    #[must_use]
    pub fn default_for(shape: SignalShape) -> Self {
        if shape == SignalShape::Real {
            Self::Linear
        } else {
            Self::Step
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Linear => "linear",
        }
    }
}

impl AnalogRange {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Trace => "whole trace",
            Self::Window => "visible window",
            Self::Type => "type limits",
        }
    }
}

pub const EXACT_RATIO: f64 = 0.25;
pub const DOT_SPACING: f32 = 9.0;
pub const SUMMARY_BLOCK: usize = 64;
pub const SUMMARY_MIN_CHANGES: usize = 1 << 14;
const RANGE_TAU_S: f64 = 0.065;

#[derive(Clone, Debug)]
pub struct Analog {
    pub draw: AnalogDraw,
    pub range: AnalogRange,
    pub restore_height: Option<RowHeight>,
    pub shown: Option<(f64, f64)>,
    pub target: Option<(f64, f64)>,
    trace: Option<TraceRange>,
}

#[derive(Clone, Debug)]
struct TraceRange {
    history: usize,
    kind: NumericKind,
    range: Option<(f64, f64)>,
}

impl PartialEq for Analog {
    fn eq(&self, other: &Self) -> bool {
        self.draw == other.draw && self.range == other.range
    }
}

impl Analog {
    #[must_use]
    pub fn new(draw: AnalogDraw, range: AnalogRange) -> Self {
        Self {
            draw,
            range,
            restore_height: None,
            shown: None,
            target: None,
            trace: None,
        }
    }

    pub fn update_target(
        &mut self,
        series: &Series<'_>,
        tr: &dyn Translator,
        shape: SignalShape,
        vp: &Viewport,
    ) {
        let Some(range) = self.raw_target(series, tr, shape, vp) else {
            return;
        };
        let target = padded(range.unwrap_or((0.0, 1.0)), shape);
        self.target = Some(target);
        if self.shown.is_none() {
            self.shown = Some(target);
        }
    }

    #[expect(
        clippy::option_option,
        reason = "Outer None means data is pending; inner None means no finite samples."
    )]
    pub(crate) fn raw_target(
        &mut self,
        series: &Series<'_>,
        tr: &dyn Translator,
        shape: SignalShape,
        vp: &Viewport,
    ) -> Option<Option<(f64, f64)>> {
        let range = match self.range {
            AnalogRange::Type => tr.limits(shape),
            AnalogRange::Window if series.waiting(visible_changes(series.history, vp)) => {
                return None;
            }
            AnalogRange::Window => series.window_range(vp),
            AnalogRange::Trace => None,
        };
        let range = match range {
            Some(range) => Some(range),
            None => match self.trace_range(series) {
                Some(range) => range,
                None if !series.waiting(visible_changes(series.history, vp)) => {
                    series.window_range(vp)
                }
                None => return None,
            },
        };
        Some(range)
    }

    #[expect(
        clippy::option_option,
        reason = "Outer None means data is pending; inner None means no finite samples."
    )]
    fn trace_range(&mut self, series: &Series<'_>) -> Option<Option<(f64, f64)>> {
        if let Some(summary) = series.summary {
            return Some(summary.root().range());
        }
        if series.waiting(series.history.len()) {
            return None;
        }
        let history = series.identity;
        match &self.trace {
            Some(c) if c.history == history && c.kind == series.kind => Some(c.range),
            _ => {
                let range = series.range(None, series.history.len().checked_sub(1));
                self.trace = Some(TraceRange {
                    history,
                    kind: series.kind,
                    range,
                });
                Some(range)
            }
        }
    }

    pub fn ease(&mut self, dt: f64) -> bool {
        let (Some(shown), Some(target)) = (self.shown, self.target) else {
            return false;
        };
        let span = (target.1 / 2.0 - target.0 / 2.0)
            .abs()
            .max(f64::MIN_POSITIVE);
        let k = 1.0 - (-dt / RANGE_TAU_S).exp();
        let step = |from: f64, to: f64| {
            if (to / 2.0 - from / 2.0).abs() <= span * 2e-3 {
                to
            } else {
                interpolate(from, to, k)
            }
        };
        let next = (step(shown.0, target.0), step(shown.1, target.1));
        self.shown = Some(next);
        next != target
    }

    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.shown == self.target
    }
}

pub fn numeric_kind(shape: SignalShape, tr: &dyn Translator) -> Option<NumericKind> {
    match shape {
        SignalShape::Bit => Some(NumericKind::Unsigned),
        SignalShape::Vector { .. } | SignalShape::Real => tr.numeric_kind(),
        SignalShape::Text | SignalShape::Event => None,
    }
}

pub fn supports(shape: SignalShape, tr: &dyn Translator) -> bool {
    matches!(shape, SignalShape::Vector { .. } | SignalShape::Real) && tr.numeric_kind().is_some()
}

pub fn history_identity(h: &Arc<dyn SignalHistory>) -> usize {
    Arc::as_ptr(h).cast::<()>() as usize
}

pub(crate) fn padded((lo, hi): (f64, f64), shape: SignalShape) -> (f64, f64) {
    if hi - lo > 1e-12 * lo.abs().max(hi.abs()).max(1.0) {
        return (lo, hi);
    }
    let d = if shape == SignalShape::Real {
        (lo.abs() * 0.05).max(0.05)
    } else {
        (lo.abs() * (16.0 * f64::EPSILON)).max(0.5)
    };
    ((lo - d).max(-f64::MAX), (hi + d).min(f64::MAX))
}

fn interpolate(from: f64, to: f64, fraction: f64) -> f64 {
    if from.signum() == to.signum() {
        from + (to - from) * fraction
    } else {
        from * (1.0 - fraction) + to * fraction
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sample {
    Missing,
    Undefined,
    Value(f64),
}

pub fn sample(h: &dyn SignalHistory, kind: NumericKind, i: Option<usize>) -> Sample {
    if h.shape() == SignalShape::Bit && kind == NumericKind::Unsigned {
        return match h.bit(i) {
            Bit::Zero => Sample::Value(0.0),
            Bit::One => Sample::Value(1.0),
            Bit::Unavailable => Sample::Missing,
            Bit::Other => match h.value_view(i) {
                ValueView::Logic(l) if l.width == 1 => match l.bit(0) {
                    b'h' | b'H' => Sample::Value(1.0),
                    b'l' | b'L' => Sample::Value(0.0),
                    _ => Sample::Undefined,
                },
                _ => Sample::Undefined,
            },
            Bit::X | Bit::Z | Bit::DontCare => Sample::Undefined,
        };
    }
    let view = h.value_view(i);
    if matches!(view, ValueView::Unavailable) {
        return Sample::Missing;
    }
    kind.read(&view).map_or(Sample::Undefined, Sample::Value)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub lo: f64,
    pub hi: f64,
    pub undefined: bool,
}

impl Extent {
    pub const EMPTY: Self = Self {
        lo: f64::INFINITY,
        hi: f64::NEG_INFINITY,
        undefined: false,
    };

    fn add(&mut self, s: Sample) {
        match s {
            Sample::Value(v) => {
                self.lo = self.lo.min(v);
                self.hi = self.hi.max(v);
            }
            Sample::Undefined => self.undefined = true,
            Sample::Missing => {}
        }
    }

    fn merge(&mut self, other: &Self) {
        self.lo = self.lo.min(other.lo);
        self.hi = self.hi.max(other.hi);
        self.undefined |= other.undefined;
    }

    #[must_use]
    pub fn range(&self) -> Option<(f64, f64)> {
        (self.lo <= self.hi).then_some((self.lo, self.hi))
    }
}

pub struct AnalogSummary {
    kind: NumericKind,
    history: usize,
    len: usize,
    levels: Vec<Vec<Extent>>,
    _reservation: volna_trace::remote::memory::Reservation,
}

impl std::fmt::Debug for AnalogSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalogSummary")
            .field("kind", &self.kind)
            .field("len", &self.len)
            .field("bytes", &self.resident_bytes())
            .finish_non_exhaustive()
    }
}

impl AnalogSummary {
    pub fn build(
        h: &Arc<dyn SignalHistory>,
        kind: NumericKind,
        budget: &volna_trace::remote::memory::MemoryBudget,
    ) -> anyhow::Result<Self> {
        let len = h.len();
        let (bytes, level_count) = crate::data::admission::pyramid(
            len.div_ceil(SUMMARY_BLOCK),
            2,
            1,
            std::mem::size_of::<Vec<Extent>>(),
            crate::data::admission::bytes::<Extent>,
        )?;
        let reservation = budget.reserve_object("the analog summary", bytes)?;
        let mut leaves = vec![Extent::EMPTY; len.div_ceil(SUMMARY_BLOCK)];
        for (i, leaf) in (0..len).zip((0..len).map(|i| i / SUMMARY_BLOCK)) {
            leaves[leaf].add(sample(h.as_ref(), kind, Some(i)));
        }
        let mut levels = Vec::with_capacity(level_count);
        levels.push(leaves);
        while levels.last().is_some_and(|l| l.len() > 1) {
            let below = levels.last().unwrap();
            let above = below
                .chunks(2)
                .map(|pair| {
                    let mut e = pair[0];
                    if let Some(b) = pair.get(1) {
                        e.merge(b);
                    }
                    e
                })
                .collect();
            levels.push(above);
        }
        Ok(Self {
            kind,
            history: history_identity(h),
            len,
            levels,
            _reservation: reservation,
        })
    }

    pub fn resident_bytes(&self) -> u64 {
        let nodes: usize = self.levels.iter().map(Vec::capacity).sum();
        (nodes * std::mem::size_of::<Extent>()
            + self.levels.capacity() * std::mem::size_of::<Vec<Extent>>()) as u64
    }

    #[must_use]
    pub fn kind(&self) -> NumericKind {
        self.kind
    }

    #[must_use]
    pub fn identity(&self) -> usize {
        self.history
    }

    pub fn matches(&self, h: &Arc<dyn SignalHistory>, kind: NumericKind) -> bool {
        self.kind == kind && self.history == history_identity(h) && self.len == h.len()
    }

    fn root(&self) -> Extent {
        self.levels
            .last()
            .and_then(|l| l.first())
            .copied()
            .unwrap_or(Extent::EMPTY)
    }

    pub fn query(&self, h: &dyn SignalHistory, a: usize, b: usize) -> Extent {
        let mut e = Extent::EMPTY;
        let b = b.min(self.len.saturating_sub(1));
        if a > b {
            return e;
        }
        let (first, end) = (a.div_ceil(SUMMARY_BLOCK), (b + 1) / SUMMARY_BLOCK);
        if first >= end {
            for i in a..=b {
                e.add(sample(h, self.kind, Some(i)));
            }
            return e;
        }
        for i in (a..first * SUMMARY_BLOCK).chain(end * SUMMARY_BLOCK..=b) {
            e.add(sample(h, self.kind, Some(i)));
        }
        let (mut l, mut r) = (first, end);
        for level in &self.levels {
            if l >= r {
                break;
            }
            if l & 1 == 1 {
                e.merge(&level[l]);
                l += 1;
            }
            if r & 1 == 1 {
                r -= 1;
                e.merge(&level[r]);
            }
            l >>= 1;
            r >>= 1;
        }
        e
    }
}

#[derive(Clone, Copy)]
pub struct Series<'a> {
    pub history: &'a dyn SignalHistory,
    pub kind: NumericKind,
    pub summary: Option<&'a AnalogSummary>,
    pub building: bool,
    identity: usize,
}

impl<'a> Series<'a> {
    pub fn new(
        history: &'a Arc<dyn SignalHistory>,
        kind: NumericKind,
        summary: Option<&'a AnalogSummary>,
    ) -> Self {
        Self {
            history: history.as_ref(),
            kind,
            summary: summary.filter(|s| s.matches(history, kind)),
            building: false,
            identity: history_identity(history),
        }
    }

    pub fn of(
        doc: &'a crate::document::Document,
        signal: Option<crate::trace::Traced<volna_trace::data::SignalRef>>,
        history: &'a Arc<dyn SignalHistory>,
        kind: NumericKind,
    ) -> Self {
        use crate::document::SummaryLoad;
        let load = signal.and_then(|s| doc.analog_summary(s, kind));
        let summary = match load {
            Some(SummaryLoad::Ready(s)) => Some(s.as_ref()),
            _ => None,
        };
        let mut series = Self::new(history, kind, summary);
        series.building = matches!(load, Some(SummaryLoad::Building { .. }));
        series
    }

    #[must_use]
    pub fn waiting(&self, changes: usize) -> bool {
        self.building && self.summary.is_none() && changes >= SUMMARY_MIN_CHANGES
    }

    #[must_use]
    pub fn sample(&self, i: Option<usize>) -> Sample {
        sample(self.history, self.kind, i)
    }

    #[must_use]
    pub fn extent(&self, from: Option<usize>, to: Option<usize>) -> Extent {
        let mut e = Extent::EMPTY;
        let Some(to) = to else {
            e.add(self.sample(None));
            return e;
        };
        if from.is_none() {
            e.add(self.sample(None));
        }
        let a = from.unwrap_or(0);
        match self.summary {
            Some(s) if to - a >= 2 * SUMMARY_BLOCK => e.merge(&s.query(self.history, a, to)),
            _ => {
                for i in a..=to.min(self.history.len().saturating_sub(1)) {
                    e.add(self.sample(Some(i)));
                }
            }
        }
        e
    }

    #[must_use]
    pub fn range(&self, from: Option<usize>, to: Option<usize>) -> Option<(f64, f64)> {
        self.extent(from, to).range()
    }

    #[must_use]
    pub fn window_range(&self, vp: &Viewport) -> Option<(f64, f64)> {
        self.range(
            index_at(self.history, vp.start),
            index_at(self.history, vp.end),
        )
    }
}

fn index_at(h: &dyn SignalHistory, t: f64) -> Option<usize> {
    if t < 0.0 {
        return None;
    }
    h.index_at(t.floor() as u64)
}

fn changes_between(a: Option<usize>, b: Option<usize>) -> usize {
    match (a, b) {
        (_, None) => 0,
        (None, Some(b)) => b + 1,
        (Some(a), Some(b)) => b.saturating_sub(a),
    }
}

pub fn visible_changes(h: &dyn SignalHistory, vp: &Viewport) -> usize {
    changes_between(index_at(h, vp.start), index_at(h, vp.end))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawMode {
    Exact,
    Envelope,
}

pub fn draw_mode(h: &dyn SignalHistory, vp: &Viewport, width_px: f32) -> DrawMode {
    if visible_changes(h, vp) as f64 <= f64::from(width_px) * EXACT_RATIO {
        DrawMode::Exact
    } else {
        DrawMode::Envelope
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Plot {
    pub left: f32,
    pub width: f32,
    pub top: f32,
    pub bottom: f32,
    pub lo: f64,
    pub hi: f64,
}

impl Plot {
    #[must_use]
    pub fn y_of(&self, v: f64) -> f32 {
        let span = self.hi - self.lo;
        let f = if span.is_finite() {
            (v - self.lo) / span
        } else {
            (v / 2.0 - self.lo / 2.0) / (self.hi / 2.0 - self.lo / 2.0)
        };
        (f64::from(self.bottom) - f * f64::from(self.bottom - self.top))
            .clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32
    }

    fn x_of(&self, vp: &Viewport, t: u64) -> f32 {
        self.left + vp.x_of_tick(t, f64::from(self.width)) as f32
    }

    #[must_use]
    pub fn baseline(&self) -> f32 {
        self.y_of(0.0f64.clamp(self.lo, self.hi))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Geometry {
    pub envelope: bool,
    pub waiting: bool,
    pub runs: Vec<Vec<Point>>,
    pub undefined: Vec<(f32, f32)>,
    pub dots: Vec<Point>,
}

impl Geometry {
    fn undefined(&mut self, a: f32, b: f32) {
        match self.undefined.last_mut() {
            Some(last) if a <= last.1 + 0.5 => last.1 = last.1.max(b),
            _ => self.undefined.push((a, b)),
        }
    }
}

#[must_use]
pub fn geometry(
    series: &Series<'_>,
    draw: AnalogDraw,
    vp: &Viewport,
    plot: &Plot,
    zoom: f32,
) -> Geometry {
    let mut g = Geometry::default();
    if plot.width <= 0.0 || vp.width() <= 0.0 || plot.hi <= plot.lo {
        return g;
    }
    match draw_mode(series.history, vp, plot.width) {
        DrawMode::Exact => exact(series, draw, vp, plot, zoom, &mut g),
        DrawMode::Envelope if series.waiting(visible_changes(series.history, vp)) => {
            g.envelope = true;
            g.waiting = true;
        }
        DrawMode::Envelope => envelope(series, draw, vp, plot, &mut g),
    }
    g
}

fn exact(
    series: &Series<'_>,
    draw: AnalogDraw,
    vp: &Viewport,
    plot: &Plot,
    zoom: f32,
    g: &mut Geometry,
) {
    let h = series.history;
    let len = h.len();
    let first = vp
        .tick_at(0.0, f64::from(plot.width))
        .and_then(|t| h.index_at(t));
    let right = plot.left + plot.width + 1.0;
    let last = match index_at(h, vp.end) {
        Some(i) => (i + 1).min(len - 1),
        None if len > 0 => 0,
        None => {
            let (a, b) = (plot.left - 1.0, right);
            match series.sample(None) {
                Sample::Value(v) => g
                    .runs
                    .push(vec![point(a, plot.y_of(v)), point(b, plot.y_of(v))]),
                Sample::Undefined => g.undefined(a, b),
                Sample::Missing => {}
            }
            return;
        }
    };
    let spacing = plot.width / (visible_changes(h, vp).max(1) as f32);
    let mark = draw == AnalogDraw::Linear && spacing >= DOT_SPACING * zoom;
    let indices = std::iter::once(first)
        .filter(std::option::Option::is_none)
        .chain((first.unwrap_or(0)..=last).map(Some));
    let mut run: Option<Vec<Point>> = None;
    for i in indices {
        let raw_a = i.map(|i| plot.x_of(vp, h.time(i)));
        let xa = raw_a.map_or(plot.left - 1.0, |x| x.max(plot.left - 1.0));
        let next = i.map_or(0, |i| i + 1);
        let raw_b = (next < len).then(|| plot.x_of(vp, h.time(next)));
        let xb = raw_b.map_or(right, |x| x.min(right));
        match series.sample(i) {
            Sample::Missing => g.runs.extend(run.take()),
            Sample::Undefined => {
                g.runs.extend(run.take());
                g.undefined(xa, xb);
            }
            Sample::Value(v) => {
                let y = plot.y_of(v);
                let points = run.get_or_insert_with(Vec::new);
                match draw {
                    AnalogDraw::Step => points.extend([point(xa, y), point(xb, y)]),
                    AnalogDraw::Linear => {
                        let to = match (next < len).then(|| series.sample(Some(next))) {
                            Some(Sample::Value(to)) => Some(to),
                            _ => None,
                        };
                        let at = |x: f32| match (raw_a, raw_b, to) {
                            (Some(a), Some(b), Some(to)) if b > a => {
                                let f = f64::from((x - a) / (b - a));
                                plot.y_of(interpolate(v, to, f.clamp(0.0, 1.0)))
                            }
                            _ => y,
                        };
                        let clipped = raw_a.is_some_and(|a| a < xa);
                        points.push(point(xa, if clipped { at(xa) } else { y }));
                        if mark && i.is_some() && !clipped {
                            g.dots.push(point(xa, y));
                        }
                        if to.is_none() {
                            points.push(point(xb, y));
                        } else if raw_b.is_some_and(|b| b > right) {
                            points.push(point(right, at(right)));
                        }
                    }
                }
            }
        }
        if xb >= right {
            break;
        }
    }
    g.runs.extend(run);
}

fn envelope(series: &Series<'_>, draw: AnalogDraw, vp: &Viewport, plot: &Plot, g: &mut Geometry) {
    let n = plot.width.ceil().max(1.0) as usize;
    let w = f64::from(plot.width);
    let mut run: Option<Vec<Point>> = None;
    let mut hint = None;
    for x in 0..n {
        let xa = x as f64;
        let xb = ((x + 1) as f64).min(w);
        let Column {
            extent: e,
            changes,
            entry,
            exit,
            last,
        } = column(series, draw, vp, w, xa, xb, hint);
        hint = last;
        let xc = plot.left + xa.midpoint(xb) as f32;
        if e.undefined {
            g.runs.extend(run.take());
            g.undefined(plot.left + xa as f32, plot.left + xb as f32);
        }
        match e.range() {
            None => g.runs.extend(run.take()),
            Some((lo, hi)) => {
                if !matches!(entry, Sample::Value(_)) {
                    g.runs.extend(run.take());
                }
                let points = run.get_or_insert_with(Vec::new);
                if let Sample::Value(v) = entry {
                    points.push(point(xc, plot.y_of(v)));
                }
                if changes > 0 {
                    points.extend([point(xc, plot.y_of(lo)), point(xc, plot.y_of(hi))]);
                }
                match exit {
                    Sample::Value(v) => points.push(point(xc, plot.y_of(v))),
                    _ => g.runs.extend(run.take()),
                }
            }
        }
        if e.undefined {
            g.runs.extend(run.take());
        }
    }
    g.envelope = true;
    g.runs.extend(run);
}

struct Column {
    extent: Extent,
    changes: usize,
    entry: Sample,
    exit: Sample,
    last: Option<usize>,
}

fn column(
    series: &Series<'_>,
    draw: AnalogDraw,
    vp: &Viewport,
    width: f64,
    from: f64,
    to: f64,
    hint: Option<usize>,
) -> Column {
    let h = series.history;
    let lookup = |time, hint| match hint {
        Some(hint) => h.index_at_hint(time, hint),
        None => h.index_at(time),
    };
    let first = vp.tick_at(from, width).and_then(|t| lookup(t, hint));
    let last = vp
        .tick_before(to, width)
        .and_then(|t| lookup(t, first))
        .max(first);
    let changes = changes_between(first, last);
    let mut entry = series.sample(first);
    let mut exit = series.sample(last);
    let mut extent = Extent::EMPTY;
    if draw == AnalogDraw::Linear {
        for (sample, index, x) in [(&mut entry, first, from), (&mut exit, last, to)] {
            if matches!(sample, Sample::Value(_))
                && let Some(v) = value_at_x(series, draw, vp, index, x, width)
            {
                *sample = Sample::Value(v);
            }
            extent.add(*sample);
        }
    } else {
        extent.add(entry);
    }
    if changes > 0 {
        extent.merge(&series.extent(Some(first.map_or(0, |i| i + 1)), last));
    }
    Column {
        extent,
        changes,
        entry,
        exit,
        last,
    }
}

#[must_use]
pub fn y_at(series: &Series<'_>, draw: AnalogDraw, plot: &Plot, t: u64) -> Option<f32> {
    let h = series.history;
    let i = h.index_at(t);
    let next = i.map_or(0, |i| i + 1);
    let fraction = if let Some(i) = i.filter(|&i| next < h.len() && h.time(next) > h.time(i)) {
        (t - h.time(i)) as f64 / (h.time(next) - h.time(i)) as f64
    } else {
        0.0
    };
    interpolated_value(series, draw, i, fraction).map(|v| plot.y_of(v))
}

fn value_at_x(
    series: &Series<'_>,
    draw: AnalogDraw,
    vp: &Viewport,
    i: Option<usize>,
    x: f64,
    width: f64,
) -> Option<f64> {
    let h = series.history;
    let next = i.map_or(0, |i| i + 1);
    let fraction = if let Some(i) = i.filter(|_| next < h.len()) {
        let (a, b) = (
            vp.x_of_tick(h.time(i), width),
            vp.x_of_tick(h.time(next), width),
        );
        if b > a { (x - a) / (b - a) } else { 0.0 }
    } else {
        0.0
    };
    interpolated_value(series, draw, i, fraction)
}

fn interpolated_value(
    series: &Series<'_>,
    draw: AnalogDraw,
    i: Option<usize>,
    fraction: f64,
) -> Option<f64> {
    let h = series.history;
    let Sample::Value(mut v) = series.sample(i) else {
        return None;
    };
    let next = i.map_or(0, |i| i + 1);
    if draw == AnalogDraw::Linear
        && i.is_some()
        && next < h.len()
        && let Sample::Value(v1) = series.sample(Some(next))
    {
        v = interpolate(v, v1, fraction.clamp(0.0, 1.0));
    }
    Some(v)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Readout {
    Sample {
        index: Option<usize>,
        time: f64,
        at: Point,
    },
    Column {
        lo: f64,
        hi: f64,
        changes: usize,
        x: f32,
    },
}

#[must_use]
pub fn readout(
    series: &Series<'_>,
    draw: AnalogDraw,
    vp: &Viewport,
    plot: &Plot,
    x: f32,
) -> Option<Readout> {
    let h = series.history;
    let w = f64::from(plot.width);
    let dx = f64::from(x - plot.left);
    if dx < 0.0 || dx > w {
        return None;
    }
    if draw_mode(h, vp, plot.width) == DrawMode::Envelope {
        if series.waiting(visible_changes(h, vp)) {
            return None;
        }
        let col = dx.floor().min(w.ceil() - 1.0);
        let end = (col + 1.0).min(w);
        let column = column(series, draw, vp, w, col, end, None);
        let (lo, hi) = column.extent.range()?;
        return Some(Readout::Column {
            lo,
            hi,
            changes: column.changes,
            x: plot.left + col.midpoint(end) as f32,
        });
    }
    let t = vp.time_at(dx, w);
    let mut i = vp.tick_at(dx, w).and_then(|t| h.index_at(t));
    if draw == AnalogDraw::Linear {
        let next = i.map_or(0, |i| i + 1);
        let here = i.map(|i| (plot.x_of(vp, h.time(i)) - x).abs());
        if next < h.len() {
            let there = (plot.x_of(vp, h.time(next)) - x).abs();
            if here.is_none_or(|here| there < here) {
                i = Some(next);
            }
        }
    }
    let Sample::Value(v) = series.sample(i) else {
        return None;
    };
    let (time, px) = match (draw, i) {
        (AnalogDraw::Linear, Some(i)) => (h.time(i) as f64, plot.x_of(vp, h.time(i))),
        _ => (t, x),
    };
    Some(Readout::Sample {
        index: i,
        time,
        at: point(px, plot.y_of(v)),
    })
}

#[must_use]
pub fn fill_columns(runs: &[Vec<Point>], left: f32, width: f32) -> Vec<(f32, f32, f32)> {
    let n = width.max(0.0).ceil() as usize;
    let mut ys: Vec<Option<f32>> = vec![None; n];
    let col = |x: f32| {
        let c = (x - left).floor();
        (c >= 0.0 && (c as usize) < n).then_some(c as usize)
    };
    for run in runs {
        if let [only] = run.as_slice()
            && let Some(c) = col(only.x)
        {
            ys[c] = Some(only.y);
        }
        for pair in run.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if b.x <= a.x {
                if let Some(c) = col(b.x) {
                    ys[c] = Some(b.y);
                }
                continue;
            }
            let c0 = (a.x - left).floor().max(0.0) as usize;
            let c1 = (b.x - left).ceil().clamp(0.0, n as f32) as usize;
            for (c, y) in ys.iter_mut().enumerate().take(c1).skip(c0) {
                let xc = left + c as f32 + (width - c as f32).min(1.0) / 2.0;
                if xc >= a.x && xc <= b.x {
                    let f = f64::from(xc - a.x) / f64::from(b.x - a.x);
                    *y = Some((f64::from(a.y) + (f64::from(b.y) - f64::from(a.y)) * f) as f32);
                }
            }
        }
    }
    let mut out: Vec<(f32, f32, f32)> = Vec::new();
    for (c, y) in ys.into_iter().enumerate() {
        let Some(y) = y else { continue };
        let x = left + c as f32;
        let width = (width - c as f32).min(1.0);
        match out.last_mut() {
            Some((x0, w, y0)) if (*x0 + *w - x).abs() < 1e-3 && (*y0 - y).abs() < 0.5 => {
                *w += width;
            }
            _ => out.push((x, width, y)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_trace::data::WaveValue;
    use volna_trace::data::history::VecHistory;

    fn history(values: &[Option<i64>]) -> Arc<dyn SignalHistory> {
        Arc::new(VecHistory {
            shape: SignalShape::Vector { width: 16 },
            times: (0..values.len() as u64).collect(),
            values: values
                .iter()
                .map(|v| match v {
                    Some(v) => WaveValue::Bits(format!("{:016b}", *v as u16)),
                    None => WaveValue::Bits("x".repeat(16)),
                })
                .collect(),
            initial: WaveValue::Unavailable,
        })
    }

    #[test]
    fn summary_queries_match_direct_scans_at_every_boundary() {
        let values: Vec<Option<i64>> = (0..1000i64)
            .map(|i| (i % 97 != 13).then_some((i * 7919) % 2001 - 1000))
            .collect();
        let h = history(&values);
        let s = AnalogSummary::build(
            &h,
            NumericKind::Signed,
            &volna_trace::remote::memory::MemoryBudget::new(u64::MAX),
        )
        .unwrap();
        let direct = |a: usize, b: usize| {
            let mut e = Extent::EMPTY;
            for i in a..=b {
                e.add(sample(h.as_ref(), NumericKind::Signed, Some(i)));
            }
            e
        };
        for a in [0, 1, 63, 64, 65, 127, 128, 500, 999] {
            for b in [a, a + 1, 63, 64, 128, 200, 511, 512, 998, 999] {
                if b >= a && b < 1000 {
                    assert_eq!(s.query(h.as_ref(), a, b), direct(a, b), "{a}..={b}");
                }
            }
        }
        assert_eq!(s.root(), direct(0, 999));
        assert!(s.root().undefined);
        assert_eq!(
            s.resident_bytes(),
            (31 * std::mem::size_of::<Extent>() + 5 * std::mem::size_of::<Vec<Extent>>()) as u64
        );
    }

    #[test]
    fn linear_plots_cross_the_edges_on_the_line() {
        let h: Arc<dyn SignalHistory> = Arc::new(VecHistory {
            shape: SignalShape::Vector { width: 16 },
            times: (0..=10).map(|i| i * 100).collect(),
            values: (0..=10)
                .map(|i| WaveValue::Bits(format!("{:016b}", i * 10)))
                .collect(),
            initial: WaveValue::Unavailable,
        });
        let series = Series::new(&h, NumericKind::Unsigned, None);
        let plot = Plot {
            left: 0.0,
            width: 500.0,
            top: 0.0,
            bottom: 100.0,
            lo: 0.0,
            hi: 100.0,
        };
        let on_line = |x: f32, vp: &Viewport| plot.y_of((vp.time_at(f64::from(x), 500.0)) / 10.0);
        for vp in [
            Viewport {
                start: 250.0,
                end: 750.0,
            },
            Viewport {
                start: 420.0,
                end: 580.0,
            },
        ] {
            let g = geometry(&series, AnalogDraw::Linear, &vp, &plot, 1.0);
            let run = &g.runs[0];
            let (first, last) = (run[0], run[run.len() - 1]);
            assert!((first.y - on_line(first.x, &vp)).abs() < 0.01, "{run:?}");
            assert_eq!(last.x, plot.width + 1.0, "{run:?}");
            assert!((last.y - on_line(last.x, &vp)).abs() < 0.01, "{run:?}");
        }
    }

    fn real_history(times: Vec<u64>, values: Vec<f64>) -> Arc<dyn SignalHistory> {
        Arc::new(VecHistory {
            shape: SignalShape::Real,
            times,
            values: values.into_iter().map(WaveValue::Real).collect(),
            initial: WaveValue::Unavailable,
        })
    }

    fn test_plot(width: f32, lo: f64, hi: f64) -> Plot {
        Plot {
            left: 0.0,
            width,
            top: 0.0,
            bottom: 100.0,
            lo,
            hi,
        }
    }

    #[test]
    fn envelopes_break_at_unknown_boundaries_without_carrying_the_previous_sample() {
        let h = real_history(
            (0..40).collect(),
            (0..40)
                .map(|t| {
                    if (10..20).contains(&t) {
                        f64::NAN
                    } else {
                        f64::from(t)
                    }
                })
                .collect(),
        );
        let series = Series::new(&h, NumericKind::Real, None);
        let vp = Viewport {
            start: 0.0,
            end: 40.0,
        };
        let plot = test_plot(40.0, 0.0, 40.0);
        for draw in [AnalogDraw::Step, AnalogDraw::Linear] {
            let g = geometry(&series, draw, &vp, &plot, 1.0);
            assert!(g.envelope);
            assert_eq!(g.undefined, [(10.0, 20.0)]);
            assert!(
                g.runs
                    .iter()
                    .flatten()
                    .all(|p| !(10.0..20.0).contains(&p.x))
            );
            assert_eq!(g.runs.len(), 2);
        }
    }

    #[test]
    fn linear_envelopes_and_readouts_include_interpolated_column_edges() {
        let h = real_history(vec![0, 100], vec![0.0, 100.0]);
        let series = Series::new(&h, NumericKind::Real, None);
        let vp = Viewport {
            start: 40.0,
            end: 100.0,
        };
        let plot = test_plot(2.5, 0.0, 100.0);
        let g = geometry(&series, AnalogDraw::Linear, &vp, &plot, 1.0);
        assert!(g.envelope);
        assert!(g.runs[0].iter().all(|p| p.y <= plot.y_of(40.0)));
        assert!(
            g.runs[0]
                .iter()
                .any(|p| p.x == 0.5 && p.y == plot.y_of(64.0))
        );
        assert!(
            g.runs[0]
                .iter()
                .any(|p| p.x == 2.25 && p.y == plot.y_of(100.0))
        );
        assert_eq!(
            readout(&series, AnalogDraw::Linear, &vp, &plot, 0.5),
            Some(Readout::Column {
                lo: 40.0,
                hi: 64.0,
                changes: 0,
                x: 0.5
            })
        );
        let Some(Readout::Column { lo, hi, changes, x }) =
            readout(&series, AnalogDraw::Linear, &vp, &plot, 2.5)
        else {
            panic!("expected a column readout")
        };
        assert!((lo - 88.0).abs() < 1e-12);
        assert_eq!((hi, changes, x), (100.0, 0, 2.25));
    }

    #[test]
    fn finite_extreme_values_keep_ranges_interpolation_and_animation_finite() {
        let plot = test_plot(100.0, -f64::MAX, f64::MAX);
        assert_eq!(plot.y_of(-f64::MAX), plot.bottom);
        assert_eq!(plot.y_of(0.0), 50.0);
        assert_eq!(plot.y_of(f64::MAX), plot.top);
        assert_eq!(interpolate(-f64::MAX, f64::MAX, 0.5), 0.0);
        for shape in [SignalShape::Real, SignalShape::Vector { width: 64 }] {
            for value in [-f64::MAX, f64::MAX, 1e30] {
                let (lo, hi) = padded((value, value), shape);
                assert!(lo.is_finite() && hi.is_finite() && hi > lo);
            }
        }
        let mut a = Analog::new(AnalogDraw::Linear, AnalogRange::Trace);
        a.shown = Some((-f64::MAX, -f64::MAX / 2.0));
        a.target = Some((f64::MAX / 2.0, f64::MAX));
        for _ in 0..100 {
            a.ease(0.016);
            let (lo, hi) = a.shown.unwrap();
            assert!(lo.is_finite() && hi.is_finite() && hi > lo);
        }
        assert!(a.is_settled());
    }

    #[test]
    fn negative_traces_fill_toward_zero_and_fractional_columns_stay_inside_the_plot() {
        let plot = test_plot(2.5, -10.0, -1.0);
        assert_eq!(plot.baseline(), plot.top);
        let run = vec![point(-1.0, 50.0), point(3.5, 50.0)];
        let columns = fill_columns(&[run], plot.left, plot.width);
        assert_eq!(columns, [(0.0, 2.5, 50.0)]);
        let run = vec![point(0.5, 50.0), point(2.25, 50.0)];
        assert_eq!(fill_columns(&[run], 0.0, 2.5), [(0.0, 2.5, 50.0)]);
        let run = vec![point(0.0, -f32::MAX), point(2.0, f32::MAX)];
        let columns = fill_columns(&[run], 0.0, 2.0);
        assert!(columns.iter().all(|(_, _, y)| y.is_finite()));
        assert_eq!(columns[0].2, -columns[1].2);
    }

    #[test]
    fn late_linear_probes_interpolate_between_distinct_integer_ticks() {
        let origin = 1u64 << 60;
        let h = real_history(vec![origin + 1, origin + 3], vec![0.0, 10.0]);
        let series = Series::new(&h, NumericKind::Real, None);
        let plot = test_plot(4096.0, 0.0, 10.0);
        assert_eq!(
            y_at(&series, AnalogDraw::Linear, &plot, origin + 2),
            Some(50.0)
        );
        assert_eq!(
            y_at(&series, AnalogDraw::Step, &plot, origin + 2),
            Some(100.0)
        );
        let vp = Viewport {
            start: origin as f64,
            end: origin as f64 + 4096.0,
        };
        assert_eq!(
            value_at_x(&series, AnalogDraw::Linear, &vp, Some(0), 2.0, 4096.0),
            Some(5.0)
        );
    }
}
