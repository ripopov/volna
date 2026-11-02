//! Headless transaction lane tests for shared records, navigation, persistence, and
//! painting.

use std::path::PathBuf;
use std::sync::Arc;
use volna_core::testing::{a, a_all};

use volna_core::Theme;
use volna_core::app::{Action, App, Command};
use volna_core::document::TrackLoadState;
use volna_core::geometry::{Modifiers, MouseButton, Point, Rect, point};
use volna_core::panels::PanelId;
use volna_core::scene::{MonoMeasure, Prim};
use volna_core::wave::PointerEvent;
use volna_core::wave::lane::{self, LaneGeometry};
use volna_core::wave::model::{MenuAction, RowHeight, WaveRow};
use volna_core::wave::viewport::Viewport;
use volna_core::workspace::Workspace;
use volna_trace::data::Member;
use volna_trace::data::transactions::{Status, TrackRef};
use volna_trace::session::{OpenSpec, Session};

const TRACE: &str = "file:///tmp/trace.vtr";
const LOCATION: &str = "file:///tmp/trace.vtr.volna.json";

fn example(name: &str) -> Arc<dyn Session> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../volna/examples")
        .join(name);
    OpenSpec::Path(path).open().unwrap()
}

fn pump(app: &mut App) {
    loop {
        let requests = app.take_requests();
        if requests.is_empty() {
            return;
        }
        for request in requests {
            app.deliver(request.perform());
        }
    }
}

fn frame(app: &mut App, id: PanelId) -> &volna_core::scene::Scene {
    frame_w(app, id, 1400.0)
}

fn frame_w(app: &mut App, id: PanelId, width: f32) -> &volna_core::scene::Scene {
    let theme = Theme::one_dark();
    app.layout_panel(
        id,
        Rect::from_xywh(0.0, 0.0, width, 600.0),
        &theme,
        1.0,
        &mut MonoMeasure,
    )
    .unwrap();
    app.render_panel(id, &theme, &mut MonoMeasure)
}

fn track(session: &dyn Session, path: &str) -> TrackRef {
    session
        .tracks()
        .iter()
        .find(|t| t.path.join(".") == path)
        .unwrap_or_else(|| panic!("no track {path}"))
        .id
}

fn generator(session: &dyn Session, track: TrackRef) -> Member {
    Member::Generator(
        session
            .hierarchy()
            .generators()
            .iter()
            .position(|g| g.track == track)
            .unwrap(),
    )
}

fn with_lanes(paths: &[&str]) -> (App, Arc<dyn Session>, PanelId, Vec<TrackRef>) {
    let session = example("pipeline_showcase.vtr");
    let mut app = App::new();
    app.set_session(session.clone());
    let tracks: Vec<_> = paths.iter().map(|p| track(session.as_ref(), p)).collect();
    app.handle(Command::AddToWaves(a_all(
        tracks.iter().map(|t| generator(session.as_ref(), *t)),
    )));
    let waves = app.panels.focused_id();
    pump(&mut app);
    frame(&mut app, waves);
    (app, session, waves, tracks)
}

const CPU0: &str = "soc.cpu0.pipeline.instruction";
const CPU1: &str = "soc.cpu1.pipeline.instruction";

fn show(app: &mut App, waves: PanelId, start: f64, end: f64) {
    let volna_core::App { panels, doc, .. } = app;
    panels
        .waves_mut(waves)
        .unwrap()
        .nav
        .jump_to(doc, Viewport { start, end });
    frame(app, waves);
}

fn bar_point(app: &App, waves: PanelId, row: usize, ordinal: usize) -> Point {
    let w = app.panels.waves(waves).unwrap();
    let lane = w.items()[row].lane().unwrap();
    let g = lane.generator(&app.doc).unwrap();
    let tx = &g.transactions()[ordinal];
    let layout = w.last_layout();
    let vp = w.viewport(&app.doc);
    let width = layout.wave_width_f64();
    let mid = (tx.begin + tx.end) as f64 / 2.0;
    let geometry = LaneGeometry::new(
        layout.row_y(row),
        layout.row_height(row),
        layout.row_h,
        lane.height,
    );
    let sub = g.sub_row(ordinal) - lane.first_sub_row;
    point(
        layout.waves.left() + vp.x_of(mid, width) as f32,
        geometry.sub_top(sub) + geometry.sub_h / 2.0,
    )
}

