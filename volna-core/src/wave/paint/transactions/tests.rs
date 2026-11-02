use super::paint;
use crate::data::transaction_index::TransactionIndex;
use crate::{
    data::loaded_tracks::LoadedGenerator,
    document::Document,
    geometry::Rect,
    scene::{MonoMeasure, Prim, Scene, TextCache},
    theme::Theme,
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
            theme: &theme,
            text,
            measure: &mut MonoMeasure,
            scene,
        },
    );
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
