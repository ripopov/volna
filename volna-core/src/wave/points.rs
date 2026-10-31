//! Point-record summaries and shared geometry for painting and hit testing.

use std::ops::Range;

use crate::data::points::{Counts, PointIndex};
use crate::geometry::{Point, Rect};
use crate::wave::viewport::Viewport;
use volna_trace::data::transactions::Transaction;

#[derive(Clone, Debug)]
pub struct PointGroup {
    pub records: Range<usize>,
    pub first: u64,
    pub last: u64,
    pub counts: Counts,
    pub rect: Rect,
    pub anchor: f32,
    pub end_anchor: f32,
}

impl PointGroup {
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        self.rect.contains(point)
    }
}

#[must_use]
pub fn layout(
    records: &[Transaction],
    index: &PointIndex,
    viewport: Viewport,
    area: Rect,
    zoom: f32,
) -> Vec<PointGroup> {
    if area.width() <= 0.0 || area.height() <= 0.0 || records.is_empty() {
        return Vec::new();
    }
    let width = f64::from(area.width());
    let x = |time| viewport.x_of_tick(time, width) as f32 + area.left();
    let card_w = (172.0 * zoom).min(area.width());
    let gap = 6.0 * zoom;
    let lane_h = 52.0 * zoom;
    let anchor_gap = gap.min(area.height() / 4.0);
    let lanes = (((area.height() - anchor_gap + gap) / lane_h).floor() as usize).clamp(1, 3);
    let card_h = (lane_h - gap).min(area.height() - anchor_gap);

    // Global time buckets keep membership and lanes independent of the visible records.
    // Same-lane anchors are separated by at least lanes - 1 buckets.
    let bucket_px = f64::from(card_w + gap) / lanes.saturating_sub(1).max(1) as f64;
    let bucket_ticks = bucket_ticks(viewport.width() * bucket_px / width);
    let bucket_of = |time: u64| u128::from(time) / bucket_ticks;
    let start_bucket = viewport
        .tick_at(-f64::from(card_w), width)
        .map_or(0, bucket_of);
    let Some(end) = viewport.tick_at(width, width) else {
        return Vec::new();
    };
    let end_bucket = bucket_of(end);
    let mut ordinal = records.partition_point(|r| bucket_of(r.begin) < start_bucket);
    let mut groups = Vec::new();
    while ordinal < records.len() {
        let begin = ordinal;
        let time = records[begin].begin;
        let bucket = bucket_of(time);
        if bucket > end_bucket {
            break;
        }
        ordinal += records[ordinal..].partition_point(|r| bucket_of(r.begin) == bucket);
        let final_time = records[ordinal - 1].begin;
        let left = if lanes == 1 {
            // A single lane needs fixed label slots to avoid collisions across bucket edges.
            x((bucket * bucket_ticks) as u64)
        } else {
            x(time)
        };
        let lane = (bucket % lanes as u128) as usize;
        let rect = Rect::from_xywh(
            left,
            area.top() + anchor_gap + lane as f32 * lane_h,
            card_w,
            card_h,
        );
        let anchor = x(time);
        let end_anchor = x(final_time);
        if rect.left() <= area.right() && (rect.right() >= area.left() || end_anchor >= area.left())
        {
            groups.push(PointGroup {
                records: begin..ordinal,
                first: time,
                last: final_time,
                counts: index.counts(begin..ordinal),
                rect,
                anchor,
                end_anchor,
            });
        }
    }
    groups
}

