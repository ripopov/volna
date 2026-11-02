//! Exact concurrency integrals and a peak hierarchy over transaction boundaries.

use crate::data::admission::Storage;
use volna_trace::data::transactions::{Status, Transaction};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OccupancySample {
    pub average: f64,
    pub peak: usize,
    pub failed: bool,
}

#[derive(Debug)]
pub struct Occupancy {
    times: Vec<u64>,
    counts: Vec<usize>,
    integrals: Vec<u128>,
    peaks: Vec<(usize, bool)>,
    leaves: usize,
    error_points: Vec<u64>,
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
        for tx in records {
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
        let (mut count, mut failed, mut integral, mut previous) = (0i64, 0i64, 0u128, 0);
        let mut i = 0;
        while i < events.len() {
            let time = events[i].0;
            integral += u128::from(time - previous) * count as u128;
            while i < events.len() && events[i].0 == time {
                count += events[i].1;
                failed += events[i].2;
                i += 1;
            }
            times.push(time);
            counts.push(count as usize);
            integrals.push(integral);
            failures.push(failed > 0);
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
        let (a, af) = self.integral_at(start);
        let (b, bf) = self.integral_at(end);
        OccupancySample {
            average: ((b - a) as f64 + bf - af) / (end - start),
            peak: peak.0,
            failed: peak.1
                || self
                    .error_points
                    .get(self.error_points.partition_point(|&t| before(t, start)))
                    .is_some_and(|&t| before(t, end)),
        }
    }

    fn integral_at(&self, time: f64) -> (u128, f64) {
        let integer = time.max(0.0).floor() as u64;
        let n = self.times.partition_point(|&t| t <= integer);
        n.checked_sub(1).map_or((0, 0.0), |i| {
            (
                self.integrals[i] + u128::from(integer - self.times[i]) * self.counts[i] as u128,
                (time.max(0.0) - integer as f64) * self.counts[i] as f64,
            )
        })
    }

    pub(crate) fn bytes(&self) -> usize {
        self.times.capacity() * size_of::<u64>()
            + self.counts.capacity() * size_of::<usize>()
            + self.integrals.capacity() * size_of::<u128>()
            + self.peaks.capacity() * size_of::<(usize, bool)>()
            + self.error_points.capacity() * size_of::<u64>()
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
