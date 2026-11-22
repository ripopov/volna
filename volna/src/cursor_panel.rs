//! The At Cursor panel: virtualized list of transactions active at the timeline cursor.

use gpui_kit::base::dock as base;
use gpui_kit::component::{dock::Panel, menu::PopupMenu, scroll::Scrollbar, tooltip::Tooltip};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    ScrollStrategy, SharedString, UniformListScrollHandle, WeakEntity, Window, div, px,
    uniform_list,
};
use volna_core::Command;
use volna_core::panels::{PanelId, PanelsCommand};
use volna_core::trace::Traced;
use volna_core::transaction::cursor::{CursorRecords, CursorRecordsState};
use volna_trace::data::transactions::{TrackRef, TransactionRef};

use crate::app::Workspace;
use crate::theme::{ThemePx, theme};

#[derive(Clone, Copy, PartialEq)]
struct CursorListKey {
    track: Traced<TrackRef>,
    time: u64,
    selected: Option<TransactionRef>,
    zoom: u32,
}

pub(crate) struct CursorRecordsPanelView {
    ws: WeakEntity<Workspace>,
    id: PanelId,
    generation: u64,
    pub(crate) focus: FocusHandle,
    scroll: UniformListScrollHandle,
    synced: Option<CursorListKey>,
}

impl CursorRecordsPanelView {
    pub(crate) fn new(
        ws: WeakEntity<Workspace>,
        id: PanelId,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        cx.on_focus_in(&focus, window, |view, window, cx| {
            view.send(Command::Panels(PanelsCommand::Focus(view.id)), window, cx);
        })
        .detach();
        Self {
            ws,
            id,
            generation,
            focus,
            scroll: UniformListScrollHandle::new(),
            synced: None,
        }
    }

    fn send(&self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        drop(self.ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(self.generation, command, Some(window), cx);
        }));
    }

    fn cursor_records(&self, cx: &App) -> CursorRecordsState {
        self.ws.upgrade().map_or(CursorRecordsState::Hidden, |ws| {
            ws.read(cx).app.cursor_records_panel()
        })
    }

    fn move_cursor_record(&self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.cursor_records(cx).ready() else {
            return;
        };
        if query.count == 0 {
            return;
        }
        let rank = query.selected_rank.map_or(0, |rank| {
            rank.saturating_add_signed(delta).min(query.count - 1)
        });
        if let Some(record) = query.page(rank..rank + 1).first() {
            self.send(
                Command::CursorSelect {
                    track: query.track,
                    id: record.id,
                    time: query.time,
                },
                window,
                cx,
            );
        }
    }

    fn cursor_rows(
        query: &CursorRecords,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = *theme(cx);
        let (track, time, selected) = (query.track, query.time, query.selected);
        query
            .page(range)
            .into_iter()
            .map(|record| {
                let id = record.id;
                let row_number = record.sub_row + 1;
                let visibility = if record.hidden {
                    "Outside displayed waveform rows"
                } else {
                    "Visible in waveform"
                };
                let hint = format!(
                    "#{} · row {} · {}\n[{}, {}) ticks\n{}",
                    id.0, row_number, visibility, record.begin, record.end, record.label
                );
                let is_selected = selected == Some(id);
                div()
                    .id(("tx-active-record", id.0))
                    .role(gpui_kit::accesskit::Role::Button)
                    .aria_label(format!(
                        "Transaction #{}: {}, row {}, {}{}",
                        id.0,
                        record.label,
                        row_number,
                        visibility,
                        if is_selected { ", selected" } else { "" },
                    ))
                    .debug_selector(move || format!("tx-active-record-{}", id.0))
                    .h(t.px(32.0))
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .cursor_pointer()
                    .bg(if is_selected {
                        t.selection.bg
                    } else if record.hidden {
                        t.panel.bg
                    } else {
                        t.panel.bg.blend(t.wave_row_hover)
                    })
                    .when(!is_selected, |row| row.hover(|style| style.bg(t.hover.bg)))
                    .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                    .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        window.focus(&view.focus, cx);
                        view.send(Command::CursorSelect { track, id, time }, window, cx);
                    }))
                    .child(
                        div()
                            .w(t.px(3.0))
                            .h(t.px(16.0))
                            .flex_none()
                            .rounded_sm()
                            .bg(if record.failed {
                                t.editor.error
                            } else if is_selected {
                                t.border_focused
                            } else {
                                t.border_variant
                            }),
                    )
                    .child(
                        div()
                            .font_family(t.mono_font)
                            .text_size(px(t.ui_size_small))
                            .text_color(t.panel.text_muted)
                            .flex_none()
                            .w(t.px(66.0))
                            .truncate()
                            .child(SharedString::from(format!("#{}", id.0))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .font_family(t.mono_font)
                            .text_color(if record.failed {
                                t.editor.error
                            } else {
                                t.editor.text
                            })
                            .child(SharedString::from(if record.label.is_empty() {
                                "(no label)".into()
                            } else {
                                record.label
                            })),
                    )
                    .into_any_element()
            })
            .collect()
    }
}

