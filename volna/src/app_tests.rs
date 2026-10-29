//! GPUI adapter tests: the load loop runs on the GPUI executor and theme changes leave core
//! state alone.

use super::*;
use gpui_kit::TestAppContext;
use std::sync::Arc;
use volna_core::session::{LoadRequest, LoadResult};
use volna_core::testing::ProceduralTrace;
use volna_core::testing::a_all;
use volna_core::trace::TraceId;
use volna_trace::session::OpenSpec;

const SELECT_ALL: &str = if cfg!(target_os = "macos") {
    "cmd-a"
} else {
    "ctrl-a"
};

fn init(cx: &mut TestAppContext) {
    cx.update(crate::init_app);
}

#[gpui_kit::test]
fn native_import_status_and_errors_remain_usable_across_themes_and_zoom(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{VisualTestContext, size};
    init(cx);
    let diagnostic = format!(
        "Cannot convert recording.fst to VTR (exit status: 2).\n{}",
        "invalid FST section in source recording\n".repeat(50)
    );
    for dark in [true, false] {
        for width in [800.0, 360.0] {
            for zoom in [1.0, 1.6] {
                let mut workspace = None;
                let root = cx.add_window(|window, cx| {
                    let ws = cx.new(|cx| Workspace::new(window, cx));
                    workspace = Some(ws.clone());
                    gpui_kit::component::Root::new(ws, window, cx)
                });
                let ws = workspace.unwrap();
                let mut vcx = VisualTestContext::from_window(root.into(), cx);
                vcx.simulate_resize(size(px(width), px(700.0)));
                let work = ws.update(&mut vcx, |ws, cx| {
                    crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                    crate::theme::set_zoom(zoom, cx);
                    ws.app.sidebar_visible = width >= 800.0;
                    ws.app
                        .open_path("recording with a long original source name.fst".into());
                    let request = ws.app.take_requests().remove(0);
                    let work = ws.native_imports.prepare(request);
                    ws.native_imports.set_progress(
                        TraceId::A,
                        vtr_progress::Snapshot {
                            phase: vtr_progress::Phase::Writing,
                            completed: 42,
                            total: Some(100),
                            input_bytes: 256 << 20,
                            output_bytes: 20 << 20,
                            ..Default::default()
                        },
                    );
                    assert_eq!(
                        ws.loading_message("recording.fst"),
                        "Converting recording.fst to VTR…"
                    );
                    ws.after(None, cx);
                    work
                });
                vcx.run_until_parked();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                let loading = vcx.debug_bounds("trace-loading").unwrap();
                let message = vcx.debug_bounds("trace-loading-message").unwrap();
                assert!(
                    loading.contains(&message.center()),
                    "dark={dark} width={width} zoom={zoom}: loading={loading:?}, message={message:?}"
                );
                assert!(message.left() >= px(0.0) && message.right() <= px(width));
                let bar = vcx.debug_bounds("conversion-bar-A").unwrap();
                let fill = vcx.debug_bounds("conversion-fill-A").unwrap();
                assert!((fill.size.width / bar.size.width - 0.42).abs() < 0.01);
                assert!(loading.contains(&bar.center()));
                ws.update(&mut vcx, |ws, cx| {
                    ws.native_imports.set_progress(
                        TraceId::A,
                        vtr_progress::Snapshot {
                            phase: vtr_progress::Phase::Writing,
                            completed: 75,
                            total: Some(100),
                            input_bytes: 256 << 20,
                            output_bytes: 48 << 20,
                            ..Default::default()
                        },
                    );
                    cx.notify();
                });
                vcx.run_until_parked();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                let fill = vcx.debug_bounds("conversion-fill-A").unwrap();
                assert!((fill.size.width / bar.size.width - 0.75).abs() < 0.01);
                ws.update(&mut vcx, |ws, cx| {
                    let generation = ws.app.doc.traces().get(TraceId::A).unwrap().generation();
                    let error = anyhow::anyhow!("{diagnostic}");
                    let result = LoadResult::Opened {
                        trace: TraceId::A,
                        generation,
                        result: Err(error),
                    };
                    ws.native_imports.finish(&result);
                    ws.app.deliver(result);
                    ws.after(None, cx);
                });
                drop(work);
                vcx.run_until_parked();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                assert!(vcx.debug_bounds("trace-loading").is_none());
                vcx.update(|window, cx| {
                    let details = window.find("trace-error-details").bounds();
                    assert!(details.left() >= px(0.0) && details.right() <= px(width));
                    assert!(details.top() >= px(0.0) && details.bottom() <= px(700.0));
                    window.click("trace-error-details", cx);
                });
                vcx.run_until_parked();
                vcx.update(|window, cx| {
                    let text = window
                        .focused_input(cx)
                        .unwrap()
                        .as_textarea()
                        .unwrap()
                        .clone();
                    assert_eq!(text.read(cx).value().as_ref(), diagnostic);
                    window.click("close-message", cx);
                });
                vcx.run_until_parked();
                vcx.update(|window, cx| {
                    let focus = ws.read(cx).focus_handle.clone();
                    window.focus(&focus, cx);
                });
                vcx.simulate_keystrokes("ctrl-o");
                vcx.run_until_parked();
                assert!(
                    vcx.did_prompt_for_paths(),
                    "open shortcut remains available after import failure"
                );
                vcx.simulate_path_prompt_response(|_| None);
                vcx.run_until_parked();
            }
        }
    }
}

#[gpui_kit::test]
fn failed_secondary_import_is_visible_without_replacing_the_open_recording(
    cx: &mut TestAppContext,
) {
    init(cx);
    let window = cx.add_window(Workspace::new);
    window
        .update(cx, |ws, _, cx| {
            ws.app.set_session(Arc::new(ProceduralTrace::new(100)));
            ws.app
                .add_resource(OpenSpec::Path("bad.fst".into()), "file:///bad.fst".into());
            let request = ws
                .app
                .take_requests()
                .into_iter()
                .find(|request| matches!(request, LoadRequest::Open { .. }))
                .unwrap();
            ws.app.deliver(
                request.fail(anyhow::anyhow!("Cannot convert bad.fst: truncated section")),
            );
            ws.after(None, cx);
            assert!(ws.app.doc.is_loaded());
            assert_eq!(ws.app.doc.traces().len(), 1);
            assert!(
                ws.app
                    .status()
                    .workspace_notice
                    .unwrap()
                    .contains("Cannot convert bad.fst")
            );
        })
        .unwrap();
}

#[gpui_kit::test]
fn conversion_cancel_preserves_other_recordings(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{VisualTestContext, size};
    init(cx);
    for secondary in [false, true] {
        for dark in [true, false] {
            let mut workspace = None;
            let root = cx.add_window(|window, cx| {
                let ws = cx.new(|cx| Workspace::new(window, cx));
                workspace = Some(ws.clone());
                gpui_kit::component::Root::new(ws, window, cx)
            });
            let ws = workspace.unwrap();
            let mut vcx = VisualTestContext::from_window(root.into(), cx);
            vcx.simulate_resize(size(px(800.0), px(700.0)));
            let (work, trace) = ws.update(&mut vcx, |ws, cx| {
                crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../vtr/vtr-convert/tests/fixtures/values.fst");
                if secondary {
                    ws.app.set_session(Arc::new(ProceduralTrace::new(100)));
                    ws.app
                        .add_resource(OpenSpec::Path(path), "file:///values.fst".into());
                } else {
                    ws.app.open_path(path);
                }
                let request = ws
                    .app
                    .take_requests()
                    .into_iter()
                    .find(|r| matches!(r, LoadRequest::Open { .. }))
                    .unwrap();
                let trace = match &request {
                    LoadRequest::Open { trace, .. } => *trace,
                    _ => unreachable!(),
                };
                let work = ws.native_imports.prepare(request);
                ws.native_imports.set_progress(
                    trace,
                    vtr_progress::Snapshot {
                        phase: vtr_progress::Phase::Finalizing,
                        input_bytes: 1000,
                        output_bytes: 2000,
                        ..Default::default()
                    },
                );
                ws.after(None, cx);
                (work, trace)
            });
            vcx.run_until_parked();
            vcx.update(|window, cx| {
                window.draw(cx).clear(cx);
                let bounds = window.find(format!("cancel-conversion-{trace}")).bounds();
                assert!(bounds.top() >= px(0.0) && bounds.bottom() <= px(700.0));
                window.click(format!("cancel-conversion-{trace}"), cx);
            });
            vcx.run_until_parked();
            ws.update(&mut vcx, |ws, _| {
                assert!(!ws.native_imports.converting(trace));
                assert_eq!(ws.app.doc.is_loaded(), secondary);
                assert_eq!(ws.app.doc.traces().len(), usize::from(secondary));
            });
            let LoadResult::Opened { result, .. } = work() else {
                panic!("open result")
            };
            assert!(result.err().unwrap().to_string().contains("cancelled"));
        }
    }
}

#[gpui_kit::test]
fn transaction_links_support_header_history_and_browser_shortcuts(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, VisualTestContext, size};
    use volna_core::{
        panels::PanelsCommand,
        testing::a,
        transaction::{ParentTraceState, SectionKey, TransactionCommand},
    };
    use volna_trace::data::transactions::AttributeValue;
    init(cx);
    for dark in [true, false] {
        for width in [900.0, 360.0] {
            let mut workspace = None;
            let root = cx.add_window(|window, cx| {
                let ws = cx.new(|cx| Workspace::new(window, cx));
                workspace = Some(ws.clone());
                gpui_kit::component::Root::new(ws, window, cx)
            });
            let window = RootWindow {
                root,
                workspace: workspace.unwrap(),
            };
            let mut vcx = VisualTestContext::from_window(root.into(), cx);
            vcx.simulate_resize(size(px(width), px(1000.0)));
            let (panel, request, instruction) = window
                .update(&mut vcx, |ws, window, cx| {
                    let session = OpenSpec::Path(
                        concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/examples/pipeline_showcase.vtr"
                        )
                        .into(),
                    )
                    .open()
                    .unwrap();
                    let track = session
                        .tracks()
                        .iter()
                        .find(|t| t.path.join(".") == "soc.l2.bus.read")
                        .unwrap()
                        .id;
                    let instructions = session.tracks().iter()
                        .find(|t| t.path.join(".") == "soc.cpu0.pipeline.instruction")
                        .unwrap().id;
                    let instruction = session.load_track(instructions).unwrap().generators.iter()
                        .flat_map(|g| g.transactions())
                        .find(|t| t.attributes.iter().any(|a| a.key == "vtr.label"
                            && matches!(&a.value, AttributeValue::Text(label) if label == "80000060: lw   a1, 0(a2)")))
                        .unwrap().id;
                    let request = session.load_track(track).unwrap().generators.iter()
                        .flat_map(|g| g.transactions()).find(|t| t.parent == Some(instruction))
                        .unwrap().id;
                    ws.set_session(session, cx);
                    ws.app.sidebar_visible = false;
                    let origin = ws.app.panels.focused_id();
                    ws.dispatch(
                        Command::SelectTransaction {
                            panel: origin,
                            track: a(track),
                            id: request,
                            cursor: None,
                        },
                        Some(window),
                        cx,
                    );
                    ws.dispatch(Command::ShowTransaction { from: origin }, Some(window), cx);
                    let panel = ws.app.panels.focused_id();
                    ws.dispatch(
                        Command::Panels(PanelsCommand::Close(origin)),
                        Some(window),
                        cx,
                    );
                    for key in SectionKey::ALL {
                        ws.dispatch(
                            Command::Transaction(panel, TransactionCommand::Collapse(key, true)),
                            Some(window),
                            cx,
                        );
                    }
                    crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                    (panel, request, instruction)
                })
                .unwrap();
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let navigation = |vcx: &mut VisualTestContext| {
                window
                    .update(vcx, |ws, _, _| {
                        let model = ws.app.panels.transaction(panel).unwrap();
                        (
                            model.shown().unwrap().id.0,
                            model.can_go_back(),
                            model.can_go_forward(),
                        )
                    })
                    .unwrap()
            };
            let click = |vcx: &mut VisualTestContext, selector: &'static str| {
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                let bounds = vcx.debug_bounds(selector).expect(selector);
                vcx.simulate_click(bounds.center(), Modifiers::default());
                vcx.run_until_parked();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
            };
            assert_eq!(navigation(&mut vcx), (request.0, false, false));
            click(&mut vcx, "tx-back");
            click(&mut vcx, "tx-forward");
            assert_eq!(navigation(&mut vcx), (request.0, false, false));
            let header = vcx.debug_bounds("tx-header").unwrap();
            let back = vcx.debug_bounds("tx-back").unwrap();
            let forward = vcx.debug_bounds("tx-forward").unwrap();
            let body = vcx.debug_bounds("tx-body").unwrap();
            assert!(header.bottom() <= body.top());
            assert!(back.left() >= header.left() && forward.right() <= header.right());
            assert!(vcx.debug_bounds("tx-parent-graph").is_none());
            let button = vcx
                .debug_bounds("tx-trace-parents")
                .expect("on-demand trace button");
            vcx.simulate_click(button.center(), Modifiers::default());
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let graph = vcx.debug_bounds("tx-parent-graph").expect("parent graph");
            let root_node = vcx.debug_bounds("tx-parent-node-0").expect("oldest parent");
            let current = vcx
                .debug_bounds("tx-parent-node-1")
                .expect("selected transaction");
            assert!(graph.size.width > px(0.0) && graph.size.width <= px(width));
            assert!(root_node.bottom() < current.top());
            window
                .update(&mut vcx, |ws, _, _| {
                    let parents = ws.app.panels.transaction(panel).unwrap().parents();
                    assert_eq!(parents.state(), &ParentTraceState::Ready);
                    assert_eq!(parents.nodes().len(), 2);
                })
                .unwrap();
            vcx.simulate_click(root_node.center(), Modifiers::default());
            vcx.run_until_parked();
            window
                .update(&mut vcx, |ws, _, _| {
                    let model = ws.app.panels.transaction(panel).unwrap();
                    assert_eq!(model.shown().unwrap().id, instruction);
                    assert_eq!(model.parents().state(), &ParentTraceState::Idle);
                })
                .unwrap();
            assert_eq!(navigation(&mut vcx), (instruction.0, true, false));
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let follows = vcx.debug_bounds("tx-follows").unwrap();
            assert!(follows.bottom() <= vcx.debug_bounds("tx-back").unwrap().top());
            click(&mut vcx, "tx-back");
            assert_eq!(navigation(&mut vcx), (request.0, false, true));
            click(&mut vcx, "tx-back");
            assert_eq!(navigation(&mut vcx), (request.0, false, true));
            click(&mut vcx, "tx-forward");
            assert_eq!(navigation(&mut vcx), (instruction.0, true, false));
            click(&mut vcx, "tx-forward");
            assert_eq!(navigation(&mut vcx), (instruction.0, true, false));
            for (back, forward) in [("alt-left", "alt-right"), ("cmd-[", "cmd-]")] {
                vcx.simulate_keystrokes(back);
                vcx.run_until_parked();
                assert_eq!(navigation(&mut vcx), (request.0, false, true));
                vcx.simulate_keystrokes(forward);
                vcx.run_until_parked();
                assert_eq!(navigation(&mut vcx), (instruction.0, true, false));
            }
            click(&mut vcx, "tx-back");
            window
                .update(&mut vcx, |ws, window, cx| {
                    ws.dispatch(
                        Command::Transaction(
                            panel,
                            TransactionCommand::Collapse(SectionKey::Related, false),
                        ),
                        Some(window),
                        cx,
                    );
                })
                .unwrap();
            vcx.run_until_parked();
            click(&mut vcx, "tx-ref-0");
            assert_eq!(navigation(&mut vcx), (instruction.0, true, false));
            vcx.simulate_keystrokes("alt-right");
            vcx.run_until_parked();
            assert_eq!(navigation(&mut vcx), (instruction.0, true, false));

            window
                .update(&mut vcx, |ws, window, cx| {
                    for key in SectionKey::ALL {
                        ws.dispatch(
                            Command::Transaction(panel, TransactionCommand::Collapse(key, false)),
                            Some(window),
                            cx,
                        );
                    }
                })
                .unwrap();
            vcx.simulate_resize(size(px(width), px(480.0)));
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let before = vcx.debug_bounds("tx-back").unwrap();
            let body = vcx.debug_bounds("tx-body").unwrap();
            let record = vcx.debug_bounds("tx-record-path").unwrap();
            let scroll_at = gpui_kit::point(body.center().x, before.bottom() + px(60.0));
            vcx.simulate_mouse_move(scroll_at, None, Modifiers::default());
            vcx.simulate_event(gpui_kit::ScrollWheelEvent {
                position: scroll_at,
                delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.0), px(-300.0))),
                ..Default::default()
            });
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(vcx.debug_bounds("tx-back").unwrap(), before);
            assert!(vcx.debug_bounds("tx-record-path").unwrap().top() < record.top());
            click(&mut vcx, "tx-back");
            assert_eq!(navigation(&mut vcx), (request.0, false, true));
        }
    }
}

#[gpui_kit::test]
fn desktop_startup_hosts_workspace_and_dialogs(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use gpui_kit::component::WindowExt as _;

    init(cx);
    for embedded in [false, true] {
        let (workspace, handle) = cx.update(|cx| {
            let workspace = crate::open_main_window(cx, embedded).unwrap();
            let handle = *cx.windows().last().unwrap();
            assert_eq!(workspace.read(cx).embedded, embedded);
            (workspace, handle)
        });
        let mut vcx = VisualTestContext::from_window(handle, cx);
        vcx.update(|window, cx| {
            let root = gpui_kit::base::Root::read(window, cx);
            assert_eq!(root.view().entity_id(), workspace.entity_id());
            window.open_dialog(cx, |dialog, _, _| {
                dialog.child(
                    div()
                        .debug_selector(|| "startup-dialog".into())
                        .child("Startup dialog"),
                )
            });
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("startup-dialog").is_some());
        vcx.update(gpui_kit::component::WindowExt::close_dialog);
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("startup-dialog").is_none());
    }
}

#[gpui_kit::test]
fn workspace_details_wrap_scroll_and_copy_complete_errors(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Modifiers, VisualTestContext, size};

    init(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let first = format!(
        "Workspace restore failed; autosave paused: /{}trace.vtr: invalid start panel",
        "long_directory_name/".repeat(80)
    );
    let second = "Missing trace: /tmp/волна.vtr\nCaused by: file not found\n".repeat(20);
    for dark in [true, false] {
        for width in [900.0, 360.0] {
            for scheduler_only in [false, true] {
                let mut workspace = None;
                let root = cx.add_window(|window, cx| {
                    let ws = cx.new(|cx| Workspace::new(window, cx));
                    workspace = Some(ws.clone());
                    gpui_kit::component::Root::new(ws, window, cx)
                });
                let workspace = RootWindow {
                    root,
                    workspace: workspace.unwrap(),
                };
                let mut vcx = VisualTestContext::from_window(root.into(), cx);
                vcx.simulate_resize(size(px(width), px(480.0)));
                let expected = if scheduler_only {
                    first.clone()
                } else {
                    format!("{first}\n\n{second}")
                };
                workspace
                    .update(&mut vcx, |ws, _, cx| {
                        crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                        if scheduler_only {
                            ws.app.workspace.scheduler.suspend(first.clone());
                        } else {
                            ws.app.workspace.notices = vec![first.clone(), second.clone()];
                        }
                        cx.notify();
                    })
                    .unwrap();
                vcx.run_until_parked();
                vcx.update(gpui_kit::test::TestWindowExt::render_frame);
                let notice = vcx.debug_bounds("workspace-notice").unwrap();
                vcx.simulate_click(notice.center(), Modifiers::default());
                vcx.run_until_parked();
                vcx.update(|window, cx| {
                    window.render_frame(cx);
                    let text = window
                        .focused_input(cx)
                        .expect("message is focused")
                        .as_textarea()
                        .expect("selectable diagnostic text")
                        .clone();
                    assert_eq!(text.read(cx).value().as_ref(), expected);
                    let bounds = text.read(cx).input_bounds();
                    assert!(bounds.left() >= px(0.0) && bounds.right() <= px(width));
                    assert!(bounds.top() >= px(0.0) && bounds.bottom() <= px(480.0));
                    let close = window.find("close-message").bounds();
                    assert!(close.top() >= bounds.bottom() && close.bottom() <= px(480.0));

                    #[cfg(target_os = "macos")]
                    let (start, end, select_all, copy) = ("cmd-up", "cmd-down", "cmd-a", "cmd-c");
                    #[cfg(not(target_os = "macos"))]
                    let (start, end, select_all, copy) =
                        ("ctrl-home", "ctrl-end", "ctrl-a", "ctrl-c");
                    window.press(start, cx);
                    let first_caret = text.read(cx).cursor_layout().unwrap().0;
                    window.press("down", cx);
                    let next_caret = text.read(cx).cursor_layout().unwrap().0;
                    assert!(next_caret.top() > first_caret.top());
                    assert!(text.read(cx).cursor() < first.find('\n').unwrap_or(first.len()));
                    window.drag(first_caret.center(), next_caret.center(), cx);
                    let selection = text.read(cx).selected_range();
                    assert!(!selection.is_empty());
                    window.press(copy, cx);
                    assert_eq!(
                        cx.read_from_clipboard().unwrap().text().unwrap(),
                        expected[selection]
                    );
                    window.press(end, cx);
                    assert!(text.read(cx).scroll_offset().y < px(0.0));
                    assert_eq!(text.read(cx).scroll_offset().x, px(0.0));
                    window.press(select_all, cx);
                    window.press(copy, cx);
                    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), expected);
                    window.press("backspace", cx);
                    window.input("replacement", cx);
                    assert_eq!(text.read(cx).value().as_ref(), expected);
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("old".into()));
                    window.click("copy-message", cx);
                    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), expected);
                    window.click("close-message", cx);
                });
                vcx.run_until_parked();
                assert!(vcx.debug_bounds("message-details").is_none());
                let notice = vcx.debug_bounds("workspace-notice").unwrap();
                vcx.simulate_click(notice.center(), Modifiers::default());
                vcx.run_until_parked();
                vcx.update(|window, cx| {
                    window.render_frame(cx);
                    assert_eq!(
                        window.focused_input(cx).unwrap().value(cx).as_ref(),
                        expected
                    );
                });
            }
        }
    }
}

#[gpui_kit::test]
fn desktop_window_matches_shell_identity(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let options = crate::window_options(cx);
        assert_eq!(options.app_id.as_deref(), Some(crate::desktop::APP_ID));
        let launcher = include_str!("../packaging/io.github.ripopov.volna.desktop");
        let wm_class = launcher
            .lines()
            .find_map(|line| line.strip_prefix("StartupWMClass="));
        assert_eq!(options.app_id.as_deref(), wm_class);
        #[cfg(target_os = "linux")]
        {
            let icon = options.icon.expect("X11 window icon");
            assert_eq!(icon.dimensions(), (256, 256));
            assert!(icon.pixels().any(|pixel| pixel.0[0] > 240));
        }
    });
}

