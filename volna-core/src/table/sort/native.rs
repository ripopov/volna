//! In-memory row permutations admitted before allocation and retained with their reservation.

use super::{Sort, Source};
use anyhow::{Context, Result, ensure};
use std::{
    cmp::Ordering,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        mpsc,
    },
    thread,
};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

pub(in crate::table) fn required_bytes(len: u64) -> Option<u64> {
    len.checked_mul(if len <= u64::from(u32::MAX) + 1 { 4 } else { 8 })
}

enum Indices {
    Narrow(Vec<u32>),
    Wide(Vec<u64>),
}
impl Indices {
    fn build(len: u64, compare: &impl Fn(u64, u64) -> Ordering, stop: &AtomicBool) -> Result<Self> {
        let count = usize::try_from(len)?;
        if len <= u64::from(u32::MAX) + 1 {
            Ok(Self::Narrow(build(
                count,
                |i| i as u32,
                |a, b| compare(u64::from(a), u64::from(b)),
                stop,
            )?))
        } else {
            Ok(Self::Wide(build(count, |i| i as u64, compare, stop)?))
        }
    }
    fn len(&self) -> usize {
        match self {
            Self::Narrow(v) => v.len(),
            Self::Wide(v) => v.len(),
        }
    }
    fn get(&self, rank: u64) -> Option<u64> {
        let rank = usize::try_from(rank).ok()?;
        match self {
            Self::Narrow(v) => v.get(rank).map(|&v| u64::from(v)),
            Self::Wide(v) => v.get(rank).copied(),
        }
    }
}

pub(in crate::table) struct Order {
    rows: Indices,
    pub sort: Sort,
    source: Source,
    _reservation: Reservation,
}
impl Order {
    pub fn source_row(&self, rank: u64) -> Option<u64> {
        self.rows.get(rank)
    }
    pub fn rank(&self, row: u64) -> Option<u64> {
        let (mut low, mut high) = (0, self.rows.len() as u64);
        while low < high {
            let mid = low + (high - low) / 2;
            match self.source.compare(self.sort, self.source_row(mid)?, row) {
                Ordering::Less => low = mid + 1,
                Ordering::Greater => high = mid,
                Ordering::Equal => return Some(mid),
            }
        }
        None
    }
}

pub(in crate::table) struct Job {
    cancel: Arc<AtomicBool>,
    result: Option<mpsc::Receiver<Result<Order>>>,
}
impl Job {
    pub fn start(source: Source, len: u64, sort: Sort, budget: MemoryBudget) -> Result<Self> {
        let reservation = budget.reserve_object(
            "table sort index",
            required_bytes(len).context("Sort index is too large")?,
        )?;
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (send, result) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("table-sort".into())
            .spawn(move || {
                let result = (|| {
                    let rows = Indices::build(len, &|a, b| source.compare(sort, a, b), &stop)?;
                    Ok(Order {
                        rows,
                        _reservation: reservation,
                        sort,
                        source,
                    })
                })();
                if !stop.load(AtomicOrdering::Relaxed) {
                    let _ = send.send(result);
                }
            })?;
        Ok(Self {
            cancel,
            result: Some(result),
        })
    }
    pub fn cancel(&self) {
        self.cancel.store(true, AtomicOrdering::Relaxed);
    }
    pub fn poll(&mut self) -> Option<Result<Order>> {
        let result = match self.result.as_ref()?.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(anyhow::anyhow!("Sort cancelled or worker stopped")))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        };
        if result.is_some() {
            self.result = None;
        }
        result
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
        // A completion may already be queued when its panel closes.
        if let Some(result) = self.result.take() {
            let _ = thread::Builder::new()
                .name("table-sort-retire".into())
                .spawn(move || drop(result));
        }
    }
}

pub(in crate::table) fn retire(order: Order) {
    // Releasing a large index and its source must not run in GPUI's frame callback.
    let _ = thread::Builder::new()
        .name("table-sort-retire".into())
        .spawn(move || drop(order));
}

fn checkpoint(stop: &AtomicBool) -> Result<()> {
    ensure!(!stop.load(AtomicOrdering::Relaxed), "Sort cancelled");
    Ok(())
}

