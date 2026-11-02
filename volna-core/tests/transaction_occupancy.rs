//! Concurrency and cursor paging are checked against raw lifetimes, including points and gaps.

use std::{collections::HashSet, sync::Arc};
use volna_core::data::loaded_tracks::LoadedGenerator;
use volna_core::data::transaction_index::TransactionIndex;
use volna_trace::data::transactions::*;

fn record(id: u64, begin: u64, end: u64) -> Transaction {
    Transaction {
        id: TransactionRef(id),
        generator: TrackRef(1),
        begin,
        end,
        status: if id.is_multiple_of(7) {
            Status::Error
        } else {
            Status::Ok
        },
        kind: SpanKind::Unspecified,
        parent: None,
        attributes: vec![],
        events: vec![],
        stages: vec![],
    }
}

fn loaded(records: Vec<Transaction>) -> TransactionIndex {
    TransactionIndex::build(
        Arc::new(LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap()),
        None,
    )
    .unwrap()
}

#[test]
fn binned_activity_matches_lifetimes_errors_and_points_at_fractional_boundaries() {
    let index = loaded(
        (0..501)
            .map(|id| {
                let begin = (id * 37) % 600;
                record(id, begin, begin + (id * 17) % 93)
            })
            .collect(),
    );
    let records = index.source().transactions();
    for (start, end) in [
        (-10.5f64, 750.0f64),
        (100.25, 101.75),
        (0.0, 1.0),
        (21.0, 22.0),
        (600.0, 693.0),
        (740.0, 750.0),
    ] {
        for count in [1, 7, 512] {
            let mut a = start;
            let samples: Vec<_> = index.occupancy.bins(start, end, count).collect();
            assert_eq!(samples.len(), count);
            for skip in [0, count / 2, count - 1] {
                assert_eq!(
                    index.occupancy.bins(start, end, count).nth(skip),
                    Some(samples[skip])
                );
                assert_eq!(
                    index
                        .occupancy
                        .bins(start, end, count)
                        .skip(skip)
                        .collect::<Vec<_>>(),
                    samples[skip..]
                );
            }
            for (i, sample) in samples.into_iter().enumerate() {
                let b = if i + 1 == count {
                    end
                } else {
                    start + (i + 1) as f64 * (end - start) / count as f64
                };
                let average = records
                    .iter()
                    .map(|tx| (b.min(tx.end as f64) - a.max(tx.begin as f64)).max(0.0))
                    .sum::<f64>()
                    / (b - a);
                let points = records
                    .iter()
                    .any(|tx| tx.begin == tx.end && tx.begin as f64 >= a && (tx.begin as f64) < b);
                let failed = records.iter().any(|tx| {
                    tx.status == Status::Error
                        && (tx.begin as f64) < b
                        && (tx.end as f64 > a || tx.begin == tx.end && tx.begin as f64 >= a)
                });
                assert!(
                    (sample.average - average).abs() < 1e-9,
                    "{a}..{b}: {sample:?}, {average}"
                );
                assert_eq!((sample.failed, sample.has_points), (failed, points));
                a = b;
            }
        }
    }
    for (start, end, count) in [
        (0.0, 100.0, 0),
        (1.0, 1.0, 100),
        (10.0, 0.0, 100),
        (f64::NAN, 10.0, 100),
        (0.0, f64::INFINITY, 100),
    ] {
        assert_eq!(index.occupancy.bins(start, end, count).count(), 0);
    }
}

#[test]
fn binned_activity_preserves_short_late_spans_and_handles_coincident_boundaries() {
    let base = 1u64 << 60;
    let index = loaded(vec![
        record(1, 0, base + 512),
        record(2, 0, base + 512),
        record(7, base + 511, base + 512),
        record(14, base + 511, base + 511),
    ]);
    let sample = index
        .occupancy
        .bins(base as f64, (base + 512) as f64, 1)
        .next()
        .unwrap();
    assert_eq!(sample.average, 2.0 + 1.0 / 512.0);
    assert!(sample.failed && sample.has_points);
    let after = index
        .occupancy
        .bins((base + 512) as f64, (base + 1024) as f64, 1)
        .next()
        .unwrap();
    assert_eq!(after, Default::default());

    let constant = loaded((1..=3).map(|id| record(id, base, base + 4096)).collect());
    let samples: Vec<_> = constant
        .occupancy
        .bins(base as f64, (base + 512) as f64, 1000)
        .collect();
    assert_eq!(samples.len(), 1000);
    assert!(
        samples
            .iter()
            .all(|sample| sample.average == 3.0 && !sample.failed && !sample.has_points)
    );
}

