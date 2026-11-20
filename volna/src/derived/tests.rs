use super::*;
use gpui_kit::{Modifiers, TestAppContext, VisualTestContext, size};
use volna_core::testing::a_all;
use volna_trace::session::OpenSpec;

#[gpui_kit::test]
fn formula_editor_creates_edits_and_removes_real_waveforms_in_both_themes(cx: &mut TestAppContext) {
    formula_editor_workflow(cx, false);
}

#[gpui_kit::test]
fn formula_editor_uses_a_signed_vector_and_produces_a_real_waveform(cx: &mut TestAppContext) {
    formula_editor_workflow(cx, true);
}

fn formula_editor_workflow(cx: &mut TestAppContext, signed: bool) {
    cx.update(crate::init_app);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let ws = workspace.unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.simulate_resize(size(px(1200.0), px(900.0)));
    ws.update(&mut vcx, |ws, cx| {
        let session = OpenSpec::Bytes {
            name: "analog.vtr".into(),
            bytes: include_bytes!("../../examples/analog_showcase.vtr").to_vec(),
        }
        .open()
        .unwrap();
        let var = if signed {
            session
                .hierarchy()
                .vars()
                .position(|v| matches!(v.shape, volna_trace::data::SignalShape::Vector { .. }))
                .unwrap()
        } else {
            0
        };
        ws.set_session(session, cx);
        ws.dispatch(Command::AddVars(a_all([var])), None, cx);
        if signed {
            let panel = ws.app.panels.focused_id();
            ws.dispatch(Command::OpenSignalMenu(panel), None, cx);
            ws.dispatch(
                Command::MenuSelect(
                    panel,
                    volna_core::wave::model::MenuAction::Format("sdec".into()),
                ),
                None,
                cx,
            );
        }
    });
    vcx.run_until_parked();
    let click = |vcx: &mut VisualTestContext, selector| {
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        vcx.simulate_click(bounds.center(), Modifiers::default());
        vcx.run_until_parked();
    };
    click(&mut vcx, "derive-signal");
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.derived_editor.is_some());
        assert!(matches!(
            ws.app.derived_preview(),
            Some(ComputationStatus::Ready(_))
        ));
        assert_eq!(ws.app.panels.focused_waves().unwrap().items().len(), 1);
        assert_eq!(
            ws.app.derived_editor.as_ref().unwrap().bindings[0].kind,
            if signed {
                volna_core::data::NumericKind::Signed
            } else {
                volna_core::data::NumericKind::Real
            }
        );
    });
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let field = vcx.debug_bounds("formula-expression").unwrap();
    vcx.simulate_click(
        gpui_kit::point(field.center().x, field.bottom() - px(10.0)),
        Modifiers::default(),
    );
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    let alias = ws.read_with(&vcx, |ws, _| {
        ws.app.derived_editor.as_ref().unwrap().bindings[0]
            .alias
            .clone()
    });
    vcx.simulate_input(&format!("moving_avg({alias}, 4 us, step: 2 ns)"));
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert!(
            ws.app
                .derived_editor
                .as_ref()
                .unwrap()
                .draft
                .formula
                .contains("4 us")
        );
        assert!(
            matches!(ws.app.derived_preview(), Some(ComputationStatus::Ready(_))),
            "error {:?}, status {}, draft {:?}",
            ws.app.derived_editor.as_ref().unwrap().error,
            match ws.app.derived_preview() {
                Some(ComputationStatus::Waiting) => "waiting",
                Some(ComputationStatus::Building) => "building",
                Some(ComputationStatus::Failed(error)) => error.as_str(),
                Some(ComputationStatus::Ready(_)) => "ready",
                Some(ComputationStatus::ReadyMeasurement(_)) => "measurement",
                Some(ComputationStatus::ReadyTable(_)) => "table",
                None => "none",
            },
            ws.app.derived_editor.as_ref().unwrap().draft
        );
    });
    click(&mut vcx, "derived-apply");
    ws.read_with(&vcx, |ws, _| {
        let waves = ws.app.panels.focused_waves().unwrap();
        assert_eq!(waves.items().len(), 2);
        assert_eq!(
            waves.signal(1).unwrap().shape,
            volna_trace::data::SignalShape::Real
        );
        assert_eq!(waves.signal(1).unwrap().translator.id(), "real");
        let volna_core::computed::Operation::Formula { expression, .. } = &waves
            .signal(1)
            .unwrap()
            .source
            .computed()
            .unwrap()
            .operation;
        assert!(expression.contains("4 us"));
    });
    vcx.simulate_keystrokes("shift-f10");
    vcx.run_until_parked();
    click(&mut vcx, "wave-menu-edit-formula");
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.derived_editor.as_ref().unwrap().editing);
    });
    for dark in [false, true] {
        vcx.update(|_, cx| crate::theme::set(crate::theme::CoreTheme::volna(dark), cx));
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let tray = vcx.debug_bounds("derived-signal-editor").unwrap();
        assert!(tray.size.width > px(280.0));
        let status = vcx.debug_bounds("derived-status").unwrap();
        assert!(status.left() >= tray.left() && status.right() <= tray.right());
    }
    click(&mut vcx, "derived-remove");
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.derived_editor.is_none());
        assert_eq!(ws.app.panels.focused_waves().unwrap().items().len(), 1);
    });
}

