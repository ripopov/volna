//! Declared clocks: the catalog of a trace's `CLOCK` generators, their timelines built from
//! loaded stretches, each panel's clock choices, and the ruler and readout arithmetic every
//! timed panel shares.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
pub use vtr::{ClockTimeline, CycleAt};

use crate::data::loaded_tracks::LoadedGenerator;
use crate::trace::{TraceId, Traced};
use crate::wave::timeline::TimeBase;
use crate::wave::viewport::Viewport;
use volna_trace::data::transactions::{AttributeValue, Track, TrackKind, TrackRef, TxStatus};

pub const GENERATOR_KIND: &str = vtr::CLOCK_GENERATOR_KIND;
pub const LINK_ATTRIBUTE: &str = vtr::clock::KEY_CLOCK;
pub const PERIOD_ATTRIBUTE: &str = vtr::clock::KEY_PERIOD;

#[derive(Clone, Debug)]
pub enum ClockState {
    Loading,
    Ready(Arc<ClockTimeline>),
    Failed(String),
}

pub type ClockKey = Traced<String>;

#[derive(Clone, Debug)]
pub struct Clock {
    pub trace: TraceId,
    pub track: TrackRef,
    pub path: String,
    pub name: String,
    pub state: ClockState,
}

impl Clock {
    #[must_use]
    pub fn timeline(&self) -> Option<&Arc<ClockTimeline>> {
        match &self.state {
            ClockState::Ready(t) => Some(t),
            _ => None,
        }
    }

    #[must_use]
    pub fn key(&self) -> ClockKey {
        Traced::new(self.trace, self.path.clone())
    }

    #[must_use]
    pub fn is(&self, key: &ClockKey) -> bool {
        self.trace == key.trace && self.path == key.item
    }

