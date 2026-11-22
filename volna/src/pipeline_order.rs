//! Toolbar **Stages & Order** popover for single-source and merged pipeline panels.

use std::hash::{DefaultHasher, Hash, Hasher};

use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonCustomVariant, ButtonVariants},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, Context, CursorStyle, Empty, Entity, FocusHandle, IntoElement, Pixels, Point, Render,
    SharedString, WeakEntity, Window, anchored, deferred, div, point, px,
};
use volna_core::app::Command;
use volna_core::panels::PanelId;
use volna_core::pipeline::{
    RowOrder, RowOrderCommand, RowOrderMode, StageOrderOption, StagePalette, StageVisibility,
    StageVisibilityCommand, UnmatchedPolicy,
};

use crate::app::Workspace;
use crate::theme::{ThemePx, canvas_theme, hsla, theme};
use crate::ui::{Icon, IconName};

pub(crate) struct PipelineOrderView {
    ws: WeakEntity<Workspace>,
    panel: PanelId,
    generation: u64,
    open: bool,
    anchor_pos: Option<Point<Pixels>>,
    dismissed_at: Option<Point<Pixels>>,
    last_fingerprint: u64,
    focus: FocusHandle,
}

impl PipelineOrderView {
    pub(crate) fn new(
        ws: WeakEntity<Workspace>,
        panel: PanelId,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        if let Some(owner) = ws.upgrade() {
            cx.observe(&owner, |view, owner, cx| {
                let fp = view.compute_fingerprint(&owner, cx);
                if fp != view.last_fingerprint {
                    view.last_fingerprint = fp;
                    cx.notify();
                }
            })
            .detach();
        }
        Self {
            ws,
            panel,
            generation,
            open: false,
            anchor_pos: None,
            dismissed_at: None,
            last_fingerprint: 0,
            focus: cx.focus_handle(),
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn toggle_at(
        &mut self,
        pos: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.open
            && let Some(dismissed) = self.dismissed_at.take()
            && (f32::from(dismissed.x) - f32::from(pos.x)).abs() <= 6.0
            && (f32::from(dismissed.y) - f32::from(pos.y)).abs() <= 6.0
        {
            return;
        }
        self.dismissed_at = None;
        self.open = !self.open;
        if self.open {
            self.anchor_pos = Some(pos);
            window.focus(&self.focus, cx);
        }
        self.last_fingerprint = 0;
        cx.notify();
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            self.last_fingerprint = 0;
            cx.notify();
        }
    }

    fn compute_fingerprint(&self, owner: &Entity<Workspace>, cx: &Context<Self>) -> u64 {
        let app = &owner.read(cx).app;
        let mut h = DefaultHasher::new();
        self.open.hash(&mut h);
        let Some(pipeline) = app.panels.pipeline(self.panel) else {
            0u8.hash(&mut h);
            return h.finish();
        };
        1u8.hash(&mut h);
        pipeline.order.hash(&mut h);
        pipeline.stage_visibility.hash(&mut h);
        pipeline.suggested_order_stage().hash(&mut h);
        if self.open {
            pipeline.palette().names().hash(&mut h);
            pipeline.unmatched_row_count(&app.doc).hash(&mut h);
            for opt in pipeline.stage_order_options(&app.doc) {
                opt.name.hash(&mut h);
                opt.count.hash(&mut h);
                opt.total_transactions.hash(&mut h);
            }
        }
        h.finish()
    }

    fn dispatch(&self, command: RowOrderCommand, window: &mut Window, cx: &mut Context<Self>) {
        drop(self.ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(
                self.generation,
                Command::PipelineOrder(self.panel, command),
                Some(window),
                cx,
            );
        }));
    }

    fn dispatch_visibility(
        &self,
        command: StageVisibilityCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        drop(self.ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(
                self.generation,
                Command::PipelineStageVisibility(self.panel, command),
                Some(window),
                cx,
            );
        }));
    }
}

