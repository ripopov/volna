//! Bus views exercise the same editing, loading, painting and persistence paths as signals.

use std::sync::Arc;

use volna_core::Theme;
use volna_core::app::{Action, App, Command, EditTarget};
use volna_core::geometry::{Modifiers, MouseButton, Rect, point};
use volna_core::panels::PanelId;
use volna_core::scene::{MonoMeasure, Scene};
use volna_core::session::LoadRequest;
use volna_core::testing::{a, a_all};
use volna_core::wave::layout::{CHEVRON_W, indent_x};
use volna_core::wave::model::MenuAction;
use volna_core::wave::tree::{self, Place};
use volna_core::wave::{PointerEvent, Tint, WaveModel};
use volna_core::workspace::Workspace;
use volna_trace::data::{SignalHistory, WaveValue};
use volna_trace::session::OpenSpec;
use vtr::Recorder;

const TRACE: &str = "file:///tmp/buses.vtr";
const LOCATION: &str = "file:///tmp/buses.volna.json";

fn fixture() -> (tempfile::NamedTempFile, App, PanelId) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let scope = writer
        .scope_with(
            vtr::SourceOptions::new("top", None),
            vtr::ScopeType::Module,
            "",
        )
        .unwrap();
    let mut signal = |name, width| {
        writer
            .signal(
                vtr::SourceOptions::new(name, Some(scope)),
                vtr::SignalKind::Bits { width, states: 4 },
                vtr::VarType::Wire,
                vtr::Direction::Output,
            )
            .unwrap()
    };
    let data = signal("data [7:4]", 4);
    let ascending = signal("ascending [4:7]", 4);
    let wide = signal("wide", 256);
    let scalar = signal("ready", 1);
    for (time, value) in [(5, 0), (10, 1), (15, 3), (20, 2), (25, 2)] {
        writer.advance_time(time).unwrap();
        writer.emit_u64(data, value).unwrap();
        writer.emit_u64(ascending, value).unwrap();
        let mut bytes = [0; 64];
        bytes[0] = value as u8;
        writer.emit_packed(wide, 4, &bytes).unwrap();
        writer.emit_u64(scalar, value & 1).unwrap();
    }
    writer.advance_time(30).unwrap();
    writer.emit_packed(data, 4, &[0b10_11_00_01]).unwrap();
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    app.handle(Command::AddVars(a_all(0..4)));
    pump(&mut app);
    let id = app.panels.focused_id();
    frame(&mut app, id, true);
    (file, app, id)
}

fn pump(app: &mut App) {
    for _ in 0..20 {
        let requests = app.take_requests();
        if requests.is_empty() {
            return;
        }
        for request in requests {
            app.deliver(request.perform());
        }
    }
    panic!("loads did not settle");
}

fn waves(app: &App, id: PanelId) -> &WaveModel {
    app.panels.waves(id).unwrap()
}

fn frame(app: &mut App, id: PanelId, dark: bool) -> &Scene {
    let theme = Theme::volna(dark);
    app.layout_panel(
        id,
        Rect::from_xywh(0.0, 0.0, 1100.0, 500.0),
        &theme,
        1.0,
        &mut MonoMeasure,
    )
    .unwrap();
    app.render_panel(id, &theme, &mut MonoMeasure)
}

fn select(app: &mut App, id: PanelId, row: usize) {
    let w = app.panels.waves_mut(id).unwrap();
    w.selected = [row].into();
    w.anchor = Some(row);
}

fn menu(app: &mut App, id: PanelId, row: usize, action: MenuAction) {
    select(app, id, row);
    app.handle(Command::OpenSignalMenu(id));
    app.handle(Command::MenuSelect(id, action));
}

fn history(app: &App, id: PanelId, row: usize) -> Arc<dyn SignalHistory> {
    waves(app, id).signal(row).unwrap().history.clone().unwrap()
}

