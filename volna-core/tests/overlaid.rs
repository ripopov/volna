//! Group overlays: shared scales, independent gaps and envelopes, exact
//! probes, row edits, workspace persistence, and worker summary lifetimes.

use std::sync::Arc;
use volna_core::Theme;
use volna_core::app::{Action, App, Command};
use volna_core::data::NumericKind;
use volna_core::document::SummaryLoad;
use volna_core::geometry::{Point, Rect, point};
use volna_core::panels::PanelId;
use volna_core::scene::{MonoMeasure, Prim, Scene};
use volna_core::testing::a;
use volna_core::trace::TraceId;
use volna_core::wave::analog::{self, AnalogDraw, AnalogRange, Plot, Series};
use volna_core::wave::model::MenuAction;
use volna_core::wave::{GroupStyle, PointerEvent};
use volna_core::workspace::Workspace;
use volna_trace::data::history::VecHistory;
use volna_trace::data::source::Lookup;
use volna_trace::data::{SignalHistory, SignalShape, WaveValue};
use volna_trace::session::OpenSpec;

const TRACE: &str = "file:///tmp/overlaid.vtr";
const LOCATION: &str = "file:///tmp/overlaid.vtr.volna.json";

/// The group contains two 8-bit buses, a bit, text, and an event. Values
/// change at different times; q is undefined from 40 through 49 ns.
fn fixture(changes: usize) -> (tempfile::NamedTempFile, App, PanelId) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut w = vtr::Writer::create(file.path()).unwrap();
    w.set_timescale(vtr::Timescale::Exponent(-9)).unwrap();
    let top = w
        .add_scope(None, "top", vtr::ScopeType::Module, "top")
        .unwrap();
    let mut var = |name, typ, kind| {
        w.add_var(Some(top), name, typ, vtr::Direction::Output, kind)
            .unwrap()
            .1
    };
    let p = var(
        "p",
        vtr::VarType::Wire,
        vtr::SignalKind::Bits {
            width: 8,
            states: 4,
        },
    );
    let q = var(
        "q",
        vtr::VarType::Wire,
        vtr::SignalKind::Bits {
            width: 8,
            states: 4,
        },
    );
    let bit = var(
        "bit",
        vtr::VarType::Wire,
        vtr::SignalKind::Bits {
            width: 1,
            states: 2,
        },
    );
    let mode = var("mode", vtr::VarType::String, vtr::SignalKind::VarLen);
    let event = var(
        "tick",
        vtr::VarType::Event,
        vtr::SignalKind::Bits {
            width: 1,
            states: 2,
        },
    );
    for t in 0..changes.max(101) as u64 {
        w.set_time(t).unwrap();
        if changes > 101 {
            w.emit_u64(p, if t == 77 { 200 } else { t % 2 }).unwrap();
            w.emit_u64(q, 3 + t % 2).unwrap();
        } else {
            match t {
                0 => {
                    w.emit_u64(p, 2).unwrap();
                    w.emit_u64(q, 3).unwrap();
                }
                40 => w.emit_logic_str(q, b"xxxxxxxx").unwrap(),
                50 => w.emit_u64(q, 4).unwrap(),
                70 => w.emit_u64(p, 250).unwrap(),
                _ => {}
            }
        }
        if t == 0 {
            w.emit_bit(bit, vtr::Logic::One).unwrap();
            w.emit_varlen(mode, b"run").unwrap();
        }
        if t == 60 {
            w.emit_bit(bit, vtr::Logic::Zero).unwrap();
        }
        w.emit_bit(event, vtr::Logic::One).unwrap();
    }
    w.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    pump(&mut app);
    let Lookup::Found(top) = app.doc.hierarchy(TraceId::A).unwrap().find_scope(&["top"]) else {
        panic!();
    };
    app.handle(Command::AddScopeAsGroup {
        scope: a(top),
        recursive: false,
    });
    pump(&mut app);
    let id = app.panels.focused_id();
    frame(&mut app, id, &Theme::volna(true));
    (file, app, id)
}

fn pump(app: &mut App) {
    loop {
        let requests = app.take_requests();
        if requests.is_empty() {
            break;
        }
        for request in requests {
            app.deliver(request.perform());
        }
    }
}

fn frame<'a>(app: &'a mut App, id: PanelId, t: &Theme) -> &'a Scene {
    app.layout_panel(id, Rect::from_xywh(0.0, 0.0, 1200.0, 600.0), t)
        .unwrap();
    app.render_panel(id, t, &mut MonoMeasure)
}