// A user searches for a hidden recorded input, and applies the live difference waveform.
#[gpui_kit::test]
fn formula_picker_and_expression_create_a_two_input_waveform(cx: &mut TestAppContext) {
    cx.update(crate::init_app);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let ws = workspace.unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.simulate_resize(size(px(1400.0), px(1100.0)));
    ws.update(&mut vcx, |ws, cx| {
        let session = OpenSpec::Bytes {
            name: "analog.vtr".into(),
            bytes: include_bytes!("../../examples/analog_showcase.vtr").to_vec(),
        }
        .open()
        .unwrap();
        ws.set_session(session, cx);
        ws.dispatch(Command::AddVars(a_all([0])), None, cx);
    });
    vcx.run_until_parked();
    let click = |vcx: &mut VisualTestContext, selector| {
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        vcx.simulate_click(bounds.center(), Modifiers::default());
        vcx.run_until_parked();
    };
    click(&mut vcx, "derive-signal");

    click(&mut vcx, "add-formula-input");
    click(&mut vcx, "recorded-input-search");
    vcx.simulate_input("cosine");
    vcx.run_until_parked();
    click(&mut vcx, "recorded-candidate-A-1");
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(ws.app.derived_editor.as_ref().unwrap().bindings.len(), 2);
    });
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let field = vcx.debug_bounds("formula-expression").unwrap();
    vcx.simulate_click(
        gpui_kit::point(field.center().x, field.bottom() - px(10.0)),
        Modifiers::default(),
    );
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("input - cosine");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.app.derived_editor.as_ref().unwrap().draft.formula.trim(),
            "input - cosine"
        );
        let Some(ComputationStatus::Ready(history)) = ws.app.derived_preview() else {
            panic!("formula did not preview");
        };
        assert_eq!(
            history.value(history.index_at(0)),
            volna_trace::data::WaveValue::Real(-1.0)
        );
        assert_eq!(ws.app.panels.focused_waves().unwrap().items().len(), 1);
    });
    click(&mut vcx, "derived-apply");
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.derived_editor.is_none());
        let waves = ws.app.panels.focused_waves().unwrap();
        assert_eq!(waves.items().len(), 2);
        let h = waves.signal(1).unwrap().history.as_ref().unwrap();
        assert_eq!(
            h.value(h.index_at(0)),
            volna_trace::data::WaveValue::Real(-1.0)
        );
    });
}

fn formula_workspace(
    cx: &mut TestAppContext,
    vars: Vec<usize>,
) -> (Entity<Workspace>, VisualTestContext) {
    let session = OpenSpec::Bytes {
        name: "analog.vtr".into(),
        bytes: include_bytes!("../../examples/analog_showcase.vtr").to_vec(),
    }
    .open()
    .unwrap();
    formula_workspace_session(cx, session, vars)
}
fn formula_workspace_session(
    cx: &mut TestAppContext,
    session: std::sync::Arc<dyn volna_trace::session::Session>,
    vars: Vec<usize>,
) -> (Entity<Workspace>, VisualTestContext) {
    cx.update(crate::init_app);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let ws = workspace.unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.simulate_resize(size(px(1400.0), px(2000.0)));
    ws.update(&mut vcx, |ws, cx| {
        ws.set_session(session, cx);
        ws.dispatch(Command::AddVars(a_all(vars)), None, cx);
    });
    vcx.run_until_parked();
    click_control(&mut vcx, "derive-signal");
    (ws, vcx)
}
fn click_control(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    vcx.simulate_click(bounds.center(), Modifiers::default());
    vcx.run_until_parked();
}

// A user opens Formula in either theme and scrolls its reference without moving the editor controls.
#[gpui_kit::test]
fn formula_reference_is_a_scrollable_bottom_panel_in_both_themes(cx: &mut TestAppContext) {
    let (_ws, mut vcx) = formula_workspace(cx, vec![0]);
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("formula-reference").is_none());
    click_control(&mut vcx, "formula-guide-toggle");

    for dark in [false, true] {
        vcx.update(|_, cx| crate::theme::set(crate::theme::CoreTheme::volna(dark), cx));
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let editor = vcx.debug_bounds("derived-signal-editor").unwrap();
        let reference = vcx.debug_bounds("formula-reference").unwrap();
        let first = vcx.debug_bounds("formula-reference-section-0").unwrap();
        let expression = vcx.debug_bounds("formula-expression").unwrap();
        assert!(reference.left() >= editor.left() && reference.right() <= editor.right());
        assert!(reference.size.height > px(130.0) && reference.size.height < px(400.0));
        let at = reference.center();
        vcx.simulate_mouse_move(at, None, Modifiers::default());
        vcx.simulate_event(gpui_kit::ScrollEvent {
            position: at,
            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.0), px(-800.0))),
            ..Default::default()
        });
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            vcx.debug_bounds("formula-reference-section-0")
                .unwrap()
                .top()
                < first.top()
        );
        assert_eq!(vcx.debug_bounds("formula-expression").unwrap(), expression);
    }
    vcx.simulate_resize(size(px(360.0), px(900.0)));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let editor = vcx.debug_bounds("derived-signal-editor").unwrap();
    let reference = vcx.debug_bounds("formula-reference-scroll").unwrap();
    assert!(reference.left() >= editor.left() && reference.right() <= editor.right());
    click_control(&mut vcx, "formula-guide-toggle");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("formula-reference-scroll").is_none());
    click_control(&mut vcx, "formula-guide-toggle");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("formula-reference-scroll").is_some());
}

