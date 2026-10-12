//! Stacked areas: a group drawn as one area whose layers are its signals.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;

use crate::data::{NumericKind, Translator};
use crate::wave::analog::{self, EXACT_RATIO, Sample};
use crate::wave::viewport::Viewport;
use volna_trace::data::{SignalHistory, SignalShape};

pub const WALK_MAX: usize = super::group::WALK_MAX;
const RECOMPOSE_EVERY: u32 = 256;
pub const INTEGRAL_BLOCK: usize = 64;
pub const INTEGRAL_MIN_CHANGES: usize = analog::SUMMARY_MIN_CHANGES;
const SUMMARY_PER_COLUMN: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reading {
    Bit,
    Number(NumericKind),
}

impl Reading {
    pub fn of(shape: SignalShape, tr: &dyn Translator) -> Option<Self> {
        let kind = analog::numeric_kind(shape, tr)?;
        Some(if shape == SignalShape::Bit {
            Self::Bit
        } else {
            Self::Number(kind)
        })
    }

    pub fn is_integer(self) -> bool {
        matches!(
            self,
            Self::Bit | Self::Number(NumericKind::Unsigned | NumericKind::Signed)
        )
    }

    pub fn sample(self, h: &dyn SignalHistory, i: Option<usize>) -> Sample {
        let Self::Number(kind) = self else {
            return analog::sample(h, NumericKind::Unsigned, i);
        };
        analog::sample(h, kind, i)
    }
}

#[derive(Clone)]
pub struct Layer {
    pub entry: usize,
    pub history: Arc<dyn SignalHistory>,
    pub reading: Reading,
    pub summary: Option<Arc<IntegralSummary>>,
    pub building: bool,
}

impl Layer {
    pub fn new(entry: usize, history: Arc<dyn SignalHistory>, reading: Reading) -> Self {
        Self {
            entry,
            history,
            reading,
            summary: None,
            building: false,
        }
    }

    pub fn with_summary(mut self, summary: Option<Arc<IntegralSummary>>) -> Self {
        self.summary = summary.filter(|s| s.matches(&self.history, self.reading));
        self
    }

    pub fn sample(&self, i: Option<usize>) -> Sample {
        self.reading.sample(self.history.as_ref(), i)
    }

    pub fn at(&self, t: u64) -> Sample {
        self.sample(self.history.index_at(t))
    }
}

pub type Key = Vec<(usize, Reading)>;

pub fn key(layers: &[Layer]) -> Key {
    layers
        .iter()
        .map(|l| (analog::history_identity(&l.history), l.reading))
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Composed {
    pub top: f64,
    pub bottom: f64,
    pub undefined: bool,
}

pub fn compose(values: &[Sample], mut band: impl FnMut(usize, f64, f64)) -> Composed {
    let mut c = Composed::default();
    for (k, v) in values.iter().enumerate().rev() {
        match *v {
            Sample::Value(v) if v >= 0.0 => {
                band(k, c.top, c.top + v);
                c.top += v;
            }
            Sample::Value(v) => {
                band(k, c.bottom + v, c.bottom);
                c.bottom += v;
            }
            Sample::Undefined | Sample::Missing => {
                c.undefined = true;
                break;
            }
        }
    }
    c
}

pub fn total(values: &[Sample]) -> Option<f64> {
    values.iter().try_fold(0.0, |sum, v| match v {
        Sample::Value(v) => Some(sum + v),
        _ => None,
    })
}

pub fn values_at(layers: &[Layer], t: u64) -> Vec<Sample> {
    layers.iter().map(|l| l.at(t)).collect()
}

fn index_at(h: &dyn SignalHistory, t: f64) -> Option<usize> {
    if t < 0.0 {
        return None;
    }
    h.index_at(t.floor() as u64)
}

pub fn visible_changes(layers: &[Layer], vp: &Viewport) -> usize {
    layers
        .iter()
        .map(|l| analog::visible_changes(l.history.as_ref(), vp))
        .sum()
}

#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub exact: bool,
    pub spans: Vec<(f64, f64)>,
    values: Vec<Sample>,
    layers: usize,
    changes: Vec<u32>,
    waiting: bool,
}

impl Frame {
    pub fn values(&self, span: usize) -> &[Sample] {
        &self.values[span * self.layers..(span + 1) * self.layers]
    }