fn build<T: Copy>(
    len: usize,
    ordinal: impl Fn(usize) -> T,
    compare: impl Fn(T, T) -> Ordering,
    stop: &AtomicBool,
) -> Result<Vec<T>> {
    checkpoint(stop)?;
    let mut rows = Vec::new();
    rows.try_reserve_exact(len)
        .context("Cannot allocate table sort index")?;
    for start in (0..len).step_by(4096) {
        checkpoint(stop)?;
        rows.extend((start..start.saturating_add(4096).min(len)).map(&ordinal));
    }
    sort_run(&mut rows, &compare, stop)?;
    checkpoint(stop)?;
    Ok(rows)
}

fn sort_run<T: Copy>(
    rows: &mut [T],
    compare: &impl Fn(T, T) -> Ordering,
    stop: &AtomicBool,
) -> Result<()> {
    let greater = |a: &T, b: &T| compare(*a, *b) == Ordering::Greater;
    for i in (0..rows.len() / 2).rev() {
        if i.is_multiple_of(256) {
            checkpoint(stop)?;
        }
        sift(rows, i, &greater);
    }
    for end in (1..rows.len()).rev() {
        if end.is_multiple_of(256) {
            checkpoint(stop)?;
        }
        rows.swap(0, end);
        sift(&mut rows[..end], 0, &greater);
    }
    Ok(())
}
fn sift<T>(heap: &mut [T], mut i: usize, less: &impl Fn(&T, &T) -> bool) {
    loop {
        let mut child = i * 2 + 1;
        if child >= heap.len() {
            return;
        }
        if child + 1 < heap.len() && less(&heap[child + 1], &heap[child]) {
            child += 1;
        }
        if !less(&heap[child], &heap[i]) {
            return;
        }
        heap.swap(i, child);
        i = child;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn in_memory_order_is_exact_stable_and_cancellable() {
        let stop = AtomicBool::new(false);
        for descending in [false, true] {
            let compare = |a: u64, b: u64| {
                let order = (a.wrapping_mul(997) % 71).cmp(&(b.wrapping_mul(997) % 71));
                (if descending { order.reverse() } else { order }).then(a.cmp(&b))
            };
            let actual = Indices::build(12_347, &compare, &stop).unwrap();
            let mut expected: Vec<_> = (0..12_347).collect();
            expected.sort_by(|&a, &b| compare(a, b));
            assert_eq!(
                (0..12_347)
                    .map(|i| actual.get(i).unwrap())
                    .collect::<Vec<_>>(),
                expected
            );
        }
        stop.store(true, AtomicOrdering::Relaxed);
        assert!(Indices::build(100, &|a, b| a.cmp(&b), &stop).is_err());
        stop.store(false, AtomicOrdering::Relaxed);
        let comparisons = std::cell::Cell::new(0usize);
        let compare = |a: u64, b: u64| {
            comparisons.set(comparisons.get() + 1);
            if comparisons.get() == 1000 {
                stop.store(true, AtomicOrdering::Relaxed);
            }
            b.cmp(&a)
        };
        assert!(Indices::build(100_000, &compare, &stop).is_err());
        assert!(comparisons.get() < 10_000);
        let mut wide = vec![u64::MAX, 1 << 32, 0, (1 << 32) - 1];
        stop.store(false, AtomicOrdering::Relaxed);
        sort_run(&mut wide, &|a, b| a.cmp(&b), &stop).unwrap();
        assert_eq!(wide, vec![0, (1 << 32) - 1, 1 << 32, u64::MAX]);
    }

    #[test]
    #[ignore = "100M-row RAM and throughput validation; run in release mode"]
    fn hundred_million_rows() {
        let stop = AtomicBool::new(false);
        let budget = MemoryBudget::new(400_000_000);
        let reservation = budget
            .reserve(required_bytes(100_000_000).unwrap())
            .unwrap();
        let start = std::time::Instant::now();
        let compare = |a: u64, b: u64| {
            a.wrapping_mul(6364136223846793005)
                .cmp(&b.wrapping_mul(6364136223846793005))
        };
        let rows = Indices::build(100_000_000, &compare, &stop).unwrap();
        assert!(matches!(&rows, Indices::Narrow(v) if v.capacity() == 100_000_000));
        for rank in 0..100_000_000 {
            let row = rows.get(rank).unwrap();
            assert!(row < 100_000_000);
            if rank > 0 {
                assert_eq!(compare(rows.get(rank - 1).unwrap(), row), Ordering::Less);
            }
        }
        eprintln!(
            "100M ordinals sorted and verified in {:?}; index {} bytes",
            start.elapsed(),
            budget.used()
        );
        drop(rows);
        drop(reservation);
        assert_eq!(budget.used(), 0);
    }
}
