//! Native column editors placed using the canvas's table geometry.

use crate::{app::Workspace, theme::theme};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, Focusable, Render, Subscription, WeakEntity, Window, div, px,
};
use std::collections::HashMap;
use volna_core::{
    Command,
    panels::PanelId,
    table::{
        TableCommand,
        filter::{ColumnFilter, FilterMode, Missing},
    },
};

gpui_kit::actions!(table_filter, [RevertFilter]);
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([gpui_kit::KeyBinding::new(
        "escape",
        RevertFilter,
        Some("TableFilterInput > Input"),
    )]);
}

pub(crate) struct FilterRow {
    ws: WeakEntity<Workspace>,
    panel: PanelId,
    generation: u64,
    editors: HashMap<usize, Entity<FilterEditor>>,
}
impl FilterRow {
    pub fn new(
        ws: WeakEntity<Workspace>,
        panel: PanelId,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        if let Some(owner) = ws.upgrade() {
            cx.observe(&owner, |_, _, cx| cx.notify()).detach();
        }
        Self {
            ws,
            panel,
            generation,
            editors: HashMap::new(),
        }
    }
    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.editors.values().any(|editor| {
            editor
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        })
    }
}
impl Render for FilterRow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div().size_full().relative().overflow_hidden();
        let Some(owner) = self.ws.upgrade() else {
            return row;
        };
        let ws = owner.read(cx);
        let Some(table) = ws.app.panels.get(self.panel).and_then(|p| p.kind.table()) else {
            return row;
        };
        self.editors.retain(|&index, _| index < table.columns.len());
        let left = table.layout.header.left();
        let columns: Vec<_> = table
            .layout
            .columns
            .iter()
            .map(|c| (c.source_index, c.rect.left() - left, c.rect.width()))
            .collect();
        for (column, x, width) in columns {
            let editor = self
                .editors
                .entry(column)
                .or_insert_with(|| {
                    cx.new(|cx| {
                        FilterEditor::new(
                            self.ws.clone(),
                            self.panel,
                            self.generation,
                            column,
                            window,
                            cx,
                        )
                    })
                })
                .clone();
            row = row.child(
                div()
                    .id(("filter-cell", column))
                    .debug_selector(move || format!("filter-cell-{column}"))
                    .absolute()
                    .left(px(x))
                    .top_0()
                    .w(px(width))
                    .h_full()
                    .overflow_hidden()
                    .px(px(4.0 * theme(cx).zoom))
                    .py(px(4.0 * theme(cx).zoom))
                    .child(editor),
            );
        }
        row
    }
}

