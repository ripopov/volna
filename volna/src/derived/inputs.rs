//! Recorded binding controls; validation and interpretation belong to the core draft.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InputPicker {
    Add,
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
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.open_picker(InputPicker::Replace(index), cx);
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
                .on_click(cx.listener(|view, _, _, cx| {
                    view.open_picker(InputPicker::Add, cx);
                })),
        );
        if let Some(target) = self.picker {
            let candidates = self.input_search.results.as_deref().unwrap_or_default();
            let loading = self.input_search.results.is_none();
            let empty = candidates.is_empty();
            let mut picker = div().flex().flex_col().gap_1().child(
                div()
                    .debug_selector(|| "recorded-input-search".into())
                    .child(Input::new(&self.search).small()),
            );
            for candidate in candidates {
                let var = candidate.var;
                let label = candidate.label.clone();
                picker = picker.child(
                    Button::new(gpui_kit::SharedString::from(format!(
                        "recorded-candidate-{}-{}",
                        var.trace, var.item
                    )))
                    .label(label)
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
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.close_picker();
                            cx.notify();
                        })),
                ),
            );
        }
        result.into_any_element()
    }
}
