//! Native attribute trees shared by records, selected stages, and embedded events.

use super::*;
use volna_core::transaction::attributes::{AttributeId, AttributeTree};
use volna_core::transaction::{AttrRow, StageDetails};

impl TransactionPanelView {
    pub(super) fn attributes(
        &mut self,
        t: &crate::theme::Theme,
        view: &TxView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tree = &view.attributes;
        let mut section = Self::attribute_section(
            t,
            tree,
            if view.point_kind.is_some() {
                "Arguments"
            } else {
                "Attributes"
            },
            cx,
        );
        if !tree.collapsed {
            section = section
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .pb_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&self.filter).small()),
                        )
                        .child(
                            action_button(
                                "tx-expand-all",
                                gpui_kit::component::Icon::empty()
                                    .path(IconName::ChevronsRight.path()),
                                cx,
                            )
                            .accessibility_label("Expand all attributes")
                            .tooltip("Expand all attributes")
                            .ghost()
                            .disabled(tree.filtering)
                            .on_click(cx.listener(
                                |view, _, window, cx| {
                                    view.dispatch(
                                        TransactionCommand::ExpandAttributes(true),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .child(
                            action_button(
                                "tx-collapse-all",
                                gpui_kit::component::Icon::empty()
                                    .path(IconName::ChevronsLeft.path()),
                                cx,
                            )
                            .accessibility_label("Collapse all attributes")
                            .tooltip("Collapse all attributes")
                            .ghost()
                            .disabled(tree.filtering)
                            .on_click(cx.listener(
                                |view, _, window, cx| {
                                    view.dispatch(
                                        TransactionCommand::ExpandAttributes(false),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        ),
                )
                .child(self.attribute_tree(t, tree, "tx-attr", cx));
        }
        section.into_any_element()
    }

    fn attribute_section(
        t: &crate::theme::Theme,
        tree: &AttributeTree,
        title: &str,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let heading = Section::<()> {
            key: tree.key,
            total: tree.total,
            matched: tree.total,
            collapsed: tree.collapsed,
            rows: Vec::new(),
        };
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .border_t_1()
            .border_color(t.border_variant)
            .child(Self::named_heading(t, &heading, title, cx))
    }

    pub(super) fn stage_attributes(
        &mut self,
        t: &crate::theme::Theme,
        stage: &StageDetails,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tree = &stage.attributes;
        let mut section = Self::attribute_section(t, tree, "Stage attributes", cx)
            .debug_selector(|| "tx-stage-attributes".into());
        if !tree.collapsed {
            section = section
                .child(
                    detail_text(cx, "tx-stage-attributes-name", stage.name.clone())
                        .px_3()
                        .py_1()
                        .font_family(t.mono_font),
                )
                .child(self.attribute_tree(t, tree, "tx-stage-attr", cx));
        }
        section.into_any_element()
    }

    pub(super) fn events(
        &mut self,
        t: &crate::theme::Theme,
        section: &Section<EventRow>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut element = div()
            .flex()
            .flex_col()
            .min_w_0()
            .border_t_1()
            .border_color(t.border_variant)
            .child(Self::heading(t, section, cx));
        if !section.collapsed {
            for (i, event) in section.rows.iter().enumerate() {
                let time = event.time;
                element = element.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .child(
                            detail_text(cx, format!("tx-event-name-{i}"), event.name.clone())
                                .flex_1()
                                .font_family(t.mono_font),
                        )
                        .child(
                            Button::new(SharedString::from(format!("tx-event-{i}")))
                                .label(event.time_text.clone())
                                .ghost()
                                .small()
                                .tooltip("Move cursor to this event")
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.dispatch(TransactionCommand::Cursor(time), window, cx);
                                })),
                        ),
                );
                if event.attributes.total > 0 {
                    element = element.child(self.attribute_tree(
                        t,
                        &event.attributes,
                        "tx-event-attr",
                        cx,
                    ));
                }
            }
            if let Some(note) = Self::cut_note(t, section) {
                element = element.child(note);
            }
        }
        element.into_any_element()
    }

    fn attribute_tree(
        &mut self,
        t: &crate::theme::Theme,
        tree: &AttributeTree,
        prefix: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut element = div()
            .id(SharedString::from(format!("tx-attr-tree-{}", tree.owner)))
            .flex()
            .flex_col()
            .min_w_0()
            .pb_1()
            .role(gpui_kit::accesskit::Role::Tree)
            .aria_label("Recorded attributes");
        for (index, row) in tree.rows.iter().enumerate() {
            let focus = self
                .attribute_focus
                .entry(row.id.clone())
                .or_insert_with(|| cx.focus_handle().tab_stop(true))
                .clone();
            element = element.child(Self::attribute_row(
                t,
                row,
                prefix,
                index,
                &focus,
                tree.filtering,
                cx,
            ));
        }
        if tree.rows.is_empty() {
            element = element.child(div().px_3().py_2().text_color(t.panel.text_muted).child(
                if tree.limited {
                    "The tree exceeds the display limit. Copy a parent value to inspect its contents."
                } else if tree.filtering && tree.has_next() {
                    "Search is in progress. Continue to search the next section."
                } else if tree.filtering {
                    "No matching fields or values."
                } else { "No attributes." },
            ));
        }
        if tree.limited && !tree.rows.is_empty() {
            element = element.child(div().px_3().py_2().text_color(t.panel.text_muted)
                .child("Remaining details exceed the display limit. Copy a parent value for the complete data."));
        }
        if tree.page > 0 || tree.has_next() {
            let owner = tree.owner;
            let page = tree.page;
            element = element.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .flex_wrap()
                    .gap_1()
                    .px_3()
                    .py_1()
                    .text_size(px(t.ui_size_small))
                    .text_color(t.panel.text_muted)
                    .child(SharedString::from(format!("Page {}", page + 1)))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!("tx-attr-first-{owner}")))
                                    .label("First")
                                    .small()
                                    .ghost()
                                    .disabled(page == 0)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.dispatch(
                                            TransactionCommand::AttributePage(owner, 0),
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("tx-attr-prev-{owner}")))
                                    .label("Previous")
                                    .small()
                                    .ghost()
                                    .disabled(!tree.has_previous())
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.dispatch(
                                            TransactionCommand::AttributePage(
                                                owner,
                                                page.saturating_sub(1),
                                            ),
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("tx-attr-next-{owner}")))
                                    .label(if tree.filtering && tree.rows.is_empty() {
                                        "Continue search"
                                    } else {
                                        "Next"
                                    })
                                    .small()
                                    .ghost()
                                    .disabled(!tree.has_next())
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.dispatch(
                                            TransactionCommand::AttributePage(owner, page + 1),
                                            window,
                                            cx,
                                        );
                                    })),
                            ),
                    ),
            );
        }
        element.into_any_element()
    }

    fn copy_attribute(&self, id: &AttributeId, cx: &mut Context<Self>) {
        let Some(owner) = self.ws.upgrade() else {
            return;
        };
        let ws = owner.read(cx);
        let result = ws
            .app
            .panels
            .transaction(self.id)
            .map(|model| model.copy_attribute(&ws.app.doc, id));
        Workspace::copy_result(&owner, self.generation, result, cx);
    }

    fn copy_attribute_path(&self, id: &AttributeId, cx: &mut Context<Self>) {
        let Some(owner) = self.ws.upgrade() else {
            return;
        };
        let ws = owner.read(cx);
        let result = ws
            .app
            .panels
            .transaction(self.id)
            .map(|model| model.copy_attribute_path(&ws.app.doc, id));
        Workspace::copy_result(&owner, self.generation, result, cx);
    }
    fn show_attribute_column(&self, id: &AttributeId, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.ws.upgrade().and_then(|owner| {
            let ws = owner.read(cx);
            ws.app
                .panels
                .transaction(self.id)?
                .attribute_column(&ws.app.doc, id)
        });
        if let Some(key) = key {
            self.send(
                Command::ShowAttributeColumn { from: self.id, key },
                window,
                cx,
            );
        }
    }

    fn attribute_row(
        t: &crate::theme::Theme,
        row: &AttrRow,
        prefix: &str,
        index: usize,
        focus: &FocusHandle,
        filtering: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let branch = row.children > 0;
        let key_selector = format!("{prefix}-key-{index}");
        let value_selector = format!("{prefix}-{index}");
        let id = row.id.clone();
        let click_id = id.clone();
        let key_id = id.clone();
        let click_radix = id.clone();
        let expanded = row.expanded;
        let numeric = row.radix.is_some();
        let click_focus = focus.clone();
        let key = detail_text(
            cx,
            SharedString::from(key_selector.clone()),
            row.key.clone(),
        )
        .debug_selector(move || key_selector)
        .flex_1()
        .text_color(if branch {
            t.panel.text
        } else {
            t.panel.text_muted
        });
        let value = div()
            .id(SharedString::from(value_selector.clone()))
            .debug_selector(move || value_selector)
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_normal()
            .font_family(t.mono_font)
            .when_some(row.radix, |el, radix| {
                let hint = format!("{} · click or press Enter to change radix", radix.name());
                el.tooltip_show_delay(crate::ui::tooltip::delay(cx))
                    .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
            })
            .when(branch, |el| {
                el.text_color(t.panel.text_muted)
                    .text_size(px(t.ui_size_small))
            })
            .child(SharedString::from(row.value.clone()))
            .when(row.truncated, |el| {
                el.child(
                    div()
                        .text_color(t.panel.text_muted)
                        .text_size(px(t.ui_size_small))
                        .child("Preview · Copy value for full content"),
                )
            });
        let owner = cx.entity().downgrade();
        let copy_id = id.clone();
        let row_id = id.clone();
        let column = row.depth == 0 && prefix == "tx-attr";
        div()
            .id(SharedString::from(format!("tx-attr-node-{id}")))
            .debug_selector(move || format!("tx-attr-node-{id}"))
            .track_focus(focus)
            .tab_stop(true)
            .role(gpui_kit::accesskit::Role::TreeItem)
            .aria_label(format!(
                "{}: {}{}",
                row.path,
                row.value,
                if branch {
                    if expanded {
                        ", expanded"
                    } else {
                        ", collapsed"
                    }
                } else {
                    ""
                }
            ))
            .flex()
            .items_start()
            .min_w_0()
            .gap_2()
            .px_3()
            .py_0p5()
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .hover(|style| style.bg(t.hover.bg))
            .focus_visible(|style| style.border_color(t.border_focused).bg(t.selection.bg))
            .when(branch || numeric, gpui_kit::Styled::cursor_pointer)
            .on_click(cx.listener(move |view, _, window, cx| {
                window.focus(&click_focus, cx);
                if branch && !filtering {
                    view.dispatch(
                        TransactionCommand::ToggleAttribute(click_id.clone()),
                        window,
                        cx,
                    );
                } else if numeric {
                    view.dispatch(
                        TransactionCommand::AttributeRadix(click_radix.clone()),
                        window,
                        cx,
                    );
                }
                cx.stop_propagation();
            }))
            .on_key_down(
                cx.listener(move |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "tab" => {
                            if event.keystroke.modifiers.shift {
                                window.focus_prev(cx);
                            } else {
                                window.focus_next(cx);
                            }
                        }
                        "up" => window.focus_prev(cx),
                        "down" => window.focus_next(cx),
                        "right" if branch && expanded => window.focus_next(cx),
                        "left" if !branch || !expanded => {
                            if let Some(parent) = key_id.parent()
                                && let Some(focus) = view.attribute_focus.get(&parent)
                            {
                                window.focus(focus, cx);
                            }
                        }
                        "enter" | "space" if branch && !filtering => view.dispatch(
                            TransactionCommand::ToggleAttribute(key_id.clone()),
                            window,
                            cx,
                        ),
                        "right" if branch && !expanded && !filtering => view.dispatch(
                            TransactionCommand::ToggleAttribute(key_id.clone()),
                            window,
                            cx,
                        ),
                        "left" if branch && expanded && !filtering => view.dispatch(
                            TransactionCommand::ToggleAttribute(key_id.clone()),
                            window,
                            cx,
                        ),
                        "enter" | "space" if numeric => {
                            view.dispatch(
                                TransactionCommand::AttributeRadix(key_id.clone()),
                                window,
                                cx,
                            );
                        }
                        "c" if event.keystroke.modifiers.control
                            || event.keystroke.modifiers.platform =>
                        {
                            view.copy_attribute(&key_id, cx);
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .w(relative(0.46))
                    .min_w_0()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .w(relative(row.depth.min(8) as f32 * 0.04))
                            .max_w(t.px((row.depth.min(8) * 14) as f32))
                            .flex_none(),
                    )
                    .child(div().w(t.px(12.0)).flex_none().when(branch, |el| {
                        el.child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(t.px(12.0))
                            .color(t.panel.icon_muted),
                        )
                    }))
                    .child(key)
                    .when_some(row.phase, |el, phase| el.child(tag(t, phase))),
            )
            .child(value)
            .context_menu(move |menu, _, _| {
                let value_owner = owner.clone();
                let copy_id = copy_id.clone();
                let mut menu =
                    menu.item(PopupMenuItem::new("Copy value").on_click(move |_, _, cx| {
                        drop(value_owner.update(cx, |view, cx| view.copy_attribute(&copy_id, cx)));
                    }));
                let path_owner = owner.clone();
                let path_id = row_id.clone();
                menu = menu.item(PopupMenuItem::new("Copy field path").on_click(
                    move |_, _, cx| {
                        drop(
                            path_owner
                                .update(cx, |view, cx| view.copy_attribute_path(&path_id, cx)),
                        );
                    },
                ));
                if column {
                    let owner = owner.clone();
                    let column_id = row_id.clone();
                    menu = menu.item(PopupMenuItem::new("Show as table column").on_click(
                        move |_, window, cx| {
                            drop(owner.update(cx, |view, cx| {
                                view.show_attribute_column(&column_id, window, cx);
                            }));
                        },
                    ));
                }
                menu
            })
            .into_any_element()
    }
}