struct FilterEditor {
    ws: WeakEntity<Workspace>,
    panel: PanelId,
    generation: u64,
    column: usize,
    input: Entity<InputState>,
    draft: ColumnFilter,
    synced: Option<ColumnFilter>,
    title: Option<String>,
    _subscription: Subscription,
}
impl FilterEditor {
    fn new(
        ws: WeakEntity<Workspace>,
        panel: PanelId,
        generation: u64,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if let Some(owner) = ws.upgrade() {
            cx.observe(&owner, |_, _, cx| cx.notify()).detach();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter…"));
        let subscription = cx.subscribe_in(&input, window, |view, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                view.apply(window, cx);
            }
        });
        Self {
            ws,
            panel,
            generation,
            column,
            input,
            draft: ColumnFilter::default(),
            synced: None,
            title: None,
            _subscription: subscription,
        }
    }
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft.query = self.input.read(cx).value().to_string();
        let command = TableCommand::SetFilter {
            source_index: self.column,
            filter: Some(self.draft.clone()),
        };
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
    fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.draft.mode;
        let applied = self.ws.upgrade().and_then(|owner| {
            owner
                .read(cx)
                .app
                .panels
                .get(self.panel)
                .and_then(|p| p.kind.table())
                .and_then(|table| table.applied_filter(self.column))
                .cloned()
        });
        self.draft = applied.unwrap_or(ColumnFilter {
            mode,
            ..Default::default()
        });
        let query = self.draft.query.clone();
        self.input
            .update(cx, |input, cx| input.set_value(query, window, cx));
        self.apply(window, cx);
    }
}
impl Render for FilterEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut title = String::from("column");
        let mut active = false;
        let mut error = None;
        if let Some(owner) = self.ws.upgrade() {
            let ws = owner.read(cx);
            if let Some(table) = ws.app.panels.get(self.panel).and_then(|p| p.kind.table()) {
                table.column_title(self.column).clone_into(&mut title);
                error = table
                    .query_error
                    .as_ref()
                    .filter(|e| e.starts_with(&format!("{title} filter:")))
                    .cloned();
                let filter = table.filters.get(self.column).cloned().flatten();
                active = filter.is_some();
                if self.title.as_ref() != Some(&title) || self.synced != filter {
                    self.draft = filter.clone().unwrap_or(ColumnFilter {
                        mode: table.default_filter_mode(self.column),
                        ..Default::default()
                    });
                    let query = self.draft.query.clone();
                    self.input
                        .update(cx, |input, cx| input.set_value(query, window, cx));
                    self.synced = filter;
                    self.title = Some(title.clone());
                }
            }
        }
        let t = *theme(cx);
        let mode = self.draft.mode;
        let hint = match mode {
            FilterMode::Expression => {
                "Enter to apply · > 100 · 100..500 · >= 0x100 AND < 0x200 · time units or raw ticks"
            }
            FilterMode::Wildcard => {
                "Enter to apply · * any sequence · ? one character · \\ escapes a wildcard"
            }
            FilterMode::Fuzzy => {
                "Enter to apply · letters match in order: clm matches cache_load_miss"
            }
            FilterMode::Regex => {
                "Enter to apply · regular expression · case insensitive by default"
            }
            _ => "Enter to apply · Escape reverts · filters on different columns combine with AND",
        };
        let hint = error.clone().unwrap_or_else(|| hint.to_owned());
        let editor = cx.entity().downgrade();
        let mode_button = Button::new("filter-mode")
            .label(mode.badge())
            .ghost()
            .small()
            .compact()
            .w(px(26.0 * t.zoom))
            .h(px(24.0 * t.zoom))
            .accessibility_label(format!("Filter options for {title}: {}", mode.label()))
            .tooltip(format!("{} · Filter options", mode.label()))
            .dropdown_menu(move |mut menu, _, cx| {
                let Some(entity) = editor.upgrade() else {
                    return menu;
                };
                let current = entity.read(cx).draft.clone();
                for mode in FilterMode::ALL {
                    let editor = editor.clone();
                    menu = menu.item(
                        PopupMenuItem::new(mode.label())
                            .checked(current.mode == mode)
                            .on_click(move |_, window, cx| {
                                if let Some(editor) = editor.upgrade() {
                                    editor.update(cx, |view, cx| {
                                        view.draft.mode = mode;
                                        view.apply(window, cx);
                                    });
                                }
                            }),
                    );
                }
                let case_editor = editor.clone();
                menu = menu.separator().item(
                    PopupMenuItem::new("Match case")
                        .checked(current.case_sensitive)
                        .on_click(move |_, window, cx| {
                            if let Some(editor) = case_editor.upgrade() {
                                editor.update(cx, |view, cx| {
                                    view.draft.case_sensitive = !view.draft.case_sensitive;
                                    view.apply(window, cx);
                                });
                            }
                        }),
                );
                menu = menu.separator();
                for (missing, label) in [
                    (Missing::Exclude, "Exclude missing values"),
                    (Missing::Include, "Include missing values"),
                    (Missing::Only, "Missing values only"),
                ] {
                    let editor = editor.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(current.missing == missing)
                            .on_click(move |_, window, cx| {
                                if let Some(editor) = editor.upgrade() {
                                    editor.update(cx, |view, cx| {
                                        view.draft.missing = missing;
                                        view.apply(window, cx);
                                    });
                                }
                            }),
                    );
                }
                menu
            });
        div()
            .id("column-filter-editor")
            .size_full()
            .key_context("TableFilterInput")
            .on_action(cx.listener(|view, _: &RevertFilter, window, cx| {
                cx.stop_propagation();
                view.revert(window, cx);
            }))
            .child(
                div()
                    .id("column-filter-hint")
                    .size_full()
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(hint.clone()).build(window, cx)
                    })
                    .child(
                        Input::new(&self.input)
                            .aria_label(format!("Filter {title}"))
                            .small()
                            .h_full()
                            .prefix(mode_button)
                            .when(error.is_some(), |el| el.border_color(t.panel.error))
                            .when(active, |el| {
                                el.suffix(
                                    Button::new("clear-column-filter")
                                        .icon(
                                            gpui_kit::component::Icon::empty()
                                                .path(volna_core::icons::IconName::X.path()),
                                        )
                                        .ghost()
                                        .compact()
                                        .small()
                                        .accessibility_label(format!("Clear filter for {title}"))
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            view.draft.missing = Missing::Exclude;
                                            view.input.update(cx, |input, cx| {
                                                input.set_value("", window, cx);
                                            });
                                            view.apply(window, cx);
                                        })),
                                )
                            }),
                    ),
            )
    }
}