    #[must_use]
    pub fn generator(&self) -> Traced<TrackRef> {
        Traced::new(self.trace, self.track)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Clocks {
    clocks: Vec<Clock>,
    links: HashMap<Traced<TrackRef>, TrackRef>,
    pub defaults: Vec<ClockKey>,
}

impl Clocks {
    pub(crate) fn add_trace(&mut self, trace: TraceId, tracks: &[Track]) {
        let first = self.clocks.len();
        self.clocks.extend(
            tracks
                .iter()
                .filter(
                    |t| matches!(&t.kind, TrackKind::Generator { kind } if kind == GENERATOR_KIND),
                )
                .map(|t| Clock {
                    trace,
                    track: t.id,
                    path: t.path.join("."),
                    name: t.path.last().cloned().unwrap_or_default(),
                    state: ClockState::Loading,
                }),
        );
        let added = &self.clocks[first..];
        for t in tracks {
            if let Some(path) = t.attributes.iter().find_map(|(k, v)| match v {
                AttributeValue::Text(p) if k == LINK_ATTRIBUTE => Some(p),
                _ => None,
            }) && let Some(clock) = added.iter().find(|c| &c.path == path)
            {
                self.links.insert(Traced::new(trace, t.id), clock.track);
            }
        }
    }

    pub(crate) fn remove_trace(&mut self, trace: TraceId) {
        self.clocks.retain(|c| c.trace != trace);
        self.links.retain(|track, _| track.trace != trace);
        self.defaults.retain(|key| key.trace != trace);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.clocks.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Clock> {
        self.clocks.iter()
    }

    #[must_use]
    pub fn get(&self, ix: usize) -> Option<&Clock> {
        self.clocks.get(ix)
    }

    #[must_use]
    pub fn find(&self, key: &ClockKey) -> Option<&Clock> {
        self.clocks.iter().find(|c| c.is(key))
    }

    fn of_generator(&self, generator: Traced<TrackRef>) -> Option<&Clock> {
        self.clocks.iter().find(|c| c.generator() == generator)
    }

    #[must_use]
    pub fn of_track(&self, track: Traced<TrackRef>, tracks: &[Track]) -> Option<&Clock> {
        tracks.iter().find(|t| t.id == track.item)?;
        self.of_generator(track)
    }

    #[must_use]
    pub fn linked(&self, track: Traced<TrackRef>) -> Option<&Clock> {
        self.of_generator(track.with(*self.links.get(&track)?))
    }

    #[must_use]
    pub fn is_clock_track(&self, track: Traced<TrackRef>) -> bool {
        self.of_generator(track).is_some()
    }

    pub(crate) fn deliver(
        &mut self,
        track: Traced<TrackRef>,
        result: Result<&[Arc<LoadedGenerator>], &str>,
    ) {
        let Some(clock) = self.clocks.iter_mut().find(|c| c.generator() == track) else {
            return;
        };
        clock.state = match result
            .map_err(str::to_owned)
            .and_then(|g| timeline_of(g).map_err(|e| format!("{e:#}")))
        {
            Ok(t) => ClockState::Ready(Arc::new(t)),
            Err(e) => ClockState::Failed(e),
        };
    }

    pub(crate) fn reload(&mut self, trace: TraceId) {
        for clock in self.clocks.iter_mut().filter(|c| c.trace == trace) {
            clock.state = ClockState::Loading;
        }
    }
}

pub fn timeline_of(generators: &[Arc<LoadedGenerator>]) -> anyhow::Result<ClockTimeline> {
    let mut raw = Vec::new();
    let mut open = false;
    for tx in generators.iter().flat_map(|g| g.transactions()) {
        let period = tx
            .attributes
            .iter()
            .find(|a| a.key == PERIOD_ATTRIBUTE)
            .map_or(Ok(0), |a| match a.value {
                AttributeValue::Time(p) | AttributeValue::U64(p) => Ok(p),
                _ => Err(anyhow::anyhow!("{PERIOD_ATTRIBUTE} is not a time")),
            })?;
        open |= tx.status == TxStatus::Open;
        raw.push((tx.begin, tx.end, period));
    }
    Ok(ClockTimeline::new(raw, open)?)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rulers: Option<Vec<ClockKey>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<ClockKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<u64>,
}

impl ClockView {
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    #[must_use]
    pub fn ruler_keys<'a>(&'a self, clocks: &'a Clocks) -> &'a [ClockKey] {
        self.rulers.as_deref().unwrap_or(&clocks.defaults)
    }

    #[must_use]
    pub fn rulers<'a>(&self, clocks: &'a Clocks) -> Vec<&'a Clock> {
        self.ruler_keys(clocks)
            .iter()
            .filter_map(|k| clocks.find(k))
            .collect()
    }

    #[must_use]
    pub fn selected<'a>(&self, clocks: &'a Clocks) -> Option<&'a Clock> {
        self.selected
            .as_ref()
            .and_then(|k| clocks.find(k))
            .or_else(|| self.rulers(clocks).into_iter().next())
    }

    pub fn show_ruler(&mut self, clocks: &Clocks, key: &ClockKey) {
        let mut rulers = self.ruler_keys(clocks).to_vec();
        if !rulers.contains(key) {
            rulers.push(key.clone());
        }
        self.rulers = Some(rulers);
    }

    pub fn hide_ruler(&mut self, clocks: &Clocks, key: &ClockKey) {
        if self.ruler_keys(clocks).contains(key) {
            self.toggle_ruler(clocks, key);
        }
    }

    pub fn toggle_ruler(&mut self, clocks: &Clocks, key: &ClockKey) {
        let mut rulers = self.ruler_keys(clocks).to_vec();
        match rulers.iter().position(|k| k == key) {
            Some(ix) => {
                rulers.remove(ix);
                if self.selected.as_ref() == Some(key) {
                    self.selected = None;
                }
            }
            None => rulers.push(key.clone()),
        }
        self.rulers = Some(rulers);
    }

    pub(crate) fn forget_trace(&mut self, trace: TraceId) -> bool {
        if self.selected.as_ref().is_some_and(|k| k.trace == trace) {
            self.selected = None;
        }
        let Some(rulers) = &mut self.rulers else {
            return false;
        };
        let before = rulers.len();
        rulers.retain(|k| k.trace != trace);
        rulers.len() != before
    }

    #[must_use]
    pub fn origin_cycle(&self, timeline: &ClockTimeline) -> u64 {
        self.origin
            .and_then(|t| timeline.cycle_at(t))
            .map_or(0, |c| c.cycle)
    }

    #[must_use]
    pub fn display_cycle(&self, timeline: &ClockTimeline, cycle: u64) -> i64 {
        cycle as i64 - self.origin_cycle(timeline) as i64
    }

    #[must_use]
    pub fn absolute_cycle(&self, timeline: &ClockTimeline, shown: i64) -> Option<u64> {
        u64::try_from(shown + self.origin_cycle(timeline) as i64).ok()
    }
}

