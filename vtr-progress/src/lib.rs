//! Conversion work counters and the progress messages shared by import hosts.

use std::io::Read;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

pub const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Preparing,
    Reading,
    Writing,
    Finalizing,
    Complete,
    Failed,
}

impl Phase {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing input",
            Self::Reading => "Reading input",
            Self::Writing => "Writing VTR",
            Self::Finalizing => "Finalizing VTR",
            Self::Complete => "Complete",
            Self::Failed => "Conversion failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub phase: Phase,
    pub completed: u64,
    pub total: Option<u64>,
    pub input_bytes: u64,
    pub output_bytes: u64,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: VERSION,
            phase: Phase::Preparing,
            completed: 0,
            total: None,
            input_bytes: 0,
            output_bytes: 0,
        }
    }
}

impl Snapshot {
    #[must_use]
    pub fn fraction(&self) -> Option<f32> {
        if self.phase == Phase::Complete {
            return Some(1.0);
        }
        self.total
            .filter(|&n| n > 0)
            .map(|n| (self.completed as f64 / n as f64).clamp(0.0, 1.0) as f32)
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        self.version == VERSION
            && self
                .total
                .is_none_or(|total| total > 0 && self.completed <= total)
    }
}

#[derive(Clone, Default)]
pub struct Counter(Arc<AtomicU64>);

impl Counter {
    pub fn set(&self, value: u64) {
        self.0.fetch_max(value, Ordering::Relaxed);
    }

    pub fn advance(&self, amount: u64) {
        self.0.fetch_add(amount, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Work {
    phase: Phase,
    counter: Counter,
    total: Option<u64>,
}

#[derive(Clone, Default)]
pub struct Reporter(Arc<Mutex<Work>>);

impl Reporter {
    #[expect(
        clippy::must_use_candidate,
        reason = "Starting an indeterminate phase does not need a work counter."
    )]
    pub fn begin(&self, phase: Phase, total: Option<u64>) -> Counter {
        let counter = Counter::default();
        *self.0.lock().expect("progress lock") = Work {
            phase,
            counter: counter.clone(),
            total: total.filter(|&n| n > 0),
        };
        counter
    }

    #[must_use]
    pub fn snapshot(&self, input_bytes: u64, output_bytes: u64) -> Snapshot {
        let work = self.0.lock().expect("progress lock");
        let completed = work.counter.0.load(Ordering::Relaxed);
        Snapshot {
            version: VERSION,
            phase: work.phase,
            completed: work.total.map_or(completed, |total| completed.min(total)),
            total: work.total,
            input_bytes,
            output_bytes,
        }
    }
}

pub struct CountingReader<R> {
    inner: R,
    counter: Counter,
}

impl<R> CountingReader<R> {
    pub fn new(inner: R, counter: Counter) -> Self {
        Self { inner, counter }
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(bytes)?;
        self.counter.advance(read as u64);
        Ok(read)
    }
}

#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    let mut size = bytes as f64;
    let mut unit = "B";
    for next in ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"] {
        if size < 1024.0 {
            break;
        }
        size /= 1024.0;
        unit = next;
    }
    if unit == "B" {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_changes_detach_old_counters_and_bound_progress() {
        let reporter = Reporter::default();
        let reading = reporter.begin(Phase::Reading, Some(100));
        reading.set(75);
        reading.set(50);
        assert_eq!(reporter.snapshot(100, 10).completed, 75);
        let writing = reporter.begin(Phase::Writing, Some(20));
        reading.advance(100);
        assert_eq!(reporter.snapshot(100, 10).completed, 0);
        writing.advance(30);
        let snapshot = reporter.snapshot(100, 1000);
        assert_eq!(snapshot.completed, 20);
        assert_eq!(snapshot.fraction(), Some(1.0));
        assert!(snapshot.valid());
        reporter.begin(Phase::Finalizing, None);
        assert_eq!(reporter.snapshot(100, 1000).fraction(), None);
        reporter.begin(Phase::Complete, None);
        assert_eq!(reporter.snapshot(100, 1100).fraction(), Some(1.0));
    }
}