    pub fn changes(&self, span: usize) -> u32 {
        self.changes.get(span).copied().unwrap_or(0)
    }

    pub fn span_at(&self, x: f64) -> Option<usize> {
        let i = self.spans.partition_point(|s| s.1 <= x);
        (i < self.spans.len() && self.spans[i].0 <= x).then_some(i)
    }

    pub fn waiting(&self) -> bool {
        self.waiting
    }

    pub fn extent(&self) -> (f64, f64) {
        (0..self.spans.len()).fold((0.0, 0.0), |(lo, hi), s| {
            let c = compose(self.values(s), |_, _, _| {});
            (lo.min(c.bottom), hi.max(c.top))
        })
    }
}

pub fn frame(layers: &[Layer], vp: &Viewport, width: usize) -> Frame {
    if width == 0 || vp.width() <= 0.0 || layers.is_empty() {
        return Frame {
            layers: layers.len(),
            ..Frame::default()
        };
    }
    let visible = visible_changes(layers, vp);
    if visible as f64 <= width as f64 * EXACT_RATIO {
        return steps(layers, vp, width);
    }
    let waiting = layers.iter().any(|l| {
        l.building
            && l.summary.is_none()
            && analog::visible_changes(l.history.as_ref(), vp) >= INTEGRAL_MIN_CHANGES
    });
    if waiting && visible > WALK_MAX {
        return Frame {
            waiting: true,
            layers: layers.len(),
            ..Frame::default()
        };
    }
    columns(layers, vp, width)
}

pub fn steps(layers: &[Layer], vp: &Viewport, width: usize) -> Frame {
    let w = width as f64;
    let mut times: Vec<u64> = Vec::new();
    for l in layers {
        let h = l.history.as_ref();
        let a = index_at(h, vp.start).map_or(0, |i| i + 1);
        let b = index_at(h, vp.end).map_or(0, |i| i + 1);
        times.extend((a..b).map(|i| h.time(i)));
    }
    times.sort_unstable();
    times.dedup();
    let bounds: Vec<f64> = std::iter::once(vp.start)
        .chain(times.iter().map(|&t| t as f64))
        .chain(std::iter::once(vp.end))
        .collect();
    let mut at: Vec<Option<usize>> = layers
        .iter()
        .map(|l| index_at(l.history.as_ref(), vp.start))
        .collect();
    let mut frame = Frame {
        exact: true,
        layers: layers.len(),
        ..Frame::default()
    };
    for pair in bounds.windows(2) {
        let (ta, tb) = (pair[0], pair[1]);
        if tb <= ta {
            continue;
        }
        for (l, i) in layers.iter().zip(&mut at) {
            let h = l.history.as_ref();
            while let Some(next) =
                Some(i.map_or(0, |i| i + 1)).filter(|&n| n < h.len() && h.time(n) as f64 <= ta)
            {
                *i = Some(next);
            }
            frame.values.push(l.sample(*i));
        }
        frame.spans.push((vp.x_of(ta, w), vp.x_of(tb, w)));
    }
    frame
}