// A user copies and pastes a guide example, edits it, and finds a generator to preview through guide search.
#[gpui_kit::test]
fn guide_examples_copy_paste_and_replace_the_formula(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);

    click_control(&mut vcx, "formula-guide-toggle");
    click_control(&mut vcx, "copy-example-0-0");
    vcx.update(|_, cx| {
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "a - b");
    });
    let field = vcx.debug_bounds("formula-expression").unwrap();
    vcx.simulate_click(field.center(), Modifiers::default());
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a cmd-v"
    } else {
        "ctrl-a ctrl-v"
    });
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.app.derived_editor.as_ref().unwrap().draft.formula.trim(),
            "a - b"
        );
    });
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("2");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    click_control(&mut vcx, "use-example-0-0");
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.app.derived_editor.as_ref().unwrap().draft.formula.trim(),
            "input - b"
        );
    });
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("3");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert!(matches!(
            ws.app.derived_preview(),
            Some(ComputationStatus::Ready(_))
        ));
    });
    click_control(&mut vcx, "formula-guide-search");
    vcx.simulate_input("sine");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("formula-reference-section-0").is_none());
    click_control(&mut vcx, "use-example-4-0");
    ws.read_with(&vcx, |ws, _| {
        assert!(matches!(
            ws.app.derived_preview(),
            Some(ComputationStatus::Ready(_))
        ));
        assert!(
            ws.app
                .derived_editor
                .as_ref()
                .unwrap()
                .draft
                .formula
                .starts_with("sine(")
        );
    });
}

// A user sends interval statistics to a table and sees one numeric row for each interval, including the recording edges.
#[gpui_kit::test]
fn formula_editor_adds_interval_results_to_the_existing_table(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);
    ws.update(&mut vcx, |ws, cx| {
        let (start, end) = ws.app.doc.limits();
        ws.app.doc.add_marker(start + (end - start) / 3).unwrap();
        ws.app
            .doc
            .add_marker(start + (end - start) * 2 / 3)
            .unwrap();
        cx.notify();
    });

    let field = vcx.debug_bounds("formula-expression").unwrap();
    vcx.simulate_click(field.center(), Modifiers::default());
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("interval_max(3)");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    click_control(&mut vcx, "scope-marker-1");
    click_control(&mut vcx, "scope-marker-2");
    click_control(&mut vcx, "derived-table");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    ws.read_with(&vcx, |ws, _| {
        let table = ws
            .app
            .panels
            .iter()
            .find_map(|panel| panel.kind.table())
            .unwrap();
        assert_eq!(table.len(), 3);
        assert_eq!(table.window.rows.len(), 3);
        assert!(
            table
                .window
                .rows
                .iter()
                .all(|row| row.cells.iter().any(|cell| cell.text == "3.0"))
        );
        assert!(ws.app.derived_editor.is_none());
    });
}

// A user expands the bottom editor, scrolls the form with actions in place, and copies an actionable formula error.
#[gpui_kit::test]
fn formula_editor_resizes_and_copies_diagnostics(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);

    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let original = vcx.debug_bounds("derived-signal-editor").unwrap();
    click_control(&mut vcx, "derived-expand");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let expanded = vcx.debug_bounds("derived-signal-editor").unwrap();
    assert!(expanded.size.height > original.size.height + px(100.0));
    assert_eq!(expanded.size.width, original.size.width);
    assert_eq!(expanded.bottom(), original.bottom());
    let field = vcx.debug_bounds("formula-expression").unwrap();
    vcx.simulate_click(
        gpui_kit::point(field.center().x, field.bottom() - px(10.0)),
        Modifiers::default(),
    );
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    let alias = ws.read_with(&vcx, |ws, _| {
        ws.app.derived_editor.as_ref().unwrap().bindings[0]
            .alias
            .clone()
    });
    vcx.simulate_input(&format!("interval_sum({alias})"));
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("derived-error").is_some());
    click_control(&mut vcx, "derived-copy-error");
    vcx.update(|_, cx| {
        let item = cx.read_from_clipboard().unwrap();
        let copied = item.text().unwrap();
        assert!(!copied.is_empty());
        assert!(copied.contains("interval_sum needs sample times"));
        assert!(copied.contains("interval_sum((a - b) @ 1 us)"));
    });
    vcx.simulate_resize(size(px(1400.0), px(720.0)));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    click_control(&mut vcx, "input-details-0");
    vcx.simulate_resize(size(px(1000.0), px(650.0)));
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let status = vcx.debug_bounds("derived-status").unwrap();
    let first_field = vcx.debug_bounds("derived-name").unwrap();
    let body = vcx.debug_bounds("derived-form-scroll").unwrap();
    let at = gpui_kit::point(body.left() + px(20.0), body.top() + px(90.0));
    vcx.simulate_mouse_move(at, None, Modifiers::default());
    vcx.simulate_event(gpui_kit::ScrollEvent {
        position: at,
        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.0), px(-500.0))),
        ..Default::default()
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("derived-name").unwrap().top() < first_field.top());
    assert_eq!(vcx.debug_bounds("derived-status").unwrap(), status);
    ws.read_with(&vcx, |ws, _| assert!(ws.app.derived_editor.is_some()));
}

// A user selects two markers in reverse order and saves their interval. Removing a marker leaves the saved bounds intact.
#[gpui_kit::test]
fn selected_marker_times_are_frozen_on_apply(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);
    let (marker, left, right) = ws.update(&mut vcx, |ws, cx| {
        let (start, end) = ws.app.doc.limits();
        let left = start + (end - start) / 4;
        let right = start + (end - start) / 2;
        let marker = ws.app.doc.add_marker(left).unwrap();
        ws.app.doc.add_marker(right).unwrap();
        cx.notify();
        (marker, left, right)
    });
    enter_formula(&mut vcx, "interval_mean(input)");
    for choice in [
        "scope-marker-2",
        "scope-marker-1",
        "scope-start",
        "scope-end",
    ] {
        click_control(&mut vcx, choice);
    }
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::ReadyMeasurement(reading)) = ws.app.derived_preview() else {
            panic!("expected one interval")
        };
        assert_eq!(reading.bounds, (left, right));
    });
    click_control(&mut vcx, "derived-apply");
    ws.update(&mut vcx, |ws, _| {
        assert!(ws.app.doc.remove_marker(marker));
        let row = ws.app.panels.focused_waves().unwrap().signal(1).unwrap();
        let volna_core::computed::Operation::Formula { settings, .. } =
            &row.source.computed().unwrap().operation;
        assert_eq!(
            settings.scope,
            volna_core::computed::MeasurementScope::Explicit {
                start: left,
                end: right
            }
        );
    });
}

