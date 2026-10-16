//! Headless pipeline panel tests: open a track from the sidebar, load it, paint it,
//! navigate it, save and restore it.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;
use volna_core::sidebar::TreeNode;
use volna_core::testing::{a, a_all};
use volna_core::trace::TraceId;

use volna_core::app::{Action, App, Command, Event, PanelLayout};
use volna_core::geometry::{Modifiers, MouseButton, Point, Rect, point};
use volna_core::nav::LinkDim;
use volna_core::panels::{Layout, PanelId, PanelsCommand};
use volna_core::pipeline::{Hit, PipelineModel, RowView, Rows, TrackSource};
use volna_core::scene::{FontRole, MonoMeasure, Prim};
use volna_core::session::LoadRequest;
use volna_core::sidebar::Key;
use volna_core::wave::PointerEvent;
use volna_core::workspace::Workspace;
use volna_core::{Instant, Theme};
use volna_trace::data::Member;
use volna_trace::data::source::Lookup;
use volna_trace::data::transactions::TrackRef;
use volna_trace::session::{OpenSpec, Session};

const STAGES: [&str; 3] = ["F", "D", "X"];

fn fixture(n: u64) -> Arc<dyn Session> {
    fixture_with_stages(n, &STAGES)
}

fn fixture_with_stages(n: u64, stages: &[&str]) -> Arc<dyn Session> {
    use vtr::{Direction, ScopeType, SignalKind, TxStatus, Value, VarType};
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut w = vtr::Writer::create(file.path()).unwrap();
    w.set_timescale(vtr::Timescale::Exponent(0)).unwrap();
    let unit = w.intern("cycle");
    w.set_file_attr("time.unit", Value::Str(unit)).unwrap();
    let cpu = w.add_scope(None, "cpu", ScopeType::Core, "").unwrap();
    let (_, counter) = w
        .add_var(
            Some(cpu),
            "counter",
            VarType::Logic,
            Direction::Implicit,
            SignalKind::Bits {
                width: 8,
                states: 2,
            },
        )
        .unwrap();
    let stream = w
        .add_scope(Some(cpu), "thread0", vtr::ScopeType::Generic, "")
        .unwrap();
    let insn = w
        .add_generator(Some(stream), "instruction", "PIPELINE")
        .unwrap();
    let bus = w
        .add_scope(Some(cpu), "bus", vtr::ScopeType::Generic, "")
        .unwrap();
    let req = w.add_generator(Some(bus), "request", "TX").unwrap();
    let last = n + 4;
    for t in 0..=last {
        w.set_time(t).unwrap();
        w.emit_u64(counter, t % 256).unwrap();
    }
    let label = w.intern("vtr.label");
    let lane0 = w.intern("0");
    let lane1 = w.intern("1");
    let stl = w.intern("stl");
    let names: Vec<_> = stages.iter().map(|s| w.intern(s)).collect();
    for i in 0..n {
        let tx = w.begin_tx(insn, i).unwrap();
        let caption = format!("{:08x}: op{i}", 0x1000 + 4 * i);
        let value = if i == 0 {
            Value::Text(caption)
        } else {
            Value::Str(w.intern(&caption))
        };
        w.tx_attr(tx, label, &value).unwrap();
        let flushed = i == 3;
        let open = i + 1 == n;
        let count = if flushed { 2 } else { 3 };
        for (k, name) in names.iter().enumerate().take(count) {
            let (b, e) = (i + k as u64, i + k as u64 + 1);
            if open && k + 1 == count {
                w.tx_stage_begin(tx, *name, lane0, b).unwrap();
            } else {
                w.tx_stage(tx, *name, lane0, b, e, &[]).unwrap();
            }
        }
        if i % 7 == 0 {
            w.tx_stage(tx, stl, lane1, i + 1, i + 3, &[]).unwrap();
        }
        if flushed {
            w.end_tx(tx, i + 2, TxStatus::Aborted).unwrap();
        } else if !open {
            w.end_tx(tx, i + 3, TxStatus::Unset).unwrap();
        }
    }
    let tx = w.begin_tx(req, 2).unwrap();
    w.end_tx(tx, 9, TxStatus::Ok).unwrap();
    w.set_time(last).unwrap();
    w.close().unwrap();
    OpenSpec::Path(file.path().into()).open().unwrap()
}

fn scope(session: &dyn Session, path: &[&str]) -> usize {
    match session.hierarchy().find_scope(path) {
        Lookup::Found(id) => id,
        other => panic!("{other:?}"),
    }
}

fn generator_track(session: &dyn Session, path: &[&str]) -> TrackRef {
    let h = session.hierarchy();
    h.member_track(Member::Generator(
        session
            .hierarchy()
            .scope(scope(session, path))
            .generators
            .iter()
            .next()
            .unwrap(),
    ))
    .unwrap()
}

fn pump(app: &mut App) {
    loop {
        let requests = app.take_requests();
        if requests.is_empty() {
            return;
        }
        for r in requests {
            app.deliver(r.perform());
        }
    }
}

const BOUNDS: Rect = Rect::from_xywh(0.0, 0.0, 1200.0, 600.0);

fn frame(app: &mut App, id: PanelId, theme: &Theme) {
    app.layout_panel(id, BOUNDS, theme, 1.0).unwrap();
    app.render_panel(id, theme, &mut MonoMeasure);
}

fn opened(n: u64) -> (App, Arc<dyn Session>, PanelId, PanelId) {
    let session = fixture(n);
    let mut app = App::new();
    app.set_session(session.clone());
    app.handle(Command::AddVars(a_all(vec![0])));
    let waves = app.panels.focused_id();
    assert!(app.panels.waves(waves).is_some());
    app.handle(Command::SelectScope(TreeNode::scope(a(scope(
        session.as_ref(),
        &["cpu", "thread0"],
    )))));
    app.take_events();
    app.handle(Command::SelectVar {
        ix: 0,
        modifiers: Modifiers::default(),
    });
    app.handle(Command::VariablesKey(Key::Enter, Modifiers::default()));
    let pipeline = app.panels.focused_id();
    assert_ne!(pipeline, waves);
    app.handle(Command::PipelineActivity(
        pipeline,
        volna_core::pipeline::ActivityCommand::Toggle,
    ));
    (app, session, waves, pipeline)
}

#[test]
fn activity_follows_cursor_then_view_center_and_preserves_other_axes() {
    use volna_core::pipeline::{ActivityCommand, FollowActivity};
    use volna_core::wave::viewport::Viewport;
    let (mut app, _, waves, pipeline) = opened(1000);
    pump(&mut app);
    let theme = Theme::one_dark();
    app.doc.shared.viewport.set(Viewport {
        start: 100.0,
        end: 900.0,
    });
    app.doc.shared.cursor = Some(200);
    frame(&mut app, pipeline, &theme);
    let row_px = app.panels.pipeline(pipeline).unwrap().rows.value.row_px;
    let wave_rows = app.panels.waves(waves).unwrap().scroll_y;
    app.handle(Command::PipelineActivity(pipeline, ActivityCommand::Toggle));
    frame(&mut app, pipeline, &theme);
    let p = app.panels.pipeline(pipeline).unwrap();
    assert_eq!(p.follow, FollowActivity::Following);
    assert!(p.last_layout().row_range.contains(&200));
    let top = p.rows.value.top;
    frame(&mut app, pipeline, &theme);
    assert_eq!(app.panels.pipeline(pipeline).unwrap().rows.value.top, top);
    app.doc.shared.cursor = Some(1);
    frame(&mut app, pipeline, &theme);
    let p = app.panels.pipeline(pipeline).unwrap();
    assert!(p.last_layout().row_range.contains(&500));
    assert_eq!(p.rows.value.row_px, row_px);
    assert_eq!(app.doc.shared.cursor, Some(1));
    assert_eq!(
        app.doc.shared.viewport.value,
        Viewport {
            start: 100.0,
            end: 900.0
        }
    );
    assert_eq!(app.panels.waves(waves).unwrap().scroll_y, wave_rows);
    app.handle(Command::Panels(PanelsCommand::ToggleLink {
        panel: pipeline,
        dim: LinkDim::Viewport,
    }));
    app.handle(Command::Panels(PanelsCommand::ToggleLink {
        panel: pipeline,
        dim: LinkDim::Cursor,
    }));
    let p = app.panels.pipeline_mut(pipeline).unwrap();
    p.nav.local_viewport.set(Viewport {
        start: 700.0,
        end: 900.0,
    });
    p.nav.local_cursor = Some(750);
    frame(&mut app, pipeline, &theme);
    assert!(
        app.panels
            .pipeline(pipeline)
            .unwrap()
            .last_layout()
            .row_range
            .contains(&750)
    );
    assert_eq!(app.doc.shared.cursor, Some(1));
}

