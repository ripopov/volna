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
            (ws.app.panels.focused_id(), selected)
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
                let divider = vcx.debug_bounds("tx-cursor-divider").unwrap();
                let body = vcx.debug_bounds("tx-body").unwrap();
                for (time, expected) in [(200, 3), (350, 1), (500, 0), (20, 5000)] {
                    host.update(&mut vcx, |ws, _, cx| {
                        ws.app.doc.shared.cursor = Some(time);
                        cx.notify();
                    })
                    .unwrap();
                    vcx.run_until_parked();
                    vcx.update(|window, cx| window.draw(cx).clear(cx));
                    assert_eq!(
                        vcx.debug_bounds("tx-cursor-divider").unwrap(),
                        divider,
                        "cursor count must not move the divider"
                    );
                    assert_eq!(
                        vcx.debug_bounds("tx-body").unwrap(),
                        body,
                        "cursor count must not move the details"
                    );
                    host.update(&mut vcx, |ws, _, _| {
                        assert_eq!(
                            ws.app.cursor_records(details).ready().unwrap().count,
                            expected
                        );
                    })
                    .unwrap();
                }
                host.update(&mut vcx, |ws, _, _| {
                    let query = ws.app.cursor_records(details).ready().unwrap();
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
            let query = ws.app.cursor_records(details).ready().unwrap();
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
        let query = ws.app.cursor_records(details).ready().unwrap();
        assert_eq!(query.count, 0);
        assert_eq!(query.selected_rank, None);
        assert_eq!(query.selected, Some(previous));
        assert_eq!(ws.app.doc.selection().unwrap().id, previous);
    })
    .unwrap();
    check_cursor_split(&host, &mut vcx, details);
}

