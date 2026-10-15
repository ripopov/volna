//! Searchable column controls over the loaded generator's attribute catalogue.

use crate::app::Workspace;
use crate::theme::{ThemePx, theme};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    popover::{Popover, PopoverState},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, Render, ScrollStrategy, SharedString,
    Subscription, UniformListScrollHandle, WeakEntity, Window, div, uniform_list,
};
use std::sync::Arc;
use volna_core::Command;
use volna_core::data::loaded_tracks::LoadedGenerator;
use volna_core::panels::PanelId;
use volna_core::table::columns::{ColumnSet, TransactionColumn};
use volna_core::table::{TableCommand, TableState};

gpui_kit::actions!(
    table_columns,
    [
        NextAttribute,
        PreviousAttribute,
        ToggleAttribute,
        CloseColumns
    ]
);

pub(crate) fn init(cx: &mut App) {
    use gpui_kit::{KeyBinding, NoAction};
    cx.bind_keys([
        KeyBinding::new("down", NextAttribute, Some("ColumnPicker")),
        KeyBinding::new("up", PreviousAttribute, Some("ColumnPicker")),
        KeyBinding::new("enter", NoAction, Some("ColumnPicker")),
        KeyBinding::new("space", NoAction, Some("ColumnPicker")),
        KeyBinding::new("escape", CloseColumns, Some("ColumnPicker")),
        KeyBinding::new("down", NextAttribute, Some("ColumnSearch > Input")),
        KeyBinding::new("up", PreviousAttribute, Some("ColumnSearch > Input")),
        KeyBinding::new("enter", ToggleAttribute, Some("ColumnSearch > Input")),
        KeyBinding::new("space", NoAction, Some("ColumnSearch > Input")),
        KeyBinding::new("escape", CloseColumns, Some("ColumnSearch > Input")),
    ]);
}

pub(crate) struct ColumnPicker {
    ws: WeakEntity<Workspace>,
    panel: PanelId,
    generation: u64,
    focus: FocusHandle,
    search: Entity<InputState>,
    open: bool,
    attributes: bool,
    active: usize,
    synced_query: String,
    rows: Vec<AttributeRow>,
    scroll: UniformListScrollHandle,
    popover: Option<WeakEntity<PopoverState>>,
    _subscriptions: Vec<Subscription>,
}

struct AttributeRow {
    key: String,
    column: TransactionColumn,
    value_type: &'static str,
    example_row: Option<usize>,
    checked: bool,
}

