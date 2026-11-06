use super::*;
use gpui_kit::{Modifiers, TestAppContext, VisualTestContext, size};
use volna_core::testing::a_all;
use volna_trace::session::OpenSpec;

#[gpui_kit::test]
fn inspector_creates_edits_and_removes_real_waveforms_in_both_themes(cx: &mut TestAppContext) {
    inspector_workflow(cx, false);
}

#[gpui_kit::test]
fn inspector_uses_a_signed_vector_and_produces_a_real_waveform(cx: &mut TestAppContext) {
    inspector_workflow(cx, true);
}

fn inspector_workflow(cx: &mut TestAppContext, signed: bool) {
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
    let field = vcx.debug_bounds("average-window").unwrap();
    vcx.simulate_click(
        gpui_kit::point(field.center().x, field.bottom() - px(10.0)),
        Modifiers::default(),
    );
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("4 us");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(ws.app.derived_editor.as_ref().unwrap().draft.window, "4 us");
        assert!(
            matches!(ws.app.derived_preview(), Some(ComputationStatus::Ready(_))),
            "preview key {:?}, error {:?}, status {}",
            ws.app.derived_editor.as_ref().unwrap().preview,
            ws.app.derived_editor.as_ref().unwrap().error,
            match ws.app.derived_preview() {
                Some(ComputationStatus::Waiting) => "waiting",
                Some(ComputationStatus::Building) => "building",
                Some(ComputationStatus::Failed(error)) => error.as_str(),
                Some(ComputationStatus::Ready(_)) => "ready",
                Some(ComputationStatus::ReadyMeasurement(_)) => "measurement",
                Some(ComputationStatus::ReadyTable(_)) => "table",
                None => "none",
            }
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
        assert_eq!(
            {
                let volna_core::computed::Operation::MovingAverage { settings } = &waves
                    .signal(1)
                    .unwrap()
                    .source
                    .computed()
                    .unwrap()
                    .operation
                else {
                    panic!()
                };
                settings.window
            },
            4000
        );
    });
    vcx.simulate_keystrokes("shift-f10");
    vcx.run_until_parked();
    click(&mut vcx, "wave-menu-derived");
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.derived_editor.as_ref().unwrap().editing);
    });
    for dark in [false, true] {
        vcx.update(|_, cx| crate::theme::set(crate::theme::CoreTheme::volna(dark), cx));
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let inspector = vcx.debug_bounds("derived-signal-editor").unwrap();
        assert!(inspector.size.width > px(280.0));
        assert!(vcx.debug_bounds("average-preview").is_none());
        let status = vcx.debug_bounds("derived-status").unwrap();
        assert!(status.left() >= inspector.left() && status.right() <= inspector.right());
    }
    click(&mut vcx, "derived-remove");
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.derived_editor.is_none());
        assert_eq!(ws.app.panels.focused_waves().unwrap().items().len(), 1);
    });
}

// A user chooses Formula, searches for a hidden recorded input, and applies the live difference waveform.
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
    click(&mut vcx, "operation-formula");
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
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.app
                .derived_editor
                .as_ref()
                .unwrap()
                .draft
                .formula
                .as_deref(),
            Some("input - cosine")
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
    click_control(&mut vcx, "operation-formula");
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
    click_control(&mut vcx, "operation-formula");
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
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.app
                .derived_editor
                .as_ref()
                .unwrap()
                .draft
                .formula
                .as_deref(),
            Some("a - b")
        );
    });
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("2");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    click_control(&mut vcx, "use-example-0-0");
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.app
                .derived_editor
                .as_ref()
                .unwrap()
                .draft
                .formula
                .as_deref(),
            Some("a - b")
        );
    });
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("3");
    vcx.simulate_keystrokes("enter");
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
                .as_ref()
                .unwrap()
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
    click_control(&mut vcx, "operation-formula");
    let field = vcx.debug_bounds("formula-expression").unwrap();
    vcx.simulate_click(field.center(), Modifiers::default());
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_input("interval_max(3)");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
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

// A user widens the editor, scrolls the form with actions in place, and copies an actionable formula error.
#[gpui_kit::test]
fn formula_editor_resizes_and_copies_diagnostics(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);
    click_control(&mut vcx, "operation-formula");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let original = vcx.debug_bounds("derived-signal-editor").unwrap();
    click_control(&mut vcx, "derived-width");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let widened = vcx.debug_bounds("derived-signal-editor").unwrap();
    assert!(widened.size.width > original.size.width + px(100.0));
    let divider = vcx.debug_bounds("derived-resize").unwrap();
    let from = divider.center();
    let to = gpui_kit::point(from.x + px(80.0), from.y);
    vcx.simulate_mouse_down(from, gpui_kit::MouseButton::Left, Modifiers::default());
    vcx.simulate_mouse_move(to, Some(gpui_kit::MouseButton::Left), Modifiers::default());
    vcx.simulate_mouse_up(to, gpui_kit::MouseButton::Left, Modifiers::default());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let dragged = vcx.debug_bounds("derived-signal-editor").unwrap();
    assert!(dragged.size.width < widened.size.width - px(40.0));
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
    vcx.simulate_keystrokes("enter");
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
    let footer = vcx.debug_bounds("derived-status").unwrap();
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
    assert_eq!(vcx.debug_bounds("derived-status").unwrap(), footer);
    ws.read_with(&vcx, |ws, _| assert!(ws.app.derived_editor.is_some()));
}

