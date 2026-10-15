//! GPUI chrome for the reduced table.

use crate::dock::CanvasPanelView;
use crate::theme::theme;
use crate::ui::action_button;
#[cfg(not(target_arch = "wasm32"))]
use gpui_kit::ClipboardItem;
use gpui_kit::component::{
    Disableable, Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Empty, Entity, Focusable, Window, div, px};
use volna_core::Command;
use volna_core::icons::IconName;
use volna_core::table::{TableCommand, TableState};

pub(crate) fn format_menu(
    mut menu: PopupMenu,
    owner: &Entity<crate::app::Workspace>,
    panel: volna_core::panels::PanelId,
    generation: u64,
    column: Option<usize>,
    window: &Window,
    cx: &App,
) -> PopupMenu {
    let ws = owner.read(cx);
    if ws.app.doc.generation() != generation {
        return menu;
    }
    let Some(table) = ws.app.panels.get(panel).and_then(|p| p.kind.table()) else {
        return menu;
    };
    let t = theme(cx);
    let available = (window.viewport_size().width - px(32.0 * t.zoom)).max(px(0.0));
    let menu_width = px(340.0).min(available);
    let content_width = (menu_width - px(56.0 * t.zoom)).max(px(0.0));
    let (heading, choices) = if let Some(column) = column {
        let Some(format) = table.column_formats(column) else {
            return menu;
        };
        (format!("Format · {}", table.column_title(column)), format)
    } else {
        ("Default number format".into(), table.number_formats())
    };
    menu = menu.item(
        PopupMenuItem::element(move |_, cx| {
            div()
                .id("table-format-heading")
                .w(content_width)
                .truncate()
                .text_color(theme(cx).panel.text_muted)
                .aria_label(heading.clone())
                .child(heading.clone())
        })
        .disabled(true),
    );
    for (index, choice) in choices.into_iter().enumerate() {
        if column.is_some() && index == 1 {
            menu = menu.separator();
        }
        let owner = owner.clone();
        let selector = format!(
            "table-format-choice-{}",
            choice.translator.as_deref().unwrap_or("inherit")
        );
        let command = match column {
            Some(source_index) => TableCommand::ColumnFormat {
                source_index,
                translator: choice.translator,
            },
            None => TableCommand::NumberFormat(choice.translator.expect("number format")),
        };
        menu = menu.item(
            PopupMenuItem::element(move |_, cx| {
                let t = theme(cx);
                let selector = selector.clone();
                div()
                    .id(("table-format-choice", index))
                    .debug_selector(move || selector)
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .w(content_width)
                    .max_w(content_width)
                    .items_center()
                    .gap_2()
                    .aria_label(format!("{} {}", choice.label, choice.preview))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(choice.label.clone()),
                    )
                    .when(!choice.preview.is_empty(), |el| {
                        el.child(
                            div()
                                .max_w(px(110.0 * t.zoom).min(content_width * 0.45))
                                .truncate()
                                .font_family(t.mono_font)
                                .text_color(t.panel.text_muted)
                                .child(choice.preview.clone()),
                        )
                    })
            })
            .checked(choice.checked)
            .on_click(move |_, window, cx| {
                owner.update(cx, |ws, cx| {
                    ws.dispatch_if_current(
                        generation,
                        Command::Table(panel, command.clone()),
                        Some(window),
                        cx,
                    );
                });
            }),
        );
    }
    menu.min_w(menu_width).max_w(menu_width)
}

gpui_kit::actions!(
    table_column,
    [NarrowColumn, WidenColumn, FitColumn, SortColumn]
);

pub(crate) fn init(cx: &mut gpui_kit::App) {
    use gpui_kit::KeyBinding;
    cx.bind_keys([
        KeyBinding::new("enter", SortColumn, Some("TableColumnSort")),
        KeyBinding::new("space", SortColumn, Some("TableColumnSort")),
        KeyBinding::new("left", NarrowColumn, Some("TableColumnResize")),
        KeyBinding::new("shift-left", NarrowColumn, Some("TableColumnResize")),
        KeyBinding::new("right", WidenColumn, Some("TableColumnResize")),
        KeyBinding::new("shift-right", WidenColumn, Some("TableColumnResize")),
        KeyBinding::new("enter", FitColumn, Some("TableColumnResize")),
    ]);
}