#[test]
fn activity_vertical_override_edges_resume_and_idle_are_explicit() {
    use volna_core::pipeline::{ActivityCommand, FollowActivity};
    use volna_core::wave::viewport::Viewport;
    let (mut app, _, _, pipeline) = opened(1000);
    pump(&mut app);
    let theme = Theme::one_dark();
    app.doc.shared.viewport.set(Viewport {
        start: 400.0,
        end: 450.0,
    });
    app.doc.shared.cursor = None;
    frame(&mut app, pipeline, &theme);
    app.handle(Command::PipelineActivity(pipeline, ActivityCommand::Toggle));
    let area = cells(&mut app, pipeline, &theme);
    let position = point(area.left() + 100.0, area.top() + 100.0);
    let wheel = |dx, dy| {
        Command::Pointer(
            pipeline,
            PointerEvent::Wheel {
                position,
                dx,
                dy,
                precise: true,
                modifiers: Modifiers::default(),
            },
        )
    };
    app.handle(wheel(5.0, 0.0));
    assert_eq!(
        app.panels.pipeline(pipeline).unwrap().follow,
        FollowActivity::Following
    );
    app.handle(wheel(0.0, 2000.0));
    frame(&mut app, pipeline, &theme);
    let p = app.panels.pipeline(pipeline).unwrap();
    assert_eq!(p.follow, FollowActivity::Suspended);
    let top = p.rows.value.top;
    app.doc.shared.cursor = Some(440);
    frame(&mut app, pipeline, &theme);
    assert_eq!(app.panels.pipeline(pipeline).unwrap().rows.value.top, top);
    let p = app.panels.pipeline(pipeline).unwrap();
    assert!(p.activity(&app.doc).below > 0);
    let (_, rect, _) = p
        .last_layout()
        .activity_controls
        .iter()
        .find(|(command, _, _)| *command == ActivityCommand::RevealBelow)
        .unwrap();
    app.handle(Command::Pointer(
        pipeline,
        PointerEvent::Down {
            position: point(rect.left() + 10.0, rect.top() + 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    frame(&mut app, pipeline, &theme);
    let p = app.panels.pipeline(pipeline).unwrap();
    assert_eq!(p.follow, FollowActivity::Suspended);
    assert!(p.activity(&app.doc).visible > 0);
    app.handle(Command::PipelineActivity(pipeline, ActivityCommand::Toggle));
    assert_eq!(
        app.panels.pipeline(pipeline).unwrap().follow,
        FollowActivity::Following
    );
    let top = app.panels.pipeline(pipeline).unwrap().rows.value.top;
    app.doc.shared.viewport.set(Viewport {
        start: 2000.0,
        end: 2100.0,
    });
    frame(&mut app, pipeline, &theme);
    assert_eq!(app.panels.pipeline(pipeline).unwrap().rows.value.top, top);
    let scene = app.render_panel(pipeline, &theme, &mut MonoMeasure);
    assert!(scene.prims.iter().any(|prim| matches!(prim, Prim::Text { text, .. } if text.contains("No pipeline activity") && text.contains('←'))));
}

#[test]
fn activity_state_survives_workspace_and_split() {
    use volna_core::pipeline::FollowActivity;
    for follow in [
        FollowActivity::Off,
        FollowActivity::Following,
        FollowActivity::Suspended,
    ] {
        let (mut app, session, _, pipeline) = opened(100);
        pump(&mut app);
        app.panels.pipeline_mut(pipeline).unwrap().follow = follow;
        assert_eq!(
            app.panels.pipeline(pipeline).unwrap().clone_view().follow,
            follow
        );
        let saved =
            Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();
        let mut restored = App::new();
        restored.set_session(session);
        Workspace::parse(&saved.to_bytes().unwrap())
            .unwrap()
            .prepare(
                &restored,
                "file:///tmp/trace.vtr",
                "file:///tmp/trace.vtr.volna.json",
            )
            .unwrap()
            .commit(&mut restored)
            .unwrap();
        assert_eq!(restored.panels.pipeline(pipeline).unwrap().follow, follow);
    }
}

#[test]
fn activity_keeps_long_overlaps_and_points_in_one_generator() {
    use volna_core::wave::viewport::Viewport;
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let stream = writer
        .add_scope(None, "pipeline", vtr::ScopeType::Generic, "")
        .unwrap();
    let first = writer
        .add_generator(Some(stream), "first", "PIPELINE")
        .unwrap();
    let _second = writer
        .add_generator(Some(stream), "second", "PIPELINE")
        .unwrap();
    let long = writer.begin_tx(first, 0).unwrap();
    writer.end_tx(long, 1000, vtr::TxStatus::Ok).unwrap();
    for i in 1..100 {
        let tx = writer.begin_tx(first, i).unwrap();
        writer.end_tx(tx, i, vtr::TxStatus::Ok).unwrap();
    }
    let point_tx = writer.begin_tx(first, 500).unwrap();
    writer.end_tx(point_tx, 500, vtr::TxStatus::Ok).unwrap();
    writer.set_time(1000).unwrap();
    writer.close().unwrap();
    let session = OpenSpec::Path(file.path().into()).open().unwrap();
    let track = generator_track(session.as_ref(), &["pipeline"]);
    let mut app = App::new();
    app.set_session(session);
    app.handle(Command::OpenPipeline { track: a(track) });
    let panel = app.panels.focused_id();
    app.doc.shared.viewport.set(Viewport {
        start: 500.0,
        end: 500.5,
    });
    app.doc.shared.cursor = Some(500);
    let theme = Theme::one_dark();
    frame(&mut app, panel, &theme);
    pump(&mut app);
    frame(&mut app, panel, &theme);
    let p = app.panels.pipeline(panel).unwrap();
    let activity = p.activity(&app.doc);
    assert_eq!(activity.visible + activity.above + activity.below, 2);
    assert_eq!(
        activity.target,
        Some(0),
        "nearest visible overlap stays put"
    );
    assert_eq!(
        activity.nearest_below,
        Some(100),
        "point follows the earlier rows"
    );
    app.doc.shared.viewport.set(Viewport {
        start: 500.1,
        end: 500.9,
    });
    frame(&mut app, panel, &theme);
    let activity = app.panels.pipeline(panel).unwrap().activity(&app.doc);
    assert_eq!(activity.visible + activity.above + activity.below, 1);
    assert_eq!(
        activity.target,
        Some(0),
        "fractional windows still contain long lifetimes"
    );
    app.doc.shared.viewport.set(Viewport {
        start: -2.0,
        end: -1.0,
    });
    frame(&mut app, panel, &theme);
    let activity = app.panels.pipeline(panel).unwrap().activity(&app.doc);
    assert_eq!(activity.target, None);
    assert!(activity.later);
}

fn cells(app: &mut App, id: PanelId, theme: &Theme) -> Rect {
    match app.layout_panel(id, BOUNDS, theme, 1.0).unwrap() {
        PanelLayout::Pipeline(l) => l.cells,
        PanelLayout::Waves(_) | PanelLayout::Table(_) => panic!("not a pipeline panel"),
    }
}

#[test]
fn narrow_pipeline_stages_show_the_prefix_that_fits() {
    use volna_core::wave::viewport::Viewport;

    let session = fixture_with_stages(2, &["fetch", "decode", "execute"]);
    let mut app = App::new();
    app.set_session(session.clone());
    pump(&mut app);
    app.handle(Command::ActivateMembers(a_all(vec![Member::Generator(
        session
            .hierarchy()
            .scope(scope(session.as_ref(), &["cpu", "thread0"]))
            .generators
            .iter()
            .next()
            .unwrap(),
    )])));
    pump(&mut app);
    let panel = app.panels.focused_id();
    let theme = Theme::one_dark();
    frame(&mut app, panel, &theme);
    let layout = app.panels.pipeline(panel).unwrap().last_layout();
    let row_px = layout.rows.row_px;
    let font_size = (row_px * 0.62).clamp(6.0, theme.mono_size);
    let char_width = font_size * 0.6;
    let gap = if row_px >= 8.0 { 3.0 } else { 1.0 };
    let cells_width = layout.cells.width();

    for (chars, expected) in [
        (0, vec![]),
        (1, vec!["f", "d", "e"]),
        (2, vec!["fe", "de", "ex"]),
        (8, vec!["fetch", "decode", "execute"]),
    ] {
        let cell_width = gap + 4.0 + (chars as f32 + 0.5) * char_width;
        app.panels
            .pipeline_mut(panel)
            .unwrap()
            .nav
            .viewport_state_mut(&mut app.doc)
            .set(Viewport {
                start: 0.0,
                end: f64::from(cells_width / cell_width),
            });
        frame(&mut app, panel, &theme);
        let layout = app.panels.pipeline(panel).unwrap().last_layout();
        let cells = layout.cells;
        let first_row_y = layout.row_y(0);
        let labels: Vec<_> = app
            .scene()
            .prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::Text {
                    origin,
                    text,
                    font: FontRole::Mono,
                    ..
                } if cells.contains(*origin) && origin.y == first_row_y => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, expected, "space for {chars} characters");
    }
}

#[test]
fn enter_on_a_generator_opens_a_panel_loads_the_track_once_and_paints_cells() {
    let (mut app, session, waves, pipeline) = opened(40);
    assert!(app.status().sidebar_notice.is_none());
    assert!(
        !app.take_events()
            .iter()
            .any(|e| matches!(e, Event::Notice(_)))
    );
    let requests = app.take_requests();
    let track = generator_track(session.as_ref(), &["cpu", "thread0"]);
    let tracks: Vec<_> = requests
        .iter()
        .filter_map(|r| match r {
            LoadRequest::Track { track, .. } => Some(*track),
            _ => None,
        })
        .collect();
    assert_eq!(tracks, [track], "the wave row's signals and one track load");
    let theme = Theme::one_dark();
    frame(&mut app, pipeline, &theme);
    assert!(matches!(
        app.panels.pipeline(pipeline).unwrap().rows(&app.doc),
        Rows::Loading
    ));
    assert!(
        app.scene()
            .texts()
            .any(|t| t.starts_with("Loading cpu.thread0"))
    );
    for r in requests {
        app.deliver(r.perform());
    }
    pump(&mut app);
    frame(&mut app, pipeline, &theme);
    let p = app.panels.pipeline(pipeline).unwrap();
    let Rows::Ready(set) = p.rows(&app.doc) else {
        panic!("loaded")
    };
    assert_eq!(set.len(), 40);
    assert_eq!(PipelineModel::label(set.get(0).unwrap().1), "00001000: op0");
    assert_eq!(PipelineModel::label(set.get(1).unwrap().1), "00001004: op1");
    assert_eq!(p.palette().names(), STAGES);
    let fills: Vec<_> = STAGES
        .iter()
        .map(|s| p.palette().style(s, &theme).fill)
        .collect();
    let layout = p.last_layout().clone();
    let viewport = p.nav.viewport(&app.doc);
    let (mut expected, mut hollow) = (0, 0);
    for r in layout.row_range.clone() {
        let (_, tx) = set.get(r).unwrap();
        let visible = tx
            .stages
            .iter()
            .filter(|s| s.lane == "0")
            .filter(|s| {
                let end = s.end.unwrap_or(tx.end).max(s.begin);
                (end as f64) > viewport.start && (s.begin as f64) < viewport.end
            })
            .count();
        if tx.status == vtr::TxStatus::Aborted {
            hollow += visible;
        } else {
            expected += visible;
        }
    }
    let painted = app
        .scene()
        .quads()
        .filter(|(rect, color)| fills.contains(color) && layout.cells.contains(rect.origin))
        .count();
    assert_eq!(painted, expected);
    let outlined = app
        .scene()
        .prims
        .iter()
        .filter(|p| {
            matches!(p, volna_core::scene::Prim::Quad { rect, fill, border_width, .. }
                if fill.a == 0.0 && *border_width == 1.0 && layout.cells.contains(rect.origin))
        })
        .count();
    assert_eq!(outlined, hollow);
    let bands = app
        .scene()
        .quads()
        .filter(|(rect, color)| {
            *color == theme.editor.text.with_alpha(0.28) && layout.cells.contains(rect.origin)
        })
        .count();
    let overlays = layout
        .row_range
        .filter(|r| set.get(*r).unwrap().1.stages.iter().any(|s| s.lane == "1"))
        .count();
    assert_eq!(bands, overlays);
    assert!(app.scene().texts().any(|t| t == "00001004: op1"));
    assert!(
        app.scene().texts().any(|t| t == "cycle"),
        "the producer's unit"
    );

    let generator = Member::Generator(0);
    app.handle(Command::ActivateMembers(a_all(vec![generator])));
    assert_eq!(app.panels.len(), 3);
    assert!(app.take_requests().is_empty());
    let second = app.panels.focused_id();
    frame(&mut app, second, &theme);
    assert!(matches!(
        app.panels.pipeline(second).unwrap().rows(&app.doc),
        Rows::Ready(set) if set.len() == 40
    ));
    app.handle(Command::ActivateMembers(a_all(vec![Member::Generator(
        session
            .hierarchy()
            .scope(scope(session.as_ref(), &["cpu", "thread0"]))
            .generators
            .iter()
            .next()
            .unwrap(),
    )])));
    assert_eq!(app.panels.len(), 3);
    assert_eq!(app.panels.focused_id(), pipeline);
    let bus = scope(session.as_ref(), &["cpu", "bus"]);
    app.handle(Command::ActivateMembers(a_all(vec![Member::Generator(
        session
            .hierarchy()
            .scope(bus)
            .generators
            .iter()
            .next()
            .unwrap(),
    )])));
    pump(&mut app);
    let bus_panel = app.panels.focused_id();
    assert_eq!(app.panels.len(), 4);
    frame(&mut app, bus_panel, &theme);
    let grey = theme.stage_fallback().fill;
    assert_eq!(app.scene().quads().filter(|(_, c)| *c == grey).count(), 1);
    assert!(app.panels.waves(waves).is_some());
    let state = app.debug_state();
    assert!(state.contains("pipeline track=cpu.thread0"));
    assert!(state.contains("ready rows=40"));
}

#[test]
fn zoom_about_the_pointer_keeps_time_and_row_at_both_interface_zooms() {
    for zoom in [1.0f32, 2.0] {
        let (mut app, _, _, pipeline) = opened(200);
        pump(&mut app);
        let theme = Theme::one_dark().zoomed(zoom);
        app.panels
            .pipeline_mut(pipeline)
            .unwrap()
            .rows
            .set(RowView {
                top: 0.0,
                row_px: 8.0,
            });
        let cells = cells(&mut app, pipeline, &theme);
        frame(&mut app, pipeline, &theme);
        let pointer = point(cells.left() + cells.width() * 0.3, cells.top() + 137.0);
        let before = {
            let p = app.panels.pipeline(pipeline).unwrap();
            let l = p.last_layout();
            (
                l.time_at(&p.nav.viewport(&app.doc), pointer.x),
                l.rows.row_at(pointer.y - l.cells.top()),
                l.rows.row_px,
                p.nav.viewport(&app.doc).width(),
            )
        };
        app.handle(Command::Pointer(
            pipeline,
            PointerEvent::Wheel {
                position: pointer,
                dx: 0.0,
                dy: 120.0,
                modifiers: Modifiers {
                    control: true,
                    ..Modifiers::default()
                },
                precise: true,
            },
        ));
        assert!(!app.is_animating());
        frame(&mut app, pipeline, &theme);
        let after = {
            let p = app.panels.pipeline(pipeline).unwrap();
            let l = p.last_layout();
            (
                l.time_at(&p.nav.viewport(&app.doc), pointer.x),
                l.rows.row_at(pointer.y - l.cells.top()),
                l.rows.row_px,
            )
        };
        assert!((before.0 - after.0).abs() < 1e-6, "time under the pointer");
        assert!((before.1 - after.1).abs() < 1e-6, "row under the pointer");
        let cycle_px = f64::from(cells.width()) / before.3;
        let nearest = (cycle_px * f64::from(before.2)).sqrt();
        let expected = (nearest * 2.0).min(24.0 * f64::from(zoom));
        assert!((f64::from(after.2) - expected).abs() < 1e-4);
        assert_square_cells(&mut app, pipeline, &theme, 1.0);
        assert_eq!(
            app.panels.pipeline(pipeline).unwrap().rows.value.row_px,
            after.2 / zoom,
            "stored at interface zoom 1.0"
        );
        app.handle(Command::Pointer(
            pipeline,
            PointerEvent::Pinch {
                position: pointer,
                delta: -0.5,
            },
        ));
        assert!(!app.is_animating());
        frame(&mut app, pipeline, &theme);
        let p = app.panels.pipeline(pipeline).unwrap();
        let l = p.last_layout();
        assert!((l.time_at(&p.nav.viewport(&app.doc), pointer.x) - before.0).abs() < 1e-6);
        assert!((l.rows.row_at(pointer.y - l.cells.top()) - before.1).abs() < 1e-6);
    }
}

#[test]
fn linked_navigation_moves_the_wave_panel_and_unlinked_navigation_does_not() {
    let (mut app, _, waves, pipeline) = opened(60);
    pump(&mut app);
    let theme = Theme::one_dark();
    let cells = cells(&mut app, pipeline, &theme);
    frame(&mut app, pipeline, &theme);
    let shared_before = app.doc.shared.viewport.value;
    let now = Instant::now();
    let wheel = |app: &mut App, now| {
        app.handle_at(
            Command::Pointer(
                pipeline,
                PointerEvent::Wheel {
                    position: point(cells.left() + 200.0, cells.top() + 50.0),
                    dx: 0.0,
                    dy: 60.0,
                    modifiers: Modifiers {
                        platform: true,
                        ..Modifiers::default()
                    },
                    precise: false,
                },
            ),
            now,
        );
    };
    wheel(&mut app, now);
    let shared_after = app.doc.shared.viewport.value;
    assert_ne!(shared_before, shared_after);
    assert_eq!(
        app.panels.waves(waves).unwrap().viewport(&app.doc),
        shared_after
    );
    app.handle(Command::Panels(PanelsCommand::ToggleLink {
        panel: pipeline,
        dim: LinkDim::Viewport,
    }));
    frame(&mut app, pipeline, &theme);
    let cells = app.panels.pipeline(pipeline).unwrap().last_layout().cells;
    wheel(&mut app, now);
    assert_eq!(
        app.doc.shared.viewport.value, shared_after,
        "shared unchanged"
    );
    let local = app
        .panels
        .pipeline(pipeline)
        .unwrap()
        .nav
        .viewport(&app.doc);
    assert_ne!(local, shared_after);
    let rows_before = app.panels.pipeline(pipeline).unwrap().rows.value;
    app.handle_at(
        Command::Pointer(
            pipeline,
            PointerEvent::Wheel {
                position: point(cells.left() + 200.0, cells.top() + 50.0),
                dx: 40.0,
                dy: -30.0,
                modifiers: Modifiers::default(),
                precise: true,
            },
        ),
        now,
    );
    assert!(!app.is_animating());
    let p = app.panels.pipeline(pipeline).unwrap();
    assert!(
        p.nav.viewport(&app.doc).start < local.start,
        "content follows the fingers"
    );
    assert!(p.rows.value.top > rows_before.top);
    assert_eq!(p.rows.value.row_px, rows_before.row_px);
    let viewport_before = p.nav.viewport(&app.doc);
    let anchor_x = cells.left() + 200.0;
    let time_before = p.last_layout().time_at(&viewport_before, anchor_x);
    let rows_before = p.rows.value;
    app.panels.pipeline_mut(pipeline).unwrap().follow =
        volna_core::pipeline::FollowActivity::Following;
    app.handle_at(
        Command::Pointer(
            pipeline,
            PointerEvent::Wheel {
                position: point(anchor_x, cells.top() - 10.0),
                dx: 0.0,
                dy: 120.0,
                modifiers: Modifiers {
                    control: true,
                    ..Modifiers::default()
                },
                precise: true,
            },
        ),
        now,
    );
    let p = app.panels.pipeline(pipeline).unwrap();
    let viewport_after = p.nav.viewport(&app.doc);
    let side = ((f64::from(cells.width()) / viewport_before.width()
        * f64::from(rows_before.row_px))
    .sqrt()
        * 2.0)
        .min(24.0);
    assert!((f64::from(cells.width()) / viewport_after.width() - side).abs() < 1e-6);
    assert!(
        (p.last_layout().time_at(&viewport_after, anchor_x) - time_before).abs() < 1e-6,
        "time under the pointer"
    );
    assert!((f64::from(p.rows.value.row_px) - side).abs() < 1e-5);
    assert_eq!(p.follow, volna_core::pipeline::FollowActivity::Suspended);
    let rows_before = p.rows.value;
    app.handle_at(Command::Action(Action::MoveSelectionDown), now);
    app.tick(now + Duration::from_secs(1));
    let p = app.panels.pipeline(pipeline).unwrap();
    assert!((p.rows.value.top - (rows_before.top + 3.0)).abs() < 1.0);
    let width = p.nav.viewport(&app.doc).width();
    let row_px = p.rows.value.row_px;
    app.handle_at(Command::Action(Action::ZoomIn), now);
    app.tick(now + Duration::from_secs(1));
    let p = app.panels.pipeline(pipeline).unwrap();
    let expected = (width / 2.0).max(f64::from(cells.width()) / 24.0);
    assert!((p.nav.viewport(&app.doc).width() - expected).abs() < 1e-6);
    assert!((p.rows.value.row_px - (row_px * 2.0).min(24.0)).abs() < 1e-3);
}

fn wheel_with(app: &mut App, id: PanelId, position: Point, dx: f32, dy: f32, modifiers: Modifiers) {
    app.handle(Command::Pointer(
        id,
        PointerEvent::Wheel {
            position,
            dx,
            dy,
            modifiers,
            precise: false,
        },
    ));
}

fn ctrl() -> Modifiers {
    Modifiers {
        control: true,
        ..Modifiers::default()
    }
}

fn box_pointer(app: &mut App, id: PanelId, event: PointerEvent, now: Instant) {
    app.handle_at(Command::Pointer(id, event), now);
}

fn box_press(app: &mut App, id: PanelId, position: Point, modifiers: Modifiers, now: Instant) {
    box_pointer(
        app,
        id,
        PointerEvent::Down {
            position,
            button: MouseButton::Left,
            modifiers,
        },
        now,
    );
}

fn box_view(theme: &Theme) -> (App, PanelId, Rect) {
    let (mut app, _, _, id) = opened(1000);
    pump(&mut app);
    app.doc
        .shared
        .viewport
        .set(volna_core::wave::viewport::Viewport {
            start: 200.0,
            end: 400.0,
        });
    app.panels.pipeline_mut(id).unwrap().rows.set(RowView {
        top: 200.0,
        row_px: 6.0,
    });
    frame(&mut app, id, theme);
    let cells = app.panels.pipeline(id).unwrap().last_layout().cells;
    (app, id, cells)
}

fn in_box(cells: Rect, x: f32, y: f32) -> Point {
    point(
        cells.left() + cells.width() * x,
        cells.top() + cells.height() * y,
    )
}

#[test]
fn box_zoom_fits_uniformly_in_every_direction_and_preserves_selection() {
    use volna_core::settings::Animation;
    for zoom in [1.0, 2.0] {
        let theme = Theme::one_dark().zoomed(zoom);
        for platform in [false, true] {
            for (reverse_x, reverse_y) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                let (mut app, id, cells) = box_view(&theme);
                app.doc.navigation.animation = Animation::Off;
                app.panels
                    .pipeline_mut(id)
                    .unwrap()
                    .select_row(&mut app.doc, id, 210);
                app.doc.shared.cursor = Some(211);
                let selected = app.doc.selection();
                let undo = app.undo_label().map(str::to_owned);
                let rows = app.panels.pipeline(id).unwrap().rows.value.zoomed(zoom);
                let time = app.doc.shared.viewport.value;
                let now = Instant::now();
                let start = in_box(
                    cells,
                    if reverse_x { 0.45 } else { 0.25 },
                    if reverse_y { 0.7 } else { 0.3 },
                );
                let end = in_box(
                    cells,
                    if reverse_x { 0.25 } else { 0.45 },
                    if reverse_y { 0.3 } else { 0.7 },
                );
                box_press(
                    &mut app,
                    id,
                    start,
                    Modifiers {
                        control: !platform,
                        platform,
                        ..Default::default()
                    },
                    now,
                );
                box_pointer(&mut app, id, PointerEvent::Move { position: end }, now);
                assert_eq!(app.doc.shared.viewport.value, time);
                assert_eq!(
                    app.panels.pipeline(id).unwrap().rows.value.zoomed(zoom),
                    rows
                );
                box_pointer(&mut app, id, PointerEvent::Up, now);
                let p = app.panels.pipeline(id).unwrap();
                let target = p.nav.viewport(&app.doc);
                let rows_target = p.rows.value.zoomed(zoom);
                assert!((time.width() / target.width() - 2.5).abs() < 1e-5);
                assert!((f64::from(rows_target.row_px / rows.row_px) - 2.5).abs() < 1e-5);
                assert!((target.start.midpoint(target.end) - 270.0).abs() < 1e-5);
                assert!(
                    (rows_target.row_at(cells.height() * 0.5) - rows.row_at(cells.height() * 0.5))
                        .abs()
                        < 1e-5
                );
                assert_eq!(app.doc.selection(), selected);
                assert_eq!(app.doc.shared.cursor, Some(211));
                assert_eq!(p.row_cap, 24.0);
                assert!(!p.is_animating());
                assert_eq!(
                    app.undo_label(),
                    undo.as_deref(),
                    "navigation creates no undo step"
                );
            }
        }
    }
}

#[test]
fn box_zoom_stops_both_axes_at_the_first_cap() {
    use volna_core::wave::viewport::Viewport;
    for zoom in [1.0, 2.0] {
        let theme = Theme::one_dark().zoomed(zoom);
        for (width, row_px, cap) in [
            (200.0, 18.0, 24.0),
            (12.0, 6.0, 24.0),
            (5.0, 6.0, 24.0),
            (200.0, 30.0, 24.0),
            (200.0, 10.0, 40.0),
        ] {
            let (mut app, id, cells) = box_view(&theme);
            app.doc.shared.viewport.set(Viewport {
                start: 200.0,
                end: 200.0 + width,
            });
            let p = app.panels.pipeline_mut(id).unwrap();
            p.rows.set(RowView { top: 200.0, row_px });
            p.row_cap = cap;
            frame(&mut app, id, &theme);
            let expected = 4.0_f64
                .min((120.0 * f64::from(zoom) * width / f64::from(cells.width())).max(1.0))
                .min(f64::from(cap / row_px).max(1.0));
            let now = Instant::now();
            box_press(&mut app, id, in_box(cells, 0.375, 0.375), ctrl(), now);
            box_pointer(
                &mut app,
                id,
                PointerEvent::Move {
                    position: in_box(cells, 0.625, 0.625),
                },
                now,
            );
            box_pointer(&mut app, id, PointerEvent::Up, now);
            let p = app.panels.pipeline(id).unwrap();
            assert!(
                (width / p.nav.viewport_state(&app.doc).target().width() - expected).abs() < 1e-5
            );
            assert!((f64::from(p.rows.target().row_px / row_px) - expected).abs() < 1e-5);
            assert_eq!(p.row_cap, cap);
        }
    }
}

#[test]
fn box_zoom_clips_the_overlay_and_fits_the_same_rectangle_on_outside_release() {
    for theme in [
        Theme::one_dark(),
        Theme::volna(false),
        Theme::one_dark().zoomed(2.0),
    ] {
        let (mut app, id, cells) = box_view(&theme);
        let now = Instant::now();
        let start = in_box(cells, 0.5, 0.5);
        let end = point(BOUNDS.right() + 100.0, BOUNDS.bottom() + 100.0);
        box_press(&mut app, id, start, ctrl(), now);
        box_pointer(&mut app, id, PointerEvent::Move { position: end }, now);
        frame(&mut app, id, &theme);
        let expected_rect =
            Rect::from_xywh(start.x, start.y, cells.width() * 0.5, cells.height() * 0.5);
        let index = app.scene().prims.iter().position(|prim| matches!(prim, Prim::Quad { rect, fill, border_color, border_width, .. }
            if *rect == expected_rect && *fill == theme.selection.bg.with_alpha(0.35) && *border_color == theme.editor.text && *border_width == 1.0)).expect("the themed selection box");
        assert_eq!(app.scene().prims[index - 1], Prim::PushClip(cells));
        assert!(
            matches!(&app.scene().prims[index + 1], Prim::Text { text, color, .. } if text == "Zoom to selected area · Esc to cancel" && *color == theme.editor.text)
        );
        assert_eq!(app.scene().prims[index + 2], Prim::PopClip);
        box_pointer(&mut app, id, PointerEvent::Leave, now);
        assert!(app.panels.get(id).unwrap().dragging());
        box_pointer(&mut app, id, PointerEvent::Up, now);
        let p = app.panels.pipeline(id).unwrap();
        let target = p.nav.viewport_state(&app.doc).target();
        assert!((target.start - 300.0).abs() < 1e-5);
        assert!((target.end - 400.0).abs() < 1e-5);
        assert!((p.rows.target().row_px - 12.0).abs() < 1e-5);
        assert!(!app.panels.get(id).unwrap().dragging());
    }
}

#[test]
fn box_zoom_tiny_boxes_escape_and_resizing_cancel_without_clicking() {
    for zoom in [1.0, 2.0] {
        let theme = Theme::one_dark().zoomed(zoom);
        for (dx, dy, cancel) in [
            (0.0, 0.0, 0),
            (3.9, 40.0, 0),
            (40.0, 3.9, 0),
            (40.0, 40.0, 1),
            (40.0, 40.0, 2),
            (40.0, 40.0, 3),
        ] {
            let (mut app, id, cells) = box_view(&theme);
            app.panels
                .pipeline_mut(id)
                .unwrap()
                .select_row(&mut app.doc, id, 210);
            app.doc.shared.cursor = Some(211);
            let selected = app.doc.selection();
            let before = app.doc.shared.viewport.value;
            let rows = app.panels.pipeline(id).unwrap().rows.value;
            let now = Instant::now();
            let start = in_box(cells, 0.5, 0.5);
            box_press(&mut app, id, start, ctrl(), now);
            box_pointer(
                &mut app,
                id,
                PointerEvent::Move {
                    position: point(start.x + dx * zoom, start.y + dy * zoom),
                },
                now,
            );
            match cancel {
                1 => app.handle_at(Command::Action(Action::ClearSelection), now),
                2 => {
                    app.layout_panel(id, Rect::from_xywh(30.0, 20.0, 1200.0, 600.0), &theme, 1.0)
                        .unwrap();
                }
                3 => {
                    app.layout_panel(id, BOUNDS, &theme.zoomed(zoom * 1.1), 1.0)
                        .unwrap();
                }
                _ => {}
            }
            box_pointer(&mut app, id, PointerEvent::Up, now);
            assert_eq!(app.doc.shared.viewport.target(), before);
            assert_eq!(app.panels.pipeline(id).unwrap().rows.target(), rows);
            assert_eq!(app.doc.selection(), selected);
            assert_eq!(app.doc.shared.cursor, Some(211));
        }
    }
}

#[test]
fn box_zoom_freezes_displayed_animation_and_follow_then_animates_both_targets() {
    use volna_core::pipeline::FollowActivity;
    use volna_core::settings::Animation;
    use volna_core::wave::viewport::Viewport;
    let theme = Theme::one_dark();
    let (mut app, id, cells) = box_view(&theme);
    app.doc.navigation.animation = Animation::On;
    let now = Instant::now();
    app.doc.shared.viewport.animate_to(
        Viewport {
            start: 600.0,
            end: 700.0,
        },
        now,
        Animation::On,
    );
    app.panels.pipeline_mut(id).unwrap().rows.animate_to(
        RowView {
            top: 600.0,
            row_px: 12.0,
        },
        now,
        Animation::On,
    );
    app.tick(now + Duration::from_millis(30));
    frame(&mut app, id, &theme);
    let before = app.doc.shared.viewport.value;
    let rows = app.panels.pipeline(id).unwrap().last_layout().rows;
    app.panels.pipeline_mut(id).unwrap().follow = FollowActivity::Following;
    let press_time = now + Duration::from_millis(30);
    box_press(&mut app, id, in_box(cells, 0.25, 0.25), ctrl(), press_time);
    assert_eq!(app.doc.shared.viewport.target(), before);
    assert_eq!(app.panels.pipeline(id).unwrap().rows.target(), rows);
    for event in [
        PointerEvent::Wheel {
            position: in_box(cells, 0.5, 0.5),
            dx: 100.0,
            dy: 120.0,
            modifiers: ctrl(),
            precise: false,
        },
        PointerEvent::Pinch {
            position: in_box(cells, 0.5, 0.5),
            delta: 1.0,
        },
    ] {
        box_pointer(&mut app, id, event, press_time);
    }
    app.doc.shared.cursor = Some(before.start as u64);
    app.tick(now + Duration::from_secs(1));
    frame(&mut app, id, &theme);
    assert_eq!(app.doc.shared.viewport.value, before);
    assert_eq!(app.panels.pipeline(id).unwrap().rows.value, rows);
    assert_eq!(
        app.panels.pipeline(id).unwrap().follow,
        FollowActivity::Following
    );
    box_pointer(
        &mut app,
        id,
        PointerEvent::Move {
            position: in_box(cells, 0.75, 0.75),
        },
        now,
    );
    let release_time = now + Duration::from_secs(1);
    box_pointer(&mut app, id, PointerEvent::Up, release_time);
    let p = app.panels.pipeline(id).unwrap();
    assert_eq!(p.follow, FollowActivity::Suspended);
    assert_eq!(p.rows.value, rows);
    assert_eq!(app.doc.shared.viewport.value, before);
    let target = app.doc.shared.viewport.target();
    let rows_target = p.rows.target();
    let factor = before.width() / target.width();
    assert!((f64::from(rows_target.row_px / rows.row_px) - factor).abs() < 1e-5);
    assert!(app.doc.shared.viewport.is_animating() && p.rows.is_animating());
    app.tick(release_time + Duration::from_secs(1));
    assert_eq!(app.doc.shared.viewport.value, target);
    assert_eq!(app.panels.pipeline(id).unwrap().rows.value, rows_target);
}

#[test]
fn box_zoom_release_schedules_autosave_for_linked_and_local_targets() {
    use volna_core::workspace::persistence::{IDLE, Persistence, Target};
    for linked in [true, false] {
        let theme = Theme::one_dark();
        let (mut app, id, cells) = box_view(&theme);
        if !linked {
            app.handle(Command::Panels(PanelsCommand::ToggleLink {
                panel: id,
                dim: LinkDim::Viewport,
            }));
        }
        app.configure_persistence(Persistence::Auto);
        app.workspace
            .scheduler
            .begin(
                Some(Target::Storage {
                    key: "box-zoom-test".into(),
                }),
                None,
            )
            .unwrap();
        let shared = app.doc.shared.viewport.target();
        let now = Instant::now();
        box_press(&mut app, id, in_box(cells, 0.25, 0.25), ctrl(), now);
        let revision = app.workspace.scheduler.revision();
        box_pointer(
            &mut app,
            id,
            PointerEvent::Move {
                position: in_box(cells, 0.75, 0.75),
            },
            now,
        );
        assert_eq!(
            app.workspace.scheduler.revision(),
            revision,
            "the overlay changes no persistent fields"
        );
        box_pointer(&mut app, id, PointerEvent::Up, now);
        assert_eq!(app.workspace.scheduler.revision(), revision + 1);
        assert!(app.workspace.scheduler.dirty());
        assert!(
            app.workspace
                .scheduler
                .next(now + IDLE * 2, true, false)
                .is_none()
        );
        let ticket = app
            .workspace
            .scheduler
            .next(now + IDLE * 4, false, false)
            .expect("release schedules a save after animation settles");
        assert_eq!(ticket.revision, revision + 1);
        let p = app.panels.pipeline(id).unwrap();
        let target = p.nav.viewport_state(&app.doc).target();
        assert!((target.width() - 100.0).abs() < 1e-5);
        assert_eq!(
            app.doc.shared.viewport.target(),
            if linked { target } else { shared }
        );
        let saved =
            Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();
        let json = serde_json::to_value(saved).unwrap();
        let panel = json["panels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["kind"] == "pipeline")
            .unwrap();
        assert_eq!(panel["rows"]["row_px"], 12.0);
        if !linked {
            assert_eq!(panel["viewport"]["start"], 250.0);
        }
        box_pointer(&mut app, id, PointerEvent::Up, now);
        assert_eq!(
            app.workspace.scheduler.revision(),
            revision + 1,
            "an unchanged release is a no-op"
        );
    }
}

#[test]
fn box_zoom_retains_alt_and_special_region_hit_precedence() {
    use volna_core::pipeline::model::Drag;
    let theme = Theme::one_dark();
    let (mut app, id, cells) = box_view(&theme);
    app.doc.add_marker(300);
    frame(&mut app, id, &theme);
    let now = Instant::now();
    let alt = Modifiers {
        alt: true,
        control: true,
        ..Default::default()
    };
    box_press(&mut app, id, in_box(cells, 0.5, 0.5), alt, now);
    assert!(app.doc.reference_time().is_some());
    assert!(!app.panels.get(id).unwrap().dragging());
    frame(&mut app, id, &theme);
    let layout = app.panels.pipeline(id).unwrap().last_layout().clone();
    box_press(
        &mut app,
        id,
        point(layout.labels.left() + 20.0, layout.labels.top() + 10.0),
        ctrl(),
        now,
    );
    assert!(app.doc.selection().is_some());
    assert!(!app.panels.get(id).unwrap().dragging());
    box_press(&mut app, id, in_box(layout.header, 0.75, 0.5), ctrl(), now);
    assert_eq!(app.panels.pipeline(id).unwrap().drag, Some(Drag::Cursor));
    box_pointer(&mut app, id, PointerEvent::Up, now);
    box_press(
        &mut app,
        id,
        in_box(layout.label_split, 0.5, 0.5),
        ctrl(),
        now,
    );
    assert_eq!(
        app.panels.pipeline(id).unwrap().drag,
        Some(Drag::LabelSplit)
    );
    box_pointer(&mut app, id, PointerEvent::Up, now);
    box_press(
        &mut app,
        id,
        in_box(layout.marker_lane.chips[0].rect, 0.5, 0.5),
        ctrl(),
        now,
    );
    assert!(matches!(
        app.panels.pipeline(id).unwrap().drag,
        Some(Drag::Marker(_))
    ));
    box_pointer(&mut app, id, PointerEvent::Up, now);
    frame(&mut app, id, &theme);
    let control = app
        .panels
        .pipeline(id)
        .unwrap()
        .last_layout()
        .activity_controls[0]
        .1;
    let rows = app.panels.pipeline(id).unwrap().rows.value;
    box_press(&mut app, id, in_box(control, 0.5, 0.5), ctrl(), now);
    assert!(!app.panels.get(id).unwrap().dragging());
    assert_ne!(app.panels.pipeline(id).unwrap().rows.value, rows);
}

#[test]
fn wheel_zoom_restores_squares_after_box_zoom() {
    let theme = Theme::one_dark();
    let (mut app, id, cells) = box_view(&theme);
    let now = Instant::now();
    wheel_with(&mut app, id, in_box(cells, 0.5, 0.5), 0.0, 120.0, ctrl());
    app.doc
        .shared
        .viewport
        .set(volna_core::wave::viewport::Viewport {
            start: 200.0,
            end: 400.0,
        });
    frame(&mut app, id, &theme);
    box_press(&mut app, id, in_box(cells, 0.375, 0.375), ctrl(), now);
    box_pointer(
        &mut app,
        id,
        PointerEvent::Move {
            position: in_box(cells, 0.625, 0.625),
        },
        now,
    );
    box_pointer(&mut app, id, PointerEvent::Up, now);
    app.tick(now + Duration::from_secs(1));
    frame(&mut app, id, &theme);
    let time = app.doc.shared.viewport.value.width();
    let row_px = app.panels.pipeline(id).unwrap().rows.value.row_px;
    assert!((row_px - 24.0).abs() < 1e-5);
    wheel_with(&mut app, id, in_box(cells, 0.5, 0.5), 0.0, -120.0, ctrl());
    let nearest = (f64::from(cells.width()) / time * f64::from(row_px))
        .sqrt()
        .min(24.0);
    assert!(
        (f64::from(app.panels.pipeline(id).unwrap().rows.value.row_px) - nearest * 0.5).abs()
            < 1e-5
    );
    assert_square_cells(&mut app, id, &theme, 1.0);
}

fn scales(app: &mut App, id: PanelId, theme: &Theme) -> (f64, f32) {
    frame(app, id, theme);
    let p = app.panels.pipeline(id).unwrap();
    (
        p.nav.viewport(&app.doc).width(),
        p.last_layout().rows.row_px,
    )
}

fn same_scales(a: (f64, f32), b: (f64, f32)) -> bool {
    (a.0 / b.0 - 1.0).abs() < 1e-6 && (a.1 / b.1 - 1.0).abs() < 1e-4
}

fn assert_square_cells(app: &mut App, id: PanelId, theme: &Theme, period: f64) {
    let (time, row_px) = scales(app, id, theme);
    let width = f64::from(app.panels.pipeline(id).unwrap().last_layout().cells.width());
    let cycle_px = width / time * period;
    assert!(
        (cycle_px / f64::from(row_px) - 1.0).abs() < 1e-5,
        "cycle width {cycle_px} differs from row height {row_px}"
    );
}

#[test]
fn pipeline_wheel_restores_nearest_squares_after_linked_waveform_zoom() {
    use volna_core::wave::viewport::Viewport;
    for zoom in [1.0, 2.0] {
        for wave_delta in [-120.0, 120.0] {
            for pipeline_delta in [-30.0, 30.0] {
                let (mut app, _, waves, pipeline) = opened(1000);
                pump(&mut app);
                let theme = Theme::one_dark().zoomed(zoom);
                let cells = cells(&mut app, pipeline, &theme);
                let side = 8.0 * f64::from(zoom);
                let width = f64::from(cells.width()) / side;
                app.doc.shared.viewport.set(Viewport {
                    start: 500.0 - width * 0.5,
                    end: 500.0 + width * 0.5,
                });
                app.panels
                    .pipeline_mut(pipeline)
                    .unwrap()
                    .rows
                    .set(RowView {
                        top: 200.0,
                        row_px: 8.0,
                    });
                assert_square_cells(&mut app, pipeline, &theme, 1.0);
                frame(&mut app, waves, &theme);
                let wave_area = app.panels.waves(waves).unwrap().last_layout().waves;
                wheel_with(
                    &mut app,
                    waves,
                    in_box(wave_area, 0.5, 0.5),
                    0.0,
                    wave_delta,
                    ctrl(),
                );
                let stretched = scales(&mut app, pipeline, &theme);
                let cycle_px = f64::from(cells.width()) / stretched.0;
                assert!((cycle_px / f64::from(stretched.1) - 1.0).abs() > 0.1);
                let pointer = in_box(cells, 0.5, 0.5);
                let p = app.panels.pipeline(pipeline).unwrap();
                let time = p
                    .nav
                    .viewport(&app.doc)
                    .time_at(f64::from(cells.width()) * 0.5, f64::from(cells.width()));
                let row = p.rows.value.zoomed(zoom).row_at(cells.height() * 0.5);
                let expected = (cycle_px * f64::from(stretched.1)).sqrt()
                    * 2.0f64.powf(f64::from(pipeline_delta) / 120.0);
                wheel_with(&mut app, pipeline, pointer, 0.0, pipeline_delta, ctrl());
                assert_square_cells(&mut app, pipeline, &theme, 1.0);
                let restored = scales(&mut app, pipeline, &theme);
                assert!((f64::from(restored.1) - expected).abs() < 1e-4);
                let p = app.panels.pipeline(pipeline).unwrap();
                assert!(
                    (p.nav
                        .viewport(&app.doc)
                        .time_at(f64::from(cells.width()) * 0.5, f64::from(cells.width()))
                        - time)
                        .abs()
                        < 1e-6
                );
                assert!((p.last_layout().rows.row_at(cells.height() * 0.5) - row).abs() < 1e-5);
                wheel_with(&mut app, pipeline, pointer, 0.0, -pipeline_delta, ctrl());
                assert_square_cells(&mut app, pipeline, &theme, 1.0);
                let next = scales(&mut app, pipeline, &theme);
                let nearest = (cycle_px * f64::from(stretched.1)).sqrt();
                assert!((f64::from(next.1) - nearest).abs() < 1e-4);
            }
        }
    }
}

#[test]
fn animated_square_zoom_keeps_cells_square_and_accumulates_targets() {
    use volna_core::settings::Animation;
    use volna_core::wave::viewport::Viewport;
    for linked in [true, false] {
        let (mut app, _, _, pipeline) = opened(1000);
        pump(&mut app);
        let theme = Theme::one_dark();
        if !linked {
            app.handle(Command::Panels(PanelsCommand::ToggleLink {
                panel: pipeline,
                dim: LinkDim::Viewport,
            }));
        }
        let cells = cells(&mut app, pipeline, &theme);
        app.doc.navigation.animation = Animation::On;
        let p = app.panels.pipeline_mut(pipeline).unwrap();
        let width = f64::from(cells.width()) / 4.0;
        p.nav.viewport_state_mut(&mut app.doc).set(Viewport {
            start: 500.0 - width * 0.5,
            end: 500.0 + width * 0.5,
        });
        p.rows.set(RowView {
            top: 300.0,
            row_px: 4.0,
        });
        assert_square_cells(&mut app, pipeline, &theme, 1.0);
        let now = Instant::now();
        app.handle_at(Command::Action(Action::ZoomIn), now);
        app.tick(now + Duration::from_millis(30));
        assert_square_cells(&mut app, pipeline, &theme, 1.0);
        let intermediate = scales(&mut app, pipeline, &theme).1;
        assert!(intermediate > 4.0 && intermediate < 8.0);
        app.handle_at(
            Command::Action(Action::ZoomIn),
            now + Duration::from_millis(30),
        );
        app.tick(now + Duration::from_millis(60));
        assert_square_cells(&mut app, pipeline, &theme, 1.0);
        app.tick(now + Duration::from_secs(1));
        assert_square_cells(&mut app, pipeline, &theme, 1.0);
        assert!((scales(&mut app, pipeline, &theme).1 - 16.0).abs() < 1e-5);
    }
}

#[test]
fn square_cells_use_the_stream_clock_period_instead_of_time_units() {
    for zoom in [1.0, 2.0] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../volna/examples/pipeline_demo.vtr");
        let session = OpenSpec::Path(path).open().unwrap();
        let mut app = App::new();
        app.set_session(session.clone());
        let stream = scope(session.as_ref(), &["TOP", "tb", "core", "pipeline"]);
        app.handle(Command::ActivateMembers(a_all(vec![Member::Generator(
            session
                .hierarchy()
                .scope(stream)
                .generators
                .iter()
                .next()
                .unwrap(),
        )])));
        pump(&mut app);
        let pipeline = app.panels.focused_id();
        let theme = Theme::one_dark().zoomed(zoom);
        let cells = cells(&mut app, pipeline, &theme);
        let before = scales(&mut app, pipeline, &theme);
        let nearest = (f64::from(cells.width()) / before.0 * 10.0 * f64::from(before.1))
            .sqrt()
            .min(24.0 * f64::from(zoom));
        wheel_with(
            &mut app,
            pipeline,
            in_box(cells, 0.5, 0.5),
            0.0,
            30.0,
            ctrl(),
        );
        assert_square_cells(&mut app, pipeline, &theme, 10.0);
        let expected = (nearest * 2.0f64.powf(0.25)).min(24.0 * f64::from(zoom));
        let actual = scales(&mut app, pipeline, &theme).1;
        assert!(
            (f64::from(actual) - expected).abs() < 1e-4,
            "zoom {zoom}: expected {expected}, got {actual}, before {before:?}, cells {cells:?}"
        );
    }
}

#[test]
fn one_cycle_primary_stage_fills_are_square_at_readable_scales() {
    use volna_core::wave::viewport::Viewport;
    for zoom in [1.0, 2.0] {
        for side in [6.0, 12.0, 24.0] {
            let (mut app, _, _, pipeline) = opened(200);
            pump(&mut app);
            let theme = Theme::one_dark().zoomed(zoom);
            let cells = cells(&mut app, pipeline, &theme);
            let width = f64::from(cells.width()) / f64::from(side * zoom);
            app.doc.shared.viewport.set(Viewport {
                start: 0.0,
                end: width,
            });
            app.panels
                .pipeline_mut(pipeline)
                .unwrap()
                .rows
                .set(RowView {
                    top: 0.0,
                    row_px: side,
                });
            frame(&mut app, pipeline, &theme);
            let p = app.panels.pipeline(pipeline).unwrap();
            let fill = p.palette().style("F", &theme).fill;
            let quads: Vec<_> = app
                .scene()
                .quads()
                .filter(|(rect, color)| {
                    *color == fill
                        && rect.left() > cells.left()
                        && rect.right() < cells.right()
                        && rect.top() > cells.top()
                        && rect.bottom() < cells.bottom()
                })
                .map(|(rect, _)| rect)
                .collect();
            assert!(!quads.is_empty());
            for rect in quads {
                assert!((rect.width() - rect.height()).abs() < 0.001, "{rect:?}");
            }
        }
    }
}

#[test]
fn square_zoom_stops_at_fit_and_retraces_its_scales() {
    use volna_core::wave::viewport::Viewport;
    for zoom in [1.0f32, 2.0] {
        let theme = Theme::one_dark().zoomed(zoom);
        let (mut app, _, _, pipeline) = opened(10_000);
        pump(&mut app);
        let cells = cells(&mut app, pipeline, &theme);
        app.doc.shared.viewport.set(Viewport {
            start: 5000.0,
            end: 5000.0 + f64::from(cells.width()) / (16.0 * f64::from(zoom)),
        });
        app.panels
            .pipeline_mut(pipeline)
            .unwrap()
            .rows
            .set(RowView {
                top: 5000.0,
                row_px: 16.0,
            });
        let pointer = point(cells.left() + cells.width() * 0.4, cells.top() + 100.0);
        let mut path = vec![scales(&mut app, pipeline, &theme)];
        loop {
            wheel_with(&mut app, pipeline, pointer, 0.0, -120.0, ctrl());
            assert_square_cells(&mut app, pipeline, &theme, 1.0);
            let next = scales(&mut app, pipeline, &theme);
            if same_scales(next, *path.last().unwrap()) {
                break;
            }
            path.push(next);
            assert!(path.len() < 64, "zoom-out never ends");
        }
        for pair in path.windows(2).take(path.len() - 2) {
            assert!(same_scales(pair[1], (pair[0].0 * 2.0, pair[0].1 * 0.5)));
        }
        let p = app.panels.pipeline(pipeline).unwrap();
        let layout = p.last_layout();
        let expected_fit = (layout.cells.height() - 24.0 * zoom) / 10_000.0;
        assert!((layout.rows.row_px - expected_fit).abs() < 1e-5);
        assert!(p.nav.viewport(&app.doc).width() > app.doc.limits().1 as f64);
        assert!(layout.row_y(0) >= layout.cells.top() - 0.01);
        assert!(layout.row_y(10_000) <= layout.cells.bottom() + 0.01);
        let full = path.len() - 2;
        app.doc.shared.viewport.set(Viewport {
            start: 5000.0,
            end: 5000.0 + path[full].0,
        });
        app.panels
            .pipeline_mut(pipeline)
            .unwrap()
            .rows
            .set(RowView {
                top: 0.0,
                row_px: path[full].1 / zoom,
            });
        for expected in path[..full].iter().rev() {
            wheel_with(&mut app, pipeline, pointer, 0.0, 120.0, ctrl());
            let actual = scales(&mut app, pipeline, &theme);
            assert!(same_scales(actual, *expected), "{actual:?} != {expected:?}");
        }
    }
}

#[test]
fn square_zoom_stops_both_axes_at_the_row_cap_and_alt_wheel_can_raise_it() {
    for zoom in [1.0f32, 2.0] {
        let (mut app, _, _, pipeline) = opened(200);
        pump(&mut app);
        let theme = Theme::one_dark().zoomed(zoom);
        let cells = cells(&mut app, pipeline, &theme);
        let pointer = point(cells.left() + 300.0, cells.top() + 60.0);
        let mut last = scales(&mut app, pipeline, &theme);
        for _ in 0..40 {
            wheel_with(&mut app, pipeline, pointer, 0.0, 120.0, ctrl());
            last = scales(&mut app, pipeline, &theme);
        }
        let px_per_cycle = f64::from(cells.width()) / last.0;
        assert!((px_per_cycle - 24.0 * f64::from(zoom)).abs() < 1e-6);
        assert!((last.1 - 24.0 * zoom).abs() < 1e-4);
        for _ in 0..3 {
            let modifiers = Modifiers {
                alt: true,
                ..Modifiers::default()
            };
            wheel_with(&mut app, pipeline, pointer, 0.0, 120.0, modifiers);
        }
        let tall = scales(&mut app, pipeline, &theme);
        assert_eq!(tall.0, last.0, "time unchanged");
        assert!((tall.1 - 48.0 * zoom).abs() < 1e-4);
        assert_eq!(app.panels.pipeline(pipeline).unwrap().row_cap, 48.0);
        wheel_with(&mut app, pipeline, pointer, 0.0, -120.0, ctrl());
        let out = scales(&mut app, pipeline, &theme);
        let nearest = (24.0f64 * 48.0).sqrt() * f64::from(zoom);
        assert!((f64::from(out.1) - nearest * 0.5).abs() < 1e-4);
        assert_square_cells(&mut app, pipeline, &theme, 1.0);
        wheel_with(&mut app, pipeline, pointer, 0.0, 120.0, ctrl());
        let restored = scales(&mut app, pipeline, &theme);
        assert!((f64::from(restored.1) - nearest).abs() < 1e-4);
        assert_square_cells(&mut app, pipeline, &theme, 1.0);
        wheel_with(&mut app, pipeline, pointer, 0.0, 120.0, ctrl());
        let tall = scales(&mut app, pipeline, &theme);
        assert!((tall.1 - 48.0 * zoom).abs() < 1e-4);
        assert_square_cells(&mut app, pipeline, &theme, 1.0);
        let now = Instant::now();
        app.handle_at(Command::Action(Action::DecreaseRowHeight), now);
        app.tick(now + Duration::from_secs(1));
        let stepped = scales(&mut app, pipeline, &theme);
        assert!((stepped.1 - 48.0 * zoom / std::f32::consts::SQRT_2).abs() < 1e-3);
        assert_eq!(stepped.0, tall.0);
        app.handle_at(Command::Action(Action::ResetRowHeight), now);
        app.tick(now + Duration::from_secs(1));
        assert!((scales(&mut app, pipeline, &theme).1 - 18.0 * zoom).abs() < 1e-4);
        assert_eq!(app.panels.pipeline(pipeline).unwrap().row_cap, 24.0);
    }
}

#[test]
fn plain_wheel_scrolls_rows_and_shift_or_sideways_wheel_pans_time() {
    let (mut app, _, _, pipeline) = opened(500);
    pump(&mut app);
    let theme = Theme::one_dark();
    let cells = cells(&mut app, pipeline, &theme);
    app.doc
        .shared
        .viewport
        .set(volna_core::wave::viewport::Viewport {
            start: 100.0,
            end: 200.0,
        });
    frame(&mut app, pipeline, &theme);
    let labels = app.panels.pipeline(pipeline).unwrap().last_layout().labels;
    for (position, precise) in [
        (point(cells.left() + 200.0, cells.top() + 50.0), false),
        (point(cells.left() + 200.0, cells.top() + 50.0), true),
        (point(labels.left() + 20.0, labels.top() + 50.0), false),
    ] {
        let before = app.panels.pipeline(pipeline).unwrap().rows.value;
        let viewport = app.doc.shared.viewport.value;
        app.handle(Command::Pointer(
            pipeline,
            PointerEvent::Wheel {
                position,
                dx: 0.0,
                dy: -36.0,
                modifiers: Modifiers::default(),
                precise,
            },
        ));
        let after = app.panels.pipeline(pipeline).unwrap().rows.value;
        assert!((after.top - before.top - 2.0).abs() < 1e-9, "{after:?}");
        assert_eq!(after.row_px, before.row_px);
        assert_eq!(app.doc.shared.viewport.value, viewport);
    }
    let rows = app.panels.pipeline(pipeline).unwrap().rows.value;
    let start = app.doc.shared.viewport.value.start;
    let position = point(cells.left() + 200.0, cells.top() + 50.0);
    wheel_with(
        &mut app,
        pipeline,
        position,
        -40.0,
        0.0,
        Modifiers::default(),
    );
    let panned = app.doc.shared.viewport.value.start;
    assert!(panned > start);
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    wheel_with(&mut app, pipeline, position, 0.0, -40.0, shift);
    assert!(app.doc.shared.viewport.value.start > panned);
    assert_eq!(app.panels.pipeline(pipeline).unwrap().rows.value, rows);
}

#[test]
fn zoom_fit_keeps_square_cells_and_fits_short_and_tall_traces() {
    use volna_core::pipeline::FollowActivity;
    for zoom in [1.0, 2.0] {
        for n in [10u64, 10_000] {
            let (mut app, _, _, pipeline) = opened(n);
            pump(&mut app);
            let theme = Theme::one_dark().zoomed(zoom);
            let cells = cells(&mut app, pipeline, &theme);
            app.panels.pipeline_mut(pipeline).unwrap().follow = FollowActivity::Following;
            let now = Instant::now();
            app.handle_at(Command::Action(Action::ZoomFit), now);
            app.tick(now + Duration::from_secs(1));
            assert_square_cells(&mut app, pipeline, &theme, 1.0);
            let p = app.panels.pipeline(pipeline).unwrap();
            assert_eq!(p.follow, FollowActivity::Suspended);
            let l = p.last_layout();
            let viewport = p.nav.viewport(&app.doc);
            let (a, b) = app.doc.limits();
            assert!(viewport.start <= a as f64 && viewport.end >= b as f64);
            assert!(l.row_y(0) >= l.cells.top() - 0.01);
            assert!(l.row_y(n as usize) <= l.cells.bottom() + 0.01);
            let fitted = scales(&mut app, pipeline, &theme);
            let pointer = in_box(cells, 0.5, 0.5);
            wheel_with(&mut app, pipeline, pointer, 0.0, 120.0, ctrl());
            let next = scales(&mut app, pipeline, &theme);
            let side = (fitted.1 * 2.0).min(24.0 * zoom);
            assert!((next.1 - side).abs() < 1e-4);
            assert_square_cells(&mut app, pipeline, &theme, 1.0);
            wheel_with(&mut app, pipeline, pointer, 40.0, 0.0, Modifiers::default());
            assert_square_cells(&mut app, pipeline, &theme, 1.0);
            assert!(same_scales(scales(&mut app, pipeline, &theme), next));
        }
    }
}

fn marker_lane(app: &mut App, id: PanelId, theme: &Theme) -> volna_core::wave::overlay::MarkerLane {
    match app.layout_panel(id, BOUNDS, theme, 1.0).unwrap() {
        PanelLayout::Waves(l) => l.marker_lane.clone(),
        PanelLayout::Pipeline(l) => l.marker_lane.clone(),
        PanelLayout::Table(_) => panic!("a table panel has no time axis"),
    }
}

fn press(app: &mut App, id: PanelId, position: Point, modifiers: Modifiers) {
    app.handle(Command::Pointer(
        id,
        PointerEvent::Down {
            position,
            button: MouseButton::Left,
            modifiers,
        },
    ));
    app.handle(Command::Pointer(id, PointerEvent::Up));
}

#[test]
fn the_markers_lane_is_the_same_in_wave_and_pipeline_panels() {
    let (mut app, _, waves, pipeline) = opened(60);
    pump(&mut app);
    let theme = Theme::one_dark();
    let (lo, hi) = app.doc.limits();
    let near = lo + (hi - lo) / 4;
    let far = lo + (hi - lo) * 3 / 4;
    for t in [near, near + 1, near + 2, far] {
        app.doc.add_marker(t);
    }
    let texts = |lane: &volna_core::wave::overlay::MarkerLane| {
        lane.chips
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    };
    for id in [waves, pipeline] {
        let lane = marker_lane(&mut app, id, &theme);
        let first = &lane.chips[0];
        assert!(
            first.is_cluster() && first.markers.start == 0,
            "markers a cycle apart share a chip"
        );
        assert_eq!(texts(&lane).last().unwrap(), "4");
        assert_eq!(lane.spans.len(), 1);
        assert_eq!(lane.spans[0].first, 2);
        let label = lane.spans[0]
            .labels
            .first()
            .expect("room for a label")
            .clone();
        frame(&mut app, id, &theme);
        assert!(app.scene().texts().any(|t| t == label), "{label}");
        assert!(
            app.scene().texts().any(|t| t == "Markers"),
            "the lane has its title"
        );
        assert!(app.scene().texts().any(|t| *t == first.text));
    }
    let centre = |r: Rect| point(r.left() + r.width() / 2.0, r.top() + r.height() / 2.0);

    let light = Theme::from_host(&volna_core::theme::vscode::host_palette(include_str!(
        "fixtures/vscode/light-modern.txt"
    )));
    for theme in [Theme::one_dark(), light] {
        let lane = marker_lane(&mut app, waves, &theme);
        frame(&mut app, waves, &theme);
        let quads: Vec<(Rect, volna_core::color::Color)> = app
            .scene()
            .prims
            .iter()
            .filter_map(|p| match p {
                Prim::Quad { rect, fill, .. } => Some((*rect, *fill)),
                _ => None,
            })
            .collect();
        let far_chip = lane.chips.last().unwrap();
        let colours = theme.marker(3);
        assert!(
            quads.contains(&(far_chip.rect, colours.background)),
            "chip 4 in marker colour 4"
        );
        assert!(
            quads.iter().any(|(r, c)| *c == colours.stroke
                && r.width() == 1.0
                && r.top() == far_chip.rect.top()),
            "marker 4's line starts at its chip"
        );
        assert!(
            quads.contains(&(lane.chips[0].rect, theme.badge.bg)),
            "a cluster in badge colours"
        );
        assert!(app.scene().prims.iter().any(|p| matches!(p,
            Prim::Text { text, color, .. } if text == "4" && *color == colours.text)));
    }

    let lane = marker_lane(&mut app, pipeline, &theme);
    press(
        &mut app,
        pipeline,
        centre(lane.chips.last().unwrap().rect),
        Modifiers::default(),
    );
    assert_eq!(app.doc.shared.cursor, Some(far));
    frame(&mut app, waves, &theme);
    assert!(
        app.scene().texts().any(|t| t == "at 4"),
        "the value cell names the marker under the cursor"
    );

    let lane = marker_lane(&mut app, waves, &theme);
    let cluster = centre(lane.chips[0].rect);
    let members = lane.chips[0].markers.len();
    let last = near + members as u64 - 1;
    app.handle(Command::Pointer(
        waves,
        PointerEvent::Move { position: cluster },
    ));
    frame(&mut app, waves, &theme);
    let title = format!("{members} markers · click to zoom in");
    assert!(app.scene().texts().any(|t| *t == title));
    assert!(app.scene().texts().any(|t| t == "Marker 2"));
    press(&mut app, waves, cluster, Modifiers::default());
    let target = app.doc.shared.viewport.target();
    assert!(target.start < near as f64 && target.end > last as f64);
    assert!(
        target.width() < 4.0,
        "zoomed to the cluster, not beyond: {target:?}"
    );
    app.doc.shared.viewport.set(target);
    for id in [waves, pipeline] {
        assert_eq!(texts(&marker_lane(&mut app, id, &theme)), ["1", "2", "3"]);
    }

    let lane = marker_lane(&mut app, pipeline, &theme);
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    press(&mut app, pipeline, centre(lane.chips[1].rect), shift);
    assert_eq!(app.doc.markers().len(), 4);
    assert_eq!(app.doc.shared.cursor, Some(near + 1));
    app.handle(Command::Action(Action::RemoveMarkerAtCursor));
    assert_eq!(
        app.doc
            .markers()
            .iter()
            .map(|m| m.id.get())
            .collect::<Vec<_>>(),
        [1, 3, 4]
    );
    let lane = marker_lane(&mut app, waves, &theme);
    let beside = point(
        f32::midpoint(lane.chips[0].rect.right(), lane.chips[1].rect.left()),
        lane.band.top() + 4.0,
    );
    press(&mut app, waves, beside, Modifiers::default());
    let cursor = app.doc.shared.cursor.unwrap();
    assert!(
        cursor > near && cursor < near + 2,
        "cursor {cursor} between markers 1 and 3"
    );
}

#[test]
fn click_sets_the_shared_cursor_to_the_cycle_and_the_wave_value_follows() {
    let (mut app, _, waves, pipeline) = opened(60);
    pump(&mut app);
    let theme = Theme::one_dark();
    let cells = cells(&mut app, pipeline, &theme);
    frame(&mut app, pipeline, &theme);
    let x = cells.left() + cells.width() * 0.5;
    let expected = {
        let p = app.panels.pipeline(pipeline).unwrap();
        p.last_layout()
            .time_at(&p.nav.viewport(&app.doc), x)
            .floor() as u64
    };
    let position = point(x, cells.top() + 20.0);
    app.handle(Command::Pointer(
        pipeline,
        PointerEvent::Down {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    assert!(app.panels.get(pipeline).unwrap().dragging());
    assert_eq!(app.doc.shared.cursor, None, "a press is not yet a click");
    app.handle(Command::Pointer(pipeline, PointerEvent::Up));
    assert_eq!(app.doc.shared.cursor, Some(expected));
    assert_eq!(
        app.panels.waves(waves).unwrap().cursor(&app.doc),
        Some(expected)
    );
    frame(&mut app, waves, &theme);
    let value = {
        let row = app.panels.waves(waves).unwrap().signal(0).unwrap();
        let history = row.history.as_ref().unwrap();
        row.translator
            .translate(&history.value(history.index_at(expected)))
            .text
    };
    assert!(
        app.scene().texts().any(|t| t == value),
        "value column shows {value} at cycle {expected}"
    );
    assert!(
        app.scene()
            .texts()
            .any(|t| t == format!("{expected} cycle"))
    );
    frame(&mut app, pipeline, &theme);
    assert!(
        app.scene()
            .texts()
            .any(|t| t == format!("{expected} cycle"))
    );
    assert!(app.status().cursor.unwrap().ends_with("cycle"));
    let before = app
        .panels
        .pipeline(pipeline)
        .unwrap()
        .nav
        .viewport(&app.doc);
    app.handle(Command::Pointer(
        pipeline,
        PointerEvent::Down {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    app.handle(Command::Pointer(
        pipeline,
        PointerEvent::Move {
            position: point(position.x + 40.0, position.y + 10.0),
        },
    ));
    app.handle(Command::Pointer(pipeline, PointerEvent::Up));
    assert_eq!(app.doc.shared.cursor, Some(expected));
    let after = app
        .panels
        .pipeline(pipeline)
        .unwrap()
        .nav
        .viewport(&app.doc);
    assert!(after.start < before.start);
    frame(&mut app, pipeline, &theme);
    let (row, hover_at) = {
        let p = app.panels.pipeline(pipeline).unwrap();
        let l = p.last_layout();
        let row = l.row_range.start + 1;
        let (_, tx) = match p.rows(&app.doc) {
            Rows::Ready(set) => set.get(row).unwrap(),
            _ => panic!(),
        };
        let vp = p.nav.viewport(&app.doc);
        let x =
            l.cells.left() + vp.x_of(tx.stages[1].begin as f64 + 0.5, l.cells_width_f64()) as f32;
        (row, point(x, l.row_y(row) + l.rows.row_px / 2.0))
    };
    app.handle(Command::Pointer(
        pipeline,
        PointerEvent::Move { position: hover_at },
    ));
    let p = app.panels.pipeline(pipeline).unwrap();
    assert_eq!(p.hover, Some(Hit::Cell { row, stage: 1 }));
    let hover = app.status().hover.unwrap();
    assert!(hover.starts_with(&format!("#{row} · D [")), "{hover}");
    assert!(hover.contains("op"), "{hover}");
    assert!(app.doc.selection().is_some());
    app.handle(Command::Action(Action::ClearSelection));
    assert_eq!(app.doc.selection(), None);
    assert_eq!(app.doc.shared.cursor, Some(expected));
    app.handle(Command::Action(Action::ClearSelection));
    assert_eq!(app.doc.shared.cursor, None);
    let split = app
        .panels
        .pipeline(pipeline)
        .unwrap()
        .last_layout()
        .label_split;
    let grab = point(split.left() + split.width() / 2.0, split.top() + 100.0);
    app.handle(Command::Pointer(
        pipeline,
        PointerEvent::Down {
            position: grab,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    app.handle(Command::Pointer(
        pipeline,
        PointerEvent::Move {
            position: point(250.0, grab.y),
        },
    ));
    app.handle(Command::Pointer(pipeline, PointerEvent::Up));
    frame(&mut app, pipeline, &theme);
    frame(&mut app, waves, &theme);
    assert_eq!(
        app.panels
            .pipeline(pipeline)
            .unwrap()
            .last_layout()
            .labels
            .width(),
        250.0
    );
    assert_eq!(
        app.panels.waves(waves).unwrap().last_layout().waves.left(),
        250.0
    );
}

#[test]
fn each_layout_uses_its_own_device_scale() {
    let (mut app, _, _, pipeline) = opened(1000);
    pump(&mut app);
    let theme = Theme::one_dark();
    app.panels
        .pipeline_mut(pipeline)
        .unwrap()
        .rows
        .set(RowView {
            top: 0.0,
            row_px: 1.0,
        });
    for (scale, expected) in [(3.0, 3.0), (1.25, 1.25), (f64::NAN, 1.0), (0.0, 1.0)] {
        let Some(PanelLayout::Pipeline(layout)) = app.layout_panel(pipeline, BOUNDS, &theme, scale)
        else {
            panic!("pipeline layout");
        };
        assert_eq!(layout.device_scale, expected);
        assert_eq!(layout.is_overview(), expected < 2.0);
    }
}

#[test]
fn overview_uses_physical_pixel_presence_with_bounded_output() {
    let (mut app, _, _, pipeline) = opened(10_000);
    pump(&mut app);
    let theme = Theme::one_dark();
    app.panels
        .pipeline_mut(pipeline)
        .unwrap()
        .rows
        .set(RowView {
            top: 0.0,
            row_px: 0.5,
        });
    app.handle(Command::Action(Action::ZoomFit));
    app.tick(Instant::now() + Duration::from_secs(1));
    app.layout_panel(
        pipeline,
        Rect::from_xywh(0.37, 0.23, BOUNDS.width(), BOUNDS.height()),
        &theme,
        1.25,
    )
    .unwrap();
    app.render_panel(pipeline, &theme, &mut MonoMeasure);
    let layout = app.panels.pipeline(pipeline).unwrap().last_layout().clone();
    assert!(layout.is_overview());
    assert_eq!(layout.device_scale, 1.25);
    let mut clips = Vec::new();
    for prim in &app.scene().prims {
        match prim {
            Prim::PushClip(rect) => clips.push(*rect),
            Prim::PopClip => {
                clips.pop().unwrap();
            }
            Prim::Presence { .. } => assert!(clips.contains(&layout.cells)),
            Prim::Quad { rect, fill, .. }
                if *fill == theme.editor.text && rect.height() <= 0.81 =>
            {
                assert!(clips.contains(&layout.cells));
            }
            _ => {}
        }
    }
    let runs: Vec<_> = app
        .scene()
        .quads()
        .filter(|(rect, color)| *color == theme.editor.text && rect.height() <= 0.81)
        .collect();
    assert!(
        !runs.is_empty()
            || app
                .scene()
                .prims
                .iter()
                .any(|p| matches!(p, Prim::Presence { .. }))
    );
    assert!(runs.len() <= 4096);
    assert!(!app.scene().texts().any(|t| t.starts_with("00001004")));
    let position = point(
        layout.cells.left() + layout.cells.width() * 0.5,
        layout.cells.top() + 2.0,
    );
    app.handle(Command::Pointer(pipeline, PointerEvent::Move { position }));
    assert_eq!(
        app.panels.pipeline(pipeline).unwrap().hover,
        Some(Hit::Overview)
    );
    assert_eq!(
        app.panels
            .pipeline(pipeline)
            .unwrap()
            .hover_text(&app.doc)
            .as_deref(),
        Some("Pipeline presence · zoom in to inspect records")
    );
}

#[test]
fn flushed_and_open_rows_are_told_apart_by_colour() {
    let (mut app, _, _, pipeline) = opened(12);
    pump(&mut app);
    let theme = Theme::one_dark();
    app.panels
        .pipeline_mut(pipeline)
        .unwrap()
        .rows
        .set(RowView {
            top: 0.0,
            row_px: 24.0,
        });
    frame(&mut app, pipeline, &theme);
    let p = app.panels.pipeline(pipeline).unwrap();
    let layout = p.last_layout().clone();
    let flush_tint = theme.editor.error.with_alpha(0.16);
    let tinted: Vec<f32> = app
        .scene()
        .quads()
        .filter(|(_, c)| *c == flush_tint)
        .map(|(r, _)| r.top())
        .collect();
    assert_eq!(tinted, [layout.row_y(3)], "row 3 was flushed");
    let dashed = app.scene().prims.iter().any(|prim| {
        matches!(prim, Prim::Lines { segments, .. }
            if segments.iter().all(|[a, b]| a.x == b.x
                && a.y >= layout.row_y(11) && b.y <= layout.row_y(12)))
    });
    assert!(dashed, "the open row ends with a dashed edge");
    let muted = app
        .scene()
        .prims
        .iter()
        .filter(|prim| {
            matches!(prim, Prim::Text { text, color, .. }
                if text.starts_with("0000") && *color == theme.panel.text_muted)
        })
        .count();
    assert_eq!(muted, 2);
    assert!(app.status().hover.is_none());
}

#[test]
fn workspace_round_trip_keeps_pipeline_panels_and_unresolved_tracks_survive() {
    let (mut app, session, waves, pipeline) = opened(30);
    pump(&mut app);
    let theme = Theme::one_dark();
    frame(&mut app, pipeline, &theme);
    app.handle(Command::Panels(PanelsCommand::ToggleLink {
        panel: pipeline,
        dim: LinkDim::Cursor,
    }));
    {
        let p = app.panels.pipeline_mut(pipeline).unwrap();
        p.nav.local_cursor = Some(7);
        p.rows.set(RowView {
            top: 4.5,
            row_px: 12.0,
        });
        p.label_width = 240.0;
        p.row_cap = 40.0;
    }
    let saved = Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    let json = serde_json::to_value(&saved).unwrap();
    let panel_json = json["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == "pipeline")
        .unwrap()
        .clone();
    assert_eq!(
        panel_json["track"],
        serde_json::json!(["cpu", "thread0", "instruction"])
    );
    assert_eq!(panel_json["rows"]["row_px"], 12.0);
    assert_eq!(panel_json["cursor"], 7);
    assert_eq!(panel_json["label_width"], 240.0);
    assert_eq!(panel_json["row_cap"], 40.0);
    let mut restored = App::new();
    restored.set_session(session.clone());
    let plan = Workspace::parse(&saved.to_bytes().unwrap())
        .unwrap()
        .prepare(
            &restored,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap();
    assert!(plan.report().notices.is_empty());
    plan.commit(&mut restored).unwrap();
    assert_eq!(restored.panels.len(), 3);
    let p = restored.panels.pipeline(pipeline).unwrap();
    assert_eq!(p.track.path(), ["cpu", "thread0", "instruction"]);
    assert!(p.is_attached());
    assert!(!p.nav.link.cursor && p.nav.link.viewport);
    assert_eq!(p.nav.local_cursor, Some(7));
    assert_eq!(
        p.rows.value,
        RowView {
            top: 4.5,
            row_px: 12.0
        }
    );
    assert_eq!(p.label_width, 240.0);
    assert_eq!(p.row_cap, 40.0);
    assert!(restored.panels.waves(waves).is_some());
    let requests = restored.take_requests();
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, LoadRequest::Track { .. }))
    );
    for r in requests {
        restored.deliver(r.perform());
    }
    frame(&mut restored, pipeline, &theme);
    assert!(matches!(
        restored.panels.pipeline(pipeline).unwrap().rows(&restored.doc),
        Rows::Ready(set) if set.len() == 30
    ));
    let mut altered = json.clone();
    for panel in altered["panels"].as_array_mut().unwrap() {
        if panel["kind"] == "pipeline" {
            panel["track"] = serde_json::json!(["gone", "stream"]);
        }
    }
    let mut other = App::new();
    other.set_session(session.clone());
    let plan = Workspace::parse(serde_json::to_string(&altered).unwrap().as_bytes())
        .unwrap()
        .prepare(
            &other,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap();
    assert!(
        plan.report()
            .notices
            .iter()
            .any(|n| n.contains("Missing pipeline track"))
    );
    plan.commit(&mut other).unwrap();
    let p = other.panels.pipeline(pipeline).unwrap();
    assert_eq!(
        p.track,
        TrackSource::Unresolved {
            trace: TraceId::A,
            path: vec!["gone".into(), "stream".into()]
        }
    );
    assert!(
        !other
            .take_requests()
            .iter()
            .any(|r| matches!(r, LoadRequest::Track { .. })),
        "an unresolved track loads nothing"
    );
    frame(&mut other, pipeline, &theme);
    assert!(other.scene().texts().any(|t| t == "Not in this trace"));
    let again = serde_json::to_value(
        Workspace::capture(&other, volna_core::testing::paths("trace.vtr"), None).unwrap(),
    )
    .unwrap();
    let kept = again["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == "pipeline")
        .unwrap();
    assert_eq!(kept["track"], serde_json::json!(["gone", "stream"]));
    let mut bad = json;
    for panel in bad["panels"].as_array_mut().unwrap() {
        if panel["kind"] == "pipeline" {
            panel["rows"]["row_px"] = serde_json::json!(1000.0);
        }
    }
    assert!(
        Workspace::parse(serde_json::to_string(&bad).unwrap().as_bytes())
            .unwrap()
            .prepare(
                &other,
                "file:///tmp/trace.vtr",
                "file:///tmp/trace.vtr.volna.json"
            )
            .is_err()
    );
}

#[test]
fn closing_the_last_panel_releases_the_track_and_late_delivery_is_ignored() {
    let (mut app, session, waves, pipeline) = opened(20);
    let track = generator_track(session.as_ref(), &["cpu", "thread0"]);
    let request = app
        .take_requests()
        .into_iter()
        .find(|r| matches!(r, LoadRequest::Track { .. }))
        .unwrap();
    app.handle(Command::Action(Action::SplitRight));
    let split = app.panels.focused_id();
    assert!(app.panels.pipeline(split).unwrap().is_attached());
    assert!(app.take_requests().is_empty());
    app.handle(Command::Panels(PanelsCommand::Close(split)));
    assert!(app.doc.track(a(track)).is_some());
    app.handle(Command::Panels(PanelsCommand::Close(pipeline)));
    assert!(
        app.doc.track(a(track)).is_none(),
        "released with its last consumer"
    );
    assert_eq!(
        app.panels.focused_id(),
        app.panels.first_free_transaction().unwrap()
    );
    app.handle(Command::Panels(PanelsCommand::Focus(waves)));
    app.deliver(request.perform());
    assert!(
        app.doc.track(a(track)).is_none(),
        "late delivery is a no-op"
    );
    app.handle(Command::ActivateMembers(a_all(vec![Member::Generator(0)])));
    let generator = app
        .panels
        .pipeline(app.panels.focused_id())
        .unwrap()
        .track
        .track()
        .unwrap();
    app.handle(Command::Panels(PanelsCommand::CloseOthers(waves)));
    assert_eq!(app.panels.len(), 1);
    assert!(app.doc.track(generator).is_none());
    let mut failing = App::new();
    failing.set_session(session.clone());
    failing.handle(Command::OpenPipeline { track: a(track) });
    let request = failing.take_requests().pop().unwrap();
    let LoadRequest::Track {
        generation,
        request_id,
        track,
        ..
    } = request
    else {
        panic!()
    };
    failing.deliver(volna_core::session::LoadResult::Track {
        trace: TraceId::A,
        generation,
        request_id,
        track,
        result: Err(anyhow::anyhow!("disk on fire")),
    });
    let id = failing.panels.focused_id();
    let theme = Theme::one_dark();
    frame(&mut failing, id, &theme);
    assert!(failing.scene().texts().any(|t| t == "disk on fire"));
    let retry = failing
        .panels
        .pipeline(id)
        .unwrap()
        .last_layout()
        .retry
        .unwrap();
    failing.handle(Command::Pointer(
        id,
        PointerEvent::Down {
            position: Point {
                x: retry.left() + 2.0,
                y: retry.top() + 2.0,
            },
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    assert!(matches!(
        failing.take_requests().as_slice(),
        [LoadRequest::Track { .. }]
    ));
}

#[test]
fn a_stream_opened_first_replaces_the_start_panel_and_rows_find_a_waveform_tab() {
    let session = fixture(20);
    let mut app = App::new();
    app.set_session(session.clone());
    let start = app.panels.focused_id();
    assert!(app.panels.focused().kind.is_start());
    let track = generator_track(session.as_ref(), &["cpu", "thread0"]);
    app.handle(Command::OpenPipeline { track: a(track) });
    assert_eq!(app.panels.len(), 2);
    let pipeline = app.panels.focused_id();
    assert_ne!(pipeline, start);
    assert!(app.panels.pipeline(pipeline).unwrap().is_attached());
    pump(&mut app);
    assert!(app.doc.track(a(track)).is_some());
    app.handle(Command::AddVars(a_all(vec![0])));
    assert_eq!(app.panels.len(), 3);
    let waves = app.panels.focused_id();
    assert_eq!(app.panels.waves(waves).unwrap().items().len(), 1);
    assert_eq!(
        app.panels.layout().center.as_ref().unwrap(),
        &Layout::Tabs {
            tabs: vec![pipeline, waves],
            active: waves
        }
    );
    app.handle(Command::Panels(PanelsCommand::Focus(pipeline)));
    app.handle(Command::AddVars(a_all(vec![0])));
    assert_eq!(app.panels.len(), 3);
    assert_eq!(app.panels.focused_id(), waves);
    assert_eq!(app.panels.waves(waves).unwrap().items().len(), 2);
    app.handle(Command::Panels(PanelsCommand::Close(waves)));
    app.handle(Command::Panels(PanelsCommand::Close(pipeline)));
    assert_eq!(app.panels.len(), 1);
    assert!(app.panels.focused().kind.transaction().is_some());
    assert!(app.doc.track(a(track)).is_none());
    app.panels.validate().unwrap();
}

#[test]
fn the_checked_in_showcase_has_two_pipeline_generators_on_a_cycle_time_base() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../volna/examples/pipeline_showcase.vtr");
    let session = OpenSpec::Path(path).open().unwrap();
    assert_eq!(session.info().time_unit.as_deref(), Some("cycle"));
    let mut app = App::new();
    app.set_session(session.clone());
    let theme = Theme::one_dark();
    let mut counts = Vec::new();
    for core in ["cpu0", "cpu1"] {
        let stream = scope(session.as_ref(), &["soc", core, "pipeline"]);
        app.handle(Command::ActivateMembers(a_all(vec![Member::Generator(
            session
                .hierarchy()
                .scope(stream)
                .generators
                .iter()
                .next()
                .unwrap(),
        )])));
        pump(&mut app);
        let id = app.panels.focused_id();
        frame(&mut app, id, &theme);
        let p = app.panels.pipeline(id).unwrap();
        let Rows::Ready(set) = p.rows(&app.doc) else {
            panic!("{core} loaded")
        };
        counts.push(set.len());
        assert!(
            p.palette().names().len() >= 5,
            "{core}: {:?}",
            p.palette().names()
        );
        assert_eq!(p.palette().primary_lane(), "0");
        let flushed = (0..set.len())
            .filter(|r| set.get(*r).unwrap().1.status == vtr::TxStatus::Aborted)
            .count();
        let open = (0..set.len())
            .filter(|r| set.get(*r).unwrap().1.status == vtr::TxStatus::Open)
            .count();
        assert!(
            flushed > 0 && open > 0,
            "{core}: {flushed} flushed, {open} open"
        );
        assert!(app.scene().texts().any(|t| t.contains(": lw   a1, 0(a2)")));
    }
    assert_eq!(app.panels.len(), 3);
    assert_eq!(
        app.panels
            .iter()
            .filter(|p| p.kind.pipeline().is_some())
            .count(),
        2
    );
    assert!(app.panels.first_free_transaction().is_some());
    assert!(counts.iter().all(|n| *n > 500), "{counts:?}");
    assert!(app.status().time_range.unwrap().ends_with("cycle"));
}

#[test]
fn the_feature_showcase_cpu_paints_stage_cells_and_stall_bands() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../volna/examples/feature_showcase.vtr");
    let session = OpenSpec::Path(path).open().unwrap();
    let mut app = App::new();
    app.set_session(session.clone());
    pump(&mut app);
    let stream = scope(session.as_ref(), &["soc", "cpu", "thread0"]);
    app.handle(Command::ActivateMembers(a_all(vec![Member::Generator(
        session
            .hierarchy()
            .scope(stream)
            .generators
            .iter()
            .next()
            .unwrap(),
    )])));
    pump(&mut app);
    let id = app.panels.focused_id();
    let theme = Theme::one_dark();
    frame(&mut app, id, &theme);
    let p = app.panels.pipeline(id).expect("a pipeline panel");
    assert_eq!(p.palette().primary_lane(), "main");
    let stages = ["fetch", "decode", "execute", "memory", "writeback"];
    assert_eq!(p.palette().names(), stages);
    let Rows::Ready(set) = p.rows(&app.doc) else {
        panic!("loaded")
    };
    let aborted = (0..set.len())
        .filter(|r| set.get(*r).unwrap().1.status == vtr::TxStatus::Aborted)
        .count();
    assert_eq!((set.len(), aborted), (146, 0));
    let fills: Vec<_> = stages
        .iter()
        .map(|s| p.palette().style(s, &theme).fill)
        .collect();
    let band = theme.editor.text.with_alpha(0.28);
    let cells = p.last_layout().cells;
    let inside: Vec<_> = app
        .scene()
        .quads()
        .filter(|(rect, _)| cells.contains(rect.origin))
        .map(|(_, color)| color)
        .collect();
    for fill in &fills {
        assert!(inside.contains(fill), "a {fill:?} cell");
    }
    assert!(inside.contains(&band), "a stall band");
}

#[test]
fn the_theme_showcase_runs_the_c910_stage_ladder_with_flushed_and_open_rows() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../volna/examples/volna-theme.vtr");
    let session = OpenSpec::Path(path).open().unwrap();
    let mut app = App::new();
    app.set_session(session.clone());
    pump(&mut app);
    let theme = Theme::one_dark();
    for (core, stages) in [
        ("cpu0", &["F", "D", "X", "M", "W"][..]),
        (
            "cpu1",
            &[
                "ID", "IR", "IS", "IQ", "RF", "EX", "AG", "BJ", "DC", "CM", "DA", "RT", "WB",
            ][..],
        ),
    ] {
        let stream = scope(session.as_ref(), &["soc", core, "pipeline"]);
        app.handle(Command::ActivateMembers(a_all(vec![Member::Generator(
            session
                .hierarchy()
                .scope(stream)
                .generators
                .iter()
                .next()
                .unwrap(),
        )])));
        pump(&mut app);
        let id = app.panels.focused_id();
        frame(&mut app, id, &theme);
        let p = app.panels.pipeline(id).expect("a pipeline panel");
        assert_eq!(p.palette().names(), stages, "{core}");
        let Rows::Ready(set) = p.rows(&app.doc) else {
            panic!("{core} loaded")
        };
        let count = |status| {
            (0..set.len())
                .filter(|r| set.get(*r).unwrap().1.status == status)
                .count()
        };
        let (flushed, open) = (count(vtr::TxStatus::Aborted), count(vtr::TxStatus::Open));
        assert!(
            set.len() > 1500 && flushed > 50 && open > 0,
            "{core}: {} rows, {flushed} flushed, {open} open",
            set.len()
        );
    }
}

#[test]
fn the_verilator_demo_counts_in_its_clock_and_feeds_the_transaction_panel() {
    use volna_core::transaction::TxPanelState;
    use volna_trace::data::transactions::TransactionRef;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../volna/examples/pipeline_demo.vtr");
    let session = OpenSpec::Path(path).open().unwrap();
    let mut app = App::new();
    app.set_session(session.clone());
    pump(&mut app);
    let stream = scope(session.as_ref(), &["TOP", "tb", "core", "pipeline"]);
    app.handle(Command::ActivateMembers(a_all(vec![Member::Generator(
        session
            .hierarchy()
            .scope(stream)
            .generators
            .iter()
            .next()
            .unwrap(),
    )])));
    pump(&mut app);
    let id = app.panels.focused_id();
    app.handle(Command::PipelineActivity(
        id,
        volna_core::pipeline::ActivityCommand::Toggle,
    ));
    app.panels
        .pipeline_mut(id)
        .unwrap()
        .rows
        .set(RowView::default());
    let theme = Theme::one_dark();
    frame(&mut app, id, &theme);
    let p = app
        .panels
        .pipeline(id)
        .expect("a PIPELINE stream opens a pipeline panel");
    assert_eq!(p.clock(&app.doc).unwrap().path, "TOP.tb.clk");
    assert_eq!(p.palette().primary_lane(), "");
    assert_eq!(p.palette().names(), ["F", "D", "Q", "X", "C", "R"]);
    let Rows::Ready(set) = p.rows(&app.doc) else {
        panic!("loaded")
    };
    let statuses: Vec<_> = (0..set.len())
        .map(|r| set.get(r).unwrap().1.status)
        .collect();
    let count = |s| statuses.iter().filter(|x| **x == s).count();
    assert_eq!(statuses.len(), 12);
    assert_eq!(
        [
            count(vtr::TxStatus::Ok),
            count(vtr::TxStatus::Error),
            count(vtr::TxStatus::Aborted),
            count(vtr::TxStatus::Open)
        ],
        [5, 1, 5, 1]
    );
    let layout = p.last_layout().clone();
    let viewport = p.nav.viewport(&app.doc);
    let y = layout.row_y(0) + layout.rows.row_px / 2.0;
    let x = layout.cells.left() + viewport.x_of(50.0, layout.cells_width_f64()) as f32;
    app.handle(Command::Pointer(
        id,
        PointerEvent::Move {
            position: point(x, y),
        },
    ));
    let hover = app.status().hover.unwrap();
    assert!(hover.contains("X cycles 4–5 (1 cycle)"), "{hover}");
    app.handle(Command::Pointer(
        id,
        PointerEvent::Down {
            position: point(x, y),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    app.handle(Command::Pointer(id, PointerEvent::Up));
    assert_eq!(app.doc.shared.cursor, Some(45));
    let selection = app.doc.selection().unwrap();
    app.handle(Command::ShowTransaction { from: id });
    let panel = app.panels.focused_id();
    let TxPanelState::Ready(v) = app.panels.transaction(panel).unwrap().state(&app.doc) else {
        panic!("the Transaction panel shows the selection")
    };
    assert_eq!(v.identity.id, selection.id);
    assert_eq!(v.identity.label.as_deref(), Some("0100 add a0,a0,a1"));
    assert_eq!(v.identity.scope, ["TOP", "tb", "core", "pipeline"]);
    let names: Vec<_> = v.lifeline[0]
        .cells
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["F", "D", "Q", "X", "C", "R"]);
    assert!(
        v.related
            .rows
            .iter()
            .any(|r| r.group == "wakeup · to" && r.target.transaction != TransactionRef(0)),
        "{:?}",
        v.related.rows.iter().map(|r| &r.group).collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "Manual half-million-row performance benchmark; requires substantial memory and time."]
fn half_million_rows_load_lay_out_and_paint() {
    use std::time::Instant as Clock;
    fn rss_mib() -> f64 {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<f64>()
            .unwrap_or(0.0)
            / 1024.0
    }
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let (path, stream_path) = if let Ok(p) = std::env::var("VOLNA_PIPELINE_TRACE") {
        (
            std::path::PathBuf::from(p),
            std::env::var("VOLNA_PIPELINE_STREAM").expect("VOLNA_PIPELINE_STREAM"),
        )
    } else {
        use vtr::{TxStatus, Value};
        let t0 = Clock::now();
        let mut w = vtr::Writer::create(file.path()).unwrap();
        w.set_timescale(vtr::Timescale::Exponent(-10)).unwrap();
        let core = w
            .add_scope(None, "core", vtr::ScopeType::Module, "")
            .unwrap();
        let stream = w
            .add_scope(Some(core), "pipeline", vtr::ScopeType::Generic, "")
            .unwrap();
        let insn = w
            .add_generator(Some(stream), "instruction", "PIPELINE")
            .unwrap();
        let names: Vec<_> = [
            "ID", "IR", "IS", "IQ", "RF", "EX", "CM", "RT", "BJ", "FX", "AG", "DC", "DA", "WB",
        ]
        .iter()
        .map(|s| w.intern(s))
        .collect();
        let lane = w.intern("");
        let (label, pc_key, iid_key) = (w.intern("vtr.label"), w.intern("pc"), w.intern("iid"));
        let captions: Vec<_> = (0..2000u64)
            .map(|pc| w.intern(&format!("0x{:04x} op{pc}", pc * 4)))
            .collect();
        let mut rng = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for i in 0..500_000u64 {
            let begin = 2 * (i * 10 / 13);
            let tx = w.begin_tx(insn, begin).unwrap();
            let r = next();
            let aborted = r % 5 == 0;
            let mut path = vec![0usize, 1, 2, 3, 4];
            match (r >> 8) % 6 {
                0 => path.push(8),
                1 => path.push(9),
                2 | 3 => path.extend([10, 11, 12, 13]),
                _ => path.push(5),
            }
            path.extend([6, 7]);
            if aborted {
                path.truncate(1 + (r >> 16) as usize % 4);
            }
            let mut t = begin;
            for &s in &path {
                let len = 2 * (1 + (next() % 3));
                w.tx_stage(tx, names[s], lane, t, t + len, &[]).unwrap();
                t += len;
            }
            w.tx_attr(tx, label, &Value::Str(captions[(i % 2000) as usize]))
                .unwrap();
            w.tx_attr(tx, pc_key, &Value::U64((i % 2000) * 4)).unwrap();
            if !aborted {
                w.tx_attr(tx, iid_key, &Value::U64(i % 128)).unwrap();
            }
            w.end_tx(
                tx,
                t,
                if aborted {
                    TxStatus::Aborted
                } else {
                    TxStatus::Ok
                },
            )
            .unwrap();
        }
        w.close().unwrap();
        println!(
            "generated 500000 rows in {:.2} s",
            t0.elapsed().as_secs_f64()
        );
        (file.path().to_path_buf(), "core.pipeline".to_owned())
    };
    let rss0 = rss_mib();
    let t0 = Clock::now();
    let session = OpenSpec::Path(path).open().unwrap();
    let mut app = App::new();
    if let Ok(mib) = std::env::var("VOLNA_OBJECT_MIB") {
        for (id, v) in [
            ("memory.budgetMiB", 8192),
            ("memory.objectMiB", mib.parse::<i64>().unwrap()),
        ] {
            app.handle(Command::Settings(volna_core::app::SettingsCommand::Set {
                id: id.into(),
                value: volna_core::settings::Value::Integer(v),
            }));
        }
    }
    app.set_session(session.clone());
    pump(&mut app);
    let parts: Vec<&str> = stream_path.split('.').collect();
    let stream = scope(session.as_ref(), &parts);
    let opened = t0.elapsed().as_secs_f64();
    let t1 = Clock::now();
    app.handle(Command::ActivateMembers(a_all(vec![Member::Generator(
        session
            .hierarchy()
            .scope(stream)
            .generators
            .iter()
            .next()
            .unwrap(),
    )])));
    pump(&mut app);
    let loaded = t1.elapsed().as_secs_f64();
    let id = app.panels.focused_id();
    let theme = Theme::one_dark();
    let t2 = Clock::now();
    frame(&mut app, id, &theme);
    let first = t2.elapsed().as_secs_f64();
    let p = app.panels.pipeline(id).unwrap();
    let set = match p.rows(&app.doc) {
        Rows::Ready(set) => set,
        Rows::Failed(e) => panic!("load failed: {e}"),
        _ => panic!("rows not ready"),
    };
    let rows = set.len();
    let stages = p.palette().names().to_vec();
    let mut report = format!(
        "{rows} rows, {} stage names {:?}\nopen {opened:.3} s, load {loaded:.3} s, first frame {first:.3} s, \
         resident {:.0} MiB (+{:.0})\n",
        stages.len(),
        stages,
        rss_mib(),
        rss_mib() - rss0
    );
    let mid = {
        let (b, e) = (
            set.get(0).unwrap().1.begin,
            set.get(rows - 1).unwrap().1.begin,
        );
        (b + e) as f64 / 2.0
    };
    for (what, window) in [
        ("whole trace", None),
        ("5000-cycle window", Some(10_000.0)),
        ("150-cycle window", Some(300.0)),
    ] {
        match window {
            None => {
                app.handle_at(Command::Action(Action::ZoomFit), volna_core::Instant::now());
                app.tick(volna_core::Instant::now() + Duration::from_secs(1));
                frame(&mut app, id, &theme);
            }
            Some(w) => app
                .doc
                .shared
                .viewport
                .set(volna_core::wave::viewport::Viewport {
                    start: mid - w / 2.0,
                    end: mid + w / 2.0,
                }),
        }
        let n = 20;
        let t = Clock::now();
        for _ in 0..n {
            frame(&mut app, id, &theme);
        }
        let per = t.elapsed().as_secs_f64() / f64::from(n) * 1e3;
        let prims = app.scene().prims.len();
        writeln!(
            report,
            "{what}: {per:.2} ms per layout + paint, {prims} primitives"
        )
        .unwrap();
    }
    println!("{report}");
}

#[test]
fn marker_walks_move_a_pipeline_panel_cursor_like_a_wave_panel_cursor() {
    let (mut app, _, waves, pipeline) = opened(60);
    pump(&mut app);
    let (lo, hi) = app.doc.limits();
    let (near, far) = (lo + (hi - lo) / 4, lo + (hi - lo) * 3 / 4);
    app.doc.add_marker(near);
    app.doc.add_marker(far);
    let cursor = |app: &App, id| {
        app.panels
            .get(id)
            .unwrap()
            .kind
            .nav()
            .unwrap()
            .cursor(&app.doc)
    };
    assert_eq!(app.panels.focused_id(), pipeline);
    app.doc.shared.cursor = Some(lo);
    app.handle(Command::Action(Action::NextMarker));
    assert_eq!(cursor(&app, pipeline), Some(near));
    assert_eq!(cursor(&app, waves), Some(near), "the cursor is linked");
    app.handle(Command::Panels(PanelsCommand::ToggleLink {
        panel: pipeline,
        dim: LinkDim::Cursor,
    }));
    app.handle(Command::Action(Action::GoToMarker(
        volna_core::marker::MarkerId::new(2).unwrap(),
    )));
    assert_eq!(cursor(&app, pipeline), Some(far));
    assert_eq!(cursor(&app, waves), Some(near));
    app.handle(Command::Action(Action::JumpBack));
    assert_eq!(cursor(&app, pipeline), Some(near));
}

#[test]
fn the_pipeline_measures_from_the_reference_like_the_waves() {
    let (mut app, _, waves, pipeline) = opened(60);
    pump(&mut app);
    let theme = Theme::one_dark();
    let (lo, hi) = app.doc.limits();
    let (a, b) = (lo + (hi - lo) / 4, lo + (hi - lo) / 2);
    app.doc.add_marker(a);
    app.handle(Command::Panels(PanelsCommand::Focus(pipeline)));
    app.doc.shared.cursor = Some(a);
    app.handle(Command::Action(Action::SetReference));
    assert_eq!(
        app.doc.reference(),
        Some(volna_core::marker::Reference::Marker(
            volna_core::marker::MarkerId::new(1).unwrap()
        ))
    );
    app.doc.shared.cursor = Some(b);
    for id in [waves, pipeline] {
        let lane = marker_lane(&mut app, id, &theme);
        let m = lane.measure.expect("the Measure lane");
        let live = m.live.expect("the live span");
        assert_eq!((live.measurement.from, live.measurement.to), (a, b));
        frame(&mut app, id, &theme);
        assert!(app.scene().texts().any(|t| t == "R → cursor"));
    }
    let cells_top = match app.layout_panel(pipeline, BOUNDS, &theme, 1.0).unwrap() {
        PanelLayout::Pipeline(l) => l.cells.top(),
        _ => unreachable!(),
    };
    let lane = marker_lane(&mut app, pipeline, &theme);
    assert_eq!(cells_top, lane.strip().bottom());

    let x = f32::midpoint(lane.time_left, lane.band.right());
    let p = point(x, cells_top + 3.0);
    let alt = Modifiers {
        alt: true,
        ..Default::default()
    };
    press(&mut app, pipeline, p, alt);
    let t = app.doc.reference_time().unwrap();
    assert!(matches!(
        app.doc.reference(),
        Some(volna_core::marker::Reference::Time(_))
    ));
    assert_eq!(app.doc.shared.cursor, Some(b), "Alt-click keeps the cursor");
    app.handle(Command::Action(Action::ClearReference));
    app.handle(Command::Pointer(
        pipeline,
        PointerEvent::Down {
            position: p,
            button: MouseButton::Middle,
            modifiers: Modifiers::default(),
        },
    ));
    assert_eq!(app.doc.reference(), None, "measured on release");
    app.handle(Command::Pointer(pipeline, PointerEvent::Up));
    assert_eq!(app.doc.reference_time(), Some(t));

    let clear = marker_lane(&mut app, pipeline, &theme)
        .measure
        .unwrap()
        .clear;
    assert!(clear.left() > 0.0);
    press(
        &mut app,
        pipeline,
        point(
            clear.left() + clear.width() / 2.0,
            clear.top() + clear.height() / 2.0,
        ),
        Modifiers::default(),
    );
    assert_eq!(app.doc.reference(), None);
}

#[test]
fn a_pipeline_chip_goes_on_click_moves_onto_cycles_on_drag_and_esc_cancels() {
    let (mut app, _, _, pipeline) = opened(60);
    pump(&mut app);
    let theme = Theme::one_dark();
    let (lo, hi) = app.doc.limits();
    let a = lo + (hi - lo) / 4;
    app.doc.add_marker(a);
    app.handle(Command::Panels(PanelsCommand::Focus(pipeline)));
    app.doc.shared.cursor = Some(lo);
    let lane = marker_lane(&mut app, pipeline, &theme);
    let chip = lane.chips[0].rect;
    let p = point(chip.left() + 3.0, chip.top() + chip.height() / 2.0);
    let down = |app: &mut App, p| {
        app.handle(Command::Pointer(
            pipeline,
            PointerEvent::Down {
                position: p,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        ));
    };
    let to = |app: &mut App, x: f32| {
        app.handle(Command::Pointer(
            pipeline,
            PointerEvent::Move {
                position: point(x, p.y),
            },
        ));
    };
    down(&mut app, p);
    to(&mut app, p.x + 2.0);
    app.handle(Command::Pointer(pipeline, PointerEvent::Up));
    assert_eq!(app.doc.shared.cursor, Some(a));
    assert_eq!(app.doc.markers()[0].time, a);

    down(&mut app, p);
    to(&mut app, p.x + 40.0);
    to(&mut app, p.x + 83.0);
    app.handle(Command::Pointer(pipeline, PointerEvent::Up));
    let moved = app.doc.markers()[0].time;
    assert!(moved > a, "{moved}");
    let expected = {
        let model = app.panels.pipeline(pipeline).unwrap();
        let v = model.nav.viewport(&app.doc);
        model.last_layout().time_at(&v, chip.left() + 83.0).floor() as u64
    };
    assert_eq!(moved, expected, "a whole cycle in this cycle-counted trace");
    assert_eq!(app.undo_label(), Some("Move marker 1"));
    assert_eq!(app.doc.shared.cursor, Some(a), "a drag keeps the cursor");

    let lane = marker_lane(&mut app, pipeline, &theme);
    let chip = lane.chips[0].rect;
    let p = point(chip.left() + 3.0, chip.top() + chip.height() / 2.0);
    down(&mut app, p);
    to(&mut app, p.x - 60.0);
    assert_ne!(app.doc.markers()[0].time, moved);
    app.handle(Command::Action(Action::ClearSelection));
    app.handle(Command::Pointer(pipeline, PointerEvent::Up));
    assert_eq!(app.doc.markers()[0].time, moved);
    assert_eq!(app.undo_label(), Some("Move marker 1"));
    app.handle(Command::Undo);
    assert_eq!(app.doc.markers()[0].time, a);
}

fn set_center(app: &mut App, center: Layout) {
    let mut layout = app.panels.layout().clone();
    layout.center = Some(center);
    app.handle(Command::Panels(PanelsCommand::SetLayout {
        layout,
        from_revision: app.panels.revision(),
    }));
}

fn dock_timelines(
    app: &mut App,
    waves: PanelId,
    pipeline: PanelId,
    axis: volna_core::panels::Axis,
) {
    set_center(
        app,
        Layout::Split {
            split: axis,
            sizes: vec![0.5, 0.5],
            children: vec![Layout::single(waves), Layout::single(pipeline)],
        },
    );
}

fn timeline_frames(app: &mut App, waves: PanelId, pipeline: PanelId, width: f32, theme: &Theme) {
    app.layout_panel(
        pipeline,
        Rect::from_xywh(25.0, 350.0, width, 300.0),
        theme,
        1.0,
    )
    .unwrap();
    app.layout_panel(waves, Rect::from_xywh(25.0, 20.0, width, 300.0), theme, 1.0)
        .unwrap();
}

fn aligned_metadata(app: &App, waves: PanelId, pipeline: PanelId) -> f32 {
    let wave = app.panels.waves(waves).unwrap().last_layout();
    let pipe = app.panels.pipeline(pipeline).unwrap().last_layout();
    assert_eq!(wave.waves.left(), pipe.cells.left());
    assert_eq!(wave.waves.width(), pipe.cells.width());
    let viewport = app.doc.shared.viewport.value;
    for time in [
        viewport.start,
        f64::midpoint(viewport.start, viewport.end),
        viewport.end,
    ] {
        let wx = f64::from(wave.waves.left()) + viewport.x_of(time, wave.wave_width_f64());
        let px = f64::from(pipe.cells.left()) + viewport.x_of(time, pipe.cells_width_f64());
        assert_eq!(wx, px);
    }
    wave.waves.left() - wave.bounds.left()
}

fn drag_divider(app: &mut App, panel: PanelId, hit: Rect, to_x: f32) {
    let position = point(
        hit.left() + hit.width() / 2.0,
        hit.top() + hit.height() / 2.0,
    );
    app.handle(Command::Pointer(
        panel,
        PointerEvent::Down {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    assert!(app.panels.get(panel).unwrap().dragging());
    app.handle(Command::Pointer(
        panel,
        PointerEvent::Move {
            position: point(to_x, position.y),
        },
    ));
    app.handle(Command::Pointer(panel, PointerEvent::Up));
}

#[test]
fn stacked_timelines_share_divider_and_keep_names_divider_local() {
    use volna_core::panels::Axis;
    for theme in [Theme::one_dark(), Theme::volna(false).zoomed(1.5)] {
        let (mut app, _, waves, pipeline) = opened(30);
        pump(&mut app);
        dock_timelines(&mut app, waves, pipeline, Axis::Horizontal);
        app.panels.pipeline_mut(pipeline).unwrap().label_width = 400.0;
        dock_timelines(&mut app, waves, pipeline, Axis::Vertical);
        timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
        assert_eq!(aligned_metadata(&app, waves, pipeline), 400.0 * theme.zoom);

        let pipe_split = app
            .panels
            .pipeline(pipeline)
            .unwrap()
            .last_layout()
            .label_split;
        drag_divider(&mut app, pipeline, pipe_split, 25.0 + 500.0 * theme.zoom);
        timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
        assert_eq!(aligned_metadata(&app, waves, pipeline), 500.0 * theme.zoom);
        assert_eq!(app.panels.pipeline(pipeline).unwrap().label_width, 400.0);

        let names_split = app.panels.waves(waves).unwrap().last_layout().names_split;
        drag_divider(&mut app, waves, names_split, 25.0 + 300.0 * theme.zoom);
        timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
        assert_eq!(aligned_metadata(&app, waves, pipeline), 500.0 * theme.zoom);
        assert_eq!(
            app.panels.waves(waves).unwrap().last_layout().names.width(),
            300.0 * theme.zoom
        );
        assert_eq!(
            app.panels
                .waves(waves)
                .unwrap()
                .last_layout()
                .values
                .width(),
            200.0 * theme.zoom
        );

        let wave_split = app.panels.waves(waves).unwrap().last_layout().values_split;
        drag_divider(&mut app, waves, wave_split, 25.0 + 350.0 * theme.zoom);
        timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
        assert_eq!(aligned_metadata(&app, waves, pipeline), 350.0 * theme.zoom);
        wheel_with(&mut app, pipeline, point(900.0, 450.0), 0.0, 120.0, ctrl());
        timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
        aligned_metadata(&app, waves, pipeline);
        app.render_panel(waves, &theme, &mut MonoMeasure);
        app.render_panel(pipeline, &theme, &mut MonoMeasure);
    }
}

#[test]
fn timeline_alignment_requires_stacked_viewport_links() {
    use volna_core::panels::Axis;
    let (mut app, _, waves, pipeline) = opened(30);
    let theme = Theme::one_dark();
    dock_timelines(&mut app, waves, pipeline, Axis::Horizontal);
    timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
    assert_eq!(
        app.panels.waves(waves).unwrap().last_layout().waves.left(),
        365.0
    );
    assert_eq!(
        app.panels
            .pipeline(pipeline)
            .unwrap()
            .last_layout()
            .cells
            .left(),
        215.0
    );
    dock_timelines(&mut app, waves, pipeline, Axis::Vertical);
    timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
    assert_eq!(aligned_metadata(&app, waves, pipeline), 340.0);
    app.handle(Command::Panels(PanelsCommand::ToggleLink {
        panel: pipeline,
        dim: LinkDim::Cursor,
    }));
    timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
    aligned_metadata(&app, waves, pipeline);
    app.handle(Command::Panels(PanelsCommand::ToggleLink {
        panel: pipeline,
        dim: LinkDim::Viewport,
    }));
    timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
    assert_eq!(
        app.panels
            .pipeline(pipeline)
            .unwrap()
            .last_layout()
            .cells
            .left(),
        215.0
    );
    assert!(
        !app.panels
            .waves(waves)
            .unwrap()
            .last_layout()
            .shared_metadata
    );
    app.handle(Command::Panels(PanelsCommand::ToggleLink {
        panel: pipeline,
        dim: LinkDim::Viewport,
    }));
    timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
    aligned_metadata(&app, waves, pipeline);
}

#[test]
fn timeline_divider_survives_narrow_windows_and_workspace_restore() {
    use volna_core::panels::Axis;
    let (mut app, session, waves, pipeline) = opened(30);
    let theme = Theme::one_dark();
    dock_timelines(&mut app, waves, pipeline, Axis::Vertical);
    timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
    let split = app
        .panels
        .pipeline(pipeline)
        .unwrap()
        .last_layout()
        .label_split;
    drag_divider(&mut app, pipeline, split, 525.0);
    for width in [360.0, 250.0, 100.0, 1200.0] {
        timeline_frames(&mut app, waves, pipeline, width, &theme);
        let expected = 500.0_f32.min((width - 160.0).max(0.0));
        assert_eq!(aligned_metadata(&app, waves, pipeline), expected);
    }
    let saved = Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    assert_eq!(saved.timeline_dividers.len(), 1);
    assert_eq!(saved.timeline_dividers[0].width, 500.0);
    let mut restored = App::new();
    restored.set_session(session);
    Workspace::parse(&saved.to_bytes().unwrap())
        .unwrap()
        .prepare(
            &restored,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut restored)
        .unwrap();
    timeline_frames(&mut restored, waves, pipeline, 1200.0, &theme);
    assert_eq!(aligned_metadata(&restored, waves, pipeline), 500.0);

    let mut invalid = serde_json::to_value(&saved).unwrap();
    for divider in [
        serde_json::json!({"panels":[waves,pipeline],"width":-1}),
        serde_json::json!({"panels":[waves],"width":500}),
        serde_json::json!({"panels":[waves,999_999],"width":500}),
    ] {
        invalid["timeline_dividers"] = serde_json::json!([divider]);
        assert!(
            Workspace::parse(&serde_json::to_vec(&invalid).unwrap())
                .unwrap()
                .prepare(
                    &restored,
                    "file:///tmp/trace.vtr",
                    "file:///tmp/trace.vtr.volna.json"
                )
                .is_err()
        );
    }
    timeline_frames(&mut restored, waves, pipeline, 1200.0, &theme);
    assert_eq!(aligned_metadata(&restored, waves, pipeline), 500.0);
}

#[test]
fn timeline_columns_include_inactive_tabs_and_exclude_different_horizontal_spans() {
    use volna_core::panels::Axis;
    let (mut app, _, waves, pipeline) = opened(30);
    dock_timelines(&mut app, waves, pipeline, Axis::Horizontal);
    app.handle(Command::Panels(PanelsCommand::Split {
        panel: waves,
        axis: Axis::Vertical,
    }));
    let side = app.panels.focused_id();
    app.handle(Command::Panels(PanelsCommand::NewTab { group_of: waves }));
    let inactive = app.panels.focused_id();
    app.panels.waves_mut(inactive).unwrap().names_width = 500.0;
    let tabs = Layout::Tabs {
        tabs: vec![waves, inactive],
        active: waves,
    };
    set_center(
        &mut app,
        Layout::Split {
            split: Axis::Horizontal,
            sizes: vec![0.4, 0.3, 0.3],
            children: vec![tabs.clone(), Layout::single(pipeline), Layout::single(side)],
        },
    );
    let stack = |children| Layout::Split {
        split: Axis::Vertical,
        sizes: vec![0.5, 0.5],
        children,
    };
    set_center(
        &mut app,
        Layout::Split {
            split: Axis::Horizontal,
            sizes: vec![0.7, 0.3],
            children: vec![
                stack(vec![tabs.clone(), Layout::single(pipeline)]),
                Layout::single(side),
            ],
        },
    );
    let theme = Theme::one_dark();
    timeline_frames(&mut app, waves, pipeline, 1200.0, &theme);
    assert_eq!(aligned_metadata(&app, waves, pipeline), 620.0);
    app.handle(Command::Panels(PanelsCommand::Focus(inactive)));
    timeline_frames(&mut app, inactive, pipeline, 1200.0, &theme);
    assert_eq!(aligned_metadata(&app, inactive, pipeline), 620.0);
    app.layout_panel(
        side,
        Rect::from_xywh(1240.0, 20.0, 500.0, 600.0),
        &theme,
        1.0,
    )
    .unwrap();
    assert!(
        !app.panels
            .waves(side)
            .unwrap()
            .last_layout()
            .shared_metadata
    );

    set_center(
        &mut app,
        stack(vec![
            Layout::single(pipeline),
            Layout::Split {
                split: Axis::Horizontal,
                sizes: vec![0.5, 0.5],
                children: vec![tabs, Layout::single(side)],
            },
        ]),
    );
    let saved = Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    assert!(saved.timeline_dividers.is_empty());
}

#[test]
fn names_divider_in_a_narrow_stack_keeps_a_restorable_preference() {
    let (mut app, session, waves, pipeline) = opened(30);
    let theme = Theme::one_dark();
    dock_timelines(
        &mut app,
        waves,
        pipeline,
        volna_core::panels::Axis::Vertical,
    );
    for width in [250.0, 100.0] {
        timeline_frames(&mut app, waves, pipeline, width, &theme);
        let split = app.panels.waves(waves).unwrap().last_layout().names_split;
        drag_divider(&mut app, waves, split, 25.0);
        let saved =
            Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();
        let mut restored = App::new();
        restored.set_session(session.clone());
        Workspace::parse(&saved.to_bytes().unwrap())
            .unwrap()
            .prepare(
                &restored,
                "file:///tmp/trace.vtr",
                "file:///tmp/trace.vtr.volna.json",
            )
            .unwrap()
            .commit(&mut restored)
            .unwrap();
        timeline_frames(&mut restored, waves, pipeline, 1200.0, &theme);
        aligned_metadata(&restored, waves, pipeline);
    }
}

#[test]
fn saved_timeline_membership_uses_the_dock_tree_without_settings() {
    use volna_core::panels::Axis;
    let (mut app, session, waves, pipeline) = opened(30);
    app.handle(Command::Settings(volna_core::app::SettingsCommand::Open));
    let settings = app.panels.settings_id().unwrap();
    set_center(
        &mut app,
        Layout::Split {
            split: Axis::Vertical,
            sizes: vec![0.5, 0.5],
            children: vec![
                Layout::single(waves),
                Layout::Split {
                    split: Axis::Horizontal,
                    sizes: vec![0.5, 0.5],
                    children: vec![Layout::single(pipeline), Layout::single(settings)],
                },
            ],
        },
    );
    let saved = Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    let mut restored = App::new();
    restored.set_session(session);
    Workspace::parse(&saved.to_bytes().unwrap())
        .unwrap()
        .prepare(
            &restored,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut restored)
        .unwrap();
    timeline_frames(&mut restored, waves, pipeline, 1200.0, &Theme::one_dark());
    aligned_metadata(&restored, waves, pipeline);
}
