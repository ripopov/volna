//! Whole-window frame timing for the status bar and its details popup.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;

pub const TICK_SECONDS: f64 = 0.5;
pub const RECENT_FRAMES: u64 = 120;
const IDLE_TICKS: u32 = 2;

pub const DRAW_BUDGET_MS: f64 = 1000.0 / 60.0;
pub const LATENCY_BUDGET_MS: f64 = 50.0;

pub const BUCKET_EDGES_MS: [f64; 10] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];
pub const BUCKETS: usize = BUCKET_EDGES_MS.len() + 1;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Distribution {
    counts: BTreeMap<u64, u64>,
}

impl Distribution {
    pub fn record(&mut self, nanos: u64, count: u64) {
        if count > 0 {
            *self.counts.entry(nanos).or_default() += count;
        }
    }

    pub fn add(&mut self, other: &Self) {
        for (&nanos, &count) in &other.counts {
            self.record(nanos, count);
        }
    }

    #[must_use]
    pub fn count(&self) -> u64 {
        self.counts.values().sum()
    }

    #[must_use]
    pub fn quantile_ms(&self, q: f64) -> Option<f64> {
        let total = self.count();
        let rank = ((q * total as f64).ceil() as u64).clamp(1, total.max(1));
        let mut seen = 0;
        self.counts.iter().find_map(|(&nanos, &count)| {
            seen += count;
            (seen >= rank).then(|| ms(nanos))
        })
    }

    #[must_use]
    pub fn max_ms(&self) -> Option<f64> {
        self.counts.keys().next_back().map(|&nanos| ms(nanos))
    }

    #[must_use]
    pub fn summary(&self) -> Option<Summary> {
        Some(Summary {
            median: self.quantile_ms(0.5)?,
            p90: self.quantile_ms(0.9)?,
            p99: self.quantile_ms(0.99)?,
            max: self.max_ms()?,
            count: self.count(),
        })
    }

    #[must_use]
    pub fn buckets(&self) -> [u64; BUCKETS] {
        let mut out = [0; BUCKETS];
        for (&nanos, &count) in &self.counts {
            let value = ms(nanos);
            let i = BUCKET_EDGES_MS
                .iter()
                .position(|&edge| value < edge)
                .unwrap_or(BUCKETS - 1);
            out[i] += count;
        }
        out
    }
}

#[must_use]
pub fn bucket_position(value_ms: f64) -> f64 {
    let first = BUCKET_EDGES_MS[0];
    if value_ms < first {
        (value_ms / first).max(0.0)
    } else {
        (1.0 + (value_ms / first).log2()).min(BUCKETS as f64)
    }
}