fn click(app: &mut App, waves: PanelId, p: Point) {
    app.handle(Command::Pointer(
        waves,
        PointerEvent::Down {
            position: p,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    app.handle(Command::Pointer(waves, PointerEvent::Up));
}

#[test]
fn add_to_waves_loads_shares_and_releases_generator_records() {
    let (mut app, _session, waves, tracks) = with_lanes(&[CPU0, CPU1]);
    let w = app.panels.waves(waves).unwrap();
    assert_eq!(w.items().len(), 2);
    assert_eq!(w.selected.iter().copied().collect::<Vec<_>>(), [0, 1]);
    let heights: Vec<_> = w.items().iter().map(|r| r.height().multiple()).collect();
    assert_eq!(heights, [4, 4]);
    let cpu1 = w.items()[1].lane().unwrap();
    assert_eq!(cpu1.generator(&app.doc).unwrap().depth(), 16);
    assert_eq!(lane::hidden_rows(16, cpu1.height), 14);

    app.handle(Command::OpenPipeline {
        track: a(tracks[0]),
    });
    pump(&mut app);
    let pipeline = app.panels.focused_id();
    let shared = |app: &App| match app.doc.track(a(tracks[0])) {
        Some(TrackLoadState::Ready(loaded)) => loaded.generators[0].clone(),
        _ => panic!("not resident"),
    };
    let object = shared(&app);
    let lane_object = std::ptr::from_ref(
        app.panels.waves(waves).unwrap().items()[0]
            .lane()
            .unwrap()
            .generator(&app.doc)
            .unwrap(),
    );
    assert!(std::ptr::eq(Arc::as_ptr(&object), lane_object));

    app.handle(Command::Panels(volna_core::panels::PanelsCommand::Focus(
        waves,
    )));
    app.panels.waves_mut(waves).unwrap().selected = [0, 1].into();
    app.handle(Command::Action(Action::RemoveSelected));
    assert!(app.panels.waves(waves).unwrap().items().is_empty());
    assert!(matches!(
        app.doc.track(a(tracks[0])),
        Some(TrackLoadState::Ready(_))
    ));
    assert!(app.doc.track(a(tracks[1])).is_none());
    app.handle(Command::Panels(volna_core::panels::PanelsCommand::Close(
        pipeline,
    )));
    assert!(app.doc.track(a(tracks[0])).is_none());

    app.handle(Command::Panels(volna_core::panels::PanelsCommand::Focus(
        waves,
    )));
    app.handle(Command::AddToWaves(a_all(vec![
        Member::Generator(0),
        Member::Var(0),
    ])));
    let w = app.panels.waves(waves).unwrap();
    assert_eq!(w.items().len(), 2);
    assert_eq!(
        w.items()
            .iter()
            .filter(|item| item.lane().is_some())
            .count(),
        1
    );
    assert_eq!(
        w.items()
            .iter()
            .filter(|item| item.signal().is_some())
            .count(),
        1
    );
}

#[test]
fn clicking_a_bar_selects_details_and_occupancy_retains_it() {
    let (mut app, _session, waves, tracks) = with_lanes(&[CPU0]);
    let g = app.panels.waves(waves).unwrap().items()[0]
        .lane()
        .unwrap()
        .generator(&app.doc)
        .unwrap()
        .transactions()
        .to_vec();
    let ordinal = (0..g.len())
        .find(|&i| {
            app.panels.waves(waves).unwrap().items()[0]
                .lane()
                .unwrap()
                .generator(&app.doc)
                .unwrap()
                .sub_row(i)
                == 1
        })
        .unwrap();
    let tx = &g[ordinal];
    show(
        &mut app,
        waves,
        tx.begin as f64 - 20.0,
        tx.end as f64 + 40.0,
    );
    let p = bar_point(&app, waves, 0, ordinal);
    click(&mut app, waves, p);
    let selection = app.doc.selection().expect("selected");
    assert_eq!(
        (selection.track, selection.id, selection.origin),
        (a(tracks[0]), tx.id, waves)
    );
    assert!(app.panels.waves(waves).unwrap().pressed_record);
    let cursor = app.panels.waves(waves).unwrap().cursor(&app.doc).unwrap();
    assert!(cursor >= tx.begin && cursor <= tx.end);

    app.handle(Command::ShowTransaction { from: waves });
    let panel = app.panels.focused_id();
    assert!(app.panels.transaction(panel).is_some());

    let theme = Theme::one_dark();
    let scene = frame(&mut app, waves);
    assert!(scene.prims.iter().any(|prim| matches!(
        prim,
        Prim::Quad { border_width, border_color, .. } if *border_width == 2.0 && *border_color == theme.border_focused
    )));

    let layout = app.panels.waves(waves).unwrap().last_layout().clone();
    let empty = point(
        layout.waves.left() + 5.0,
        layout.row_y(0) + layout.row_height(0) - 2.0,
    );
    click(&mut app, waves, empty);
    assert_eq!(app.doc.selection().unwrap().id, tx.id);
    assert!(app.panels.waves(waves).unwrap().pressed_record);

    show(
        &mut app,
        waves,
        tx.begin as f64 - 20.0,
        tx.end as f64 + 40.0,
    );
    let w = app.panels.waves(waves).unwrap();
    let layout = w.last_layout().clone();
    let vp = w.viewport(&app.doc);
    let x = layout.waves.left() + vp.x_of(tx.end as f64, layout.wave_width_f64()) as f32 + 2.0;
    click(&mut app, waves, point(x, p.y));
    assert_eq!(
        app.panels.waves(waves).unwrap().cursor(&app.doc),
        Some(tx.end - 1)
    );
}

#[test]
fn edge_steps_walk_record_begins_and_ends_on_a_lane() {
    let (mut app, _session, waves, _tracks) = with_lanes(&[CPU0]);
    let mut bounds: Vec<u64> = {
        let g = app.panels.waves(waves).unwrap().items()[0]
            .lane()
            .unwrap()
            .generator(&app.doc)
            .unwrap();
        g.transactions()
            .iter()
            .flat_map(|t| [t.begin, t.end])
            .collect()
    };
    bounds.sort_unstable();
    bounds.dedup();
    let cursor = |app: &App| app.panels.waves(waves).unwrap().cursor(&app.doc);
    app.panels.waves_mut(waves).unwrap().selected = [0].into();
    app.panels.waves_mut(waves).unwrap().anchor = Some(0);
    let volna_core::App { panels, doc, .. } = &mut app;
    panels.waves_mut(waves).unwrap().set_cursor(doc, Some(0));
    for &expected in &bounds[..12] {
        app.handle(Command::Action(Action::NextEdge));
        assert_eq!(cursor(&app), Some(expected));
    }
    for &expected in bounds[..11].iter().rev() {
        app.handle(Command::Action(Action::PrevEdge));
        assert_eq!(cursor(&app), Some(expected));
    }
}

#[test]
fn lanes_cut_paste_resize_and_round_trip_through_the_workspace() {
    let (mut app, session, waves, tracks) = with_lanes(&[CPU0]);
    app.handle(Command::AddVars(a_all(vec![0])));
    assert_eq!(app.panels.waves(waves).unwrap().items().len(), 2);

    app.panels.waves_mut(waves).unwrap().selected = [0, 1].into();
    app.handle(Command::Action(Action::CutSignals));
    assert!(app.panels.waves(waves).unwrap().items().is_empty());
    assert!(app.doc.track(a(tracks[0])).is_none(), "no lane shows it");
    app.handle(Command::Action(Action::SplitRight));
    let other = app.panels.focused_id();
    app.handle(Command::Action(Action::PasteSignals));
    pump(&mut app);
    let w = app.panels.waves(other).unwrap();
    assert!(matches!(w.items()[0].row, WaveRow::Lane(_)));
    assert_eq!(w.items()[0].height().multiple(), 4);
    assert!(w.items()[0].lane().unwrap().generator(&app.doc).is_some());

    frame(&mut app, other);
    app.panels.waves_mut(other).unwrap().selected = [0].into();
    app.handle(Command::OpenSignalMenu(other));
    let menu = app.panels.waves(other).unwrap().menu.clone().unwrap();
    assert!(menu.items().any(|i| i.label == "Remove lane"));
    app.handle(Command::MenuSelect(
        other,
        MenuAction::RowHeight(RowHeight::DEFAULT),
    ));
    let lane = app.panels.waves(other).unwrap().items()[0]
        .lane()
        .unwrap()
        .clone();
    assert_eq!(lane.height, RowHeight::DEFAULT);
    assert!(!lane.auto_height);
    let scene = frame(&mut app, other);
    assert!(scene.texts().any(|t| t == "+5"));

    let saved = serde_json::to_value(
        Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap(),
    )
    .unwrap();
    let rows = saved["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == other.0)
        .unwrap()["rows"]
        .clone();
    assert_eq!(rows[0]["type"], "lane");
    assert_eq!(
        rows[0]["generator"],
        serde_json::json!(["soc", "cpu0", "pipeline", "instruction"])
    );
    assert_eq!(rows[1]["type"], "signal");
    let restore = |app: &mut App, value: &serde_json::Value| {
        Workspace::parse(&serde_json::to_vec(value).unwrap())
            .unwrap()
            .prepare(app, TRACE, LOCATION)
            .unwrap()
            .commit(app)
            .unwrap()
    };
    restore(&mut app, &saved);
    pump(&mut app);
    let w = app.panels.waves(other).unwrap();
    let lane = w.items()[0].lane().unwrap();
    assert_eq!(lane.height, RowHeight::DEFAULT);
    assert!(lane.generator(&app.doc).is_some(), "restored lanes load");
    assert_eq!(
        serde_json::to_value(
            Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap()
        )
        .unwrap(),
        saved
    );

    let mut missing = saved;
    for panel in missing["panels"].as_array_mut().unwrap() {
        if panel["id"] == other.0 {
            panel["rows"][0]["generator"] = serde_json::json!(["no", "such"]);
        }
    }
    let report = restore(&mut app, &missing);
    assert!(report.notices.iter().any(|n| n.contains("Missing lane")));
    let w = app.panels.waves(other).unwrap();
    assert!(w.items()[0].lane().unwrap().track().is_none());
    let scene = frame(&mut app, other);
    assert!(scene.texts().any(|t| t == "not in trace"));
    drop(session);
}

#[test]
fn cursor_labels_align_with_bar_rows_in_both_themes_and_at_zoom() {
    let (mut app, _, waves, tracks) = with_lanes(&[CPU1]);
    let cursor = {
        let g = app.panels.waves(waves).unwrap().items()[0]
            .lane()
            .unwrap()
            .generator(&app.doc)
            .unwrap();
        g.transactions()
            .iter()
            .map(|tx| tx.begin)
            .find(|&time| {
                app.doc
                    .transaction_index(a(tracks[0]))
                    .and_then(|state| match state {
                        volna_core::analysis::AnalysisLoad::Ready(index) => {
                            Some(index.active_count(time))
                        }
                        _ => None,
                    })
                    .unwrap()
                    > 5
            })
            .unwrap()
    };
    show(
        &mut app,
        waves,
        cursor.saturating_sub(10) as f64,
        (cursor + 100) as f64,
    );
    let App { panels, doc, .. } = &mut app;
    panels.waves_mut(waves).unwrap().values_width = 400.0;
    panels
        .waves_mut(waves)
        .unwrap()
        .set_cursor(doc, Some(cursor));

    for theme in [Theme::one_dark(), Theme::islands_light()] {
        for zoom in [1.0, 1.5] {
            let theme = theme.zoomed(zoom);
            app.layout_panel(
                waves,
                Rect::from_xywh(0.0, 0.0, 1800.0, 600.0),
                &theme,
                1.0,
                &mut MonoMeasure,
            )
            .unwrap();
            let w = app.panels.waves(waves).unwrap();
            let layout = w.last_layout();
            let lane = w.items()[0].lane().unwrap();
            let geometry = LaneGeometry::new(
                layout.row_y(0),
                layout.row_height(0),
                layout.row_h,
                lane.height,
            );
            let values = layout.values;
            let expected = lane::row_values(
                match app.doc.transaction_index(a(tracks[0])).unwrap() {
                    volna_core::analysis::AnalysisLoad::Ready(index) => index,
                    _ => panic!("index ready"),
                },
                cursor,
                lane.height,
                lane.first_sub_row,
                256,
            );
            assert_eq!(expected.len(), 2);
            let scene = app.render_panel(waves, &theme, &mut MonoMeasure);
            let texts: Vec<_> = scene
                .prims
                .iter()
                .filter_map(|prim| match prim {
                    Prim::Text {
                        origin,
                        height,
                        text,
                        size,
                        color,
                        ..
                    } if values.contains(*origin) => Some((origin, height, text, size, color)),
                    _ => None,
                })
                .collect();
            assert_eq!(
                texts.len(),
                expected.iter().filter(|row| !row.text.is_empty()).count() + 1
            );
            for (sub, row) in expected
                .iter()
                .enumerate()
                .filter(|(_, row)| !row.text.is_empty())
            {
                let (origin, height, text, size, color) = texts
                    .iter()
                    .find(|(origin, ..)| origin.y == geometry.sub_top(sub) + 2.0 * zoom)
                    .unwrap();
                assert_eq!(origin.x, values.left() + 8.0 * zoom);
                assert_eq!(**height, geometry.sub_h - 4.0 * zoom);
                assert_eq!(**size, (0.9 * **height).min(theme.mono_size));
                assert!(row.text.starts_with(text.trim_end_matches('…')));
                if row.failed {
                    assert_eq!(**color, theme.editor.error);
                }
            }
        }
    }
}

#[test]
fn bars_keep_separate_rows_and_always_show_total_occupancy() {
    let (mut app, _session, waves, _tracks) = with_lanes(&[CPU1]);
    let theme = Theme::one_dark();
    let (median, depth, first) = {
        let g = app.panels.waves(waves).unwrap().items()[0]
            .lane()
            .unwrap()
            .generator(&app.doc)
            .unwrap();
        (g.median_lifetime(), g.depth(), g.transactions()[0].clone())
    };
    assert!(depth > 5);

    show(&mut app, waves, 0.0, (median * 6) as f64);
    let scene = frame_w(&mut app, waves, 1560.0);
    let label = lane::label(&first);
    let caption = |t: &str| t.starts_with("800000") && t.contains(": ");
    assert!(
        scene.texts().filter(|t| caption(t)).count() > 5,
        "bar captions"
    );
    assert!(scene.texts().any(|t| t == format!("+{}", depth - 2)));
    assert!(scene.texts().any(|t| t.ends_with("records")));
    let bar_quads = scene
        .quads()
        .filter(|(_, c)| *c == theme.wave_signal)
        .count();
    assert!(bar_quads > 5);
    assert!(
        scene
            .prims
            .iter()
            .any(|prim| matches!(prim, Prim::Lines { .. }))
    );

    let volna_core::App { panels, doc, .. } = &mut app;
    panels
        .waves_mut(waves)
        .unwrap()
        .set_cursor(doc, Some(first.begin));
    let expected = label;
    let scene = frame(&mut app, waves);
    assert!(
        scene
            .texts()
            .any(|t| t.len() > 8 && expected.starts_with(t.trim_end_matches('…'))),
        "value {expected}"
    );

    let end = app.doc.limits().1 as f64;
    show(&mut app, waves, 0.0, end);
    frame_w(&mut app, waves, 520.0);
    let scene = frame_w(&mut app, waves, 520.0);
    assert!(scene.texts().any(|text| text.starts_with("occupancy ·")));
    let columns = scene
        .quads()
        .filter(|(_, c)| c.h == theme.wave_signal.h && c.s == theme.wave_signal.s && c.a < 1.0)
        .count();
    assert!(columns > 0 && columns <= 4096, "{columns} density runs");
    let p = {
        let layout = app.panels.waves(waves).unwrap().last_layout();
        point(layout.waves.left() + 50.0, layout.row_y(0) + 10.0)
    };
    click(&mut app, waves, p);
    assert!(app.doc.record_cursor().is_some());
}

#[test]
fn a_failed_record_is_red_in_bars_and_values() {
    let session = example("feature_showcase.vtr");
    let mut app = App::new();
    app.set_session(session.clone());
    let read = track(session.as_ref(), "soc.dma.memory_bus.read");
    app.handle(Command::AddToWaves(a_all(vec![generator(
        session.as_ref(),
        read,
    )])));
    let waves = app.panels.focused_id();
    pump(&mut app);
    let failed = {
        let g = app.panels.waves(waves).unwrap().items()[0]
            .lane()
            .unwrap()
            .generator(&app.doc)
            .unwrap();
        g.transactions()
            .iter()
            .find(|t| t.status == Status::Error)
            .unwrap()
            .clone()
    };
    let theme = Theme::one_dark();
    show(
        &mut app,
        waves,
        failed.begin as f64 - 10.0,
        failed.end as f64 + 10.0,
    );
    let volna_core::App { panels, doc, .. } = &mut app;
    panels
        .waves_mut(waves)
        .unwrap()
        .set_cursor(doc, Some(failed.begin + 1));
    let scene = frame(&mut app, waves);
    assert!(
        scene.quads().any(|(_, c)| c == theme.editor.error),
        "red bar"
    );
    assert!(scene.prims.iter().any(|prim| matches!(
        prim,
        Prim::Text { text, color, .. } if *color == theme.editor.error && text.starts_with("DMA transfer")
    )));
}

#[test]
fn cursor_records_keep_the_waveforms_local_cursor_when_selecting_from_the_list() {
    use volna_core::document::RecordCursor;
    use volna_core::transaction::TransactionCommand;
    let (mut app, _, waves, tracks) = with_lanes(&[CPU0]);
    let tx = app
        .doc
        .resident_generator(a(tracks[0]))
        .unwrap()
        .transactions()[0]
        .clone();
    let time = tx.begin;
    {
        let App { panels, doc, .. } = &mut app;
        let model = panels.waves_mut(waves).unwrap();
        model.nav.link.cursor = false;
        model.set_cursor(doc, Some(time));
        doc.shared.cursor = Some(time + 1000);
        doc.set_record_cursor(RecordCursor {
            track: a(tracks[0]),
            origin: waves,
        });
    }
    app.handle(Command::ShowTransaction { from: waves });
    let details = app.panels.focused_id();
    let query = app.cursor_records(details).ready().unwrap();
    assert_eq!(query.time, time);
    assert!(query.count > 0);
    app.handle(Command::Transaction(
        details,
        TransactionCommand::CursorSelect {
            track: a(tracks[0]),
            id: tx.id,
            time,
        },
    ));
    assert_eq!(app.cursor_records(details).ready().unwrap().time, time);
    assert_eq!(app.doc.shared.cursor, Some(time + 1000));
    app.handle(Command::Panels(volna_core::panels::PanelsCommand::Focus(
        waves,
    )));
    app.handle(Command::Action(Action::SplitRight));
    let other = app.panels.focused_id();
    app.handle(Command::SelectTransaction {
        panel: other,
        track: a(tracks[0]),
        id: tx.id,
        cursor: Some(time + 2),
    });
    assert_eq!(app.cursor_records(details).ready().unwrap().time, time + 2);
}

#[test]
fn optional_index_refusal_keeps_records_available_retries_and_rejects_stale_results() {
    use volna_core::panels::PanelsCommand;
    use volna_core::{
        analysis::AnalysisLoad,
        session::{LoadRequest, LoadResult},
    };
    let session = example("pipeline_showcase.vtr");
    let track = a(track(session.as_ref(), CPU0));
    let member = a(generator(session.as_ref(), track.item));
    let mut app = App::new();
    app.set_session(session);
    app.handle(Command::OpenPipeline { track });
    let pipeline = app.panels.focused_id();
    loop {
        let requests = app.take_requests();
        if requests.is_empty() {
            break;
        }
        for request in requests {
            assert!(!matches!(request, LoadRequest::TransactionIndex { .. }));
            app.deliver(request.perform());
        }
    }
    let source = app.doc.resident_generator(track).unwrap();
    let budget = app
        .doc
        .session(track.trace)
        .unwrap()
        .memory_budget()
        .unwrap();
    let object_limit = budget.object_limit();
    budget.set_object_limit(1);
    app.handle(Command::AddToWaves(vec![member]));
    let waves = app.panels.focused_id();
    let mut requests = app.take_requests();
    assert_eq!(requests.len(), 1);
    let failed = requests.remove(0).perform();
    assert!(matches!(
        &failed,
        LoadResult::TransactionIndex { result: Err(_), .. }
    ));
    app.deliver(failed);
    assert!(matches!(
        app.doc.transaction_index(track),
        Some(AnalysisLoad::Failed)
    ));
    assert!(matches!(
        app.panels.pipeline(pipeline).unwrap().rows(&app.doc),
        volna_core::pipeline::Rows::Ready(_)
    ));
    assert!(Arc::ptr_eq(
        &source,
        &app.doc.resident_generator(track).unwrap()
    ));
    budget.set_object_limit(object_limit);
    app.handle(Command::SetSidebarWidth(app.sidebar_width));
    pump(&mut app);
    let Some(AnalysisLoad::Ready(index)) = app.doc.transaction_index(track) else {
        panic!("index retried")
    };
    let old_index = Arc::downgrade(index);
    app.handle(Command::Panels(PanelsCommand::Focus(waves)));
    app.handle(Command::Action(Action::RemoveSelected));
    assert!(app.doc.transaction_index(track).is_none());
    assert!(old_index.upgrade().is_none());
    assert!(app.doc.resident_generator(track).is_some());
    app.handle(Command::AddToWaves(vec![member]));
    let pending = app.take_requests().remove(0);
    app.handle(Command::Action(Action::RemoveSelected));
    app.handle(Command::AddToWaves(vec![member]));
    let current = app.take_requests().remove(0);
    let result = pending.perform();
    let LoadResult::TransactionIndex {
        result: Ok(index), ..
    } = &result
    else {
        panic!("prepared stale index")
    };
    let stale = Arc::downgrade(index);
    app.deliver(result);
    assert!(stale.upgrade().is_none());
    assert!(matches!(
        app.doc.transaction_index(track),
        Some(AnalysisLoad::Building(_))
    ));
    app.deliver(current.perform());
    assert!(matches!(
        app.doc.transaction_index(track),
        Some(AnalysisLoad::Ready(_))
    ));
    app.handle(Command::Action(Action::RemoveSelected));
    budget.set_object_limit(1);
    app.handle(Command::AddToWaves(vec![member]));
    let failure = app.take_requests().remove(0).perform();
    budget.set_object_limit(object_limit);
    app.handle(Command::SetSidebarWidth(app.sidebar_width));
    app.deliver(failure);
    pump(&mut app);
    assert!(matches!(
        app.doc.transaction_index(track),
        Some(AnalysisLoad::Ready(_))
    ));
}

#[test]
fn revealed_sub_rows_autosave_round_trip_and_follow_undo() {
    use volna_core::panels::PanelsCommand;
    let (mut app, _, waves, tracks) = with_lanes(&[CPU1]);
    let track = a(tracks[0]);
    let source = app.doc.resident_generator(track).unwrap();
    let ordinal = (0..source.transactions().len())
        .max_by_key(|&i| source.sub_row(i))
        .unwrap();
    let id = source.transactions()[ordinal].id;
    app.doc.shared.cursor = Some(source.transactions()[ordinal].begin);
    app.handle(Command::SelectTransaction {
        panel: waves,
        track,
        id,
        cursor: None,
    });
    app.handle(Command::ShowTransaction { from: waves });
    let details = app.panels.focused_id();
    app.configure_persistence(volna_core::workspace::persistence::Persistence::Auto);
    let revision = app.workspace.scheduler.revision();
    app.handle(Command::RevealWaveTransaction { panel: details });
    let first = app.panels.waves(waves).unwrap().items()[0]
        .lane()
        .unwrap()
        .first_sub_row;
    assert!(first > 0);
    assert!(app.workspace.scheduler.revision() > revision);
    app.handle(Command::Undo);
    assert_eq!(
        app.panels.waves(waves).unwrap().items()[0]
            .lane()
            .unwrap()
            .first_sub_row,
        0
    );
    app.handle(Command::Redo);
    assert_eq!(
        app.panels.waves(waves).unwrap().items()[0]
            .lane()
            .unwrap()
            .first_sub_row,
        first
    );
    let save = || Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    let bytes = serde_json::to_vec(&save()).unwrap();
    Workspace::parse(&bytes)
        .unwrap()
        .prepare(&app, TRACE, LOCATION)
        .unwrap()
        .commit(&mut app)
        .unwrap();
    pump(&mut app);
    assert_eq!(
        app.panels.waves(waves).unwrap().items()[0]
            .lane()
            .unwrap()
            .first_sub_row,
        first
    );
    let mut invalid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let panel = invalid["panels"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["id"] == waves.0)
        .unwrap();
    panel["rows"][0]["first_sub_row"] = serde_json::json!(usize::MAX);
    assert!(
        Workspace::parse(&serde_json::to_vec(&invalid).unwrap())
            .unwrap()
            .prepare(&app, TRACE, LOCATION)
            .is_err()
    );
    app.handle(Command::Panels(PanelsCommand::Focus(waves)));
}