pub fn columns(layers: &[Layer], vp: &Viewport, width: usize) -> Frame {
    let n = layers.len();
    let w = width as f64;
    let mut values = vec![Sample::Missing; width * n];
    let mut changes = vec![0u32; width];
    for (k, l) in layers.iter().enumerate() {
        let h = l.history.as_ref();
        if let Some(summary) = &l.summary
            && analog::visible_changes(h, vp) > SUMMARY_PER_COLUMN * width
        {
            let mut prefixes = Prefixes::new(summary, h);
            let mut ia = index_at(h, vp.time_at(0.0, w));
            let mut density = 0;
            for c in 0..width {
                let (ta, tb) = (vp.time_at(c as f64, w), vp.time_at((c + 1) as f64, w));
                let guess = ia.map(|i| i + density);
                let ib = index_before(h, tb, guess).max(ia);
                density = changes_between(ia, ib);
                values[c * n + k] = prefixes.mean(ia, ib, ta, tb);
                changes[c] += (changes_between(ia, ib)) as u32;
                ia = match ib {
                    Some(ib) if tb >= 0.0 => h.index_at_hint(tb.floor() as u64, ib),
                    _ => index_at(h, tb),
                }
                .max(ib);
            }
            continue;
        }
        let next_time = |i: Option<usize>| {
            let next = i.map_or(0, |i| i + 1);
            if next < h.len() {
                h.time(next) as f64
            } else {
                f64::INFINITY
            }
        };
        let mut i = index_at(h, vp.time_at(0.0, w));
        let mut value = l.sample(i);
        let mut next = next_time(i);
        for c in 0..width {
            let (ta, tb) = (vp.time_at(c as f64, w), vp.time_at((c + 1) as f64, w));
            let (mut sum, mut defined, mut t) = (0.0, true, ta);
            loop {
                let until = next.min(tb);
                if until > t {
                    match value {
                        Sample::Value(v) => sum += v * (until - t),
                        _ => defined = false,
                    }
                }
                if next >= tb {
                    break;
                }
                if next > ta {
                    changes[c] += 1;
                }
                t = next;
                i = Some(i.map_or(0, |i| i + 1));
                value = l.sample(i);
                next = next_time(i);
            }
            values[c * n + k] = if defined {
                Sample::Value(sum / (tb - ta))
            } else {
                Sample::Undefined
            };
        }
    }
    Frame {
        exact: false,
        spans: (0..width).map(|c| (c as f64, c as f64 + 1.0)).collect(),
        values,
        layers: n,
        changes,
        waiting: false,
    }
}

fn index_before(h: &dyn SignalHistory, t: f64, hint: Option<usize>) -> Option<usize> {
    if t <= 0.0 {
        return None;
    }
    let t = (t.ceil() as u64).saturating_sub(1);
    match hint {
        Some(hint) => h.index_at_hint(t, hint),
        None => h.index_at(t),
    }
}

fn changes_between(a: Option<usize>, b: Option<usize>) -> usize {
    match (a, b) {
        (_, None) => 0,
        (None, Some(b)) => b + 1,
        (Some(a), Some(b)) => b.saturating_sub(a),
    }
}

#[derive(Clone, Copy, Default)]
struct Kahan {
    sum: f64,
    carry: f64,
}

impl Kahan {
    fn add(&mut self, v: f64) {
        let y = v - self.carry;
        let t = self.sum + y;
        self.carry = (t - self.sum) - y;
        self.sum = t;
    }
}

pub struct IntegralSummary {
    reading: Reading,
    history: usize,
    len: usize,
    integral: Vec<f64>,
    undefined: Vec<u32>,
    _reservation: volna_trace::remote::memory::Reservation,
}

impl std::fmt::Debug for IntegralSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntegralSummary")
            .field("reading", &self.reading)
            .field("len", &self.len)
            .field("bytes", &self.resident_bytes())
            .finish()
    }
}

impl IntegralSummary {
    pub fn build(
        h: &Arc<dyn SignalHistory>,
        reading: Reading,
        budget: &volna_trace::remote::memory::MemoryBudget,
    ) -> anyhow::Result<Self> {
        let len = h.len();
        let blocks = len.saturating_sub(1).div_ceil(INTEGRAL_BLOCK).max(1);
        let bytes = crate::data::admission::bytes::<f64>(blocks)?
            + crate::data::admission::bytes::<u32>(blocks)?;
        let reservation = budget.reserve_object("the integral summary", bytes)?;
        let mut integral = Vec::with_capacity(blocks);
        let mut undefined = Vec::with_capacity(blocks);
        let (mut sum, mut count) = (Kahan::default(), 0u32);
        for j in 0..len.saturating_sub(1) {
            if j % INTEGRAL_BLOCK == 0 {
                integral.push(sum.sum);
                undefined.push(count);
            }
            let dur = (h.time(j + 1) - h.time(j)) as f64;
            match reading.sample(h.as_ref(), Some(j)) {
                Sample::Value(v) => sum.add(v * dur),
                _ if dur > 0.0 => count += 1,
                _ => {}
            }
        }
        if integral.is_empty() {
            integral.push(0.0);
            undefined.push(0);
        }
        Ok(Self {
            reading,
            history: analog::history_identity(h),
            len,
            integral,
            undefined,
            _reservation: reservation,
        })
    }

