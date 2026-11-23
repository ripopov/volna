//! Title-bar trace controls, renaming, and adding traces by dialog, command line and drop.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Bounds, Context, CursorStyle, Entity, Focusable, FontWeight, IntoElement,
    MouseButton, Pixels, Point, SharedString, Subscription, Window, canvas, div, px,
};
use volna_core::app::{Command, TraceChip};
use volna_core::trace::TraceId;

use crate::app::Workspace;
use crate::theme::{ThemePx, core_theme, hsla, theme};
use crate::ui::text_input::TextInputEvent;
use crate::ui::{Icon, IconName, TextInput};

#[derive(Clone, Default)]
pub(crate) struct TraceChipBounds(Rc<RefCell<HashMap<TraceId, Bounds<Pixels>>>>);

impl TraceChipBounds {
    fn set(&self, trace: TraceId, bounds: Bounds<Pixels>) {
        self.0.borrow_mut().insert(trace, bounds);
    }

    fn get(&self, trace: TraceId) -> Option<Bounds<Pixels>> {
        self.0
            .borrow()
            .get(&trace)
            .copied()
            .filter(|b| b.size.width > px(0.0) && b.size.height > px(0.0))
    }
}

pub(crate) struct TraceRename {
    pub trace: TraceId,
    pub input: Entity<TextInput>,
    initial: String,
    generation: u64,
    _subscriptions: [Subscription; 2],
}

fn middle_truncate_path(path: &str, max_chars: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max_chars || max_chars < 4 {
        return path.to_owned();
    }
    // Prefer keeping the trailing path segments so deep run directories stay identifiable.
    let budget = max_chars - 1;
    let sep = if path.contains('/') {
        '/'
    } else if path.contains('\\') {
        '\\'
    } else {
        '\0'
    };
    if sep != '\0' {
        let mut tail_start = None;
        for (byte_idx, ch) in path.char_indices() {
            if ch == sep {
                let remaining = path[byte_idx..].chars().count();
                if remaining <= budget {
                    tail_start = Some(byte_idx);
                    break;
                }
            }
        }
        if let Some(idx) = tail_start {
            return format!("…{}", &path[idx..]);
        }
    }
    let tail: String = path.chars().skip(char_count - budget).collect();
    format!("…{tail}")
}

pub(crate) fn letter_badge(trace: TraceId, cx: &gpui_kit::App) -> impl IntoElement {
    let t = theme(cx);
    let colors = core_theme(cx).trace(trace);
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .w(t.px(16.0))
        .h(t.px(16.0))
        .rounded(t.px(3.0))
        .bg(hsla(colors.background))
        .text_color(hsla(colors.text))
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(t.ui_size_small))
        .child(SharedString::from(trace.letter().to_string()))
}

impl Workspace {
    fn trace_chip_detail(&self, chip: &TraceChip, full: bool) -> SharedString {
        #[cfg(not(target_family = "wasm"))]
        if let Some(progress) = self.native_imports.snapshot(chip.trace) {
            return progress
                .fraction()
                .map_or_else(
                    || "converting…".to_owned(),
                    |f| format!("converting {:.0}%", f * 100.0),
                )
                .into();
        }
        if full {
            chip.row_detail().into()
        } else {
            chip.detail().into()
        }
    }

    pub(crate) fn render_trace_chips(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let chips = self.app.trace_chips();
        if chips.is_empty() {
            return None;
        }
        let row = div()
            .id("trace-chips")
            .flex()
            .min_w_0()
            .items_center()
            .gap_1()
            .children(
                chips
                    .iter()
                    .map(|chip| self.trace_chip(chip, chips.len() > 1, cx)),
            );
        Some(row.into_any_element())
    }

