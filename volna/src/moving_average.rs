//! Moving-average controls with debounced previews on the shared waveform timeline.

#[cfg(all(test, not(target_family = "wasm")))]
mod tests;

use crate::{
    app::Workspace,
    theme::{ThemePx, theme},
};
use gpui_kit::base::Disableable;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, IntoElement, Render, Subscription, WeakEntity, Window, div, px};
use volna_core::{
    Command,
    moving_average::{Command as AverageCommand, ComputationStatus, Draft},
    panels::PanelId,
};

pub(crate) struct MovingAverageEditor {
    pub id: u64,
    owner: WeakEntity<Workspace>,
    panel: PanelId,
    generation: u64,
    name: Entity<InputState>,
    window: Entity<InputState>,
    interval: Entity<InputState>,
    partial: bool,
    skip_unknown: bool,
    preview_task: Option<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MovingAverageEditor {
    pub fn new(
        owner: WeakEntity<Workspace>,
        panel: PanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let entity = owner.upgrade().expect("editor has workspace");
        let ws = entity.read(cx);
        let editor = ws.app.moving_average_editor.as_ref().expect("open editor");
        let (id, draft, generation) = (editor.id, editor.draft.clone(), ws.app.doc.generation());
        let name = cx.new(|cx| InputState::new(window, cx).default_value(draft.name));
        let trailing = cx.new(|cx| InputState::new(window, cx).default_value(draft.window));
        let interval = cx.new(|cx| InputState::new(window, cx).default_value(draft.interval));
        let mut subscriptions = Vec::new();
        for input in [&name, &trailing, &interval] {
            subscriptions.push(
                cx.subscribe_in(input, window, |view, _, event, window, cx| {
                    if matches!(event, InputEvent::Change) && view.update_draft(window, cx) {
                        view.schedule_preview(window, cx);
                    }
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        view.preview_task = None;
                        view.send(AverageCommand::Preview, window, cx);
                    }
                }),
            );
        }
        cx.observe(&entity, |_, _, cx| cx.notify()).detach();
        Self {
            id,
            owner,
            panel,
            generation,
            name,
            window: trailing,
            interval,
            partial: draft.partial,
            skip_unknown: draft.skip_unknown,
            preview_task: None,
            _subscriptions: subscriptions,
        }
    }

    fn send(&self, command: AverageCommand, window: &mut Window, cx: &mut Context<Self>) {
        drop(self.owner.update(cx, |ws, cx| {
            if ws
                .app
                .moving_average_editor
                .as_ref()
                .is_none_or(|e| e.id != self.id)
            {
                return;
            }
            ws.dispatch_if_current(
                self.generation,
                Command::MovingAverage(self.panel, command),
                Some(window),
                cx,
            );
            if ws.app.doc.generation() == self.generation
                && ws.app.moving_average_editor.is_none()
                && let Some(focus) = ws.panel_focus(self.panel, cx)
            {
                window.focus(&focus, cx);
            }
        }));
    }

    fn schedule_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(300));
        self.preview_task = Some(cx.spawn_in(window, async move |this, cx| {
            timer.await;
            this.update_in(cx, |view, window, cx| {
                view.preview_task = None;
                view.send(AverageCommand::Preview, window, cx);
            })
            .ok();
        }));
    }

    fn update_draft(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(owner) = self.owner.upgrade() else {
            return false;
        };
        let revision = owner
            .read(cx)
            .app
            .moving_average_editor
            .as_ref()
            .map(|editor| editor.calculation_revision);
        self.send(
            AverageCommand::Update(Draft {
                name: self.name.read(cx).value().to_string(),
                window: self.window.read(cx).value().to_string(),
                interval: self.interval.read(cx).value().to_string(),
                partial: self.partial,
                skip_unknown: self.skip_unknown,
            }),
            window,
            cx,
        );
        owner
            .read(cx)
            .app
            .moving_average_editor
            .as_ref()
            .is_some_and(|editor| Some(editor.calculation_revision) != revision)
    }
}