#[gpui_kit::test]
fn loads_run_on_the_executor_and_fill_rows(cx: &mut TestAppContext) {
    init(cx);
    let window = cx.add_window(Workspace::new);
    let trace = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr");
    window
        .update(cx, |ws, _, cx| ws.open_path(trace.into(), cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, window, cx| {
            assert!(ws.app.doc.is_loaded());
            let count = ws.app.doc.hierarchy(TraceId::A).unwrap().var_count();
            ws.dispatch(Command::AddVars(a_all(0..count)), Some(window), cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, _, _| {
            assert!(!ws.app.panels.focused_waves().unwrap().items().is_empty());
            assert_eq!(
                ws.app.panels.focused_waves().unwrap().loaded_count(),
                ws.app.panels.focused_waves().unwrap().items().len()
            );
        })
        .unwrap();
}

#[gpui_kit::test]
fn scope_rows_without_activity_hide_counts_in_both_themes(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::sidebar::{ScopeTreeModel, TreeNode};
    use volna_core::trace::Traced;
    init(cx);
    let window = cx.add_window(Workspace::new);
    let trace = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr");
    let directory = tempfile::tempdir().unwrap();
    let specimen = directory.path().join("picorv32.vtr");
    std::fs::copy(trace, &specimen).unwrap();
    window
        .update(cx, |ws, _, cx| {
            ws.app
                .settings_loaded(r#"{"hierarchy.activityIndex":"never"}"#);
            ws.open_path(specimen.clone(), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    let root = window
        .update(&mut vcx, |ws, _, _| {
            let root = ws
                .app
                .doc
                .hierarchy(TraceId::A)
                .unwrap()
                .roots()
                .first()
                .unwrap();
            let size = ScopeTreeModel::size(
                ws.app.doc.traces(),
                TreeNode::scope(Traced::new(TraceId::A, root)),
            )
            .expect("counted after open");
            assert!(size.signals > 0 && size.variables >= size.signals);
            assert!(
                ws.app
                    .scope_activity(TreeNode::scope(Traced::new(TraceId::A, root)))
                    .is_none()
            );
            root
        })
        .unwrap();
    for appearance in [
        crate::theme::Appearance::Light,
        crate::theme::Appearance::Dark,
    ] {
        vcx.update(|_, cx| {
            crate::theme::install(
                crate::theme::CoreTheme::from_host(&crate::theme::HostPalette {
                    appearance,
                    ..Default::default()
                }),
                cx,
            );
        });
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let row = vcx
            .debug_bounds(Box::leak(format!("scope-row-A-{root}").into_boxed_str()))
            .expect("root scope row");
        assert!(row.size.width > gpui_kit::px(0.0), "{appearance:?}");
        assert!(
            vcx.debug_bounds(Box::leak(format!("scope-size-A-{root}").into_boxed_str()))
                .is_none(),
            "{appearance:?}: signal totals belong in the tooltip"
        );
        assert!(
            vcx.debug_bounds(Box::leak(format!("scope-meter-A-{root}").into_boxed_str()))
                .is_none()
        );
    }
}

#[gpui_kit::test]
fn inactive_signal_names_follow_the_viewport_and_preserve_selection(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::sidebar::{Key, TreeNode};
    use volna_core::trace::Traced;
    use volna_core::wave::Viewport;
    use volna_trace::data::Member;
    use vtr::Recorder;
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("activity.vtr");
    let mut writer = vtr::Writer::create(&path).unwrap();
    let top = writer
        .scope_with(
            vtr::SourceOptions::new("top", None),
            vtr::ScopeType::Module,
            "top",
        )
        .unwrap();
    let mut signals = Vec::new();
    for name in ["changing", "constant"] {
        signals.push(
            writer
                .signal(
                    vtr::SourceOptions::new(name, Some(top)),
                    vtr::SignalKind::Bits {
                        width: 1,
                        states: 2,
                    },
                    vtr::VarType::Wire,
                    vtr::Direction::Input,
                )
                .unwrap(),
        );
    }
    writer
        .transactions::<()>(vtr::SourceOptions::new("transactions", Some(top)), "raw")
        .unwrap();
    writer.advance_time(0).unwrap();
    for &signal in &signals {
        writer.emit_u64(signal, 0).unwrap();
    }
    writer.advance_time(10).unwrap();
    writer.emit_u64(signals[0], 1).unwrap();
    writer.advance_time(100).unwrap();
    writer.close().unwrap();
    let window = cx.add_window(Workspace::new);
    window
        .update(cx, |ws, _, cx| ws.open_path(path.clone(), cx))
        .unwrap();
    let mut view = VisualTestContext::from_window(window.into(), cx);
    view.run_until_parked();
    let (changing, constant, generator) = window
        .update(&mut view, |ws, _, cx| {
            let scope = ws
                .app
                .doc
                .hierarchy(TraceId::A)
                .unwrap()
                .roots()
                .first()
                .unwrap();
            ws.app
                .handle(Command::SelectScope(TreeNode::scope(Traced::new(
                    TraceId::A,
                    scope,
                ))));
            ws.after(None, cx);
            let find = |name: &str| {
                ws.app
                    .variables
                    .rows
                    .iter()
                    .position(|m| {
                        ws.app.doc.hierarchy(m.trace).unwrap().member_name(m.item) == name
                    })
                    .unwrap()
            };
            (find("changing"), find("constant"), find("transactions"))
        })
        .unwrap();
    for dark in [true, false] {
        view.update(|_, cx| crate::theme::set(crate::theme::CoreTheme::volna(dark), cx));
        for (window_range, selected, muted) in [
            ((0.0, 100.0), None, vec![constant]),
            ((0.0, 100.0), Some(constant), vec![]),
            ((20.0, 30.0), None, vec![changing, constant]),
        ] {
            window
                .update(&mut view, |ws, _, cx| {
                    ws.app.doc.shared.viewport.set(Viewport {
                        start: window_range.0,
                        end: window_range.1,
                    });
                    ws.app
                        .handle(Command::VariablesKey(Key::Escape, Default::default()));
                    if let Some(ix) = selected {
                        ws.app.handle(Command::SelectVar {
                            ix,
                            modifiers: Default::default(),
                        });
                    }
                    ws.after(None, cx);
                })
                .unwrap();
            view.run_until_parked();
            view.update(|window, cx| window.draw(cx).clear(cx));
            for ix in [changing, constant, generator] {
                let selector = Box::leak(format!("inactive-member-A-{ix}").into_boxed_str());
                assert_eq!(
                    view.debug_bounds(selector).is_some(),
                    muted.contains(&ix),
                    "dark={dark}, row={ix}, window={window_range:?}, selected={selected:?}"
                );
            }
        }
    }
    let bytes = std::fs::read(path).unwrap();
    window
        .update(&mut view, |ws, _, cx| {
            ws.set_session(
                OpenSpec::Bytes {
                    name: "unindexed.vtr".into(),
                    bytes,
                }
                .open()
                .unwrap(),
                cx,
            );
            ws.app
                .handle(Command::SelectScope(TreeNode::scope(Traced::new(
                    TraceId::A,
                    0,
                ))));
            ws.after(None, cx);
            assert!(
                ws.app
                    .variables
                    .rows
                    .iter()
                    .any(|m| matches!(m.item, Member::Var(_)))
            );
        })
        .unwrap();
    view.run_until_parked();
    view.update(|window, cx| window.draw(cx).clear(cx));
    for ix in [changing, constant] {
        let selector = Box::leak(format!("inactive-member-A-{ix}").into_boxed_str());
        assert!(
            view.debug_bounds(selector).is_none(),
            "no index means unknown activity"
        );
    }
}

#[gpui_kit::test]
fn scope_rows_show_activity_meters_in_both_themes(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::sidebar::TreeNode;
    use volna_core::trace::Traced;
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let trace = dir.path().join("picorv32.vtr");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr"),
        &trace,
    )
    .unwrap();
    let reader = vtr::Reader::open(&trace).unwrap();
    let id = vtr::activity::Identity::of(&reader).unwrap();
    vtr::activity::Sidecar::new(&trace, &id, None)
        .write(|w| vtr::activity::build(&reader, w, &Default::default()))
        .unwrap();
    let window = cx.add_window(Workspace::new);
    window
        .update(cx, |ws, _, cx| ws.open_path(trace.clone(), cx))
        .unwrap();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.run_until_parked();
    let root = window
        .update(&mut vcx, |ws, _, _| {
            let root = ws
                .app
                .doc
                .hierarchy(TraceId::A)
                .unwrap()
                .roots()
                .first()
                .unwrap();
            let a = ws
                .app
                .scope_activity(TreeNode::scope(Traced::new(TraceId::A, root)))
                .expect("meters once classified");
            assert!(a.changing > 0 && a.changing <= a.total, "{a:?}");
            root
        })
        .unwrap();
    for appearance in [
        crate::theme::Appearance::Light,
        crate::theme::Appearance::Dark,
    ] {
        vcx.update(|_, cx| {
            crate::theme::install(
                crate::theme::CoreTheme::from_host(&crate::theme::HostPalette {
                    appearance,
                    ..Default::default()
                }),
                cx,
            );
        });
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let mut bounds = |name: String| vcx.debug_bounds(Box::leak(name.into_boxed_str()));
        let row = bounds(format!("scope-row-A-{root}")).expect("root scope row");
        let meter = bounds(format!("scope-meter-A-{root}")).expect("meter");
        assert!(
            bounds(format!("scope-size-A-{root}")).is_none(),
            "the meter replaces the size"
        );
        assert!(
            bounds(format!("scope-activity-A-{root}")).is_none(),
            "activity counts are available in the tooltip"
        );
        assert!(
            meter.right() <= row.right() && meter.size.width > gpui_kit::px(20.0),
            "{appearance:?}: {meter:?} in {row:?}"
        );
        assert!(row.top() <= meter.top() && meter.bottom() <= row.bottom());
    }
}

#[gpui_kit::test]
fn feature_showcase_workspace_restores_with_scope_activity_meters(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::sidebar::TreeNode;
    use volna_core::trace::Traced;
    use volna_core::workspace::Workspace as SavedWorkspace;
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let trace = dir.path().join("feature_showcase.vtr");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/examples/feature_showcase.vtr"),
        &trace,
    )
    .unwrap();
    let reader = vtr::Reader::open(&trace).unwrap();
    let identity = vtr::activity::Identity::of(&reader).unwrap();
    vtr::activity::Sidecar::new(&trace, &identity, None)
        .write(|w| vtr::activity::build(&reader, w, &Default::default()))
        .unwrap();
    let session = OpenSpec::Path(trace.clone()).open().unwrap();
    let saved =
        SavedWorkspace::parse(include_bytes!("../tests/fixtures/showcase.volna.json")).unwrap();
    let uri = crate::native_workspace::file_uri(&trace).unwrap();
    let window = cx.add_window(Workspace::new);
    window
        .update(cx, |ws, window, cx| {
            ws.app.set_session(session.clone());
            saved
                .prepare(&ws.app, &uri, &format!("{uri}.volna.json"))
                .unwrap()
                .commit(&mut ws.app)
                .unwrap();
            ws.after(Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.run_until_parked();
    let soc = window
        .update(&mut vcx, |ws, _, _| {
            let h = session.hierarchy();
            let soc = h
                .roots()
                .iter()
                .find(|&id| h.scope(id).name == "soc")
                .unwrap();
            let activity = ws
                .app
                .scope_activity(TreeNode::scope(Traced::new(TraceId::A, soc)))
                .expect("restored workspace has activity counts");
            assert!(activity.changing > 0);
            soc
        })
        .unwrap();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = vcx
        .debug_bounds(Box::leak(format!("scope-meter-A-{soc}").into_boxed_str()))
        .expect("activity meter is rendered after restore");
    assert!(bounds.size.width > gpui_kit::px(0.0));
}

#[gpui_kit::test]
fn latest_open_wins_and_a_stale_open_cannot_replace_the_trace(cx: &mut TestAppContext) {
    init(cx);
    let window = cx.add_window(Workspace::new);
    let trace = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr");
    let slow = window
        .update(cx, |ws, _, cx| {
            ws.app.open_path(trace.into());
            let mut reqs = ws.app.take_requests();
            ws.after(None, cx);
            reqs.pop().unwrap()
        })
        .unwrap();
    let current = ProceduralTrace::session(7);
    window
        .update(cx, |ws, _, cx| {
            ws.app.open_bytes("current.vtr".into(), Vec::new());
            let LoadRequest::Open { generation, .. } = ws.app.take_requests().pop().unwrap() else {
                panic!("expected an open request");
            };
            ws.app.deliver(LoadResult::Opened {
                trace: TraceId::A,
                generation,
                result: Ok(current.clone()),
            });
            ws.after(None, cx);
        })
        .unwrap();
    window
        .update(cx, |_, _, cx| Workspace::spawn_request(slow, cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, _, _| {
            assert!(
                matches!(ws.app.trace_state(), TraceState::Loaded(s) if Arc::ptr_eq(&s, &current))
            );
            assert!(matches!(
                ws.app.panels.focused().kind,
                volna_core::panels::PanelKind::Start
            ));
        })
        .unwrap();
}

#[gpui_kit::test]
fn theme_changes_preserve_trace_and_interaction_state(cx: &mut TestAppContext) {
    init(cx);
    let window = cx.add_window(Workspace::new);
    let source: Arc<dyn Session> = Arc::new(ProceduralTrace::new(100));
    window
        .update(cx, |ws, _, cx| {
            ws.set_session(source.clone(), cx);
            ws.app.handle(Command::SetSidebarWidth(355.0));
            ws.app.handle(Command::SetScopesFraction(0.61));
            ws.app.handle(Command::AddVars(a_all(vec![0; 100])));
            ws.after(None, cx);
            ws.app.doc.shared.cursor = Some(42);
            ws.app
                .panels
                .focused_waves_mut()
                .unwrap()
                .selected
                .insert(1);
            ws.app.panels.focused_waves_mut().unwrap().anchor = Some(1);
            ws.app.doc.shared.viewport.value.start = 20.0;
            ws.app.doc.shared.viewport.value.end = 80.0;
            ws.app.panels.focused_waves_mut().unwrap().names_width = 260.0;
            ws.app.panels.focused_waves_mut().unwrap().values_width = 140.0;
            ws.app.panels.focused_waves_mut().unwrap().scroll_y = 24.0;
            ws.app.doc.add_marker(30);
        })
        .unwrap();
    cx.run_until_parked();
    let (before, history) = window
        .update(cx, |ws, _, _| {
            (
                ws.debug_state(),
                ws.app.panels.focused_waves().unwrap().items()[0]
                    .signal()
                    .unwrap()
                    .history
                    .clone()
                    .unwrap(),
            )
        })
        .unwrap();
    let host_themes = [
        crate::theme::Appearance::Light,
        crate::theme::Appearance::Dark,
        crate::theme::Appearance::HighContrastDark,
        crate::theme::Appearance::HighContrastLight,
    ]
    .map(|appearance| {
        crate::theme::Themes::uniform(crate::theme::CoreTheme::from_host(
            &crate::theme::HostPalette {
                appearance,
                ..Default::default()
            },
        ))
    });
    let builtins = volna_core::theme::builtin::choices()
        .map(|(id, _)| volna_core::theme::builtin::resolve(id, true).unwrap());
    for themes in host_themes.into_iter().chain(builtins) {
        cx.update(|cx| crate::theme::install_themes(themes, cx));
        cx.run_until_parked();
        window
            .update(cx, |ws, _, _| {
                assert!(
                    matches!(ws.app.trace_state(), TraceState::Loaded(s) if Arc::ptr_eq(&s, &source))
                );
                assert_eq!(ws.debug_state(), before);
                let w = &ws.app.panels.focused_waves().unwrap();
                assert!(Arc::ptr_eq(
                    w.items()[0].signal().unwrap().history.as_ref().unwrap(),
                    &history
                ));
                assert_eq!(w.names_width, 260.0);
                assert_eq!(w.values_width, 140.0);
                assert_eq!(w.scroll_y, 24.0);
                assert_eq!(ws.app.doc.markers()[0].time, 30);
            })
            .unwrap();
    }
}

#[gpui_kit::test]
fn native_idle_save_reopens_a_copied_trace_with_its_workspace(cx: &mut TestAppContext) {
    use crate::native_workspace::Store;
    use volna_core::workspace::persistence::Persistence;
    init(cx);
    let temporary = tempfile::tempdir().unwrap();
    let trace = temporary.path().join("trace.vtr");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr"),
        &trace,
    )
    .unwrap();
    let mut store = Store::new(Some(temporary.path().join("config"))).unwrap();
    store.data_dir = temporary.path().join("fallback");
    let window = cx.add_window(Workspace::new);
    window
        .update(cx, |ws, _, cx| {
            ws.enable_native_persistence(store, Persistence::Auto, false, cx);
            ws.open_path(trace.clone(), cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, _, cx| {
            assert!(ws.app.doc.is_loaded());
            assert!(
                !ws.app.workspace.scheduler.suspended(),
                "{:?}",
                ws.app.workspace.notices
            );
            ws.dispatch(Command::AddVars(a_all(vec![0, 1])), None, cx);
            ws.dispatch(Command::Action(Action::SplitRight), None, cx);
            ws.app.tick(Instant::now() + Duration::from_secs(2));
            ws.after(None, cx);
            assert!(!ws.app.workspace.scheduler.dirty());
            assert!(temporary.path().join("trace.vtr.volna.json").exists());
            ws.open_path(trace.clone(), cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, _, _| {
            assert_eq!(ws.app.panels.len(), 2);
            for panel in ws.app.panels.iter() {
                let waves = panel.kind.waves().unwrap();
                assert_eq!(waves.items().len(), 2);
                assert_eq!(waves.loaded_count(), 2);
            }
            assert!(!ws.app.workspace.scheduler.dirty());
        })
        .unwrap();
}

#[gpui_kit::test]
fn settings_tab_results_json_view_and_palette_render(cx: &mut TestAppContext) {
    use volna_core::app::SettingsCommand;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.app
                .settings_loaded("{\n  \"waves.snapPixels\": 4, // four\n}\n");
            ws.dispatch(Command::Settings(SettingsCommand::Open), Some(window), cx);
        })
        .unwrap();
    cx.run_until_parked();
    let settings = window
        .update(cx, |ws, _, _| {
            ws.app.panels.settings_id().expect("tab open")
        })
        .unwrap();
    for query in [
        "",
        "snap px",
        "@modified",
        "@id:waves",
        "nothing matches this",
    ] {
        window
            .update(cx, |ws, window, cx| {
                ws.dispatch(
                    Command::Settings(SettingsCommand::Query(query.into())),
                    Some(window),
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
    }
    window
        .update(cx, |ws, window, cx| {
            ws.dispatch(
                Command::Settings(SettingsCommand::ToggleJson),
                Some(window),
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, window, cx| {
            let view = ws.settings_view(settings, window, cx);
            assert!(view.read(cx).json.is_some(), "the JSON view was rendered");
            let text = view.read(cx).json.as_ref().unwrap().text(cx);
            assert_eq!(text, ws.app.settings.text());
            ws.dispatch(
                Command::Settings(SettingsCommand::ReplaceText(
                    "{\"waves.snapPixels\": 4,,}".into(),
                )),
                Some(window),
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    let source: Arc<dyn Session> = Arc::new(ProceduralTrace::new(20));
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(source, cx);
            ws.dispatch(
                Command::Settings(SettingsCommand::ToggleJson),
                Some(window),
                cx,
            );
            ws.dispatch(
                Command::Settings(SettingsCommand::Set {
                    id: "panels.linkByDefault".into(),
                    value: volna_core::settings::Value::Bool(false),
                }),
                Some(window),
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, window, cx| {
            assert_eq!(ws.app.panels.settings_id(), Some(settings));
            assert_eq!(ws.app.panels.len(), 2);
            assert!(ws.app.settings.text().contains(",,"));
            ws.open_palette(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, window, cx| {
            ws.dispatch(Command::Settings(SettingsCommand::Close), Some(window), cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, _, _| {
            assert_eq!(ws.app.panels.settings_id(), None);
            assert_eq!(ws.app.panels.len(), 1);
        })
        .unwrap();
}

#[gpui_kit::test]
fn activity_index_settings_menu_disables_and_reenables_automatic_builds(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use gpui_kit::test::TestWindowExt as _;
    use volna_core::settings::ActivityIndexPolicy;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.dispatch(
                Command::Settings(SettingsCommand::Reveal {
                    id: "hierarchy.activityIndex".into(),
                }),
                Some(window),
                cx,
            );
        })
        .unwrap();
    let mut view = VisualTestContext::from_window(root.into(), cx);
    for (label, policy) in [
        ("Disabled", ActivityIndexPolicy::Never),
        ("Build automatically", ActivityIndexPolicy::Always),
    ] {
        view.run_until_parked();
        view.update(|window, cx| {
            window.render_frame(cx);
            window.click("setting-enum-hierarchy.activityIndex", cx);
        });
        view.run_until_parked();
        view.update(|window, cx| {
            window.render_frame(cx);
            let mut menu = window.within("popup-menu");
            let item = (0..3usize)
                .find(|ix| menu.find(*ix).label() == Some(label))
                .expect("activity index policy choice");
            menu.click(item, cx);
        });
        view.run_until_parked();
        window
            .update(&mut view, |ws, _, _| {
                assert_eq!(ws.app.settings.resolved().hierarchy.activity_index, policy);
                let mut restored = volna_core::App::new();
                restored.settings_loaded(ws.app.settings.text());
                assert_eq!(
                    restored.settings.resolved().hierarchy.activity_index,
                    policy
                );
            })
            .unwrap();
    }
}

#[gpui_kit::test]
fn interface_zoom_scales_settings_tab_and_wave_rows_and_keeps_viewer_state(
    cx: &mut TestAppContext,
) {
    use volna_core::app::SettingsCommand;
    use volna_core::panels::PanelsCommand;
    use volna_core::settings::{Value, ZoomStep};
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let source: Arc<dyn Session> = Arc::new(ProceduralTrace::new(100));
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(source.clone(), cx);
            ws.app.handle(Command::AddVars(a_all(vec![0; 400])));
            ws.after(None, cx);
            ws.app.doc.shared.cursor = Some(42);
            ws.app.doc.shared.viewport.value.start = 20.0;
            ws.app.doc.shared.viewport.value.end = 80.0;
            ws.app.doc.add_marker(30);
            let w = ws.app.panels.focused_waves_mut().unwrap();
            w.selected.insert(2);
            w.anchor = Some(2);
            w.names_width = 260.0;
            w.scroll_y = 48.0;
            ws.dispatch(Command::Action(Action::SplitRight), Some(window), cx);
            ws.dispatch(Command::Settings(SettingsCommand::Open), Some(window), cx);
            ws.dispatch(
                Command::Panels(PanelsCommand::FocusIndex(0)),
                Some(window),
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    let (before, settings, waves) = window
        .update(cx, |ws, _, _| {
            let waves = ws.app.panels.focused_id();
            assert!(ws.app.panels.waves(waves).is_some());
            (
                ws.debug_state(),
                ws.app.panels.settings_id().expect("settings tab"),
                waves,
            )
        })
        .unwrap();
    let design_row = crate::theme::CoreTheme::one_dark().row_height;
    let design_font = crate::theme::CoreTheme::one_dark().ui_size;
    for zoom in [0.5f32, 1.0, 2.0] {
        window
            .update(cx, |ws, window, cx| {
                ws.dispatch(
                    Command::Settings(SettingsCommand::Set {
                        id: "appearance.zoom".into(),
                        value: Value::Number(f64::from(zoom)),
                    }),
                    Some(window),
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |ws, window, cx| {
                assert_eq!(ws.app.settings.resolved().appearance.zoom, f64::from(zoom));
                let t = *crate::theme::theme(cx);
                assert_eq!(t.zoom, zoom);
                assert_eq!(t.row_height, design_row * zoom);
                assert_eq!(t.ui_size, design_font * zoom);
                assert_eq!(
                    gpui_kit::component::Theme::global(cx).font_size,
                    px(design_font * zoom)
                );
                assert_eq!(f32::from(window.rem_size()), design_font * zoom);
                assert_eq!(ws.app.panels.settings_id(), Some(settings));
                let view = ws.settings_view(settings, window, cx);
                assert!(view.read(cx).json.is_none());
                let w = ws.app.panels.waves(waves).unwrap();
                let layout = w.last_layout();
                assert_eq!(layout.zoom, zoom, "wave panel rendered at zoom {zoom}");
                assert_eq!(layout.row_h, design_row * zoom);
                assert_eq!(layout.names_split.width(), 8.0 * zoom);
                if zoom <= 1.0 {
                    assert_eq!(layout.names.width(), 260.0 * zoom);
                } else {
                    assert!(layout.names.width() >= 72.0 * zoom);
                }
                assert!(layout.bounds.width() > 0.0 && w.frames_painted > 0);
                assert!(
                    matches!(ws.app.trace_state(), TraceState::Loaded(s) if Arc::ptr_eq(&s, &source))
                );
                assert_eq!(ws.debug_state(), before);
                assert_eq!(w.names_width, 260.0);
                assert!(w.selected.contains(&2));
                assert_eq!(ws.app.doc.markers()[0].time, 30);
                assert_eq!(w.scroll_y, 48.0 * zoom);
            })
            .unwrap();
    }
    window
        .update(cx, |ws, window, cx| {
            ws.dispatch(
                Command::Settings(SettingsCommand::Zoom(ZoomStep::In)),
                Some(window),
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, _, cx| {
            assert_eq!(crate::theme::theme(cx).zoom, 2.1);
            assert!(ws.app.settings.text().contains("\"appearance.zoom\": 2.1"));
            assert_eq!(ws.debug_state(), before);
        })
        .unwrap();
    window
        .update(cx, |ws, window, cx| {
            ws.dispatch(
                Command::Settings(SettingsCommand::Zoom(ZoomStep::Reset)),
                Some(window),
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, _, cx| {
            assert_eq!(crate::theme::theme(cx).zoom, 1.0);
            assert_eq!(crate::theme::theme(cx).row_height, design_row);
            assert!(!ws.app.settings.text().contains("appearance.zoom"));
            assert_eq!(
                ws.app.panels.waves(waves).unwrap().last_layout().row_h,
                design_row
            );
        })
        .unwrap();
}

#[gpui_kit::test]
fn tab_close_buttons_and_middle_click_close_their_own_tab(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, MouseButton, VisualTestContext};
    use volna_core::app::SettingsCommand;
    use volna_core::panels::PanelsCommand;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let (first, second, settings) = window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.app.handle(Command::AddVars(a_all(vec![0; 4])));
            let first = ws.app.panels.focused_id();
            ws.dispatch(
                Command::Panels(PanelsCommand::NewTab { group_of: first }),
                Some(window),
                cx,
            );
            let second = ws.app.panels.focused_id();
            ws.dispatch(Command::Settings(SettingsCommand::Open), Some(window), cx);
            (first, second, ws.app.panels.settings_id().unwrap())
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    let bounds = |vcx: &mut VisualTestContext, selector: String| {
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        vcx.debug_bounds(Box::leak(selector.into_boxed_str()))
    };
    for id in [first, second, settings] {
        assert!(
            bounds(&mut vcx, format!("tab-{}", id.0)).is_some(),
            "tab {id:?}"
        );
    }
    let tab = bounds(&mut vcx, format!("tab-{}", second.0)).unwrap();
    let close = bounds(&mut vcx, format!("tab-close-{}", second.0)).expect("close button");
    assert!(
        close.left() >= tab.center().x,
        "the close button follows the title"
    );
    vcx.simulate_mouse_move(close.center(), None, Modifiers::default());
    vcx.simulate_click(close.center(), Modifiers::default());
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            assert!(ws.app.panels.get(second).is_none());
            assert!(ws.app.panels.get(first).is_some());
            assert_eq!(ws.app.panels.settings_id(), Some(settings));
        })
        .unwrap();
    let tab = bounds(&mut vcx, format!("tab-{}", settings.0)).unwrap();
    vcx.simulate_mouse_move(tab.center(), None, Modifiers::default());
    vcx.simulate_mouse_down(tab.center(), MouseButton::Middle, Modifiers::default());
    vcx.simulate_mouse_up(tab.center(), MouseButton::Middle, Modifiers::default());
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            assert_eq!(ws.app.panels.settings_id(), None);
            assert_eq!(ws.app.panels.len(), 1);
        })
        .unwrap();
}

fn choose_tab_action(
    view: &mut gpui_kit::VisualTestContext,
    panel: volna_core::panels::PanelId,
    label: &str,
    keyboard: bool,
) {
    use gpui_kit::test::TestWindowExt as _;
    view.run_until_parked();
    let at = view.update(|window, cx| {
        window.render_frame(cx);
        let bounds = window.find(("panel-tab", panel.0)).bounds();
        point(bounds.left() + px(2.0), bounds.center().y)
    });
    view.simulate_mouse_move(at, None, Default::default());
    view.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
    view.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
    view.run_until_parked();
    let items = view.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("popup-menu").is_some(),
            "menu did not open for {panel:?}: {label}"
        );
        let menu = window.within("popup-menu");
        (0..32usize)
            .filter_map(|ix| {
                let item = menu.try_find(ix)?;
                Some((ix, item.label()?.to_owned()))
            })
            .collect::<Vec<_>>()
    });
    let position = items
        .iter()
        .position(|(_, text)| text == label)
        .unwrap_or_else(|| panic!("missing {label}: {items:?}"));
    if keyboard {
        view.simulate_mouse_move(point(px(1.0), px(1.0)), None, Default::default());
        view.simulate_keystrokes(&format!("{}enter", "down ".repeat(position + 1)));
    } else {
        view.update(|window, cx| window.within("popup-menu").click(items[position].0, cx));
    }
    view.run_until_parked();
}

fn assert_titlebar_panel(
    view: &mut gpui_kit::VisualTestContext,
    panel: volna_core::panels::PanelId,
    promoted: bool,
) {
    use gpui_kit::test::TestWindowExt as _;
    view.run_until_parked();
    view.update(|window, cx| {
        window.render_frame(cx);
        let header = window.find("auxiliary-titlebar").bounds();
        let tab = window.find(("panel-tab", panel.0)).bounds();
        if promoted {
            let toolbar = window.find("auxiliary-panel-toolbar").bounds();
            assert!(header.contains(&tab.center()));
            assert!(header.contains(&toolbar.center()));
            assert!(toolbar.right() <= header.right());
            assert!(toolbar.left() >= tab.left());
            assert!(window.try_find(("close-tab", panel.0 as usize)).is_none());
        } else {
            assert!(window.try_find("auxiliary-panel-toolbar").is_none());
            assert!(tab.top() >= header.bottom());
        }
    });
}

#[gpui_kit::test]
fn tab_context_actions_target_the_clicked_tab_across_themes_and_windows(cx: &mut TestAppContext) {
    use gpui_kit::{VisualTestContext, size};
    use volna_core::panels::{PanelsCommand, Placement};
    init(cx);
    for (theme, width, zoom) in [
        ("volna-dark", 1000.0, 1.0),
        ("volna-light", 360.0, 1.5),
        ("volna-islands-dark", 1000.0, 1.0),
        ("volna-islands-light", 360.0, 1.5),
    ] {
        let owner = cx.update(|cx| crate::open_main_window(cx, false).unwrap());
        let main = cx.update(|cx| cx.windows()[0]);
        let mut view = VisualTestContext::from_window(main, cx);
        view.simulate_resize(size(px(width), px(850.0)));
        let (a, b) = owner.update(&mut view, |ws, cx| {
            crate::theme::install_themes(
                volna_core::theme::builtin::resolve(theme, true).unwrap(),
                cx,
            );
            crate::theme::set_zoom(zoom, cx);
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.app.sidebar_visible = false;
            ws.dispatch(Command::AddVars(a_all(vec![0, 1])), None, cx);
            let a = ws.app.panels.focused_id();
            ws.dispatch(
                Command::Panels(PanelsCommand::Rename(a, Some("A".into()))),
                None,
                cx,
            );
            ws.dispatch(
                Command::Panels(PanelsCommand::NewTab { group_of: a }),
                None,
                cx,
            );
            let b = ws.app.panels.focused_id();
            ws.dispatch(
                Command::Panels(PanelsCommand::Rename(b, Some("B".into()))),
                None,
                cx,
            );
            (a, b)
        });
        choose_tab_action(&mut view, a, "Rename…", true);
        view.simulate_keystrokes(SELECT_ALL);
        view.simulate_input("Named");
        view.simulate_keystrokes("enter");
        view.run_until_parked();
        owner.read_with(&view, |ws, _| {
            assert_eq!(ws.app.panels.get(a).unwrap().title(), "Named");
            assert_eq!(ws.app.panels.get(b).unwrap().title(), "B");
        });
        choose_tab_action(
            &mut view,
            a,
            if width < 500.0 {
                "Split Down"
            } else {
                "Split Right"
            },
            false,
        );
        owner.read_with(&view, |ws, _| {
            let copy = ws.app.panels.focused_id();
            assert_ne!(copy, a);
            assert_ne!(copy, b);
            assert_eq!(ws.app.panels.waves(copy).unwrap().items().len(), 2);
            ws.app.panels.validate().unwrap();
        });
        owner.update(&mut view, |ws, cx| {
            ws.dispatch(Command::Panels(PanelsCommand::Focus(b)), None, cx);
        });
        view.simulate_resize(size(px(width.max(900.0)), px(850.0)));
        choose_tab_action(&mut view, b, "Move to right sidebar", false);
        owner.read_with(&view, |ws, _| {
            assert_eq!(ws.app.panels.layout().placement(b), Some(Placement::Right));
        });
        choose_tab_action(&mut view, b, "Move to new window", false);
        let auxiliary = owner.read_with(&view, |ws, _| {
            ws.windows.handle_for(b, &ws.app.panels).unwrap()
        });
        let mut detached = VisualTestContext::from_window(auxiliary, &view);
        detached.simulate_resize(size(px(width), px(850.0)));
        assert_titlebar_panel(&mut detached, b, true);
        if width < 500.0 {
            choose_tab_action(&mut detached, b, "New waveform tab", false);
        } else {
            detached.update(|window, cx| {
                use gpui_kit::test::TestWindowExt as _;
                window
                    .within("auxiliary-panel-toolbar")
                    .click("new-tab", cx);
            });
        }
        detached.run_until_parked();
        let extra = owner.update(&mut detached, |ws, cx| {
            let extra = ws.app.panels.focused_id();
            ws.dispatch(
                Command::Panels(PanelsCommand::Rename(extra, Some("C".into()))),
                None,
                cx,
            );
            extra
        });
        assert_ne!(extra, b);
        assert_titlebar_panel(&mut detached, b, false);
        detached.simulate_resize(size(px(width.max(900.0)), px(850.0)));
        choose_tab_action(&mut detached, extra, "Close panel", false);
        detached.simulate_resize(size(px(width), px(850.0)));
        assert_titlebar_panel(&mut detached, b, true);
        choose_tab_action(&mut detached, b, "Split Down", true);
        let extra = owner.read_with(&detached, |ws, _| ws.app.panels.focused_id());
        assert_ne!(extra, b);
        assert_titlebar_panel(&mut detached, b, false);
        choose_tab_action(&mut detached, extra, "Close panel", false);
        assert_titlebar_panel(&mut detached, b, true);
        choose_tab_action(&mut detached, b, "Follow shared cursor", true);
        owner.read_with(&detached, |ws, _| {
            assert!(!ws.app.panels.waves(b).unwrap().nav.link.cursor);
            assert!(ws.app.panels.waves(a).unwrap().nav.link.cursor);
        });
        choose_tab_action(&mut detached, b, "Return to main window", false);
        owner.read_with(&view, |ws, _| {
            assert_eq!(ws.app.panels.layout().placement(b), Some(Placement::Center));
        });
        choose_tab_action(&mut view, b, "Toggle panel zoom", false);
        owner.read_with(&view, |ws, cx| {
            assert!(ws.dock.as_ref().unwrap().read(cx).area.read(cx).is_zoomed());
        });
        choose_tab_action(&mut view, b, "Toggle panel zoom", false);
        owner.read_with(&view, |ws, cx| {
            assert!(!ws.dock.as_ref().unwrap().read(cx).area.read(cx).is_zoomed());
        });
        choose_tab_action(&mut view, b, "New waveform tab", false);
        owner.read_with(&view, |ws, _| assert_ne!(ws.app.panels.focused_id(), b));
        owner.update(&mut view, |ws, cx| {
            ws.dispatch(Command::Panels(PanelsCommand::Focus(b)), None, cx);
        });
        choose_tab_action(&mut view, b, "Close Other Panels", false);
        owner.read_with(&view, |ws, _| {
            assert_eq!(ws.app.panels.len(), 1);
            assert_eq!(ws.app.panels.focused_id(), b);
        });
        view.simulate_resize(size(px(width), px(850.0)));
        choose_tab_action(&mut view, b, "Close panel", false);
        owner.read_with(&view, |ws, _| {
            assert!(ws.app.panels.focused().kind.is_start());
        });
        main.update(&mut view, |_, window, _| window.remove_window())
            .unwrap();
    }
}

#[gpui_kit::test]
fn tab_context_rename_and_close_work_for_every_panel_kind(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::panels::{PanelId, PanelsCommand};
    init(cx);
    let owner = cx.update(|cx| crate::open_main_window(cx, false).unwrap());
    let main = cx.update(|cx| cx.windows()[0]);
    let mut view = VisualTestContext::from_window(main, cx);
    let settings = owner.update(&mut view, |ws, cx| {
        let trace =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/landing.vtr"));
        ws.set_session(OpenSpec::Path(trace.into()).open().unwrap(), cx);
        let saved: volna_core::workspace::Workspace =
            serde_json::from_str(include_str!("../examples/landing-page.volna.json")).unwrap();
        saved
            .prepare(
                &ws.app,
                &crate::native_workspace::file_uri(trace).unwrap(),
                &crate::native_workspace::file_uri(std::path::Path::new(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/examples/landing-page.volna.json"
                )))
                .unwrap(),
            )
            .unwrap()
            .commit(&mut ws.app)
            .unwrap();
        ws.dispatch(Command::Settings(SettingsCommand::Open), None, cx);
        ws.app.panels.settings_id().unwrap()
    });
    for panel in [PanelId(37), PanelId(38), PanelId(39), PanelId(40), settings] {
        owner.update(&mut view, |ws, cx| {
            ws.dispatch(Command::Panels(PanelsCommand::Focus(panel)), None, cx);
        });
        choose_tab_action(&mut view, panel, "Rename…", false);
        view.simulate_keystrokes(SELECT_ALL);
        view.simulate_input("Custom");
        view.simulate_keystrokes("enter");
        view.run_until_parked();
        owner.read_with(&view, |ws, _| {
            assert_eq!(ws.app.panels.get(panel).unwrap().title(), "Custom");
        });
    }
    let pinned = owner.read_with(&view, |ws, _| {
        ws.app.panels.transaction(PanelId(37)).unwrap().pinned()
    });
    choose_tab_action(&mut view, PanelId(37), "Pin record", false);
    owner.read_with(&view, |ws, _| {
        assert_eq!(
            ws.app.panels.transaction(PanelId(37)).unwrap().pinned(),
            !pinned
        );
    });
    choose_tab_action(&mut view, settings, "Close panel", true);
    owner.read_with(&view, |ws, _| {
        assert!(ws.app.panels.settings_id().is_none());
    });
    owner.update(&mut view, |ws, cx| {
        ws.dispatch(Command::Panels(PanelsCommand::Focus(PanelId(37))), None, cx);
    });
    choose_tab_action(&mut view, PanelId(37), "Close tab", false);
    owner.read_with(&view, |ws, _| {
        assert!(ws.app.panels.get(PanelId(37)).is_none());
    });
    let settings = owner.update(&mut view, |ws, cx| {
        ws.dispatch(Command::Settings(SettingsCommand::Open), None, cx);
        ws.app.panels.settings_id().unwrap()
    });
    choose_tab_action(&mut view, settings, "Close Other Panels", false);
    let start = owner.read_with(&view, |ws, _| ws.app.panels.first_start().unwrap());
    choose_tab_action(&mut view, start, "Rename…", false);
    view.simulate_keystrokes(SELECT_ALL);
    view.simulate_input("Welcome");
    view.simulate_keystrokes("enter");
    view.run_until_parked();
    owner.read_with(&view, |ws, _| {
        assert_eq!(ws.app.panels.get(start).unwrap().title(), "Welcome");
    });
    choose_tab_action(&mut view, start, "New waveform tab", true);
    owner.read_with(&view, |ws, _| {
        assert!(ws.app.panels.first_start().is_none());
        assert!(ws.app.panels.focused().kind.waves().is_some());
    });
    main.update(&mut view, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn signal_menu_height_submenu_and_row_height_actions(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0, 1, 2])), Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let heights = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| {
                ws.app
                    .panels
                    .focused_waves()
                    .unwrap()
                    .items()
                    .iter()
                    .map(|item| item.height().multiple())
                    .collect::<Vec<_>>()
            })
            .unwrap()
    };
    vcx.simulate_keystrokes("shift-f10");
    vcx.run_until_parked();
    assert!(
        window
            .update(&mut vcx, |ws, _, _| ws.wave_menu.is_some())
            .unwrap(),
        "the popup is hosted"
    );
    vcx.simulate_keystrokes("down down down down down down right down enter");
    vcx.run_until_parked();
    assert_eq!(heights(&mut vcx), [2, 2, 2]);
    window
        .update(&mut vcx, |ws, _, _| {
            assert!(ws.app.panels.focused_waves().unwrap().menu.is_none());
            assert!(ws.wave_menu.is_none());
        })
        .unwrap();
    vcx.update(|window, cx| {
        window.dispatch_action(Box::new(IncreaseRowHeight), cx);
    });
    vcx.run_until_parked();
    assert_eq!(heights(&mut vcx), [3, 3, 3]);
    vcx.update(|window, cx| {
        window.dispatch_action(Box::new(DecreaseRowHeight), cx);
        window.dispatch_action(Box::new(DecreaseRowHeight), cx);
    });
    vcx.run_until_parked();
    assert_eq!(heights(&mut vcx), [1, 1, 1]);
    vcx.update(|window, cx| {
        window.dispatch_action(Box::new(IncreaseRowHeight), cx);
        window.dispatch_action(Box::new(ResetRowHeight), cx);
    });
    vcx.run_until_parked();
    assert_eq!(heights(&mut vcx), [1, 1, 1]);
}

#[gpui_kit::test]
fn wave_copy_paste_keys_duplicate_rows(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0, 1, 2])), Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let names = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| {
                ws.app
                    .panels
                    .focused_waves()
                    .unwrap()
                    .items()
                    .iter()
                    .map(|item| item.name().to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap()
    };
    let original = names(&mut vcx);
    window
        .update(&mut vcx, |ws, _, _| {
            ws.app.panels.focused_waves_mut().unwrap().selected = [0].into();
        })
        .unwrap();
    let (copy, cut, paste) = if cfg!(target_os = "macos") {
        ("cmd-c", "cmd-x", "cmd-v")
    } else {
        ("ctrl-c", "ctrl-x", "ctrl-v")
    };
    vcx.simulate_keystrokes(copy);
    vcx.simulate_keystrokes(paste);
    vcx.simulate_keystrokes(paste);
    vcx.run_until_parked();
    let clk = original[0].as_str();
    assert_eq!(
        names(&mut vcx),
        [clk, clk, clk, original[1].as_str(), original[2].as_str()]
    );
    window
        .update(&mut vcx, |ws, _, _| {
            let rows = &ws.app.panels.focused_waves().unwrap().items();
            assert!(rows[..3].iter().all(|row| Arc::ptr_eq(
                row.signal().unwrap().history.as_ref().unwrap(),
                rows[0].signal().unwrap().history.as_ref().unwrap()
            )));
        })
        .unwrap();
    vcx.simulate_keystrokes(cut);
    vcx.run_until_parked();
    assert_eq!(
        names(&mut vcx),
        [clk, clk, original[1].as_str(), original[2].as_str()]
    );
}

#[gpui_kit::test]
fn status_bar_memory_meter_tracks_the_budget(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    let bounds = |vcx: &mut VisualTestContext, selector: &'static str| {
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        vcx.debug_bounds(selector)
    };
    assert!(bounds(&mut vcx, "status-memory").is_none(), "no trace");
    let trace = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr");
    window
        .update(&mut vcx, |ws, window, cx| {
            let session = OpenSpec::Path(trace.into()).open().unwrap();
            ws.set_session(session, cx);
            ws.dispatch(Command::AddVars(a_all(vec![0, 1, 2, 3])), Some(window), cx);
        })
        .unwrap();
    vcx.run_until_parked();
    let memory = window
        .update(&mut vcx, |ws, _, _| ws.app.status().memory.unwrap())
        .unwrap();
    assert!(memory.used > 0);
    let pill = bounds(&mut vcx, "status-memory").unwrap();
    assert_eq!(f32::from(pill.size.width), MEMORY_PILL_W);
    let label = bounds(&mut vcx, "status-memory-label").unwrap();
    assert!(
        label.left() > pill.left() && label.right() < pill.right(),
        "{label:?} inside {pill:?}"
    );
    assert!(
        f32::from((label.center().x - pill.center().x).abs()) < 1.0,
        "centred"
    );
    let fill = bounds(&mut vcx, "status-memory-fill").unwrap();
    let expected = (memory.fraction() * MEMORY_PILL_W).max(3.0);
    assert!(
        (f32::from(fill.size.width) - expected).abs() < 0.5,
        "{fill:?} for {memory:?}"
    );

    use gpui_kit::Modifiers;
    vcx.simulate_click(pill.center(), Modifiers::default());
    vcx.run_until_parked();
    vcx.simulate_mouse_move(
        gpui_kit::point(gpui_kit::px(1.0), gpui_kit::px(1.0)),
        None,
        Modifiers::default(),
    );
    assert!(
        window
            .update(&mut vcx, |ws, _, _| ws.status_menu.is_some())
            .unwrap()
    );
    vcx.simulate_keystrokes("down left down down enter");
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            assert_eq!(ws.app.settings.resolved().memory.budget_mib, 1024);
            assert_eq!(ws.app.status().memory.unwrap().limit, 1024 * 1024 * 1024);
            assert!(ws.status_menu.is_none());
        })
        .unwrap();
    let pill = bounds(&mut vcx, "status-memory").unwrap();
    let label = window
        .update(&mut vcx, |ws, _, _| ws.app.status().memory.unwrap().label())
        .unwrap();
    assert!(label.ends_with("/ 1 GiB"), "{label}");

    vcx.simulate_click(pill.center(), Modifiers::default());
    vcx.run_until_parked();
    vcx.simulate_keystrokes("up enter");
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            assert!(ws.app.panels.settings_id().is_some());
            assert_eq!(ws.app.settings_view.query, "@id:memory");
        })
        .unwrap();
}

#[gpui_kit::test]
fn status_bar_groups_stay_put_under_long_messages(cx: &mut TestAppContext) {
    use gpui_kit::{VisualTestContext, px, size};
    init(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    let bounds = |vcx: &mut VisualTestContext, selector: &'static str| {
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        vcx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector}"))
    };
    let trace = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr");
    window
        .update(&mut vcx, |ws, window, cx| {
            let session = OpenSpec::Path(trace.into()).open().unwrap();
            ws.set_session(session, cx);
            ws.dispatch(Command::AddVars(a_all(vec![0, 1, 2, 3])), Some(window), cx);
        })
        .unwrap();
    let set_message = |vcx: &mut VisualTestContext, text: Option<String>| {
        window
            .update(vcx, |ws, _, cx| {
                ws.app.variables.notice = text;
                cx.notify();
            })
            .unwrap();
    };

    vcx.simulate_resize(size(px(1200.0), px(700.0)));
    let meter = bounds(&mut vcx, "status-memory");
    let right = bounds(&mut vcx, "status-right");
    let long = "#123 · X cycles 4–5 (1 cycle) ".repeat(40);
    set_message(&mut vcx, Some(long.clone()));
    assert_eq!(bounds(&mut vcx, "status-memory"), meter, "meter moved");
    assert_eq!(bounds(&mut vcx, "status-right"), right, "right group moved");
    let slot = bounds(&mut vcx, "status-message");
    let left = bounds(&mut vcx, "status-left");
    assert!(slot.left() >= left.right(), "{slot:?} after {left:?}");
    assert!(slot.right() <= right.left(), "{slot:?} before {right:?}");
    let bar = slot.size.height;
    assert!(f32::from(bar) <= 24.0, "one line: {slot:?}");
    let notice = bounds(&mut vcx, "sidebar-notice");
    vcx.simulate_click(notice.center(), gpui_kit::Modifiers::default());
    vcx.run_until_parked();
    vcx.update(|window, cx| {
        use gpui_kit::test::TestWindowExt as _;
        window.render_frame(cx);
        window.click("copy-message", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), long);
        window.click("close-message", cx);
    });

    vcx.simulate_resize(size(px(560.0), px(700.0)));
    let right = bounds(&mut vcx, "status-right");
    let meter = bounds(&mut vcx, "status-memory");
    assert!(
        f32::from(right.right()) <= 560.0,
        "{right:?} inside the window"
    );
    assert!(meter.left() >= right.left() && meter.right() <= right.right());
    let left = bounds(&mut vcx, "status-left");
    assert!(
        left.right() <= right.left(),
        "{left:?} clipped before {right:?}"
    );
    set_message(&mut vcx, None);
    assert_eq!(
        bounds(&mut vcx, "status-right"),
        right,
        "message-free width"
    );
}

struct RootWindow {
    root: gpui_kit::WindowHandle<gpui_kit::component::Root>,
    workspace: gpui_kit::Entity<Workspace>,
}

#[gpui_kit::test]
fn pipeline_box_zoom_captures_outside_release_and_ignores_double_click_open(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{
        Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, VisualTestContext,
    };
    use volna_core::pipeline::{FollowActivity, RowView};
    use volna_core::wave::viewport::Viewport;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, _, cx| {
            ws.open_path(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/examples/pipeline_showcase.vtr"
                )
                .into(),
                cx,
            );
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let id = window
        .update(&mut vcx, |ws, window, cx| {
            let track = ws.app.pipeline_generators()[0].1;
            ws.dispatch(Command::OpenPipeline { track }, Some(window), cx);
            let id = ws.app.panels.focused_id();
            ws.app.doc.navigation.animation = volna_core::settings::Animation::Off;
            ws.app.doc.shared.viewport.set(Viewport {
                start: 200.0,
                end: 400.0,
            });
            let p = ws.app.panels.pipeline_mut(id).unwrap();
            p.follow = FollowActivity::Off;
            p.rows.set(RowView {
                top: 200.0,
                row_px: 6.0,
            });
            id
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let layout =
        window
            .update(&mut vcx, |ws, _, _| {
                assert!(ws.app.panels.pipeline_mut(id).unwrap().select_row(
                    &mut ws.app.doc,
                    id,
                    210
                ));
                ws.app.panels.pipeline(id).unwrap().last_layout().clone()
            })
            .unwrap();
    let cells = layout.cells;
    let at = |x, y| gpui_kit::point(gpui_kit::px(x), gpui_kit::px(y));
    let start = at(
        cells.left() + cells.width() * 0.5,
        cells.top() + cells.height() * 0.5,
    );
    let outside = at(
        layout.bounds.left() - 30.0,
        cells.top() + cells.height() * 0.75,
    );
    let modifiers = Modifiers {
        control: true,
        ..Default::default()
    };
    vcx.simulate_mouse_move(start, None, modifiers);
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: start,
        modifiers,
        click_count: 2,
        first_mouse: false,
    });
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            assert!(matches!(
                ws.app.panels.pipeline(id).unwrap().drag,
                Some(volna_core::pipeline::model::Drag::BoxZoom { .. })
            ));
            assert_eq!(ws.app.panels.focused_id(), id);
            assert_eq!(
                ws.app.panels.iter().count(),
                2,
                "a modified second press opens no additional transaction tab"
            );
        })
        .unwrap();
    vcx.simulate_event(MouseMoveEvent {
        position: outside,
        pressed_button: Some(MouseButton::Left),
        modifiers,
    });
    vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: outside,
        modifiers,
        click_count: 2,
    });
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            let p = ws.app.panels.pipeline(id).unwrap();
            assert!(p.drag.is_none());
            assert!((p.nav.viewport(&ws.app.doc).width() - 100.0).abs() < 1e-5);
            assert!((p.rows.value.row_px - 12.0).abs() < 1e-5);
        })
        .unwrap();
}