#[must_use]
pub fn nearest_edge(timeline: &ClockTimeline, t: f64, tolerance: f64) -> Option<u64> {
    let at = t.max(0.0).floor() as u64;
    let before = timeline.cycle_at(at).map(|c| c.edge);
    let after = timeline.next_edge(at);
    [before, after]
        .into_iter()
        .flatten()
        .map(|e| ((e as f64 - t).abs(), e))
        .filter(|(d, _)| *d <= tolerance)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, e)| e)
}

pub struct ClockHistory {
    timeline: Arc<ClockTimeline>,
}

impl ClockHistory {
    #[must_use]
    pub fn new(timeline: Arc<ClockTimeline>) -> Self {
        Self { timeline }
    }

    fn period(&self, cycle: u64) -> u64 {
        let s = self.timeline.stretches();
        let i = s
            .partition_point(|s| s.first_cycle <= cycle)
            .saturating_sub(1);
        s.get(i).map_or(0, |s| s.period)
    }
}

impl volna_trace::data::SignalHistory for ClockHistory {
    fn shape(&self) -> volna_trace::data::SignalShape {
        volna_trace::data::SignalShape::Bit
    }

    fn always_normal(&self) -> bool {
        true
    }

    fn len(&self) -> usize {
        (self.timeline.edge_count() * 2) as usize
    }

    fn time(&self, i: usize) -> u64 {
        let cycle = (i / 2) as u64;
        let edge = self.timeline.edge(cycle).unwrap_or(0);
        if i.is_multiple_of(2) {
            return edge;
        }
        let fall = edge + (self.period(cycle) / 2).max(1);
        self.timeline
            .edge(cycle + 1)
            .map_or(fall, |next| fall.min(next))
    }

    fn value(&self, i: Option<usize>) -> volna_trace::data::WaveValue {
        match i {
            None => volna_trace::data::WaveValue::Unavailable,
            Some(i) => {
                volna_trace::data::WaveValue::Bits(if i % 2 == 0 { "1" } else { "0" }.into())
            }
        }
    }

    fn bit(&self, i: Option<usize>) -> volna_trace::data::Bit {
        match i {
            None => volna_trace::data::Bit::Unavailable,
            Some(i) if i % 2 == 0 => volna_trace::data::Bit::One,
            Some(_) => volna_trace::data::Bit::Zero,
        }
    }
}

#[must_use]
pub fn format_position(view: &ClockView, timeline: &ClockTimeline, at: &CycleAt) -> String {
    let cycle = view.display_cycle(timeline, at.cycle);
    if at.stopped {
        format!("{cycle} (stopped)")
    } else if at.fraction > 0.0 {
        format!("{cycle} + {:.2}", at.fraction)
    } else {
        cycle.to_string()
    }
}

#[must_use]
pub fn position_at(view: &ClockView, clock: &Clock, time: u64) -> String {
    match clock.timeline().and_then(|t| Some((t, t.cycle_at(time)?))) {
        Some((t, at)) => format!("{} {}", clock.name, format_position(view, t, &at)),
        None => format!("{} –", clock.name),
    }
}

#[must_use]
pub fn speed_label(period: u64, base: TimeBase<'_>) -> String {
    match frequency(period as f64, base) {
        Some(f) => format!("→ {f}"),
        None => format!("→ {period} {}", base.unit.unwrap_or_default()),
    }
}