    pub fn resident_bytes(&self) -> u64 {
        (self.integral.capacity() * std::mem::size_of::<f64>()
            + self.undefined.capacity() * std::mem::size_of::<u32>()) as u64
    }

    pub fn reading(&self) -> Reading {
        self.reading
    }

    pub fn identity(&self) -> usize {
        self.history
    }

    pub fn matches(&self, h: &Arc<dyn SignalHistory>, reading: Reading) -> bool {
        self.reading == reading
            && self.history == analog::history_identity(h)
            && self.len == h.len()
    }

    pub fn prefix(&self, h: &dyn SignalHistory, k: usize) -> (f64, u32) {
        let span = |j: usize| {
            let dur = (h.time(j + 1) - h.time(j)) as f64;
            match self.reading.sample(h, Some(j)) {
                Sample::Value(v) => (v * dur, 0),
                _ if dur > 0.0 => (0.0, 1),
                _ => (0.0, 0),
            }
        };
        let m = (k / INTEGRAL_BLOCK).min(self.integral.len() - 1);
        let up = m + 1;
        if k % INTEGRAL_BLOCK > INTEGRAL_BLOCK / 2 && up < self.integral.len() {
            let (mut sum, mut count) = (self.integral[up], self.undefined[up]);
            for j in k..up * INTEGRAL_BLOCK {
                let (v, u) = span(j);
                sum -= v;
                count -= u;
            }
            return (sum, count);
        }
        let (mut sum, mut count) = (self.integral[m], self.undefined[m]);
        for j in m * INTEGRAL_BLOCK..k {
            let (v, u) = span(j);
            sum += v;
            count += u;
        }
        (sum, count)
    }

    pub fn mean(
        &self,
        h: &dyn SignalHistory,
        ia: Option<usize>,
        ib: Option<usize>,
        ta: f64,
        tb: f64,
    ) -> Sample {
        Prefixes::new(self, h).mean(ia, ib, ta, tb)
    }
}

struct Prefixes<'a> {
    summary: &'a IntegralSummary,
    h: &'a dyn SignalHistory,
    last: Option<(usize, f64, u32)>,
}

impl<'a> Prefixes<'a> {
    fn new(summary: &'a IntegralSummary, h: &'a dyn SignalHistory) -> Self {
        Self {
            summary,
            h,
            last: None,
        }
    }

    fn at(&mut self, k: usize) -> (f64, u32) {
        let block = k % INTEGRAL_BLOCK;
        let to_block = block.min(INTEGRAL_BLOCK - block);
        let (sum, count) = match self.last {
            Some((last, sum, count)) if k >= last && k - last <= to_block => {
                let (mut sum, mut count) = (sum, count);
                for j in last..k {
                    let dur = (self.h.time(j + 1) - self.h.time(j)) as f64;
                    match self.summary.reading.sample(self.h, Some(j)) {
                        Sample::Value(v) => sum += v * dur,
                        _ if dur > 0.0 => count += 1,
                        _ => {}
                    }
                }
                (sum, count)
            }
            _ => self.summary.prefix(self.h, k),
        };
        self.last = Some((k, sum, count));
        (sum, count)
    }

    fn mean(&mut self, ia: Option<usize>, ib: Option<usize>, ta: f64, tb: f64) -> Sample {
        let h = self.h;
        let value = |i: Option<usize>| self.summary.reading.sample(h, i);
        let Some(ib) = ib.filter(|&ib| Some(ib) != ia) else {
            return match value(ia) {
                Sample::Value(v) => Sample::Value(v),
                _ => Sample::Undefined,
            };
        };
        let first = ia.map_or(0, |i| i + 1);
        let (Sample::Value(va), Sample::Value(vb)) = (value(ia), value(Some(ib))) else {
            return Sample::Undefined;
        };
        let (from, from_undefined) = self.at(first);
        let (to, to_undefined) = self.at(ib);
        if to_undefined > from_undefined {
            return Sample::Undefined;
        }
        let head = va * (h.time(first) as f64 - ta);
        let tail = vb * (tb - h.time(ib) as f64);
        Sample::Value((head + (to - from) + tail) / (tb - ta))
    }
}

pub enum IntegralLoad {
    Building { history: usize },
    Ready(Arc<IntegralSummary>),
    Failed { history: usize },
}

