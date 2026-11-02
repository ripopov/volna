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
        24.0 * f32::from(lane.height.multiple()),
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
    assert!(counts[2] <= counts[0] + 4);
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
