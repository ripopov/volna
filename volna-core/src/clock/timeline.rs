//! Clock edge navigation and interpolation for viewer rulers.

use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stretch {
    pub begin: u64,
    pub end: u64,
    pub period: u64,
    pub first_cycle: u64,
}

impl Stretch {
    #[must_use]
    pub fn edges(&self) -> u64 {
        (self.end - self.begin)
            .checked_div(self.period)
            .map_or(1, |n| n + 1)
    }

    fn edge(&self, k: u64) -> u64 {
        self.begin + k * self.period
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CycleAt {
    pub cycle: u64,
    pub edge: u64,
    pub next_edge: Option<u64>,
    pub fraction: f64,
    pub stopped: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ClockTimeline {
    stretches: Vec<Stretch>,
    open: bool,
}

impl ClockTimeline {
    pub fn new(mut raw: Vec<(u64, u64, u64)>, open: bool) -> Result<ClockTimeline> {
        raw.sort_unstable();
        let mut stretches = Vec::with_capacity(raw.len());
        let mut cycle = 0u64;
        let mut prev_end: Option<u64> = None;
        for (begin, end, period) in raw {
            if end < begin
                || (period == 0 && end != begin)
                || (period != 0 && (end - begin) % period != 0)
            {
                return Err(anyhow::anyhow!(
                    "clock stretch is not a whole number of periods",
                ));
            }
            if prev_end.is_some_and(|p| begin <= p) {
                return Err(anyhow::anyhow!("clock stretches overlap"));
            }
            let s = Stretch {
                begin,
                end,
                period,
                first_cycle: cycle,
            };
            let count = (end - begin)
                .checked_div(period)
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("clock edge count overflow"))?;
            cycle = cycle
                .checked_add(count)
                .ok_or_else(|| anyhow::anyhow!("clock cycle overflow"))?;
            anyhow::ensure!(
                cycle <= usize::MAX as u64 / 2,
                "clock has too many edges for this platform"
            );
            prev_end = Some(end);
            stretches.push(s);
        }
        Ok(ClockTimeline {
            open: open && !stretches.is_empty(),
            stretches,
        })
    }

    #[must_use]
    pub fn stretches(&self) -> &[Stretch] {
        &self.stretches
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    #[must_use]
    pub fn edge_count(&self) -> u64 {
        self.stretches
            .last()
            .map_or(0, |s| s.first_cycle + s.edges())
    }

    fn stretch_at(&self, t: u64) -> Option<usize> {
        self.stretches
            .partition_point(|s| s.begin <= t)
            .checked_sub(1)
    }

    fn index_in(s: &Stretch, t: u64) -> u64 {
        (t - s.begin)
            .checked_div(s.period)
            .map_or(0, |k| k.min(s.edges() - 1))
    }

    fn following(&self, i: usize, k: u64) -> Option<u64> {
        let s = &self.stretches[i];
        if k + 1 < s.edges() {
            Some(s.edge(k + 1))
        } else {
            self.stretches.get(i + 1).map(|n| n.begin)
        }
    }

    #[must_use]
    pub fn cycle_at(&self, t: u64) -> Option<CycleAt> {
        let i = self.stretch_at(t)?;
        let s = &self.stretches[i];
        let k = Self::index_in(s, t);
        let edge = s.edge(k);
        let next_edge = self.following(i, k);
        let fraction = match next_edge {
            Some(n) => (t - edge) as f64 / (n - edge) as f64,
            None => 0.0,
        };
        let stopped = if k + 1 < s.edges() {
            false
        } else {
            match self.stretches.get(i + 1) {
                Some(n) => {
                    let gap = n.begin - s.end;
                    (s.period == 0 || gap > s.period.saturating_mul(2))
                        && (n.period == 0 || gap > n.period.saturating_mul(2))
                }
                None => !self.open,
            }
        };
        Some(CycleAt {
            cycle: s.first_cycle + k,
            edge,
            next_edge,
            fraction,
            stopped,
        })
    }

    #[must_use]
    pub fn edge(&self, cycle: u64) -> Option<u64> {
        let i = self
            .stretches
            .partition_point(|s| s.first_cycle <= cycle)
            .checked_sub(1)?;
        let s = &self.stretches[i];
        let k = cycle - s.first_cycle;
        (k < s.edges()).then(|| s.edge(k))
    }

    #[must_use]
    pub fn next_edge(&self, t: u64) -> Option<u64> {
        match self.stretch_at(t) {
            None => self.stretches.first().map(|s| s.begin),
            Some(i) => self.following(i, Self::index_in(&self.stretches[i], t)),
        }
    }

    #[must_use]
    pub fn prev_edge(&self, t: u64) -> Option<u64> {
        self.cycle_at(t.checked_sub(1)?).map(|c| c.edge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{ClockHistory, ClockView};
    use std::sync::Arc;
    use volna_trace::data::SignalHistory as _;

    #[test]
    fn large_periods_counts_and_display_origins_do_not_overflow() {
        assert!(ClockTimeline::new(vec![(0, u64::MAX, 1)], false).is_err());
        assert!(ClockTimeline::new(vec![(0, usize::MAX as u64 / 2, 1)], false).is_err());
        let timeline = Arc::new(
            ClockTimeline::new(
                vec![(0, u64::MAX - 2, u64::MAX - 2), (u64::MAX, u64::MAX, 0)],
                false,
            )
            .unwrap(),
        );
        assert_eq!(timeline.edge_count(), 3);
        let at = timeline.cycle_at(u64::MAX - 1).unwrap();
        assert_eq!(
            (at.cycle, at.edge, at.next_edge),
            (1, u64::MAX - 2, Some(u64::MAX))
        );
        assert_eq!(at.fraction, 0.5);
        let history = ClockHistory::new(timeline.clone());
        assert_eq!(history.time(history.len() - 1), u64::MAX);
        assert!((1..history.len()).all(|i| history.time(i - 1) <= history.time(i)));
        let view = ClockView {
            origin: Some(u64::MAX),
            ..Default::default()
        };
        assert_eq!(
            view.absolute_cycle(&timeline, i64::MAX),
            Some(i64::MAX as u64 + 2)
        );
    }
}