impl ColumnPicker {
    pub(crate) fn new(
        ws: WeakEntity<Workspace>,
        panel: PanelId,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if let Some(owner) = ws.upgrade() {
            cx.observe(&owner, |_, _, cx| cx.notify()).detach();
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search attributes…"));
        let subscription = cx.subscribe_in(&search, window, |view, _, event, _, cx| {
            if let InputEvent::Change = event {
                view.active = 0;
                view.scroll.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }
        });
        Self {
            ws,
            panel,
            generation,
            focus: cx.focus_handle(),
            search,
            open: false,
            attributes: false,
            active: 0,
            synced_query: String::new(),
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            popover: None,
            _subscriptions: vec![subscription],
        }
    }

    pub(crate) fn popover(picker: Entity<Self>, cx: &App) -> Popover {
        let focus = picker.read(cx).focus.clone();
        let content = picker.clone();
        Popover::new("table-columns-popover")
            .track_focus(&focus)
            .trigger(
                crate::ui::action_button(
                    "table-columns",
                    gpui_kit::component::Icon::empty()
                        .path(volna_core::icons::IconName::TableColumns.path()),
                    cx,
                )
                .label("Columns")
                .debug_selector(|| "table-columns".into())
                .ghost(),
            )
            .on_open_change(move |open, window, cx| {
                let picker = picker.clone();
                let open = *open;
                window.defer(cx, move |window, cx| {
                    picker.update(cx, |view, cx| {
                        view.open = open;
                        if open {
                            view.attributes = false;
                            view.active = 0;
                            view.search
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        cx.notify();
                    });
                });
            })
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                content.update(cx, |view, _| view.popover = Some(popover));
                div().child(content.clone())
            })
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    fn dispatch(&self, command: TableCommand, window: &mut Window, cx: &mut Context<Self>) {
        drop(self.ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(
                self.generation,
                Command::Table(self.panel, command),
                Some(window),
                cx,
            );
        }));
        cx.notify();
    }

    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(popover) = self.popover.as_ref().and_then(WeakEntity::upgrade) {
            popover.update(cx, |state, cx| state.dismiss(window, cx));
        }
    }

    fn show_attributes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.attributes = true;
        self.active = 0;
        self.search.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn sync_rows(&mut self, cx: &App) {
        let Some(owner) = self.ws.upgrade() else {
            self.rows.clear();
            return;
        };
        let Some(table) = owner
            .read(cx)
            .app
            .panels
            .get(self.panel)
            .and_then(|p| p.kind.table())
        else {
            self.rows.clear();
            return;
        };
        let ColumnSet::Transactions(columns) = table.columns.get() else {
            self.rows.clear();
            return;
        };
        let query = self.search.read(cx).value().to_lowercase();
        if self.synced_query != query {
            self.active = 0;
            self.synced_query.clone_from(&query);
        }
        let generator = table.loaded_generator().cloned();
        self.rows = generator.as_ref().map_or_else(Vec::new, |g| {
            g.attribute_catalogue()
                .iter()
                .map(|attribute| {
                    let column = if attribute.key == "vtr.label" {
                        TransactionColumn::Label
                    } else {
                        TransactionColumn::Attribute(attribute.key.clone())
                    };
                    AttributeRow {
                        key: attribute.key.clone(),
                        checked: columns.visible(&column),
                        column,
                        value_type: attribute.value_type.name(),
                        example_row: Some(attribute.example_row),
                    }
                })
                .collect()
        });
        for (_, column, checked) in columns.iter() {
            if let TransactionColumn::Attribute(key) = column
                && !self.rows.iter().any(|r| &r.key == key)
            {
                self.rows.push(AttributeRow {
                    key: key.clone(),
                    column: column.clone(),
                    checked,
                    value_type: "No values in this source",
                    example_row: None,
                });
            }
        }
        self.rows
            .retain(|row| row.key.to_lowercase().contains(&query));
        self.rows.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        self.active = self.active.min(self.rows.len().saturating_sub(1));
    }

    fn move_active(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.attributes {
            if direction > 0 {
                window.focus_next(cx);
            } else {
                window.focus_prev(cx);
            }
            cx.stop_propagation();
            return;
        }
        self.sync_rows(cx);
        if !self.rows.is_empty() {
            self.active = self
                .active
                .saturating_add_signed(direction)
                .min(self.rows.len() - 1);
            self.scroll
                .scroll_to_item(self.active, ScrollStrategy::Nearest);
            self.focus.focus(window, cx);
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn toggle_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_rows(cx);
        if let Some(row) = self.rows.get(self.active) {
            self.dispatch(
                TableCommand::ToggleTransactionColumn(row.column.clone()),
                window,
                cx,
            );
        }
    }

    fn attribute_row(
        &self,
        index: usize,
        generator: Option<&Arc<LoadedGenerator>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *theme(cx);
        let row = &self.rows[index];
        let column = row.column.clone();
        let example = row
            .example_row
            .and_then(|i| generator?.transactions().get(i))
            .and_then(|tx| tx.attributes.iter().find(|a| a.key == row.key))
            .map(|a| volna_core::data::text::format_attribute(&a.value, 256));
        let hint = example.map_or_else(
            || format!("{} · No values in this source", row.key),
            |value| format!("{} · Example: {value}", row.key),
        );
        let key = row.key.clone();
        let query = self.search.read(cx).value().to_lowercase();
        let lower = key.to_lowercase();
        // Case folding can change byte lengths; highlight only when offsets remain valid.
        let highlight = lower
            .find(&query)
            .filter(|_| !query.is_empty() && lower.len() == key.len())
            .map(|start| start..start + query.len())
            .filter(|range| key.is_char_boundary(range.start) && key.is_char_boundary(range.end));
        let key_label = gpui_kit::StyledText::new(SharedString::from(key.clone())).with_highlights(
            highlight.map(|range| {
                (
                    range,
                    gpui_kit::HighlightStyle {
                        color: Some(t.panel.icon_accent),
                        ..Default::default()
                    },
                )
            }),
        );
        div()
            .id(("attribute-row", index))
            .debug_selector(move || format!("table-attribute-{key}"))
            .h(t.px(32.0))
            .w_full()
            .rounded_sm()
            .when(index == self.active, |el| el.bg(t.hover.bg))
            .child(
                Checkbox::new(("attribute-check", index))
                    .checked(row.checked)
                    .accessibility_label(row.key.clone())
                    .tooltip(hint)
                    .w_full()
                    .h_full()
                    .px_2()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.dispatch(
                            TableCommand::ToggleTransactionColumn(column.clone()),
                            window,
                            cx,
                        );
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .font_family(t.mono_font)
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(key_label),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(gpui_kit::px(t.ui_size_small))
                                    .text_color(t.panel.text_muted)
                                    .child(if row.column == TransactionColumn::Label {
                                        "Shown as “Label”"
                                    } else {
                                        row.value_type
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for ColumnPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme(cx);
        if self.attributes {
            self.sync_rows(cx);
        }
        let Some(owner) = self.ws.upgrade() else {
            return div().into_any_element();
        };
        let Some(table) = owner
            .read(cx)
            .app
            .panels
            .get(self.panel)
            .and_then(|p| p.kind.table())
        else {
            return div().into_any_element();
        };
        let width = (window.viewport_size().width / t.zoom - gpui_kit::px(48.0))
            .min(gpui_kit::px(if self.attributes { 390.0 } else { 270.0 }));
        let accessible_label = if self.attributes {
            self.rows.get(self.active).map_or_else(
                || "Attribute columns".into(),
                |row| {
                    format!(
                        "{}, {}, {}",
                        row.key,
                        row.value_type,
                        if row.checked { "shown" } else { "hidden" }
                    )
                },
            )
        } else {
            "Columns".into()
        };
        let mut root = div()
            .id("table-column-picker")
            .debug_selector(|| "table-column-picker".into())
            .w(width * t.zoom)
            .max_h(window.viewport_size().height * 0.8)
            .flex()
            .flex_col()
            .gap_1()
            .track_focus(&self.focus)
            .role(gpui_kit::accesskit::Role::Group)
            .aria_label(accessible_label)
            .key_context("ColumnPicker")
            .on_key_down(
                cx.listener(|view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if view.attributes
                        && view.focus.is_focused(window)
                        && matches!(event.keystroke.key.as_str(), "enter" | "space")
                    {
                        view.toggle_active(window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .on_action(
                cx.listener(|view, _: &NextAttribute, window, cx| view.move_active(1, window, cx)),
            )
            .on_action(cx.listener(|view, _: &PreviousAttribute, window, cx| {
                view.move_active(-1, window, cx);
            }))
            .on_action(cx.listener(|view, _: &ToggleAttribute, window, cx| {
                if view.attributes {
                    view.toggle_active(window, cx);
                }
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|view, _: &CloseColumns, window, cx| {
                view.close(window, cx);
                cx.stop_propagation();
            }));
        match table.columns.get() {
            ColumnSet::Transactions(columns) if self.attributes => {
                let query = self.search.read(cx).value().to_lowercase();
                let generator = table.loaded_generator().cloned();
                let selected = columns
                    .iter()
                    .filter(|(_, c, shown)| *shown && matches!(c, TransactionColumn::Attribute(_)))
                    .count();
                root = root
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("attribute-back")
                                    .label("← Columns")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.attributes = false;
                                        view.focus.focus(window, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(div().flex_1().text_color(t.panel.text_muted).child(
                                SharedString::from(format!(
                                    "Attribute columns · {}",
                                    self.rows.len()
                                )),
                            )),
                    )
                    .child(
                        div()
                            .key_context("ColumnSearch")
                            .child(Input::new(&self.search).small()),
                    );
                if self.rows.is_empty() {
                    root = root.child(div().p_3().text_color(t.panel.text_muted).child(
                        if matches!(table.state, TableState::Loading) {
                            "Track is loading…"
                        } else if query.is_empty() {
                            "No transaction attributes in this source."
                        } else {
                            "No matching attributes."
                        },
                    ));
                } else {
                    let count = self.rows.len();
                    root = root.child(
                        uniform_list(
                            "table-attribute-list",
                            count,
                            cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|index| view.attribute_row(index, generator.as_ref(), cx))
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.scroll)
                        .h(t.px(32.0 * count.min(8) as f32))
                        .w_full()
                        .min_h_0(),
                    );
                }
                root = root.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_t_1()
                        .border_color(t.border_variant)
                        .pt_1()
                        .child(
                            div()
                                .text_size(gpui_kit::px(t.ui_size_small))
                                .text_color(t.panel.text_muted)
                                .child(SharedString::from(format!(
                                    "{selected} attribute {} shown",
                                    if selected == 1 { "column" } else { "columns" }
                                ))),
                        )
                        .child(
                            Button::new("attribute-done")
                                .label("Done")
                                .ghost()
                                .small()
                                .on_click(
                                    cx.listener(|view, _, window, cx| view.close(window, cx)),
                                ),
                        ),
                );
            }
            _ => {
                let items: Vec<_> = match table.columns.get() {
                    ColumnSet::Transactions(columns) => columns
                        .iter()
                        .filter(|(_, c, _)| !matches!(c, TransactionColumn::Attribute(_)))
                        .map(|(index, c, shown)| {
                            (
                                index,
                                c.title().to_owned(),
                                shown,
                                TableCommand::ToggleTransactionColumn(c.clone()),
                            )
                        })
                        .collect(),
                    ColumnSet::Signals { time, visible } => std::iter::once((
                        0,
                        "Time".into(),
                        *time,
                        TableCommand::ToggleSignalColumn(0),
                    ))
                    .chain(visible.iter().enumerate().map(|(i, shown)| {
                        (
                            i + 1,
                            table.column_title(i + 1).to_owned(),
                            *shown,
                            TableCommand::ToggleSignalColumn(i + 1),
                        )
                    }))
                    .collect(),
                };
                let mut list = div()
                    .id("standard-columns")
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .max_h(t.px(300.0));
                for (index, label, checked, command) in items {
                    list = list.child(
                        Checkbox::new(("table-column", index))
                            .label(label)
                            .checked(checked)
                            .w_full()
                            .px_2()
                            .py_1()
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.dispatch(command.clone(), window, cx);
                            })),
                    );
                }
                root = root.child(list);
                if let ColumnSet::Transactions(columns) = table.columns.get() {
                    let attributes: Vec<_> = columns
                        .iter()
                        .filter(|(_, c, _)| matches!(c, TransactionColumn::Attribute(_)))
                        .collect();
                    if !attributes.is_empty() {
                        root =
                            root.child(div().px_2().pt_2().text_color(t.panel.text_muted).child(
                                SharedString::from(format!(
                                    "Attribute columns · {}",
                                    self.rows.len()
                                )),
                            ));
                        let mut list = div()
                            .id("chosen-attributes")
                            .max_h(t.px(180.0))
                            .overflow_y_scroll();
                        for (index, column, shown) in attributes {
                            let column = column.clone();
                            list = list.child(
                                Checkbox::new(("chosen-attribute", index))
                                    .label(column.title().to_owned())
                                    .checked(shown)
                                    .w_full()
                                    .px_2()
                                    .py_1()
                                    .font_family(t.mono_font)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.dispatch(
                                            TableCommand::ToggleTransactionColumn(column.clone()),
                                            window,
                                            cx,
                                        );
                                    })),
                            );
                        }
                        root = root.child(list);
                    }
                    root = root.child(
                        Button::new("add-attribute-columns")
                            .label("＋ Add attribute columns…")
                            .justify_start()
                            .debug_selector(|| "add-attribute-columns".into())
                            .ghost()
                            .w_full()
                            .on_click(
                                cx.listener(|view, _, window, cx| view.show_attributes(window, cx)),
                            ),
                    );
                }
                root = root.child(
                    div()
                        .border_t_1()
                        .border_color(t.border_variant)
                        .pt_1()
                        .child(
                            Button::new("reset-table-columns")
                                .label("Reset to defaults")
                                .justify_start()
                                .ghost()
                                .w_full()
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.dispatch(TableCommand::ResetColumns, window, cx);
                                })),
                        ),
                );
            }
        }
        root.into_any_element()
    }
}