fn ms(nanos: u64) -> f64 {
    nanos as f64 / 1e6
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameSample {
    pub draw: Distribution,
    pub latency: Distribution,
    pub input: Distribution,
}

impl FrameSample {
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.draw.count()
    }

    fn add(&mut self, other: &Self) {
        self.draw.add(&other.draw);
        self.latency.add(&other.latency);
        self.input.add(&other.input);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub median: f64,
    pub p90: f64,
    pub p99: f64,
    pub max: f64,
    pub count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    #[default]
    Ok,
    Warn,
    Bad,
}

impl Level {
    fn of(p99: f64, budget: f64) -> Self {
        if p99 > 2.0 * budget {
            Level::Bad
        } else if p99 > budget {
            Level::Warn
        } else {
            Level::Ok
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameStatus {
    pub text: String,
    pub level: Level,
    pub idle: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameRow {
    pub title: &'static str,
    pub hint: &'static str,
    pub recent: Option<Summary>,
    pub total: Option<Summary>,
    pub buckets: [u64; BUCKETS],
    pub budget_ms: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameDetails {
    pub rows: Vec<FrameRow>,
    pub fps: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct FrameStats {
    recent: VecDeque<FrameSample>,
    total: FrameSample,
    idle_ticks: u32,
    echo: u64,
    pub details: bool,
}

impl FrameStats {
    pub fn push(&mut self, sample: FrameSample) -> bool {
        let before = self.view();
        if sample.frames() > self.echo {
            self.total.add(&sample);
            self.recent.push_back(sample);
            while self.recent.len() > 1
                && self
                    .recent
                    .iter()
                    .skip(1)
                    .map(FrameSample::frames)
                    .sum::<u64>()
                    >= RECENT_FRAMES
            {
                self.recent.pop_front();
            }
            self.idle_ticks = 0;
        } else {
            self.idle_ticks = self.idle_ticks.saturating_add(1);
        }
        let changed = self.view() != before;
        self.echo = u64::from(changed);
        changed
    }

    pub fn reset(&mut self) {
        self.total = FrameSample::default();
    }

    #[must_use]
    pub fn status(&self) -> Option<FrameStatus> {
        let recent = self.recent_sample();
        let draw = recent.draw.summary()?;
        let latency = recent.latency.summary();
        let mut text = format!("draw {}", format_ms(draw.median));
        if let Some(latency) = latency {
            write!(text, " · lat {}", format_ms(latency.median)).unwrap();
        }
        text += " ms";
        let level = Level::of(draw.p99, DRAW_BUDGET_MS)
            .max(latency.map_or(Level::Ok, |l| Level::of(l.p99, LATENCY_BUDGET_MS)));
        Some(FrameStatus {
            text,
            level,
            idle: self.idle_ticks >= IDLE_TICKS,
        })
    }

    #[must_use]
    pub fn details(&self) -> FrameDetails {
        let recent = self.recent_sample();
        let row = |title, hint, pick: fn(&FrameSample) -> &Distribution, budget_ms| FrameRow {
            title,
            hint,
            recent: pick(&recent).summary(),
            total: pick(&self.total).summary(),
            buckets: pick(&self.total).buckets(),
            budget_ms,
        };
        let seconds = self.recent.len() as f64 * TICK_SECONDS;
        FrameDetails {
            rows: vec![
                row(
                    "Input latency",
                    "Key press or mouse move until the result is on screen",
                    |s| &s.input,
                    Some(LATENCY_BUDGET_MS),
                ),
                row(
                    "Change-to-screen latency",
                    "Any change, including background data, until it is on screen",
                    |s| &s.latency,
                    Some(LATENCY_BUDGET_MS),
                ),
                row(
                    "Draw time",
                    "CPU time to build and paint the whole window",
                    |s| &s.draw,
                    Some(DRAW_BUDGET_MS),
                ),
            ],
            fps: (seconds > 0.0).then(|| recent.frames() as f64 / seconds),
        }
    }

    #[must_use]
    pub fn debug_state(&self) -> String {
        let status = self.status();
        format!(
            "frames recent={} total={} status={:?} level={:?} idle={} details={}",
            self.recent_sample().frames(),
            self.total.frames(),
            status.as_ref().map(|s| s.text.as_str()),
            status.as_ref().map(|s| s.level),
            status.as_ref().is_some_and(|s| s.idle),
            self.details,
        )
    }

    fn recent_sample(&self) -> FrameSample {
        let mut out = FrameSample::default();
        for sample in &self.recent {
            out.add(sample);
        }
        out
    }

    fn view(&self) -> (Option<FrameStatus>, Option<FrameDetails>) {
        (self.status(), self.details.then(|| self.details()))
    }
}

#[must_use]
pub fn format_ms(value: f64) -> String {
    if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(draw_ms: &[(f64, u64)], latency_ms: &[(f64, u64)]) -> FrameSample {
        let mut s = FrameSample::default();
        for &(value, count) in draw_ms {
            s.draw.record((value * 1e6) as u64, count);
        }
        for &(value, count) in latency_ms {
            s.latency.record((value * 1e6) as u64, count);
        }
        s
    }

    #[test]
    fn quantiles_use_nearest_rank() {
        let mut d = Distribution::default();
        d.record(1_000_000, 98);
        d.record(20_000_000, 1);
        d.record(80_000_000, 1);
        assert_eq!(d.count(), 100);
        assert_eq!(d.quantile_ms(0.5), Some(1.0));
        assert_eq!(d.quantile_ms(0.99), Some(20.0));
        assert_eq!(d.max_ms(), Some(80.0));
        assert_eq!(Distribution::default().summary(), None);
    }

    #[test]
    fn buckets_split_on_log_edges() {
        let mut d = Distribution::default();
        d.record(100_000, 1);
        d.record(1_000_000, 2);
        d.record(17_000_000, 3);
        d.record(500_000_000, 4);
        let b = d.buckets();
        assert_eq!(b[0], 1);
        assert_eq!(b[3], 2);
        assert_eq!(b[7], 3);
        assert_eq!(b[BUCKETS - 1], 4);
    }

    #[test]
    fn chart_positions_follow_the_bucket_edges() {
        assert_eq!(bucket_position(0.125), 0.5);
        assert_eq!(bucket_position(0.25), 1.0);
        assert_eq!(bucket_position(16.0), 7.0);
        assert!((bucket_position(DRAW_BUDGET_MS) - 7.059).abs() < 0.001);
        assert_eq!(bucket_position(1e6), BUCKETS as f64);
    }

    #[test]
    fn status_shows_medians_of_the_recent_frames() {
        let mut stats = FrameStats::default();
        assert_eq!(stats.status(), None);
        assert!(stats.push(sample(&[(1.1, 60)], &[(9.0, 60)])));
        let status = stats.status().unwrap();
        assert_eq!(status.text, "draw 1.1 · lat 9.0 ms");
        assert_eq!(status.level, Level::Ok);
        assert!(!status.idle);
    }

    #[test]
    fn recent_window_keeps_at_least_120_frames() {
        let mut stats = FrameStats::default();
        stats.push(sample(&[(50.0, 100)], &[]));
        stats.push(sample(&[(1.0, 100)], &[]));
        assert_eq!(stats.recent_sample().frames(), 200);
        stats.push(sample(&[(1.0, 30)], &[]));
        assert_eq!(stats.recent_sample().frames(), 130);
        assert_eq!(stats.status().unwrap().level, Level::Ok);
        assert_eq!(stats.details().rows[2].total.unwrap().count, 230);
    }

    #[test]
    fn slow_tail_tints_the_status() {
        let mut stats = FrameStats::default();
        stats.push(sample(&[(1.0, 97), (20.0, 3)], &[]));
        assert_eq!(stats.status().unwrap().level, Level::Warn);
        stats.push(sample(&[(1.0, 97), (40.0, 3)], &[(8.0, 100)]));
        assert_eq!(stats.status().unwrap().level, Level::Bad);
        let mut laggy = FrameStats::default();
        laggy.push(sample(&[(1.0, 100)], &[(8.0, 97), (60.0, 3)]));
        assert_eq!(laggy.status().unwrap().level, Level::Warn);
    }

    #[test]
    fn own_redraws_settle_to_idle() {
        let mut stats = FrameStats::default();
        assert!(stats.push(sample(&[(1.0, 30)], &[(5.0, 30)])));
        assert!(!stats.push(sample(&[(3.0, 1)], &[(3.0, 1)])));
        assert_eq!(
            stats.recent_sample().frames(),
            30,
            "echo frame is not counted"
        );
        assert!(stats.push(FrameSample::default()));
        assert!(stats.status().unwrap().idle);
        assert!(!stats.push(sample(&[(3.0, 1)], &[])));
        for _ in 0..5 {
            assert!(!stats.push(FrameSample::default()));
        }
        assert_eq!(stats.status().unwrap().text, "draw 1.0 · lat 5.0 ms");
        assert!(stats.push(sample(&[(2.0, 200)], &[])));
        assert!(!stats.status().unwrap().idle);
    }

    #[test]
    fn open_details_count_as_visible() {
        let mut stats = FrameStats::default();
        stats.push(sample(&[(1.0, 200)], &[]));
        assert!(!stats.push(sample(&[(1.0, 200)], &[])));
        stats.details = true;
        assert!(stats.push(sample(&[(1.0, 200)], &[])));
        let details = stats.details();
        assert_eq!(details.rows[2].total.unwrap().count, 600);
        assert_eq!(details.rows[2].budget_ms, Some(DRAW_BUDGET_MS));
        assert_eq!(details.fps, Some(200.0 / TICK_SECONDS));
    }

    #[test]
    fn reset_clears_totals_only() {
        let mut stats = FrameStats::default();
        stats.push(sample(&[(1.0, 50)], &[]));
        stats.reset();
        let draw = &stats.details().rows[2];
        assert_eq!(draw.total, None);
        assert_eq!(draw.buckets, [0; BUCKETS]);
        assert_eq!(draw.recent.unwrap().count, 50);
    }

    #[test]
    fn milliseconds_format() {
        assert_eq!(format_ms(0.04), "0.0");
        assert_eq!(format_ms(9.94), "9.9");
        assert_eq!(format_ms(16.7), "17");
    }
}