fn choose(app: &mut App, id: PanelId, row: usize, action: MenuAction) {
    app.panels
        .waves_mut(id)
        .unwrap()
        .open_signal_menu(&app.doc, row, Point::default());
    app.handle(Command::MenuSelect(id, action));
}

fn style(app: &App, id: PanelId) -> GroupStyle {
    app.panels.waves(id).unwrap().items()[0]
        .group()
        .unwrap()
        .style
}

fn view(app: &mut App, id: PanelId, start: f64, end: f64) {
    let App { panels, doc, .. } = app;
    panels
        .waves_mut(id)
        .unwrap()
        .nav
        .jump_to(doc, volna_core::wave::Viewport { start, end });
}

fn settle(app: &mut App) {
    let now = web_time::Instant::now();
    for i in 0..100 {
        app.tick(now + std::time::Duration::from_millis(i * 20));
    }
}

fn bounds(scene: &Scene) -> (String, String) {
    let labels: Vec<_> = scene
        .texts()
        .filter(|s| s.starts_with("max ") || s.starts_with("min "))
        .collect();
    (labels[0].to_owned(), labels[1].to_owned())
}

#[test]
fn group_styles_switch_as_single_steps_and_preserve_height_and_options() {
    let (_f, mut app, id) = fixture(101);
    app.handle(Command::Action(Action::ToggleOverlaid));
    assert_eq!(
        style(&app, id),
        GroupStyle::Overlaid {
            draw: AnalogDraw::Step,
            range: AnalogRange::Trace
        }
    );
    assert_eq!(app.undo_label(), Some("Overlay top"));
    assert_eq!(
        app.panels.waves(id).unwrap().items()[0].height().multiple(),
        3
    );
    app.handle(Command::Undo);
    assert_eq!(style(&app, id), GroupStyle::Activity);
    app.handle(Command::Redo);
    choose(
        &mut app,
        id,
        0,
        MenuAction::OverlaidDraw(AnalogDraw::Linear),
    );
    choose(
        &mut app,
        id,
        0,
        MenuAction::OverlaidRange(AnalogRange::Type),
    );
    let label = app.undo_label().map(str::to_owned);
    choose(&mut app, id, 0, MenuAction::Overlaid);
    assert_eq!(
        app.undo_label(),
        label.as_deref(),
        "reselecting the style is a no-op"
    );
    assert_eq!(
        style(&app, id),
        GroupStyle::Overlaid {
            draw: AnalogDraw::Linear,
            range: AnalogRange::Type
        }
    );
    app.handle(Command::Action(Action::ToggleStack));
    assert_eq!(style(&app, id), GroupStyle::Stack { peak: true });
    app.handle(Command::Action(Action::ToggleOverlaid));
    app.handle(Command::Action(Action::ToggleOverlaid));
    assert_eq!(style(&app, id), GroupStyle::Activity);
    assert_eq!(
        app.panels.waves(id).unwrap().items()[0].height().multiple(),
        1
    );
    app.handle(Command::Action(Action::ToggleOverlaid));
    app.handle(Command::Action(Action::IncreaseRowHeight));
    app.handle(Command::Action(Action::ToggleOverlaid));
    assert_eq!(
        app.panels.waves(id).unwrap().items()[0].height().multiple(),
        4
    );
}

#[test]
fn menus_hide_isolate_restore_and_undo_without_removing_member_rows() {
    let (_f, mut app, id) = fixture(101);
    app.handle(Command::Action(Action::ToggleOverlaid));
    let w = app.panels.waves_mut(id).unwrap();
    w.open_signal_menu(&app.doc, 0, Point::default());
    let menu = w.menu.as_ref().unwrap();
    let overlay = menu
        .items()
        .find(|i| i.action == MenuAction::Overlaid)
        .unwrap();
    assert_eq!(
        (
            overlay.label.as_str(),
            overlay.badge.as_deref(),
            overlay.checked
        ),
        ("Overlaid lines", Some("⇧O"), true)
    );
    assert!(
        !menu
            .items()
            .find(|i| i.action == MenuAction::Stack(false))
            .unwrap()
            .checked
    );
    assert!(menu.items().any(|i| i.label == "Visible window"));
    w.selected.clear();
    choose(&mut app, id, 2, MenuAction::OverlayHidden(true));
    assert!(
        app.panels
            .waves(id)
            .unwrap()
            .signal(2)
            .unwrap()
            .overlay_hidden
    );
    assert_eq!(app.panels.waves(id).unwrap().items().len(), 6);
    choose(&mut app, id, 1, MenuAction::IsolateOverlay);
    assert_eq!(
        (1..=3)
            .map(|i| app
                .panels
                .waves(id)
                .unwrap()
                .signal(i)
                .unwrap()
                .overlay_hidden)
            .collect::<Vec<_>>(),
        [false, true, true]
    );
    app.handle(Command::Undo);
    assert!(
        !app.panels
            .waves(id)
            .unwrap()
            .signal(3)
            .unwrap()
            .overlay_hidden
    );
    choose(&mut app, id, 1, MenuAction::ShowAllOverlay);
    assert!((1..=3).all(|i| {
        !app.panels
            .waves(id)
            .unwrap()
            .signal(i)
            .unwrap()
            .overlay_hidden
    }));
}

