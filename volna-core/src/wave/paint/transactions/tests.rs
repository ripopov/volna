use super::paint;
use crate::data::transaction_index::TransactionIndex;
use crate::{
    color::Color,
    data::loaded_tracks::LoadedGenerator,
    document::Document,
    geometry::{Point, Rect, point},
    scene::{MonoMeasure, Prim, Scene, TextCache},
    theme::{Theme, builtin},
    trace::TraceId,
    wave::{
        RowHeight, Viewport,
        lane::{DetailCache, LaneGeometry, RenderPlan, TxLane},
        overlay::TextPainter,
    },
};
use std::sync::Arc;
use volna_trace::data::transactions::{
    AttributeValue, SpanKind, Status, TrackRef, Transaction, TransactionAttribute, TransactionRef,
    TransactionStage,
};

#[test]
#[ignore = "manual rendering latency and preparation memory comparison"]
fn rendering_profile() {
    use std::time::Instant;
    for (name, count, overlap) in [("short", 200_000, 1), ("deep", 131_072, 1024)] {
        let records = (0..count)
            .map(|i| {
                let begin = (i / overlap * 20) as u64;
                record(i as u64, begin, begin + 10, 0)
            })
            .collect();
        let started = Instant::now();
        let generator = Arc::new(
            LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap(),
        );
        let index = TransactionIndex::build(generator, None).unwrap();
        let preparation = started.elapsed();
        let rows = std::env::var("VOLNA_TX_PROFILE_ROWS")
            .ok()
            .and_then(|rows| rows.parse::<u8>().ok())
            .unwrap_or(4);
        let mut lane = TxLane::unresolved(TraceId::A, vec![], RowHeight::try_from(rows).unwrap());
        let mut cache = DetailCache::default();
        let mut scene = Scene::default();
        let mut text = TextCache::default();
        let mut samples = Vec::new();
        for frame in 0..120 {
            lane.first_sub_row = if overlap > 1 { frame % 32 } else { 0 };
            let start = frame as f64 * 0.5;
            let viewport = Viewport {
                start,
                end: start + f64::from(count / overlap * 20),
            };
            scene.clear();
            let started = Instant::now();
            draw(&index, &lane, viewport, &mut cache, &mut scene, &mut text);
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "{name}: preparation={preparation:?}, index_bytes={}, paint_p50={:.3}ms paint_p95={:.3}ms",
            index.resident_bytes(),
            samples[60],
            samples[114]
        );
    }
}

fn record(id: u64, begin: u64, end: u64, stages: usize) -> Transaction {
    Transaction {
        id: TransactionRef(id),
        generator: TrackRef(1),
        begin,
        end,
        status: Status::Ok,
        kind: SpanKind::Unspecified,
        parent: None,
        attributes: vec![TransactionAttribute {
            key: "vtr.label".into(),
            value: AttributeValue::Text(format!("instruction {id}")),
        }],
        events: vec![],
        stages: (0..stages)
            .map(|i| TransactionStage {
                id: TransactionRef(((id + 1) << 32) | i as u64),
                generator: None,
                name: "stage".into(),
                lane: "0".into(),
                begin,
                end: Some(end),
                attributes: vec![],
            })
            .collect(),
    }
}

#[test]
fn constant_one_occupancy_has_uniform_ribbon_intensity_at_fractional_scales() {
    let mut records = vec![record(0, 0, 10_000, 0)];
    records.extend((1..61).map(|id| record(id, 20_000, 20_100, 0)));
    let index = TransactionIndex::build(
        Arc::new(LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap()),
        None,
    )
    .unwrap();
    let lane = TxLane::unresolved(TraceId::A, vec![], RowHeight::PRESETS[3]);
    for dark in [true, false] {
        for viewport in [
            Viewport {
                start: 0.1,
                end: 1000.3,
            },
            Viewport {
                start: 19.81,
                end: 2000.23,
            },
            Viewport {
                start: 100.2,
                end: 123.456,
            },
        ] {
            let mut scene = Scene::default();
            draw_with_theme(
                &index,
                &lane,
                viewport,
                &mut DetailCache::default(),
                &mut scene,
                &mut TextCache::default(),
                &Theme::volna(dark),
            );
            let first = bar_ink_at(&scene, point(0.5, 100.0)).unwrap();
            assert_eq!(first.a, 0.20);
            for x in 1..1000 {
                assert_eq!(
                    bar_ink_at(&scene, point(x as f32 + 0.5, 100.0)),
                    Some(first),
                    "constant occupancy at {viewport:?}, pixel {x}"
                );
            }
        }
    }
}