// A user removes an input whose slice text is invalid; the remaining valid formula must resume its automatic preview.
#[gpui_kit::test]
fn removing_invalid_slice_resumes_preview(cx: &mut TestAppContext) {
    let session = OpenSpec::Bytes {
        name: "analog.vtr".into(),
        bytes: include_bytes!("../../examples/analog_showcase.vtr").to_vec(),
    }
    .open()
    .unwrap();
    let vector = session
        .hierarchy()
        .vars()
        .position(|v| matches!(v.shape, volna_trace::data::SignalShape::Vector { .. }))
        .unwrap();
    let (ws, mut vcx) = formula_workspace(cx, vec![0, vector]);
    click_control(&mut vcx, "input-details-1");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let field = vcx.debug_bounds("recorded-input-1-slice").unwrap();
    vcx.simulate_click(
        gpui_kit::point(field.center().x, field.bottom() - px(10.0)),
        Modifiers::default(),
    );
    vcx.simulate_input("bad");
    vcx.run_until_parked();
    click_control(&mut vcx, "remove-input-1");
    ws.read_with(&vcx, |ws, _| {
        let editor = ws.app.derived_editor.as_ref().unwrap();
        assert_eq!(editor.bindings.len(), 1);
        assert!(
            matches!(ws.app.derived_preview(), Some(ComputationStatus::Ready(_))),
            "remaining valid formula did not resume preview: error={:?}, stopped={}",
            editor.error,
            editor.stopped()
        );
    });
}

// A user opens Replace and removes an earlier binding; the stale picker must close, and a newly opened picker must edit the intended input.
#[gpui_kit::test]
fn binding_removal_closes_the_stale_replacement_picker(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0, 1, 2]);
    click_control(&mut vcx, "input-details-1");
    click_control(&mut vcx, "replace-input-1");
    click_control(&mut vcx, "input-details-0");
    click_control(&mut vcx, "remove-input-0");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("recorded-input-search").is_none());
    click_control(&mut vcx, "input-details-0");
    click_control(&mut vcx, "replace-input-0");
    click_control(&mut vcx, "recorded-candidate-A-3");
    ws.read_with(&vcx, |ws, _| {
        let editor = ws.app.derived_editor.as_ref().unwrap();
        let cosine = editor
            .bindings
            .iter()
            .find(|input| input.alias == "cosine")
            .unwrap();
        assert!(cosine.label(&ws.app.doc).contains("sawtooth"));
        assert!(
            editor
                .bindings
                .iter()
                .find(|input| input.alias == "triangle")
                .unwrap()
                .label(&ws.app.doc)
                .contains("triangle")
        );
    });
}

// A user changes a search in a large flat recording; only the latest query's matching input must be offered and bound.
#[gpui_kit::test]
fn recorded_picker_handles_large_scopes_and_superseded_queries(cx: &mut TestAppContext) {
    use vtr::Recorder;
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let mut signals = Vec::new();
    for n in 0..5000 {
        signals.push(
            writer
                .signal(
                    vtr::SourceOptions::new(format!("probe_{n}"), None),
                    vtr::SignalKind::Real,
                    vtr::VarType::Real,
                    vtr::Direction::Output,
                )
                .unwrap(),
        );
    }
    writer.emit_real(signals[0], 0.0).unwrap();
    writer.emit_real(signals[4999], 100.0).unwrap();
    writer.advance_time(10).unwrap();
    writer.emit_real(signals[0], 1.0).unwrap();
    writer.close().unwrap();
    let session = OpenSpec::Path(file.path().into()).open().unwrap();
    let (ws, mut vcx) = formula_workspace_session(cx, session, vec![0]);

    click_control(&mut vcx, "add-formula-input");
    click_control(&mut vcx, "recorded-input-search");
    vcx.simulate_input("not_a_signal");
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("PrObE_4999");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("recorded-candidate-A-0").is_none());
    click_control(&mut vcx, "recorded-candidate-A-4999");
    ws.read_with(&vcx, |ws, _| {
        let bindings = &ws.app.derived_editor.as_ref().unwrap().bindings;
        assert_eq!(bindings.len(), 2);
        assert!(bindings[1].label(&ws.app.doc).contains("probe_4999"));
    });
}

// A user starts a moving average from a bound input, edits it, and applies it;
// the preview must honor full-window warm-up and the saved formula must reopen.
#[gpui_kit::test]
fn moving_average_preset_previews_and_reopens_as_a_formula(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("formula-default-step").is_none());
    assert!(vcx.debug_bounds("scope-start").is_none());
    click_control(&mut vcx, "preset-moving_avg");
    let field = vcx.debug_bounds("formula-expression").unwrap();
    vcx.simulate_click(field.center(), Modifiers::default());
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("moving_avg(input, 32 ns, step: 2 ns)");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::Ready(history)) = ws.app.derived_preview() else { panic!("average did not preview"); };
        assert_eq!(history.value(history.index_at(0)), volna_trace::data::WaveValue::Unavailable);
        let base = ws.app.doc.time_base();
        let time = volna_core::moving_average::duration("32 ns", base).unwrap();
        assert!(matches!(history.value(history.index_at(time)), volna_trace::data::WaveValue::Real(value) if value.is_finite()));
    });
    click_control(&mut vcx, "derived-apply");
    vcx.simulate_keystrokes("shift-f10");
    vcx.run_until_parked();
    click_control(&mut vcx, "wave-menu-edit-formula");
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.app.derived_editor.as_ref().unwrap().draft.formula,
            "moving_avg(input, 32 ns, step: 2 ns)"
        );
    });
}