#[gpui_kit::test]
fn pipeline_label_double_click_pans_and_cell_double_click_opens_details(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, VisualTestContext};
    use volna_core::pipeline::FollowActivity;
    use volna_core::wave::viewport::Viewport;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, _, cx| {
            ws.open_path(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/examples/pipeline_showcase.vtr"
                )
                .into(),
                cx,
            );
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let panel = window
        .update(&mut vcx, |ws, window, cx| {
            let track = ws.app.pipeline_generators()[0].1;
            ws.dispatch(Command::OpenPipeline { track }, Some(window), cx);
            let panel = ws.app.panels.focused_id();
            ws.app.doc.navigation.animation = volna_core::settings::Animation::Off;
            ws.app.panels.pipeline_mut(panel).unwrap().follow = FollowActivity::Off;
            panel
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let (row, begin, label, cell) = window
        .update(&mut vcx, |ws, _, _| {
            let pipeline = ws.app.panels.pipeline(panel).unwrap();
            let layout = pipeline.last_layout();
            let row = (layout.row_range.start + 10).min(layout.row_range.end - 1);
            let volna_core::pipeline::Rows::Ready(set) = pipeline.rows(&ws.app.doc) else {
                panic!("loaded pipeline")
            };
            let begin = set.get(row).unwrap().1.begin;
            let y = layout.row_y(row) + layout.rows.row_px * 0.5;
            (
                row,
                begin,
                (layout.labels.left() + 20.0, y),
                (layout.cells.left() + 40.0, y),
            )
        })
        .unwrap();
    let initial = Viewport {
        start: 200.0,
        end: 400.0,
    };
    window
        .update(&mut vcx, |ws, _, _| {
            ws.app
                .panels
                .pipeline_mut(panel)
                .unwrap()
                .nav
                .viewport_state_mut(&mut ws.app.doc)
                .set(initial);
        })
        .unwrap();
    vcx.run_until_parked();
    let at = |(x, y)| gpui_kit::point(gpui_kit::px(x), gpui_kit::px(y));
    let modifiers = Modifiers::default();
    vcx.simulate_mouse_move(at(label), None, modifiers);
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at(label),
        modifiers,
        click_count: 2,
        first_mouse: false,
    });
    vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: at(label),
        modifiers,
        click_count: 2,
    });
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            let pipeline = ws.app.panels.pipeline(panel).unwrap();
            let viewport = pipeline.nav.viewport(&ws.app.doc);
            assert_eq!(pipeline.selected_row(&ws.app.doc), Some(row));
            assert_eq!(ws.app.panels.iter().count(), 2);
            assert!((viewport.width() - initial.width()).abs() < 1e-6);
            assert!(
                (viewport.x_of_tick(begin, pipeline.last_layout().cells_width_f64()) - 32.0).abs()
                    < 1e-5
            );
        })
        .unwrap();
    vcx.simulate_mouse_move(at(cell), None, modifiers);
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at(cell),
        modifiers,
        click_count: 2,
        first_mouse: false,
    });
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            assert_eq!(ws.app.panels.iter().count(), 2);
            assert!(ws.app.panels.focused().kind.transaction().is_some());
        })
        .unwrap();
}