#[test]
fn visible_activity_stays_visible_in_ribbon_when_track_peak_is_sixty() {
    let mut records = vec![record(0, 20, 30, 0)];
    records.extend((1..5).map(|id| record(id, 40, 50, 0)));
    records.extend((5..65).map(|id| record(id, 80, 90, 0)));
    let index = TransactionIndex::build(
        Arc::new(LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap()),
        None,
    )
    .unwrap();
    assert_eq!(index.occupancy.maximum(), 60);
    assert_eq!(index.active_count(45), 4);
    assert_eq!(index.hidden_count(45, 0..4), 0);
    assert_eq!(index.active_count(25), 1);
    assert_eq!(index.hidden_count(25, 0..4), 0);
    let lane = TxLane::unresolved(TraceId::A, vec![], RowHeight::PRESETS[3]);
    for dark in [true, false] {
        let mut scene = Scene::default();
        draw_with_theme(
            &index,
            &lane,
            Viewport {
                start: 0.0,
                end: 100.0,
            },
            &mut DetailCache::default(),
            &mut scene,
            &mut TextCache::default(),
            &Theme::volna(dark),
        );
        assert!(bar_ink_at(&scene, point(150.0, 100.0)).is_none());
        let one = bar_ink_at(&scene, point(250.0, 100.0)).unwrap();
        let four = bar_ink_at(&scene, point(450.0, 100.0)).unwrap();
        let burst = bar_ink_at(&scene, point(850.0, 100.0)).unwrap();
        assert_eq!(one.a, 0.20, "one transaction uses the baseline intensity");
        assert!(four.a > one.a && burst.a > four.a);
    }
}

#[test]
fn ribbon_total_activity_is_independent_of_visible_rows() {
    let mut records: Vec<_> = (0..4).map(|id| record(id, 0, 100, 0)).collect();
    records.extend([record(4, 30, 40, 0), record(5, 30, 40, 0)]);
    let mut failed = record(6, 35, 36, 0);
    failed.status = Status::Error;
    records.extend([failed, record(7, 35, 35, 0)]);
    let index = TransactionIndex::build(
        Arc::new(LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap()),
        None,
    )
    .unwrap();
    let mut lane = TxLane::unresolved(TraceId::A, vec![], RowHeight::PRESETS[3]);
    let mut scene = Scene::default();
    let viewport = Viewport {
        start: 0.0,
        end: 100.0,
    };
    let mut cache = DetailCache::default();
    let mut text = TextCache::default();
    for dark in [true, false] {
        let theme = Theme::volna(dark);
        draw_with_theme(
            &index, &lane, viewport, &mut cache, &mut scene, &mut text, &theme,
        );
        assert!(
            bar_ink_at(&scene, point(200.0, 100.0)).is_some(),
            "visible activity registers before the overflow burst"
        );
        assert!(bar_ink_at(&scene, point(320.0, 100.0)).is_some());
        assert_eq!(
            bar_ink_at(&scene, point(355.0, 100.0)),
            Some(theme.editor.error)
        );
        assert!(scene.prims.iter().any(|prim| matches!(prim, Prim::Rects { rects, color } if *color == theme.wave_event_coalesced && rects.iter().any(|rect| rect.contains(point(350.0, 100.0))))));
        assert!(
            !scene
                .prims
                .iter()
                .any(|prim| matches!(prim, Prim::Polygon { .. }))
        );
        scene.clear();
    }
    draw(&index, &lane, viewport, &mut cache, &mut scene, &mut text);
    let samples = [200.0, 320.0, 350.0, 355.0];
    let before = samples.map(|x| bar_ink_at(&scene, point(x, 100.0)));
    scene.clear();
    lane.first_sub_row = 4;
    draw(&index, &lane, viewport, &mut cache, &mut scene, &mut text);
    assert_eq!(samples.map(|x| bar_ink_at(&scene, point(x, 100.0))), before);
    let theme = Theme::volna(true);
    assert!(
        bar_ink_at(&scene, point(200.0, 100.0)).is_some(),
        "lower rows are hidden after scrolling"
    );
    assert_eq!(
        bar_ink_at(&scene, point(355.0, 100.0)),
        Some(theme.editor.error),
        "errors stay in the summary when their rows become visible"
    );
    assert!(scene.prims.iter().any(
        |prim| matches!(prim, Prim::Rects { color, .. } if *color == theme.wave_event_coalesced)
    ));
}