impl IntegralLoad {
    pub fn history(&self) -> usize {
        match self {
            Self::Building { history } | Self::Failed { history } => *history,
            Self::Ready(s) => s.identity(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Readout {
    pub span: usize,
    pub values: Vec<Sample>,
    pub total: Option<f64>,
    pub hot: Option<usize>,
}

impl Readout {
    pub fn share(&self, layer: usize) -> Option<f64> {
        let Sample::Value(v) = self.values.get(layer)? else {
            return None;
        };
        let whole: f64 = self
            .values
            .iter()
            .map(|v| match v {
                Sample::Value(v) => v.abs(),
                _ => 0.0,
            })
            .sum();
        (whole > 0.0).then(|| v.abs() / whole)
    }
}

pub fn readout(frame: &Frame, x: f64, value: f64) -> Option<Readout> {
    let span = frame.span_at(x)?;
    let values = frame.values(span).to_vec();
    let mut hot = None;
    compose(&values, |k, lo, hi| {
        if value >= lo && value < hi {
            hot = Some(k);
        }
    });
    Some(Readout {
        span,
        total: total(&values),
        values,
        hot,
    })
}

pub fn total_edge(layers: &[Layer], from: u64, forward: bool, limit: usize) -> Option<u64> {
    let sum_at = |t: u64| {
        let values = values_at(layers, t);
        total(&values).map(f64::to_bits)
    };
    let mut at = from;
    for _ in 0..limit {
        let t = if forward {
            layers
                .iter()
                .filter_map(|l| l.history.next_change_after(at))
                .min()?
        } else {
            layers
                .iter()
                .filter_map(|l| l.history.prev_change_before(at))
                .max()?
        };
        if sum_at(t) != t.checked_sub(1).map_or(Some(f64::NAN.to_bits()), sum_at) {
            return Some(t);
        }
        at = t;
    }
    None
}

pub fn clean_range((lo, hi): (f64, f64)) -> (f64, f64) {
    fn up(x: f64) -> f64 {
        if x <= 0.0 || !x.is_finite() {
            return 0.0;
        }
        let e = 10f64.powf(x.log10().floor());
        let f = x / e;
        let step = [1.0, 1.2, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0]
            .into_iter()
            .find(|&s| s >= f * (1.0 - 1e-12))
            .unwrap_or(10.0);
        step * e
    }
    let lo = if lo < 0.0 { -up(-lo) } else { 0.0 };
    let hi = up(hi);
    if hi > lo { (lo, hi) } else { (0.0, 1.0) }
}

pub fn changes(layers: &[Layer]) -> usize {
    layers.iter().map(|l| l.history.len()).sum()
}

struct Merge<'a> {
    layers: &'a [Layer],
    at: Vec<Option<usize>>,
    values: Vec<Sample>,
    heap: BinaryHeap<Reverse<(u64, usize)>>,
    pos: f64,
    neg: f64,
    undefined: u32,
    instants: u32,
}

impl<'a> Merge<'a> {
    fn new(layers: &'a [Layer], at: Vec<Option<usize>>) -> Self {
        let values: Vec<Sample> = layers.iter().zip(&at).map(|(l, &i)| l.sample(i)).collect();
        let mut heap = BinaryHeap::with_capacity(layers.len());
        for (k, (l, i)) in layers.iter().zip(&at).enumerate() {
            let next = i.map_or(0, |i| i + 1);
            if next < l.history.len() {
                heap.push(Reverse((l.history.time(next), k)));
            }
        }
        let mut merge = Self {
            layers,
            at,
            values,
            heap,
            pos: 0.0,
            neg: 0.0,
            undefined: 0,
            instants: 0,
        };
        merge.resum();
        merge
    }

    fn resum(&mut self) {
        (self.pos, self.neg, self.undefined) =
            self.values
                .iter()
                .fold((0.0, 0.0, 0u32), |(p, n, u), v| match *v {
                    Sample::Value(v) if v >= 0.0 => (p + v, n, u),
                    Sample::Value(v) => (p, n + v, u),
                    _ => (p, n, u + 1),
                });
    }

    fn next(&self) -> Option<u64> {
        self.heap.peek().map(|Reverse((t, _))| *t)
    }

    fn step(&mut self) -> Option<u64> {
        let t = self.next()?;
        while let Some(&Reverse((tk, k))) = self.heap.peek() {
            if tk != t {
                break;
            }
            self.heap.pop();
            let h = self.layers[k].history.as_ref();
            let mut i = self.at[k].map_or(0, |i| i + 1);
            while i + 1 < h.len() && h.time(i + 1) == t {
                i += 1;
            }
            self.at[k] = Some(i);
            let new = self.layers[k].sample(Some(i));
            for (v, sign) in [(self.values[k], -1.0), (new, 1.0)] {
                match v {
                    Sample::Value(v) if v >= 0.0 => self.pos += sign * v,
                    Sample::Value(v) => self.neg += sign * v,
                    _ if sign > 0.0 => self.undefined += 1,
                    _ => self.undefined -= 1,
                }
            }
            self.values[k] = new;
            if i + 1 < h.len() {
                self.heap.push(Reverse((h.time(i + 1), k)));
            }
        }
        self.instants += 1;
        if self.instants.is_multiple_of(RECOMPOSE_EVERY) {
            self.resum();
        }
        Some(t)
    }

    fn total(&self) -> Option<f64> {
        (self.undefined == 0).then_some(self.pos + self.neg)
    }

    fn reach(&self) -> (f64, f64) {
        if self.undefined == 0 {
            (self.pos, self.neg)
        } else {
            let c = compose(&self.values, |_, _, _| {});
            (c.top, c.bottom)
        }
    }
}

pub type Peak = Option<(f64, f64)>;

fn widen(peak: &mut Peak, v: f64) {
    *peak = Some(match *peak {
        Some((lo, hi)) => (lo.min(v), hi.max(v)),
        None => (v, v),
    });
}

pub fn peaks(layers: &[Layer], vp: &Viewport, width: usize) -> Vec<Peak> {
    let w = width as f64;
    let t0 = vp.time_at(0.0, w);
    let at = layers
        .iter()
        .map(|l| index_at(l.history.as_ref(), t0))
        .collect();
    let mut merge = Merge::new(layers, at);
    let mut out = vec![None; width];
    for (c, peak) in out.iter_mut().enumerate() {
        let (ta, tb) = (vp.time_at(c as f64, w), vp.time_at((c + 1) as f64, w));
        while merge.next().is_some_and(|t| (t as f64) <= ta) {
            merge.step();
        }
        if let Some(v) = merge.total() {
            widen(peak, v);
        }
        while merge.next().is_some_and(|t| (t as f64) < tb) {
            merge.step();
            if let Some(v) = merge.total() {
                widen(peak, v);
            }
        }
    }
    out
}

const TOTAL_BLOCKS: u64 = 1 << 16;
const TOTAL_FANOUT: usize = 16;

pub struct TotalSummary {
    key: Key,
    range: (f64, f64),
    start: u64,
    shift: u32,
    levels: Vec<Vec<Peak>>,
    _reservation: volna_trace::remote::memory::Reservation,
}

impl TotalSummary {
    pub fn build(
        layers: &[Layer],
        span: (u64, u64),
        budget: &volna_trace::remote::memory::MemoryBudget,
    ) -> anyhow::Result<Self> {
        let (start, end) = (span.0, span.1.max(span.0));
        let shift = (0..64)
            .find(|&s| ((end - start) >> s) < TOTAL_BLOCKS)
            .unwrap_or(63);
        let blocks = ((end - start) >> shift) as usize + 1;
        let (arrays, level_count) = crate::data::admission::pyramid(
            blocks,
            TOTAL_FANOUT,
            TOTAL_FANOUT,
            std::mem::size_of::<Vec<Peak>>(),
            crate::data::admission::bytes::<Peak>,
        )?;
        let resident = arrays
            .checked_add(std::mem::size_of::<Self>() as u64)
            .and_then(|n| {
                n.checked_add(crate::data::admission::bytes::<(usize, Reading)>(layers.len()).ok()?)
            })
            .ok_or_else(|| anyhow::anyhow!("analysis storage size overflow"))?;
        let scratch = crate::data::admission::sum([
            crate::data::admission::bytes::<Option<usize>>(layers.len())?,
            crate::data::admission::bytes::<Sample>(layers.len())?,
            crate::data::admission::bytes::<Reverse<(u64, usize)>>(layers.len())?,
        ])?;
        let reservation = budget.reserve_object(
            "the stack total",
            resident
                .checked_add(scratch)
                .ok_or_else(|| anyhow::anyhow!("analysis storage size overflow"))?,
        )?;
        let block = |t: u64| ((t.clamp(start, end) - start) >> shift) as usize;
        let mut level: Vec<Peak> = vec![None; blocks];
        let hold = |level: &mut [Peak], total: Option<f64>, since: u64, until: u64| {
            if let Some(v) = total
                && until > since
            {
                for peak in &mut level[block(since)..=block(until - 1)] {
                    widen(peak, v);
                }
            }
        };
        let mut merge = Merge::new(layers, vec![None; layers.len()]);
        let mut range = (0.0f64, 0.0f64);
        let mut account = |merge: &Merge<'_>| {
            let (top, bottom) = merge.reach();
            range = (range.0.min(bottom), range.1.max(top));
        };
        account(&merge);
        let mut since = start;
        while let Some(t) = merge.next() {
            hold(&mut level, merge.total(), since, t);
            merge.step();
            since = since.max(t);
            account(&merge);
        }
        hold(&mut level, merge.total(), since, end + 1);
        drop(merge);
        let mut levels = Vec::with_capacity(level_count);
        levels.push(level);
        while levels.last().is_some_and(|l| l.len() > TOTAL_FANOUT) {
            let above = levels
                .last()
                .unwrap()
                .chunks(TOTAL_FANOUT)
                .map(|chunk| {
                    chunk.iter().flatten().fold(None, |mut peak, &(lo, hi)| {
                        widen(&mut peak, lo);
                        widen(&mut peak, hi);
                        peak
                    })
                })
                .collect();
            levels.push(above);
        }
        Ok(Self {
            key: key(layers),
            range,
            start,
            shift,
            levels,
            _reservation: crate::data::admission::finish(reservation, resident)?,
        })
    }

    fn block_ticks(&self, level: usize) -> f64 {
        (1u64 << self.shift) as f64 * (TOTAL_FANOUT as f64).powi(level as i32)
    }

    pub fn peaks(&self, vp: &Viewport, width: usize) -> Vec<Peak> {
        let mut out = vec![None; width];
        if width == 0 || vp.width() <= 0.0 {
            return out;
        }
        let per_column = vp.width() / width as f64;
        let k = (0..self.levels.len())
            .rev()
            .find(|&k| self.block_ticks(k) <= per_column)
            .unwrap_or(0);
        let (level, ticks) = (&self.levels[k], self.block_ticks(k));
        let w = width as f64;
        let block_at = |t: f64| ((t - self.start as f64) / ticks).floor();
        for (c, peak) in out.iter_mut().enumerate() {
            let a = block_at(vp.time_at(c as f64, w)).max(0.0);
            let b = block_at(vp.time_at((c + 1) as f64, w) - 1e-9 * ticks);
            if b < 0.0 || a >= level.len() as f64 {
                continue;
            }
            let b = (b as usize).min(level.len() - 1);
            for &(lo, hi) in level[a as usize..=b].iter().flatten() {
                widen(peak, lo);
                widen(peak, hi);
            }
        }
        out
    }

    pub fn resident_bytes(&self) -> u64 {
        let blocks: usize = self.levels.iter().map(Vec::capacity).sum();
        (std::mem::size_of::<Self>()
            + self.key.capacity() * std::mem::size_of::<(usize, Reading)>()
            + self.levels.capacity() * std::mem::size_of::<Vec<Peak>>()
            + blocks * std::mem::size_of::<Peak>()) as u64
    }

    pub fn key(&self) -> &Key {
        &self.key
    }

    pub fn range(&self) -> (f64, f64) {
        self.range
    }
}

pub enum TotalLoad {
    Building,
    Ready(Arc<TotalSummary>),
    Failed,
}

pub fn number(v: f64) -> String {
    let v = if v == 0.0 { 0.0 } else { v };
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{v:.0}");
    }
    let digits = match v.abs() {
        a if a >= 1000.0 => 0,
        a if a >= 100.0 => 1,
        a if a >= 10.0 => 2,
        _ => 3,
    };
    let text = format!("{v:.digits$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    if text == "-0" { "0".into() } else { text }
}
