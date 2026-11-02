//! Exact concurrency integrals and a peak hierarchy over transaction boundaries.

use crate::data::admission::Storage;
use volna_trace::data::transactions::{Status, Transaction};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OccupancySample {
    pub average: f64,
    pub peak: usize,
    pub failed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OccupancyBin {
    pub average: f64,
    pub failed: bool,
    pub has_points: bool,
}

// Keep integer differences exact so short bursts remain measurable after
// very large accumulated lifetimes.
#[derive(Clone, Copy, Default)]
struct Integral {
    whole: u128,
    fraction: f64,
}

impl Integral {
    fn at(time: f64, boundary: u64, count: usize, integral: u128) -> Self {
        let integer = time.max(0.0).floor() as u64;
        Self {
            whole: integral + u128::from(integer - boundary) * count as u128,
            fraction: (time.max(0.0) - integer as f64) * count as f64,
        }
    }

    fn since(self, start: Self) -> f64 {
        ((self.whole - start.whole) as f64 + self.fraction - start.fraction).max(0.0)
    }
}

#[derive(Clone, Copy, Default)]
struct ActivityPrefix {
    duration: Integral,
    failed_duration: Integral,
    points: usize,
    failed_points: usize,
}

impl ActivityPrefix {
    fn since(self, start: Self, width: f64) -> OccupancyBin {
        OccupancyBin {
            average: self.duration.since(start.duration) / width,
            failed: self.failed_duration.since(start.failed_duration) > 0.0
                || self.failed_points > start.failed_points,
            has_points: self.points > start.points,
        }
    }
}

#[derive(Debug)]
pub struct Occupancy {
    times: Vec<u64>,
    counts: Vec<usize>,
    integrals: Vec<u128>,
    peaks: Vec<(usize, bool)>,
    leaves: usize,
    error_points: Vec<u64>,
    points: Vec<u64>,
    failed_counts: Vec<usize>,
    failed_integrals: Vec<u128>,
}

impl Occupancy {
    pub(crate) fn build(records: &[Transaction], storage: &mut Storage) -> anyhow::Result<Self> {
        let (mut spans, mut points) = (0usize, 0usize);
        for tx in records {
            if tx.begin < tx.end {
                spans += 1;
            } else if tx.status == Status::Error {
                points += 1;
            }
        }
        let mut events = storage.vec(
            spans
                .checked_mul(2)
                .ok_or_else(|| anyhow::anyhow!("occupancy size overflow"))?,
        )?;
        let mut error_points = storage.vec(points)?;
        let mut all_points = storage.vec(records.len() - spans)?;
        let has_errors = records
            .iter()
            .any(|tx| tx.begin < tx.end && tx.status == Status::Error);
        for tx in records {
            if tx.begin == tx.end {
                all_points.push(tx.begin);
            }
            if tx.begin < tx.end {
                let failed = i64::from(tx.status == Status::Error);
                events.push((tx.begin, 1i64, failed));
                events.push((tx.end, -1, -failed));
            } else if tx.status == Status::Error {
                error_points.push(tx.begin);
            }
        }
        events.sort_unstable_by_key(|event| event.0);
        let mut unique = 0;
        let mut last = None;
        for event in &events {
            if last != Some(event.0) {
                unique += 1;
                last = Some(event.0);
            }
        }
        let mut times = storage.vec(unique)?;
        let mut counts = storage.vec(unique)?;
        let mut integrals = storage.vec(unique)?;
        let mut failures = storage.vec(unique)?;
        let mut failed_counts = storage.vec(if has_errors { unique } else { 0 })?;
        let mut failed_integrals = storage.vec(if has_errors { unique } else { 0 })?;
        let mut failed_integral = 0u128;
        let (mut count, mut failed, mut integral, mut previous) = (0i64, 0i64, 0u128, 0);
        let mut i = 0;
        while i < events.len() {
            let time = events[i].0;
            integral += u128::from(time - previous) * count as u128;
            failed_integral += u128::from(time - previous) * failed as u128;
            while i < events.len() && events[i].0 == time {
                count += events[i].1;
                failed += events[i].2;
                i += 1;
            }
            times.push(time);
            counts.push(count as usize);
            integrals.push(integral);
            failures.push(failed > 0);
            if has_errors {
                failed_counts.push(failed as usize);
                failed_integrals.push(failed_integral);
            }
            previous = time;
        }
        storage.release(events)?;
        let leaves = times
            .len()
            .max(1)
            .checked_next_power_of_two()
            .ok_or_else(|| anyhow::anyhow!("occupancy size overflow"))?;
        let count = leaves
            .checked_mul(2)
            .ok_or_else(|| anyhow::anyhow!("occupancy size overflow"))?;
        let mut peaks = storage.vec(count)?;
        peaks.resize(count, (0, false));
        for (i, (&count, &failed)) in counts.iter().zip(&failures).enumerate() {
            peaks[leaves + i] = (count, failed);
        }
        for i in (1..leaves).rev() {
            peaks[i] = combine(peaks[2 * i], peaks[2 * i + 1]);
        }
        storage.release(failures)?;
        Ok(Self {
            times,
            counts,
            integrals,
            peaks,
            leaves,
            error_points,
            points: all_points,
            failed_counts,
            failed_integrals,
        })
    }

    #[must_use]
    pub fn at(&self, time: u64) -> (usize, bool) {
        let n = self.times.partition_point(|&t| t <= time);
        let (count, failed) = n
            .checked_sub(1)
            .map_or((0, false), |i| self.peaks[self.leaves + i]);
        (
            count,
            failed || self.error_points.binary_search(&time).is_ok(),
        )
    }

    #[must_use]
    pub fn maximum(&self) -> usize {
        self.peaks[1].0
    }

    #[must_use]
    pub fn sample(&self, start: f64, end: f64) -> OccupancySample {
        if start >= end || end <= 0.0 {
            return OccupancySample::default();
        }
        let first = self
            .times
            .partition_point(|&t| at_or_before(t, start))
            .saturating_sub(1);
        let last = self.times.partition_point(|&t| before(t, end));
        let (mut a, mut b) = (self.leaves + first, self.leaves + last);
        let mut peak = (0, false);
        while a < b {
            if a % 2 == 1 {
                peak = combine(peak, self.peaks[a]);
                a += 1;
            }
            if b % 2 == 1 {
                b -= 1;
                peak = combine(peak, self.peaks[b]);
            }
            a /= 2;
            b /= 2;
        }
        OccupancySample {
            average: self.integral_at(end).since(self.integral_at(start)) / (end - start),
            peak: peak.0,
            failed: peak.1
                || self
                    .error_points
                    .get(self.error_points.partition_point(|&t| before(t, start)))
                    .is_some_and(|&t| before(t, end)),
        }
    }

    /// Samples equal time bins without allocating or scanning transactions.
    /// Point records mark their bin without contributing to its average.
    /// Empty or non-finite windows yield no bins.
    pub fn bins(
        &self,
        start: f64,
        end: f64,
        count: usize,
    ) -> impl Iterator<Item = OccupancyBin> + '_ {
        let span = end - start;
        let count = if start.is_finite() && end.is_finite() && span.is_finite() && span > 0.0 {
            count
        } else {
            0
        };
        let step = span / count.max(1) as f64;
        let previous = (start, self.prefix_at(start));
        (0..count).scan(previous, move |(previous_time, previous), i| {
            let time = if i + 1 == count {
                end
            } else {
                start + (i + 1) as f64 * step
            };
            let next = self.prefix_at(time);
            let sample = if time > *previous_time {
                next.since(*previous, time - *previous_time)
            } else if time < 0.0 {
                OccupancyBin::default()
            } else {
                // Adjacent boundaries can coincide beyond f64's integer precision.
                let time = time.floor() as u64;
                let (count, failed) = self.at(time);
                OccupancyBin {
                    average: count as f64,
                    failed,
                    has_points: self.points.binary_search(&time).is_ok(),
                }
            };
            *previous = next;
            *previous_time = time;
            Some(sample)
        })
    }

    fn prefix_at(&self, time: f64) -> ActivityPrefix {
        let integer = time.max(0.0).floor() as u64;
        let index = self.times.partition_point(|&t| t <= integer).checked_sub(1);
        let mut prefix = ActivityPrefix {
            points: self.points.partition_point(|&t| before(t, time)),
            failed_points: self.error_points.partition_point(|&t| before(t, time)),
            ..Default::default()
        };
        if let Some(i) = index {
            prefix.duration = Integral::at(time, self.times[i], self.counts[i], self.integrals[i]);
            if let Some(&count) = self.failed_counts.get(i) {
                prefix.failed_duration =
                    Integral::at(time, self.times[i], count, self.failed_integrals[i]);
            }
        }
        prefix
    }

    fn integral_at(&self, time: f64) -> Integral {
        let integer = time.max(0.0).floor() as u64;
        let n = self.times.partition_point(|&t| t <= integer);
        n.checked_sub(1).map_or_else(Integral::default, |i| {
            Integral::at(time, self.times[i], self.counts[i], self.integrals[i])
        })
    }

    pub(crate) fn bytes(&self) -> usize {
        self.times.capacity() * size_of::<u64>()
            + self.counts.capacity() * size_of::<usize>()
            + self.integrals.capacity() * size_of::<u128>()
            + self.peaks.capacity() * size_of::<(usize, bool)>()
            + self.error_points.capacity() * size_of::<u64>()
            + self.points.capacity() * size_of::<u64>()
            + self.failed_counts.capacity() * size_of::<usize>()
            + self.failed_integrals.capacity() * size_of::<u128>()
    }
}

fn combine(a: (usize, bool), b: (usize, bool)) -> (usize, bool) {
    (a.0.max(b.0), a.1 || b.1)
}

fn at_or_before(time: u64, boundary: f64) -> bool {
    boundary >= 0.0 && (boundary >= u64::MAX as f64 || time <= boundary as u64)
}

fn before(time: u64, boundary: f64) -> bool {
    boundary > 0.0 && (boundary >= u64::MAX as f64 || time < boundary.ceil() as u64)
}