#[test]
fn chevrons_preserve_the_parent_and_bits_have_their_own_edges() {
    let (_file, mut app, id) = fixture();
    let before = history(&app, id, 0);
    let layout = waves(&app, id).last_layout().clone();
    let top = layout.entry_span(0).unwrap().0;
    let p = point(
        indent_x(layout.name_left, 0, layout.zoom) + CHEVRON_W * layout.zoom / 2.0,
        top + layout.row_h / 2.0,
    );
    app.handle(Command::Pointer(
        id,
        PointerEvent::Down {
            position: p,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    app.handle(Command::Pointer(id, PointerEvent::Up));
    pump(&mut app);
    assert!(Arc::ptr_eq(&before, &history(&app, id, 0)));
    assert_eq!(
        waves(&app, id)
            .items()
            .iter()
            .take(5)
            .map(|e| e.name())
            .collect::<Vec<_>>(),
        ["data [7:4]", "data[7]", "data[6]", "data[5]", "data[4]"]
    );
    let bit = history(&app, id, 4);
    assert_eq!(
        (0..bit.len()).map(|i| bit.time(i)).collect::<Vec<_>>(),
        [5, 10, 20, 30]
    );
    assert_eq!(bit.value(bit.index_at(4)), WaveValue::Bits("x".into()));
    assert_eq!(bit.value(bit.index_at(15)), WaveValue::Bits("1".into()));
    assert_eq!(
        history(&app, id, 1).value(history(&app, id, 1).index_at(30)),
        WaveValue::Bits("x".into())
    );
    assert_eq!(
        history(&app, id, 2).value(history(&app, id, 2).index_at(30)),
        WaveValue::Bits("z".into())
    );
    select(&mut app, id, 4);
    app.panels
        .waves_mut(id)
        .unwrap()
        .set_cursor(&mut app.doc, Some(10));
    app.handle(Command::Action(Action::NextEdge));
    assert_eq!(waves(&app, id).cursor(&app.doc), Some(20));
    app.handle(Command::Action(Action::PrevEdge));
    assert_eq!(waves(&app, id).cursor(&app.doc), Some(10));
    for dark in [true, false] {
        let scene = frame(&mut app, id, dark);
        assert!(scene.texts().any(|s| s == "data[4]"));
        assert_eq!(waves(&app, id).last_layout().entry_span(0).unwrap().0, top);
        let rows: Vec<_> = waves(&app, id).accessible_rows(&app.doc).collect();
        assert_eq!(rows[0].expanded, Some(true));
        assert_eq!(rows[4].level, 2);
    }
    let theme = Theme::volna(true);
    app.layout_panel(
        id,
        Rect::from_xywh(0.0, 0.0, 360.0, 320.0),
        &theme,
        1.0,
        &mut MonoMeasure,
    )
    .unwrap();
    let scene = app.render_panel(id, &theme, &mut MonoMeasure);
    for index in ["[7]", "[6]", "[5]", "[4]"] {
        assert!(
            scene.texts().any(|s| s.ends_with(index)),
            "narrow rows retain bit identity {index}"
        );
    }
}

#[test]
fn collapse_keeps_bit_settings_and_keyboard_navigation_returns_to_the_parent() {
    let (_file, mut app, id) = fixture();
    select(&mut app, id, 0);
    app.handle(Command::Action(Action::PanRight));
    pump(&mut app);
    app.handle(Command::Action(Action::PanRight));
    assert_eq!(waves(&app, id).anchor, Some(1));
    app.handle(Command::Action(Action::SetTint(Some(Tint::Pink))));
    app.handle(Command::Action(Action::IncreaseRowHeight));
    app.handle(Command::Action(Action::PanLeft));
    assert_eq!(waves(&app, id).anchor, Some(0));
    let steps = app.history.undo_steps().count();
    app.handle(Command::Action(Action::PanLeft));
    assert_eq!(waves(&app, id).visible().as_ref(), [0, 5, 6, 7]);
    assert_eq!(app.history.undo_steps().count(), steps);
    app.handle(Command::Action(Action::PanRight));
    assert_eq!(waves(&app, id).signal(1).unwrap().tint, Some(Tint::Pink));
    assert_eq!(waves(&app, id).items()[1].height().multiple(), 2);
    menu(&mut app, id, 0, MenuAction::Format("udec".into()));
    assert_eq!(waves(&app, id).signal(0).unwrap().translator.id(), "udec");
    assert_eq!(waves(&app, id).signal(1).unwrap().translator.id(), "bit");
    menu(&mut app, id, 0, MenuAction::ReverseBits);
    assert_eq!(waves(&app, id).items()[1].name(), "data[4]");
    app.handle(Command::Undo);
    assert_eq!(waves(&app, id).items()[1].name(), "data[7]");
}

#[test]
fn slices_use_declared_indices_and_stay_visible_without_the_bus() {
    let (_file, mut app, id) = fixture();
    menu(&mut app, id, 1, MenuAction::AddSlice);
    let edit = app.text_edit().unwrap();
    assert_eq!(
        edit.target,
        EditTarget::Slice {
            panel: id,
            entry: 1
        }
    );
    app.handle(Command::CommitText(edit.target, Some("5:7".into())));
    pump(&mut app);
    assert_eq!(waves(&app, id).items()[2].name(), "ascending[5:7]");
    let slice = history(&app, id, 2);
    assert_eq!(
        slice.value(slice.index_at(15)),
        WaveValue::Bits("011".into())
    );
    menu(&mut app, id, 0, MenuAction::AddSlice);
    let edit = app.text_edit().unwrap();
    app.handle(Command::CommitText(edit.target, Some("8".into())));
    assert!(app.text_edit().is_some(), "invalid input stays editable");
    app.handle(Command::CommitText(edit.target, Some("4:5".into())));
    pump(&mut app);
    let slice = history(&app, id, 1);
    assert_eq!(
        slice.value(slice.index_at(10)),
        WaveValue::Bits("10".into())
    );
    assert!(app.text_edit().is_none());
    select(&mut app, id, 0);
    app.handle(Command::Action(Action::PanRight));
    pump(&mut app);
    menu(&mut app, id, 4, MenuAction::AddIndependent);
    let independent = waves(&app, id).anchor.unwrap();
    assert_eq!(waves(&app, id).items()[independent].name(), "data[4]");
    assert!(Arc::ptr_eq(
        &history(&app, id, independent),
        &history(&app, id, 4)
    ));
    select(&mut app, id, 0);
    app.handle(Command::Action(Action::PanLeft));
    assert!(waves(&app, id).visible().contains(&(independent as u32)));
}

#[test]
fn slices_compose_and_expand_using_the_original_signal_positions() {
    let (_file, mut app, id) = fixture();
    menu(&mut app, id, 0, MenuAction::AddSlice);
    let edit = app.text_edit().unwrap();
    app.handle(Command::CommitText(edit.target, Some("4:6".into())));
    pump(&mut app);
    select(&mut app, id, 1);
    app.handle(Command::Action(Action::PanRight));
    pump(&mut app);
    assert_eq!(
        waves(&app, id).items()[1..5]
            .iter()
            .map(|e| e.name())
            .collect::<Vec<_>>(),
        ["data[4:6]", "data[4]", "data[5]", "data[6]"]
    );
    assert_eq!(
        history(&app, id, 2).value(history(&app, id, 2).index_at(10)),
        WaveValue::Bits("1".into())
    );
    assert_eq!(
        history(&app, id, 3).value(history(&app, id, 3).index_at(10)),
        WaveValue::Bits("0".into())
    );
    let saved = Workspace::capture(&app, volna_core::testing::paths(TRACE), None)
        .unwrap()
        .to_bytes()
        .unwrap();
    Workspace::parse(&saved)
        .unwrap()
        .prepare(&app, TRACE, LOCATION)
        .unwrap()
        .commit(&mut app)
        .unwrap();
    pump(&mut app);
    assert_eq!(
        waves(&app, id).items()[1..5]
            .iter()
            .map(|e| e.name())
            .collect::<Vec<_>>(),
        ["data[4:6]", "data[4]", "data[5]", "data[6]"]
    );
}

#[test]
fn malformed_saved_projections_are_rejected_before_installing_rows() {
    let (_file, mut app, id) = fixture();
    select(&mut app, id, 0);
    app.handle(Command::Action(Action::PanRight));
    let saved = serde_json::to_value(
        Workspace::capture(&app, volna_core::testing::paths(TRACE), None).unwrap(),
    )
    .unwrap();
    for (field, value) in [
        (
            "projection",
            serde_json::json!({"first": u32::MAX, "last": 0}),
        ),
        ("signal", serde_json::json!(["top", "ascending [4:7]"])),
    ] {
        let mut invalid = saved.clone();
        invalid["panels"][0]["rows"][0]["rows"][0][field] = value;
        let workspace = Workspace::parse(&serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(workspace.prepare(&app, TRACE, LOCATION).is_err());
        assert_eq!(waves(&app, id).items().len(), 8);
    }
}

#[test]
fn expanded_buses_remain_one_member_of_signal_groups() {
    let (_file, mut app, id) = fixture();
    select(&mut app, id, 0);
    app.handle(Command::Action(Action::PanRight));
    pump(&mut app);
    select(&mut app, id, 0);
    app.handle(Command::Action(Action::GroupSelection));
    let edit = app.text_edit().unwrap();
    app.handle(Command::CommitText(edit.target, Some("Payload".into())));
    assert_eq!(waves(&app, id).group_histories(0).len(), 1);
    assert_eq!(waves(&app, id).numeric_members(0), [1]);
    assert_eq!(
        tree::selected_leaves(waves(&app, id).items(), &[0].into()),
        [1]
    );
    app.handle(Command::Action(Action::Ungroup));
    assert_eq!(waves(&app, id).items()[0].name(), "data [7:4]");
    assert_eq!(waves(&app, id).items()[4].depth, 1);
    tree::validate(waves(&app, id).items()).unwrap();
}

#[test]
fn analog_summaries_distinguish_a_slice_from_its_parent() {
    use volna_core::analysis::AnalysisLoad;
    use volna_core::data::NumericKind;
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let scope = writer
        .scope_with(
            vtr::SourceOptions::new("top", None),
            vtr::ScopeType::Module,
            "",
        )
        .unwrap();
    let data = writer
        .signal(
            vtr::SourceOptions::new("data", Some(scope)),
            vtr::SignalKind::Bits {
                width: 4,
                states: 2,
            },
            vtr::VarType::Wire,
            vtr::Direction::Output,
        )
        .unwrap();
    for time in 0..20_000 {
        writer.advance_time(time).unwrap();
        writer.emit_u64(data, time % 16).unwrap();
    }
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    app.handle(Command::AddVars(a_all([0])));
    pump(&mut app);
    let id = app.panels.focused_id();
    app.panels
        .waves_mut(id)
        .unwrap()
        .add_slice(&app.doc, 0, "1:0")
        .unwrap();
    pump(&mut app);
    for row in [0, 1] {
        select(&mut app, id, row);
        app.handle(Command::Action(Action::ToggleAnalog));
    }
    pump(&mut app);
    let parent = history(&app, id, 0);
    let slice = history(&app, id, 1);
    let signal = waves(&app, id).items()[0].signal_ref().unwrap();
    let Some(AnalysisLoad::Ready(a)) =
        app.doc
            .analog_summary(signal, NumericKind::Unsigned, &parent)
    else {
        panic!("parent summary missing");
    };
    let Some(AnalysisLoad::Ready(b)) =
        app.doc
            .analog_summary(signal, NumericKind::Unsigned, &slice)
    else {
        panic!("slice summary missing");
    };
    assert!(!Arc::ptr_eq(a, b));
    assert!(a.matches(&parent, NumericKind::Unsigned));
    assert!(b.matches(&slice, NumericKind::Unsigned));
}

#[test]
fn expansion_clipboard_and_workspace_round_trip_preserve_owned_bits() {
    let (_file, mut app, id) = fixture();
    select(&mut app, id, 0);
    app.handle(Command::Action(Action::PanRight));
    pump(&mut app);
    menu(&mut app, id, 0, MenuAction::ReverseBits);
    select(&mut app, id, 0);
    app.handle(Command::Action(Action::CopySignals));
    assert_eq!(app.doc.copied_rows.len(), 5);
    select(&mut app, id, 7);
    app.handle(Command::Action(Action::PasteSignals));
    pump(&mut app);
    assert_eq!(waves(&app, id).items()[8].name(), "data [7:4]");
    assert_eq!(waves(&app, id).items()[9].name(), "data[4]");
    assert!(Arc::ptr_eq(&history(&app, id, 4), &history(&app, id, 12)));
    app.handle(Command::Undo);
    assert_eq!(waves(&app, id).items().len(), 8);
    app.handle(Command::Redo);
    pump(&mut app);
    assert_eq!(waves(&app, id).items().len(), 13);
    let saved = Workspace::capture(&app, volna_core::testing::paths(TRACE), None)
        .unwrap()
        .to_bytes()
        .unwrap();
    let plan = Workspace::parse(&saved)
        .unwrap()
        .prepare(&app, TRACE, LOCATION)
        .unwrap();
    assert!(plan.report().notices.is_empty());
    plan.commit(&mut app).unwrap();
    pump(&mut app);
    let again = Workspace::capture(&app, volna_core::testing::paths(TRACE), None)
        .unwrap()
        .to_bytes()
        .unwrap();
    assert_eq!(saved, again);
    tree::validate(waves(&app, id).items()).unwrap();
    select(&mut app, id, 7);
    let w = app.panels.waves_mut(id).unwrap();
    assert!(
        !w.move_selected_to(Place { at: 2, depth: 1 }),
        "ordinary signals cannot become bits"
    );
    select(&mut app, id, 1);
    assert!(
        !app.panels.waves_mut(id).unwrap().group_selected(),
        "a group cannot replace an owned bit"
    );
    select(&mut app, id, 0);
    app.handle(Command::Action(Action::RemoveSelected));
    assert_eq!(waves(&app, id).items().len(), 8);
    app.handle(Command::Undo);
    pump(&mut app);
    assert_eq!(waves(&app, id).items().len(), 13);
}

#[test]
fn wide_buses_only_prepare_visible_bits_and_late_results_cannot_fill_replacement_rows() {
    let (_file, mut app, id) = fixture();
    select(&mut app, id, 2);
    app.handle(Command::Action(Action::PanRight));
    let requests = app.take_requests();
    let projections = requests
        .iter()
        .filter(|r| matches!(r, LoadRequest::Projection { .. }))
        .count();
    assert!(
        projections > 0 && projections < 40,
        "only viewport bits are prepared: {projections}"
    );
    assert_eq!(waves(&app, id).items().len(), 260);
    for request in requests {
        app.deliver(request.perform());
    }
    let constant = history(&app, id, 3);
    assert_eq!(
        constant.len(),
        1,
        "other bits' changes never become dense activity"
    );
    select(&mut app, id, 2);
    app.handle(Command::Action(Action::RemoveSelected));
    pump(&mut app);
    assert_eq!(waves(&app, id).items().len(), 3);
    app.handle(Command::AddVars(vec![a(2)]));
    select(&mut app, id, 3);
    app.handle(Command::Action(Action::PanRight));
    let pending = app.take_requests();
    app.close_trace();
    drop(app.take_requests());
    for request in pending {
        app.deliver(request.perform());
    }
    assert!(
        app.panels
            .iter()
            .all(|p| p.kind.waves().is_none_or(|w| w.items().is_empty()))
    );
}
