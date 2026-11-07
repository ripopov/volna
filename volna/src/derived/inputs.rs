//! Recorded binding controls; validation and interpretation belong to the core draft.

use super::*;
use gpui_kit::component::checkbox::Checkbox;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InputPicker {
    Add,
    Sampling,
    Replace(usize),
}

pub(super) struct BindingFields {
    source: volna_core::wave::model::RowSource,
    synced_alias: String,
    synced_slice: String,
    alias: Entity<InputState>,
    expanded: bool,
    slice: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}
impl BindingFields {
    fn matches(&self, input: &volna_core::computed::InputDraft) -> bool {
        self.source == input.source
            && self.synced_alias == input.alias
            && self.synced_slice == input.slice
    }
}
impl DerivedEditor {
    pub(super) fn sync_bindings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let Some(editor) = owner.read(cx).app.derived_editor.as_ref() else {
            return;
        };
        let inputs = editor.bindings.clone();
        if self.bindings.len() == inputs.len()
            && self
                .bindings
                .iter()
                .zip(&inputs)
                .all(|(fields, input)| fields.matches(input))
        {
            return;
        }
        let mut previous = std::mem::take(&mut self.bindings)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        self.bindings = inputs
            .iter()
            .enumerate()
            .map(|(index, input)| {
                if let Some(fields) = previous
                    .get_mut(index)
                    .and_then(Option::take)
                    .filter(|fields| fields.matches(input))
                {
                    return fields;
                }
                let alias =
                    cx.new(|cx| InputState::new(window, cx).default_value(input.alias.clone()));
                let slice = cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(input.slice.clone())
                        .placeholder("Whole signal, or 7:0")
                });
                let subscriptions = [&alias, &slice]
                    .into_iter()
                    .map(|field| {
                        cx.subscribe_in(field, window, move |view, _, event, window, cx| {
                            if matches!(event, InputEvent::Change) {
                                view.update_binding(index, window, cx);
                                view.schedule_preview(window, cx);
                            }
                            if matches!(event, InputEvent::PressEnter { .. }) {
                                view.update_binding(index, window, cx);
                                view.preview_task = None;
                                view.send(DerivedCommand::Preview, window, cx);
                            }
                        })
                    })
                    .collect();
                BindingFields {
                    source: input.source.clone(),
                    synced_alias: input.alias.clone(),
                    synced_slice: input.slice.clone(),
                    alias,
                    expanded: false,
                    slice,
                    _subscriptions: subscriptions,
                }
            })
            .collect();
    }
    fn update_binding(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let Some(input) = owner
            .read(cx)
            .app
            .derived_editor
            .as_ref()
            .and_then(|e| e.bindings.get(index))
            .cloned()
        else {
            return;
        };
        let Some(fields) = self.bindings.get_mut(index) else {
            return;
        };
        let alias = fields.alias.read(cx).value().to_string();
        let slice = fields.slice.read(cx).value().to_string();
        fields.synced_alias.clone_from(&alias);
        fields.synced_slice.clone_from(&slice);
        self.send(
            DerivedCommand::SetInput {
                index,
                alias,
                kind: input.kind,
                slice,
            },
            window,
            cx,
        );
    }
    pub(super) fn input_controls(
        &mut self,
        inputs: &[volna_core::computed::InputDraft],
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        use volna_core::data::NumericKind;
        let t = *theme(cx);
        let mut result = div()
            .flex()
            .flex_col()
            .gap_2()
            .child("Inputs · click an alias to insert");
        for (index, input) in inputs.iter().enumerate() {
            let Some(fields) = self.bindings.get(index) else {
                continue;
            };
            let owner = self.owner.upgrade().unwrap();
            let doc = &owner.read(cx).app.doc;
            let shape = input.effective_shape(doc);
            let mut formats = div().flex().flex_wrap().gap_1();
            let kinds = if shape == volna_trace::data::SignalShape::Real {
                vec![NumericKind::Real]
            } else if shape == volna_trace::data::SignalShape::Bit {
                vec![NumericKind::Unsigned]
            } else {
                let mut kinds = vec![NumericKind::Unsigned, NumericKind::Signed];
                if matches!(shape.width(), 16 | 32 | 64) {
                    kinds.push(NumericKind::Float);
                }
                kinds
            };
            for kind in kinds {
                let label = if shape == volna_trace::data::SignalShape::Bit {
                    "Boolean"
                } else {
                    volna_core::computed::Interpretation {
                        kind,
                        projection: None,
                    }
                    .label()
                };
                let slice = input.slice.clone();
                let alias = input.alias.clone();
                formats = formats.child(
                    Button::new(("input-format", index * 4 + kind as usize))
                        .label(label)
                        .xsmall()
                        .selected(
                            input.kind == kind || shape == volna_trace::data::SignalShape::Bit,
                        )
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.preview_task = None;
                            view.send(
                                DerivedCommand::SetInput {
                                    index,
                                    alias: alias.clone(),
                                    kind,
                                    slice: slice.clone(),
                                },
                                window,
                                cx,
                            );
                            view.send(DerivedCommand::Preview, window, cx);
                        })),
                );
            }
            let alias = input.alias.clone();
            let expanded = fields.expanded;
            let summary = div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    Button::new(("insert-alias", index))
                        .label(input.alias.clone())
                        .small()
                        .debug_selector(move || format!("insert-input-{index}"))
                        .tooltip("Insert alias at the formula cursor")
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.insert_formula(&alias, window, cx);
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(t.panel.text_muted)
                        .child(input.label(doc)),
                )
                .child(if shape == volna_trace::data::SignalShape::Bit {
                    "Boolean"
                } else {
                    volna_core::computed::Interpretation {
                        kind: input.kind,
                        projection: None,
                    }
                    .label()
                })
                .child(
                    Button::new(("input-details", index))
                        .label(if expanded { "Hide" } else { "Edit" })
                        .ghost()
                        .xsmall()
                        .debug_selector(move || format!("input-details-{index}"))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if let Some(fields) = view.bindings.get_mut(index) {
                                fields.expanded = !fields.expanded;
                            }
                            cx.notify();
                        })),
                );
            let mut details = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child("Alias")
                        .debug_selector(move || format!("recorded-input-{index}-alias"))
                        .child(Input::new(&fields.alias).small()),
                )
                .child(formats);
            if input.shape.is_digital() && input.shape.width() > 1 {
                details = details.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child("Bit or slice (optional)")
                        .debug_selector(move || format!("recorded-input-{index}-slice"))
                        .child(Input::new(&fields.slice).small()),
                );
            }
            details = details.child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new(("rebind-input", index))
                            .label("Replace…")
                            .ghost()
                            .xsmall()
                            .debug_selector(move || format!("replace-input-{index}"))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.open_picker(InputPicker::Replace(index), window, cx);
                            })),
                    )
                    .child(
                        Button::new(("remove-input", index))
                            .label("Remove")
                            .ghost()
                            .xsmall()
                            .debug_selector(move || format!("remove-input-{index}"))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.send(DerivedCommand::RemoveInput(index), window, cx);
                                view.send(DerivedCommand::Preview, window, cx);
                            })),
                    ),
            );
            let binding = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .border_1()
                .border_color(t.border)
                .child(summary)
                .when(expanded, |el| el.child(details));
            result = result.child(binding);
        }
        result = result.child(
            Button::new("add-formula-input")
                .label("Add recorded input…")
                .small()
                .disabled(inputs.len() >= 16)
                .debug_selector(|| "add-formula-input".into())
                .on_click(cx.listener(|view, _, window, cx| {
                    view.open_picker(InputPicker::Add, window, cx);
                })),
        );
        if self.picker != Some(InputPicker::Sampling) {
            result = result.child(self.recorded_picker(cx));
        }
        result.into_any_element()
    }
    pub(super) fn recorded_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut result = div().flex().flex_col().gap_2();
        if let Some(target) = self.picker {
            let candidates = self.input_search.results.as_deref().unwrap_or_default();
            let loading = self.input_search.results.is_none();
            let empty = candidates.is_empty();
            let mut picker = div()
                .flex()
                .flex_col()
                .gap_1()
                .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        view.close_picker();
                        view.expression.read(cx).focus_handle(cx).focus(window, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
                .child(
                    div()
                        .debug_selector(|| "recorded-input-search".into())
                        .child(Input::new(&self.search).small()),
                );
            if target == InputPicker::Sampling {
                picker = picker.child(
                    Checkbox::new("sampling-use-default")
                        .debug_selector(|| "sampling-use-default".into())
                        .label("Make this the trace default")
                        .checked(self.sampling_default)
                        .on_click(cx.listener(|view, checked, _, cx| {
                            view.sampling_default = *checked;
                            cx.notify();
                        })),
                );
            }
            if target == InputPicker::Sampling {
                picker = picker.child(
                    div()
                        .text_color(theme(cx).panel.text_muted)
                        .child("The first choice for each trace becomes its default."),
                );
            }
            for candidate in candidates {
                if target == InputPicker::Sampling {
                    let var = candidate.var;
                    let Some(owner) = self.owner.upgrade() else {
                        continue;
                    };
                    let Some(shape) = owner
                        .read(cx)
                        .app
                        .doc
                        .hierarchy(var.trace)
                        .and_then(|h| h.get_var(var.item))
                        .map(|v| v.shape)
                    else {
                        continue;
                    };
                    if !volna_core::sampling::SamplingMode::ValueChanges.accepts(shape) {
                        continue;
                    }
                    let view = cx.entity().downgrade();
                    let selector = format!("sampling-candidate-{}", candidate.label);
                    picker = picker.child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "sampling-candidate-{}-{}",
                            var.trace, var.item
                        )))
                        .label(candidate.label.clone())
                        .tooltip(candidate.label.clone())
                        .w_full()
                        .overflow_hidden()
                        .debug_selector(move || selector)
                        .ghost()
                        .xsmall()
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            for mode in volna_core::sampling::SamplingMode::ALL
                                .into_iter()
                                .filter(|mode| mode.accepts(shape))
                            {
                                let view = view.clone();
                                menu = menu.item(
                                    PopupMenuItem::element(move |_, _| {
                                        div()
                                            .id(gpui_kit::SharedString::from(format!(
                                                "sampling-mode-{mode:?}"
                                            )))
                                            .debug_selector(move || {
                                                format!("sampling-mode-{mode:?}")
                                            })
                                            .aria_label(mode.label())
                                            .child(mode.label())
                                    })
                                    .on_click(
                                        move |_, window, cx| {
                                            drop(view.update(cx, |view, cx| {
                                                view.choose_sampling(
                                                    var,
                                                    mode,
                                                    view.sampling_default,
                                                    window,
                                                    cx,
                                                );
                                            }));
                                        },
                                    ),
                                );
                            }
                            menu
                        }),
                    );
                    continue;
                }

                let var = candidate.var;
                let label = candidate.label.clone();
                picker = picker.child(
                    Button::new(gpui_kit::SharedString::from(format!(
                        "recorded-candidate-{}-{}",
                        var.trace, var.item
                    )))
                    .label(label.clone())
                    .tooltip(label)
                    .w_full()
                    .overflow_hidden()
                    .debug_selector(move || {
                        format!("recorded-candidate-{}-{}", var.trace, var.item)
                    })
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        if view.picker != Some(target) {
                            return;
                        }
                        view.send(
                            match target {
                                InputPicker::Sampling => unreachable!(),
                                InputPicker::Add => DerivedCommand::AddInput(var),
                                InputPicker::Replace(index) => DerivedCommand::Rebind(index, var),
                            },
                            window,
                            cx,
                        );
                        view.picker = None;
                        view.send(DerivedCommand::Preview, window, cx);
                    })),
                );
            }
            if candidates.len() == 20 {
                picker = picker.child(
                    div()
                        .text_color(theme(cx).panel.text_muted)
                        .child("Showing up to 20 matches · refine the signal path to find more."),
                );
            }
            if loading {
                picker = picker.child("Searching recorded signals…");
            } else if empty {
                picker = picker.child("No matching recorded signals.");
            }
            result = result.child(
                picker.child(
                    Button::new("close-input-picker")
                        .label("Close picker")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.close_picker();
                            view.expression.read(cx).focus_handle(cx).focus(window, cx);
                            cx.notify();
                        })),
                ),
            );
        }
        result.into_any_element()
    }
}