    fn trace_chip(&self, chip: &TraceChip, multiple: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = *theme(cx);
        let trace = chip.trace;
        let menu_open = self
            .status_menu
            .as_ref()
            .is_some_and(|menu| menu.trace == Some(trace));
        let renaming = self.trace_rename.as_ref().filter(|r| r.trace == trace);
        let tooltip = chip.tooltip();
        let name: SharedString = chip.name.clone().into();
        let detail = self.trace_chip_detail(chip, false);
        let bounds_store = self.trace_chip_bounds.clone();

        if let Some(rename) = renaming {
            return div()
                .id(("trace-chip", trace.index()))
                .debug_selector(move || format!("trace-chip-{trace}"))
                .relative()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1()
                .h(t.px(28.0))
                .px(t.px(6.0))
                .rounded(t.px(4.0))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down_out(
                    cx.listener(|this, _: &gpui_kit::MouseDownEvent, window, cx| {
                        this.commit_trace_rename(window, cx);
                    }),
                )
                .when(multiple, |el| el.child(letter_badge(trace, cx)))
                .child(
                    div()
                        .w(t.px(160.0))
                        .h(t.px(20.0))
                        .child(rename.input.clone()),
                )
                .child(
                    canvas(
                        move |bounds, _, _| bounds_store.set(trace, bounds),
                        |_, (), _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .into_any_element();
        }

        let chevron_color = if menu_open {
            t.bar.icon
        } else {
            t.bar.icon_muted
        };

        div()
            .id(("trace-chip", trace.index()))
            .debug_selector(move || format!("trace-chip-{trace}"))
            .relative()
            .flex()
            .min_w_0()
            .items_center()
            .gap_1()
            .h(t.px(28.0))
            .pl(t.px(8.0))
            .pr(t.px(6.0))
            .rounded(t.px(4.0))
            .when(menu_open, |el| el.bg(t.bar_hover.bg))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(t.bar_hover.bg))
            .when(chip.loading, |el| el.opacity(0.6))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(
                cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if this
                        .status_menu
                        .as_ref()
                        .is_some_and(|menu| menu.trace == Some(trace))
                    {
                        this.status_menu = None;
                        cx.notify();
                        return;
                    }
                    let position = this.trace_chip_bounds.get(trace).map_or_else(
                        || event.position(),
                        |bounds| Point::new(bounds.left(), bounds.bottom() + theme(cx).px(4.0)),
                    );
                    this.open_trace_menu(trace, position, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &gpui_kit::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_trace_menu(trace, ev.position, window, cx);
                }),
            )
            .when(!menu_open, |el| {
                el.tooltip_show_delay(crate::ui::tooltip::delay(cx))
                    .tooltip(move |w, cx| Tooltip::new(tooltip.clone()).build(w, cx))
            })
            .when(multiple, |el| el.child(letter_badge(trace, cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .max_w(t.px(180.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(t.bar.text)
                            .child(name),
                    )
                    .when(multiple || chip.loading, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .text_size(px(t.ui_size_small))
                                .text_color(t.bar.text_muted)
                                .child(detail),
                        )
                    }),
            )
            .child(
                Icon::new(IconName::ChevronDown)
                    .size(t.px(12.0))
                    .color(chevron_color),
            )
            .child(
                canvas(
                    move |bounds, _, _| bounds_store.set(trace, bounds),
                    |_, (), _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }

    pub(crate) fn open_trace_menu(
        &mut self,
        trace: TraceId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_trace_rename(window, cx);
        let Some(chip) = self.app.trace_chip(trace) else {
            return;
        };
        let multiple = self.app.trace_chips().len() > 1;
        let detail = self.trace_chip_detail(&chip, true);
        let workspace = cx.entity().downgrade();
        let run = move |f: fn(&mut Workspace, TraceId, &mut Window, &mut Context<Workspace>)| {
            let workspace = workspace.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                workspace
                    .update(cx, |this, cx| {
                        this.status_menu = None;
                        f(this, trace, window, cx);
                    })
                    .ok();
            }
        };
        let focus = self.window_focus(window);
        let t = *theme(cx);
        let width = t
            .px(300.0)
            .min((window.viewport_size().width - t.px(16.0)).max(px(0.0)));
        let content_width = (width - t.px(32.0)).max(px(0.0));
        let copy_workspace = cx.entity().downgrade();
        let has_source = !chip.source.is_empty();
        let display_path: SharedString = middle_truncate_path(&chip.source, 42).into();
        let full_path: SharedString = chip.source.clone().into();
        let header_name: SharedString = chip.name.clone().into();
        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            let close = if !chip.closable {
                PopupMenuItem::new("Close All Traces").on_click(run(|this, _, w, cx| {
                    this.dispatch(Command::CloseTrace, Some(w), cx);
                }))
            } else if multiple {
                PopupMenuItem::new(format!("Close Trace {trace}")).on_click(run(
                    |this, trace, w, cx| this.dispatch(Command::RemoveTrace(trace), Some(w), cx),
                ))
            } else {
                PopupMenuItem::new("Close Trace").on_click(run(|this, trace, w, cx| {
                    this.dispatch(Command::RemoveTrace(trace), Some(w), cx);
                }))
            };
            let mut menu = menu
                .item(
                    PopupMenuItem::element({
                        let header_name = header_name.clone();
                        let detail = detail.clone();
                        let display_path = display_path.clone();
                        let full_path = full_path.clone();
                        move |_, cx| {
                            let t = theme(cx);
                            div()
                                .id("trace-menu-details")
                                .w(content_width)
                                .flex()
                                .flex_col()
                                .gap(t.px(2.0))
                                .py(t.px(2.0))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(t.px(6.0))
                                        .min_w_0()
                                        .when(multiple, |el| el.child(letter_badge(trace, cx)))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .flex_1()
                                                .truncate()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(t.panel.text)
                                                .child(header_name.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(t.ui_size_small))
                                        .text_color(t.panel.text_muted)
                                        .child(detail.clone()),
                                )
                                .when(has_source, |el| {
                                    let tooltip_path = full_path.clone();
                                    el.child(
                                        div()
                                            .id("trace-menu-path")
                                            .truncate()
                                            .font_family(t.mono_font)
                                            .text_size(px(t.ui_size_small))
                                            .text_color(t.panel.text_muted)
                                            .aria_label(full_path.clone())
                                            .child(display_path.clone())
                                            .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                                            .tooltip(move |w, cx| {
                                                Tooltip::new(tooltip_path.clone()).build(w, cx)
                                            }),
                                    )
                                })
                        }
                    })
                    .disabled(true),
                )
                .separator();
            if has_source {
                menu = menu.item(PopupMenuItem::new("Copy File Path").on_click({
                    let path = chip.source.clone();
                    let workspace = copy_workspace.clone();
                    move |_, _, cx| {
                        crate::clipboard::copy(path.as_str(), cx);
                        workspace
                            .update(cx, |this, cx| {
                                this.status_menu = None;
                                cx.notify();
                            })
                            .ok();
                    }
                }));
            }
            menu = menu
                .item(
                    PopupMenuItem::new("Reveal in Scopes").on_click(run(|this, trace, w, cx| {
                        this.dispatch(Command::RevealTrace(trace), Some(w), cx);
                    })),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Rename Trace")
                        .on_click(run(super::app::Workspace::start_trace_rename)),
                );
            if chip.renamed {
                menu = menu.item(PopupMenuItem::new("Use File Name").on_click(run(
                    |this, trace, w, cx| {
                        this.dispatch(Command::RenameTrace(trace, None), Some(w), cx);
                    },
                )));
            }
            menu.separator()
                .item(
                    PopupMenuItem::new("Add Trace…").on_click(run(|this, _, w, cx| {
                        this.dispatch(Command::RequestAddTraceDialog, Some(w), cx);
                    })),
                )
                .item(close)
                .min_w(width)
                .max_w(width)
                .action_context(focus)
        });
        self.show_status_menu(position, menu, window, cx);
        if let Some(menu) = self.status_menu.as_mut() {
            menu.trace = Some(trace);
        }
    }

    pub(crate) fn start_trace_rename(
        &mut self,
        trace: TraceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(not(target_family = "wasm"))]
        let target = self.windows.main();
        #[cfg(target_family = "wasm")]
        let target = window.window_handle();
        self.run_in_window(target, window, cx, move |ws, window, cx| {
            ws.edit_trace_name(trace, window, cx);
            #[cfg(not(target_family = "wasm"))]
            if !window.is_window_active() {
                window.activate_window();
            }
        });
    }

    pub(crate) fn commit_trace_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.trace_rename.take() else {
            return;
        };
        let name = rename.input.read(cx).text().to_owned();
        if name != rename.initial {
            self.dispatch_if_current(
                rename.generation,
                Command::RenameTrace(rename.trace, Some(name)),
                Some(window),
                cx,
            );
        }
        cx.notify();
    }

    fn edit_trace_name(&mut self, trace: TraceId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chip) = self.app.trace_chip(trace) else {
            return;
        };
        self.status_menu = None;
        let initial = chip.name.clone();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(format!("Name of trace {trace}"), cx).plain();
            input.set_text(chip.name.clone(), cx);
            input.select_all(cx);
            input
        });
        let generation = self.app.doc.generation();
        let input_focus = input.read(cx).focus_handle(cx);
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &TextInputEvent, window, cx| {
                let name = match event {
                    TextInputEvent::Submit => Some(input.read(cx).text().to_owned()),
                    TextInputEvent::Cancel => None,
                    TextInputEvent::Changed => return,
                };
                this.trace_rename = None;
                if let Some(name) = name {
                    this.dispatch_if_current(
                        generation,
                        Command::RenameTrace(trace, Some(name)),
                        Some(window),
                        cx,
                    );
                }
                window.focus(&this.focus_handle, cx);
                cx.notify();
            },
        );
        window.focus(&input_focus, cx);
        let blur = cx.on_blur(&input_focus, window, |this, window, cx| {
            this.commit_trace_rename(window, cx);
        });
        self.trace_rename = Some(TraceRename {
            trace,
            input,
            initial,
            generation,
            _subscriptions: [subscription, blur],
        });
        cx.notify();
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn add_path(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        if !self.app.doc.is_loaded() && self.app.doc.traces().is_empty() {
            self.open_path(path, cx);
            return;
        }
        match path
            .canonicalize()
            .map_err(anyhow::Error::from)
            .and_then(|path| Ok((crate::native_workspace::file_uri(&path)?, path)))
        {
            Ok((uri, path)) => self
                .app
                .add_resource(volna_trace::session::OpenSpec::Path(path), uri),
            Err(_) => self
                .app
                .handle(Command::AddTrace(volna_trace::session::OpenSpec::Path(
                    path,
                ))),
        }
        self.after(None, cx);
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn open_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let mut paths = paths.into_iter();
        if let Some(first) = paths.next() {
            self.open_path(first, cx);
        }
        for path in paths {
            self.add_path(path, cx);
        }
    }

    #[cfg_attr(
        not(target_family = "wasm"),
        expect(
            clippy::unused_self,
            reason = "The web implementation updates the workspace directly; the native dialog uses its entity context."
        )
    )]
    pub(crate) fn add_trace_dialog(&mut self, cx: &mut Context<Self>) {
        #[cfg(not(target_family = "wasm"))]
        {
            let rx = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: Some("Add".into()),
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(paths))) = rx.await {
                    this.update(cx, |this, cx| {
                        for path in paths {
                            this.add_path(path, cx);
                        }
                    })
                    .ok();
                }
            })
            .detach();
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = cx;
            self.app.handle(Command::Notice(
                "Adding a trace is not available in this host yet.".into(),
            ));
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn drop_paths(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let workspaces = paths
            .iter()
            .any(|p| crate::native_workspace::is_workspace_path(p));
        if self.app.doc.traces().is_empty() || workspaces {
            self.open_paths(paths, cx);
            return;
        }
        let name = |p: &std::path::Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let what = match paths.as_slice() {
            [one] => name(one),
            many => format!("{} traces", many.len()),
        };
        let add_label = match (paths.len(), self.app.doc.traces().free_id()) {
            (1, Some(letter)) => format!("Add {what} as {letter}"),
            _ => format!("Add {what}"),
        };
        let workspace = cx.entity().downgrade();
        let choose = move |add: bool, paths: Vec<std::path::PathBuf>| {
            let workspace = workspace.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                let paths = paths.clone();
                workspace
                    .update(cx, |this, cx| {
                        this.status_menu = None;
                        if add {
                            for path in paths {
                                this.add_path(path, cx);
                            }
                        } else {
                            this.open_paths(paths, cx);
                        }
                    })
                    .ok();
            }
        };
        let focus = self.window_focus(window);
        let min_w = theme(cx).px(200.0);
        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            menu.item(PopupMenuItem::new(add_label.clone()).on_click(choose(true, paths.clone())))
                .item(
                    PopupMenuItem::new(format!("Open {what} Instead"))
                        .on_click(choose(false, paths.clone())),
                )
                .min_w(min_w)
                .action_context(focus)
        });
        self.show_status_menu(position, menu, window, cx);
    }
}