#[test]
fn shared_scale_fits_members_and_hidden_members_do_not_affect_it() {
    let (_f, mut app, id) = fixture(101);
    let t = Theme::volna(true);
    app.handle(Command::Action(Action::ToggleOverlaid));
    assert_eq!(
        bounds(frame(&mut app, id, &t)),
        ("max 250".into(), "min 0".into())
    );
    view(&mut app, id, 0.0, 30.0);
    assert_eq!(
        bounds(frame(&mut app, id, &t)).0,
        "max 250",
        "whole trace is stable while panning"
    );
    choose(
        &mut app,
        id,
        0,
        MenuAction::OverlaidRange(AnalogRange::Window),
    );
    frame(&mut app, id, &t);
    settle(&mut app);
    assert_eq!(
        bounds(frame(&mut app, id, &t)),
        ("max 3".into(), "min 0".into())
    );
    choose(
        &mut app,
        id,
        0,
        MenuAction::OverlaidRange(AnalogRange::Type),
    );
    frame(&mut app, id, &t);
    settle(&mut app);
    assert_eq!(
        bounds(frame(&mut app, id, &t)),
        ("max 255".into(), "min 0".into())
    );
    app.panels.waves_mut(id).unwrap().selected.clear();
    choose(&mut app, id, 1, MenuAction::Format("sdec".into()));
    frame(&mut app, id, &t);
    settle(&mut app);
    assert_eq!(
        bounds(frame(&mut app, id, &t)),
        ("max 255".into(), "min -128".into())
    );
    choose(&mut app, id, 1, MenuAction::OverlayHidden(true));
    frame(&mut app, id, &t);
    settle(&mut app);
    assert_eq!(
        bounds(frame(&mut app, id, &t)),
        ("max 255".into(), "min 0".into())
    );
}

#[test]
fn both_themes_draw_independent_gaps_and_exact_held_probe_values() {
    for dark in [false, true] {
        let (_f, mut app, id) = fixture(101);
        let t = Theme::volna(dark);
        app.handle(Command::Action(Action::ToggleOverlaid));
        view(&mut app, id, 0.0, 100.0);
        let scene = frame(&mut app, id, &t);
        assert!(scene.texts().any(|s| s == "overlaid · 3/3 lines"));
        assert!(
            scene.texts().any(|s| s.starts_with("not ove")),
            "{:?}",
            scene.texts().collect::<Vec<_>>()
        );
        assert!(!scene.texts().any(|s| s.starts_with('Σ')));
        let l = app.panels.waves(id).unwrap().last_layout().clone();
        let x = |time: f64| l.waves.left() + time as f32 / 100.0 * l.waves.width().floor();
        let row_top = l.row_y(0);
        let row_bottom = row_top + l.row_height(0);
        let segments = |scene: &Scene, k: usize| {
            scene
                .prims
                .iter()
                .filter_map(|p| match p {
                    Prim::Lines {
                        segments, color, ..
                    } if *color == t.markers[k].stroke => Some(segments),
                    _ => None,
                })
                .flatten()
                .filter(|pair| {
                    pair[0].y >= row_top && pair[1].y < row_bottom && pair[0].x >= l.waves.left()
                })
                .copied()
                .collect::<Vec<_>>()
        };
        let scene = frame(&mut app, id, &t);
        assert!(
            segments(scene, 0)
                .iter()
                .any(|pair| pair[0].x < x(45.0) && pair[1].x > x(45.0)),
            "p continues while q is X"
        );
        assert!(
            !segments(scene, 1)
                .iter()
                .any(|pair| pair[0].x < x(45.0) && pair[1].x > x(45.0)),
            "q has its own gap"
        );
        choose(
            &mut app,
            id,
            0,
            MenuAction::OverlaidDraw(AnalogDraw::Linear),
        );
        frame(&mut app, id, &t);
        app.handle(Command::Pointer(
            id,
            PointerEvent::Move {
                position: point(x(45.0), row_top + 25.0),
            },
        ));
        let texts: Vec<_> = frame(&mut app, id, &t).texts().map(str::to_owned).collect();
        assert!(
            texts.iter().any(|s| s == "02"),
            "held p value, not interpolated: {texts:?}"
        );
        assert!(texts.iter().any(|s| s.contains("exact held values")));
        assert!(
            texts.iter().any(|s| s == "X" || s == "xx"),
            "q is undefined: {texts:?}"
        );
        let accessible: Vec<_> = app
            .panels
            .waves(id)
            .unwrap()
            .accessible_rows(&app.doc)
            .map(|r| r.label)
            .collect();
        assert!(accessible[0].contains("overlaid lines"));
        app.handle(Command::Action(Action::PanLeft));
        assert!(
            frame(&mut app, id, &t)
                .texts()
                .any(|s| s == "overlaid · 3/3 lines")
        );
    }
}

