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
    click(&mut vcx, "derive-moving-average");
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.moving_average_editor.is_some());
        assert!(matches!(
            ws.app.moving_average_preview(),
            Some(ComputationStatus::Ready(_))
        ));
        assert_eq!(ws.app.panels.focused_waves().unwrap().items().len(), 1);
        assert_eq!(
            ws.app
                .moving_average_editor
                .as_ref()
                .unwrap()
                .interpretation
                .kind,
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
        assert_eq!(
            ws.app.moving_average_editor.as_ref().unwrap().draft.window,
            "4 us"
        );
        assert!(
            matches!(
                ws.app.moving_average_preview(),
                Some(ComputationStatus::Ready(_))
            ),
            "preview key {:?}, error {:?}, status {}",
            ws.app.moving_average_editor.as_ref().unwrap().preview,
            ws.app.moving_average_editor.as_ref().unwrap().error,
            match ws.app.moving_average_preview() {
                Some(ComputationStatus::Waiting) => "waiting",
                Some(ComputationStatus::Building) => "building",
                Some(ComputationStatus::Failed(error)) => error.as_str(),
                Some(ComputationStatus::Ready(_)) => "ready",
                None => "none",
            }
        );
    });
    click(&mut vcx, "average-apply");
    ws.read_with(&vcx, |ws, _| {
        let waves = ws.app.panels.focused_waves().unwrap();
        assert_eq!(waves.items().len(), 2);
        assert_eq!(
            waves.signal(1).unwrap().shape,
            volna_trace::data::SignalShape::Real
        );
        assert_eq!(waves.signal(1).unwrap().translator.id(), "real");
        assert_eq!(
            waves
                .signal(1)
                .unwrap()
                .source
                .moving_average()
                .unwrap()
                .settings
                .window,
            4000
        );
    });
    vcx.simulate_keystrokes("shift-f10 down enter");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.moving_average_editor.as_ref().unwrap().editing);
    });
    for dark in [false, true] {
        vcx.update(|_, cx| crate::theme::set(crate::theme::CoreTheme::volna(dark), cx));
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let inspector = vcx.debug_bounds("moving-average-editor").unwrap();
        assert!(inspector.size.width > px(280.0));
        assert!(vcx.debug_bounds("average-preview").is_none());
        let status = vcx.debug_bounds("average-status").unwrap();
        assert!(status.left() >= inspector.left() && status.right() <= inspector.right());
    }
    click(&mut vcx, "average-remove");
    ws.read_with(&vcx, |ws, _| {
        assert!(ws.app.moving_average_editor.is_none());
        assert_eq!(ws.app.panels.focused_waves().unwrap().items().len(), 1);
    });
}
