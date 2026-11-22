//! Native list virtualization, selection retention and reveal across viewer themes.

use super::{Workspace, init};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, VisualTestContext, px, size};
use volna_core::{
    app::{Action, Command},
    document::RecordCursor,
    panels::PanelsCommand,
    testing::{a, a_all},
};
use volna_trace::{data::Member, session::OpenSpec};
use vtr::Recorder;

#[gpui_kit::test]
fn stage_selection_moves_attributes_below_the_list_and_supports_keyboard_selection(
    cx: &mut TestAppContext,
) {
    use volna_core::transaction::{TransactionCommand, TxPanelState};
    use volna_trace::data::transactions::TransactionRef;

    init(cx);
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let track = writer
        .transactions::<()>(vtr::SourceOptions::new("instructions", None), "PIPELINE")
        .unwrap();
    let name = writer.intern("execute").unwrap();
    let completed = writer.intern("completed").unwrap();
    let owner = writer.begin_at(track, 10, &(), None).unwrap();
    let first = writer
        .embedded_stage(
            owner.owner(),
            10,
            name,
            0,
            Some(12),
            vec![vtr::Attribute {
                key: completed,
                value: vtr::Value::Bool(false),
            }],
        )
        .unwrap();
    let second = writer
        .embedded_stage(
            owner.owner(),
            12,
            name,
            0,
            Some(14),
            vec![vtr::Attribute {
                key: completed,
                value: vtr::Value::Bool(true),
            }],
        )
        .unwrap();
    writer.end_at(owner, 15, vtr::Status::Ok).unwrap();
    writer.close().unwrap();
    let session = OpenSpec::Path(file.path().into()).open().unwrap();
    let track = a(session.tracks()[0].id);
    let host = cx.add_window(Workspace::new);
    let mut vcx = VisualTestContext::from_window(host.into(), cx);
    vcx.simulate_resize(size(px(1200.0), px(1600.0)));
    let waves = host
        .update(&mut vcx, |ws, window, cx| {
            ws.set_session(session, cx);
            ws.app.sidebar_visible = false;
            ws.dispatch(
                Command::AddToWaves(a_all(vec![Member::Generator(0)])),
                Some(window),
                cx,
            );
            ws.app.panels.focused_id()
        })
        .unwrap();
    vcx.run_until_parked();
    let details = host
        .update(&mut vcx, |ws, window, cx| {
            ws.dispatch(
                Command::SelectTransaction {
                    panel: waves,
                    track,
                    id: TransactionRef(owner.id()),
                    cursor: None,
                },
                Some(window),
                cx,
            );
            ws.dispatch(Command::ShowTransaction { from: waves }, Some(window), cx);
            ws.app.panels.focused_id()
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("tx-stage-attributes").is_none());
    let first_selector: &'static str = Box::leak(format!("tx-stage-{first}").into_boxed_str());
    let second_selector: &'static str = Box::leak(format!("tx-stage-{second}").into_boxed_str());
    for dark in [true, false] {
        for width in [1200.0, 360.0] {
            vcx.simulate_resize(size(px(width), px(1100.0)));
            host.update(&mut vcx, |ws, window, cx| {
                crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                ws.dispatch(
                    Command::Transaction(
                        details,
                        TransactionCommand::SelectStage(TransactionRef(second)),
                    ),
                    Some(window),
                    cx,
                );
                cx.notify();
            })
            .unwrap();
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let first_bounds = vcx.debug_bounds(first_selector).unwrap();
            vcx.simulate_click(first_bounds.center(), gpui_kit::Modifiers::default());
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let section = vcx.debug_bounds("tx-stage-attributes").unwrap();
            let last_stage = vcx.debug_bounds(second_selector).unwrap();
            assert!(section.top() >= last_stage.bottom());
            assert!(section.right() <= px(width));
            let key = vcx.debug_bounds("tx-stage-attr-key-0").unwrap();
            let value = vcx.debug_bounds("tx-stage-attr-0").unwrap();
            assert!(key.right() <= value.left());
            assert!(value.right() <= section.right());
            host.update(&mut vcx, |ws, _, _| {
                let TxPanelState::Ready(view) = ws
                    .app
                    .panels
                    .transaction(details)
                    .unwrap()
                    .state(&ws.app.doc)
                else {
                    panic!("record details ready");
                };
                assert!(view.stages.rows[0].selected);
                assert!(!view.stages.rows[1].selected);
                assert_eq!(
                    view.stage_details.unwrap().attributes.rows[0].value,
                    "false"
                );
                assert_eq!(ws.app.doc.shared.cursor, Some(10));
            })
            .unwrap();
            vcx.simulate_keystrokes("tab");
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            vcx.simulate_keystrokes(if dark { "enter" } else { "space" });
            vcx.run_until_parked();
            host.update(&mut vcx, |ws, _, _| {
                let TxPanelState::Ready(view) = ws
                    .app
                    .panels
                    .transaction(details)
                    .unwrap()
                    .state(&ws.app.doc)
                else {
                    panic!("record details ready");
                };
                assert!(!view.stages.rows[0].selected);
                assert!(view.stages.rows[1].selected);
                assert_eq!(view.stage_details.unwrap().attributes.rows[0].value, "true");
                assert_eq!(ws.app.doc.shared.cursor, Some(12));
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn cursor_list_reveals_deep_selection_and_keeps_details_after_cursor_moves(
    cx: &mut TestAppContext,
) {
    init(cx);
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let track = writer
        .transactions::<()>(vtr::SourceOptions::new("instructions", None), "PIPELINE")
        .unwrap();
    let key = writer.intern("vtr.label").unwrap();
    let label = writer.intern(&"Load packet · λ ".repeat(24)).unwrap();
    for i in 0..5000 {
        let tx = writer.begin_at(track, 10, &(), None).unwrap();
        writer
            .attribute(tx.owner(), key, vtr::Value::Str(label))
            .unwrap();
        writer
            .end_at(
                tx,
                if i == 0 {
                    450
                } else if i < 3 {
                    300
                } else {
                    100
                },
                vtr::Status::Ok,
            )
            .unwrap();
    }
    writer.advance_time(1000).unwrap();
    writer.close().unwrap();
    let session = OpenSpec::Path(file.path().into()).open().unwrap();
    let track = a(session.tracks()[0].id);
    let host = cx.add_window(Workspace::new);
    let mut vcx = VisualTestContext::from_window(host.into(), cx);
    vcx.simulate_resize(size(px(1200.0), px(800.0)));
    let waves = host
        .update(&mut vcx, |ws, window, cx| {
            ws.set_session(session, cx);
            ws.app.sidebar_visible = false;
            ws.dispatch(
                Command::AddToWaves(a_all(vec![Member::Generator(0)])),
                Some(window),
                cx,
            );
            ws.app.panels.focused_id()
        })
        .unwrap();
    vcx.run_until_parked();
    let (details, selected) = host
        .update(&mut vcx, |ws, window, cx| {
            let generator = ws.app.doc.resident_generator(track).unwrap();
            let selected = generator.transactions().last().unwrap().id;
            ws.app.doc.shared.cursor = Some(20);
            ws.app.doc.set_record_cursor(RecordCursor {
                track,
                origin: waves,
            });
            ws.dispatch(
                Command::SelectTransaction {
                    panel: waves,
                    track,
                    id: selected,
                    cursor: None,
                },
                Some(window),
                cx,
            );
            ws.dispatch(Command::ShowTransaction { from: waves }, Some(window), cx);
            let details = ws.app.panels.focused_id();
            ws.dispatch(
                Command::Panels(PanelsCommand::ToggleBottomDock),
                Some(window),
                cx,
            );
            (details, selected)
        })
        .unwrap();
    vcx.run_until_parked();
    let selected_selector: &'static str =
        Box::leak(format!("tx-active-record-{}", selected.0).into_boxed_str());
    for dark in [true, false] {
        for zoom in [1.0, 1.5] {
            for width in [1200.0, 360.0] {
                vcx.simulate_resize(size(px(width), px(800.0)));
                host.update(&mut vcx, |_, _, cx| {
                    crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                    crate::theme::set_zoom(zoom, cx);
                    cx.notify();
                })
                .unwrap();
                vcx.run_until_parked();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                // Record Details has no cursor split divider; At Cursor lives in the Bottom Dock.
                assert!(vcx.debug_bounds("tx-cursor-divider").is_none());
                let bounds = vcx.debug_bounds("tx-cursor-records").unwrap();
                let selected_bounds = vcx.debug_bounds(selected_selector).unwrap();
                assert!(
                    bounds.contains(&selected_bounds.center()),
                    "selected row scrolls into view"
                );
                assert!(
                    selected_bounds.left() >= bounds.left()
                        && selected_bounds.right() <= bounds.right()
                );
                assert!(bounds.right() <= px(width));
                for (time, expected) in [(200, 3), (350, 1), (500, 0), (20, 5000)] {
                    host.update(&mut vcx, |ws, _, cx| {
                        ws.app.doc.shared.cursor = Some(time);
                        cx.notify();
                    })
                    .unwrap();
                    vcx.run_until_parked();
                    vcx.update(|window, cx| window.draw(cx).clear(cx));
                    host.update(&mut vcx, |ws, _, _| {
                        assert_eq!(
                            ws.app.cursor_records_panel().ready().unwrap().count,
                            expected
                        );
                    })
                    .unwrap();
                }
                host.update(&mut vcx, |ws, _, _| {
                    let query = ws.app.cursor_records_panel().ready().unwrap();
                    assert_eq!(query.count, 5000);
                    assert!(query.selected_rank.is_some());
                    assert!(query.page(0..1)[0].hidden);
                    assert!(
                        !query.page(query.selected_rank.unwrap()..query.selected_rank.unwrap() + 1)
                            [0]
                        .hidden
                    );
                    assert_eq!(
                        ws.app
                            .panels
                            .transaction(details)
                            .unwrap()
                            .shown()
                            .unwrap()
                            .id,
                        selected
                    );
                })
                .unwrap();
            }
        }
    }
    vcx.simulate_resize(size(px(1200.0), px(800.0)));
    vcx.update(|_, cx| crate::theme::set_zoom(1.0, cx));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let duration = vcx.debug_bounds("tx-time-Duration").unwrap();
    vcx.simulate_click(duration.center(), gpui_kit::Modifiers::default());
    vcx.run_until_parked();
    host.update(&mut vcx, |ws, _, _| {
        let viewport = ws.app.doc.shared.viewport.target();
        assert_eq!(viewport.start, -12.0);
        assert_eq!(viewport.end, 472.0);
    })
    .unwrap();
    vcx.update(|window, cx| window.click("tx-reveal-selected", cx));
    vcx.run_until_parked();
    host.update(&mut vcx, |ws, _, _| {
        let lane = ws.app.panels.waves(waves).unwrap().items()[0]
            .lane()
            .unwrap();
        let sub = lane.generator(&ws.app.doc).unwrap().sub_row(4999);
        assert!(
            lane.first_sub_row <= sub
                && sub < lane.first_sub_row + volna_core::wave::lane::capacity(lane.height)
        );
    })
    .unwrap();
    let previous = host
        .update(&mut vcx, |ws, _, _| {
            let query = ws.app.cursor_records_panel().ready().unwrap();
            query
                .page(query.selected_rank.unwrap() - 1..query.selected_rank.unwrap())
                .remove(0)
                .id
        })
        .unwrap();
    let previous_selector: &'static str =
        Box::leak(format!("tx-active-record-{}", previous.0).into_boxed_str());
    let previous_bounds = vcx.debug_bounds(previous_selector).unwrap();
    vcx.simulate_click(previous_bounds.center(), gpui_kit::Modifiers::default());
    vcx.run_until_parked();
    host.update(&mut vcx, |ws, window, cx| {
        assert_eq!(ws.app.doc.shared.cursor, Some(20));
        assert_eq!(
            ws.app
                .panels
                .transaction(details)
                .unwrap()
                .shown()
                .unwrap()
                .id,
            previous
        );
        ws.dispatch(
            Command::Panels(PanelsCommand::Focus(waves)),
            Some(window),
            cx,
        );
        ws.dispatch(Command::Action(Action::ResetRowHeight), Some(window), cx);
        ws.app.doc.shared.cursor = Some(500);
        cx.notify();
    })
    .unwrap();
    vcx.run_until_parked();
    host.update(&mut vcx, |ws, _, _| {
        let query = ws.app.cursor_records_panel().ready().unwrap();
        assert_eq!(query.count, 0);
        assert_eq!(query.selected_rank, None);
        assert_eq!(query.selected, Some(previous));
        assert_eq!(ws.app.doc.selection().unwrap().id, previous);
    })
    .unwrap();
}

#[gpui_kit::test]
fn attribute_tree_opens_expanded_and_supports_keyboard_copy_in_both_themes(
    cx: &mut TestAppContext,
) {
    use volna_core::transaction::{TransactionCommand, TxPanelState};
    use volna_trace::data::transactions::TransactionRef;
    use vtr::{Attribute, Value};

    init(cx);
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let track = writer
        .transactions::<[u64; 4]>(vtr::SourceOptions::new("instructions", None), "INSTRUCTION")
        .unwrap();
    let instruction = writer.intern("instruction").unwrap();
    let mnemonic = writer.intern("mnemonic").unwrap();
    let pc = writer.intern("pc").unwrap();
    let owner = writer.begin_at(track, 10, &[0, 1, 2, 3], None).unwrap();
    writer
        .attribute(
            owner.owner(),
            instruction,
            Value::Map(vec![
                Attribute {
                    key: mnemonic,
                    value: Value::Text("ds_read_b32".into()),
                },
                Attribute {
                    key: pc,
                    value: Value::U64(u64::MAX),
                },
            ]),
        )
        .unwrap();
    writer.end_at(owner, 15, vtr::Status::Ok).unwrap();
    writer.close().unwrap();
    let session = OpenSpec::Path(file.path().into()).open().unwrap();
    let track = a(session.tracks()[0].id);
    let host = cx.add_window(Workspace::new);
    let mut vcx = VisualTestContext::from_window(host.into(), cx);
    vcx.simulate_resize(size(px(1200.0), px(1600.0)));
    let details = host
        .update(&mut vcx, |ws, window, cx| {
            ws.set_session(session, cx);
            ws.app.sidebar_visible = false;
            ws.dispatch(
                Command::AddToWaves(a_all(vec![Member::Generator(0)])),
                Some(window),
                cx,
            );
            ws.app.panels.focused_id()
        })
        .unwrap();
    vcx.run_until_parked();
    let details = host
        .update(&mut vcx, |ws, window, cx| {
            ws.dispatch(
                Command::SelectTransaction {
                    panel: details,
                    track,
                    id: TransactionRef(owner.id()),
                    cursor: None,
                },
                Some(window),
                cx,
            );
            ws.dispatch(Command::ShowTransaction { from: details }, Some(window), cx);
            ws.app.panels.focused_id()
        })
        .unwrap();
    vcx.run_until_parked();
    for dark in [true, false] {
        for width in [1200.0, 360.0] {
            host.update(&mut vcx, |ws, window, cx| {
                crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                ws.dispatch(
                    Command::Transaction(details, TransactionCommand::ExpandAttributes(true)),
                    Some(window),
                    cx,
                );
            })
            .unwrap();
            vcx.simulate_resize(size(px(width), px(1100.0)));
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let (branch, leaf_index) = host
                .update(&mut vcx, |ws, _, _| {
                    let TxPanelState::Ready(view) = ws
                        .app
                        .panels
                        .transaction(details)
                        .unwrap()
                        .state(&ws.app.doc)
                    else {
                        panic!("details ready")
                    };
                    let branch = view
                        .attributes
                        .rows
                        .iter()
                        .find(|row| row.key == "attributes.instruction")
                        .unwrap();
                    assert!(branch.expanded);
                    assert!(
                        view.attributes
                            .rows
                            .iter()
                            .any(|row| row.key == "[3]" && row.value == "3")
                    );
                    let leaf = view
                        .attributes
                        .rows
                        .iter()
                        .position(|row| row.key == "mnemonic" && row.value == "ds_read_b32")
                        .unwrap();
                    (branch.id.clone(), leaf)
                })
                .unwrap();
            let key_selector: &'static str =
                Box::leak(format!("tx-attr-key-{leaf_index}").into_boxed_str());
            let value_selector: &'static str =
                Box::leak(format!("tx-attr-{leaf_index}").into_boxed_str());
            let key = vcx.debug_bounds(key_selector).unwrap();
            let value = vcx.debug_bounds(value_selector).unwrap();
            assert!(key.right() <= value.left());
            assert!(value.right() <= px(width));
            let selector: &'static str =
                Box::leak(format!("tx-attr-node-{branch}").into_boxed_str());
            let bounds = vcx.debug_bounds(selector).unwrap();
            vcx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
            vcx.run_until_parked();
            host.update(&mut vcx, |ws, _, _| {
                let TxPanelState::Ready(view) = ws
                    .app
                    .panels
                    .transaction(details)
                    .unwrap()
                    .state(&ws.app.doc)
                else {
                    panic!("details ready")
                };
                assert!(!view.attributes.rows.iter().any(|row| row.key == "mnemonic"));
            })
            .unwrap();
            vcx.simulate_keystrokes("right");
            vcx.run_until_parked();
            host.update(&mut vcx, |ws, _, _| {
                let TxPanelState::Ready(view) = ws
                    .app
                    .panels
                    .transaction(details)
                    .unwrap()
                    .state(&ws.app.doc)
                else {
                    panic!("details ready")
                };
                assert!(view.attributes.rows.iter().any(|row| row.key == "mnemonic"));
            })
            .unwrap();
            vcx.simulate_keystrokes("right");
            vcx.simulate_keystrokes("ctrl-c");
            assert_eq!(
                vcx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
                "ds_read_b32"
            );
            vcx.simulate_keystrokes("up");
            vcx.simulate_keystrokes("ctrl-c");
            assert!(
                vcx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
                    .contains("\"mnemonic\": \"ds_read_b32\"")
            );
            vcx.simulate_keystrokes("space");
            vcx.run_until_parked();
            host.update(&mut vcx, |ws, _, _| {
                let TxPanelState::Ready(view) = ws
                    .app
                    .panels
                    .transaction(details)
                    .unwrap()
                    .state(&ws.app.doc)
                else {
                    panic!("details ready")
                };
                assert!(!view.attributes.rows.iter().any(|row| row.key == "mnemonic"));
            })
            .unwrap();
        }
    }
}

/// User scenario: the Bottom Dock starts collapsed; clicking the header Bottom Dock
/// toggle opens it with the At Cursor panel, clicking again hides it while preserving
/// the panel, and clicking once more reopens it.
#[gpui_kit::test]
fn bottom_dock_toggle_opens_collapses_and_reopens_at_cursor_panel(cx: &mut TestAppContext) {
    use gpui_kit::base::dock::DockPlacement;
    init(cx);
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let track = writer
        .transactions::<()>(vtr::SourceOptions::new("instructions", None), "PIPELINE")
        .unwrap();
    let tx = writer.begin_at(track, 10, &(), None).unwrap();
    writer.end_at(tx, 20, vtr::Status::Ok).unwrap();
    writer.close().unwrap();
    let session = OpenSpec::Path(file.path().into()).open().unwrap();

    let host = cx.add_window(Workspace::new);
    let mut vcx = VisualTestContext::from_window(host.into(), cx);
    vcx.simulate_resize(size(px(1200.0), px(800.0)));
    host.update(&mut vcx, |ws, _, cx| {
        ws.set_session(session, cx);
        ws.app.sidebar_visible = false;
    })
    .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    host.update(&mut vcx, |ws, _, cx| {
        assert!(ws.app.panels.layout().bottom.is_none());
        let area = ws.dock.as_ref().unwrap().read(cx).area.read(cx);
        assert!(!area.has_dock(DockPlacement::Bottom));
    })
    .unwrap();

    // Click the header Bottom Dock toggle button to open it.
    let toggle = vcx.debug_bounds("toggle-bottom-dock").unwrap();
    vcx.simulate_click(toggle.center(), gpui_kit::Modifiers::default());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    let cursor_panel_id = host
        .update(&mut vcx, |ws, _, cx| {
            let bottom = ws.app.panels.layout().bottom.as_ref().unwrap();
            assert!(bottom.open);
            let id = bottom.layout.panels()[0];
            assert!(ws.app.panels.get(id).unwrap().kind.is_cursor_records());
            let area = ws.dock.as_ref().unwrap().read(cx).area.read(cx);
            assert!(area.is_dock_open(DockPlacement::Bottom));
            id
        })
        .unwrap();
    assert!(vcx.debug_bounds("tx-cursor-records").is_some());

    // Click again to collapse while preserving the At Cursor panel.
    let toggle = vcx.debug_bounds("toggle-bottom-dock").unwrap();
    vcx.simulate_click(toggle.center(), gpui_kit::Modifiers::default());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    host.update(&mut vcx, |ws, _, cx| {
        let bottom = ws.app.panels.layout().bottom.as_ref().unwrap();
        assert!(!bottom.open);
        assert_eq!(bottom.layout.panels(), vec![cursor_panel_id]);
        let area = ws.dock.as_ref().unwrap().read(cx).area.read(cx);
        assert!(!area.is_dock_open(DockPlacement::Bottom));
    })
    .unwrap();

    // Click a third time to reopen the same panel.
    let toggle = vcx.debug_bounds("toggle-bottom-dock").unwrap();
    vcx.simulate_click(toggle.center(), gpui_kit::Modifiers::default());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    host.update(&mut vcx, |ws, _, cx| {
        let bottom = ws.app.panels.layout().bottom.as_ref().unwrap();
        assert!(bottom.open);
        assert_eq!(bottom.layout.panels(), vec![cursor_panel_id]);
        let area = ws.dock.as_ref().unwrap().read(cx).area.read(cx);
        assert!(area.is_dock_open(DockPlacement::Bottom));
    })
    .unwrap();
}

/// User scenario: navigating inside Record Details (Jump, Back, Forward) never causes
/// the panel to claim it follows itself, and pressing Escape returns focus to the
/// external source panel rather than looping back to Record Details.
#[gpui_kit::test]
fn header_never_self_follows_and_escape_returns_to_source_panel(cx: &mut TestAppContext) {
    use volna_core::transaction::TransactionCommand;
    use volna_trace::data::transactions::TransactionRef;

    init(cx);
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let track = writer
        .transactions::<()>(vtr::SourceOptions::new("kernels", None), "PIPELINE")
        .unwrap();
    let first = writer.begin_at(track, 10, &(), None).unwrap();
    writer.end_at(first, 50, vtr::Status::Ok).unwrap();
    let second = writer.begin_at(track, 60, &(), None).unwrap();
    writer.end_at(second, 100, vtr::Status::Ok).unwrap();
    writer.close().unwrap();

    let session = OpenSpec::Path(file.path().into()).open().unwrap();
    let track = a(session.tracks()[0].id);
    let host = cx.add_window(Workspace::new);
    let mut vcx = VisualTestContext::from_window(host.into(), cx);
    vcx.simulate_resize(size(px(1200.0), px(800.0)));

    let waves = host
        .update(&mut vcx, |ws, window, cx| {
            ws.set_session(session, cx);
            ws.app.sidebar_visible = false;
            ws.dispatch(
                Command::AddToWaves(a_all(vec![Member::Generator(0)])),
                Some(window),
                cx,
            );
            ws.app.panels.focused_id()
        })
        .unwrap();
    vcx.run_until_parked();

    let details = host
        .update(&mut vcx, |ws, window, cx| {
            ws.dispatch(
                Command::SelectTransaction {
                    panel: waves,
                    track,
                    id: TransactionRef(first.id()),
                    cursor: None,
                },
                Some(window),
                cx,
            );
            ws.dispatch(Command::ShowTransaction { from: waves }, Some(window), cx);
            ws.app.panels.focused_id()
        })
        .unwrap();
    vcx.run_until_parked();

    // Jump to second record, then step Back: origin must remain the Waves panel, never details.
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::Transaction(
                details,
                TransactionCommand::Jump {
                    track,
                    id: TransactionRef(second.id()),
                },
            ),
            Some(window),
            cx,
        );
        assert_eq!(ws.app.doc.selection().unwrap().origin, waves);
        assert_eq!(
            ws.app
                .panels
                .transaction(details)
                .unwrap()
                .external_origin(),
            Some(waves)
        );
        ws.dispatch(
            Command::Transaction(details, TransactionCommand::Back),
            Some(window),
            cx,
        );
        assert_eq!(ws.app.doc.selection().unwrap().origin, waves);
        assert_ne!(ws.app.doc.selection().unwrap().origin, details);
    })
    .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    // Focus Record Details and press Escape: focus must return to Waves.
    host.update(&mut vcx, |ws, window, cx| {
        if let Some(focus) = ws.panel_focus(details, cx) {
            window.focus(&focus, cx);
        }
    })
    .unwrap();
    vcx.run_until_parked();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    host.update(&mut vcx, |ws, _, _| {
        assert_eq!(ws.app.panels.focused_id(), waves);
    })
    .unwrap();
}

/// User scenario: a record with >20 stages renders 100% of stages in the lifeline
/// (no truncation at `detail_items=20`), defaults to Linear scale, switches to Balanced
/// scale so micro-stages widen, supports inline "Show all" and "Grouped" stage views,
/// groups dotted attributes with visible DEC/HEX/BIN radix pills, hides Expand/Collapse
/// All when flat, and hides the empty Events 0 section.
#[gpui_kit::test]
fn full_range_lifeline_stages_attributes_radix_pills_and_empty_events(cx: &mut TestAppContext) {
    use volna_core::transaction::{LifelineScale, TxPanelState};
    use volna_trace::data::transactions::TransactionRef;
    use vtr::Value;

    init(cx);
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let track = writer
        .transactions::<()>(vtr::SourceOptions::new("gemm", None), "PIPELINE")
        .unwrap();
    let wait_name = writer.intern("wait").unwrap();
    let compute_name = writer.intern("compute").unwrap();
    let grid_x = writer.intern("kernel.grid_x").unwrap();
    let addr_key = writer.intern("mem.addr").unwrap();

    let owner = writer.begin_at(track, 0, &(), None).unwrap();
    writer
        .attribute(owner.owner(), grid_x, Value::U64(128))
        .unwrap();
    writer
        .attribute(owner.owner(), addr_key, Value::U64(4096))
        .unwrap();
    // 24 short micro-stages + 1 dominant compute stage (25 total > default 20 detail_items)
    for i in 0..24u64 {
        writer
            .embedded_stage(
                owner.owner(),
                i * 10,
                wait_name,
                0,
                Some(i * 10 + 10),
                vec![],
            )
            .unwrap();
    }
    writer
        .embedded_stage(owner.owner(), 240, compute_name, 0, Some(60_000), vec![])
        .unwrap();
    writer.end_at(owner, 60_000, vtr::Status::Ok).unwrap();
    writer.close().unwrap();

    let session = OpenSpec::Path(file.path().into()).open().unwrap();
    let track = a(session.tracks()[0].id);
    let host = cx.add_window(Workspace::new);
    let mut vcx = VisualTestContext::from_window(host.into(), cx);
    vcx.simulate_resize(size(px(1200.0), px(1600.0)));

    let waves = host
        .update(&mut vcx, |ws, window, cx| {
            ws.set_session(session, cx);
            ws.app.sidebar_visible = false;
            ws.dispatch(
                Command::AddToWaves(a_all(vec![Member::Generator(0)])),
                Some(window),
                cx,
            );
            ws.app.panels.focused_id()
        })
        .unwrap();
    vcx.run_until_parked();

    let details = host
        .update(&mut vcx, |ws, window, cx| {
            ws.dispatch(
                Command::SelectTransaction {
                    panel: waves,
                    track,
                    id: TransactionRef(owner.id()),
                    cursor: None,
                },
                Some(window),
                cx,
            );
            ws.dispatch(Command::ShowTransaction { from: waves }, Some(window), cx);
            ws.app.panels.focused_id()
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    // Verify full-range lifeline (25 cells, not truncated at 20), Linear default,
    // flat attributes hide expand/collapse all, dotted namespaces exist, and Events 0 is hidden.
    let linear_micro_width = host
        .update(&mut vcx, |ws, _, _| {
            let model = ws.app.panels.transaction(details).unwrap();
            assert_eq!(model.prefs().lifeline_scale, LifelineScale::Linear);
            assert!(!model.prefs().stages_grouped);
            let TxPanelState::Ready(view) = model.state(&ws.app.doc) else {
                panic!("details ready");
            };
            assert_eq!(view.lifeline[0].cells.len(), 25);
            assert_eq!(view.stages.total, 25);
            assert_eq!(view.stages.rows.len(), 20);
            assert_eq!(view.events.total, 0);
            assert!(!view.attributes.has_branches());
            view.lifeline[0].cells[0].width
        })
        .unwrap();

    assert!(vcx.debug_bounds("tx-expand-all").is_none());
    assert!(vcx.debug_bounds("tx-collapse-all").is_none());
    assert!(vcx.debug_bounds("tx-attr-ns-kernel").is_some());
    assert!(vcx.debug_bounds("tx-attr-ns-mem").is_some());
    assert!(vcx.debug_bounds("tx-stages-show-all").is_some());

    // Switch lifeline to Balanced scale: micro-stage becomes noticeably wider.
    vcx.update(|window, cx| window.click("tx-lifeline-balanced", cx));
    vcx.run_until_parked();
    host.update(&mut vcx, |ws, _, _| {
        let model = ws.app.panels.transaction(details).unwrap();
        assert_eq!(model.prefs().lifeline_scale, LifelineScale::Balanced);
        let TxPanelState::Ready(view) = model.state(&ws.app.doc) else {
            panic!("details ready");
        };
        assert!(view.lifeline[0].cells[0].width > linear_micro_width * 5.0);
    })
    .unwrap();

    // Click inline "Show all 25 stages": all 25 stages appear without settings detours.
    vcx.update(|window, cx| window.click("tx-stages-show-all", cx));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    host.update(&mut vcx, |ws, _, _| {
        let TxPanelState::Ready(view) = ws
            .app
            .panels
            .transaction(details)
            .unwrap()
            .state(&ws.app.doc)
        else {
            panic!("details ready");
        };
        assert_eq!(view.stages.rows.len(), 25);
    })
    .unwrap();
    assert!(vcx.debug_bounds("tx-stages-show-fewer").is_some());

    // Switch stages to Grouped view: 24 wait + 1 compute collapse into 2 groups.
    vcx.update(|window, cx| window.click("tx-stages-grouped", cx));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("tx-stage-group-0").is_some());
    assert!(vcx.debug_bounds("tx-stage-group-1").is_some());
    assert!(vcx.debug_bounds("tx-stage-group-2").is_none());

    // Click visible radix pill on first numeric attribute: DEC -> HEX (128 -> 0x80).
    let pill = vcx.debug_bounds("tx-attr-radix-0").unwrap();
    vcx.simulate_click(pill.center(), gpui_kit::Modifiers::default());
    vcx.run_until_parked();
    host.update(&mut vcx, |ws, _, _| {
        let TxPanelState::Ready(view) = ws
            .app
            .panels
            .transaction(details)
            .unwrap()
            .state(&ws.app.doc)
        else {
            panic!("details ready");
        };
        assert_eq!(view.attributes.rows[0].value, "0x80");
    })
    .unwrap();

    // Switch Attributes to Raw view: namespace headers disappear.
    vcx.update(|window, cx| window.click("tx-attr-raw", cx));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("tx-attr-ns-kernel").is_none());
}

/// User scenario: clicking Show in Waves, Pipeline, and Table from Record Details opens
/// a new panel when none displays the track, and focuses the existing panel when one does.
#[gpui_kit::test]
fn show_in_actions_focus_or_open_waves_pipeline_and_table(cx: &mut TestAppContext) {
    use volna_trace::data::transactions::TransactionRef;

    init(cx);
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let track = writer
        .transactions::<()>(vtr::SourceOptions::new("dma", None), "PIPELINE")
        .unwrap();
    let tx = writer.begin_at(track, 10, &(), None).unwrap();
    writer.end_at(tx, 90, vtr::Status::Ok).unwrap();
    writer.close().unwrap();

    let session = OpenSpec::Path(file.path().into()).open().unwrap();
    let track = a(session.tracks()[0].id);
    let host = cx.add_window(Workspace::new);
    let mut vcx = VisualTestContext::from_window(host.into(), cx);
    vcx.simulate_resize(size(px(1200.0), px(900.0)));

    // Open Pipeline only (no Waves lane, no Table panel yet), then open Record Details.
    let (pipeline, details) = host
        .update(&mut vcx, |ws, window, cx| {
            ws.set_session(session, cx);
            ws.app.sidebar_visible = false;
            ws.dispatch(Command::OpenPipeline { track }, Some(window), cx);
            let pipeline = ws.app.panels.focused_id();
            ws.dispatch(
                Command::SelectTransaction {
                    panel: pipeline,
                    track,
                    id: TransactionRef(tx.id()),
                    cursor: None,
                },
                Some(window),
                cx,
            );
            ws.dispatch(
                Command::ShowTransaction { from: pipeline },
                Some(window),
                cx,
            );
            (pipeline, ws.app.panels.focused_id())
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    // All three Show in buttons are present and clickable even when Waves & Table aren't open yet.
    assert!(vcx.debug_bounds("tx-reveal-selected").is_some());
    assert!(vcx.debug_bounds("tx-reveal").is_some());
    assert!(vcx.debug_bounds("tx-reveal-table").is_some());

    // Show in Table opens a new Table panel for the track and focuses it.
    vcx.update(|window, cx| window.click("tx-reveal-table", cx));
    vcx.run_until_parked();
    let table_id = host
        .update(&mut vcx, |ws, _, _| {
            let table = ws.app.panels.table_for_track(track).unwrap();
            assert_eq!(ws.app.panels.focused_id(), table);
            table
        })
        .unwrap();

    // Show in Waves adds/opens a Waves panel for the track and focuses it.
    vcx.update(|window, cx| window.click("tx-reveal-selected", cx));
    vcx.run_until_parked();
    let waves_id = host
        .update(&mut vcx, |ws, _, _| {
            let waves = ws.app.panels.waves_for_track(track).unwrap();
            assert_eq!(ws.app.panels.focused_id(), waves);
            waves
        })
        .unwrap();

    // Show in Pipeline focuses the existing Pipeline panel without opening a duplicate.
    vcx.update(|window, cx| window.click("tx-reveal", cx));
    vcx.run_until_parked();
    host.update(&mut vcx, |ws, _, _| {
        assert_eq!(ws.app.panels.focused_id(), pipeline);
        assert_eq!(ws.app.panels.table_for_track(track), Some(table_id));
        assert_eq!(ws.app.panels.waves_for_track(track), Some(waves_id));
        assert!(ws.app.panels.transaction(details).is_some());
    })
    .unwrap();
}