// A user inserts a bound alias at the cursor and switches to an interval mean;
// the waveform must use that input, and measurement controls appear only when needed.
#[gpui_kit::test]
fn aliases_insert_at_the_cursor_and_settings_follow_the_formula(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);
    let set_formula = |vcx: &mut VisualTestContext, expression: &str| {
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let field = vcx.debug_bounds("formula-expression").unwrap();
        vcx.simulate_click(field.center(), Modifiers::default());
        vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        vcx.simulate_input(expression);
    };
    set_formula(&mut vcx, "1 + ");
    click_control(&mut vcx, "insert-input-0");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::Ready(history)) = ws.app.derived_preview() else {
            panic!("inserted alias did not preview");
        };
        assert_eq!(
            history.value(history.index_at(0)),
            volna_trace::data::WaveValue::Real(1.0)
        );
    });
    set_formula(&mut vcx, "moving_avg(input, 32 ns)");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("formula-default-step").is_some());
    click_control(&mut vcx, "preset-interval_mean");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("formula-default-step").is_some());
    set_formula(&mut vcx, "input");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    click_control(&mut vcx, "preset-interval_mean");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("formula-default-step").is_none());
    assert!(vcx.debug_bounds("scope-start").is_some());
}

// A user creates a generator, removes its unused recorded binding, and edits the
// applied waveform again; generators must remain editable without any inputs.
#[gpui_kit::test]
fn generator_without_bindings_reopens_for_editing(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);
    let field = vcx.debug_bounds("formula-expression").unwrap();
    vcx.simulate_click(field.center(), Modifiers::default());
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("sine(period: 8 ns, step: 1 ns)");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    click_control(&mut vcx, "input-details-0");
    click_control(&mut vcx, "remove-input-0");
    click_control(&mut vcx, "derived-apply");
    vcx.simulate_keystrokes("shift-f10");
    vcx.run_until_parked();
    click_control(&mut vcx, "wave-menu-edit-formula");
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.derived_editor.as_ref().unwrap().bindings.is_empty());
        let Some(ComputationStatus::Ready(history)) = ws.app.derived_preview() else {
            panic!("generator did not reopen");
        };
        assert_eq!(
            history.value(history.index_at(0)),
            volna_trace::data::WaveValue::Real(0.0)
        );
    });
}

// A user prefixes an invalid formula with more arithmetic. The error stays the
// same, but its visible underline must move to the alias's new location.
#[gpui_kit::test]
fn unknown_alias_underlines_follow_edits_with_the_same_error(cx: &mut TestAppContext) {
    use gpui_kit::component::input::RopeExt;
    let (_ws, editor, mut vcx) = standalone_formula_editor(cx);
    let mut previous_message = None;
    for expression in ["missing", "1 + missing", "min(input, missing)"] {
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let field = vcx.debug_bounds("formula-expression").unwrap();
        vcx.simulate_click(field.center(), Modifiers::default());
        vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        vcx.simulate_input(expression);
        vcx.simulate_keystrokes("ctrl-enter");
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        editor.read_with(&vcx, |editor, cx| {
            let state = editor.expression.read(cx);
            let diagnostic = state.diagnostics().unwrap().iter().next().unwrap();
            let start = state
                .text()
                .position_to_offset(&diagnostic.diagnostic.range.start);
            let end = state
                .text()
                .position_to_offset(&diagnostic.diagnostic.range.end);
            assert_eq!(&expression[start..end], "missing");
            if let Some(message) = &previous_message {
                assert_eq!(&diagnostic.message, message);
            }
            previous_message = Some(diagnostic.message.clone());
            assert_eq!(
                diagnostic.severity,
                gpui_kit::base::input::DiagnosticSeverity::Error
            );
        });
        assert!(vcx.debug_bounds("derived-error").is_some());
    }
}

fn standalone_formula_editor(
    cx: &mut TestAppContext,
) -> (Entity<Workspace>, Entity<DerivedEditor>, VisualTestContext) {
    let (ws, _) = formula_workspace(cx, vec![0]);
    let panel = ws.read_with(cx, |ws, _| ws.app.panels.focused_id());
    let mut editor = None;
    let root = cx.add_window(|window, cx| {
        let view = cx.new(|cx| DerivedEditor::new(ws.downgrade(), panel, window, cx));
        editor = Some(view.clone());
        gpui_kit::component::Root::new(view, window, cx)
    });
    let editor = editor.unwrap();
    let vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.simulate_resize(size(px(800.0), px(900.0)));
    vcx.run_until_parked();
    (ws, editor, vcx)
}

// A user completes a function, sees its argument hint, then renames an input
// and completes the new alias. Completion must use the language and live bindings.
#[gpui_kit::test]
fn completion_and_argument_help_follow_functions_and_renamed_inputs(cx: &mut TestAppContext) {
    let (ws, editor, mut vcx) = standalone_formula_editor(cx);
    let set_expression = |vcx: &mut VisualTestContext, text: &str| {
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let field = vcx.debug_bounds("formula-expression").unwrap();
        vcx.simulate_click(field.center(), Modifiers::default());
        vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        vcx.simulate_input(text);
        vcx.run_until_parked();
    };
    set_expression(&mut vcx, "moving_");
    vcx.simulate_keystrokes("ctrl-space");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    editor.read_with(&vcx, |editor, cx| {
        assert_eq!(editor.expression.read(cx).value().trim(), "moving_avg");
    });
    vcx.simulate_input("(");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("formula-signature").is_some());
    click_control(&mut vcx, "input-details-0");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let alias = vcx.debug_bounds("recorded-input-0-alias").unwrap();
    vcx.simulate_click(
        gpui_kit::point(alias.center().x, alias.bottom() - px(10.0)),
        Modifiers::default(),
    );
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("sample");
    vcx.run_until_parked();
    set_expression(&mut vcx, "sam");
    vcx.simulate_keystrokes("ctrl-space");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    editor.read_with(&vcx, |editor, cx| {
        assert_eq!(editor.expression.read(cx).value().trim(), "sample");
    });
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::Ready(history)) = ws.app.derived_preview() else {
            panic!("completed alias did not preview");
        };
        assert_eq!(
            history.value(history.index_at(0)),
            volna_trace::data::WaveValue::Real(0.0)
        );
    });
}