impl RootWindow {
    fn update<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Workspace, &mut gpui_kit::Window, &mut gpui_kit::Context<Workspace>) -> R,
    ) -> anyhow::Result<R> {
        use gpui_kit::AppContext as _;
        let workspace = self.workspace.clone();
        cx.update_window(self.root.into(), move |_, window, cx| {
            workspace.update(cx, |ws, cx| f(ws, window, cx))
        })
    }
}

#[gpui_kit::test]
fn analog_key_and_format_popup_sections(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::geometry::{Modifiers, MouseButton, point};
    use volna_core::wave::PointerEvent;
    use volna_trace::data::SignalShape;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            let session = Arc::new(ProceduralTrace::new(100));
            ws.set_session(session.clone(), cx);
            let vector = session
                .hierarchy()
                .vars()
                .position(|v| matches!(v.shape, SignalShape::Vector { .. }))
                .unwrap();
            ws.dispatch(Command::AddVars(a_all(vec![vector])), Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let row = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| {
                let item = ws.app.panels.focused_waves().unwrap().items()[0].clone();
                (
                    item.signal().unwrap().analog.as_ref().map(|a| a.draw),
                    item.height().multiple(),
                )
            })
            .unwrap()
    };
    window
        .update(&mut vcx, |ws, _, _| {
            ws.app.panels.focused_waves_mut().unwrap().selected = [0].into();
        })
        .unwrap();
    vcx.simulate_keystrokes("a");
    vcx.run_until_parked();
    assert_eq!(
        row(&mut vcx),
        (Some(volna_core::wave::analog::AnalogDraw::Step), 3)
    );
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    window
        .update(&mut vcx, |ws, window, cx| {
            let panel = ws.app.panels.focused_id();
            let badge = ws.app.panels.focused_waves().unwrap().last_layout().badges[0].1;
            ws.dispatch(
                Command::Pointer(
                    panel,
                    PointerEvent::Down {
                        position: point(badge.left() + 2.0, badge.top() + 2.0),
                        button: MouseButton::Left,
                        modifiers: Modifiers::default(),
                    },
                ),
                Some(window),
                cx,
            );
        })
        .unwrap();
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            let menu = ws
                .app
                .panels
                .focused_waves()
                .unwrap()
                .menu
                .as_ref()
                .unwrap();
            assert!(
                menu.entries
                    .contains(&volna_core::wave::MenuEntry::Label("Range".into()))
            );
            assert!(ws.wave_menu.is_some(), "the popup is hosted");
        })
        .unwrap();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(
        window
            .update(&mut vcx, |ws, _, _| ws.wave_menu.is_none())
            .unwrap()
    );
    vcx.simulate_keystrokes("a");
    vcx.run_until_parked();
    assert_eq!(row(&mut vcx), (None, 1));
}

#[gpui_kit::test]
fn color_submenu_and_palette_commands_colour_the_selection(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::wave::{MenuEntry, Tint};
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0, 1, 2])), Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    window
        .update(&mut vcx, |ws, window, cx| {
            ws.app.panels.focused_waves_mut().unwrap().selected = [0, 1].into();
            let panel = ws.app.panels.focused_id();
            ws.dispatch(Command::OpenSignalMenu(panel), Some(window), cx);
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    window
        .update(&mut vcx, |ws, _, _| {
            assert!(ws.wave_menu.is_some(), "the popup is hosted");
            let menu = ws.app.panels.focused_waves().unwrap().menu.as_ref().unwrap();
            assert!(menu.entries.iter().any(
                |e| matches!(e, MenuEntry::Submenu { label, items } if label == "Color" && items.len() == 6)
            ));
        })
        .unwrap();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    let labels = window
        .update(&mut vcx, |ws, _, _| {
            crate::palette::commands(&ws.app, "color")
                .into_iter()
                .map(|(label, _)| label)
                .filter(|l| l.starts_with("Color: "))
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(
        labels,
        [
            "Color: Default",
            "Color: Blue",
            "Color: Cyan",
            "Color: Violet",
            "Color: Pink",
            "Color: Grey"
        ]
    );
    vcx.update(|window, cx| {
        window.dispatch_action(
            Box::new(SetTint {
                tint: Some(Tint::Pink),
            }),
            cx,
        );
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    window
        .update(&mut vcx, |ws, _, _| {
            let w = ws.app.panels.focused_waves().unwrap();
            assert_eq!(
                (0..3).map(|i| w.ink(i)).collect::<Vec<_>>(),
                [Some(Tint::Pink), Some(Tint::Pink), None]
            );
            assert_eq!(ws.app.undo_label(), Some("Color 2 signals Pink"));
        })
        .unwrap();
}

#[gpui_kit::test]
fn shift_a_and_the_group_menu_stack_and_unstack_a_group(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::wave::model::MenuAction;
    use volna_core::wave::{GroupStyle, MenuEntry};
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0, 1, 2])), Some(window), cx);
            let w = ws.app.panels.focused_waves_mut().unwrap();
            w.selected = [0, 1, 2].into();
            w.anchor = Some(0);
            ws.dispatch(Command::Action(Action::GroupSelection), Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    let style = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| {
                let g = ws.app.panels.focused_waves().unwrap().items()[0]
                    .group()
                    .unwrap()
                    .clone();
                (g.style, g.height.multiple())
            })
            .unwrap()
    };
    assert_eq!(style(&mut vcx), (GroupStyle::Activity, 1));
    vcx.simulate_keystrokes("shift-a");
    vcx.run_until_parked();
    assert_eq!(style(&mut vcx), (GroupStyle::Stack { peak: true }, 3));
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    window
        .update(&mut vcx, |ws, window, cx| {
            assert_eq!(ws.app.undo_label(), Some("Stack Group 1"));
            let panel = ws.app.panels.focused_id();
            ws.dispatch(Command::OpenSignalMenu(panel), Some(window), cx);
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    window
        .update(&mut vcx, |ws, window, cx| {
            assert!(ws.wave_menu.is_some(), "the popup is hosted");
            let menu = ws.app.panels.focused_waves().unwrap().menu.clone().unwrap();
            let stacked = menu
                .items()
                .find(|i| i.action == MenuAction::Stack(true))
                .unwrap();
            assert_eq!(
                (stacked.label.as_str(), stacked.checked),
                ("Stacked area", true)
            );
            assert!(
                menu.entries
                    .iter()
                    .any(|e| matches!(e, MenuEntry::Label(l) if l == "Draw"))
            );
            let peak = menu.items().find(|i| i.label == "Peak of total").unwrap();
            assert_eq!(
                (&peak.action, peak.checked),
                (&MenuAction::Peak(false), true)
            );
            let panel = ws.app.panels.focused_id();
            ws.dispatch(
                Command::MenuSelect(panel, MenuAction::Stack(false)),
                Some(window),
                cx,
            );
        })
        .unwrap();
    vcx.run_until_parked();
    assert_eq!(style(&mut vcx), (GroupStyle::Activity, 1));
    let labels = window
        .update(&mut vcx, |ws, _, _| {
            crate::palette::commands(&ws.app, "stacked")
                .into_iter()
                .map(|(label, _)| label)
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert!(
        labels.iter().any(|l| l == "Toggle Stacked Area"),
        "{labels:?}"
    );
}

#[gpui_kit::test]
fn bus_keys_and_the_hosted_slice_editor(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::wave::model::MenuAction;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![4])), Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    vcx.simulate_keystrokes("right");
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            let w = ws.app.panels.focused_waves().unwrap();
            assert_eq!(w.items().len(), 9);
            assert_eq!(w.items()[1].name(), "counter[7]");
            assert!(
                w.items()
                    .iter()
                    .filter_map(|e| e.signal())
                    .all(|s| s.history.is_some())
            );
        })
        .unwrap();
    window
        .update(&mut vcx, |ws, window, cx| {
            let panel = ws.app.panels.focused_id();
            ws.app.handle(Command::OpenSignalMenu(panel));
            ws.dispatch(
                Command::MenuSelect(panel, MenuAction::AddSlice),
                Some(window),
                cx,
            );
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    window
        .update(&mut vcx, |ws, window, cx| {
            let hosted = ws.rename.as_ref().expect("the slice editor is hosted");
            assert!(hosted.input.read(cx).focus_handle(cx).is_focused(window));
            assert!(matches!(
                hosted.target,
                volna_core::app::EditTarget::Slice { .. }
            ));
        })
        .unwrap();
    vcx.simulate_input("3:0");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            assert!(ws.rename.is_none());
            let w = ws.app.panels.focused_waves().unwrap();
            assert_eq!(w.items().last().unwrap().name(), "counter[3:0]");
            assert!(
                w.items()
                    .last()
                    .unwrap()
                    .signal()
                    .unwrap()
                    .history
                    .is_some()
            );
        })
        .unwrap();
    vcx.simulate_keystrokes("left");
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            let w = ws.app.panels.focused_waves().unwrap();
            assert_eq!(w.items().last().unwrap().name(), "counter[3:0]");
            assert!(w.items().last().unwrap().collapsed().unwrap());
        })
        .unwrap();
}

#[gpui_kit::test]
fn group_keys_and_the_hosted_name_editor(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0, 1, 2, 3])), Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let outline = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| {
                ws.app
                    .panels
                    .focused_waves()
                    .unwrap()
                    .items()
                    .iter()
                    .map(|e| {
                        let mark = match e.group() {
                            Some(g) if g.collapsed => "+",
                            Some(_) => "-",
                            None => "",
                        };
                        format!("{}{mark}{}", "  ".repeat(usize::from(e.depth)), e.name())
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap()
    };
    let names: Vec<String> = outline(&mut vcx);
    window
        .update(&mut vcx, |ws, _, _| {
            let w = ws.app.panels.focused_waves_mut().unwrap();
            w.selected = [1, 2].into();
            w.anchor = Some(1);
        })
        .unwrap();
    vcx.simulate_keystrokes("g");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let editing = window
        .update(&mut vcx, |ws, window, cx| {
            let hosted = ws.rename.as_ref().expect("the name editor is hosted");
            let focused = hosted.input.read(cx).focus_handle(cx).is_focused(window);
            (hosted.input.read(cx).text().to_owned(), focused)
        })
        .unwrap();
    assert_eq!(editing, ("Group 1".to_owned(), true));
    assert!(
        window
            .update(&mut vcx, |ws, _, cx| ws
                .rename
                .as_ref()
                .unwrap()
                .input
                .read(cx)
                .accent_color())
            .unwrap()
            .is_none(),
        "a group's name field keeps the input's outline"
    );
    vcx.simulate_keystrokes("a x i enter");
    vcx.run_until_parked();
    assert_eq!(
        outline(&mut vcx),
        [
            names[0].clone(),
            "-axi".into(),
            format!("  {}", names[1]),
            format!("  {}", names[2]),
            names[3].clone()
        ]
    );
    assert!(
        window
            .update(&mut vcx, |ws, _, _| ws.rename.is_none())
            .unwrap()
    );
    vcx.simulate_keystrokes("f2");
    vcx.run_until_parked();
    assert!(
        window
            .update(&mut vcx, |ws, _, _| ws.rename.is_some())
            .unwrap()
    );
    vcx.simulate_keystrokes("z escape");
    vcx.run_until_parked();
    assert!(
        window
            .update(&mut vcx, |ws, _, _| ws.rename.is_none())
            .unwrap()
    );
    assert_eq!(outline(&mut vcx)[1], "-axi");
    vcx.simulate_keystrokes("f2 right 2 enter");
    vcx.run_until_parked();
    assert_eq!(outline(&mut vcx)[1], "-axi2");
    vcx.simulate_keystrokes("f2 enter");
    vcx.run_until_parked();
    assert_eq!(outline(&mut vcx)[1], "-axi2");
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let at = window
        .update(&mut vcx, |ws, _, _| {
            let w = ws.app.panels.focused_waves().unwrap();
            let l = w.last_layout();
            let (y, _) = l.entry_span(1).unwrap();
            let on_name: Vec<_> = (0..400)
                .map(|dx| volna_core::geometry::point(l.names.left() + dx as f32, y + 4.0))
                .filter(|&p| w.group_name_at(p) == Some(1))
                .collect();
            on_name[on_name.len() / 2]
        })
        .unwrap();
    let at = gpui_kit::point(gpui_kit::px(at.x), gpui_kit::px(at.y));
    vcx.simulate_mouse_move(at, None, gpui_kit::Modifiers::default());
    for click_count in [1, 2] {
        vcx.simulate_event(gpui_kit::MouseDownEvent {
            button: gpui_kit::MouseButton::Left,
            position: at,
            modifiers: gpui_kit::Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        vcx.simulate_event(gpui_kit::MouseUpEvent {
            button: gpui_kit::MouseButton::Left,
            position: at,
            modifiers: gpui_kit::Modifiers::default(),
            click_count,
        });
    }
    vcx.run_until_parked();
    vcx.simulate_keystrokes("b u s enter");
    vcx.run_until_parked();
    assert_eq!(outline(&mut vcx)[1], "-bus");
    vcx.simulate_keystrokes("alt-left");
    vcx.run_until_parked();
    assert_eq!(outline(&mut vcx)[1], "+bus");
    vcx.simulate_keystrokes("alt-right");
    vcx.run_until_parked();
    assert_eq!(outline(&mut vcx)[1], "-bus");
    vcx.simulate_keystrokes("shift-g");
    vcx.run_until_parked();
    assert_eq!(outline(&mut vcx), names);
}

#[test]
fn wave_rows_are_tree_items() {
    use gpui_kit::accesskit::Role;
    use volna_core::geometry::Rect;
    use volna_core::wave::model::AccessibleRow;
    let row = AccessibleRow {
        entry: 3,
        label: "Write, 7 rows".into(),
        level: 2,
        expanded: Some(false),
        selected: true,
        bounds: Rect::from_xywh(0.0, 48.0, 220.0, 24.0),
    };
    let node = crate::canvas::wave_row_node(&row, 2.0);
    assert_eq!(node.role(), Role::TreeItem);
    assert_eq!(node.level(), Some(2));
    assert_eq!(node.is_expanded(), Some(false));
    assert_eq!(node.is_selected(), Some(true));
    assert_eq!(node.label(), Some("Write, 7 rows"));
    assert_eq!(node.bounds().map(|b| b.y1), Some(144.0));
    let leaf = crate::canvas::wave_row_node(
        &AccessibleRow {
            expanded: None,
            selected: false,
            ..row
        },
        1.0,
    );
    assert_eq!(leaf.is_expanded(), None);
}

#[gpui_kit::test]
fn undo_keys_reach_the_history_and_text_fields_keep_their_own(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0, 1, 2, 3])), Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let rows = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| {
                ws.app
                    .panels
                    .focused_waves()
                    .unwrap()
                    .items()
                    .iter()
                    .map(|e| e.name().to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap()
    };
    let (undo, redo) = if cfg!(target_os = "macos") {
        ("cmd-z", "cmd-shift-z")
    } else {
        ("ctrl-z", "ctrl-shift-z")
    };
    let all = rows(&mut vcx);
    window
        .update(&mut vcx, |ws, _, _| {
            let w = ws.app.panels.focused_waves_mut().unwrap();
            w.selected = [1].into();
            w.anchor = Some(1);
        })
        .unwrap();
    vcx.simulate_keystrokes("delete");
    vcx.run_until_parked();
    assert_eq!(rows(&mut vcx).len(), 3);

    let edit_menu = |vcx: &mut VisualTestContext| {
        vcx.update(|_, cx| {
            let menus = cx.get_menus().expect("the application menu");
            let edit = menus
                .iter()
                .find(|m| m.name == "Edit")
                .expect("an Edit menu");
            edit.items
                .iter()
                .filter_map(|item| match item {
                    gpui_kit::OwnedMenuItem::Action { name, disabled, .. } => {
                        Some((name.clone(), *disabled))
                    }
                    _ => None,
                })
                .take(2)
                .collect::<Vec<_>>()
        })
    };
    let removed = window
        .update(&mut vcx, |ws, _, _| ws.app.undo_label().unwrap().to_owned())
        .unwrap();
    assert_eq!(
        edit_menu(&mut vcx),
        [(format!("Undo {removed}"), false), ("Redo".into(), true)]
    );
    let labels = window
        .update(&mut vcx, |ws, _, _| {
            crate::palette::commands(&ws.app, "")
                .into_iter()
                .map(|(label, _)| label)
                .take(2)
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(labels, [format!("Undo: {removed}"), "Redo".to_owned()]);

    vcx.simulate_keystrokes(undo);
    vcx.run_until_parked();
    assert_eq!(rows(&mut vcx), all);
    assert_eq!(
        edit_menu(&mut vcx),
        [
            ("Undo Add 4 signals".into(), false),
            (format!("Redo {removed}"), false)
        ]
    );
    vcx.simulate_keystrokes(redo);
    vcx.run_until_parked();
    assert_eq!(rows(&mut vcx).len(), 3);
    vcx.simulate_keystrokes(undo);
    vcx.run_until_parked();
    if !cfg!(target_os = "macos") {
        vcx.simulate_keystrokes("ctrl-y");
        vcx.run_until_parked();
        assert_eq!(rows(&mut vcx).len(), 3, "Ctrl+Y redoes");
        vcx.simulate_keystrokes(undo);
        vcx.run_until_parked();
    }
    assert_eq!(rows(&mut vcx), all);

    window
        .update(&mut vcx, |ws, _, _| {
            let w = ws.app.panels.focused_waves_mut().unwrap();
            w.selected = [1, 2].into();
            w.anchor = Some(1);
        })
        .unwrap();
    vcx.simulate_keystrokes("g");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let steps = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| ws.app.history.undo_steps().count())
            .unwrap()
    };
    let grouped = steps(&mut vcx);
    let name = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, cx| {
                ws.rename
                    .as_ref()
                    .map(|r| r.input.read(cx).text().to_owned())
            })
            .unwrap()
    };
    vcx.simulate_keystrokes("a x");
    vcx.run_until_parked();
    assert_eq!(name(&mut vcx).as_deref(), Some("ax"));
    vcx.simulate_keystrokes(undo);
    vcx.run_until_parked();
    assert_eq!(
        name(&mut vcx).as_deref(),
        Some("a"),
        "typing undoes as a run"
    );
    vcx.simulate_keystrokes(undo);
    vcx.run_until_parked();
    assert_eq!(name(&mut vcx).as_deref(), Some("Group 1"));
    vcx.simulate_keystrokes(redo);
    vcx.run_until_parked();
    assert_eq!(name(&mut vcx).as_deref(), Some("a"));
    assert_eq!(steps(&mut vcx), grouped, "the cockpit history is untouched");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(steps(&mut vcx), grouped, "naming joins the group's step");
    assert_eq!(rows(&mut vcx)[1], "a");

    window
        .update(&mut vcx, |ws, window, cx| {
            let handle = ws.filter.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        })
        .unwrap();
    vcx.simulate_keystrokes("c l k");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("backspace");
    vcx.run_until_parked();
    let filter = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| ws.app.variables.filter.clone())
            .unwrap()
    };
    assert_eq!(filter(&mut vcx), "cl");
    vcx.simulate_keystrokes(undo);
    vcx.run_until_parked();
    assert_eq!(filter(&mut vcx), "clk");
    assert_eq!(rows(&mut vcx)[1], "a");
    window
        .update(&mut vcx, |ws, window, cx| {
            window.focus(&ws.waves_focus, cx);
        })
        .unwrap();
    vcx.simulate_keystrokes(undo);
    vcx.run_until_parked();
    assert_eq!(rows(&mut vcx), all);
}

#[gpui_kit::test]
fn marker_walk_keys_move_the_cursor(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let (near, far) = window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0])), Some(window), cx);
            let (lo, hi) = ws.app.doc.limits();
            let (near, far) = (lo + (hi - lo) / 4, lo + (hi - lo) * 3 / 4);
            for t in [near, far] {
                ws.app.doc.shared.cursor = Some(t);
                ws.dispatch(Command::Action(Action::AddOrRenameMarker), Some(window), cx);
            }
            ws.app.doc.shared.cursor = Some(lo);
            (near, far)
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let state = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| {
                (ws.app.doc.shared.cursor, ws.app.status().announcement)
            })
            .unwrap()
    };
    vcx.simulate_keystrokes(".");
    assert_eq!(
        state(&mut vcx),
        (Some(near), Some("At marker 1 · ` returns".into()))
    );
    vcx.simulate_keystrokes("2");
    assert_eq!(state(&mut vcx).0, Some(far));
    vcx.simulate_keystrokes(",");
    assert_eq!(state(&mut vcx).0, Some(near));
    vcx.simulate_keystrokes("`");
    assert_eq!(state(&mut vcx).0, Some(far));
    vcx.simulate_keystrokes("9");
    assert_eq!(state(&mut vcx), (Some(far), Some("No marker 9".into())));
}

#[gpui_kit::test]
fn ctrl_digits_place_and_move_markers_while_bare_digits_jump(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::panels::{Axis, PanelsCommand};
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let (a, b, start, step) = window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0])), Some(window), cx);
            let (lo, hi) = ws.app.doc.limits();
            let a = ws.app.panels.focused_id();
            ws.dispatch(
                Command::Panels(PanelsCommand::Split {
                    panel: a,
                    axis: Axis::Vertical,
                }),
                Some(window),
                cx,
            );
            let b = ws.app.panels.focused_id();
            ws.dispatch(Command::Panels(PanelsCommand::Focus(a)), Some(window), cx);
            (a, b, lo, (hi - lo) / 10)
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    for n in (1..=6).rev() {
        let time = start + step * u64::from(n);
        window
            .update(&mut vcx, |ws, _, _| ws.app.doc.shared.cursor = Some(time))
            .unwrap();
        vcx.simulate_keystrokes(&format!("ctrl-{n}"));
        window
            .update(&mut vcx, |ws, _, _| {
                let marker = ws
                    .app
                    .doc
                    .markers()
                    .iter()
                    .find(|m| m.id.get() == n)
                    .unwrap();
                assert_eq!(marker.time, time);
                assert_eq!(ws.app.panels.focused_id(), a);
            })
            .unwrap();
    }
    let moved = start + step * 8;
    window
        .update(&mut vcx, |ws, _, _| ws.app.doc.shared.cursor = Some(moved))
        .unwrap();
    vcx.simulate_keystrokes("ctrl-6");
    window
        .update(&mut vcx, |ws, _, _| {
            assert_eq!(ws.app.doc.markers().len(), 6);
            assert_eq!(ws.app.undo_label(), Some("Move marker 6"));
            ws.app.doc.shared.cursor = Some(start);
        })
        .unwrap();
    for n in 1..=6 {
        vcx.simulate_keystrokes(&n.to_string());
        window
            .update(&mut vcx, |ws, _, _| {
                assert_eq!(
                    ws.app.doc.shared.cursor,
                    Some(if n == 6 { moved } else { start + step * n })
                );
                assert_eq!(ws.app.panels.focused_id(), a);
            })
            .unwrap();
    }
    vcx.simulate_keystrokes("ctrl-alt-2");
    window
        .update(&mut vcx, |ws, _, _| {
            assert_eq!(ws.app.panels.focused_id(), b);
        })
        .unwrap();
}

