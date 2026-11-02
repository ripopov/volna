//! Virtual cursor records share the existing record details and selection commands.

use super::{TransactionCommand, TransactionPanelView};
use crate::theme::{ThemePx, theme};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, SharedString, Window, div, px, uniform_list};
use gpui_kit::{ScrollStrategy, UniformListScrollHandle};
use volna_core::{
    trace::Traced,
    transaction::cursor::{CursorRecords, CursorRecordsState},
};
use volna_trace::data::transactions::{TrackRef, TransactionRef};

#[derive(Default)]
pub(super) struct CursorList {
    scroll: UniformListScrollHandle,
    synced: Option<(Traced<TrackRef>, u64, Option<TransactionRef>, u32)>,
}

impl TransactionPanelView {
    fn cursor_records(&self, cx: &App) -> CursorRecordsState {
        self.ws.upgrade().map_or(CursorRecordsState::Hidden, |ws| {
            ws.read(cx).app.cursor_records(self.id)
        })
    }

    pub(super) fn render_cursor_records(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let query = match self.cursor_records(cx) {
            CursorRecordsState::Ready(query) => query,
            CursorRecordsState::Hidden => return div().into_any_element(),
            state => {
                let t = *theme(cx);
                let unavailable = matches!(state, CursorRecordsState::Unavailable);
                return div()
                    .px_3()
                    .py_2()
                    .flex_none()
                    .text_size(px(t.ui_size_small))
                    .text_color(if unavailable {
                        t.editor.error
                    } else {
                        t.panel.text_muted
                    })
                    .child(if unavailable {
                        "Transaction summary unavailable. Raise memory limits to retry."
                    } else {
                        "Preparing transactions at cursor…"
                    })
                    .into_any_element();
            }
        };
        let t = *theme(cx);
        let time_text = self.ws.upgrade().map_or_else(
            || query.time.to_string(),
            |ws| {
                volna_core::wave::timeline::format_time(
                    query.time as f64,
                    ws.read(cx).app.doc.time_base(),
                )
            },
        );
        let key = (query.track, query.time, query.selected, t.zoom.to_bits());
        if self.cursor_list.synced != Some(key) {
            self.cursor_list
                .scroll
                .scroll_to_item(query.selected_rank.unwrap_or(0), ScrollStrategy::Nearest);
            self.cursor_list.synced = Some(key);
        }
        let count = query.count;
        let outside = query.selected.is_some() && query.selected_rank.is_none();
        let mut section = div()
            .id(("tx-cursor-context", self.id.0))
            .role(gpui_kit::accesskit::Role::Group)
            .aria_label(format!(
                "{count} transactions at cursor. Up and Down select records."
            ))
            .debug_selector(|| "tx-cursor-records".into())
            .w_full()
            .min_w_0()
            .flex_none()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(t.border_variant)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .text_size(px(t.ui_size_small))
                    .text_color(t.panel.text_muted)
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .text_ellipsis()
                            .child(SharedString::from(format!("At cursor · {count} active"))),
                    )
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
                    .px_3()
                    .py_2()
                    .text_size(px(t.ui_size_small))
                    .text_color(t.panel.text_muted)
                    .child("No transactions at this time"),
            );
        } else {
            section = section.child(
                div()
                    .relative()
                    .h(t.px(34.0 * count.min(4) as f32))
                    .w_full()
                    .min_w_0()
                    .flex_none()
                    .child(
                        uniform_list(
                            ("tx-active-list", self.id.0),
                            count,
                            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                                Self::cursor_rows(&query, range, cx)
                            }),
                        )
                        .track_scroll(&self.cursor_list.scroll)
                        .size_full(),
                    )
                    .child(Scrollbar::vertical(&self.cursor_list.scroll)),
            );
        }
        if outside {
            section = section.child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(t.ui_size_small))
                    .text_color(t.panel.text_muted)
                    .child("Selected record is outside the cursor’s active set"),
            );
        }
        section.into_any_element()
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
                let hint = format!(
                    "#{} · [{}, {}) ticks\n{}",
                    id.0, record.begin, record.end, record.label
                );
                let is_selected = selected == Some(id);
                div()
                    .id(("tx-active-record", id.0))
                    .role(gpui_kit::accesskit::Role::Button)
                    .aria_label(format!(
                        "Transaction #{}: {}{}",
                        id.0,
                        record.label,
                        if is_selected { ", selected" } else { "" }
                    ))
                    .debug_selector(move || format!("tx-active-record-{}", id.0))
                    .h(t.px(34.0))
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .cursor_pointer()
                    .bg(if is_selected {
                        t.selection.bg
                    } else {
                        t.panel.bg
                    })
                    .hover(|style| style.bg(t.hover.bg))
                    .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        window.focus(&view.focus, cx);
                        view.dispatch(
                            TransactionCommand::CursorSelect { track, id, time },
                            window,
                            cx,
                        );
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
                            .max_w(t.px(100.0))
                            .text_ellipsis()
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

    pub(super) fn move_cursor_record(
        &self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            self.dispatch(
                TransactionCommand::CursorSelect {
                    track: query.track,
                    id: record.id,
                    time: query.time,
                },
                window,
                cx,
            );
        }
    }
}