impl Render for MovingAverageEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme(cx);
        let Some(owner) = self.owner.upgrade() else {
            return div().into_any_element();
        };
        let ws = owner.read(cx);
        let Some(editor) = ws
            .app
            .moving_average_editor
            .as_ref()
            .filter(|e| e.id == self.id)
        else {
            return div().into_any_element();
        };
        let mut path = editor.input.locator(&ws.app.doc).0;
        if let Some(slice) = editor.interpretation.projection
            && let Some(var) = editor.input.var()
            && let Some(shape) = ws
                .app
                .doc
                .hierarchy(var.trace)
                .and_then(|h| h.get_var(var.item))
                .map(|v| v.shape)
            && let Some(name) = path.last_mut()
        {
            *name = volna_core::wave::bus::BusIndices::new(name, shape.width()).label(slice);
        }
        let input = path.join(".");
        let interpretation = editor.interpretation.label();
        let bounds = format!(
            "{} — {}",
            volna_core::wave::timeline::format_time(editor.domain.0 as f64, ws.app.doc.time_base()),
            volna_core::wave::timeline::format_time(editor.domain.1 as f64, ws.app.doc.time_base())
        );
        let mut ready = false;
        let mut building = false;
        let mut failed = false;
        let status = match ws.app.moving_average_preview() {
            Some(ComputationStatus::Ready(history)) => {
                use volna_trace::data::SignalHistory;
                ready = editor.error.is_none();
                format!("{} output samples · live preview", history.len())
            }
            Some(ComputationStatus::Waiting) => {
                building = true;
                "Loading input…".into()
            }
            Some(ComputationStatus::Building) => {
                building = true;
                "Updating waveform…".into()
            }
            Some(ComputationStatus::Failed(error)) => {
                failed = true;
                error.clone()
            }
            None if editor.stopped => "Preview stopped.".into(),
            None if editor.error.is_some() => "Fix the settings to update the waveform.".into(),
            None => "Updating waveform…".into(),
        };
        let stopped = editor.stopped;
        let error = editor.error.clone();
        let editing = editor.editing;
        let title = if editing {
            "Edit derived signal"
        } else {
            "Derive signal"
        };
        let field = |label: &'static str, input: &Entity<InputState>, selector: &'static str| {
            div()
                .id(selector)
                .debug_selector(move || selector.to_owned())
                .flex()
                .flex_col()
                .gap_1()
                .child(label)
                .child(Input::new(input).small().w_full())
        };
        let close = Button::new("average-close")
            .label("Cancel")
            .ghost()
            .xsmall()
            .on_click(
                cx.listener(|view, _, window, cx| view.send(AverageCommand::Close, window, cx)),
            );
        let warmup = Button::new("average-warmup")
            .label(if self.partial {
                "Warm-up: available history"
            } else {
                "Warm-up: wait for full window"
            })
            .small()
            .selected(self.partial)
            .on_click(cx.listener(|view, _, window, cx| {
                view.partial = !view.partial;
                view.preview_task = None;
                view.update_draft(window, cx);
                view.send(AverageCommand::Preview, window, cx);
            }));
        let missing = Button::new("average-unknown")
            .label(if self.skip_unknown {
                "Missing values: skip unknown time"
            } else {
                "Missing values: propagate gaps"
            })
            .small()
            .selected(self.skip_unknown)
            .on_click(cx.listener(|view, _, window, cx| {
                view.skip_unknown = !view.skip_unknown;
                view.preview_task = None;
                view.update_draft(window, cx);
                view.send(AverageCommand::Preview, window, cx);
            }));
        let actions =
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .when(failed || stopped, |el| {
                    el.child(
                        Button::new("average-retry")
                            .label(if stopped { "Resume" } else { "Retry" })
                            .small()
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.preview_task = None;
                                view.send(AverageCommand::Preview, window, cx);
                            })),
                    )
                })
                .when(building, |el| {
                    el.child(Button::new("average-stop").label("Stop").small().on_click(
                        cx.listener(|view, _, window, cx| {
                            view.preview_task = None;
                            view.send(AverageCommand::Stop, window, cx);
                        }),
                    ))
                })
                .child(
                    Button::new("average-apply")
                        .label(if editing {
                            "Apply changes"
                        } else {
                            "Add waveform"
                        })
                        .primary()
                        .small()
                        .disabled(!ready)
                        .debug_selector(|| "average-apply".into())
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.send(AverageCommand::Apply, window, cx);
                        })),
                );
        let description = "Time-weighted mean over [t − window, t). Output values are held until the next output sample. Zoom does not change the result.";
        div()
            .id("moving-average-editor")
            .debug_selector(|| "moving-average-editor".into())
            .h_full()
            .w(t.px(320.0))
            .max_w_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(t.border)
            .bg(t.panel.bg)
            .text_color(t.panel.text)
            .text_size(px(t.ui_size_small))
            .p(t.px(12.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(title)
                    .child(close),
            )
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .child(format!("Input · {input}")),
            )
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .child("Operation · Moving average"),
            )
            .child(field("Name", &self.name, "average-name"))
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .child(format!("Interpretation · {interpretation}")),
            )
            .child(field("Trailing window", &self.window, "average-window"))
            .child(field("Output interval", &self.interval, "average-interval"))
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .child("Use time units (4 us, 250 ns) or timeline ticks."),
            )
            .child(warmup)
            .child(missing)
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .child(format!("Output domain · {bounds}")),
            )
            .child(div().text_color(t.panel.text_muted).child(description))
            .child(div().text_color(t.panel.text_muted).child(
                "Changes preview automatically in the waveform row. Cancel discards this draft.",
            ))
            .child(
                div()
                    .id("average-status")
                    .debug_selector(|| "average-status".into())
                    .child(status),
            )
            .when_some(error, |el, error| {
                el.child(div().text_color(t.panel.error).child(error))
            })
            .child(actions)
            .when(editing, |el| {
                el.child(
                    Button::new("average-remove")
                        .label("Remove definition")
                        .ghost()
                        .small()
                        .debug_selector(|| "average-remove".into())
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.send(AverageCommand::Remove, window, cx);
                        })),
                )
            })
            .into_any_element()
    }
}