#[gpui_kit::test]
fn marker_name_field_opens_over_the_chip_and_keeps_its_keys(cx: &mut TestAppContext) {
    use gpui_kit::{
        Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, VisualTestContext, point,
    };
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let (near, far) = window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0])), Some(window), cx);
            let (lo, hi) = ws.app.doc.limits();
            let (near, far) = (lo + (hi - lo) / 4, lo + (hi - lo) * 3 / 4);
            for t in [near, far] {
                ws.app.doc.shared.cursor = Some(t);
                ws.dispatch(Command::Action(Action::AddOrRenameMarker), Some(window), cx);
            }
            ws.app.doc.shared.cursor = Some(near);
            (near, far)
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    let draw = |vcx: &mut VisualTestContext| {
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
    };
    draw(&mut vcx);
    let names = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| {
                let m = ws.app.doc.markers();
                (
                    m[0].label.clone(),
                    m[1].label.clone(),
                    ws.app.doc.shared.cursor,
                )
            })
            .unwrap()
    };

    vcx.simulate_keystrokes("m");
    draw(&mut vcx);
    let field = vcx
        .debug_bounds("marker-name")
        .expect("the name field is laid over the chip");
    let chip = window
        .update(&mut vcx, |ws, _, _| {
            ws.app.panels.focused().kind.marker_lane().unwrap().chips[0].rect
        })
        .unwrap();
    assert!(
        f32::from(field.origin.x) > chip.left() && f32::from(field.origin.y) >= chip.top() - 4.0
    );
    let (accent, chip) = window
        .update(&mut vcx, |ws, _, cx| {
            let chip = crate::theme::core_theme(cx)
                .marker(0)
                .background
                .with_alpha(1.0);
            (
                ws.rename.as_ref().unwrap().input.read(cx).accent_color(),
                crate::theme::hsla(chip),
            )
        })
        .unwrap();
    assert_eq!(accent, Some(chip));
    vcx.simulate_keystrokes("r e q space 2 . enter");
    draw(&mut vcx);
    assert_eq!(names(&mut vcx), (Some("req 2.".into()), None, Some(near)));
    assert!(
        vcx.debug_bounds("marker-name").is_none(),
        "Enter closes the field"
    );

    let chip = window
        .update(&mut vcx, |ws, _, _| {
            ws.app.panels.focused().kind.marker_lane().unwrap().chips[1].rect
        })
        .unwrap();
    let at = point(
        gpui_kit::px(chip.left() + 3.0),
        gpui_kit::px(chip.top() + 3.0),
    );
    for click_count in [1, 2] {
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count,
        });
    }
    draw(&mut vcx);
    assert!(vcx.debug_bounds("marker-name").is_some());
    let focused = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, window, cx| {
                ws.rename
                    .as_ref()
                    .is_some_and(|h| h.input.read(cx).focus_handle(cx).is_focused(window))
            })
            .unwrap()
    };
    assert!(
        focused(&mut vcx),
        "the field has the keys after a double-click"
    );
    vcx.simulate_keystrokes("x escape");
    draw(&mut vcx);
    assert_eq!(names(&mut vcx), (Some("req 2.".into()), None, Some(far)));
    assert!(vcx.debug_bounds("marker-name").is_none());
    for click_count in [1, 2] {
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count,
        });
    }
    draw(&mut vcx);
    vcx.simulate_keystrokes("e n d enter");
    draw(&mut vcx);
    assert_eq!(
        names(&mut vcx),
        (Some("req 2.".into()), Some("end".into()), Some(far))
    );
}

#[gpui_kit::test]
fn double_click_on_a_span_zooms_to_it(cx: &mut TestAppContext) {
    use gpui_kit::{
        Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, VisualTestContext, point,
    };
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let (a, b) = window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0])), Some(window), cx);
            let (lo, hi) = ws.app.doc.limits();
            let (a, b) = (lo + (hi - lo) * 2 / 5, lo + (hi - lo) / 2);
            for t in [a, b] {
                ws.app.doc.shared.cursor = Some(t);
                ws.dispatch(Command::Action(Action::AddOrRenameMarker), Some(window), cx);
            }
            (a, b)
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let (x, y) = window
        .update(&mut vcx, |ws, _, _| {
            let lane = ws.app.panels.focused().kind.marker_lane().unwrap().clone();
            let span = lane.spans.iter().find(|s| s.first == 0).expect("a span");
            let (x0, x1) = lane.shown(span);
            (
                f32::midpoint(x0, x1),
                lane.band.top() + lane.band.height() / 2.0,
            )
        })
        .unwrap();
    let at = point(gpui_kit::px(x), gpui_kit::px(y));
    for click_count in [1, 2] {
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count,
        });
    }
    vcx.run_until_parked();
    let v = window
        .update(&mut vcx, |ws, _, _| ws.app.doc.shared.viewport.target())
        .unwrap();
    let (a, b) = (a as f64, b as f64);
    assert!(v.start < a && v.end > b, "{v:?}");
    assert!(v.width() < (b - a) * 1.5, "zoomed in to the span: {v:?}");
}

#[gpui_kit::test]
fn reference_keys_and_a_double_click_on_the_live_span(cx: &mut TestAppContext) {
    use gpui_kit::{
        Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, VisualTestContext, point,
    };
    use volna_core::marker::Reference;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let (a, b) = window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0])), Some(window), cx);
            let (lo, hi) = ws.app.doc.limits();
            let (a, b) = (lo + (hi - lo) * 2 / 5, lo + (hi - lo) / 2);
            ws.app.doc.shared.cursor = Some(a);
            (a, b)
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let reference = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| ws.app.doc.reference())
            .unwrap()
    };
    vcx.simulate_keystrokes("r");
    assert_eq!(reference(&mut vcx), Some(Reference::Time(a)));
    vcx.simulate_keystrokes("shift-r");
    assert_eq!(reference(&mut vcx), None);
    vcx.simulate_keystrokes("r");
    window
        .update(&mut vcx, |ws, _, _| ws.app.doc.shared.cursor = Some(b))
        .unwrap();
    vcx.simulate_keystrokes("z");
    let v = window
        .update(&mut vcx, |ws, _, _| ws.app.doc.shared.viewport.target())
        .unwrap();
    let (fa, fb) = (a as f64, b as f64);
    assert!(
        v.start < fa && v.end > fb && v.width() < (fb - fa) * 1.5,
        "{v:?}"
    );

    vcx.simulate_keystrokes("f");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let (x, y) = window
        .update(&mut vcx, |ws, _, _| {
            let lane = ws.app.panels.focused().kind.marker_lane().unwrap().clone();
            let m = lane.measure.clone().expect("the Measure lane");
            let (x0, x1) = lane.live_shown(m.live.as_ref().expect("the live span"));
            (f32::midpoint(x0, x1), m.band.top() + m.band.height() / 2.0)
        })
        .unwrap();
    let at = point(gpui_kit::px(x), gpui_kit::px(y));
    for click_count in [1, 2] {
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count,
        });
    }
    vcx.run_until_parked();
    let (v, cursor) = window
        .update(&mut vcx, |ws, _, _| {
            (
                ws.app.doc.shared.viewport.target(),
                ws.app.doc.shared.cursor,
            )
        })
        .unwrap();
    assert_eq!(cursor, Some(b), "a press on the live span keeps the cursor");
    assert!(
        v.start < fa && v.end > fb && v.width() < (fb - fa) * 1.5,
        "{v:?}"
    );
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let clear = window
        .update(&mut vcx, |ws, _, _| {
            let lane = ws.app.panels.focused().kind.marker_lane().unwrap().clone();
            lane.measure.expect("the Measure lane").clear
        })
        .unwrap();
    let at = point(
        gpui_kit::px(clear.left() + clear.width() / 2.0),
        gpui_kit::px(clear.top() + clear.height() / 2.0),
    );
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    vcx.run_until_parked();
    assert_eq!(reference(&mut vcx), None);
}

#[gpui_kit::test]
fn marker_chips_drag_open_menus_copy_and_double_click_adds(cx: &mut TestAppContext) {
    use gpui_kit::{
        Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, VisualTestContext,
        point,
    };
    use volna_core::marker::LaneVerb;
    use volna_core::wave::model::MenuAction;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let a = window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0])), Some(window), cx);
            let (lo, hi) = ws.app.doc.limits();
            let a = lo + (hi - lo) / 4;
            ws.app.doc.shared.cursor = Some(a);
            ws.dispatch(Command::Action(Action::AddOrRenameMarker), Some(window), cx);
            a
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let chip = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| {
                let lane = ws.app.panels.focused().kind.marker_lane().unwrap().clone();
                let r = lane.chips[0].rect;
                (r.left() + 3.0, r.top() + r.height() / 2.0)
            })
            .unwrap()
    };
    let (x, y) = chip(&mut vcx);
    let at = |x: f32, y: f32| point(gpui_kit::px(x), gpui_kit::px(y));
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at(x, y),
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    for dx in [30.0, 90.0] {
        vcx.simulate_event(MouseMoveEvent {
            position: at(x + dx, y),
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        });
    }
    vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: at(x + 90.0, y),
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    vcx.run_until_parked();
    let (moved, label) = window
        .update(&mut vcx, |ws, _, _| {
            (
                ws.app.doc.markers()[0].time,
                ws.app.undo_label().map(str::to_owned),
            )
        })
        .unwrap();
    assert!(moved > a, "{moved} > {a}");
    assert_eq!(label.as_deref(), Some("Move marker 1"));

    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let (x, y) = chip(&mut vcx);
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Right,
        position: at(x, y),
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Right,
        position: at(x, y),
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, window, cx| {
            assert!(ws.wave_menu.is_some(), "the lane menu is hosted");
            let panel = ws.app.panels.focused_id();
            let id = ws.app.doc.markers()[0].id;
            ws.dispatch(
                Command::MenuSelect(panel, MenuAction::Lane(LaneVerb::Copy(id))),
                Some(window),
                cx,
            );
        })
        .unwrap();
    vcx.run_until_parked();
    let copied = vcx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert!(copied.contains(" marker 1 at "), "{copied}");

    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let (hx, hy) = window
        .update(&mut vcx, |ws, _, _| {
            let l = ws.app.panels.focused_waves().unwrap().last_layout();
            (l.waves.left() + l.waves.width() * 0.8, l.header.top() + 6.0)
        })
        .unwrap();
    for click_count in [1, 2] {
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at(hx, hy),
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at(hx, hy),
            modifiers: Modifiers::default(),
            click_count,
        });
    }
    vcx.run_until_parked();
    let (count, cursor, last) = window
        .update(&mut vcx, |ws, _, _| {
            (
                ws.app.doc.markers().len(),
                ws.app.doc.shared.cursor,
                ws.app.doc.markers().last().map(|m| m.time),
            )
        })
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(last, cursor);
}

#[gpui_kit::test]
fn the_marker_navigator_lists_filters_and_acts_on_markers(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::marker::Reference;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let (start, times) = window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0])), Some(window), cx);
            let (lo, hi) = ws.app.doc.limits();
            let times = [
                lo + (hi - lo) / 5,
                lo + (hi - lo) * 2 / 5,
                lo + (hi - lo) * 3 / 5,
            ];
            for (t, name) in times.iter().zip(["req A", "resp A", "req B"]) {
                ws.app.doc.shared.cursor = Some(*t);
                ws.dispatch(Command::Action(Action::AddOrRenameMarker), Some(window), cx);
                let id = ws
                    .app
                    .doc
                    .markers()
                    .iter()
                    .find(|m| m.time == *t)
                    .unwrap()
                    .id;
                ws.app.doc.rename_marker(id, name);
            }
            ws.app.doc.shared.cursor = Some(lo);
            (lo, times)
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let shown = |vcx: &mut VisualTestContext| {
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        [
            vcx.debug_bounds("marker-row-1").is_some(),
            vcx.debug_bounds("marker-row-2").is_some(),
            vcx.debug_bounds("marker-row-3").is_some(),
        ]
    };
    assert_eq!(shown(&mut vcx), [false; 3]);
    vcx.simulate_keystrokes("'");
    vcx.run_until_parked();
    assert_eq!(shown(&mut vcx), [true; 3], "every marker is listed");
    let rows = |vcx: &mut VisualTestContext| {
        vcx.debug_bounds("marker-row-1")
            .zip(vcx.debug_bounds("marker-row-3"))
    };
    let (one, three) = rows(&mut vcx).unwrap();
    assert!(one.top() < three.top(), "in time order");

    vcx.simulate_input(" req bx");
    vcx.run_until_parked();
    assert_eq!(shown(&mut vcx), [false; 3]);
    vcx.simulate_keystrokes("left delete");
    vcx.run_until_parked();
    assert_eq!(shown(&mut vcx), [false, false, true]);
    vcx.simulate_keystrokes("shift-enter");
    vcx.run_until_parked();
    let (reference, cursor) = window
        .update(&mut vcx, |ws, _, _| {
            (ws.app.doc.reference(), ws.app.doc.shared.cursor)
        })
        .unwrap();
    assert_eq!(
        reference,
        Some(Reference::Marker(
            volna_core::marker::MarkerId::new(3).unwrap()
        ))
    );
    assert_eq!(cursor, Some(start), "no jump");
    assert_eq!(shown(&mut vcx), [false; 3], "the palette closed");

    vcx.simulate_keystrokes("'");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("delete");
    vcx.run_until_parked();
    assert_eq!(shown(&mut vcx), [false, true, true]);
    let label = window
        .update(&mut vcx, |ws, _, _| ws.app.undo_label().map(str::to_owned))
        .unwrap();
    assert_eq!(label.as_deref(), Some("Remove marker 1"));

    vcx.simulate_keystrokes("f2");
    vcx.run_until_parked();
    let edit = window
        .update(&mut vcx, |ws, _, _| ws.app.text_edit().map(|e| e.text))
        .unwrap();
    assert_eq!(edit.as_deref(), Some("resp A"));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    vcx.simulate_keystrokes("'");
    vcx.run_until_parked();
    vcx.simulate_input("3");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let cursor = window
        .update(&mut vcx, |ws, _, _| ws.app.doc.shared.cursor)
        .unwrap();
    assert_eq!(cursor, Some(times[2]));
}

#[gpui_kit::test]
fn the_palette_offers_markers_when_the_query_asks_for_them(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0])), Some(window), cx);
            let (lo, hi) = ws.app.doc.limits();
            for (k, name) in [(1, "req A"), (2, "resp A"), (3, "req B")] {
                let t = lo + (hi - lo) * k / 5;
                ws.app.doc.shared.cursor = Some(t);
                ws.dispatch(Command::Action(Action::AddOrRenameMarker), Some(window), cx);
                let id = ws
                    .app
                    .doc
                    .markers()
                    .iter()
                    .find(|m| m.time == t)
                    .unwrap()
                    .id;
                ws.app.doc.rename_marker(id, name);
            }
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    window
        .update(&mut vcx, super::Workspace::open_palette)
        .unwrap();
    vcx.run_until_parked();
    let shown = |vcx: &mut VisualTestContext| {
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        [
            vcx.debug_bounds("marker-row-1").is_some(),
            vcx.debug_bounds("marker-row-2").is_some(),
            vcx.debug_bounds("marker-row-3").is_some(),
        ]
    };
    assert_eq!(
        shown(&mut vcx),
        [false; 3],
        "an empty query lists no markers"
    );
    vcx.simulate_input("mar");
    vcx.run_until_parked();
    assert_eq!(shown(&mut vcx), [true; 3]);
    vcx.simulate_input(" req");
    vcx.run_until_parked();
    assert_eq!(
        shown(&mut vcx),
        [true, false, true],
        "the other words filter"
    );

    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    vcx.simulate_input("find marker");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(shown(&mut vcx), [true; 3]);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(shown(&mut vcx), [false; 3]);
    let at = window
        .update(&mut vcx, |ws, _, _| {
            (ws.app.doc.shared.cursor, ws.app.doc.markers()[0].time)
        })
        .unwrap();
    assert_eq!(at.0, Some(at.1));
}

#[gpui_kit::test]
fn palette_commands_chosen_by_keyboard_reach_the_panel(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0])), Some(window), cx);
            let (lo, hi) = ws.app.doc.limits();
            for t in [lo + (hi - lo) / 4, lo + (hi - lo) / 2] {
                ws.app.doc.shared.cursor = Some(t);
                ws.dispatch(Command::Action(Action::AddOrRenameMarker), Some(window), cx);
            }
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let markers = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| ws.app.doc.markers().len())
            .unwrap()
    };
    assert_eq!(markers(&mut vcx), 2);
    window
        .update(&mut vcx, super::Workspace::open_palette)
        .unwrap();
    vcx.run_until_parked();
    vcx.simulate_input("remove all markers");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(markers(&mut vcx), 0, "Remove All Markers ran");

    vcx.simulate_keystrokes("m");
    vcx.run_until_parked();
    assert_eq!(markers(&mut vcx), 1);
    window
        .update(&mut vcx, super::Workspace::open_palette)
        .unwrap();
    vcx.run_until_parked();
    vcx.simulate_input("add or name marker");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    vcx.simulate_input("irq");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let name = window
        .update(&mut vcx, |ws, _, _| ws.app.doc.markers()[0].label.clone())
        .unwrap();
    assert_eq!(name.as_deref(), Some("irq"));
}