impl EventEmitter<base::PanelEvent> for CursorRecordsPanelView {}
impl Focusable for CursorRecordsPanelView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl base::Panel for CursorRecordsPanelView {
    fn panel_name(&self) -> &'static str {
        "volna.cursor"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn dump(&self, _: &App) -> base::PanelState {
        let mut state = base::PanelState::new("volna.cursor");
        state.info = base::PanelInfo::panel(serde_json::json!({"id": self.id.0}));
        state
    }
}
impl Panel for CursorRecordsPanelView {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::dock::tab_title(&self.ws, self.id, Some(self.generation), cx)
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
    fn dropdown_menu(
        &mut self,
        menu: PopupMenu,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> PopupMenu {
        crate::dock::panel_menu(menu, &self.ws, self.id, self.generation, cx)
    }
}

impl Render for CursorRecordsPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme(cx);
        let records = self.cursor_records(cx);
        let body: AnyElement = match records {
            CursorRecordsState::Ready(query) => {
                let time_text = self.ws.upgrade().map_or_else(
                    || query.time.to_string(),
                    |ws| {
                        volna_core::wave::timeline::format_time(
                            query.time as f64,
                            ws.read(cx).app.doc.time_base(),
                        )
                    },
                );
                let key = CursorListKey {
                    track: query.track,
                    time: query.time,
                    selected: query.selected,
                    zoom: t.zoom.to_bits(),
                };
                if self.synced != Some(key) {
                    self.scroll
                        .scroll_to_item(query.selected_rank.unwrap_or(0), ScrollStrategy::Nearest);
                    self.synced = Some(key);
                }
                let count = query.count;
                let outside = query.selected.is_some() && query.selected_rank.is_none();
                let mut section =
                    div()
                        .id(("tx-cursor-context", self.id.0))
                        .role(gpui_kit::accesskit::Role::Group)
                        .aria_label(format!(
                            "{count} transactions at cursor. Up and Down select records."
                        ))
                        .debug_selector(|| "tx-cursor-records".into())
                        .size_full()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .px_3()
                                .py_1()
                                .border_b_1()
                                .border_color(t.border_variant)
                                .text_size(px(t.ui_size_small))
                                .text_color(t.panel.text_muted)
                                .child(div().min_w_0().flex_1().text_ellipsis().child(
                                    SharedString::from(format!("At cursor · {count} active")),
                                ))
                                .child(
                                    div()
                                        .flex_none()
                                        .font_family(t.mono_font)
                                        .child(SharedString::from(time_text)),
                                ),
                        );
                if count == 0 {
                    section = section.child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .px_3()
                            .py_2()
                            .text_size(px(t.ui_size_small))
                            .text_color(t.panel.text_muted)
                            .child("No transactions active at this time"),
                    );
                } else {
                    section = section.child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .debug_selector(|| "tx-cursor-list-viewport".into())
                            .w_full()
                            .min_w_0()
                            .child(
                                uniform_list(
                                    ("tx-active-list", self.id.0),
                                    count,
                                    cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                                        Self::cursor_rows(&query, range, cx)
                                    }),
                                )
                                .track_scroll(&self.scroll)
                                .size_full(),
                            )
                            .child(Scrollbar::vertical(&self.scroll)),
                    );
                }
                if outside {
                    section = section.child(
                        div()
                            .px_3()
                            .py_1()
                            .border_t_1()
                            .border_color(t.border_variant)
                            .text_size(px(t.ui_size_small))
                            .text_color(t.panel.text_muted)
                            .flex_none()
                            .text_ellipsis()
                            .child("Selected record is outside the cursor’s active set"),
                    );
                }
                section.into_any_element()
            }
            state => {
                let (message, color) = match state {
                    CursorRecordsState::Hidden => (
                        "Place the cursor on a transaction track to inspect overlapping records.",
                        t.panel.text_muted,
                    ),
                    CursorRecordsState::Preparing => {
                        ("Preparing transactions at cursor…", t.panel.text_muted)
                    }
                    CursorRecordsState::Unavailable => (
                        "Transaction summary unavailable. Raise memory limits to retry.",
                        t.editor.error,
                    ),
                    CursorRecordsState::Ready(_) => unreachable!(),
                };
                div()
                    .debug_selector(|| "tx-cursor-records".into())
                    .size_full()
                    .overflow_hidden()
                    .px_3()
                    .py_2()
                    .text_size(px(t.ui_size_small))
                    .text_color(color)
                    .child(message)
                    .into_any_element()
            }
        };
        div()
            .id(("cursor-panel", self.id.0))
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(t.panel.bg)
            .font_family(t.ui_font)
            .text_size(px(t.ui_size))
            .text_color(t.editor.text)
            .track_focus(&self.focus)
            .on_key_down(
                cx.listener(|view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    let key = event.keystroke.key.as_str();
                    if view.focus.is_focused(window) && (key == "up" || key == "down") {
                        view.move_cursor_record(if key == "up" { -1 } else { 1 }, window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .child(body)
            .into_any_element()
    }
}