struct StagesOrderSnapshot {
    order: RowOrder,
    visibility: StageVisibility,
    suggested: Option<String>,
    options: Vec<StageOrderOption>,
    unmatched_count: usize,
    palette: StagePalette,
}

impl Render for PipelineOrderView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return Empty.into_any_element();
        }
        let Some(owner) = self.ws.upgrade() else {
            return Empty.into_any_element();
        };
        let snapshot = {
            let app = &owner.read(cx).app;
            let Some(pipeline) = app.panels.pipeline(self.panel) else {
                return Empty.into_any_element();
            };
            StagesOrderSnapshot {
                order: pipeline.order.clone(),
                visibility: pipeline.stage_visibility.clone(),
                suggested: pipeline.suggested_order_stage().map(str::to_owned),
                options: pipeline.stage_order_options(&app.doc),
                unmatched_count: pipeline.unmatched_row_count(&app.doc),
                palette: pipeline.palette().clone(),
            }
        };

        let t = *theme(cx);
        let canvas_core = *canvas_theme(cx);
        let active_stages = snapshot.order.resolved_stages(&snapshot.palette);
        let is_start = matches!(snapshot.order.mode, RowOrderMode::Start);
        let is_end = matches!(snapshot.order.mode, RowOrderMode::End);
        let is_stage_based = snapshot.order.is_stage_based();
        let hidden_count = snapshot.visibility.hidden_count();
        let any_hidden = hidden_count > 0;

        let builtin_variant = |active: bool| {
            if active {
                ButtonCustomVariant::new(cx)
                    .color(t.selection.bg)
                    .foreground(t.panel.text)
                    .hover(t.selection.bg)
                    .active(t.selection.bg)
            } else {
                ButtonCustomVariant::new(cx)
                    .color(t.bar.bg)
                    .foreground(t.panel.text_muted)
                    .hover(t.hover.bg)
                    .active(t.selection.bg)
            }
        };

        let mut panel = div()
            .id(("pipeline-order-popover", self.panel.0))
            .debug_selector(|| "pipeline-order-popover".into())
            .track_focus(&self.focus)
            .w(t.px(360.0))
            .max_w_full()
            .p(t.px(10.0))
            .flex()
            .flex_col()
            .gap(t.px(8.0))
            .rounded(t.px(6.0))
            .border_1()
            .border_color(t.border)
            .bg(t.elevated.bg)
            .text_color(t.elevated.text)
            .text_size(px(t.ui_size_small))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|view, ev: &gpui_kit::MouseDownEvent, _, cx| {
                view.dismissed_at = Some(ev.position);
                view.close(cx);
            }))
            .on_key_down(cx.listener(|view, ev: &gpui_kit::KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    view.close(cx);
                }
            }));

        // Header
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(t.px(6.0))
                .child(
                    div()
                        .font_family(t.mono_font)
                        .text_size(px(t.ui_size_small * 0.9))
                        .text_color(t.panel.text_muted)
                        .child("STAGES & ORDER"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(t.px(4.0))
                        .when(any_hidden, |el| {
                            el.child(
                                Button::new("pipeline-stages-show-all")
                                    .debug_selector(|| "pipeline-stages-show-all".into())
                                    .label("Show all")
                                    .xsmall()
                                    .custom(
                                        ButtonCustomVariant::new(cx)
                                            .color(t.selection.bg)
                                            .foreground(t.panel.text)
                                            .hover(t.hover.bg)
                                            .active(t.selection.bg),
                                    )
                                    .border_1()
                                    .border_color(t.border_focused)
                                    .tooltip("Restore visibility of all hidden primary-lane stages")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.dispatch_visibility(
                                            StageVisibilityCommand::ShowAll,
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                        })
                        .when(!is_start, |el| {
                            el.child(
                                Button::new("pipeline-order-reset")
                                    .debug_selector(|| "pipeline-order-reset".into())
                                    .label("Reset order")
                                    .xsmall()
                                    .ghost()
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.dispatch(RowOrderCommand::Reset, window, cx);
                                    })),
                            )
                        }),
                ),
        );

        // Non-intrusive batched-fetch suggestion banner
        if is_start && let Some(suggested) = snapshot.suggested.clone() {
            let apply_stage = suggested.clone();
            panel = panel.child(
                div()
                    .debug_selector(|| "pipeline-order-suggestion".into())
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(t.px(8.0))
                    .px(t.px(8.0))
                    .py(t.px(6.0))
                    .rounded(t.px(4.0))
                    .border_1()
                    .border_color(t.border_focused.opacity(0.45))
                    .bg(t.selection.bg)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(t.ui_size_small * 0.92))
                            .text_color(t.panel.text)
                            .child(SharedString::from(format!(
                                "Batched fetches in view · try {suggested}"
                            ))),
                    )
                    .child(
                        Button::new("pipeline-order-suggest-apply")
                            .debug_selector(|| "pipeline-order-suggest-apply".into())
                            .label(format!("Use {suggested}"))
                            .xsmall()
                            .primary()
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.dispatch(
                                    RowOrderCommand::SetMode(RowOrderMode::Stages {
                                        stages: vec![apply_stage.clone()],
                                    }),
                                    window,
                                    cx,
                                );
                                view.close(cx);
                            })),
                    ),
            );
        }

        // Built-in lifetime ordering modes
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap(t.px(6.0))
                .child(
                    Button::new("pipeline-order-start")
                        .debug_selector(|| "pipeline-order-start".into())
                        .label("Start (tx.begin)")
                        .xsmall()
                        .custom(builtin_variant(is_start))
                        .border_1()
                        .border_color(if is_start { t.border_focused } else { t.border })
                        .flex_1()
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.dispatch(
                                RowOrderCommand::SetMode(RowOrderMode::Start),
                                window,
                                cx,
                            );
                        })),
                )
                .child(
                    Button::new("pipeline-order-end")
                        .debug_selector(|| "pipeline-order-end".into())
                        .label("End (tx.end)")
                        .xsmall()
                        .custom(builtin_variant(is_end))
                        .border_1()
                        .border_color(if is_end { t.border_focused } else { t.border })
                        .flex_1()
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.dispatch(RowOrderCommand::SetMode(RowOrderMode::End), window, cx);
                        })),
                ),
        );

        // Primary-lane stages from StagePalette + StageCensus coverage.
        // Visibility (left eye) and sorting (right controls + stage click) are separated into
        // distinct columns so showing/hiding a stage cannot be confused with ordering rows.
        if !snapshot.options.is_empty() {
            let stage_heading = if any_hidden {
                format!("PRIMARY STAGES · {hidden_count} hidden")
            } else {
                "PRIMARY STAGES".into()
            };
            panel = panel.child(
                div()
                    .pt(t.px(6.0))
                    .border_t_1()
                    .border_color(t.border_variant)
                    .flex()
                    .items_center()
                    .gap(t.px(6.0))
                    .font_family(t.mono_font)
                    .text_size(px(t.ui_size_small * 0.84))
                    .text_color(t.panel.text_muted)
                    .child(div().w(t.px(26.0)).flex_none().text_center().child("SHOW"))
                    .child(div().w(px(1.0)).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px(t.px(6.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_color(if any_hidden {
                                        t.panel.icon_accent
                                    } else {
                                        t.panel.text_muted
                                    })
                                    .child(SharedString::from(stage_heading)),
                            )
                            .child("COV"),
                    )
                    .child(div().w(px(1.0)).flex_none())
                    .child(div().w(t.px(60.0)).flex_none().text_right().child("SORT")),
            );

            let stage_count = snapshot.options.len();
            let mut stage_list = div()
                .id(("pipeline-order-stages", self.panel.0))
                .flex()
                .flex_col()
                .gap(t.px(2.0))
                .max_h(t.px(220.0))
                .overflow_y_scroll();

            for (idx, opt) in snapshot.options.iter().enumerate() {
                let name = opt.name.clone();
                let pct = (opt.coverage * 100.0).round() as u32;
                let is_hidden = snapshot.visibility.is_hidden(&name);
                let is_checked = active_stages.iter().any(|s| s == &name);
                let is_single_active = matches!(
                    &snapshot.order.mode,
                    RowOrderMode::Stages { stages } if stages.len() == 1 && stages[0] == name
                );
                let is_from_here = matches!(
                    &snapshot.order.mode,
                    RowOrderMode::FromStage { stage } if stage == &name
                );
                let style = snapshot.palette.style(&name, &canvas_core);
                let swatch_fill = hsla(style.fill);
                let swatch_edge = hsla(style.edge);

                let vis_name = name.clone();
                let check_name = name.clone();
                let row_name = name.clone();
                let from_name = name.clone();
                let is_last = idx + 1 == stage_count;

                let vis_selector = format!("pipeline-stage-vis-{idx}");
                let row_selector = format!("pipeline-order-stage-{name}");
                let check_selector = format!("pipeline-order-stage-check-{name}");
                let from_selector = format!("pipeline-order-from-{name}");

                let vis_tooltip = if is_hidden {
                    format!("Show \"{name}\" stage in timeline")
                } else {
                    format!("Hide \"{name}\" stage (dim in timeline)")
                };
                let sort_tooltip = if is_single_active {
                    format!("Ordered by \"{name}\" · click to reset order to Start")
                } else {
                    format!("Click to order rows by \"{name}\"")
                };
                let check_tooltip = if is_checked {
                    format!("Remove \"{name}\" from multi-stage sort")
                } else {
                    format!("Include \"{name}\" in multi-stage sort")
                };
                let from_tooltip = if is_from_here {
                    "Click to reset order to Start".into()
                } else if is_last {
                    format!("Order by \"{name}\" only")
                } else {
                    format!("Order from \"{name}\" onward ({name}+)")
                };

                stage_list = stage_list.child(
                    div()
                        .id(("stage-row-wrap", idx))
                        .flex()
                        .items_center()
                        .gap(t.px(6.0))
                        // Left: visibility eye toggle
                        .child(
                            div()
                                .id(("stage-vis-toggle", idx))
                                .debug_selector(move || vis_selector)
                                .w(t.px(26.0))
                                .h(t.px(24.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(t.px(4.0))
                                .border_1()
                                .border_color(if is_hidden {
                                    t.border
                                } else {
                                    gpui_kit::transparent_black()
                                })
                                .bg(if is_hidden {
                                    t.bar.bg
                                } else {
                                    gpui_kit::transparent_black()
                                })
                                .cursor(CursorStyle::PointingHand)
                                .hover(move |s| s.bg(t.hover.bg))
                                .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                                .tooltip(move |w, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(vis_tooltip.clone())
                                        .build(w, cx)
                                })
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    cx.stop_propagation();
                                    view.dispatch_visibility(
                                        StageVisibilityCommand::Toggle(vis_name.clone()),
                                        window,
                                        cx,
                                    );
                                }))
                                .child(
                                    Icon::new(if is_hidden {
                                        IconName::EyeOff
                                    } else {
                                        IconName::Eye
                                    })
                                    .size(t.px(13.0))
                                    .color(if is_hidden {
                                        t.panel.text_placeholder
                                    } else {
                                        t.panel.icon
                                    }),
                                ),
                        )
                        // Divider between visibility and stage/sort
                        .child(
                            div()
                                .w(px(1.0))
                                .h(t.px(14.0))
                                .flex_none()
                                .bg(t.border_variant),
                        )
                        // Center: swatch, name, coverage (click for single-stage sort)
                        .child(
                            div()
                                .id(("order-stage-row", idx))
                                .debug_selector(move || row_selector)
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(t.px(6.0))
                                .px(t.px(6.0))
                                .py(t.px(4.0))
                                .rounded(t.px(4.0))
                                .cursor(CursorStyle::PointingHand)
                                .bg(if is_checked {
                                    t.selection.bg
                                } else {
                                    gpui_kit::transparent_black()
                                })
                                .hover(move |s| s.bg(t.hover.bg))
                                .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                                .tooltip(move |w, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(sort_tooltip.clone())
                                        .build(w, cx)
                                })
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    if is_single_active {
                                        view.dispatch(RowOrderCommand::Reset, window, cx);
                                    } else {
                                        view.dispatch(
                                            RowOrderCommand::SetMode(RowOrderMode::Stages {
                                                stages: vec![row_name.clone()],
                                            }),
                                            window,
                                            cx,
                                        );
                                    }
                                }))
                                .child(
                                    div()
                                        .size(t.px(8.0))
                                        .flex_none()
                                        .rounded(t.px(2.0))
                                        .bg(swatch_fill)
                                        .border_1()
                                        .border_color(swatch_edge)
                                        .opacity(if is_hidden { 0.25 } else { 1.0 }),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .font_family(t.mono_font)
                                        .text_color(if is_hidden {
                                            t.panel.text_placeholder
                                        } else {
                                            t.panel.text
                                        })
                                        .when(is_hidden, gpui_kit::Styled::line_through)
                                        .child(SharedString::from(name)),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .font_family(t.mono_font)
                                        .text_size(px(t.ui_size_small * 0.88))
                                        .text_color(if is_hidden {
                                            t.panel.text_placeholder
                                        } else {
                                            t.panel.text_muted
                                        })
                                        .child(SharedString::from(format!("{pct}%"))),
                                ),
                        )
                        // Divider between stage identity and right-side sort controls
                        .child(
                            div()
                                .w(px(1.0))
                                .h(t.px(14.0))
                                .flex_none()
                                .bg(t.border_variant),
                        )
                        // Right: multi-stage sort checkbox + FromStage onward pill
                        .child(
                            div()
                                .w(t.px(60.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_end()
                                .gap(t.px(5.0))
                                .child(
                                    div()
                                        .id(("order-stage-check", idx))
                                        .debug_selector(move || check_selector)
                                        .size(t.px(15.0))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(t.px(3.0))
                                        .border_1()
                                        .border_color(if is_checked {
                                            t.button.bg
                                        } else {
                                            t.border
                                        })
                                        .bg(if is_checked { t.button.bg } else { t.bar.bg })
                                        .cursor(CursorStyle::PointingHand)
                                        .text_size(px(t.ui_size_small * 0.85))
                                        .text_color(t.button.text)
                                        .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                                        .tooltip(move |w, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(
                                                check_tooltip.clone(),
                                            )
                                            .build(w, cx)
                                        })
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            cx.stop_propagation();
                                            view.dispatch(
                                                RowOrderCommand::ToggleStage(check_name.clone()),
                                                window,
                                                cx,
                                            );
                                        }))
                                        .child(if is_checked { "✓" } else { "" }),
                                )
                                .child(
                                    div()
                                        .id(("order-stage-from", idx))
                                        .debug_selector(move || from_selector)
                                        .px(t.px(5.0))
                                        .py(t.px(1.0))
                                        .rounded(t.px(3.0))
                                        .border_1()
                                        .border_color(if is_from_here {
                                            t.border_focused
                                        } else {
                                            t.border
                                        })
                                        .bg(if is_from_here {
                                            t.selection.bg
                                        } else {
                                            t.bar.bg
                                        })
                                        .cursor(CursorStyle::PointingHand)
                                        .font_family(t.mono_font)
                                        .text_size(px(t.ui_size_small * 0.85))
                                        .text_color(if is_from_here {
                                            t.panel.text
                                        } else {
                                            t.panel.text_muted
                                        })
                                        .hover(move |s| s.bg(t.hover.bg).text_color(t.panel.text))
                                        .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                                        .tooltip(move |w, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(
                                                from_tooltip.clone(),
                                            )
                                            .build(w, cx)
                                        })
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            cx.stop_propagation();
                                            if is_from_here {
                                                view.dispatch(RowOrderCommand::Reset, window, cx);
                                            } else {
                                                view.dispatch(
                                                    RowOrderCommand::SetMode(
                                                        RowOrderMode::FromStage {
                                                            stage: from_name.clone(),
                                                        },
                                                    ),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }))
                                        .child(if is_last { "only" } else { "+" }),
                                ),
                        ),
                );
            }
            panel = panel.child(stage_list);
        }

        // Unmatched policy & FollowActivity sort-key anchoring
        if is_stage_based {
            let unmatched_heading = if snapshot.unmatched_count > 0 {
                format!("UNMATCHED ROWS ({})", snapshot.unmatched_count)
            } else {
                "UNMATCHED ROWS".into()
            };
            let current_unmatched = snapshot.order.unmatched;
            let follow_sort_key = snapshot.order.follow_sort_key;

            let mut seg = div().flex().items_center().gap(t.px(4.0)).w_full();
            for (policy, label, sel) in [
                (
                    UnmatchedPolicy::FallbackEnd,
                    "Fallback: End",
                    "pipeline-order-unmatched-end",
                ),
                (
                    UnmatchedPolicy::FallbackStart,
                    "Fallback: Start",
                    "pipeline-order-unmatched-start",
                ),
                (
                    UnmatchedPolicy::Hide,
                    "Hide",
                    "pipeline-order-unmatched-hide",
                ),
            ] {
                let active = current_unmatched == policy;
                seg = seg.child(
                    Button::new(sel)
                        .debug_selector(move || sel.into())
                        .label(label)
                        .xsmall()
                        .custom(builtin_variant(active))
                        .border_1()
                        .border_color(if active { t.border_focused } else { t.border })
                        .flex_1()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.dispatch(RowOrderCommand::SetUnmatched(policy), window, cx);
                        })),
                );
            }

            panel = panel.child(
                div()
                    .pt(t.px(6.0))
                    .border_t_1()
                    .border_color(t.border_variant)
                    .flex()
                    .flex_col()
                    .gap(t.px(6.0))
                    .child(
                        div()
                            .font_family(t.mono_font)
                            .text_size(px(t.ui_size_small * 0.85))
                            .text_color(t.panel.text_muted)
                            .child(SharedString::from(unmatched_heading)),
                    )
                    .child(seg)
                    .child(
                        div()
                            .id(("pipeline-order-follow-sort-key", self.panel.0))
                            .debug_selector(|| "pipeline-order-follow-sort-key".into())
                            .flex()
                            .items_center()
                            .gap(t.px(6.0))
                            .py(t.px(2.0))
                            .cursor(CursorStyle::PointingHand)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.dispatch(
                                    RowOrderCommand::SetFollowSortKey(!follow_sort_key),
                                    window,
                                    cx,
                                );
                            }))
                            .child(
                                div()
                                    .size(t.px(14.0))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(t.px(3.0))
                                    .border_1()
                                    .border_color(if follow_sort_key {
                                        t.button.bg
                                    } else {
                                        t.border
                                    })
                                    .bg(if follow_sort_key {
                                        t.button.bg
                                    } else {
                                        t.bar.bg
                                    })
                                    .text_size(px(t.ui_size_small * 0.85))
                                    .text_color(t.button.text)
                                    .child(if follow_sort_key { "✓" } else { "" }),
                            )
                            .child(
                                div()
                                    .text_size(px(t.ui_size_small * 0.92))
                                    .text_color(t.panel.text)
                                    .child("Anchor FollowActivity on sort-stage timestamp"),
                            ),
                    ),
            );
        }

        let pos = self.anchor_pos.map_or_else(
            || point(t.px(320.0), t.px(64.0)),
            |p| point(p.x + t.px(24.0), p.y + t.px(16.0)),
        );

        deferred(
            anchored()
                .anchor(Anchor::TopRight)
                .position(pos)
                .snap_to_window_with_margin(t.px(8.0))
                .child(panel),
        )
        .with_priority(10)
        .into_any_element()
    }
}