#[gpui_kit::test]
fn start_page_reopens_recent_traces_and_workspaces(cx: &mut TestAppContext) {
    use crate::native_workspace::{Store, file_uri};
    use gpui_kit::VisualTestContext;
    use volna_core::workspace::persistence::{Persistence, Target};
    use volna_core::workspace::recent::RecentKind;
    init(cx);
    let temporary = tempfile::tempdir().unwrap();
    let example = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr");
    let (a, b) = (
        temporary.path().join("a.vtr"),
        temporary.path().join("b.vtr"),
    );
    std::fs::copy(example, &a).unwrap();
    std::fs::copy(example, &b).unwrap();
    let config = temporary.path().join("config");
    let mut store = Store::new(Some(config.clone())).unwrap();
    store.data_dir = temporary.path().join("fallback");
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    let draw = |vcx: &mut VisualTestContext| {
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
    };
    let names = |vcx: &mut VisualTestContext| {
        window
            .update(vcx, |ws, _, _| {
                ws.app
                    .recent_rows(0, None)
                    .into_iter()
                    .map(|r| (r.name, r.missing))
                    .collect::<Vec<_>>()
            })
            .unwrap()
    };
    window
        .update(&mut vcx, |ws, _, cx| {
            ws.enable_native_persistence(store, Persistence::Auto, false, cx);
            ws.open_path(a.clone(), cx);
        })
        .unwrap();
    draw(&mut vcx);
    window
        .update(&mut vcx, |ws, _, cx| ws.open_path(b.clone(), cx))
        .unwrap();
    draw(&mut vcx);
    std::fs::remove_file(&a).unwrap();
    window
        .update(&mut vcx, |ws, window, cx| {
            assert!(ws.app.doc.is_loaded());
            ws.dispatch(Command::CloseTrace, Some(window), cx);
        })
        .unwrap();
    draw(&mut vcx);
    assert_eq!(
        names(&mut vcx),
        [("b.vtr".to_owned(), false), ("a.vtr".to_owned(), true)]
    );
    let saved = std::fs::read_to_string(config.join("state.json")).unwrap();
    assert!(saved.find("b.vtr").unwrap() < saved.find("a.vtr").unwrap());
    assert!(vcx.debug_bounds("recent-0").is_some());
    assert!(vcx.debug_bounds("recent-1").is_some());
    assert!(vcx.debug_bounds("recent-2").is_none());
    let labels = window
        .update(&mut vcx, |ws, _, _| {
            crate::palette::commands(&ws.app, "")
                .into_iter()
                .map(|(label, _)| label)
                .filter(|label| label.contains("Recent"))
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(labels.len(), 3, "{labels:?}");
    assert!(labels[0].starts_with("Open Recent: b.vtr — "), "{labels:?}");

    vcx.simulate_keystrokes("down enter");
    draw(&mut vcx);
    window
        .update(&mut vcx, |ws, _, _| {
            assert!(matches!(ws.app.trace_state(), TraceState::Error(_)));
        })
        .unwrap();
    assert_eq!(names(&mut vcx).len(), 2);

    vcx.simulate_keystrokes("home enter");
    draw(&mut vcx);
    let workspace_file = temporary.path().join("debug.volna.json");
    window
        .update(&mut vcx, |ws, _, cx| {
            assert!(ws.app.doc.is_loaded());
            assert!(
                ws.app
                    .workspace
                    .trace_uri
                    .as_deref()
                    .unwrap()
                    .ends_with("/b.vtr")
            );
            ws.app.save_workspace(Some(Target::File {
                uri: file_uri(&workspace_file).unwrap(),
            }));
            ws.after(None, cx);
        })
        .unwrap();
    draw(&mut vcx);
    assert!(workspace_file.exists());

    window
        .update(&mut vcx, |ws, window, cx| {
            ws.dispatch(Command::CloseTrace, Some(window), cx);
            ws.open_workspace_path(workspace_file.clone(), cx);
        })
        .unwrap();
    draw(&mut vcx);
    window
        .update(&mut vcx, |ws, window, cx| {
            assert!(ws.app.doc.is_loaded());
            let first = &ws.app.workspace.state.recent[0];
            assert_eq!(first.kind, RecentKind::Workspace);
            assert!(first.trace.as_deref().unwrap().ends_with("/b.vtr"));
            assert_eq!(ws.app.workspace.state.recent.len(), 3);
            ws.dispatch(Command::CloseTrace, Some(window), cx);
        })
        .unwrap();
    draw(&mut vcx);
    assert_eq!(names(&mut vcx)[0].0, "debug");

    vcx.simulate_keystrokes("3");
    draw(&mut vcx);
    assert!(
        window
            .update(&mut vcx, |ws, _, _| matches!(
                ws.app.trace_state(),
                TraceState::Error(_)
            ))
            .unwrap()
    );
    vcx.simulate_keystrokes("end delete");
    draw(&mut vcx);
    assert_eq!(
        names(&mut vcx)
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>(),
        ["debug", "b.vtr"]
    );
    let saved = std::fs::read_to_string(config.join("state.json")).unwrap();
    assert!(!saved.contains("a.vtr"));
}

#[gpui_kit::test]
fn trace_chips_and_rows_show_every_open_trace(cx: &mut TestAppContext) {
    use gpui_kit::Modifiers;
    use gpui_kit::VisualTestContext;
    use volna_core::sidebar::TreeNode;
    init(cx);
    let temporary = tempfile::tempdir().unwrap();
    let example = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/landing.vtr");
    let (a, b) = (
        temporary.path().join("cpu.vtr"),
        temporary.path().join("dram.vtr"),
    );
    std::fs::copy(example, &a).unwrap();
    std::fs::copy(example, &b).unwrap();
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, _, cx| {
            ws.app.handle(Command::Settings(SettingsCommand::Set {
                id: "hierarchy.activityIndex".into(),
                value: volna_core::settings::Value::Text("never".into()),
            }));
            ws.open_paths(vec![a.clone(), b.clone()], cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    let draw = |vcx: &mut VisualTestContext| {
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
    };
    draw(&mut vcx);
    let b_id = TraceId::from_letter('B').unwrap();
    window
        .update(&mut vcx, |ws, _, _| {
            let chips = ws.app.trace_chips();
            assert_eq!(
                chips.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
                ["cpu.vtr", "dram.vtr"]
            );
            assert!(chips.iter().all(|c| !c.loading));
        })
        .unwrap();
    assert!(vcx.debug_bounds("trace-chip-A").is_some());
    let chip = vcx.debug_bounds("trace-chip-B").expect("B's chip");
    assert!(vcx.debug_bounds("trace-row-A").is_some());
    assert!(vcx.debug_bounds("trace-row-B").is_some());
    vcx.simulate_click(chip.center(), Modifiers::default());
    draw(&mut vcx);
    window
        .update(&mut vcx, |ws, window, cx| {
            assert_eq!(ws.app.scopes.selected, Some(TreeNode::trace(b_id)));
            let track = ws
                .app
                .pipeline_generators()
                .into_iter()
                .find(|(_, t)| t.trace == b_id)
                .expect("B records a pipeline")
                .1;
            ws.dispatch(Command::OpenPipeline { track }, Some(window), cx);
            assert_eq!(ws.app.panels.focused().kind.trace(), Some(b_id));
        })
        .unwrap();
    draw(&mut vcx);
    window
        .update(&mut vcx, |ws, _, _| {
            let labels: Vec<String> = crate::palette::commands(&ws.app, "")
                .into_iter()
                .map(|(label, _)| label)
                .filter(|label| label.contains("Trace"))
                .collect();
            for label in [
                "Add Trace…",
                "Reveal Trace A cpu.vtr",
                "Rename Trace B dram.vtr…",
                "Close Trace B dram.vtr",
            ] {
                assert!(labels.iter().any(|l| l == label), "{label}: {labels:?}");
            }
            assert!(!labels.iter().any(|l| l == "Close Trace A cpu.vtr"));
        })
        .unwrap();
    window
        .update(&mut vcx, |ws, window, cx| {
            ws.dispatch(Command::RemoveTrace(b_id), Some(window), cx);
            assert_eq!(ws.app.trace_chips().len(), 1);
        })
        .unwrap();
    draw(&mut vcx);
    assert!(vcx.debug_bounds("trace-chip-A").is_none());
    assert!(vcx.debug_bounds("trace-row-A").is_none());
}

#[gpui_kit::test]
fn volna_mixed_is_a_light_window_with_dark_waves_whatever_the_system(cx: &mut TestAppContext) {
    init(cx);
    let window = cx.add_window(Workspace::new);
    let trace = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr");
    window
        .update(cx, |ws, _, cx| ws.open_path(trace.into(), cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |ws, window, cx| {
            ws.dispatch(Command::AddVars(a_all(0..1)), Some(window), cx);
        })
        .unwrap();
    cx.run_until_parked();
    let dark = volna_core::Theme::volna(true);
    let light = volna_core::Theme::volna(false);
    let workspace = window.root(cx).unwrap();
    let (waves, generation) = workspace.read_with(cx, |ws, _| {
        let waves = ws.app.panels.focused_id();
        assert!(ws.app.panels.waves(waves).is_some());
        (waves, ws.app.doc.generation())
    });
    let panel_theme = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            crate::canvas::PanelCanvas::new(workspace.clone(), waves, generation).core_theme(cx)
        })
    };
    let set = |cx: &mut TestAppContext, theme: Option<&str>, dark: bool| {
        workspace.update(cx, |ws, cx| {
            if let Some(theme) = theme {
                ws.app
                    .settings_loaded(&format!("{{\"appearance.theme\": \"{theme}\"}}"));
                ws.apply_theme_setting(cx);
            }
            ws.set_system_dark(dark, cx);
        });
    };
    let chrome = |cx: &mut TestAppContext| cx.update(|cx| *crate::theme::core_theme(cx));
    set(cx, Some("volna-mixed"), false);
    assert_eq!(chrome(cx).editor.bg, light.editor.bg);
    cx.update(|cx| {
        assert_eq!(
            crate::theme::theme(cx).panel.bg,
            crate::theme::hsla(light.panel.bg)
        );
        assert_eq!(crate::theme::canvas_theme(cx).editor.bg, dark.editor.bg);
    });
    assert_eq!(panel_theme(cx).wave_signal, dark.wave_signal);
    set(cx, None, true);
    assert_eq!(chrome(cx).editor.bg, light.editor.bg);
    assert_eq!(panel_theme(cx).editor.bg, dark.editor.bg);
    set(cx, Some("volna"), true);
    assert_eq!(chrome(cx).editor.bg, dark.editor.bg);
    set(cx, None, false);
    assert_eq!(panel_theme(cx).editor.bg, light.editor.bg);
    assert_eq!(chrome(cx).editor.bg, light.editor.bg);
}

#[gpui_kit::test]
fn settings_search_takes_wave_keys_while_a_trace_is_open(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::app::SettingsCommand;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.app.settings_loaded("{}\n");
            ws.dispatch(Command::Settings(SettingsCommand::Open), Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, window, cx| {
            ws.dispatch(Command::Settings(SettingsCommand::Open), Some(window), cx);
        })
        .unwrap();
    vcx.run_until_parked();
    let focused = window
        .update(&mut vcx, |ws, window, cx| {
            let id = ws.app.panels.settings_id().unwrap();
            let view = ws.settings_view(id, window, cx);
            view.read(cx).search_focus(cx).is_focused(window)
        })
        .unwrap();
    assert!(focused, "opening Settings focuses its search");
    vcx.simulate_keystrokes("m f s");
    vcx.run_until_parked();
    let (query, markers) = window
        .update(&mut vcx, |ws, _, _| {
            (
                ws.app.settings_view.query.clone(),
                ws.app.doc.markers().len(),
            )
        })
        .unwrap();
    assert_eq!(query, "mfs");
    assert_eq!(markers, 0, "`m` did not drop a marker");
}

#[gpui_kit::test]
fn activity_build_banner_drives_build_dismiss_and_cancel(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, VisualTestContext};
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let trace = dir.path().join("run.vtr");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr"),
        &trace,
    )
    .unwrap();
    let window = cx.add_window(Workspace::new);
    window
        .update(cx, |ws, _, cx| {
            ws.app
                .settings_loaded(r#"{"hierarchy.activityIndex":"ask"}"#);
            ws.open_path(trace.clone(), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let offer = vcx.debug_bounds("activity-banner-A").expect("build offer");
    let build = vcx.debug_bounds("activity-build-A").expect("Build button");
    let dismiss = vcx
        .debug_bounds("activity-dismiss-A")
        .expect("Not now button");
    assert!(offer.contains(&build.center()) && offer.contains(&dismiss.center()));
    vcx.simulate_click(dismiss.center(), Modifiers::default());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("activity-banner-A").is_none());
    assert!(!trace.with_extension("vtr.index").exists());
    window
        .update(&mut vcx, |ws, _, cx| ws.open_path(trace.clone(), cx))
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let build = vcx
        .debug_bounds("activity-build-A")
        .expect("Build after reopen");
    vcx.simulate_click(build.center(), Modifiers::default());
    vcx.run_until_parked();
    for _ in 0..3 {
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        vcx.run_until_parked();
    }
    assert!(trace.with_extension("vtr.index").exists());
    assert!(vcx.debug_bounds("activity-banner-A").is_none());
    window
        .update(&mut vcx, |ws, _, _| {
            assert!(ws.app.doc.session(TraceId::A).unwrap().activity().is_some());
        })
        .unwrap();
    let other = dir.path().join("other.vtr");
    std::fs::copy(&trace, &other).unwrap();
    window
        .update(&mut vcx, |ws, _, cx| ws.open_path(other.clone(), cx))
        .unwrap();
    vcx.run_until_parked();
    let request = window
        .update(&mut vcx, |ws, _, _| {
            ws.app.handle(Command::BuildActivity(TraceId::A));
            ws.app.take_requests().pop().expect("build request")
        })
        .unwrap();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("activity-progress-A").is_some());
    let cancel = vcx
        .debug_bounds("activity-cancel-A")
        .expect("Cancel button");
    vcx.simulate_click(cancel.center(), Modifiers::default());
    let result = request.perform();
    window
        .update(&mut vcx, |ws, _, cx| {
            ws.app.deliver(result);
            ws.after(None, cx);
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("activity-banner-A").is_none());
    assert!(!other.with_extension("vtr.index").exists());
    let mut paths = Vec::new();
    for i in 0..4 {
        let path = dir.path().join(format!("trace-{i}.vtr"));
        std::fs::copy(&trace, &path).unwrap();
        paths.push(path);
    }
    window
        .update(&mut vcx, |ws, _, cx| ws.open_paths(paths, cx))
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let panel = vcx.debug_bounds("scopes-panel").expect("scope panel");
    let banners = vcx.debug_bounds("activity-banners").expect("build offers");
    let tree = vcx.debug_bounds("scope-tree").expect("scope tree");
    assert!(banners.size.height <= panel.size.height * 0.4 + gpui_kit::px(1.0));
    assert!(tree.size.height >= panel.size.height * 0.4);
}

#[gpui_kit::test]
fn remote_activity_banner_names_the_reader_and_dismisses(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, VisualTestContext};
    use volna_trace::remote::{objects::Metadata, session::RemoteSession};
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.vtr");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr"),
        &path,
    )
    .unwrap();
    let source = volna_trace::session::OpenSpec::Path(path).open().unwrap();
    let mut metadata = Metadata::from_session(source.as_ref());
    metadata.server = "build-01".into();
    let remote = std::sync::Arc::new(RemoteSession::new(17, metadata).unwrap());
    let window = cx.add_window(Workspace::new);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    window
        .update(&mut vcx, |ws, _, cx| {
            ws.app
                .settings_loaded(r#"{"hierarchy.activityIndex":"ask"}"#);
            ws.app.set_session(remote);
            ws.after(None, cx);
            let views = ws.app.activity.builds(
                ws.app.doc.traces(),
                ws.app.settings.resolved().hierarchy.activity_index,
            );
            assert_eq!(views.len(), 1);
            assert!(views[0].message().contains("build-01 builds it"));
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let banner = vcx
        .debug_bounds("activity-banner-A")
        .expect("remote build offer");
    let message = vcx
        .debug_bounds("activity-message-A")
        .expect("server-named message");
    assert!(message.size.width > gpui_kit::px(0.0) && banner.contains(&message.center()));
    assert!(vcx.debug_bounds("activity-build-A").is_some());
    let dismiss = vcx.debug_bounds("activity-dismiss-A").expect("Not now");
    vcx.simulate_click(dismiss.center(), Modifiers::default());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("activity-banner-A").is_none());
}

#[gpui_kit::test]
fn clicking_waveform_appearance_steppers_repaints_visible_panels(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, VisualTestContext, size};
    use volna_core::app::SettingsCommand;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.simulate_resize(size(px(2200.0), px(900.0)));
    let waves = window
        .update(&mut vcx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all([0])), Some(window), cx);
            let first = ws.app.panels.focused_id();
            ws.dispatch(Command::Action(Action::SplitRight), Some(window), cx);
            let second = ws.app.panels.focused_id();
            ws.dispatch(Command::Action(Action::SplitRight), Some(window), cx);
            ws.dispatch(Command::Settings(SettingsCommand::Open), Some(window), cx);
            [first, second]
        })
        .unwrap();
    vcx.run_until_parked();
    for (key, base, step, selector) in [
        (
            "appearance.digitalWaveWidth",
            1.0,
            0.25,
            "setting-number-appearance.digitalWaveWidth",
        ),
        (
            "appearance.analogWaveWidth",
            1.25,
            0.25,
            "setting-number-appearance.analogWaveWidth",
        ),
        (
            "appearance.waveSpacing",
            5.0,
            1.0,
            "setting-number-appearance.waveSpacing",
        ),
    ] {
        window
            .update(&mut vcx, |ws, window, cx| {
                ws.dispatch(
                    Command::Settings(SettingsCommand::Query(format!("@id:{key}"))),
                    Some(window),
                    cx,
                );
            })
            .unwrap();
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = vcx.debug_bounds(selector).expect("numeric control");
        for increment in [true, false] {
            let before = window
                .update(&mut vcx, |ws, _, _| {
                    waves.map(|id| ws.app.panels.waves(id).unwrap().frames_painted)
                })
                .unwrap();
            assert!(
                before.iter().all(|count| *count > 0),
                "waveforms must be visible"
            );
            let button = point(
                if increment {
                    bounds.right() - px(12.0)
                } else {
                    bounds.left() + px(12.0)
                },
                bounds.center().y,
            );
            vcx.simulate_mouse_move(button, None, Modifiers::default());
            vcx.simulate_click(button, Modifiers::default());
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            window
                .update(&mut vcx, |ws, _, _| {
                    assert_eq!(
                        ws.app.settings.value(key).unwrap().as_f64(),
                        Some(base + if increment { step } else { 0.0 })
                    );
                    for (id, before) in waves.into_iter().zip(before) {
                        assert!(
                            ws.app.panels.waves(id).unwrap().frames_painted > before,
                            "{key}: panel {id:?} must repaint after a stepper click"
                        );
                    }
                })
                .unwrap();
        }
    }
}

#[gpui_kit::test]
fn shift_o_toggles_group_overlays_and_hosts_their_menu(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::wave::model::MenuAction;
    use volna_core::wave::{GroupStyle, MenuEntry};
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    window
        .update(cx, |ws, window, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0, 1, 2])), Some(window), cx);
            let w = ws.app.panels.focused_waves_mut().unwrap();
            w.selected = [0, 1, 2].into();
            w.anchor = Some(0);
            ws.dispatch(Command::Action(Action::GroupSelection), Some(window), cx);
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("shift-o");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    window
        .update(&mut vcx, |ws, window, cx| {
            let waves = ws.app.panels.focused_waves().unwrap();
            assert!(waves.items()[0].group().unwrap().is_overlaid());
            assert_eq!(waves.items()[0].height().multiple(), 3);
            assert_eq!(ws.app.undo_label(), Some("Overlay Group 1"));
            let panel = ws.app.panels.focused_id();
            ws.dispatch(Command::OpenSignalMenu(panel), Some(window), cx);
        })
        .unwrap();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    window
        .update(&mut vcx, |ws, window, cx| {
            assert!(ws.wave_menu.is_some());
            let menu = ws
                .app
                .panels
                .focused_waves()
                .unwrap()
                .menu
                .as_ref()
                .unwrap();
            assert!(
                menu.items()
                    .find(|i| i.action == MenuAction::Overlaid)
                    .unwrap()
                    .checked
            );
            assert!(
                menu.entries
                    .iter()
                    .any(|e| matches!(e, MenuEntry::Label(l) if l == "Shared range"))
            );
            let panel = ws.app.panels.focused_id();
            ws.dispatch(
                Command::MenuSelect(panel, MenuAction::Stack(false)),
                Some(window),
                cx,
            );
        })
        .unwrap();
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            assert_eq!(
                ws.app.panels.focused_waves().unwrap().items()[0]
                    .group()
                    .unwrap()
                    .style,
                GroupStyle::Activity
            );
            let labels: Vec<_> = crate::palette::commands(&ws.app, "overlaid")
                .into_iter()
                .map(|(l, _)| l)
                .collect();
            assert!(labels.iter().any(|l| l == "Toggle Overlaid Lines"));
        })
        .unwrap();
}

#[gpui_kit::test]
fn table_column_edges_resize_fit_and_undo_without_opening_details(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, MouseButton, MouseDownEvent, VisualTestContext, point, size};
    use volna_core::{panels::PanelsCommand, table::TableCommand};
    use volna_trace::data::Member;
    init(cx);
    for dark in [true, false] {
        let mut workspace = None;
        let root = cx.add_window(|window, cx| {
            let ws = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(ws.clone());
            gpui_kit::component::Root::new(ws, window, cx)
        });
        let window = RootWindow {
            root,
            workspace: workspace.unwrap(),
        };
        let mut vcx = VisualTestContext::from_window(root.into(), cx);
        vcx.simulate_resize(size(px(1600.0), px(700.0)));
        let panel = window
            .update(&mut vcx, |ws, window, cx| {
                ws.set_session(
                    OpenSpec::Path(
                        concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/examples/pipeline_showcase.vtr"
                        )
                        .into(),
                    )
                    .open()
                    .unwrap(),
                    cx,
                );
                ws.app.sidebar_visible = false;
                let origin = ws.app.panels.focused_id();
                ws.dispatch(
                    Command::OpenTable {
                        selected: a_all(vec![Member::Generator(0)]),
                        clicked: None,
                    },
                    Some(window),
                    cx,
                );
                let panel = ws.app.panels.focused_id();
                ws.dispatch(
                    Command::Panels(PanelsCommand::Close(origin)),
                    Some(window),
                    cx,
                );
                crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                panel
            })
            .unwrap();
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        vcx.simulate_resize(size(px(360.0), px(700.0)));
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let header = window
            .update(&mut vcx, |ws, _, _| {
                let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                let column = &table.layout.columns[0];
                point(px(column.rect.left() + 20.0), px(column.rect.top() + 12.0))
            })
            .unwrap();
        let (budget, previous_limit) = window
            .update(&mut vcx, |ws, _, _| {
                let budget = ws
                    .app
                    .doc
                    .session(TraceId::A)
                    .unwrap()
                    .memory_budget()
                    .unwrap();
                let previous_limit = budget.limit();
                budget.set_limit(budget.used());
                (budget, previous_limit)
            })
            .unwrap();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        vcx.simulate_click(header, Modifiers::default());
        vcx.simulate_keystrokes("enter");
        window
            .update(&mut vcx, |ws, _, _| {
                let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                assert!(!table.can_sort_column(0));
                assert_eq!(table.sort_requested, None);
                assert!(!table.querying());
            })
            .unwrap();
        budget.set_limit(previous_limit);
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        for expected in [Some(false), Some(true), None] {
            match expected {
                Some(false) => vcx.simulate_click(header, Modifiers::default()),
                Some(true) => vcx.simulate_keystrokes("enter"),
                None => vcx.simulate_keystrokes("space"),
            }
            vcx.run_until_parked();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let sorting = window
                    .update(&mut vcx, |ws, _, _| {
                        ws.app.tick(volna_core::Instant::now());
                        let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                        assert_eq!(
                            table.sort_requested.map(|s| s.descending),
                            expected,
                            "one header click must cycle once"
                        );
                        assert!(table.query_error.is_none(), "{:?}", table.query_error);
                        table.querying()
                    })
                    .unwrap();
                if !sorting {
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            window
                .update(&mut vcx, |ws, _, _| {
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    assert_eq!(table.applied_sort().map(|s| s.descending), expected);
                    assert_eq!(table.selected, None);
                    assert_eq!(ws.app.panels.len(), 1);
                })
                .unwrap();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
        }
        vcx.simulate_resize(size(px(1600.0), px(700.0)));
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let edge = |vcx: &mut VisualTestContext, source_index, in_body| {
            window
                .update(vcx, |ws, _, _| {
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    let column = table
                        .layout
                        .columns
                        .iter()
                        .find(|column| column.source_index == source_index)
                        .unwrap();
                    let y = if in_body {
                        table.layout.body.top() + table.layout.body.height() * 0.5
                    } else {
                        column.rect.top() + 12.0
                    };
                    (column.rect.right() - 1.0, y)
                })
                .unwrap()
        };
        let at = |x, y| point(px(x), px(y));
        for in_body in [false, true] {
            let (x, y) = edge(&mut vcx, 0, in_body);
            vcx.simulate_mouse_move(at(x + 2.0, y), None, Modifiers::default());
            vcx.simulate_mouse_down(at(x + 2.0, y), MouseButton::Left, Modifiers::default());
            vcx.simulate_mouse_move(
                at(x + 152.0, y + 60.0),
                Some(MouseButton::Left),
                Modifiers::default(),
            );
            vcx.simulate_mouse_up(
                at(x + 152.0, y + 60.0),
                MouseButton::Left,
                Modifiers::default(),
            );
            vcx.run_until_parked();
            window
                .update(&mut vcx, |ws, window, cx| {
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    assert_eq!(table.widths[0], Some(360.0));
                    assert_eq!(table.selected, None);
                    assert!(!table.dragging());
                    assert_eq!(ws.app.panels.len(), 1);
                    ws.dispatch(Command::Undo, Some(window), cx);
                    assert_eq!(
                        ws.app
                            .panels
                            .get(panel)
                            .unwrap()
                            .kind
                            .table()
                            .unwrap()
                            .widths[0],
                        None
                    );
                })
                .unwrap();
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
        }
        window
            .update(&mut vcx, |ws, window, cx| {
                ws.dispatch(
                    Command::Table(
                        panel,
                        TableCommand::ResizeColumn {
                            source_index: 6,
                            width: 500.0,
                        },
                    ),
                    Some(window),
                    cx,
                );
            })
            .unwrap();
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let (x, y) = edge(&mut vcx, 6, true);
        vcx.simulate_mouse_move(at(x + 2.0, y), None, Modifiers::default());
        vcx.simulate_mouse_down(at(x + 2.0, y), MouseButton::Left, Modifiers::default());
        vcx.simulate_mouse_move(
            at(x + 182.0, y + 40.0),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        vcx.simulate_mouse_up(
            at(x + 182.0, y + 40.0),
            MouseButton::Left,
            Modifiers::default(),
        );
        vcx.run_until_parked();
        window
            .update(&mut vcx, |ws, window, cx| {
                assert_eq!(
                    ws.app
                        .panels
                        .get(panel)
                        .unwrap()
                        .kind
                        .table()
                        .unwrap()
                        .widths[6],
                    Some(680.0)
                );
                ws.dispatch(
                    Command::Table(
                        panel,
                        TableCommand::ResizeColumn {
                            source_index: 0,
                            width: 48.0,
                        },
                    ),
                    Some(window),
                    cx,
                );
            })
            .unwrap();
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let (x, y) = edge(&mut vcx, 0, true);
        vcx.simulate_mouse_move(at(x, y), None, Modifiers::default());
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at(x, y),
            modifiers: Modifiers::default(),
            click_count: 2,
            first_mouse: false,
        });
        vcx.simulate_mouse_up(at(x, y), MouseButton::Left, Modifiers::default());
        vcx.run_until_parked();
        window
            .update(&mut vcx, |ws, window, cx| {
                let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                assert!(table.widths[0].unwrap() > 48.0);
                assert_eq!(table.selected, None);
                assert_eq!(ws.app.panels.len(), 1);
                ws.dispatch(
                    Command::Table(panel, TableCommand::ResetColumns),
                    Some(window),
                    cx,
                );
                assert!(
                    ws.app
                        .panels
                        .get(panel)
                        .unwrap()
                        .kind
                        .table()
                        .unwrap()
                        .widths
                        .iter()
                        .all(Option::is_none)
                );
            })
            .unwrap();
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let (x, y) = edge(&mut vcx, 6, true);
        let auto_width = window
            .update(&mut vcx, |ws, _, _| {
                ws.app
                    .panels
                    .get(panel)
                    .unwrap()
                    .kind
                    .table()
                    .unwrap()
                    .layout
                    .columns
                    .last()
                    .unwrap()
                    .rect
                    .width()
            })
            .unwrap();
        vcx.simulate_mouse_move(at(x - 2.0, y), None, Modifiers::default());
        vcx.simulate_mouse_down(at(x - 2.0, y), MouseButton::Left, Modifiers::default());
        vcx.simulate_mouse_move(
            at(x - 62.0, y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        vcx.simulate_mouse_up(at(x - 62.0, y), MouseButton::Left, Modifiers::default());
        vcx.run_until_parked();
        let before = window
            .update(&mut vcx, |ws, _, _| {
                let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                assert_eq!(table.widths[6], Some(auto_width - 60.0));
                table.widths.get().clone()
            })
            .unwrap();
        let mut keyboard_resized = false;
        for _ in 0..20 {
            vcx.simulate_keystrokes("tab");
            vcx.simulate_keystrokes("right");
            vcx.run_until_parked();
            keyboard_resized = window
                .update(&mut vcx, |ws, _, _| {
                    ws.app
                        .panels
                        .get(panel)
                        .unwrap()
                        .kind
                        .table()
                        .unwrap()
                        .widths
                        .get()
                        != &before
                })
                .unwrap();
            if keyboard_resized {
                break;
            }
        }
        assert!(
            keyboard_resized,
            "column dividers must be reachable by Tab and resize with Right"
        );
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let cell = window
            .update(&mut vcx, |ws, _, _| {
                let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                at(
                    table.layout.body.left() + 20.0,
                    table.layout.body.top() + 12.0,
                )
            })
            .unwrap();
        vcx.simulate_click(cell, Modifiers::default());
        vcx.run_until_parked();
        window
            .update(&mut vcx, |ws, _, _| {
                let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                assert!(table.selected.is_some());
                assert!(!table.dragging());
                assert_eq!(table.applied_sort(), None);
                assert_eq!(ws.app.panels.len(), 1);
            })
            .unwrap();
        vcx.simulate_resize(size(px(360.0), px(260.0)));
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let (x, y) = edge(&mut vcx, 0, true);
        vcx.simulate_mouse_move(at(x, y), None, Modifiers::default());
        vcx.simulate_event(gpui_kit::ScrollWheelEvent {
            position: at(x, y),
            delta: gpui_kit::ScrollDelta::Pixels(at(0.0, 48.0)),
            ..Default::default()
        });
        vcx.run_until_parked();
        window
            .update(&mut vcx, |ws, _, _| {
                let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                assert_eq!(
                    table.viewport.top, 2,
                    "scrolling over a divider reaches the table"
                );
                assert!(!table.dragging());
            })
            .unwrap();
    }
}