#[test]
fn row_activity_preserves_gaps_and_point_errors_at_boundaries() {
    let index = loaded(vec![
        record(7, 0, 10),
        record(8, 11, 20),
        record(14, 21, 21),
    ]);
    let row = index.row(0).unwrap();
    for (start, end, expected) in [
        (9.0, 10.0, (true, true)),
        (10.0, 11.0, (false, false)),
        (11.0, 12.0, (true, false)),
        (20.0, 21.0, (false, false)),
        (21.0, 22.0, (true, true)),
        (22.0, 23.0, (false, false)),
    ] {
        assert_eq!(row.activity(start, end), expected, "{start}..{end}");
    }
}

#[test]
fn late_summary_boundaries_keep_short_peaks_and_point_errors() {
    let base = 1u64 << 60;
    let generator = loaded(vec![
        record(1, base + 1, base + 257),
        record(2, base + 511, base + 512),
        record(3, base + 511, base + 512),
        record(7, base + 511, base + 511),
    ]);
    let summary = &generator.occupancy;
    let sample = summary.sample(base as f64, (base + 512) as f64);
    assert_eq!(sample.average, 258.0 / 512.0);
    assert_eq!(sample.peak, 2);
    assert!(sample.failed);
    assert_eq!(
        summary.sample((base + 512) as f64, (base + 1024) as f64),
        Default::default()
    );
}

#[test]
fn summaries_match_raw_integrals_peaks_errors_and_half_open_edges() {
    let generator = loaded(
        (0..501)
            .map(|id| {
                let begin = (id * 37) % 600;
                record(id, begin, begin + (id * 17) % 93)
            })
            .collect(),
    );
    let records = generator.source().transactions();
    let summary = &generator.occupancy;
    for time in 0..750 {
        let active: Vec<_> = records
            .iter()
            .filter(|tx| tx.begin <= time && time < tx.end)
            .collect();
        let (count, failed) = summary.at(time);
        assert_eq!(count, active.len());
        assert_eq!(
            failed,
            records.iter().any(|tx| tx.status == Status::Error
                && tx.begin <= time
                && (time < tx.end || time == tx.begin && tx.begin == tx.end))
        );
    }
    for (start, end) in [
        (-10.5, 750.0),
        (100.25, 101.75),
        (0.0, 1.0),
        (600.0, 693.0),
        (740.0, 750.0),
        (11.3, 487.7),
    ] {
        let actual = summary.sample(start, end);
        let area: f64 = records
            .iter()
            .map(|tx| (end.min(tx.end as f64) - start.max(tx.begin as f64)).max(0.0))
            .sum();
        assert!((actual.average - area / (end - start)).abs() < 1e-9);
        let peak = std::iter::once(start)
            .chain(
                records
                    .iter()
                    .map(|tx| tx.begin as f64)
                    .filter(|&time| time >= start && time < end),
            )
            .map(|time| {
                records
                    .iter()
                    .filter(|tx| tx.begin as f64 <= time && time < tx.end as f64)
                    .count()
            })
            .max()
            .unwrap_or(0);
        assert_eq!(actual.peak, peak);
        assert_eq!(
            actual.failed,
            records.iter().any(|tx| tx.status == Status::Error
                && (tx.begin as f64) < end
                && (tx.end as f64 > start || tx.begin == tx.end && tx.begin as f64 >= start))
        );
    }
    let points = loaded(vec![record(7, 5, 5)]);
    let sample = points.occupancy.sample(4.0, 6.0);
    assert_eq!((sample.average, sample.peak, sample.failed), (0.0, 0, true));
    let late = loaded(vec![
        record(1, u64::MAX - 4095, u64::MAX - 2047),
        record(2, u64::MAX - 3071, u64::MAX),
    ]);
    assert_eq!(late.occupancy.at(u64::MAX - 2500).0, 2);
    assert_eq!(late.occupancy.at(u64::MAX).0, 0);
}

