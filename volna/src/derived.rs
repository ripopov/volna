//! Recorded-input computation controls with debounced previews on the shared waveform timeline.

mod boundaries;
mod editor;
mod inputs;
mod pins;
mod reference;
mod sampling;
mod search;
mod table_preview;
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
    input::{Editor, EditorState, Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::ScrollableElement,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Entity, Focusable, IntoElement, KeyDownEvent, Render, Subscription,
    WeakEntity, Window, div, px,
};
use volna_core::{
    Command,
    computed::{Command as DerivedCommand, ComputationStatus, Draft},
    panels::PanelId,
};

use inputs::{BindingFields, InputPicker};
use search::InputSearch;

gpui_kit::actions!(derived, [PreviewFormula]);

pub(crate) fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("ctrl-enter", PreviewFormula, Some("FormulaEditor > Input")),
        gpui_kit::KeyBinding::new("cmd-enter", PreviewFormula, Some("FormulaEditor > Input")),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GuideVisibility {
    Open,
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Composer {
    Reading,
    Expression,
}

pub(crate) struct DerivedEditor {
    pub id: u64,
    owner: WeakEntity<Workspace>,
    panel: PanelId,
    generation: u64,
    name: Entity<InputState>,
    interval: Entity<InputState>,
    expression: Entity<EditorState>,
    reference_search: Entity<InputState>,
    search: Entity<InputState>,
    picker: Option<InputPicker>,
    input_search: InputSearch,
    sampling_default: bool,
    bindings: Vec<BindingFields>,
    skip_unknown: bool,
    scope: volna_core::computed::MeasurementScope,
    saved_ticks: Vec<u64>,
    guide: GuideVisibility,
    expanded: bool,
    composer: Composer,
    focus: gpui_kit::FocusHandle,
    preview_task: Option<gpui_kit::Task<()>>,
    diagnostic: Option<volna_core::computed::language::Diagnostic>,
    analysis: Option<std::sync::Arc<editor::Analysis>>,
    _subscriptions: Vec<Subscription>,
}

impl DerivedEditor {
    pub fn new(
        owner: WeakEntity<Workspace>,
        panel: PanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let entity = owner.upgrade().expect("editor has workspace");
        let ws = entity.read(cx);
        let editor = ws.app.derived_editor.as_ref().expect("open editor");
        let (id, draft, generation) = (editor.id, editor.draft.clone(), ws.app.doc.generation());
        let simple_measure = editor.measuring();
        let focus = cx.focus_handle();
        let name = cx.new(|cx| InputState::new(window, cx).default_value(draft.name));
        let interval = cx.new(|cx| InputState::new(window, cx).default_value(draft.interval));
        let expression =
            cx.new(|cx| editor::state(&owner, window, cx).default_value(draft.formula.clone()));
        if simple_measure {
            focus.focus(window, cx);
        } else {
            expression.read(cx).focus_handle(cx).focus(window, cx);
        }
        let reference_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a function or example…"));
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search recorded signals…"));
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe(&reference_search, |_, _, _: &InputEvent, cx| cx.notify()));
        for input in [&name, &interval] {
            subscriptions.push(
                cx.subscribe_in(input, window, |view, _, event, window, cx| {
                    if matches!(event, InputEvent::Change) && view.update_draft(window, cx) {
                        view.schedule_preview(window, cx);
                    }
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        view.preview_task = None;
                        view.send(DerivedCommand::Preview, window, cx);
                    }
                }),
            );
        }
        subscriptions.push(
            cx.subscribe_in(&expression, window, |view, _, event, window, cx| {
                if matches!(event, InputEvent::Change) && view.update_draft(window, cx) {
                    view.schedule_preview(window, cx);
                }
            }),
        );
        cx.observe(&expression, |_, _, cx| cx.notify()).detach();
        subscriptions.push(cx.subscribe(&search, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.sync_search(cx);
            }
        }));
        cx.observe(&entity, |view, _, cx| {
            view.sync_search(cx);
            cx.notify();
        })
        .detach();
        Self {
            id,
            owner,
            panel,
            generation,
            name,
            interval,
            expression,
            reference_search,
            search,
            picker: None,
            input_search: InputSearch::default(),
            sampling_default: false,
            bindings: Vec::new(),
            skip_unknown: draft.skip_unknown,
            saved_ticks: boundaries::saved_ticks(&draft.scope),
            scope: draft.scope,
            guide: GuideVisibility::Closed,
            expanded: false,
            composer: if simple_measure {
                Composer::Reading
            } else {
                Composer::Expression
            },
            focus,
            preview_task: None,
            diagnostic: None,
            analysis: None,
            _subscriptions: subscriptions,
        }
    }

    fn send(&mut self, command: DerivedCommand, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            command,
            DerivedCommand::AddInput(_)
                | DerivedCommand::RemoveInput(_)
                | DerivedCommand::Rebind(..)
                | DerivedCommand::Close
        ) {
            self.close_picker();
        }
        drop(self.owner.update(cx, |ws, cx| {
            if ws
                .app
                .derived_editor
                .as_ref()
                .is_none_or(|e| e.id != self.id)
            {
                return;
            }
            ws.dispatch_if_current(
                self.generation,
                Command::Derived(self.panel, command),
                Some(window),
                cx,
            );
            if ws.app.doc.generation() == self.generation
                && ws.app.derived_editor.is_none()
                && let Some(focus) = ws.panel_focus(self.panel, cx)
            {
                window.focus(&focus, cx);
            }
        }));
    }

    fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer == Composer::Reading {
            self.focus.focus(window, cx);
        } else {
            self.expression.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    fn schedule_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(300));
        self.preview_task = Some(cx.spawn_in(window, async move |this, cx| {
            timer.await;
            this.update_in(cx, |view, window, cx| {
                view.preview_task = None;
                view.send(DerivedCommand::Preview, window, cx);
            })
            .ok();
        }));
    }

    fn update_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(owner) = self.owner.upgrade() else {
            return false;
        };
        let revision = owner
            .read(cx)
            .app
            .derived_editor
            .as_ref()
            .map(|editor| editor.calculation_revision);
        self.send(
            DerivedCommand::Update(Draft {
                name: self.name.read(cx).value().to_string(),
                interval: self.interval.read(cx).value().to_string(),
                skip_unknown: self.skip_unknown,
                formula: self.expression.read(cx).value().to_string(),
                scope: self.scope.clone(),
            }),
            window,
            cx,
        );
        owner
            .read(cx)
            .app
            .derived_editor
            .as_ref()
            .is_some_and(|editor| Some(editor.calculation_revision) != revision)
    }

    fn set_scope(
        &mut self,
        scope: volna_core::computed::MeasurementScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            scope,
            volna_core::computed::MeasurementScope::Selected { .. }
        ) {
            self.saved_ticks = boundaries::saved_ticks(&scope);
        }
        self.scope = scope;
        self.preview_task = None;
        self.update_draft(window, cx);
        self.send(DerivedCommand::Preview, window, cx);
    }

    fn example_text(&self, formula: &str, cx: &Context<Self>) -> String {
        if let Some(owner) = self.owner.upgrade() {
            let ws = owner.read(cx);
            if let Some(editor) = &ws.app.derived_editor {
                return editor::adapt_example(formula, editor, &ws.app.doc);
            }
        }
        formula.into()
    }

    fn use_example(&mut self, formula: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut formula = self.example_text(formula, cx);
        if !formula.contains('@')
            && let Some(owner) = self.owner.upgrade()
        {
            let ws = owner.read(cx);
            if let Some(editor) = &ws.app.derived_editor
                && let Some(choice) = editor.sampling(&ws.app.doc)
                && let Some(binding) = editor.bindings.iter().find(|input| {
                    input.source.var() == choice.resolve(&ws.app.doc) && input.slice.is_empty()
                })
                && let Ok(sampled) =
                    volna_core::computed::capture_expression(&formula, choice.mode, &binding.alias)
            {
                formula = sampled;
            }
        }
        self.expression
            .update(cx, |input, cx| input.set_value(formula, window, cx));
        self.focus_editor(window, cx);
        self.preview_task = None;
        self.update_draft(window, cx);
        self.send(DerivedCommand::Preview, window, cx);
    }

    fn insert_formula(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::EntityInputHandler;
        self.expression.update(cx, |input, cx| {
            input.replace_text_in_range(None, text, window, cx);
            input.focus_handle(cx).focus(window, cx);
        });
        self.update_draft(window, cx);
        self.schedule_preview(window, cx);
    }

    fn preset(&mut self, function: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let ws = owner.read(cx);
        let Some(alias) = self
            .analysis(cx)
            .and_then(|analysis| analysis.preset_input())
        else {
            return;
        };
        let step = self.interval.read(cx).value().to_string();
        let expression = match function {
            "moving_avg" => format!(
                "moving_avg({alias}, {}, step: {step})",
                volna_core::moving_average::duration(&step, ws.app.doc.time_base()).map_or_else(
                    |_| step.clone(),
                    |ticks| {
                        let ticks = ticks.saturating_mul(16);
                        let formatted = volna_core::wave::timeline::format_time(
                            ticks as f64,
                            ws.app.doc.time_base(),
                        );
                        if volna_core::moving_average::duration(&formatted, ws.app.doc.time_base())
                            .ok()
                            == Some(ticks)
                        {
                            formatted
                        } else {
                            format!("{ticks} ticks")
                        }
                    }
                )
            ),
            "lowpass" => format!("lowpass({alias}, {step}, step: {step})"),
            _ => format!("{function}({alias})"),
        };
        self.expression
            .update(cx, |input, cx| input.set_value(expression, window, cx));
        self.focus_editor(window, cx);
        self.update_draft(window, cx);
        self.send(DerivedCommand::Preview, window, cx);
    }
}