#[gpui_kit::test]
fn numbered_markers_work_from_workspace_table_and_transaction_details(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::{panels::PanelsCommand, table::TableCommand, transaction::TransactionCommand};
    use volna_trace::data::Member;
    init(cx);
    let mut workspace = None;
    let root = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    let window = RootWindow {
        root,
        workspace: workspace.unwrap(),
    };
    let table = window
        .update(cx, |ws, window, cx| {
            let session = OpenSpec::Path(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/examples/pipeline_showcase.vtr"
                )
                .into(),
            )
            .open()
            .unwrap();
            ws.set_session(session, cx);
            ws.dispatch(
                Command::OpenTable {
                    selected: a_all(vec![Member::Generator(0)]),
                    clicked: None,
                },
                Some(window),
                cx,
            );
            ws.app.panels.focused_id()
        })
        .unwrap();
    let mut vcx = VisualTestContext::from_window(root.into(), cx);
    vcx.run_until_parked();
    let time = window
        .update(&mut vcx, |ws, window, cx| {
            ws.dispatch(
                Command::Table(table, TableCommand::GoTo(10)),
                Some(window),
                cx,
            );
            ws.app.doc.shared.cursor.expect("table row sets cursor")
        })
        .unwrap();
    vcx.simulate_keystrokes("ctrl-1");
    window
        .update(&mut vcx, |ws, window, cx| {
            assert_eq!(ws.app.doc.markers()[0].time, time);
            ws.dispatch(Command::ShowTransaction { from: table }, Some(window), cx);
        })
        .unwrap();
    vcx.run_until_parked();
    let detail = window
        .update(&mut vcx, |ws, window, cx| {
            let detail = ws.app.panels.focused_id();
            assert!(ws.app.panels.transaction(detail).is_some());
            ws.dispatch(
                Command::Transaction(detail, TransactionCommand::Cursor(time + 1)),
                Some(window),
                cx,
            );
            detail
        })
        .unwrap();
    vcx.simulate_keystrokes("ctrl-2 1");
    window
        .update(&mut vcx, |ws, window, cx| {
            assert_eq!(
                ws.app
                    .doc
                    .markers()
                    .iter()
                    .find(|m| m.id.get() == 2)
                    .unwrap()
                    .time,
                time + 1
            );
            assert_eq!(ws.app.doc.shared.cursor, Some(time));
            assert_eq!(ws.app.panels.focused_id(), detail);
            window.focus(&ws.focus_handle, cx);
            cx.notify();
        })
        .unwrap();
    vcx.simulate_keystrokes("2");
    window
        .update(&mut vcx, |ws, window, cx| {
            assert_eq!(ws.app.doc.shared.cursor, Some(time + 1));
            ws.dispatch(
                Command::Panels(PanelsCommand::Focus(table)),
                Some(window),
                cx,
            );
        })
        .unwrap();
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, window, cx| {
            window.focus(&ws.filter.read(cx).focus_handle(cx), cx);
        })
        .unwrap();
    vcx.simulate_keystrokes("1 2 3 ctrl-3");
    window
        .update(&mut vcx, |ws, _, cx| {
            assert_eq!(ws.filter.read(cx).text(), "123");
            assert_eq!(ws.app.doc.shared.cursor, Some(time + 1));
            assert_eq!(ws.app.doc.markers().len(), 2);
        })
        .unwrap();
}

#[gpui_kit::test]
fn transaction_right_dock_tracks_native_moves_resize_visibility_and_undo(cx: &mut TestAppContext) {
    check_dock_interactions(cx, None);
}

#[gpui_kit::test]
fn islands_docking_and_live_switching_preserve_panel_identity(cx: &mut TestAppContext) {
    check_dock_interactions(cx, Some("volna-islands-dark"));
}

#[gpui_kit::test]
fn islands_light_docking_and_live_switching_preserve_panel_identity(cx: &mut TestAppContext) {
    check_dock_interactions(cx, Some("volna-islands-light"));
}

fn check_dock_interactions(cx: &mut TestAppContext, island_theme: Option<&'static str>) {
    let islands = island_theme.is_some();
    use gpui_kit::base::dock::{DockPlacement, InsertTarget};
    use gpui_kit::{VisualTestContext, size};
    use volna_core::panels::{PanelsCommand, Placement};
    use volna_core::testing::a;
    use volna_trace::data::transactions::TransactionRef;
    init(cx);
    let window = cx.add_window(Workspace::new);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_resize(size(px(1200.0), px(800.0)));
    let (origin, detail) = window
        .update(&mut vcx, |ws, window, cx| {
            let session = OpenSpec::Path(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/examples/pipeline_showcase.vtr"
                )
                .into(),
            )
            .open()
            .unwrap();
            let track = session
                .tracks()
                .iter()
                .find(|track| track.path.join(".") == "soc.cpu0.pipeline.instruction")
                .unwrap()
                .id;
            ws.set_session(session, cx);
            ws.app.sidebar_visible = false;
            ws.dispatch(Command::OpenPipeline { track: a(track) }, Some(window), cx);
            let origin = ws.app.panels.focused_id();
            let detail = ws.app.panels.first_free_transaction().unwrap();
            assert!(matches!(
                ws.app
                    .panels
                    .transaction(detail)
                    .unwrap()
                    .state(&ws.app.doc),
                volna_core::transaction::TxPanelState::Empty
            ));
            ws.dispatch(
                Command::SelectTransaction {
                    panel: origin,
                    track: a(track),
                    id: TransactionRef(33),
                    cursor: None,
                },
                Some(window),
                cx,
            );
            (origin, detail)
        })
        .unwrap();
    vcx.run_until_parked();
    if let Some(island_theme) = island_theme {
        let snapshot = |ws: &Workspace, cx: &gpui_kit::App| {
            let area = ws.dock.as_ref().unwrap().read(cx).area.read(cx);
            let regions = [DockPlacement::Center, DockPlacement::Right].map(|region| {
                let tree = area.layout(region).unwrap();
                (tree.node_ids(), tree.panels().collect::<Vec<_>>())
            });
            (ws.app.panels.layout().clone(), regions)
        };
        let before = window
            .update(&mut vcx, |ws, _, cx| snapshot(ws, cx))
            .unwrap();
        for (id, _) in
            volna_core::theme::builtin::choices().chain(std::iter::once((island_theme, "")))
        {
            vcx.update(|_, cx| {
                crate::theme::install_themes(
                    volna_core::theme::builtin::resolve(id, true).unwrap(),
                    cx,
                );
            });
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            window
                .update(&mut vcx, |ws, _, cx| {
                    assert_eq!(snapshot(ws, cx), before, "{id}");
                    assert_eq!(
                        ws.app
                            .panels
                            .transaction(detail)
                            .unwrap()
                            .shown()
                            .unwrap()
                            .id,
                        TransactionRef(33)
                    );
                })
                .unwrap();
        }
    }
    if islands {
        use gpui_kit::{Modifiers, MouseButton, point};
        let (start, width) = window
            .update(&mut vcx, |ws, _, cx| {
                let area = ws.dock.as_ref().unwrap().read(cx).area.read(cx);
                let width = area.dock_size(DockPlacement::Right).unwrap();
                (
                    point(
                        area.bounds().right() - width + px(1.),
                        area.bounds().center().y,
                    ),
                    width,
                )
            })
            .unwrap();
        let modifiers = Modifiers::default();
        vcx.simulate_mouse_move(start, None, modifiers);
        vcx.simulate_mouse_down(start, MouseButton::Left, modifiers);
        for delta in [10., 40., 80.] {
            vcx.simulate_mouse_move(
                start - point(px(delta), px(0.)),
                Some(MouseButton::Left),
                modifiers,
            );
            vcx.run_until_parked();
        }
        let end = start - point(px(80.), px(0.));
        vcx.simulate_mouse_up(end, MouseButton::Left, modifiers);
        vcx.run_until_parked();
        let resized = window
            .update(&mut vcx, |ws, _, cx| {
                let area = ws.dock.as_ref().unwrap().read(cx).area.read(cx);
                let resized = area.dock_size(DockPlacement::Right).unwrap();
                assert!(
                    resized > width + px(60.),
                    "drag must continue outside the original hit band"
                );
                assert_eq!(
                    ws.app.panels.layout().right.as_ref().unwrap().width,
                    f32::from(resized)
                );
                resized
            })
            .unwrap();
        vcx.simulate_mouse_move(start, None, modifiers);
        vcx.run_until_parked();
        window
            .update(&mut vcx, |ws, _, cx| {
                assert_eq!(
                    ws.dock
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .area
                        .read(cx)
                        .dock_size(DockPlacement::Right),
                    Some(resized)
                );
            })
            .unwrap();
    }
    let resized_node = window
        .update(&mut vcx, |ws, window, cx| {
            assert_eq!(
                ws.app.panels.layout().placement(detail),
                Some(Placement::Right)
            );
            let area = ws.dock.as_ref().unwrap().read(cx).area.clone();
            let node = area
                .read(cx)
                .layout(DockPlacement::Right)
                .unwrap()
                .root()
                .id();
            area.update(cx, |area, cx| {
                area.set_dock_size(DockPlacement::Right, px(480.0), window, cx);
            });
            node
        })
        .unwrap();
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, window, cx| {
            let area = ws.dock.as_ref().unwrap().read(cx).area.clone();
            assert_eq!(
                area.read(cx)
                    .layout(DockPlacement::Right)
                    .unwrap()
                    .root()
                    .id(),
                resized_node
            );
            assert_eq!(ws.app.panels.layout().right.as_ref().unwrap().width, 480.0);
            let right = area.read(cx).layout(DockPlacement::Right).unwrap();
            let panel = right.panels().next().unwrap();
            let center = area.read(cx).layout(DockPlacement::Center).unwrap();
            let target = center
                .find_panel_node(center.panels().next().unwrap())
                .unwrap();
            // Exercise the same Kit move path used by tab drag-and-drop.
            area.update(cx, |area, cx| {
                area.move_panel(
                    panel,
                    InsertTarget::Tabs {
                        node: target,
                        ix: None,
                        activate: true,
                    },
                    window,
                    cx,
                );
            });
        })
        .unwrap();
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, window, cx| {
            assert_eq!(
                ws.app.panels.layout().placement(detail),
                Some(Placement::Center)
            );
            assert!(ws.app.panels.layout().right.is_none());
            assert_eq!(ws.app.panels.len(), 2);
            assert_eq!(
                ws.app
                    .panels
                    .transaction(detail)
                    .unwrap()
                    .shown()
                    .unwrap()
                    .id,
                TransactionRef(33)
            );
            ws.dispatch(Command::Undo, Some(window), cx);
        })
        .unwrap();
    vcx.run_until_parked();
    let nodes = |ws: &Workspace, cx: &gpui_kit::App| {
        let area = ws.dock.as_ref().unwrap().read(cx).area.read(cx);
        [DockPlacement::Center, DockPlacement::Right]
            .map(|region| area.layout(region).unwrap().root().id())
    };
    let retained_nodes = window
        .update(&mut vcx, |ws, window, cx| {
            assert_eq!(
                ws.app.panels.layout().placement(detail),
                Some(Placement::Right)
            );
            let before = nodes(ws, cx);
            ws.dispatch(
                Command::Panels(PanelsCommand::ToggleRightDock),
                Some(window),
                cx,
            );
            before
        })
        .unwrap();
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, window, cx| {
            assert_eq!(nodes(ws, cx), retained_nodes);
            assert!(
                !ws.dock
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .area
                    .read(cx)
                    .is_dock_open(DockPlacement::Right)
            );
            ws.dispatch(Command::ShowTransaction { from: origin }, Some(window), cx);
        })
        .unwrap();
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, cx| {
            assert_eq!(nodes(ws, cx), retained_nodes);
            assert!(
                ws.dock
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .area
                    .read(cx)
                    .is_dock_open(DockPlacement::Right)
            );
            assert_eq!(ws.app.panels.focused_id(), detail);
            assert_eq!(ws.app.panels.len(), 2);
            ws.app.panels.validate().unwrap();
        })
        .unwrap();
    vcx.simulate_resize(size(px(360.0), px(800.0)));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    window
        .update(&mut vcx, |ws, _, cx| {
            let area = ws.dock.as_ref().unwrap().read(cx).area.read(cx);
            assert_eq!(nodes(ws, cx), retained_nodes);
            assert!(area.dock_size(DockPlacement::Right).unwrap() < area.bounds().size.width);
            assert_eq!(ws.app.panels.layout().right.as_ref().unwrap().width, 480.0);
        })
        .unwrap();

    let show_close_menu = |vcx: &mut VisualTestContext| {
        let (owner, generation) = window
            .update(vcx, |ws, _, cx| (cx.weak_entity(), ws.app.doc.generation()))
            .unwrap();
        let menu = vcx.update(|window, cx| {
            PopupMenu::build(window, cx, |menu, _, cx| {
                crate::dock::close_menu(menu, &owner, detail, generation, cx)
            })
        });
        window
            .update(vcx, |ws, window, cx| {
                window.focus(&menu.read(cx).focus_handle(cx), cx);
                ws.show_status_menu(point(px(20.0), px(80.0)), menu, window, cx);
            })
            .unwrap();
        vcx.run_until_parked();
    };
    show_close_menu(&mut vcx);
    // The first sidebar action hides the entire region, retaining the record.
    vcx.simulate_keystrokes("down enter");
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            assert!(!ws.app.panels.layout().right.as_ref().unwrap().open);
            assert_eq!(ws.app.panels.len(), 2);
            assert_eq!(
                ws.app
                    .panels
                    .transaction(detail)
                    .unwrap()
                    .shown()
                    .unwrap()
                    .id,
                TransactionRef(33)
            );
        })
        .unwrap();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let toggle = vcx
        .debug_bounds("toggle-right-sidebar")
        .expect("hidden sidebar retains its toolbar toggle");
    vcx.simulate_click(toggle.center(), gpui_kit::Modifiers::default());
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, window, cx| {
            assert!(ws.app.panels.layout().right.as_ref().unwrap().open);
            assert_eq!(
                ws.app
                    .panels
                    .transaction(detail)
                    .unwrap()
                    .shown()
                    .unwrap()
                    .id,
                TransactionRef(33)
            );
            ws.dispatch(
                Command::Panels(PanelsCommand::Move {
                    panel: detail,
                    placement: Placement::Center,
                }),
                Some(window),
                cx,
            );
        })
        .unwrap();
    vcx.run_until_parked();
    show_close_menu(&mut vcx);
    // In the center the close action still removes the panel.
    vcx.simulate_keystrokes("down enter");
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, _| {
            assert!(ws.app.panels.get(detail).is_none());
            assert!(ws.app.panels.get(origin).is_some());
        })
        .unwrap();

    // The adapter rejects regions the core cannot represent and reinstalls
    // the accepted layout rather than leaving a divergent widget tree visible.
    let accepted = window
        .update(&mut vcx, |ws, window, cx| {
            let area = ws.dock.as_ref().unwrap().read(cx).area.clone();
            area.update(cx, |area, cx| {
                area.set_dock(
                    DockPlacement::Bottom,
                    gpui_kit::base::dock::DockLayout::h_split(),
                    window,
                    cx,
                );
            });
            ws.app.panels.layout().clone()
        })
        .unwrap();
    vcx.run_until_parked();
    window
        .update(&mut vcx, |ws, _, cx| {
            assert_eq!(ws.app.panels.layout(), &accepted);
            assert!(
                !ws.dock
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .area
                    .read(cx)
                    .has_dock(DockPlacement::Bottom)
            );
        })
        .unwrap();
}

#[gpui_kit::test]
fn table_format_menus_preserve_rows_and_support_keyboard_and_narrow_panels(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{Modifiers, VisualTestContext, size};
    use volna_core::{
        panels::PanelsCommand,
        table::{TableCommand, columns::TransactionColumn},
    };
    use volna_trace::data::Member;
    init(cx);
    for dark in [true, false] {
        for (width, zoom) in [(1000.0, 1.0), (360.0, 1.0), (1000.0, 1.5), (360.0, 1.5)] {
            let mut workspace = None;
            let root = cx.add_window(|window, cx| {
                let ws = cx.new(|cx| Workspace::new(window, cx));
                workspace = Some(ws.clone());
                gpui_kit::component::Root::new(ws, window, cx)
            });
            let window = RootWindow {
                root,
                workspace: workspace.unwrap(),
            };
            let mut vcx = VisualTestContext::from_window(root.into(), cx);
            vcx.simulate_resize(size(px(width), px(700.0)));
            let panel = window
                .update(&mut vcx, |ws, window, cx| {
                    ws.set_session(
                        OpenSpec::Path(
                            concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/examples/pipeline_showcase.vtr"
                            )
                            .into(),
                        )
                        .open()
                        .unwrap(),
                        cx,
                    );
                    ws.app.sidebar_visible = false;
                    let origin = ws.app.panels.focused_id();
                    ws.dispatch(
                        Command::OpenTable {
                            selected: a_all(vec![Member::Generator(0)]),
                            clicked: None,
                        },
                        Some(window),
                        cx,
                    );
                    let panel = ws.app.panels.focused_id();
                    ws.dispatch(
                        Command::Panels(PanelsCommand::Close(origin)),
                        Some(window),
                        cx,
                    );
                    for command in [
                        TableCommand::ShowTransactionColumn(TransactionColumn::Id),
                        TableCommand::MoveColumn {
                            source_index: 4,
                            before: Some(0),
                        },
                    ] {
                        ws.dispatch(Command::Table(panel, command), Some(window), cx);
                    }
                    crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                    crate::theme::set_zoom(zoom, cx);
                    panel
                })
                .unwrap();
            vcx.run_until_parked();
            let selected = window
                .update(&mut vcx, |ws, window, cx| {
                    ws.dispatch(
                        Command::Table(panel, TableCommand::Select(10)),
                        Some(window),
                        cx,
                    );
                    ws.app
                        .panels
                        .get(panel)
                        .unwrap()
                        .kind
                        .table()
                        .unwrap()
                        .selected
                        .clone()
                })
                .unwrap();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let format_bounds = vcx.debug_bounds("table-format-4").unwrap();
            let sort_bounds = vcx.debug_bounds("table-sort-4").unwrap();
            assert!(sort_bounds.right() < format_bounds.left());
            assert_eq!(sort_bounds.center().y, format_bounds.center().y);
            assert!(format_bounds.size.height >= px(26.0 * zoom));
            let click = |vcx: &mut VisualTestContext, selector: &'static str| {
                let bounds = vcx.debug_bounds(selector).expect(selector);
                assert!(
                    bounds.left() >= px(0.0) && bounds.right() <= px(width),
                    "{selector}: {bounds:?}"
                );
                vcx.simulate_click(bounds.center(), Modifiers::default());
                vcx.run_until_parked();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
            };
            click(&mut vcx, "table-number-format");
            click(&mut vcx, "table-format-choice-udec");
            click(&mut vcx, "table-format-4");
            click(&mut vcx, "table-format-choice-bin");
            window
                .update(&mut vcx, |ws, _, _| {
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    assert_eq!(table.number_format(), "dec");
                    assert_eq!(table.formats.columns[4].as_deref(), Some("bin"));
                    assert_eq!(table.selected, selected);
                    assert_eq!(table.sort_requested, None);
                })
                .unwrap();
            click(&mut vcx, "table-number-format");
            click(&mut vcx, "table-format-choice-hex");
            click(&mut vcx, "table-format-4");
            click(&mut vcx, "table-format-choice-inherit");
            window
                .update(&mut vcx, |ws, _, _| {
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    assert!(table.formats.columns[4].is_none());
                    assert_eq!(table.column_translator(4).unwrap().id(), "hex");
                    assert_eq!(table.selected, selected);
                })
                .unwrap();
            for _ in 0..40 {
                let focused = vcx.update(|window, _| {
                    gpui_kit::test::TestWindowExt::find(window, ("table-format", 4usize)).focused()
                        == Some(true)
                });
                if focused {
                    break;
                }
                vcx.update(|window, cx| gpui_kit::test::TestWindowExt::press(window, "tab", cx));
            }
            vcx.simulate_keystrokes("space");
            vcx.simulate_event(gpui_kit::KeyUpEvent {
                keystroke: gpui_kit::Keystroke::parse("space").unwrap(),
            });
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(vcx.debug_bounds("table-format-choice-inherit").is_some());
            vcx.simulate_keystrokes("escape");
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(vcx.debug_bounds("table-format-choice-inherit").is_none());
            vcx.simulate_keystrokes("enter");
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(vcx.debug_bounds("table-format-choice-inherit").is_some());
            vcx.simulate_keystrokes("down down down enter");
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            window
                .update(&mut vcx, |ws, _, _| {
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    assert_eq!(table.selected, selected);
                    assert_eq!(table.sort_requested, None);
                    assert!(table.formats.columns[4].is_some());
                })
                .unwrap();
        }
    }
}

#[gpui_kit::test]
fn table_attribute_picker_searches_toggles_and_preserves_selection(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, VisualTestContext, size};
    use volna_core::{
        panels::PanelsCommand,
        table::{TableCommand, columns::TransactionColumn},
    };
    use volna_trace::data::Member;
    init(cx);
    for dark in [true, false] {
        for width in [1000.0, 360.0] {
            let mut workspace = None;
            let root = cx.add_window(|window, cx| {
                let ws = cx.new(|cx| Workspace::new(window, cx));
                workspace = Some(ws.clone());
                gpui_kit::component::Root::new(ws, window, cx)
            });
            let window = RootWindow {
                root,
                workspace: workspace.unwrap(),
            };
            let mut vcx = VisualTestContext::from_window(root.into(), cx);
            vcx.simulate_resize(size(px(width), px(700.0)));
            let panel = window
                .update(&mut vcx, |ws, window, cx| {
                    ws.set_session(
                        OpenSpec::Path(
                            concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/examples/pipeline_showcase.vtr"
                            )
                            .into(),
                        )
                        .open()
                        .unwrap(),
                        cx,
                    );
                    ws.app.sidebar_visible = false;
                    let origin = ws.app.panels.focused_id();
                    ws.dispatch(
                        Command::OpenTable {
                            selected: a_all(vec![Member::Generator(0)]),
                            clicked: None,
                        },
                        Some(window),
                        cx,
                    );
                    let panel = ws.app.panels.focused_id();
                    ws.dispatch(
                        Command::Panels(PanelsCommand::Close(origin)),
                        Some(window),
                        cx,
                    );
                    crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                    panel
                })
                .unwrap();
            vcx.run_until_parked();
            let (key, selected) = window
                .update(&mut vcx, |ws, window, cx| {
                    ws.dispatch(
                        Command::Table(panel, TableCommand::Select(0)),
                        Some(window),
                        cx,
                    );
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    let key = table
                        .loaded_generator()
                        .unwrap()
                        .attribute_catalogue()
                        .iter()
                        .find(|a| a.key != "vtr.label")
                        .unwrap()
                        .key
                        .clone();
                    (key, table.selected.clone())
                })
                .unwrap();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let columns = vcx.debug_bounds("table-columns").expect("Columns button");
            vcx.simulate_click(columns.center(), Modifiers::default());
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            vcx.simulate_keystrokes("down space");
            vcx.simulate_event(gpui_kit::KeyUpEvent {
                keystroke: gpui_kit::Keystroke::parse("space").unwrap(),
            });
            window
                .update(&mut vcx, |ws, _, _| {
                    assert!(
                        !ws.app
                            .panels
                            .get(panel)
                            .unwrap()
                            .kind
                            .table()
                            .unwrap()
                            .columns
                            .transaction_visible(TransactionColumn::Label)
                    );
                })
                .unwrap();
            vcx.simulate_keystrokes("space");
            vcx.simulate_event(gpui_kit::KeyUpEvent {
                keystroke: gpui_kit::Keystroke::parse("space").unwrap(),
            });
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let add = vcx
                .debug_bounds("add-attribute-columns")
                .expect("Add attribute columns action");
            vcx.simulate_click(add.center(), Modifiers::default());
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            vcx.simulate_input("unavailable attribute");
            vcx.simulate_keystrokes("enter");
            window
                .update(&mut vcx, |ws, _, _| {
                    assert_eq!(
                        ws.app
                            .panels
                            .get(panel)
                            .unwrap()
                            .kind
                            .table()
                            .unwrap()
                            .columns
                            .len(),
                        7
                    );
                })
                .unwrap();
            vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
                "cmd-a backspace"
            } else {
                "ctrl-a backspace"
            });
            vcx.simulate_input(&key);
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            vcx.simulate_keystrokes("enter");
            vcx.run_until_parked();
            window
                .update(&mut vcx, |ws, _, _| {
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    assert!(
                        table
                            .columns
                            .transaction_visible(TransactionColumn::Attribute(key.clone()))
                    );
                    assert_eq!(table.selected, selected);
                    assert!(ws.app.panels.iter().all(|p| p.kind.transaction().is_none()));
                    assert!(ws.app.take_requests().is_empty());
                })
                .unwrap();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let picker = vcx
                .debug_bounds("table-column-picker")
                .expect("picker stays open after selection");
            assert!(picker.left() >= px(0.0) && picker.right() <= px(width));
            vcx.simulate_keystrokes("down space");
            vcx.run_until_parked();
            window
                .update(&mut vcx, |ws, _, _| {
                    assert!(
                        !ws.app
                            .panels
                            .get(panel)
                            .unwrap()
                            .kind
                            .table()
                            .unwrap()
                            .columns
                            .transaction_visible(TransactionColumn::Attribute(key.clone()))
                    );
                })
                .unwrap();
            vcx.simulate_keystrokes("space escape");
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(vcx.debug_bounds("table-column-picker").is_none());
            window
                .update(&mut vcx, |ws, _, _| {
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    assert!(
                        table
                            .columns
                            .transaction_visible(TransactionColumn::Attribute(key.clone()))
                    );
                    assert_eq!(table.selected, selected);
                })
                .unwrap();
            if width == 1000.0 {
                let (from, to) = window
                    .update(&mut vcx, |ws, _, _| {
                        let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                        let point = |index, fraction| {
                            let rect = table
                                .layout
                                .columns
                                .iter()
                                .find(|c| c.source_index == index)
                                .unwrap()
                                .rect;
                            gpui_kit::point(
                                px(rect.left() + rect.width() * fraction),
                                px(rect.top() + rect.height() * 0.5),
                            )
                        };
                        (point(7, 0.5), point(1, 0.25))
                    })
                    .unwrap();
                vcx.simulate_mouse_down(from, gpui_kit::MouseButton::Left, Modifiers::default());
                vcx.simulate_mouse_move(
                    from + gpui_kit::point(px(12.0), px(0.0)),
                    Some(gpui_kit::MouseButton::Left),
                    Modifiers::default(),
                );
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                vcx.simulate_mouse_move(
                    to,
                    Some(gpui_kit::MouseButton::Left),
                    Modifiers::default(),
                );
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                vcx.simulate_mouse_up(to, gpui_kit::MouseButton::Left, Modifiers::default());
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                let header = window
                    .update(&mut vcx, |ws, _, _| {
                        let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                        let volna_core::table::columns::ColumnSet::Transactions(columns) =
                            table.columns.get()
                        else {
                            panic!()
                        };
                        assert_eq!(columns.iter().nth(1).unwrap().0, 7);
                        assert_eq!(table.sort_requested, None);
                        let rect = table
                            .layout
                            .columns
                            .iter()
                            .find(|c| c.source_index == 7)
                            .unwrap()
                            .rect;
                        gpui_kit::point(
                            px(rect.left() + rect.width() * 0.5),
                            px(rect.top() + rect.height() * 0.5),
                        )
                    })
                    .unwrap();
                vcx.simulate_mouse_down(header, gpui_kit::MouseButton::Right, Modifiers::default());
                vcx.simulate_mouse_up(header, gpui_kit::MouseButton::Right, Modifiers::default());
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                vcx.simulate_keystrokes("down enter");
                window
                    .update(&mut vcx, |ws, _, _| {
                        let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                        assert!(
                            !table
                                .columns
                                .transaction_visible(TransactionColumn::Attribute(key.clone()))
                        );
                        assert_eq!(table.selected, selected);
                    })
                    .unwrap();
            }
        }
    }
}

