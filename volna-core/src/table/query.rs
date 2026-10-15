//! Cancellable background filtering and ordering with budgeted immutable row indexes.

use super::sort::{Sort, Source};
use crate::table::filter::{Column, ColumnFilter, Program, index::Matches};
use anyhow::{Context, Result, ensure};
use std::{
    cmp::Ordering,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

pub(in crate::table) fn required_bytes(len: u64) -> Option<u64> {
    index_bytes(len, len)
}

pub(in crate::table) fn index_bytes(source_len: u64, matches: u64) -> Option<u64> {
    matches.checked_mul(if source_len <= u64::from(u32::MAX) + 1 {
        4
    } else {
        8
    })
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
    fn from_rows(
        len: u64,
        source_len: u64,
        rows: impl Iterator<Item = u64>,
        compare: &impl Fn(u64, u64) -> Ordering,
        stop: &AtomicBool,
    ) -> Result<Self> {
        let count = usize::try_from(len)?;
        if source_len <= u64::from(u32::MAX) + 1 {
            let mut values = Vec::new();
            values.try_reserve_exact(count)?;
            for (i, row) in rows.enumerate() {
                if i.is_multiple_of(4096) {
                    checkpoint(stop)?;
                }
                values.push(u32::try_from(row)?);
            }
            sort_run(
                &mut values,
                &|a, b| compare(u64::from(a), u64::from(b)),
                stop,
            )?;
            Ok(Self::Narrow(values))
        } else {
            let mut values = Vec::new();
            values.try_reserve_exact(count)?;
            for (i, row) in rows.enumerate() {
                if i.is_multiple_of(4096) {
                    checkpoint(stop)?;
                }
                values.push(row);
            }
            sort_run(&mut values, compare, stop)?;
            Ok(Self::Wide(values))
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

enum RowIndex {
    Sorted(Indices),
    Filtered(Matches),
}

pub(in crate::table) struct Order {
    rows: RowIndex,
    pub sort: Option<Sort>,
    pub filters: Vec<Option<ColumnFilter>>,
    pub unit: crate::trace::Unit,
    source: Source,
    _reservation: Reservation,
}
impl Order {
    pub fn len(&self) -> u64 {
        match &self.rows {
            RowIndex::Sorted(rows) => rows.len() as u64,
            RowIndex::Filtered(rows) => rows.len(),
        }
    }
    pub fn source_row(&self, rank: u64) -> Option<u64> {
        match &self.rows {
            RowIndex::Sorted(rows) => rows.get(rank),
            RowIndex::Filtered(rows) => rows.source_row(rank),
        }
    }
    pub fn rank(&self, row: u64) -> Option<u64> {
        let Some(sort) = self.sort else {
            return match &self.rows {
                RowIndex::Filtered(rows) => rows.rank(row),
                RowIndex::Sorted(_) => None,
            };
        };
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let mid = low + (high - low) / 2;
            match self.source.compare(sort, self.source_row(mid)?, row) {
                Ordering::Less => low = mid + 1,
                Ordering::Greater => high = mid,
                Ordering::Equal => return Some(mid),
            }
        }
        None
    }
}

pub(in crate::table) struct Query {
    pub sort: Option<Sort>,
    pub filters: Vec<Option<ColumnFilter>>,
    pub columns: Vec<(Column, ColumnFilter)>,
    pub unit: crate::trace::Unit,
}

pub(in crate::table) struct Job {
    cancel: Arc<AtomicBool>,
    result: Option<futures_channel::oneshot::Receiver<Result<Order>>>,
}
impl Job {
    pub fn start(source: Source, len: u64, query: Query, budget: MemoryBudget) -> Result<Self> {
        let filtered = !query.columns.is_empty();
        let reservation = budget.reserve_object(
            "table query index",
            if filtered {
                crate::table::filter::index::required_bytes(len)
            } else {
                required_bytes(len)
            }
            .context("Table index is too large")?,
        )?;
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (send, result) = futures_channel::oneshot::channel();
        spawn(move || {
            let result = (|| {
                checkpoint(&stop)?;
                let mut program = Program::compile(&query.columns, &query.unit, &source)?;
                let mut scratch = String::new();
                let matches = filtered
                    .then(|| {
                        Matches::build(
                            len,
                            |row| program.matches(&source, row, &mut scratch),
                            &stop,
                        )
                    })
                    .transpose()?;
                let (rows, reservation) = if let Some(sort) = query.sort {
                    if let Some(matches) = matches {
                        let sorted_reservation = budget.reserve_object(
                            "table sort index",
                            index_bytes(len, matches.len()).context("Sort index is too large")?,
                        )?;
                        let rows = Indices::from_rows(
                            matches.len(),
                            len,
                            matches.iter(),
                            &|a, b| source.compare(sort, a, b),
                            &stop,
                        )?;
                        drop(matches);
                        drop(reservation);
                        (RowIndex::Sorted(rows), sorted_reservation)
                    } else {
                        (
                            RowIndex::Sorted(Indices::build(
                                len,
                                &|a, b| source.compare(sort, a, b),
                                &stop,
                            )?),
                            reservation,
                        )
                    }
                } else {
                    (
                        RowIndex::Filtered(matches.context("Filter is empty")?),
                        reservation,
                    )
                };
                Ok(Order {
                    rows,
                    _reservation: reservation,
                    sort: query.sort,
                    filters: query.filters,
                    unit: query.unit,
                    source,
                })
            })();
            if !stop.load(AtomicOrdering::Relaxed) {
                drop(send.send(result));
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
        let result = match self.result.as_mut()?.try_recv() {
            Ok(result) => result,
            Err(_) => Some(Err(anyhow::anyhow!(
                "Table query cancelled or worker stopped"
            ))),
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
            drop(spawn(move || drop(result)));
        }
    }
}

pub(in crate::table) fn retire(order: Order) {
    // Releasing a large index and its source must not run in GPUI's frame callback.
    drop(spawn(move || drop(order)));
}

fn checkpoint(stop: &AtomicBool) -> Result<()> {
    ensure!(!stop.load(AtomicOrdering::Relaxed), "Table query cancelled");
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

/// Installs the host's background executor before table queries start.
/// The executor must run CPU work off the UI thread and retain detached jobs.
pub fn set_executor(spawn: impl Fn(Box<dyn FnOnce() + Send>) + Send + Sync + 'static) {
    drop(EXECUTOR.set(Box::new(spawn)));
}

type Spawn = Box<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;
static EXECUTOR: std::sync::OnceLock<Spawn> = std::sync::OnceLock::new();

fn spawn(work: impl FnOnce() + Send + 'static) -> anyhow::Result<()> {
    if let Some(executor) = EXECUTOR.get() {
        executor(Box::new(work));
        return Ok(());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new()
            .name("table-worker".into())
            .spawn(work)?;
        Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    {
        anyhow::bail!("Background executor is unavailable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct BorrowedHistory {
        len: usize,
        caller: std::thread::ThreadId,
    }
    impl volna_trace::data::SignalHistory for BorrowedHistory {
        fn shape(&self) -> volna_trace::data::SignalShape {
            volna_trace::data::SignalShape::Text
        }
        fn len(&self) -> usize {
            self.len
        }
        fn time(&self, row: usize) -> u64 {
            row as u64
        }
        fn index_at(&self, time: u64) -> Option<usize> {
            (time < self.len as u64).then_some(time as usize)
        }
        fn value(&self, _: Option<usize>) -> volna_trace::data::WaveValue {
            panic!("query must borrow the value")
        }
        fn bit(&self, _: Option<usize>) -> volna_trace::data::Bit {
            volna_trace::data::Bit::Other
        }
        fn value_view(&self, row: Option<usize>) -> volna_trace::data::value_view::ValueView<'_> {
            assert_ne!(
                std::thread::current().id(),
                self.caller,
                "predicates must run off the caller thread"
            );
            volna_trace::data::value_view::ValueView::Text(
                if row.unwrap().is_multiple_of(3) {
                    "cache_load_miss"
                } else {
                    "store"
                }
                .into(),
            )
        }
    }

    fn borrowed_query(len: u64, sort: Option<Sort>) -> (Order, MemoryBudget) {
        use crate::table::filter::FilterMode;
        let source = Source::Signals {
            histories: vec![Arc::new(BorrowedHistory {
                len: len as usize,
                caller: std::thread::current().id(),
            })],
            axis: None,
            _reservation: None,
        };
        let text = ColumnFilter {
            mode: FilterMode::Wildcard,
            query: "cache*miss".into(),
            ..Default::default()
        };
        let time = ColumnFilter {
            mode: FilterMode::Expression,
            query: "> 100".into(),
            ..Default::default()
        };
        let budget = MemoryBudget::new(16 * 1024 * 1024);
        let mut job = Job::start(
            source,
            len,
            Query {
                sort,
                filters: vec![Some(time.clone()), Some(text.clone())],
                columns: vec![
                    (Column::Time, time),
                    (
                        Column::Signal {
                            index: 0,
                            name: "value".into(),
                        },
                        text,
                    ),
                ],
                unit: crate::trace::Unit {
                    timescale: -9,
                    name: None,
                },
            },
            budget.clone(),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let order = loop {
            if let Some(result) = job.poll() {
                break result.unwrap();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        (order, budget)
    }

    #[test]
    fn background_predicates_borrow_values_and_sort_only_matches() {
        let (order, budget) = borrowed_query(
            10_000,
            Some(Sort {
                column: 0,
                descending: true,
            }),
        );
        assert_eq!(order.len(), 3300);
        assert_eq!(order.source_row(0), Some(9999));
        assert_eq!(order.source_row(3299), Some(102));
        assert_eq!(order.rank(102), Some(3299));
        assert_eq!(order.rank(101), None);
        assert_eq!(budget.used(), order.len() * 4);
        drop(order);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    #[ignore = "10M-row background query measurement"]
    fn ten_million_rows_background_query() {
        let start = std::time::Instant::now();
        let (order, budget) = borrowed_query(10_000_000, None);
        assert_eq!(order.len(), 3_333_300);
        assert_eq!(order.source_row(0), Some(102));
        assert_eq!(order.source_row(order.len() - 1), Some(9_999_999));
        assert_eq!(
            budget.used(),
            crate::table::filter::index::required_bytes(10_000_000).unwrap()
        );
        eprintln!(
            "10M background wildcard + numeric query: {:?}, {} index bytes",
            start.elapsed(),
            budget.used()
        );
    }

    #[test]
    fn sparse_matches_above_u32_keep_wide_source_ordinals() {
        let last = u64::from(u32::MAX) + 1;
        let rows = Indices::from_rows(
            2,
            last + 1,
            [last, 7].into_iter(),
            &|a, b| a.cmp(&b),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(rows.get(0), Some(7));
        assert_eq!(rows.get(1), Some(last));
    }

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
            a.wrapping_mul(6_364_136_223_846_793_005)
                .cmp(&b.wrapping_mul(6_364_136_223_846_793_005))
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