fn bucket_ticks(ticks: f64) -> u128 {
    // Panning can perturb the viewport width by a few floating-point ulps.
    let ticks = (ticks * (1.0 - 32.0 * f64::EPSILON)).ceil().max(1.0) as u128;
    ticks
        .checked_next_power_of_two()
        .unwrap_or(1_u128 << 64)
        .min(1_u128 << 64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::loaded_tracks::LoadedGenerator;
    use crate::data::points::{PointKind, SEVERITIES, severity};
    use volna_trace::data::transactions::AttributeValue;
    use volna_trace::data::transactions::{
        SpanKind, Status, TrackRef, TransactionAttribute, TransactionRef,
    };

    fn record(id: u64, time: u64, kind: &str) -> Transaction {
        Transaction {
            id: TransactionRef(id),
            generator: TrackRef(1),
            begin: time,
            end: time,
            kind: SpanKind::Unspecified,
            status: Status::Ok,
            parent: None,
            attributes: vec![
                TransactionAttribute {
                    key: "record.kind".into(),
                    value: AttributeValue::Text(kind.into()),
                },
                TransactionAttribute {
                    key: "log.severity".into(),
                    value: AttributeValue::Enum {
                        value: i128::from(id % 7),
                        name: String::new(),
                    },
                },
            ],
            events: vec![],
            stages: vec![],
        }
    }

    #[test]
    fn zoom_and_pan_preserve_occurrences_counts_and_nonoverlapping_hits() {
        let records = (0..2000)
            .map(|i| record(i, if i < 1000 { 52 } else { (i - 1000) * 3 }, "log"))
            .collect();
        let loaded =
            LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap();
        let records = loaded.transactions();
        let index = loaded.points().unwrap();
        assert_eq!(loaded.depth(), 1);
        for width in [80.0, 360.0, 1000.0] {
            for (start, end) in [(0.0, 100.0), (0.0, 3000.0), (50.0, 60.0), (100.0, 1000.0)] {
                let groups = layout(
                    records,
                    index,
                    Viewport { start, end },
                    Rect::from_xywh(0.0, 0.0, width, 160.0),
                    1.0,
                );
                let actual: Vec<_> = groups.iter().flat_map(|g| g.records.clone()).collect();
                let expected: Vec<_> = records
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.begin as f64 >= start && r.begin as f64 <= end)
                    .map(|(i, _)| i)
                    .collect();
                assert_eq!(
                    actual
                        .iter()
                        .copied()
                        .filter(|i| expected.binary_search(i).is_ok())
                        .collect::<Vec<_>>(),
                    expected
                );
                for (i, a) in groups.iter().enumerate() {
                    let mut counts = [0; SEVERITIES];
                    for record in &records[a.records.clone()] {
                        counts[severity(record)] += 1;
                    }
                    assert_eq!(a.counts, counts);
                    assert!(a.contains(crate::geometry::point(
                        a.rect.left() + 1.0,
                        a.rect.top() + 1.0
                    )));
                    for b in &groups[i + 1..] {
                        assert!(
                            a.rect.right() <= b.rect.left()
                                || b.rect.right() <= a.rect.left()
                                || a.rect.bottom() <= b.rect.top()
                                || b.rect.bottom() <= a.rect.top()
                        );
                    }
                }
                let exact: Vec<_> = groups
                    .iter()
                    .filter(|g| g.first <= 52 && g.last >= 52)
                    .collect();
                if start <= 52.0 && end >= 52.0 {
                    assert_eq!(exact.len(), 1);
                }
            }
        }
    }

    #[test]
    fn panning_only_translates_groups_including_at_viewport_edges() {
        let records = (0..1000).map(|i| record(i, i * 3, "log")).collect();
        let loaded =
            LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap();
        let records = loaded.transactions();
        let index = loaded.points().unwrap();
        for width in [80.0, 360.0, 1000.0] {
            for height in [24.0, 104.0, 160.0] {
                for span in [100.0, 500.0, 16.0 * f64::from(width) / 89.0] {
                    let area = Rect::from_xywh(17.0, 31.0, width, height);
                    let before = layout(
                        records,
                        index,
                        Viewport {
                            start: 200.0,
                            end: 200.0 + span,
                        },
                        area,
                        1.0,
                    );
                    for pan in [-50.0_f64, -0.125, 0.125, 1.0, 3.0, 17.0, 50.0]
                        .into_iter()
                        .filter(|pan| pan.abs() < span / 2.0)
                    {
                        let after = layout(
                            records,
                            index,
                            Viewport {
                                start: 200.0 + pan,
                                end: 200.0 + pan + span,
                            },
                            area,
                            1.0,
                        );
                        let dx = (-pan / span * f64::from(width)) as f32;
                        for (i, a) in after.iter().enumerate() {
                            for b in &after[i + 1..] {
                                assert!(
                                    a.rect.top() != b.rect.top()
                                        || a.rect.right() <= b.rect.left()
                                        || b.rect.right() <= a.rect.left()
                                );
                            }
                        }
                        let mut compared = 0;
                        for a in &before {
                            for b in &after {
                                if a.records.start < b.records.end
                                    && b.records.start < a.records.end
                                {
                                    assert_eq!(
                                        a.records, b.records,
                                        "membership changed at width={width}, height={height}, span={span}, pan={pan}"
                                    );
                                    assert_eq!(a.counts, b.counts);
                                    assert_eq!(
                                        a.rect.top(),
                                        b.rect.top(),
                                        "lane changed during pan"
                                    );
                                    assert!(
                                        (b.rect.left() - a.rect.left() - dx).abs() < 0.001,
                                        "badge moved independently of time"
                                    );
                                    assert!((b.anchor - a.anchor - dx).abs() < 0.001);
                                    assert!((b.end_anchor - a.end_anchor - dx).abs() < 0.001);
                                    compared += 1;
                                }
                            }
                        }
                        assert!(compared > 0);
                    }
                }
            }
        }
    }

    #[test]
    fn clipped_groups_keep_their_offscreen_members_and_cover_the_last_tick() {
        let loaded = LoadedGenerator::new(
            TrackRef(1),
            vec![
                record(0, 0, "log"),
                record(1, 49, "log"),
                record(2, 62, "log"),
                record(3, u64::MAX, "log"),
            ],
            Default::default(),
            vec![],
        )
        .unwrap();
        let area = Rect::from_xywh(10.0, 0.0, 1000.0, 160.0);
        let groups = layout(
            loaded.transactions(),
            loaded.points().unwrap(),
            Viewport {
                start: 50.0,
                end: 150.0,
            },
            area,
            1.0,
        );
        let group = groups.iter().find(|g| g.records.contains(&2)).unwrap();
        assert_eq!(group.records, 1..3);
        assert_eq!((group.first, group.last), (49, 62));
        assert!(group.anchor < area.left());
        assert!(group.rect.left() < area.left());
        let groups = layout(
            loaded.transactions(),
            loaded.points().unwrap(),
            Viewport {
                start: (u64::MAX - 8191) as f64,
                end: u64::MAX as f64,
            },
            area,
            1.0,
        );
        assert_eq!(groups.last().unwrap().records, 3..4);
        assert_eq!(groups.last().unwrap().last, u64::MAX);
    }

    #[test]
    fn adjacent_large_timestamps_and_event_labels_remain_exact() {
        let base = (1_u64 << 54) + 32;
        let mut a = record(1, base, "event");
        a.attributes.push(TransactionAttribute {
            key: "vtr.label".into(),
            value: AttributeValue::Text("interrupt".into()),
        });
        let loaded = LoadedGenerator::new(
            TrackRef(1),
            vec![a, record(2, base + 1, "event")],
            Default::default(),
            vec![],
        )
        .unwrap();
        let index = loaded.points().unwrap();
        assert_eq!(index.kind, PointKind::Event);
        assert_eq!(
            index.kind.label(&loaded.transactions()[0]),
            Some(&AttributeValue::Text("interrupt".into()))
        );
        let groups = layout(
            loaded.transactions(),
            index,
            Viewport {
                start: base as f64,
                end: (base + 4) as f64,
            },
            Rect::from_xywh(0.0, 0.0, 1000.0, 160.0),
            1.0,
        );
        assert_eq!(groups.len(), 2);
        assert_eq!((groups[0].first, groups[1].first), (base, base + 1));
    }
}