// A user finds a clock in the existing picker, makes it the default, then switches
// to a counter. Measurement presets must retain the selected explicit sampling.
#[gpui_kit::test]
fn sampling_picker_and_presets_keep_the_selected_capture(cx: &mut TestAppContext) {
    let session = OpenSpec::Bytes {
        name: "feature.vtr".into(),
        bytes: include_bytes!("../../examples/feature_showcase.vtr").to_vec(),
    }
    .open()
    .unwrap();
    let var = |name: &str| {
        session
            .hierarchy()
            .vars()
            .position(|v| v.name == name)
            .unwrap()
    };
    let temperature = var("temperature_c");
    let (ws, mut vcx) = formula_workspace_session(cx, session, vec![temperature]);
    let click = |vcx: &mut VisualTestContext, selector: &'static str| {
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        vcx.simulate_click(bounds.center(), Modifiers::default());
        vcx.run_until_parked();
    };
    click(&mut vcx, "formula-sampling");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("sampling-input-changes").is_some());
    click(&mut vcx, "choose-sampling-signal");
    vcx.simulate_input("soc.clk");
    vcx.run_until_parked();
    click(&mut vcx, "sampling-use-default");
    click(&mut vcx, "sampling-candidate-A: soc.clk");
    click(&mut vcx, "sampling-mode-RisingEdge");
    click(&mut vcx, "preset-interval_mean");
    ws.read_with(&vcx, |ws, _| {
        let expression = &ws.app.derived_editor.as_ref().unwrap().draft.formula;
        assert!(expression.starts_with("interval_mean("), "{expression}");
        assert!(expression.contains("@posedge clk"), "{expression}");
    });
    for dark in [false, true] {
        vcx.update(|_, cx| crate::theme::set(crate::theme::CoreTheme::volna(dark), cx));
        click(&mut vcx, "formula-sampling");
        click(&mut vcx, "choose-sampling-signal");
        click(&mut vcx, "recorded-input-search");
        vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        vcx.simulate_input("soc.data");
        vcx.run_until_parked();
        click(&mut vcx, "sampling-candidate-A: soc.data");
        click(&mut vcx, "sampling-mode-ValueChanges");
        ws.read_with(&vcx, |ws, _| {
            let expression = &ws.app.derived_editor.as_ref().unwrap().draft.formula;
            assert!(expression.contains("@data"), "{expression}");
            assert!(!expression.contains("@posedge"), "{expression}");
        });
    }
    click(&mut vcx, "formula-sampling");
    click(&mut vcx, "sampling-input-changes");
    ws.read_with(&vcx, |ws, _| {
        let expression = &ws.app.derived_editor.as_ref().unwrap().draft.formula;
        assert!(expression.starts_with("interval_mean("), "{expression}");
        assert!(!expression.contains('@'), "{expression}");
    });
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-down"
    } else {
        "ctrl-end"
    });
    vcx.simulate_input(" + 1");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        let expression = &ws.app.derived_editor.as_ref().unwrap().draft.formula;
        assert!(expression.ends_with(" + 1"), "{expression}");
        assert!(!expression.contains('@'), "{expression}");
    });
}

fn enter_formula(vcx: &mut VisualTestContext, expression: &str) {
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let field = vcx.debug_bounds("formula-expression").unwrap();
    vcx.simulate_click(field.center(), Modifiers::default());
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input(expression);
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
}

// A user measures a ratio, reopens it, then filters a Boolean expression and measures
// a literal. Actions must preserve the entire value expression and work without data bindings.
#[gpui_kit::test]
fn expression_actions_preserve_arithmetic_and_support_boolean_and_literal_values(
    cx: &mut TestAppContext,
) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0, 1]);
    enter_formula(&mut vcx, "(input * 0 + 12) / (cosine * 0 + 3)");
    click_control(&mut vcx, "preset-interval_mean");
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::ReadyMeasurement(value)) = ws.app.derived_preview() else {
            panic!("ratio mean did not preview")
        };
        assert_eq!(value.value, Some(4.0));
    });
    click_control(&mut vcx, "preset-lowpass");
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::Ready(history)) = ws.app.derived_preview() else {
            panic!("ratio filter did not preview")
        };
        assert_eq!(
            history.value(history.index_at(0)),
            volna_trace::data::WaveValue::Real(4.0)
        );
    });
    click_control(&mut vcx, "preset-interval_mean");
    click_control(&mut vcx, "derived-apply");
    vcx.simulate_keystrokes("shift-f10");
    vcx.run_until_parked();
    click_control(&mut vcx, "wave-menu-edit-formula");
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::ReadyMeasurement(value)) = ws.app.derived_preview() else {
            panic!("saved ratio mean did not reopen")
        };
        assert_eq!(value.value, Some(4.0));
    });
    enter_formula(&mut vcx, "input == input");
    click_control(&mut vcx, "preset-lowpass");
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::Ready(history)) = ws.app.derived_preview() else {
            panic!(
                "Boolean filter did not preview: {:?}",
                ws.app
                    .derived_editor
                    .as_ref()
                    .map(|e| (&e.draft.formula, &e.error))
            )
        };
        assert_eq!(
            history.value(history.index_at(0)),
            volna_trace::data::WaveValue::Real(1.0)
        );
    });
    enter_formula(&mut vcx, "8");
    for _ in 0..2 {
        click_control(&mut vcx, "input-details-0");
        click_control(&mut vcx, "remove-input-0");
    }
    click_control(&mut vcx, "preset-interval_mean");
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::ReadyMeasurement(value)) = ws.app.derived_preview() else {
            panic!("literal mean did not preview")
        };
        assert_eq!(value.value, Some(8.0));
    });
}