#[must_use]
pub fn frequency(period: f64, base: TimeBase<'_>) -> Option<String> {
    if base.unit.is_some() || period <= 0.0 {
        return None;
    }
    let hz = 1.0 / (period * 10f64.powi(i32::from(base.timescale)));
    let (value, suffix) = [(1e9, "GHz"), (1e6, "MHz"), (1e3, "kHz"), (1.0, "Hz")]
        .into_iter()
        .find(|(scale, _)| hz * (1.0 + 1e-9) >= *scale)
        .map_or((hz, "Hz"), |(scale, s)| (hz / scale, s));
    Some(format!("{value:.2} {suffix}"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct RulerTick {
    pub time: u64,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RulerMarks {
    pub ticks: Vec<RulerTick>,
    pub flags: Vec<(u64, String)>,
    pub stopped: Vec<(f64, f64)>,
}

pub const MIN_TICK_PX: f64 = 4.0;
const MAX_TICKS: usize = 4000;
const NICE: [u64; 3] = [1, 2, 5];

fn label_step(cycle_px: f64, min_px: f64) -> u64 {
    let mut magnitude = 1u64;
    loop {
        for n in NICE {
            let step = n * magnitude;
            if step as f64 * cycle_px >= min_px || magnitude > u64::MAX / 100 {
                return step;
            }
        }
        magnitude *= 10;
    }
}

#[must_use]
pub fn ruler_marks(
    view: &ClockView,
    timeline: &ClockTimeline,
    viewport: &Viewport,
    width_px: f64,
    min_label_px: f64,
    base: TimeBase<'_>,
) -> RulerMarks {
    let mut marks = RulerMarks::default();
    let stretches = timeline.stretches();
    if stretches.is_empty() || width_px <= 0.0 {
        return marks;
    }
    let ppu = viewport.px_per_unit(width_px);
    let origin = view.origin_cycle(timeline) as i64;
    let (lo, hi) = (viewport.start.max(0.0), viewport.end.max(0.0));
    let first = stretches
        .partition_point(|s| (s.end as f64) < lo)
        .saturating_sub(1);
    let last = first + stretches[first..].partition_point(|s| (s.begin as f64) <= hi);
    for (i, s) in stretches.iter().enumerate().take(last).skip(first) {
        let next_begin = stretches.get(i + 1).map(|n| n.begin);
        let gap_end = match next_begin {
            Some(b) => {
                let at = timeline.cycle_at(s.end);
                at.filter(|a| a.stopped).map(|_| b as f64)
            }
            None => (!timeline.is_open()).then_some(f64::INFINITY),
        };
        if let Some(end) = gap_end
            && (s.end as f64) < hi
            && end > lo
        {
            marks.stopped.push((s.end as f64, end.min(viewport.end)));
        }
        if i > 0
            && s.period != 0
            && s.period != stretches[i - 1].period
            && (lo..=hi).contains(&(s.begin as f64))
        {
            marks.flags.push((s.begin, speed_label(s.period, base)));
        }
        if (s.end as f64) < lo || (s.begin as f64) > hi || marks.ticks.len() >= MAX_TICKS {
            continue;
        }
        let edges = s.edges();
        let cycle_px = if s.period == 0 {
            f64::INFINITY
        } else {
            s.period as f64 * ppu
        };
        let step = label_step(cycle_px, min_label_px);
        let k0 = if s.period == 0 || lo <= s.begin as f64 {
            0
        } else {
            ((lo - s.begin as f64) / s.period as f64).floor() as u64
        };
        let k1 = if s.period == 0 {
            0
        } else {
            (((hi - s.begin as f64) / s.period as f64).floor() as u64).min(edges - 1)
        };
        let shown = |k: u64| (s.first_cycle + k) as i64 - origin;
        let label = |k: u64| (shown(k).rem_euclid(step as i64) == 0).then(|| shown(k).to_string());
        if cycle_px >= MIN_TICK_PX {
            for k in k0..=k1 {
                if marks.ticks.len() >= MAX_TICKS {
                    break;
                }
                marks.ticks.push(RulerTick {
                    time: s.begin + k * s.period,
                    label: label(k),
                });
            }
        } else {
            let first = shown(k0);
            let mut k = k0
                + u64::from(first.rem_euclid(step as i64) != 0)
                    * (step as i64 - first.rem_euclid(step as i64)) as u64;
            while k <= k1 && marks.ticks.len() < MAX_TICKS {
                marks.ticks.push(RulerTick {
                    time: s.begin + k * s.period,
                    label: label(k),
                });
                k += step;
            }
        }
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> ClockTimeline {
        ClockTimeline::new(
            vec![(400, 19772, 334), (20272, 49772, 500), (50772, 99772, 1000)],
            true,
        )
        .unwrap()
    }

    #[test]
    fn steps_follow_the_1_2_5_series() {
        assert_eq!(label_step(100.0, 64.0), 1);
        assert_eq!(label_step(40.0, 64.0), 2);
        assert_eq!(label_step(20.0, 64.0), 5);
        assert_eq!(label_step(1.0, 64.0), 100);
    }

    #[test]
    fn ruler_ticks_every_edge_and_labels_nice_cycles() {
        let t = demo();
        let view = ClockView::default();
        let vp = Viewport {
            start: 0.0,
            end: 5000.0,
        };
        let m = ruler_marks(&view, &t, &vp, 1000.0, 64.0, TimeBase::si(-12));
        let times: Vec<u64> = m.ticks.iter().map(|t| t.time).collect();
        assert_eq!(times, (0..14).map(|k| 400 + k * 334).collect::<Vec<_>>());
        assert!(m.ticks.iter().all(|t| t.label.is_some()));
        assert_eq!(m.ticks[3].label.as_deref(), Some("3"));
        assert!(m.flags.is_empty() && m.stopped.is_empty());
        let vp = Viewport {
            start: 0.0,
            end: 100_000.0,
        };
        let m = ruler_marks(&view, &t, &vp, 1000.0, 64.0, TimeBase::si(-12));
        let first: Vec<_> = m.ticks.iter().filter(|t| t.time < 20000).collect();
        assert!(first.iter().all(|t| {
            t.label
                .as_deref()
                .is_some_and(|l| l.parse::<i64>().unwrap() % 20 == 0)
        }));
        let last: Vec<_> = m.ticks.iter().filter(|t| t.time >= 50772).collect();
        assert_eq!(last.len(), 50);
        assert!(last.iter().all(|t| {
            t.label
                .as_ref()
                .is_none_or(|l| l.parse::<i64>().unwrap() % 10 == 0)
        }));
        assert_eq!(last.iter().filter(|t| t.label.is_some()).count(), 5);
        assert_eq!(
            m.flags,
            vec![(20272, "→ 2.00 GHz".into()), (50772, "→ 1.00 GHz".into())]
        );
    }

    #[test]
    fn origin_renumbers_and_stopped_is_hatched() {
        let t = ClockTimeline::new(vec![(0, 100, 10), (1000, 1100, 10)], false).unwrap();
        let view = ClockView {
            origin: Some(55),
            ..Default::default()
        };
        assert_eq!(view.display_cycle(&t, 5), 0);
        assert_eq!(view.absolute_cycle(&t, -5), Some(0));
        let at = t.cycle_at(57).unwrap();
        assert_eq!(format_position(&view, &t, &at), "0 + 0.70");
        let vp = Viewport {
            start: 0.0,
            end: 2000.0,
        };
        let m = ruler_marks(&view, &t, &vp, 2000.0, 20.0, TimeBase::si(-9));
        assert_eq!(m.stopped, vec![(100.0, 1000.0), (1100.0, 2000.0)]);
        assert_eq!(m.ticks[0].label, None);
        assert_eq!(m.ticks[1].label.as_deref(), Some("-4"));
    }

    #[test]
    fn speeds_and_deltas() {
        assert_eq!(speed_label(500, TimeBase::si(-12)), "→ 2.00 GHz");
        assert_eq!(speed_label(3, TimeBase::si(-9)), "→ 333.33 MHz");
        let cycles = TimeBase {
            timescale: 0,
            unit: Some("cycle"),
        };
        assert_eq!(speed_label(2, cycles), "→ 2 cycle");
        assert_eq!(
            crate::measure::cycles(&demo(), 400, 20272 + 500),
            crate::measure::Cycles::Whole(60)
        );
    }
}