impl CanvasPanelView {
    fn table_command(&self, command: TableCommand, window: &mut Window, cx: &mut Context<Self>) {
        drop(self.ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(
                self.generation,
                Command::Table(self.id, command),
                Some(window),
                cx,
            );
        }));
    }

    pub(crate) fn render_table(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(owner) = self.ws.upgrade() else {
            return Empty.into_any_element();
        };
        let Some(table) = owner
            .read(cx)
            .app
            .panels
            .get(self.id)
            .and_then(|panel| panel.kind.table())
        else {
            return Empty.into_any_element();
        };
        let t = *theme(cx);
        let has_selection = table.selected.is_some();
        let failed = matches!(table.state, TableState::Failed(_) | TableState::Refused(_))
            || table.query_error.is_some();
        let loading = matches!(table.state, TableState::Loading) || table.querying();
        let number_format = table.number_format();
        let filters_visible = table.filters_visible;
        let filter_count = table.active_filter_count();
        let filters = self
            .table_filters
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    crate::table_filters::FilterRow::new(
                        self.ws.clone(),
                        self.id,
                        self.generation,
                        cx,
                    )
                })
            })
            .clone();
        let picker = self
            .table_columns
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    crate::table_columns::ColumnPicker::new(
                        self.ws.clone(),
                        self.id,
                        self.generation,
                        window,
                        cx,
                    )
                })
            })
            .clone();
        let id = self.id;
        let generation = self.generation;
        let mut toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_2()
            .min_h(px(32.0 * t.zoom))
            .flex_none();
        for (name, icon, tooltip, command) in [
            (
                "table-first",
                IconName::TableFirst,
                "First row (Home)",
                TableCommand::First,
            ),
            (
                "table-previous",
                IconName::TablePrevious,
                "Previous row (Arrow Up)",
                TableCommand::Previous,
            ),
            (
                "table-next",
                IconName::TableNext,
                "Next row (Arrow Down)",
                TableCommand::Next,
            ),
            (
                "table-last",
                IconName::TableLast,
                "Last row (End)",
                TableCommand::Last,
            ),
        ] {
            let owner = owner.clone();
            toolbar = toolbar.child(
                action_button(
                    name,
                    gpui_kit::component::Icon::empty().path(icon.path()),
                    cx,
                )
                .tooltip(tooltip)
                .ghost()
                .on_click(move |_, window, cx| {
                    owner.update(cx, |ws, cx| {
                        ws.dispatch_if_current(
                            generation,
                            Command::Table(id, command.clone()),
                            Some(window),
                            cx,
                        );
                    });
                }),
            );
        }
        toolbar = toolbar
            .child(
                Button::new("table-go-to")
                    .label("Go to…")
                    .ghost()
                    .small()
                    .on_click(cx.listener(|view, _, window, cx| view.table_go_to(window, cx))),
            )
            .child(crate::table_columns::ColumnPicker::popover(picker, cx))
            .child(
                action_button(
                    "table-filter",
                    gpui_kit::component::Icon::empty().path(IconName::ListFilter.path()),
                    cx,
                )
                .label(if filter_count == 0 {
                    "Filter".to_owned()
                } else {
                    format!("Filter · {filter_count}")
                })
                .ghost()
                .selected(filters_visible)
                .toggled(filters_visible)
                .accessibility_label(if filters_visible {
                    "Hide column filters"
                } else {
                    "Show column filters"
                })
                .tooltip("Show or hide column filters; applied filters stay active")
                .debug_selector(|| "table-filter".into())
                .on_click(cx.listener(|view, _, window, cx| {
                    view.table_command(TableCommand::ToggleFilters, window, cx);
                })),
            )
            .when(filter_count > 0, |el| {
                el.child(
                    Button::new("table-clear-filters")
                        .label("Clear filters")
                        .ghost()
                        .small()
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.table_command(TableCommand::ClearFilters, window, cx);
                        })),
                )
            })
            .child(
                action_button(
                    "table-number-format",
                    gpui_kit::component::Icon::empty().path(IconName::Hash.path()),
                    cx,
                )
                .label(number_format)
                .accessibility_label(format!("Default number format: {number_format}"))
                .ghost()
                .tooltip("Default format for numeric columns; column overrides are preserved")
                .debug_selector(|| "table-number-format".into())
                .dropdown_menu({
                    let owner = owner.clone();
                    move |menu, window, cx| {
                        format_menu(menu, &owner, id, generation, None, window, cx)
                    }
                }),
            )
            .child(
                action_button(
                    "table-details",
                    gpui_kit::component::Icon::empty().path(IconName::TableDetails.path()),
                    cx,
                )
                .label("Details")
                .ghost()
                .disabled(!has_selection)
                .tooltip("Show this record in a Transaction panel (Enter)")
                .on_click(cx.listener(|view, _, window, cx| view.show_transaction(window, cx))),
            );
        if failed || loading {
            let command = if loading {
                TableCommand::Cancel
            } else {
                TableCommand::Retry
            };
            toolbar = toolbar.child(
                Button::new("table-retry")
                    .label(if loading { "Cancel" } else { "Retry" })
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.table_command(command.clone(), window, cx);
                    })),
            );
        }

        let canvas = div().flex_1().min_h_0().overflow_hidden().child(
            crate::canvas::PanelCanvas::new(owner.clone(), id, generation)
                .table(self.focus.clone(), filters),
        );
        let body = div().flex().flex_1().min_h_0().child(canvas);

        div()
            .id(("table-panel", id.0))
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .key_context(
                if self
                    .table_columns
                    .as_ref()
                    .is_some_and(|picker| picker.read(cx).is_open())
                {
                    "TableColumns"
                } else {
                    "Waves Table"
                },
            )
            .on_key_down(
                cx.listener(|view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if view
                        .table_columns
                        .as_ref()
                        .is_some_and(|picker| picker.read(cx).is_open())
                    {
                        return;
                    }
                    if view
                        .table_filters
                        .as_ref()
                        .is_some_and(|row| row.read(cx).contains_focus(window, cx))
                    {
                        return;
                    }
                    let key = event.keystroke.key.as_str();
                    let modifiers = crate::app::to_modifiers(event.keystroke.modifiers);
                    let command = match key {
                        "home" => Some(TableCommand::First),
                        "end" => Some(TableCommand::Last),
                        "up" => Some(TableCommand::Previous),
                        "down" => Some(TableCommand::Next),
                        "pageup" => Some(TableCommand::Page(-1)),
                        "pagedown" => Some(TableCommand::Page(1)),
                        "escape" => Some(TableCommand::ClearSelection),
                        _ => None,
                    };
                    if key == "enter" {
                        view.show_transaction(window, cx);
                        cx.stop_propagation();
                    } else if let Some(command) = command {
                        view.table_command(command, window, cx);
                        cx.stop_propagation();
                    } else if key.eq_ignore_ascii_case("c") && modifiers.secondary() {
                        view.copy_table_row(cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .child(toolbar)
            .child(body)
            .into_any_element()
    }

    fn show_transaction(&self, window: &mut Window, cx: &mut Context<Self>) {
        drop(self.ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(
                self.generation,
                Command::ShowTransaction { from: self.id },
                Some(window),
                cx,
            );
        }));
    }

    fn copy_table_row(&self, cx: &mut Context<Self>) {
        let Some(owner) = self.ws.upgrade() else {
            return;
        };
        match owner
            .read(cx)
            .app
            .panels
            .get(self.id)
            .and_then(|panel| panel.kind.table())
            .map(volna_core::table::TableModel::copy_tsv)
        {
            Some(Ok(text)) => {
                #[cfg(target_arch = "wasm32")]
                crate::table_clipboard::copy(&text);
                #[cfg(not(target_arch = "wasm32"))]
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            Some(Err(error)) => {
                owner.update(cx, |ws, cx| {
                    ws.dispatch_if_current(
                        self.generation,
                        Command::Notice(error.to_string()),
                        None,
                        cx,
                    );
                });
            }
            None => {}
        }
    }

    fn table_go_to(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(owner) = self.ws.upgrade() else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("1-based row number"));
        let input_focus = input.read(cx).focus_handle(cx);
        let id = self.id;
        let generation = self.generation;
        let return_focus = self.focus.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            let input = input.clone();
            let return_focus = return_focus.clone();
            dialog
                .title("Go to row")
                .child(Input::new(&input))
                .on_ok(move |_, window, cx| {
                    let value = input.read(cx).value().trim().parse::<u64>();
                    match value.ok().and_then(|row| row.checked_sub(1)) {
                        Some(row) => owner.update(cx, |ws, cx| {
                            let in_range = ws
                                .app
                                .panels
                                .get(id)
                                .and_then(|panel| panel.kind.table())
                                .is_some_and(|table| row < table.len());
                            let command = if in_range {
                                Command::Table(id, TableCommand::GoTo(row))
                            } else {
                                Command::Notice("Choose a row within this table.".into())
                            };
                            ws.dispatch_if_current(generation, command, Some(window), cx);
                        }),
                        None => owner.update(cx, |ws, cx| {
                            ws.dispatch_if_current(
                                generation,
                                Command::Notice("Choose a whole row number starting at 1.".into()),
                                Some(window),
                                cx,
                            );
                        }),
                    }
                    window.focus(&return_focus, cx);
                    true
                })
        });
        window.focus(&input_focus, cx);
    }
}