impl Render for DerivedEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_bindings(window, cx);
        let t = *theme(cx);
        let Some(owner) = self.owner.upgrade() else {
            return div().into_any_element();
        };
        let ws = owner.read(cx);
        let Some(editor) = ws.app.derived_editor.as_ref().filter(|e| e.id == self.id) else {
            return div().into_any_element();
        };
        let inputs = editor.bindings.clone();
        let bounds = format!(
            "{} — {}",
            volna_core::wave::timeline::format_time(editor.domain.0 as f64, ws.app.doc.time_base()),
            volna_core::wave::timeline::format_time(editor.domain.1 as f64, ws.app.doc.time_base())
        );
        let mut ready = false;
        let mut building = false;
        let mut failed = false;
        let stopped = editor.stopped();
        let preview = ws.app.derived_preview();
        let status = match preview {
            Some(ComputationStatus::Ready(history)) => {
                ready = editor.error.is_none();
                let warnings = history.warnings();
                format!(
                    "{} output events · live preview{}",
                    history.len(),
                    if warnings.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", warnings.join(" · "))
                    }
                )
            }
            Some(ComputationStatus::ReadyMeasurement(measurement)) => {
                ready = editor.error.is_none();
                let detail = match measurement.status {
                    volna_core::computed::MeasurementStatus::UnknownData
                        if measurement.policy == volna_core::computed::UnknownPolicy::Gap =>
                    {
                        " · choose Skip unknown to measure the known values"
                    }
                    volna_core::computed::MeasurementStatus::UnknownData => {
                        " · unknown values excluded"
                    }
                    volna_core::computed::MeasurementStatus::NoKnownData => {
                        " · no known values in this interval"
                    }
                    volna_core::computed::MeasurementStatus::NonFinite => " · result is not finite",
                    volna_core::computed::MeasurementStatus::Complete => "",
                };
                format!(
                    "{}{detail}",
                    pins::readout(measurement, ws.app.doc.time_base())
                )
            }
            Some(ComputationStatus::ReadyTable(rows)) => {
                ready = editor.error.is_none();
                format!("{} intervals · live preview", rows.len())
            }
            Some(ComputationStatus::Waiting) => {
                building = true;
                "Loading input…".into()
            }
            Some(ComputationStatus::Building) => {
                building = true;
                "Updating waveform…".into()
            }
            Some(ComputationStatus::Failed(_)) => {
                failed = true;
                "Preview failed".into()
            }
            None if stopped => "Preview stopped.".into(),
            None if editor.error.is_some() => "Fix the settings to update the waveform.".into(),
            None => "Updating waveform…".into(),
        };
        let error = editor.error.clone();
        let editing = editor.editing;

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
        let Some(analysis) = self.analysis(cx) else {
            return div().into_any_element();
        };
        let measurement = analysis.result.as_ref().is_ok_and(|program| {
            program.kind == volna_core::computed::ResultKind::ScalarMeasurement
        });
        let title = if editing {
            "Edit formula"
        } else if measurement {
            "Measure signal"
        } else {
            "Derive signal"
        };
        let captured = analysis
            .result
            .as_ref()
            .is_ok_and(volna_core::computed::Program::measures_captures);
        let can_preset = analysis.preset_input().is_some();
        let default_step = analysis
            .result
            .as_ref()
            .is_ok_and(volna_core::computed::Program::uses_default_step)
            || volna_core::moving_average::duration(
                &self.interval.read(cx).value(),
                ws.app.doc.time_base(),
            )
            .is_err();
        let boundaries = measurement.then(|| {
            self.boundary_picker(&ws.app.doc, editor.domain, cx)
                .into_any_element()
        });
        let table_preview = (editor.measuring() && measurement)
            .then(|| Self::table_preview(ws, editor.draft.name.trim(), preview, cx));
        let skip_unknown = self.skip_unknown;
        let close = Button::new("derived-close")
            .debug_selector(|| "derived-close".into())
            .label("Cancel")
            .ghost()
            .xsmall()
            .on_click(
                cx.listener(|view, _, window, cx| view.send(DerivedCommand::Close, window, cx)),
            );
        let pin = matches!(preview, Some(ComputationStatus::ReadyMeasurement(reading)) if reading.value.is_some());
        let actions =
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .when(measurement, |el| {
                    el.child(
                        Button::new("pin-reading")
                            .label("Pin reading")
                            .small()
                            .disabled(!ready || !pin)
                            .debug_selector(|| "pin-reading".into())
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.send(DerivedCommand::Pin, window, cx);
                            })),
                    )
                })
                .when(failed || stopped, |el| {
                    el.child(
                        Button::new("derived-retry")
                            .label(if stopped { "Resume" } else { "Retry" })
                            .small()
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.preview_task = None;
                                view.send(DerivedCommand::Preview, window, cx);
                            })),
                    )
                })
                .when(building, |el| {
                    el.child(Button::new("derived-stop").label("Stop").small().on_click(
                        cx.listener(|view, _, window, cx| {
                            view.preview_task = None;
                            view.send(DerivedCommand::Stop, window, cx);
                        }),
                    ))
                })
                .child(
                    Button::new("derived-apply")
                        .label(if editing {
                            "Apply changes"
                        } else {
                            "Add to waveform"
                        })
                        .primary()
                        .small()
                        .disabled(!ready)
                        .debug_selector(|| "derived-apply".into())
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.send(DerivedCommand::Apply, window, cx);
                        })),
                )
                .child(
                    Button::new("derived-table")
                        .debug_selector(|| "derived-table".into())
                        .label("Add to table")
                        .small()
                        .disabled(!ready)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.send(DerivedCommand::ApplyTable, window, cx);
                        })),
                );
        let expression = self.expression.read(cx).value().to_string();
        let whole_expression = |message: String| volna_core::computed::language::Diagnostic {
            source: expression.clone(),
            range: 0..expression.len(),
            message,
        };
        let diagnostic = analysis.result.as_ref().err().cloned().or_else(|| {
            error
                .or_else(|| match preview {
                    Some(ComputationStatus::Failed(error)) => Some(error.clone()),
                    _ => None,
                })
                .map(whole_expression)
        });
        let narrow = ws
            .app
            .panels
            .waves(self.panel)
            .is_some_and(|w| w.last_layout().bounds.size.width < 560.0 * t.zoom);
        if self.diagnostic != diagnostic {
            self.diagnostic.clone_from(&diagnostic);
            self.expression.update(cx, |state, cx| {
                use gpui_kit::base::input::{Diagnostic, DiagnosticSeverity};
                use gpui_kit::component::input::RopeExt;
                let text = state.text().clone();
                if let Some(set) = state.diagnostics_mut() {
                    set.clear();
                    if let Some(diagnostic) = &diagnostic {
                        let range = &diagnostic.range;
                        set.push(
                            Diagnostic::new(
                                text.offset_to_position(range.start)
                                    ..text.offset_to_position(range.end),
                                diagnostic.message.clone(),
                            )
                            .with_severity(DiagnosticSeverity::Error),
                        );
                    }
                }
                cx.notify();
            });
        }
        let header = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .px(t.px(10.0))
            .py_1()
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .min_w_0()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(actions)
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new("derived-expand")
                            .debug_selector(|| "derived-expand".into())
                            .label(if self.expanded { "Compact" } else { "Expand" })
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.expanded = !view.expanded;
                                cx.notify();
                            })),
                    )
                    .when(editing, |el| {
                        el.child(
                            Button::new("derived-remove")
                                .label("Remove definition")
                                .ghost()
                                .small()
                                .debug_selector(|| "derived-remove".into())
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.send(DerivedCommand::Remove, window, cx);
                                })),
                        )
                    })
                    .child(close),
            );
        let has_diagnostic = diagnostic.is_some();
        let error_card = diagnostic.map(|diagnostic| {
            let message = diagnostic.message;
            let for_copy = message.clone();
            let for_details = message.clone();
            div()
                .id("derived-error")
                .debug_selector(|| "derived-error".into())
                .flex()
                .flex_col()
                .gap_1()
                .p(t.px(8.0))
                .border_1()
                .border_color(t.panel.error)
                .child(
                    div()
                        .max_h(t.px(90.0))
                        .overflow_y_scrollbar()
                        .whitespace_normal()
                        .text_color(t.panel.error)
                        .child(gpui_kit::base::SelectableText::new(
                            "derived-error-text",
                            message,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            Button::new("derived-copy-error")
                                .debug_selector(|| "derived-copy-error".into())
                                .label("Copy error")
                                .xsmall()
                                .on_click(move |_, _, cx| {
                                    crate::clipboard::copy(for_copy.as_str(), cx);
                                }),
                        )
                        .child(
                            Button::new("derived-error-details")
                                .label("Details…")
                                .xsmall()
                                .on_click(move |_, window, cx| {
                                    crate::ui::message_dialog::open_message_dialog(
                                        "Formula error",
                                        for_details.clone(),
                                        window,
                                        cx,
                                    );
                                }),
                        ),
                )
        });
        let composer = if self.composer == Composer::Reading {
            div()
                .id("reading-summary")
                .flex()
                .flex_col()
                .gap_2()
                .p(t.px(12.0))
                .child(
                    inputs
                        .first()
                        .map_or_else(String::new, |input| input.alias.clone()),
                )
                .child(div().font_family(t.mono_font).child(expression.clone()))
                .child(
                    Button::new("edit-measure-expression")
                        .label("Edit expression")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.composer = Composer::Expression;
                            view.focus_editor(window, cx);
                            cx.notify();
                        })),
                )
        } else {
            let composer = div()
                .id("formula-composer")
                .flex_none()
                .flex()
                .flex_col()
                .gap_2()
                .p(t.px(12.0))
                .child(field("Output name", &self.name, "derived-name"))
                .child("Expression")
                .child(
                    div()
                        .id("formula-expression")
                        .debug_selector(|| "formula-expression".into())
                        .font_family(t.mono_font)
                        .key_context("FormulaEditor")
                        .on_action(cx.listener(|view, _: &PreviewFormula, window, cx| {
                            view.preview_task = None;
                            view.update_draft(window, cx);
                            view.send(DerivedCommand::Preview, window, cx);
                        }))
                        .h(t.px(68.0))
                        .border_1()
                        .border_color(t.panel.icon_accent)
                        .rounded(t.px(4.0))
                        .child(
                            Editor::new(&self.expression)
                                .h_full()
                                .bg(t.selection.bg)
                                .bordered(false)
                                .aria_label("Formula expression"),
                        ),
                )
                .child(
                    div()
                        .text_color(t.panel.text_muted)
                        .whitespace_normal()
                        .child("Ctrl+Space for completions"),
                );
            composer.when_some(
                analysis
                    .syntax
                    .signature(self.expression.read(cx).selected_range().end),
                |el, signature| {
                    el.child(
                        div()
                            .debug_selector(|| "formula-signature".into())
                            .font_family(t.mono_font)
                            .text_color(t.panel.text_muted)
                            .whitespace_normal()
                            .child(signature),
                    )
                },
            )
        };
        let composer = composer.when_some(error_card, ParentElement::child).child(
            div().flex().flex_wrap().gap_1().children(
                (if measurement {
                    &[
                        ("interval_mean", "Time mean"),
                        ("interval_min", "Time min"),
                        ("interval_max", "Time max"),
                        ("interval_sum", "Sample sum"),
                        ("sample_mean", "Sample mean"),
                        ("sample_min", "Sample min"),
                        ("sample_max", "Sample max"),
                        ("lowpass", "Low-pass"),
                    ][..]
                } else {
                    &[
                        ("moving_avg", "Moving average"),
                        ("lowpass", "Low-pass"),
                        ("interval_mean", "Measure mean"),
                    ][..]
                })
                .iter()
                .copied()
                .filter(|(function, _)| {
                    (self.composer == Composer::Expression || *function != "lowpass")
                        && (captured
                            || !matches!(
                                *function,
                                "sample_mean" | "sample_min" | "sample_max" | "interval_sum"
                            ))
                })
                .map(|(function, label)| {
                    Button::new(format!("preset-{function}"))
                        .label(label)
                        .small()
                        .selected(
                            measurement
                                && expression
                                    .trim_start()
                                    .strip_prefix(function)
                                    .is_some_and(|tail| tail.trim_start().starts_with('(')),
                        )
                        .disabled(!can_preset)
                        .debug_selector(move || format!("preset-{function}"))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.preset(function, window, cx);
                        }))
                }),
            ),
        );
        let form = div()
            .min_w_0()
            .p(t.px(12.0))
            .flex()
            .flex_col()
            .gap_3()
            .child(self.sampling_controls(cx))
            .when(self.composer == Composer::Expression, |el| {
                el.child(self.input_controls(&inputs, cx))
            })
            .when(default_step, |el| {
                el.child(field(
                    "Default sampling step",
                    &self.interval,
                    "formula-default-step",
                ))
            })
            .when_some(boundaries, |el, boundaries| {
                el.child(boundaries).child(
                    Button::new("formula-unknown")
                        .label(if self.skip_unknown {
                            "Missing values: skip unknown"
                        } else {
                            "Missing values: propagate gaps"
                        })
                        .small()
                        .dropdown_caret(true)
                        .dropdown_menu({
                            let view = cx.entity().downgrade();
                            move |menu, _, _| {
                                [(false, "Propagate gaps"), (true, "Skip unknown")]
                                    .into_iter()
                                    .fold(menu, |menu, (skip, label)| {
                                        let view = view.clone();
                                        menu.item(
                                            PopupMenuItem::new(label)
                                                .checked(skip == skip_unknown)
                                                .on_click(move |_, window, cx| {
                                                    drop(view.update(cx, |view, cx| {
                                                        view.skip_unknown = skip;
                                                        view.update_draft(window, cx);
                                                        view.send(
                                                            DerivedCommand::Preview,
                                                            window,
                                                            cx,
                                                        );
                                                    }));
                                                }),
                                        )
                                    })
                            }
                        }),
                )
            })
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .debug_selector(|| "measure-output-domain".into())
                    .child(format!("Output domain · {bounds}")),
            );
        let status_bar = div()
            .id("derived-status")
            .debug_selector(|| "derived-status".into())
            .flex_none()
            .px(t.px(10.0))
            .py_1()
            .border_b_1()
            .border_color(t.border)
            .text_color(if failed {
                t.panel.error
            } else {
                t.panel.text_muted
            })
            .child(status);
        let guide_height = (window.viewport_size().height
            * if has_diagnostic { 0.12 } else { 0.22 })
        .min(t.px(260.0))
        .max(t.px(80.0));
        let guide = div()
            .flex_none()
            .border_t_1()
            .border_color(t.border)
            .child(
                Button::new("formula-guide-toggle")
                    .debug_selector(|| "formula-guide-toggle".into())
                    .label(if self.guide == GuideVisibility::Open {
                        "Formula guide · Hide"
                    } else {
                        "Formula guide · Show"
                    })
                    .ghost()
                    .small()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.guide = if view.guide == GuideVisibility::Open {
                            GuideVisibility::Closed
                        } else {
                            GuideVisibility::Open
                        };
                        cx.notify();
                    })),
            )
            .when(self.guide == GuideVisibility::Open, |el| {
                el.child(div().px(t.px(12.0)).pb(t.px(8.0)).flex().flex_col().gap_2()
                    .child(div().id("formula-guide-search").debug_selector(|| "formula-guide-search".into())
                        .child(Input::new(&self.reference_search).small().w_full()))
                    .child(div().text_color(t.panel.text_muted).whitespace_normal()
                        .child("Replace formula uses your bound aliases where possible. Insert adds an example at the cursor.")))
                    .child(reference::render(&t, guide_height, &cx.entity(), &self.reference_search.read(cx).value()))
            });
        let body = div()
            .id("derived-form-scroll")
            .debug_selector(|| "derived-form-scroll".into())
            .min_h_0()
            .when(table_preview.is_none(), gpui_kit::Styled::flex_1)
            .when(table_preview.is_some(), |el| el.min_h(t.px(48.)))
            .child(
                div()
                    .flex()
                    .items_start()
                    .when(narrow, gpui_kit::Styled::flex_col)
                    .child(
                        div()
                            .min_w_0()
                            .when(narrow, gpui_kit::Styled::w_full)
                            .when(!narrow, gpui_kit::Styled::flex_1)
                            .child(composer)
                            .when(self.composer == Composer::Expression, |el| el.child(guide))
                            .children(self.pinned_readings(measurement, cx)),
                    )
                    .child(
                        form.when(narrow, gpui_kit::Styled::w_full)
                            .when(!narrow, gpui_kit::Styled::flex_1),
                    ),
            );
        let body = if table_preview.is_some() {
            body.overflow_y_scroll().into_any_element()
        } else {
            body.overflow_y_scrollbar().into_any_element()
        };
        div()
            .id("derived-signal-editor")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    view.send(DerivedCommand::Close, window, cx);
                    cx.stop_propagation();
                }
            }))
            .debug_selector(|| "derived-signal-editor".into())
            .w_full()
            .h(gpui_kit::relative(if self.expanded { 0.72 } else { 0.46 }))
            .min_h_0()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(t.panel.bg)
            .border_t_1()
            .border_color(t.border)
            .text_color(t.panel.text)
            .text_size(px(t.ui_size_small))
            .child(header)
            .child(status_bar)
            .child(body)
            .children(table_preview)
            .into_any_element()
    }
}