#[test]
fn one_row_fills_the_track_and_expanding_occupancy_preserves_bars() {
    let index = TransactionIndex::build(
        Arc::new(
            LoadedGenerator::new(
                TrackRef(1),
                vec![record(1, 10, 90, 0)],
                Default::default(),
                vec![],
            )
            .unwrap(),
        ),
        None,
    )
    .unwrap();
    let mut lane = TxLane::unresolved(TraceId::A, vec![], RowHeight::DEFAULT);
    let mut scene = Scene::default();
    let mut cache = DetailCache::default();
    let mut text = TextCache::default();
    let viewport = Viewport {
        start: 0.0,
        end: 100.0,
    };
    draw(&index, &lane, viewport, &mut cache, &mut scene, &mut text);
    let fill = Theme::volna(true).transaction_style(false).fill;
    let bars: Vec<_> = scene.quads().filter(|(_, color)| *color == fill).collect();
    assert_eq!(bars.len(), 1);
    assert_eq!((bars[0].0.top(), bars[0].0.height()), (2.0, 20.0));
    assert!(
        !scene
            .prims
            .iter()
            .any(|prim| matches!(prim, Prim::Polygon { .. }))
    );
    lane.occupancy = true;
    scene.clear();
    draw(&index, &lane, viewport, &mut cache, &mut scene, &mut text);
    assert_eq!(
        scene
            .quads()
            .filter(|(_, color)| *color == fill)
            .collect::<Vec<_>>(),
        bars
    );
    assert!(scene.texts().any(|text| text.starts_with("occupancy ·")));
    assert!(
        scene
            .prims
            .iter()
            .any(|prim| matches!(prim, Prim::Polygon { .. }))
    );
}

fn draw(
    index: &TransactionIndex,
    lane: &TxLane,
    viewport: Viewport,
    cache: &mut DetailCache,
    scene: &mut Scene,
    text: &mut TextCache,
) {
    let theme = Theme::volna(true);
    draw_with_theme(index, lane, viewport, cache, scene, text, &theme);
}

fn draw_with_theme(
    index: &TransactionIndex,
    lane: &TxLane,
    viewport: Viewport,
    cache: &mut DetailCache,
    scene: &mut Scene,
    text: &mut TextCache,
    theme: &Theme,
) {
    let geometry = LaneGeometry::new(
        0.0,
        24.0 * f32::from(lane.height.multiple())
            + if lane.occupancy {
                48.0
            } else if index.source().depth() > usize::from(lane.height.multiple()) {
                8.0
            } else {
                0.0
            },
        24.0,
        lane.height,
    );
    let plan = RenderPlan::new(lane, index, &viewport, cache);
    paint(
        lane,
        &plan,
        &Document::new(),
        &viewport,
        Rect::from_xywh(0.0, 0.0, 1000.0, geometry.height),
        &geometry,
        &mut TextPainter {
            theme,
            text,
            measure: &mut MonoMeasure,
            scene,
        },
    );
}

fn bar_ink_at(scene: &Scene, position: Point) -> Option<Color> {
    scene
        .prims
        .iter()
        .filter_map(|prim| match prim {
            Prim::Rects { rects, color } if rects.iter().any(|r| r.contains(position)) => {
                Some(*color)
            }
            Prim::Lines {
                segments,
                color,
                width,
            } if segments.iter().any(|[a, b]| {
                let stroke = Rect::from_xywh(
                    a.x.min(b.x) - width / 2.0,
                    a.y.min(b.y) - width / 2.0,
                    (a.x - b.x).abs() + width,
                    (a.y - b.y).abs() + width,
                );
                stroke.contains(position)
            }) =>
            {
                Some(*color)
            }
            _ => None,
        })
        .next_back()
}