#[test]
fn stacked_and_overlaid_members_share_colors_stripes_and_solid_swatches() {
    use volna_core::wave::Tint;

    for dark in [false, true] {
        let (_f, mut app, id) = fixture(101);
        let t = Theme::volna(dark);
        view(&mut app, id, 0.0, 100.0);
        choose(&mut app, id, 2, MenuAction::Tint(Some(Tint::Blue)));
        choose(&mut app, id, 1, MenuAction::Draw(Some(AnalogDraw::Step)));
        let App { panels, doc, .. } = &mut app;
        panels.waves_mut(id).unwrap().set_cursor(doc, Some(20));
        let expected = [
            t.markers[0].stroke,
            t.ink(Some(Tint::Blue)),
            t.markers[2].stroke,
        ];
        for mode in [
            MenuAction::Overlaid,
            MenuAction::Stack(true),
            MenuAction::Overlaid,
        ] {
            choose(&mut app, id, 0, mode);
            frame(&mut app, id, &t);
            let layout = app.panels.waves(id).unwrap().last_layout().clone();
            let scene = frame(&mut app, id, &t);
            for (entry, color) in (1..=3).zip(expected) {
                let (y, height) = layout.entry_span(entry).unwrap();
                assert!(
                    scene.quads().any(|(rect, fill)| {
                        fill == color
                            && rect == Rect::from_xywh(layout.names.left(), y, 3.0, height)
                    }),
                    "member {entry} has a stripe in its resolved color"
                );
                assert!(
                    scene.prims.iter().any(|prim| {
                        let in_wave =
                            |p: Point| p.x >= layout.waves.left() && p.y >= y && p.y < y + height;
                        match prim {
                            Prim::Lines {
                                segments,
                                color: stroke,
                                ..
                            } => {
                                *stroke == color
                                    && segments.iter().any(|[a, b]| in_wave(*a) && in_wave(*b))
                            }
                            Prim::Quad { rect, fill, .. } => *fill == color && in_wave(rect.origin),
                            _ => false,
                        }
                    }),
                    "member {entry} uses the same color on its waveform"
                );
                assert!(
                    scene.prims.iter().any(|prim| matches!(prim,
                        Prim::Lines { segments, color: stroke, .. }
                        if *stroke == color && segments.iter().any(|[a, b]| {
                            layout.names.contains(*a) && layout.names.contains(*b)
                                && a.y >= y && a.y < y + height && a.y == b.y && b.x - a.x == 9.0
                        })
                    )),
                    "member {entry} has a solid line swatch"
                );
                assert!(
                    !scene.prims.iter().any(|prim| matches!(prim,
                        Prim::Quad { rect, fill, radius, .. }
                        if *fill == color && *radius > 0.0 && layout.names.contains(rect.origin)
                    )),
                    "member names have no square swatches"
                );
            }
            assert!(
                scene.prims.iter().any(|prim| matches!(prim,
                    Prim::Quad { rect, fill, radius, .. }
                    if *fill == expected[0] && *radius > 0.0
                        && layout.waves.contains(rect.origin)
                        && rect.top() >= layout.entry_span(1).unwrap().0
                )),
                "the analog cursor dot matches the member waveform"
            );
            let stripes = scene
                .quads()
                .filter(|(rect, _)| rect.left() == layout.names.left() && rect.width() == 3.0)
                .count();
            assert_eq!(stripes, 3, "nonnumeric rows have no automatic stripe");
        }
        choose(&mut app, id, 0, MenuAction::Stack(false));
        let layout = app.panels.waves(id).unwrap().last_layout().clone();
        let stripes = frame(&mut app, id, &t)
            .quads()
            .filter(|(rect, _)| rect.left() == layout.names.left() && rect.width() == 3.0)
            .count();
        assert_eq!(
            stripes, 1,
            "leaving group plots retains the manual color only"
        );
    }
}

