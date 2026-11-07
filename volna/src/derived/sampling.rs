//! Sampling choices share the recorded-input picker and keep captures visible in the expression.

use super::*;
use volna_core::{sampling::SamplingMode, trace::Traced};
use volna_trace::data::VarId;

impl DerivedEditor {
    pub(super) fn choose_sampling(
        &mut self,
        var: Traced<VarId>,
        mode: SamplingMode,
        make_default: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_sampling(
            DerivedCommand::Sample {
                var,
                mode,
                make_default,
            },
            window,
            cx,
        );
    }

    fn set_sampling(
        &mut self,
        command: DerivedCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preview_task = None;
        self.update_draft(window, cx);
        let revision = self.owner.upgrade().and_then(|owner| {
            owner
                .read(cx)
                .app
                .derived_editor
                .as_ref()
                .map(|editor| editor.calculation_revision)
        });
        self.send(command, window, cx);
        let applied = self.owner.upgrade().is_some_and(|owner| {
            owner
                .read(cx)
                .app
                .derived_editor
                .as_ref()
                .is_some_and(|editor| Some(editor.calculation_revision) != revision)
        });
        if !applied {
            cx.notify();
            return;
        }
        let formula = self.owner.upgrade().and_then(|owner| {
            owner
                .read(cx)
                .app
                .derived_editor
                .as_ref()
                .map(|editor| editor.draft.formula.clone())
        });
        if let Some(formula) = formula {
            self.expression
                .update(cx, |input, cx| input.set_value(formula, window, cx));
            self.expression.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.close_picker();
        self.sync_bindings(window, cx);
        cx.notify();
    }

    pub(super) fn sampling_controls(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(owner) = self.owner.upgrade() else {
            return div().into_any_element();
        };
        let ws = owner.read(cx);
        let Some(editor) = &ws.app.derived_editor else {
            return div().into_any_element();
        };
        let current = editor.sampling(&ws.app.doc);
        let custom = current.is_none() && editor.draft.formula.contains('@');
        let label = current.as_ref().map_or_else(
            || {
                if custom {
                    "Custom sampling · edit expression".into()
                } else {
                    "@* · Any input change".into()
                }
            },
            volna_core::sampling::SamplingChoice::label,
        );
        let choices: Vec<_> = ws
            .app
            .doc
            .sampling
            .iter()
            .map(|choice| {
                (
                    choice.clone(),
                    choice.resolve(&ws.app.doc),
                    ws.app.doc.sampling.default_for(choice.trace) == Some(choice),
                )
            })
            .collect();
        let is_default = current
            .as_ref()
            .is_some_and(|c| ws.app.doc.sampling.default_for(c.trace) == Some(c));
        let sampling_owner = self.owner.clone();
        let generation = self.generation;
        let mut controls = div().flex().flex_col().gap_1().child("Sampling").child(
            Button::new("formula-sampling")
                .label(label.clone())
                .tooltip(label)
                .w_full()
                .overflow_hidden()
                .small()
                .dropdown_caret(true)
                .debug_selector(|| "formula-sampling".into())
                .dropdown_menu({
                    let view = cx.entity().downgrade();
                    move |mut menu, window, cx| {
                        menu = menu.scrollable(true).max_h((window.viewport_size().height - theme(cx).px(16.0)).max(px(0.0)));
                        let input_changes_view = view.clone();
                        menu = menu
                            .item(
                                PopupMenuItem::element(|_, _| {
                                    div()
                                        .id("sampling-input-changes")
                                        .debug_selector(|| "sampling-input-changes".into())
                                        .aria_label("@* · Any input change")
                                        .child("@* · Any input change")
                                })
                                .checked(current.is_none() && !custom)
                                .on_click(move |_, window, cx| {
                                    drop(input_changes_view.update(cx, |view, cx| {
                                        view.set_sampling(
                                            DerivedCommand::SampleOnInputChanges,
                                            window,
                                            cx,
                                        );
                                    }));
                                }),
                            )
                            .separator();
                        for (choice, var, default) in &choices {
                            let label = format!(
                                "{}{}",
                                choice.label(),
                                if *default { " · default" } else { "" }
                            );
                            if var.is_none() {
                                let owner = sampling_owner.clone();
                                let choice = choice.clone();
                                menu = menu.item(PopupMenuItem::new(format!("Remove unavailable · {}", choice.label())).on_click(move |_, window, cx| {
                                    drop(owner.update(cx, |ws, cx| ws.dispatch_if_current(generation, Command::Sampling(volna_core::sampling::SamplingCommand::Remove(choice.clone())), Some(window), cx)));
                                }));
                                continue;
                            }
                            let var = *var;
                            let mode = choice.mode;
                            let view = view.clone();
                            let selector = format!(
                                "sampling-choice-{}-{}-{:?}",
                                choice.trace,
                                choice.path.join("."),
                                mode
                            );
                            menu = menu.item(
                                PopupMenuItem::element(move |_, _| {
                                    div()
                                        .id(gpui_kit::SharedString::from(selector.clone()))
                                        .debug_selector({
                                            let selector = selector.clone();
                                            move || selector
                                        })
                                        .aria_label(label.clone())
                                        .child(label.clone())
                                })
                                .checked(current.as_ref() == Some(choice))
                                .disabled(var.is_none())
                                .on_click(move |_, window, cx| {
                                    if let Some(var) = var {
                                        drop(view.update(cx, |view, cx| {
                                            view.choose_sampling(var, mode, false, window, cx);
                                        }));
                                    }
                                }),
                            );
                        }
                        let view = view.clone();
                        menu.separator().item(
                            PopupMenuItem::element(|_, _| {
                                div()
                                    .id("choose-sampling-signal")
                                    .debug_selector(|| "choose-sampling-signal".into())
                                    .aria_label("Choose another signal…")
                                    .child("Choose another signal…")
                            })
                            .on_click(move |_, window, cx| {
                                drop(view.update(cx, |view, cx| {
                                    view.sampling_default = false;
                                    view.open_picker(InputPicker::Sampling, window, cx);
                                }));
                            }),
                        )
                    }
                }),
        );
        if is_default {
            controls = controls.child(
                div()
                    .text_size(px(theme(cx).ui_size_small))
                    .text_color(theme(cx).panel.text_muted)
                    .child("Trace default · saved in this formula"),
            );
        }
        if self.picker == Some(InputPicker::Sampling) {
            controls = controls.child(self.recorded_picker(cx));
        }
        controls.into_any_element()
    }
}