// A user chooses marker intervals, repairs a set after one marker is removed, and enters exact bounds before applying the definition.
#[gpui_kit::test]
fn measurement_scope_controls_save_selected_markers_and_explicit_bounds(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0]);
    let (first, second, third, bounds) = ws.update(&mut vcx, |ws, cx| {
        let (start, end) = ws.app.doc.limits();
        let span = end - start;
        let first = ws.app.doc.add_marker(start + span / 4).unwrap();
        let second = ws.app.doc.add_marker(start + span / 2).unwrap();
        let third = ws.app.doc.add_marker(start + span * 3 / 4).unwrap();
        cx.notify();
        (first, second, third, (start, end))
    });
    click_control(&mut vcx, "operation-formula");
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
    vcx.simulate_input("interval_mean(input)");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();

    click_control(&mut vcx, "formula-scope");
    click_control(&mut vcx, "scope-marker-3");
    let time = |fraction| bounds.0 + (bounds.1 - bounds.0) * fraction / 4;
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::ReadyMeasurement(measurement)) = ws.app.derived_preview()
        else {
            panic!("linked marker interval did not preview");
        };
        assert_eq!(measurement.bounds, (time(2), time(3)));
    });
    click_control(&mut vcx, "formula-scope");
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::ReadyMeasurement(measurement)) = ws.app.derived_preview()
        else {
            panic!("marker snapshot did not preview");
        };
        assert_eq!(measurement.bounds, (time(2), time(3)));
    });
    click_control(&mut vcx, "formula-scope");
    click_control(&mut vcx, "scope-marker-1");
    click_control(&mut vcx, "scope-marker-2");
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::ReadyTable(rows)) = ws.app.derived_preview() else {
            panic!("selected marker set did not preview");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].start, rows[0].end, rows[0].bounds),
            (Some(first), Some(third), (time(1), time(3)))
        );
    });
    ws.update(&mut vcx, |ws, cx| {
        assert!(ws.app.doc.remove_marker(third));
        cx.notify();
    });
    click_control(&mut vcx, "scope-marker-2");
    ws.read_with(&vcx, |ws, _| {
        let Some(ComputationStatus::ReadyTable(rows)) = ws.app.derived_preview() else {
            panic!("repaired marker set did not preview");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].start, rows[0].end, rows[0].bounds),
            (Some(first), Some(second), (time(1), time(2)))
        );
    });
    click_control(&mut vcx, "formula-scope");
    let set_bound = |vcx: &mut VisualTestContext, selector, value: String| {
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let field = vcx.debug_bounds(selector).unwrap();
        vcx.simulate_click(
            gpui_kit::point(field.center().x, field.bottom() - px(10.0)),
            Modifiers::default(),
        );
        vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        vcx.simulate_input(&value);
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
    };
    let left = bounds.0 + (bounds.1 - bounds.0) / 4;
    let right = bounds.0 + (bounds.1 - bounds.0) / 2;
    set_bound(&mut vcx, "formula-scope-start", "invalid".into());
    click_control(&mut vcx, "derived-apply");
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.derived_editor.is_some());
        assert_eq!(ws.app.panels.focused_waves().unwrap().items().len(), 1);
    });
    set_bound(&mut vcx, "formula-scope-start", left.to_string());
    set_bound(&mut vcx, "formula-scope-end", right.to_string());
    click_control(&mut vcx, "derived-apply");
    ws.read_with(&vcx, |ws, _| {
        let source = &ws
            .app
            .panels
            .focused_waves()
            .unwrap()
            .signal(1)
            .unwrap()
            .source;
        let volna_core::computed::Operation::Formula { settings, .. } =
            &source.computed().unwrap().operation
        else {
            panic!("expected applied formula");
        };
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
            editor.stopped
        );
    });
}

// A user opens Replace and removes an earlier binding; the stale picker must close, and a newly opened picker must edit the intended input.
#[gpui_kit::test]
fn binding_removal_closes_the_stale_replacement_picker(cx: &mut TestAppContext) {
    let (ws, mut vcx) = formula_workspace(cx, vec![0, 1, 2]);
    click_control(&mut vcx, "replace-input-1");
    click_control(&mut vcx, "remove-input-0");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("recorded-input-search").is_none());
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
    click_control(&mut vcx, "operation-formula");
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