#[test]
fn paged_cursor_order_rank_and_random_access_match_all_raw_overlaps() {
    let mut records: Vec<_> = (0..1800)
        .map(|id| {
            let begin = (id * 37) % 1200;
            record(id, begin, begin + (id * 29) % 200)
        })
        .collect();
    records.extend((1800..2100).map(|id| record(id, 0, 1500)));
    let generator = loaded(records);
    let records = generator.source().transactions();
    let index = &generator;
    for time in (0..1600).step_by(17) {
        let expected: HashSet<_> = records
            .iter()
            .enumerate()
            .filter(|(_, tx)| {
                tx.begin <= time && (time < tx.end || tx.begin == tx.end && tx.begin == time)
            })
            .map(|(i, _)| i)
            .collect();
        let count = index.active_count(time);
        assert_eq!(count, expected.len());
        let actual: Vec<_> = (0..count)
            .step_by(31)
            .flat_map(|start| index.active_page(time, start..(start + 31).min(count)))
            .collect();
        assert_eq!(actual.iter().copied().collect::<HashSet<_>>(), expected);
        assert_eq!(actual.len(), expected.len());
        for (rank, &ordinal) in actual.iter().enumerate() {
            assert_eq!(index.active_rank(time, ordinal), Some(rank));
        }
        assert_eq!(
            index.active_page(time, 137..144),
            actual.iter().skip(137).take(7).copied().collect::<Vec<_>>()
        );
        assert!(index.active_page(time, count..count + 32).is_empty());
    }
}

#[test]
fn placement_exceeds_16_bit_depth_without_overlapping_sub_rows() {
    let generator = loaded((0..65_540).map(|id| record(id, 1, 10)).collect());
    assert_eq!(generator.source().depth(), 65_540);
    let sub_rows: HashSet<_> = (0..65_540).map(|i| generator.source().sub_row(i)).collect();
    assert_eq!(sub_rows.len(), 65_540);
    let index = &generator;
    assert_eq!(index.active_count(5), 65_540);
    assert_eq!(index.active_page(5, 0..65_540).len(), 128);
    assert_eq!(index.active_page(5, 65_530..65_540).len(), 10);
}

#[test]
fn point_markers_at_the_last_tick_keep_distinct_sub_rows() {
    let generator = loaded(vec![
        record(0, 0, 1),
        record(1, u64::MAX, u64::MAX),
        record(2, u64::MAX, u64::MAX),
    ]);
    assert_ne!(generator.source().sub_row(1), generator.source().sub_row(2));
    let index = &generator;
    assert_eq!(index.active_count(u64::MAX), 2);
    assert_eq!(index.occupancy.at(u64::MAX).0, 0);
}

#[test]
fn admission_tracks_allocated_storage_and_releases_partial_builds() {
    use volna_trace::remote::memory::MemoryBudget;
    let source = Arc::new(
        LoadedGenerator::new(
            TrackRef(1),
            (0..600).map(|id| record(id, id, id + 80)).collect(),
            Default::default(),
            vec![],
        )
        .unwrap(),
    );
    for limit in [1, 512, 4096, 32_768, 1 << 20] {
        let budget = MemoryBudget::new(limit);
        match TransactionIndex::build(source.clone(), Some(&budget)) {
            Ok(index) => {
                assert_eq!(budget.used(), index.resident_bytes());
                drop(index);
            }
            Err(_) => assert!(limit < 1 << 20),
        }
        assert_eq!(budget.used(), 0);
    }
    let budget = MemoryBudget::new(1 << 20);
    budget.set_object_limit(1);
    assert!(TransactionIndex::build(source, Some(&budget)).is_err());
    assert_eq!(budget.used(), 0);
}

#[test]
fn dense_indexes_fit_the_actual_object_budget_without_a_blanket_record_estimate() {
    use volna_trace::remote::memory::MemoryBudget;
    let source = Arc::new(
        LoadedGenerator::new(
            TrackRef(1),
            (0..400_000).map(|id| record(id, 0, 100)).collect(),
            Default::default(),
            vec![],
        )
        .unwrap(),
    );
    let budget = MemoryBudget::new(256 << 20);
    budget.set_object_limit(96 << 20);
    let index = TransactionIndex::build(source, Some(&budget)).unwrap();
    assert_eq!(index.active_count(50), 400_000);
    assert_eq!(budget.used(), index.resident_bytes());
    assert!(budget.used() < budget.object_limit());
    drop(index);
    assert_eq!(budget.used(), 0);
}
