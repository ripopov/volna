//! Timing range controls and full-range stage lifeline for Record Details.

use super::*;

impl TransactionPanelView {
    pub(super) fn inline_time(
        t: &crate::theme::Theme,
        label: &'static str,
        text: String,
        tooltip_text: &'static str,
        command: Option<TransactionCommand>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chip = div()
            .flex()
            .items_center()
            .gap_1()
            .px_1p5()
            .py_0p5()
            .rounded(t.px(4.0))
            .child(
                div()
                    .text_size(px(t.ui_size_small))
                    .text_color(t.panel.text_muted)
                    .child(label),
            )
            .child(
                div()
                    .font_family(t.mono_font)
                    .text_size(px(t.ui_size_small))
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .child(SharedString::from(text)),
            );
        match command {
            Some(command) => chip
                .id(SharedString::from(format!("tx-time-{label}")))
                .debug_selector(move || format!("tx-time-{label}"))
                .tab_stop(true)
                .cursor_pointer()
                .hover(|style| style.bg(t.hover.bg))
                .focus_visible(|style| style.border_color(t.border_focused))
                .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                .tooltip(move |window, cx| Tooltip::new(tooltip_text).build(window, cx))
                .on_key_down({
                    let command = command.clone();
                    cx.listener(move |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            view.dispatch(command.clone(), window, cx);
                            cx.stop_propagation();
                        }
                    })
                })
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.dispatch(command.clone(), window, cx);
                }))
                .into_any_element(),
            None => chip.into_any_element(),
        }
    }

    pub(super) fn tile(
        t: &crate::theme::Theme,
        label: &'static str,
        text: String,
        tooltip_text: &'static str,
        command: Option<TransactionCommand>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tile = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_0p5()
            .px_2()
            .py_1()
            .rounded(t.px(4.))
            .bg(t.islands.map_or(t.panel.bg, |i| i.heading))
            .border_1()
            .border_color(t.border_variant)
            .child(
                div()
                    .text_size(px(t.ui_size_small))
                    .text_color(t.panel.text_muted)
                    .child(label),
            )
            .child(
                detail_text(cx, format!("tx-time-value-{label}"), text).font_family(t.mono_font),
            );
        match command {
            Some(command) => tile
                .id(SharedString::from(format!("tx-time-{label}")))
                .debug_selector(move || format!("tx-time-{label}"))
                .tab_stop(true)
                .cursor_pointer()
                .hover(|style| style.bg(t.hover.bg))
                .focus_visible(|style| style.border_color(t.border_focused))
                .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                .tooltip(move |window, cx| Tooltip::new(tooltip_text).build(window, cx))
                .on_key_down({
                    let command = command.clone();
                    cx.listener(move |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            view.dispatch(command.clone(), window, cx);
                            cx.stop_propagation();
                        }
                    })
                })
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.dispatch(command.clone(), window, cx);
                }))
                .into_any_element(),
            None => tile.into_any_element(),
        }
    }

    pub(super) fn lifeline(
        &self,
        t: &crate::theme::Theme,
        view: &TxView,
        scale: LifelineScale,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut lanes = div()
            .debug_selector(|| "tx-lifeline".into())
            .flex()
            .flex_col()
            .gap_0p5()
            .relative();
        let selected_id = view.stages.rows.iter().find(|r| r.selected).map(|r| r.id);
        let mut active_readout: Option<String> = None;
        for (index, lane) in view.lifeline.iter().enumerate() {
            let mut row = div()
                .relative()
                .w_full()
                .h(t.px(LANE_PX))
                .bg(t.panel.bg)
                .rounded_sm();
            for (cell_ix, cell) in lane.cells.iter().enumerate() {
                let begin = cell.begin;
                let stage_idx = cell.stage;
                let stage_id = cell.stage_id;
                let is_hovered = stage_idx.is_some() && self.hovered_stage == stage_idx;
                let is_selected = stage_id.is_some() && selected_id == stage_id;
                if is_hovered || (active_readout.is_none() && is_selected) {
                    let pct = (cell.share * 100.0).clamp(0.0, 100.0);
                    let pct_text = if pct < 0.1 && cell.duration > 0 {
                        "<0.1%".to_owned()
                    } else if pct < 10.0 {
                        format!("{pct:.1}%")
                    } else {
                        format!("{pct:.0}%")
                    };
                    active_readout = Some(format!(
                        "{} · {} · {} ({pct_text})",
                        cell.name, cell.offset_text, cell.duration_text
                    ));
                }
                let style = crate::theme::core_theme(cx).swatch(cell.swatch);
                let tooltip_label = if cell.name.is_empty() {
                    cell.duration_text.clone()
                } else {
                    format!(
                        "{} · {} · {} ({:.1}%)",
                        cell.name,
                        cell.offset_text,
                        cell.duration_text,
                        cell.share * 100.0
                    )
                };
                row = row.child(
                    div()
                        .id(SharedString::from(format!("tx-cell-{index}-{cell_ix}")))
                        .debug_selector(move || format!("tx-cell-{index}-{cell_ix}"))
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(cell.start))
                        .w(relative(cell.width.max(0.004)))
                        .bg(hsla(style.fill))
                        .border_1()
                        .border_color(if is_selected || is_hovered {
                            t.border_focused
                        } else {
                            hsla(style.edge)
                        })
                        .cursor_pointer()
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(t.ui_size_small))
                        .text_color(hsla(style.text))
                        .font_family(t.mono_font)
                        .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                        .tooltip(move |w, cx| Tooltip::new(tooltip_label.clone()).build(w, cx))
                        .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                            if *hovered {
                                if view.hovered_stage != stage_idx {
                                    view.hovered_stage = stage_idx;
                                    cx.notify();
                                }
                            } else if view.hovered_stage == stage_idx {
                                view.hovered_stage = None;
                                cx.notify();
                            }
                        }))
                        .child(
                            div()
                                .min_w_0()
                                .text_ellipsis()
                                .child(SharedString::from(cell.name.clone())),
                        )
                        .on_click(cx.listener(move |view, _, window, cx| {
                            if let Some(id) = stage_id {
                                view.dispatch(TransactionCommand::SelectStage(id), window, cx);
                            } else {
                                view.dispatch(TransactionCommand::Cursor(begin), window, cx);
                            }
                        })),
                );
            }
            lanes = lanes.child(row);
        }
        for (index, tick) in view.event_ticks.iter().enumerate() {
            let time = tick.time;
            lanes = lanes.child(
                div()
                    .id(("tx-event-tick", index))
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(tick.at))
                    .w(t.px(2.0))
                    .bg(t.wave_event_coalesced)
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.dispatch(TransactionCommand::Cursor(time), window, cx);
                    })),
            );
        }
        if let Some(at) = view.cursor {
            lanes = lanes.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(at))
                    .w(px(1.0))
                    .bg(t.wave_cursor),
            );
        }
        let controls = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .debug_selector(|| "tx-lifeline-readout".into())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(t.mono_font)
                    .text_size(px(t.ui_size_small))
                    .text_color(if active_readout.is_some() {
                        t.editor.text
                    } else {
                        t.panel.text_muted
                    })
                    .child(SharedString::from(active_readout.unwrap_or_else(|| {
                        if view.stages.total == 0 {
                            "No stages recorded".to_owned()
                        } else {
                            format!("{} stages · hover or click to inspect", view.stages.total)
                        }
                    }))),
            )
            .when(view.stages.total > 1, |bar| {
                bar.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_0p5()
                        .child(
                            Button::new("tx-lifeline-linear")
                                .debug_selector(|| "tx-lifeline-linear".into())
                                .label("Linear")
                                .ghost()
                                .small()
                                .selected(scale == LifelineScale::Linear)
                                .tooltip("Proportional time scale")
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.dispatch(
                                        TransactionCommand::LifelineScale(LifelineScale::Linear),
                                        window,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("tx-lifeline-balanced")
                                .debug_selector(|| "tx-lifeline-balanced".into())
                                .label("Balanced")
                                .ghost()
                                .small()
                                .selected(scale == LifelineScale::Balanced)
                                .tooltip("Log-scaled so short stages remain clickable")
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.dispatch(
                                        TransactionCommand::LifelineScale(LifelineScale::Balanced),
                                        window,
                                        cx,
                                    );
                                })),
                        ),
                )
            });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(lanes)
            .child(controls)
            .into_any_element()
    }
}