#[gpui_kit::test]
fn column_filters_keep_input_focus_and_table_shortcuts_out_of_edits(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt;
    use gpui_kit::{Modifiers, VisualTestContext, point, size};
    use volna_core::{panels::PanelsCommand, table::TableCommand};
    use volna_trace::data::Member;
    init(cx);
    for dark in [true, false] {
        for width in [900.0, 360.0] {
            let mut workspace = None;
            let root = cx.add_window(|window, cx| {
                let ws = cx.new(|cx| Workspace::new(window, cx));
                workspace = Some(ws.clone());
                gpui_kit::component::Root::new(ws, window, cx)
            });
            let window = RootWindow {
                root,
                workspace: workspace.unwrap(),
            };
            let mut vcx = VisualTestContext::from_window(root.into(), cx);
            vcx.simulate_resize(size(px(width), px(700.0)));
            let panel = window
                .update(&mut vcx, |ws, window, cx| {
                    ws.set_session(
                        OpenSpec::Path(
                            concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/examples/pipeline_showcase.vtr"
                            )
                            .into(),
                        )
                        .open()
                        .unwrap(),
                        cx,
                    );
                    ws.app.sidebar_visible = false;
                    let origin = ws.app.panels.focused_id();
                    ws.dispatch(
                        Command::OpenTable {
                            selected: a_all(vec![Member::Generator(0)]),
                            clicked: None,
                        },
                        Some(window),
                        cx,
                    );
                    let panel = ws.app.panels.focused_id();
                    ws.dispatch(
                        Command::Panels(PanelsCommand::Close(origin)),
                        Some(window),
                        cx,
                    );
                    crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                    panel
                })
                .unwrap();
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(vcx.debug_bounds("filter-cell-0").is_none());
            let toggle = vcx.debug_bounds("table-filter").unwrap();
            vcx.simulate_click(toggle.center(), Modifiers::default());
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            let field = vcx.debug_bounds("filter-cell-0").unwrap();
            vcx.simulate_click(
                point(field.left() + px(70.0), field.center().y),
                Modifiers::default(),
            );
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(vcx.update(WindowExt::has_focused_input));
            vcx.simulate_input("lw");
            vcx.simulate_keystrokes("enter");
            vcx.run_until_parked();
            window
                .update(&mut vcx, |ws, _, _| {
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    assert_eq!(
                        table.filters[0].as_ref().map(|f| f.query.as_str()),
                        Some("lw")
                    );
                    assert_eq!(table.selected, None);
                    assert_eq!(
                        ws.app.panels.len(),
                        1,
                        "Enter in a filter must not open details"
                    );
                    assert!(ws.app.doc.markers().is_empty());
                })
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let working = window
                    .update(&mut vcx, |ws, _, _| {
                        ws.app.tick(volna_core::Instant::now());
                        ws.app
                            .panels
                            .get(panel)
                            .unwrap()
                            .kind
                            .table()
                            .unwrap()
                            .querying()
                    })
                    .unwrap();
                if !working {
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            });
            vcx.simulate_input("m 1");
            vcx.simulate_keystrokes("escape enter");
            window
                .update(&mut vcx, |ws, _, _| {
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    assert_eq!(table.filters[0].as_ref().unwrap().query, "lw");
                    assert!(ws.app.doc.markers().is_empty());
                })
                .unwrap();
            window
                .update(&mut vcx, |ws, window, cx| {
                    ws.dispatch(
                        Command::Table(panel, TableCommand::ToggleFilters),
                        Some(window),
                        cx,
                    );
                    let table = ws.app.panels.get(panel).unwrap().kind.table().unwrap();
                    assert!(!table.filters_visible);
                    assert_eq!(table.active_filter_count(), 1);
                })
                .unwrap();
        }
    }
}

#[gpui_kit::test]
fn auxiliary_windows_route_keyboard_navigation_and_return_panels_without_losing_state(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{Modifiers, VisualTestContext, size};
    use volna_core::panels::{PanelsCommand, Placement};
    init(cx);
    for (dark, width, zoom) in [(true, 1000.0, 1.0), (false, 360.0, 1.5)] {
        let mut owner = None;
        let root = cx.add_window(|window, cx| {
            let ws = cx.new(|cx| Workspace::new(window, cx));
            owner = Some(ws.clone());
            gpui_kit::component::Root::new(ws, window, cx)
        });
        let main = RootWindow {
            root,
            workspace: owner.unwrap(),
        };
        let mut vcx = VisualTestContext::from_window(root.into(), cx);
        main.update(&mut vcx, |ws, _, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
        })
        .unwrap();
        vcx.run_until_parked();
        let (a, b) = main
            .update(&mut vcx, |ws, window, cx| {
                crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                crate::theme::set_zoom(zoom, cx);
                ws.dispatch(Command::AddVars(a_all(vec![0, 1])), Some(window), cx);
                let a = ws.app.panels.focused_id();
                ws.dispatch(
                    Command::Panels(PanelsCommand::NewTab { group_of: a }),
                    Some(window),
                    cx,
                );
                let b = ws.app.panels.focused_id();
                (a, b)
            })
            .unwrap();
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        let menu =
            vcx.update(|window, _| gpui_kit::test::TestWindowExt::find(window, "menu").bounds());
        vcx.simulate_click(menu.center(), Modifiers::default());
        vcx.simulate_keystrokes("down enter");
        vcx.run_until_parked();
        let handle = main
            .update(&mut vcx, |ws, _, cx| {
                assert_eq!(cx.windows().len(), 2);
                assert!(ws.dock.as_ref().unwrap().read(cx).focus(b, cx).is_none());
                ws.windows.handle_for(b, &ws.app.panels).unwrap()
            })
            .unwrap();
        {
            let mut auxiliary = VisualTestContext::from_window(handle, &vcx);
            auxiliary.simulate_resize(size(px(width), px(600.0)));
            auxiliary.run_until_parked();
            auxiliary.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
            main.workspace.update(&mut auxiliary, |ws, cx| {
                assert!(ws.panel_focus(b, cx).is_some());
                assert!(
                    ws.canvas.len() >= 2,
                    "each window has its own renderer cache"
                );
                assert_eq!(ws.app.panels.focused_id(), b);
            });
            auxiliary.simulate_keystrokes("=");
            auxiliary.run_until_parked();
            main.workspace.update(&mut auxiliary, |ws, _| {
                assert_eq!(
                    ws.app.panels.waves(a).unwrap().viewport(&ws.app.doc),
                    ws.app.panels.waves(b).unwrap().viewport(&ws.app.doc)
                );
            });
            auxiliary.update(|window, cx| window.draw(cx).clear(cx));
            let bounds = auxiliary.update(|window, _| {
                gpui_kit::test::TestWindowExt::find(window, "active-panel").bounds()
            });
            let bounds = volna_core::geometry::Rect::from_xywh(
                f32::from(bounds.left()),
                f32::from(bounds.top()),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            );
            let at = main.workspace.update(&mut auxiliary, |ws, cx| {
                let layout = ws
                    .app
                    .layout_panel(
                        b,
                        bounds,
                        crate::theme::canvas_theme(cx),
                        1.0,
                        &mut volna_core::scene::MonoMeasure,
                    )
                    .unwrap();
                let volna_core::app::PanelLayout::Waves(layout) = layout else {
                    panic!()
                };
                point(px(layout.waves.left() + 8.0), px(layout.waves.top() + 8.0))
            });
            auxiliary.simulate_click(at, Modifiers::default());
            auxiliary.run_until_parked();
            main.workspace.update(&mut auxiliary, |ws, _| {
                assert!(ws.app.doc.shared.cursor.is_some());
                assert_eq!(
                    ws.app.panels.waves(a).unwrap().cursor(&ws.app.doc),
                    ws.app.panels.waves(b).unwrap().cursor(&ws.app.doc)
                );
            });
            main.root
                .update(&mut auxiliary, |_, window, _| window.activate_window())
                .unwrap();
            auxiliary.run_until_parked();
            main.update(&mut auxiliary, |ws, window, cx| {
                assert_eq!(ws.app.panels.focused_id(), a);
                assert!(ws.panel_focus(a, cx).unwrap().contains_focused(window, cx));
            })
            .unwrap();
            gpui_kit::TestAppContext::simulate_keystrokes(&mut auxiliary, root.into(), "=");
            auxiliary.run_until_parked();
            handle
                .update(&mut auxiliary, |_, window, _| window.activate_window())
                .unwrap();
            auxiliary.run_until_parked();
            main.workspace.update(&mut auxiliary, |ws, _| {
                assert_eq!(ws.app.panels.focused_id(), b);
            });
            auxiliary.simulate_keystrokes("shift-l");
            auxiliary.run_until_parked();
            main.workspace.update(&mut auxiliary, |ws, _| {
                assert!(ws.app.panels.waves(a).unwrap().nav.link.cursor);
                assert!(!ws.app.panels.waves(b).unwrap().nav.link.cursor);
            });
            auxiliary.simulate_keystrokes("ctrl-,");
            auxiliary.run_until_parked();
            main.update(&mut auxiliary, |ws, window, cx| {
                let settings = ws.app.panels.settings_id().unwrap();
                let view = ws.settings_view(settings, window, cx);
                assert!(view.read(cx).search_focus(cx).is_focused(window));
            })
            .unwrap();
            handle
                .update(&mut auxiliary, |_, window, _| window.activate_window())
                .unwrap();
            auxiliary.run_until_parked();
            if dark {
                let menu = auxiliary.update(|window, _| {
                    gpui_kit::test::TestWindowExt::find(window, "auxiliary-panel-menu").bounds()
                });
                auxiliary.simulate_click(menu.center(), Modifiers::default());
                auxiliary.simulate_keystrokes("down enter");
            } else {
                auxiliary.dispatch_action(CloseWindow);
            }
            auxiliary.run_until_parked();
        }
        vcx.run_until_parked();
        main.update(&mut vcx, |ws, _, cx| {
            assert_eq!(cx.windows().len(), 1);
            assert_eq!(ws.app.panels.layout().placement(b), Some(Placement::Center));
            assert!(!ws.app.panels.waves(b).unwrap().nav.link.cursor);
            ws.dispatch(Command::Undo, None, cx);
        })
        .unwrap();
        vcx.run_until_parked();
        main.update(&mut vcx, |ws, _, cx| {
            assert_eq!(cx.windows().len(), 2);
            assert!(matches!(
                ws.app.panels.layout().placement(b),
                Some(Placement::Window(_))
            ));
            ws.windows.close(cx);
        })
        .unwrap();
        root.update(&mut vcx, |_, window, _| window.remove_window())
            .unwrap();
    }
}

#[gpui_kit::test]
fn auxiliary_os_close_returns_tabs_and_main_close_disposes_its_windows(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::panels::{PanelsCommand, Placement};
    init(cx);
    let owner = cx.update(|cx| crate::open_main_window(cx, false).unwrap());
    let main = cx.update(|cx| cx.windows()[0]);
    owner.update(cx, |ws, cx| {
        ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
    });
    cx.run_until_parked();
    let panel = owner.update(cx, |ws, cx| {
        ws.dispatch(Command::AddVars(a_all(vec![0])), None, cx);
        let panel = ws.app.panels.focused_id();
        ws.dispatch(
            Command::Panels(PanelsCommand::MoveToNewWindow(panel)),
            None,
            cx,
        );
        panel
    });
    cx.run_until_parked();
    let auxiliary = owner.update(cx, |ws, _| {
        ws.windows.handle_for(panel, &ws.app.panels).unwrap()
    });
    {
        let mut view = VisualTestContext::from_window(auxiliary, cx);
        assert!(view.simulate_close());
        owner.update(&mut view, |ws, _| {
            assert_eq!(
                ws.app.panels.layout().placement(panel),
                Some(Placement::Center)
            );
        });
        auxiliary
            .update(&mut view, |_, window, _| window.remove_window())
            .unwrap();
        view.run_until_parked();
    }
    owner.update(cx, |ws, cx| ws.dispatch(Command::Undo, None, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);
    {
        let mut view = VisualTestContext::from_window(main, cx);
        assert!(view.simulate_close());
        assert_eq!(view.update(|_, cx| cx.windows().len()), 1);
        main.update(&mut view, |_, window, _| window.remove_window())
            .unwrap();
    }
    assert!(cx.update(|cx| cx.windows().is_empty()));
}

fn workspace_with_auxiliary_panel(
    cx: &mut TestAppContext,
) -> (
    Entity<Workspace>,
    gpui_kit::AnyWindowHandle,
    gpui_kit::AnyWindowHandle,
) {
    let owner = cx.update(|cx| crate::open_main_window(cx, false).unwrap());
    let main = cx.update(|cx| cx.windows()[0]);
    owner.update(cx, |ws, cx| {
        ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
        ws.dispatch(Command::AddVars(a_all(vec![0])), None, cx);
        let panel = ws.app.panels.focused_id();
        ws.dispatch(
            Command::Panels(volna_core::panels::PanelsCommand::MoveToNewWindow(panel)),
            None,
            cx,
        );
    });
    cx.run_until_parked();
    let auxiliary = owner.read_with(cx, |ws, _| {
        ws.windows
            .handle_for(ws.app.panels.focused_id(), &ws.app.panels)
            .unwrap()
    });
    (owner, main, auxiliary)
}

#[gpui_kit::test]
fn auxiliary_palette_commands_dispatch_and_restore_local_keyboard_input(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    init(cx);
    let (owner, main, auxiliary) = workspace_with_auxiliary_panel(cx);
    owner.update(cx, |ws, _| {
        ws.app.doc.shared.cursor = Some(25);
        ws.app.doc.add_marker(10).unwrap();
        ws.app.doc.add_marker(50).unwrap();
    });
    let mut view = VisualTestContext::from_window(auxiliary, cx);
    view.simulate_keystrokes("ctrl-k");
    view.run_until_parked();
    view.simulate_input("remove all markers");
    view.run_until_parked();
    view.simulate_keystrokes("enter");
    view.run_until_parked();
    owner.read_with(&view, |ws, _| assert!(ws.app.doc.markers().is_empty()));

    view.simulate_keystrokes("m");
    view.run_until_parked();
    owner.read_with(&view, |ws, _| assert_eq!(ws.app.doc.markers().len(), 1));
    view.simulate_keystrokes("ctrl-k");
    view.run_until_parked();
    view.simulate_input("add or name marker");
    view.run_until_parked();
    view.simulate_keystrokes("enter");
    view.run_until_parked();
    view.simulate_input("auxiliary");
    view.simulate_keystrokes("enter");
    view.run_until_parked();
    owner.read_with(&view, |ws, _| {
        assert_eq!(ws.app.doc.markers()[0].label.as_deref(), Some("auxiliary"));
    });
    owner.update(&mut view, |ws, cx| ws.windows.close(cx));
    main.update(&mut view, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn returning_a_panel_commits_its_window_owned_name_editor(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    use volna_core::panels::{PanelsCommand, Placement};

    init(cx);
    let (owner, main, auxiliary) = workspace_with_auxiliary_panel(cx);
    owner.update(cx, |ws, _| {
        ws.app.doc.shared.cursor = Some(25);
        ws.app.doc.add_marker(25).unwrap();
    });
    let mut view = VisualTestContext::from_window(auxiliary, cx);
    view.dispatch_action(AddOrRenameMarker);
    view.run_until_parked();
    view.simulate_input("draft");
    auxiliary
        .update(&mut view, |_, window, cx| {
            owner.update(cx, |ws, cx| {
                ws.dispatch(
                    Command::Panels(PanelsCommand::Move {
                        panel: ws.app.panels.focused_id(),
                        placement: Placement::Center,
                    }),
                    Some(window),
                    cx,
                );
            });
        })
        .unwrap();
    view.run_until_parked();
    owner.read_with(&view, |ws, _| {
        assert!(ws.rename.is_none());
        assert!(ws.app.text_edit().is_none());
        assert_eq!(ws.app.doc.markers()[0].label.as_deref(), Some("draft"));
    });
    main.update(&mut view, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn auxiliary_palette_trace_rename_focuses_the_main_window_editor(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;

    init(cx);
    let (owner, main, auxiliary) = workspace_with_auxiliary_panel(cx);
    owner.update(cx, |ws, cx| {
        ws.add_path(
            concat!(env!("CARGO_MANIFEST_DIR"), "/examples/picorv32.vtr").into(),
            cx,
        );
    });
    cx.run_until_parked();
    let mut view = VisualTestContext::from_window(auxiliary, cx);
    view.simulate_keystrokes("ctrl-k");
    view.run_until_parked();
    view.simulate_input("rename trace b");
    view.run_until_parked();
    view.simulate_keystrokes("enter");
    view.run_until_parked();
    assert_eq!(view.update(|_, cx| cx.active_window()), Some(main));
    main.update(&mut view, |_, window, cx| {
        let ws = owner.read(cx);
        let rename = ws.trace_rename.as_ref().expect("trace name editor");
        assert!(rename.input.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
    let mut main_view = VisualTestContext::from_window(main, &view);
    main_view.simulate_keystrokes(&format!("{SELECT_ALL} backspace"));
    main_view.simulate_input("renamed");
    main_view.simulate_keystrokes("enter");
    main_view.run_until_parked();
    owner.read_with(&main_view, |ws, _| {
        let b = TraceId::from_letter('B').unwrap();
        assert_eq!(ws.app.trace_chip(b).unwrap().name, "renamed");
        assert!(ws.trace_rename.is_none());
    });
    auxiliary
        .update(&mut main_view, |_, window, cx| {
            owner.update(cx, |ws, cx| {
                ws.start_trace_rename(TraceId::from_letter('B').unwrap(), window, cx);
                ws.set_session(Arc::new(ProceduralTrace::new(200)), cx);
            });
        })
        .unwrap();
    main_view.run_until_parked();
    owner.read_with(&main_view, |ws, _| assert!(ws.trace_rename.is_none()));
    owner.update(&mut main_view, |ws, cx| ws.windows.close(cx));
    main.update(&mut main_view, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn switching_windows_preserves_settings_search_focus(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;

    init(cx);
    let (owner, main, auxiliary) = workspace_with_auxiliary_panel(cx);
    let mut view = VisualTestContext::from_window(main, cx);
    view.dispatch_action(OpenSettings);
    view.run_until_parked();
    let settings = owner.read_with(&view, |ws, _| ws.app.panels.settings_id().unwrap());
    for detached in [false, true] {
        if detached {
            owner.update(&mut view, |ws, cx| {
                ws.dispatch(
                    Command::Panels(volna_core::panels::PanelsCommand::MoveToNewWindow(settings)),
                    None,
                    cx,
                );
            });
            view.run_until_parked();
        }
        let host = owner.read_with(&view, |ws, _| {
            ws.windows.handle_for(settings, &ws.app.panels).unwrap()
        });
        let mut local = VisualTestContext::from_window(host, &view);
        local.dispatch_action(OpenSettings);
        local.run_until_parked();
        let search = host
            .update(&mut local, |_, window, cx| {
                owner.update(cx, |ws, cx| {
                    ws.settings_view(settings, window, cx)
                        .read(cx)
                        .search_focus(cx)
                })
            })
            .unwrap();
        local.update(|window, _| assert!(search.is_focused(window)));
        local.simulate_keystrokes(&format!("{SELECT_ALL} backspace"));
        local.simulate_input("font");
        auxiliary
            .update(&mut local, |_, window, _| window.activate_window())
            .unwrap();
        local.run_until_parked();
        host.update(&mut local, |_, window, _| window.activate_window())
            .unwrap();
        local.run_until_parked();
        local.update(|window, _| assert!(search.is_focused(window)));
        local.simulate_input(" size");
        local.run_until_parked();
        owner.read_with(&local, |ws, _| {
            assert_eq!(ws.app.settings_view.query, "font size");
        });
    }
    owner.update(&mut view, |ws, cx| ws.windows.close(cx));
    main.update(&mut view, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn auxiliary_file_drop_choosers_render_locally_and_add_or_replace_traces(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput, VisualTestContext, size};
    init(cx);
    for (dark, width, zoom) in [(true, 900.0, 1.0), (false, 360.0, 1.5)] {
        let (owner, main, auxiliary) = workspace_with_auxiliary_panel(cx);
        owner.update(cx, |_, cx| {
            crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
            crate::theme::set_zoom(zoom, cx);
        });
        let trace = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/picorv32.vtr"
        ));
        let mut view = VisualTestContext::from_window(auxiliary, cx);
        view.simulate_resize(size(px(width), px(700.0)));
        view.run_until_parked();
        let drop_trace = |view: &mut VisualTestContext| {
            view.simulate_mouse_move(
                point(px(160.0), px(180.0)),
                None,
                gpui_kit::Modifiers::default(),
            );
            view.update(|window, cx| {
                window.render_frame(cx);
                let position = point(px(160.0), px(180.0));
                window.dispatch_event(
                    PlatformInput::FileDrop(FileDropEvent::Entered {
                        position,
                        paths: ExternalPaths([trace.clone()].into_iter().collect()),
                    }),
                    cx,
                );
                window.render_frame(cx);
                window.dispatch_event(
                    PlatformInput::FileDrop(FileDropEvent::Submit { position }),
                    cx,
                );
                window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Exited), cx);
                window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Ended), cx);
            });
            view.run_until_parked();
            view.update(|window, cx| window.draw(cx).clear(cx));
            view.simulate_mouse_move(
                point(px(1.0), px(1.0)),
                None,
                gpui_kit::Modifiers::default(),
            );
            view.run_until_parked();
        };
        drop_trace(&mut view);
        let menu = owner.read_with(&view, |ws, _| {
            ws.status_menu.as_ref().unwrap().popup.clone()
        });
        let mut main_view = VisualTestContext::from_window(main, &view);
        main_view.update(|window, cx| window.draw(cx).clear(cx));
        assert!(main_view.update(|window, _| window.try_find("popup-menu").is_none()));
        let bounds = view.update(|window, _| window.find("popup-menu").bounds());
        assert!(bounds.left() >= px(0.0) && bounds.right() <= px(width));
        view.update(|window, cx| {
            assert!(menu.read(cx).focus_handle(cx).contains_focused(window, cx));
        });
        view.simulate_keystrokes("escape");
        view.run_until_parked();
        owner.read_with(&view, |ws, _| assert!(ws.status_menu.is_none()));

        drop_trace(&mut view);
        let menu = owner.read_with(&view, |ws, _| {
            ws.status_menu.as_ref().unwrap().popup.clone()
        });
        view.update(|window, cx| {
            assert!(menu.read(cx).focus_handle(cx).contains_focused(window, cx));
        });
        view.simulate_keystrokes("down");
        view.run_until_parked();
        owner.read_with(&view, |ws, _| {
            assert!(
                ws.status_menu.is_some(),
                "arrow key must keep the chooser open"
            );
        });
        view.update(|window, cx| {
            assert!(menu.read(cx).focus_handle(cx).contains_focused(window, cx));
        });
        view.simulate_keystrokes("enter");
        view.run_until_parked();
        owner.read_with(&view, |ws, _| {
            assert!(ws.status_menu.is_none());
            assert!(
                ws.app
                    .doc
                    .traces()
                    .get(TraceId::new(1).unwrap())
                    .unwrap()
                    .session()
                    .is_some()
            );
        });
        drop_trace(&mut view);
        view.simulate_keystrokes("down down enter");
        view.run_until_parked();
        owner.read_with(&view, |ws, _| {
            assert!(ws.status_menu.is_none());
            assert!(ws.app.doc.traces().get(TraceId::new(1).unwrap()).is_none());
            assert_eq!(
                ws.app.doc.traces().get(TraceId::A).unwrap().uri.as_deref(),
                Some(crate::native_workspace::file_uri(&trace).unwrap().as_str())
            );
        });
        assert_eq!(
            main.update(&mut view, |_, _, cx| cx.windows().len())
                .unwrap(),
            1
        );
        main.update(&mut view, |_, window, _| window.remove_window())
            .unwrap();
    }
}