#[test]
fn independent_envelopes_preserve_a_single_sample_peak_and_solid_strokes() {
    let history: Arc<dyn SignalHistory> = Arc::new(VecHistory {
        shape: SignalShape::Vector { width: 8 },
        times: (0..1000).collect(),
        values: (0..1000)
            .map(|i| WaveValue::Bits(format!("{:08b}", if i == 77 { 200 } else { 1 })))
            .collect(),
        initial: WaveValue::Unavailable,
    });
    let vp = volna_core::wave::Viewport {
        start: 0.0,
        end: 1000.0,
    };
    let plot = Plot {
        left: 0.0,
        width: 20.0,
        top: 0.0,
        bottom: 100.0,
        lo: 0.0,
        hi: 200.0,
    };
    let g = analog::geometry(
        &Series::new(&history, NumericKind::Unsigned, None),
        AnalogDraw::Step,
        &vp,
        &plot,
        1.0,
    );
    assert!(g.envelope);
    let lines =
        volna_core::wave::overlaid::strokes(&g.runs, Rect::from_xywh(0.0, 0.0, 20.0, 100.0));
    assert!(lines.iter().flatten().any(|p| p.y == 0.0), "peak survives");
    let run = vec![point(0.0, 0.0), point(30.0, 0.0), point(30.0, 20.0)];
    let expected = vec![[run[0], run[1]], [run[1], run[2]]];
    let strokes =
        volna_core::wave::overlaid::strokes(&[run], Rect::from_xywh(0.0, 0.0, 40.0, 40.0));
    assert_eq!(strokes, expected);
}

#[test]
fn long_overlay_members_share_analog_summaries_and_release_them_on_style_change() {
    let (_f, mut app, id) = fixture(20_000);
    let signal = app.panels.waves(id).unwrap().items()[1]
        .signal_ref()
        .unwrap();
    assert!(
        app.doc
            .analog_summary(signal, NumericKind::Unsigned)
            .is_none()
    );
    app.handle(Command::Action(Action::ToggleOverlaid));
    assert!(matches!(
        app.doc.analog_summary(signal, NumericKind::Unsigned),
        Some(SummaryLoad::Building { .. })
    ));
    assert!(
        frame(&mut app, id, &Theme::volna(true))
            .texts()
            .any(|s| s.contains("Summarizing"))
    );
    pump(&mut app);
    assert!(matches!(
        app.doc.analog_summary(signal, NumericKind::Unsigned),
        Some(SummaryLoad::Ready(_))
    ));
    assert!(
        !frame(&mut app, id, &Theme::volna(true))
            .texts()
            .any(|s| s.contains("Summarizing"))
    );
    app.handle(Command::Action(Action::ToggleOverlaid));
    assert!(
        app.doc
            .analog_summary(signal, NumericKind::Unsigned)
            .is_none()
    );
    app.handle(Command::Undo);
    pump(&mut app);
    assert!(matches!(
        app.doc.analog_summary(signal, NumericKind::Unsigned),
        Some(SummaryLoad::Ready(_))
    ));
}

