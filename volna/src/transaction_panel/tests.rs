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
    for _ in 0..5000 {
        let tx = writer.begin_at(track, 10, &(), None).unwrap();
        writer
            .attribute(tx.owner(), key, vtr::Value::Str(label))
            .unwrap();
        writer.end_at(tx, 100, vtr::Status::Ok).unwrap();
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
}