fn check_cursor_split(
    host: &gpui_kit::WindowHandle<Workspace>,
    vcx: &mut VisualTestContext,
    details: volna_core::panels::PanelId,
) {
    use gpui_kit::{Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, point};
    use volna_core::transaction::view::{DEFAULT_CURSOR_HEIGHT, MIN_CURSOR_HEIGHT};
    let selected = host
        .update(vcx, |ws, _, _| {
            ws.app
                .panels
                .transaction(details)
                .unwrap()
                .shown()
                .unwrap()
                .id
        })
        .unwrap();
    let draw = |vcx: &mut VisualTestContext| {
        host.update(vcx, |ws, _, _| {
            assert_eq!(
                ws.app
                    .panels
                    .transaction(details)
                    .unwrap()
                    .shown()
                    .unwrap()
                    .id,
                selected,
                "resizing and scrolling must preserve selection"
            );
        })
        .unwrap();
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
    };
    let height = |vcx: &mut VisualTestContext| {
        host.update(vcx, |ws, _, _| {
            ws.app
                .panels
                .transaction(details)
                .unwrap()
                .prefs()
                .cursor_height
        })
        .unwrap()
    };
    let drag = |vcx: &mut VisualTestContext, delta: f32| {
        let start = vcx.debug_bounds("tx-cursor-divider").unwrap().center();
        vcx.simulate_mouse_move(start, None, Modifiers::default());
        vcx.simulate_event(MouseDownEvent {
            position: start,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        draw(vcx);
        let end = point(start.x - px(700.0), start.y + px(delta));
        vcx.simulate_event(MouseMoveEvent {
            position: end,
            pressed_button: Some(MouseButton::Left),
            ..Default::default()
        });
        draw(vcx);
        vcx.simulate_event(MouseUpEvent {
            position: end,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        draw(vcx);
    };
    vcx.update(|_, cx| crate::theme::set_zoom(1.0, cx));
    draw(vcx);
    let before = vcx.debug_bounds("tx-cursor-divider").unwrap();
    drag(vcx, 72.0);
    assert_eq!(height(vcx), DEFAULT_CURSOR_HEIGHT + 72.0);
    assert_eq!(
        vcx.debug_bounds("tx-cursor-divider").unwrap().top(),
        before.top() + px(72.0)
    );
    vcx.simulate_mouse_move(point(px(10.0), px(10.0)), None, Modifiers::default());
    draw(vcx);
    assert_eq!(
        height(vcx),
        DEFAULT_CURSOR_HEIGHT + 72.0,
        "release outside panel ends resizing"
    );

    vcx.simulate_keystrokes("up shift-down");
    draw(vcx);
    assert_eq!(height(vcx), DEFAULT_CURSOR_HEIGHT + 120.0);
    vcx.simulate_keystrokes("home");
    draw(vcx);
    assert_eq!(height(vcx), MIN_CURSOR_HEIGHT);
    vcx.simulate_keystrokes("enter");
    draw(vcx);
    assert_eq!(height(vcx), DEFAULT_CURSOR_HEIGHT);

    let start = vcx.debug_bounds("tx-cursor-divider").unwrap().center();
    vcx.simulate_event(MouseDownEvent {
        position: start,
        button: MouseButton::Left,
        click_count: 1,
        ..Default::default()
    });
    draw(vcx);
    vcx.simulate_event(MouseMoveEvent {
        position: start + point(px(0.0), px(80.0)),
        pressed_button: Some(MouseButton::Left),
        ..Default::default()
    });
    draw(vcx);
    assert_eq!(height(vcx), DEFAULT_CURSOR_HEIGHT + 80.0);
    vcx.simulate_keystrokes("escape");
    draw(vcx);
    assert_eq!(
        height(vcx),
        DEFAULT_CURSOR_HEIGHT,
        "Escape restores the original height"
    );
    vcx.simulate_event(MouseUpEvent {
        position: start,
        button: MouseButton::Left,
        click_count: 1,
        ..Default::default()
    });
    draw(vcx);

    drag(vcx, -10000.0);
    assert_eq!(height(vcx), MIN_CURSOR_HEIGHT);
    drag(vcx, 10000.0);
    let body = vcx.debug_bounds("tx-details-viewport").unwrap();
    assert!(
        body.size.height >= px(159.0),
        "large drag reserves space for details"
    );
    let preferred = height(vcx);
    vcx.simulate_resize(size(px(360.0), px(360.0)));
    draw(vcx);
    let split = vcx.debug_bounds("tx-cursor-split").unwrap();
    let list = vcx.debug_bounds("tx-cursor-records").unwrap();
    let body = vcx.debug_bounds("tx-details-viewport").unwrap();
    assert!(list.size.height > px(0.0) && body.size.height > px(0.0));
    assert!(
        body.bottom() <= split.bottom() + px(1.0),
        "body {body:?}, split {split:?}, list {list:?}"
    );
    assert_eq!(
        height(vcx),
        preferred,
        "temporary size constraints preserve the preference"
    );
    vcx.simulate_resize(size(px(1200.0), px(800.0)));
    draw(vcx);
    assert_eq!(height(vcx), preferred);
    let before = vcx.debug_bounds("tx-record-path").unwrap();
    let viewport = vcx.debug_bounds("tx-details-viewport").unwrap();
    let list_before = vcx.debug_bounds("tx-cursor-records").unwrap();
    vcx.simulate_mouse_move(viewport.center(), None, Modifiers::default());
    vcx.simulate_event(gpui_kit::ScrollEvent {
        position: viewport.center(),
        delta: gpui_kit::ScrollDelta::Pixels(point(px(0.0), px(-80.0))),
        ..Default::default()
    });
    draw(vcx);
    assert!(
        vcx.debug_bounds("tx-record-path").unwrap().top() < before.top(),
        "details scroll within their viewport"
    );
    assert_eq!(vcx.debug_bounds("tx-cursor-records").unwrap(), list_before);
    vcx.simulate_event(gpui_kit::ScrollEvent {
        position: viewport.center(),
        delta: gpui_kit::ScrollDelta::Pixels(point(px(0.0), px(10000.0))),
        ..Default::default()
    });
    draw(vcx);
    let start = vcx.debug_bounds("tx-cursor-divider").unwrap().center();
    vcx.simulate_mouse_move(start, None, Modifiers::default());
    vcx.simulate_event(MouseDownEvent {
        position: start,
        button: MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    vcx.simulate_event(MouseUpEvent {
        position: start,
        button: MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    draw(vcx);
    assert_eq!(
        height(vcx),
        DEFAULT_CURSOR_HEIGHT,
        "double-click resets the split"
    );
}