#[test]
fn overlay_workspace_round_trips_settings_and_visibility_at_version_seven() {
    let (_f, mut app, id) = fixture(101);
    app.handle(Command::Action(Action::ToggleOverlaid));
    choose(
        &mut app,
        id,
        0,
        MenuAction::OverlaidDraw(AnalogDraw::Linear),
    );
    choose(
        &mut app,
        id,
        0,
        MenuAction::OverlaidRange(AnalogRange::Type),
    );
    app.panels.waves_mut(id).unwrap().selected.clear();
    choose(&mut app, id, 2, MenuAction::OverlayHidden(true));
    let saved = serde_json::to_value(
        Workspace::capture(&app, volna_core::testing::paths(TRACE), None).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["panels"][0]["version"], 7);
    assert_eq!(saved["panels"][0]["rows"][0]["style"], "overlaid");
    assert_eq!(saved["panels"][0]["rows"][0]["overlaid"]["draw"], "linear");
    let (_f2, mut restored, _) = fixture(101);
    Workspace::parse(&serde_json::to_vec(&saved).unwrap())
        .unwrap()
        .prepare(&restored, TRACE, LOCATION)
        .unwrap()
        .commit(&mut restored)
        .unwrap();
    pump(&mut restored);
    let rid = restored.panels.focused_id();
    assert_eq!(style(&restored, rid), style(&app, id));
    assert!(
        restored
            .panels
            .waves(rid)
            .unwrap()
            .signal(2)
            .unwrap()
            .overlay_hidden
    );
    let again = serde_json::to_value(
        Workspace::capture(&restored, volna_core::testing::paths(TRACE), None).unwrap(),
    )
    .unwrap();
    assert_eq!(again["panels"], saved["panels"]);
    let mut invalid = saved.clone();
    invalid["panels"][0]["rows"][0]
        .as_object_mut()
        .unwrap()
        .remove("overlaid");
    assert!(
        Workspace::parse(&serde_json::to_vec(&invalid).unwrap())
            .unwrap()
            .prepare(&restored, TRACE, LOCATION)
            .is_err()
    );
    let mut old = saved;
    old["panels"][0]["version"] = 6.into();
    let plan = Workspace::parse(&serde_json::to_vec(&old).unwrap())
        .unwrap()
        .prepare(&restored, TRACE, LOCATION)
        .unwrap();
    assert!(
        plan.report()
            .notices
            .iter()
            .any(|s| s.contains("waves version 6"))
    );
}

#[test]
fn overlay_navigation_uses_enabled_numeric_members_only() {
    let (_f, mut app, id) = fixture(101);
    app.handle(Command::Action(Action::ToggleOverlaid));
    let set_cursor = |app: &mut App| {
        let App { panels, doc, .. } = app;
        let w = panels.waves_mut(id).unwrap();
        w.selected = [0].into();
        w.anchor = Some(0);
        w.set_cursor(doc, Some(0));
    };
    set_cursor(&mut app);
    app.handle(Command::Action(Action::NextEdge));
    assert_eq!(app.panels.waves(id).unwrap().cursor(&app.doc), Some(40));
    app.panels.waves_mut(id).unwrap().selected.clear();
    choose(&mut app, id, 2, MenuAction::OverlayHidden(true));
    set_cursor(&mut app);
    app.handle(Command::Action(Action::NextEdge));
    assert_eq!(app.panels.waves(id).unwrap().cursor(&app.doc), Some(60));
    app.panels.waves_mut(id).unwrap().selected.clear();
    choose(&mut app, id, 3, MenuAction::OverlayHidden(true));
    set_cursor(&mut app);
    app.handle(Command::Action(Action::NextEdge));
    assert_eq!(app.panels.waves(id).unwrap().cursor(&app.doc), Some(70));
}

#[test]
fn overlay_strokes_clip_offscreen_samples_and_reject_nonfinite_points() {
    let bounds = Rect::from_xywh(0.0, 0.0, 100.0, 100.0);
    let runs = [vec![point(50.0, -1e20), point(50.0, 100.0)]];
    let strokes = volna_core::wave::overlaid::strokes(&runs, bounds);
    assert_eq!(strokes, vec![[point(50.0, 0.0), point(50.0, 100.0)]]);
    let bad = [vec![point(0.0, f32::INFINITY), point(50.0, 50.0)]];
    assert!(volna_core::wave::overlaid::strokes(&bad, bounds).is_empty());
}

#[test]
fn bit_overlays_preserve_weak_levels_and_leave_x_and_z_undefined() {
    let history: Arc<dyn SignalHistory> = Arc::new(VecHistory {
        shape: SignalShape::Bit,
        times: vec![0, 1, 2, 3],
        values: ["h", "l", "x", "z"]
            .map(|s| WaveValue::Bits(s.into()))
            .to_vec(),
        initial: WaveValue::Unavailable,
    });
    let series = Series::new(&history, NumericKind::Unsigned, None);
    assert_eq!(series.sample(Some(0)), analog::Sample::Value(1.0));
    assert_eq!(series.sample(Some(1)), analog::Sample::Value(0.0));
    assert_eq!(series.sample(Some(2)), analog::Sample::Undefined);
    assert_eq!(series.sample(Some(3)), analog::Sample::Undefined);
}