#[test]
fn touching_transactions_and_one_pixel_gaps_remain_distinct_in_every_theme() {
    let lane = TxLane::unresolved(TraceId::A, vec![], RowHeight::PRESETS[3]);
    for (id, _) in builtin::choices() {
        for dark in [true, false] {
            let theme = builtin::resolve(id, dark).unwrap().canvas;
            for count in [2, 100] {
                for status in [Status::Ok, Status::Error] {
                    let style = theme.transaction_style(status == Status::Error);
                    for gap in [0, 1] {
                        let records = (0..count)
                            .map(|id| {
                                let end = (id + 1) * 10 - if id == 0 { gap } else { 0 };
                                let mut tx = record(id, id * 10, end, 0);
                                tx.status = status;
                                tx
                            })
                            .collect();
                        let generator =
                            LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![])
                                .unwrap();
                        let index = TransactionIndex::build(Arc::new(generator), None).unwrap();
                        let mut scene = Scene::default();
                        draw_with_theme(
                            &index,
                            &lane,
                            Viewport {
                                start: 0.0,
                                end: 1000.0,
                            },
                            &mut DetailCache::default(),
                            &mut scene,
                            &mut TextCache::default(),
                            &theme,
                        );
                        let boundary = 10.0 - gap as f32 - 0.5;
                        assert!(scene.prims.iter().any(|prim| {
                            matches!(prim, Prim::Lines { segments, color, width }
                            if *color == style.edge && *width == 1.0
                                && segments.contains(&[
                                    point(boundary, 2.5), point(boundary, 21.5)
                                ]))
                        }));
                        assert_eq!(
                            bar_ink_at(&scene, point(4.5, 12.0)),
                            Some(style.fill),
                            "{id}"
                        );
                        assert_eq!(
                            bar_ink_at(&scene, point(10.5, 12.0)),
                            Some(style.edge),
                            "{id}"
                        );
                        let expected = (gap == 0).then_some(style.edge);
                        assert_eq!(
                            bar_ink_at(&scene, point(9.5, 12.0)),
                            expected,
                            "{id}: {count}, {status:?}, gap={gap}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn hidden_density_and_stage_heavy_rows_keep_drawing_work_bounded() {
    let mut counts = Vec::new();
    for density in [1, 10, 100] {
        let records = (0..200 * density)
            .map(|id| record(id, id / density * 10, id / density * 10 + 80, 4))
            .collect();
        let generator =
            LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap();
        let generator = TransactionIndex::build(Arc::new(generator), None).unwrap();
        for height in RowHeight::PRESETS {
            let lane = TxLane::unresolved(TraceId::A, vec![], height);
            let mut scene = Scene::default();
            draw(
                &generator,
                &lane,
                Viewport {
                    start: 0.0,
                    end: 2000.0,
                },
                &mut DetailCache::default(),
                &mut scene,
                &mut TextCache::default(),
            );
            assert!(scene.prims.len() <= 405);
            assert!(scene.texts().count() <= 385);
            if height.multiple() <= 2 {
                assert!(scene.texts().all(|label| !label.starts_with("instruction")));
            }
            if height.multiple() == 8 {
                counts.push(scene.prims.len());
            }
        }
    }
    assert!(
        counts[2] <= counts[0] + 20,
        "bounded ribbon shades: {counts:?}"
    );
    let generator = LoadedGenerator::new(
        TrackRef(1),
        vec![record(0, 0, 2000, 65_536)],
        Default::default(),
        vec![],
    )
    .unwrap();
    let generator = TransactionIndex::build(Arc::new(generator), None).unwrap();
    let lane = TxLane::unresolved(TraceId::A, vec![], RowHeight::PRESETS[4]);
    let mut scene = Scene::default();
    draw(
        &generator,
        &lane,
        Viewport {
            start: 0.0,
            end: 2000.0,
        },
        &mut DetailCache::default(),
        &mut scene,
        &mut TextCache::default(),
    );
    assert!(scene.prims.len() <= 20);
    assert!(scene.texts().any(|text| text == "instruction 0"));
    assert_eq!(
        generator
            .source()
            .transaction(TransactionRef(0))
            .unwrap()
            .stages
            .len(),
        65_536
    );
}

#[test]
fn stage_gaps_and_overlapping_stalls_do_not_change_transaction_bars() {
    let lane = TxLane::unresolved(TraceId::A, vec![], RowHeight::PRESETS[3]);
    let viewport = Viewport {
        start: 0.0,
        end: 1000.0,
    };
    for count in [1, 100] {
        for status in [Status::Ok, Status::Error] {
            let mut baseline = None;
            for stages in [0, 1, 2] {
                let records = (0..count)
                    .map(|id| {
                        let begin = id * 10;
                        let end = begin + 8;
                        let mut tx = record(id, begin, end, stages);
                        tx.status = status;
                        if stages == 1 {
                            tx.stages[0].begin = begin + 2;
                            tx.stages[0].end = Some(end - 2);
                        } else if stages == 2 {
                            tx.stages[1].name = "stl".into();
                            tx.stages[1].lane = "stall".into();
                            tx.stages[1].begin = begin + 2;
                            tx.stages[1].end = Some(end - 2);
                        }
                        tx
                    })
                    .collect();
                let generator =
                    LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap();
                let index = TransactionIndex::build(Arc::new(generator), None).unwrap();
                let mut scene = Scene::default();
                draw(
                    &index,
                    &lane,
                    viewport,
                    &mut DetailCache::default(),
                    &mut scene,
                    &mut TextCache::default(),
                );
                if let Some(expected) = &baseline {
                    assert_eq!(&scene.prims, expected);
                } else {
                    baseline = Some(scene.prims);
                }
            }
        }
    }
}

#[test]
fn zoom_restores_bars_and_detail_hysteresis_uses_the_same_plan_for_hits() {
    let generator = LoadedGenerator::new(
        TrackRef(1),
        (0..100)
            .map(|id| record(id, id * 10, id * 10 + 5, 1))
            .collect(),
        Default::default(),
        vec![],
    )
    .unwrap();
    let generator = TransactionIndex::build(Arc::new(generator), None).unwrap();
    let mut lane = TxLane::unresolved(TraceId::A, vec![], RowHeight::PRESETS[3]);
    lane.source = crate::pipeline::TrackSource::Resolved {
        track: crate::trace::Traced::new(TraceId::A, TrackRef(1)),
        path: vec![],
    };
    let mut cache = DetailCache::default();
    for (end, exact) in [
        (1000.0, false),
        (630.0, false),
        (400.0, true),
        (630.0, true),
        (1000.0, false),
    ] {
        let plan = RenderPlan::new(&lane, &generator, &Viewport { start: 0.0, end }, &mut cache);
        assert_eq!(plan.details[0].is_some(), exact);
    }
    let mut scene = Scene::default();
    draw(
        &generator,
        &lane,
        Viewport {
            start: 0.0,
            end: 25.0,
        },
        &mut cache,
        &mut scene,
        &mut TextCache::default(),
    );
    assert!(scene.texts().any(|text| text == "instruction 0"));
    assert!(
        scene
            .prims
            .iter()
            .any(|prim| matches!(prim, Prim::Rects { .. }))
    );
}

#[test]
#[ignore = "manual release-mode density and row-scaling measurement"]
fn profile_dense_navigation() {
    let source = std::env::var("VOLNA_TX_PROFILE_TRACE").ok().map_or_else(
        || {
            (0..4000)
                .map(|id| record(id, id / 4, id / 4 + 20 + id % 17, 4))
                .collect()
        },
        |path| {
            let session = volna_trace::session::OpenSpec::Path(path.into())
                .open()
                .unwrap();
            let track = session
                .tracks()
                .iter()
                .find(|track| track.path.join(".") == "cpu.thread0")
                .unwrap()
                .id;
            session.load_track(track).unwrap().generators[0]
                .transactions()
                .to_vec()
        },
    );
    for density in [1, 10, 100] {
        let mut identity = 0;
        let mut records = Vec::with_capacity(source.len() * density);
        for _ in 0..density {
            for original in &source {
                let mut tx = original.clone();
                tx.generator = TrackRef(1);
                tx.id = TransactionRef(identity);
                identity += 1;
                tx.parent = None;
                for stage in &mut tx.stages {
                    stage.id = TransactionRef(identity);
                    stage.generator = None;
                    identity += 1;
                }
                for event in &mut tx.events {
                    event.id = TransactionRef(identity);
                    event.generator = None;
                    identity += 1;
                }
                records.push(tx);
            }
        }
        let generator =
            LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap();
        let generator = TransactionIndex::build(Arc::new(generator), None).unwrap();
        for rows in [1, 8] {
            let lane = TxLane::unresolved(TraceId::A, vec![], RowHeight::PRESETS[3]);
            let mut cache = DetailCache::default();
            let mut scene = Scene::default();
            let mut text = TextCache::default();
            let mut times = Vec::new();
            for frame in 0..150 {
                let start = 1000.0 + f64::from(frame % 100) * 0.25;
                let now = std::time::Instant::now();
                scene.clear();
                for _ in 0..rows {
                    draw(
                        &generator,
                        &lane,
                        Viewport {
                            start,
                            end: start + 3000.0,
                        },
                        &mut cache,
                        &mut scene,
                        &mut text,
                    );
                }
                if frame >= 30 {
                    times.push(now.elapsed().as_secs_f64() * 1000.0);
                }
            }
            times.sort_by(f64::total_cmp);
            eprintln!(
                "density={density} tracks={rows} records={} depth={} p50_ms={:.3} p95_ms={:.3} scene_primitives={} rectangles={}",
                generator.source().transactions().len(),
                generator.source().depth(),
                times[times.len() / 2],
                times[times.len() * 95 / 100],
                scene.prims.len(),
                scene.quads().count()
            );
        }
    }
}