// A user tries to choose sampling while an expression is unfinished. The picker
// must remain available, Escape must return to the formula, and typing must repair the draft.
#[gpui_kit::test]
fn failed_sampling_selection_preserves_the_draft_and_keyboard_recovery(cx: &mut TestAppContext) {
    let session = OpenSpec::Bytes {
        name: "feature.vtr".into(),
        bytes: include_bytes!("../../examples/feature_showcase.vtr").to_vec(),
    }
    .open()
    .unwrap();
    let temperature = session
        .hierarchy()
        .vars()
        .position(|v| v.name == "temperature_c")
        .unwrap();
    let (ws, mut vcx) = formula_workspace_session(cx, session, vec![temperature]);
    enter_formula(&mut vcx, "temperature_c + ");
    click_control(&mut vcx, "formula-sampling");
    click_control(&mut vcx, "choose-sampling-signal");
    vcx.simulate_input("soc.clk");
    vcx.run_until_parked();
    click_control(&mut vcx, "sampling-candidate-A: soc.clk");
    click_control(&mut vcx, "sampling-mode-RisingEdge");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("recorded-input-search").is_some());
    assert!(vcx.debug_bounds("derived-error").is_some());
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("recorded-input-search").is_none());
    vcx.simulate_input("2");
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.app.derived_editor.as_ref().unwrap().draft.formula,
            "temperature_c + 2"
        );
        assert!(matches!(
            ws.app.derived_preview(),
            Some(ComputationStatus::Ready(_))
        ));
    });
}

// A user adds a statistics sampling counter before adding another measurement input.
// Guide replacements must use value inputs rather than the counter, while retaining sampling.
#[gpui_kit::test]
fn guide_replacements_keep_sampling_separate_from_measurement_operands(cx: &mut TestAppContext) {
    let session = OpenSpec::Bytes {
        name: "feature.vtr".into(),
        bytes: include_bytes!("../../examples/feature_showcase.vtr").to_vec(),
    }
    .open()
    .unwrap();
    let temperature = session
        .hierarchy()
        .vars()
        .position(|v| v.name == "temperature_c")
        .unwrap();
    let (ws, mut vcx) = formula_workspace_session(cx, session, vec![temperature]);
    click_control(&mut vcx, "formula-sampling");
    click_control(&mut vcx, "choose-sampling-signal");
    vcx.simulate_input("soc.data");
    vcx.run_until_parked();
    click_control(&mut vcx, "sampling-candidate-A: soc.data");
    click_control(&mut vcx, "sampling-mode-ValueChanges");
    click_control(&mut vcx, "add-formula-input");
    vcx.simulate_input("sine_fast");
    vcx.run_until_parked();
    click_control(&mut vcx, "recorded-candidate-A-9");
    vcx.run_until_parked();
    click_control(&mut vcx, "formula-guide-toggle");
    click_control(&mut vcx, "use-example-0-0");
    ws.read_with(&vcx, |ws, _| {
        let expression = &ws.app.derived_editor.as_ref().unwrap().draft.formula;
        assert!(
            expression.contains("temperature_c - sine_fast"),
            "{expression}"
        );
        assert!(expression.contains("@data"), "{expression}");
    });
}

// With only one boundary selected, a user can still edit the expression and sampling, then restore the second boundary.
#[gpui_kit::test]
fn sampling_preserves_expression_edits_while_measurement_bounds_are_invalid(
    cx: &mut TestAppContext,
) {
    let session = OpenSpec::Bytes {
        name: "feature.vtr".into(),
        bytes: include_bytes!("../../examples/feature_showcase.vtr").to_vec(),
    }
    .open()
    .unwrap();
    let temperature = session
        .hierarchy()
        .vars()
        .position(|v| v.name == "temperature_c")
        .unwrap();
    let (ws, mut vcx) = formula_workspace_session(cx, session, vec![temperature]);
    click_control(&mut vcx, "preset-interval_mean");
    click_control(&mut vcx, "scope-end");
    enter_formula(&mut vcx, "interval_mean(temperature_c + 7)");
    click_control(&mut vcx, "formula-sampling");
    click_control(&mut vcx, "choose-sampling-signal");
    vcx.simulate_input("soc.data");
    vcx.run_until_parked();
    click_control(&mut vcx, "sampling-candidate-A: soc.data");
    click_control(&mut vcx, "sampling-mode-ValueChanges");
    ws.read_with(&vcx, |ws, _| {
        let expression = &ws.app.derived_editor.as_ref().unwrap().draft.formula;
        assert!(expression.contains("temperature_c + 7"), "{expression}");
        assert!(expression.contains("@data"), "{expression}");
        assert!(ws.app.derived_preview().is_none());
    });
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("derived-error").is_some());
    click_control(&mut vcx, "scope-end");
    ws.read_with(&vcx, |ws, _| {
        assert!(matches!(
            ws.app.derived_preview(),
            Some(ComputationStatus::ReadyMeasurement(_))
        ));
    });
}

// A user narrows the window while deriving a signal. The editor must remain
// usable at 360 px, and closing it must restore the hierarchy without changing layout preferences.
#[gpui_kit::test]
fn a_narrow_formula_editor_temporarily_yields_the_hierarchy_space(cx: &mut TestAppContext) {
    let (_ws, mut vcx) = formula_workspace(cx, vec![0]);
    vcx.simulate_resize(size(px(360.0), px(800.0)));
    for dark in [false, true] {
        vcx.update(|window, cx| {
            crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
            window.draw(cx).clear(cx);
        });
        let editor = vcx.debug_bounds("derived-signal-editor").unwrap();
        let sampling = vcx.debug_bounds("formula-sampling").unwrap();
        assert!(editor.size.width >= px(350.0));
        assert!(sampling.left() >= px(0.0) && sampling.right() <= px(360.0));
        assert!(vcx.debug_bounds("scopes-panel").is_none());
    }
    click_control(&mut vcx, "derived-close");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("scopes-panel").is_some());
}

// Interrupt an unfinished formula to measure and pin a reading, then resume it and apply its original expression.
#[gpui_kit::test]
fn measure_pins_a_reading_and_returns_to_the_unapplied_formula(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);
    enter_formula(&mut vcx, "input * 0 + 9");
    click_control(&mut vcx, "measure-signal");
    assert!(vcx.debug_bounds("formula-expression").is_none());
    click_control(&mut vcx, "preset-interval_max");
    click_control(&mut vcx, "pin-reading");
    click_control(&mut vcx, "derive-signal");
    assert!(vcx.debug_bounds("formula-expression").is_some());
    click_control(&mut vcx, "derived-apply");
    ws.read_with(&vcx, |ws, _| {
        let waves = ws.app.panels.focused_waves().unwrap();
        assert_eq!(waves.pinned_readings.len(), 1);
        assert_eq!(waves.items().len(), 2);
        let history = waves.signal(1).unwrap().history.as_ref().unwrap();
        assert_eq!(
            history.value(history.index_at(0)),
            volna_trace::data::WaveValue::Real(9.0)
        );
    });
}

// A user splits a measurement at a marker, tries dragging the fixed divider and collapses the preview,
// then opens it again and commits the same interval rows.
#[gpui_kit::test]
fn measure_table_previews_the_intervals_before_commit(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);
    ws.update(&mut vcx, |ws, cx| {
        ws.app.doc.add_marker(10).unwrap();
        cx.notify();
    });
    click_control(&mut vcx, "measure-signal");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("measure-table-row-0").is_some());
    assert!(vcx.debug_bounds("measure-table-row-10").is_none());
    click_control(&mut vcx, "scope-marker-1");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("measure-table-row-10").is_some());
    let pane = vcx.debug_bounds("measure-table-preview").unwrap();
    let domain = vcx.debug_bounds("measure-output-domain").unwrap();
    assert!((px(0.)..=px(16.)).contains(&(pane.top() - domain.bottom())));
    let button = vcx.debug_bounds("derived-table").unwrap();
    let start = gpui_kit::point(pane.center().x, pane.top() + px(1.));
    let end = gpui_kit::point(start.x, start.y - px(55.));
    let left = gpui_kit::MouseButton::Left;
    vcx.simulate_mouse_down(start, left, Modifiers::default());
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.simulate_mouse_move(end, Some(left), Modifiers::default());
    vcx.simulate_mouse_up(end, left, Modifiers::default());
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(vcx.debug_bounds("measure-table-preview").unwrap(), pane);
    assert_eq!(vcx.debug_bounds("derived-table").unwrap(), button);
    click_control(&mut vcx, "measure-table-toggle");
    click_control(&mut vcx, "derived-close");
    click_control(&mut vcx, "measure-signal");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("measure-table-row-0").is_none());
    click_control(&mut vcx, "measure-table-toggle");
    click_control(&mut vcx, "scope-marker-1");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        vcx.debug_bounds("measure-table-preview")
            .unwrap()
            .size
            .height,
        pane.size.height
    );
    click_control(&mut vcx, "derived-table");
    ws.read_with(&vcx, |ws, _| {
        let table = ws
            .app
            .panels
            .iter()
            .find_map(|panel| panel.kind.table())
            .unwrap();
        assert_eq!(table.len(), 2);
        assert_eq!((table.row_time(0), table.row_time(1)), (Some(0), Some(10)));
    });
}

#[gpui_kit::test]
fn resumed_measurement_formula_can_remove_a_saved_boundary(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);
    ws.update(&mut vcx, |ws, cx| {
        ws.app.doc.add_marker(10).unwrap();
        ws.app.doc.add_marker(20).unwrap();
        cx.notify();
    });
    enter_formula(&mut vcx, "interval_max(3)");
    click_control(&mut vcx, "scope-marker-1");
    click_control(&mut vcx, "derived-apply");
    vcx.simulate_keystrokes("shift-f10");
    vcx.run_until_parked();
    click_control(&mut vcx, "wave-menu-edit-formula");
    click_control(&mut vcx, "scope-marker-2");
    ws.update(&mut vcx, |ws, cx| {
        let panel = ws.app.panels.focused_id();
        ws.dispatch(
            Command::Derived(panel, DerivedCommand::Measure(0)),
            None,
            cx,
        );
    });
    vcx.run_until_parked();
    click_control(&mut vcx, "derive-signal");
    click_control(&mut vcx, "scope-tick-10");
    click_control(&mut vcx, "derived-table");
    ws.read_with(&vcx, |ws, _| {
        let table = ws.app.panels.iter().find_map(|p| p.kind.table()).unwrap();
        assert_eq!(table.len(), 2);
        assert_eq!((table.row_time(0), table.row_time(1)), (Some(0), Some(20)));
    });
}
