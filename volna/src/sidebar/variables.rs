//! The variable list panel: GPUI rows over `MemberListModel`, with the filter text box.

use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Selectable};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, CursorStyle, Focusable, IntoElement, KeyDownEvent, SharedString, Window, div, px,
    uniform_list,
};
use volna_core::app::Command;
use volna_core::sidebar::Key;
use volna_core::sidebar::icons::{direction_icon, member_icon};
use volna_core::sidebar::members::{describe, member_detail};
use volna_core::trace::Traced;
use volna_trace::data::Member;

use crate::app::{Workspace, to_modifiers};
use crate::theme::{ThemePx, theme};
use crate::ui::{Icon, IconName, icon_button, panel_header};

impl Workspace {
    fn members_on_waves(&mut self) -> std::rc::Rc<crate::app::OnWaves> {
        let key = self.app.rows_revision();
        match &self.on_waves {
            Some((k, set)) if *k == key => set.clone(),
            _ => {
                let set = std::rc::Rc::new(self.app.members_on_waves());
                self.on_waves = Some((key, set.clone()));
                set
            }
        }
    }

    fn variables_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "tab" {
            window.focus(&self.scopes_focus, cx);
            cx.stop_propagation();
            return;
        }
        if self
            .filter
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
        {
            if ev.keystroke.key == "down" {
                window.focus(&self.variables_focus, cx);
                self.dispatch(
                    Command::VariablesKey(Key::Down, to_modifiers(ev.keystroke.modifiers)),
                    Some(window),
                    cx,
                );
                cx.stop_propagation();
            }
            return;
        }
        let ks = &ev.keystroke;
        if ks.key == "f10" && ks.modifiers.shift {
            let selected = self
                .app
                .variables
                .selected
                .iter()
                .filter_map(|&index| self.app.variables.rows.get(index).copied())
                .collect();
            self.dispatch(
                Command::OpenTable {
                    selected,
                    clicked: None,
                },
                Some(window),
                cx,
            );
            cx.stop_propagation();
            return;
        }
        let key = match ks.key.as_str() {
            "enter" => Key::Enter,
            "down" => Key::Down,
            "up" => Key::Up,
            "escape" => Key::Escape,
            _ => {
                if let Some(c) = ks.key_char.as_deref().filter(|c| !c.is_empty())
                    && !ks.modifiers.platform
                    && !ks.modifiers.control
                {
                    let handle = self.filter.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                    let c = c.to_owned();
                    self.filter.update(cx, |f, cx| f.insert(&c, cx));
                    cx.stop_propagation();
                }
                return;
            }
        };
        self.dispatch(
            Command::VariablesKey(key, to_modifiers(ks.modifiers)),
            Some(window),
            cx,
        );
        cx.stop_propagation();
    }

    pub(crate) fn render_variables(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *theme(cx);
        let colors = t.panel;
        let focused = self.variables_focus.is_focused(window);
        let selection = t.islands.map_or(t.selection.bg, |i| i.active_tab);
        let selection_bg = if focused {
            selection
        } else {
            selection.opacity(0.72)
        };
        let vars = &self.app.variables;
        let count = vars.rows.len();
        let show_scope = vars.show_scope();
        let traces = self.app.doc.traces();
        let show_direction = vars.show_direction(traces);
        let placeholder = vars.placeholder(traces);
        let breadcrumb = vars.breadcrumb(traces);
        let letters = traces.is_combined() && show_scope;
        let everywhere = vars.search_everywhere;
        let truncated = vars.truncated;
        let header = panel_header(vars.title(traces), cx).child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .font_family(t.mono_font)
                        .text_size(px(t.ui_size_small))
                        .text_color(colors.text_placeholder)
                        .child(SharedString::from(count.to_string())),
                )
                .child(
                    icon_button("add-all", IconName::Plus, t.panel, t.hover, cx)
                        .disabled(vars.rows.is_empty())
                        .tooltip("Add all listed members (⏎)")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.dispatch(Command::AddAllVars, Some(window), cx);
                        })),
                ),
        );

        let list = uniform_list(
            "variable-list",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = *theme(cx);
                let on_waves = this.members_on_waves();
                range
                    .filter_map(|ix| {
                        let owner = cx.entity().downgrade();
                        let traced = this.app.variables.rows[ix];
                        let member = traced.item;
                        let h = this.app.doc.hierarchy(traced.trace)?;
                        let selected = this.app.variables.selected.contains(&ix);
                        let colors = t.row(selected, false);
                        let hover = t.hover;
                        let dims: SharedString = member_detail(h, member).into();
                        let name: SharedString = if show_scope {
                            h.member_path(member).into()
                        } else {
                            h.member_name(member).to_owned().into()
                        };
                        let dir = member
                            .var()
                            .and_then(|id| direction_icon(h.var(id).direction));
                        let shown = on_waves.contains(&traced);
                        let activity = this.app.member_activity(traced);
                        let mut tooltip = if shown {
                            format!("{} · on the waves", describe(h, member))
                        } else {
                            describe(h, member)
                        };
                        if let Some(active) = activity {
                            tooltip.push_str(if active {
                                "\nActivity in the view"
                            } else {
                                "\nNo activity in the view"
                            });
                        }
                        let description: SharedString =
                            h.member_description(member).to_owned().into();
                        let clock = this.app.member_clock(traced).is_some();
                        let generation = this.app.doc.generation();
                        let sampling_items = member.var().map_or_else(Vec::new, |var| {
                            this.app.doc.sampling_menu(traced.with(var))
                        });

                        let mut row = div()
                            .id(("var", ix))
                            .debug_selector(move || format!("member-row-{}-{ix}", traced.trace))
                            .w_full()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .h(px(t.row_height))
                            .when(t.islands.is_some(), |row| row.rounded(t.px(4.)))
                            .px_2()
                            .gap_2()
                            .cursor(CursorStyle::PointingHand)
                            .font_family(t.ui_font)
                            .text_size(px(t.ui_size))
                            .text_color(colors.text)
                            .on_click(cx.listener(
                                move |this, ev: &gpui_kit::ClickEvent, window, cx| {
                                    window.focus(&this.variables_focus, cx);
                                    let command = if ev.click_count() == 2 {
                                        Command::ActivateMembers(vec![traced])
                                    } else {
                                        Command::SelectVar {
                                            ix,
                                            modifiers: to_modifiers(ev.modifiers()),
                                        }
                                    };
                                    this.dispatch(command, Some(window), cx);
                                },
                            ));
                        if selected {
                            row = row
                                .bg(selection_bg)
                                .when(t.appearance.is_high_contrast(), |row| {
                                    row.border_1().border_color(t.border_focused)
                                });
                        } else {
                            row = row.hover(move |s| s.bg(hover.bg).text_color(hover.text));
                        }
                        let row = row
                            .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                            .tooltip(move |w, cx| {
                                crate::ui::tooltip::hierarchy(
                                    tooltip.clone(),
                                    description.clone(),
                                    w,
                                    cx,
                                )
                            })
                            .when(letters, |row| {
                                row.child(crate::traces::letter_badge(traced.trace, cx))
                            })
                            .child(
                                Icon::new(member_icon(h, member))
                                    .size(t.px(14.0))
                                    .color(t.panel.icon_muted),
                            )
                            .when(show_direction, |row| {
                                row.child(
                                    div()
                                        .w(t.px(24.0))
                                        .flex_none()
                                        .text_size(px(t.ui_size_small))
                                        .when_some(dir, |el, (icon, tint)| {
                                            el.child(
                                                Icon::new(icon)
                                                    .size(t.px(14.0))
                                                    .color(tint.color(&t)),
                                            )
                                        }),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .when(activity == Some(false) && !selected, |name| {
                                        name.text_color(colors.text_placeholder).debug_selector(
                                            move || {
                                                format!("inactive-member-{}-{ix}", traced.trace)
                                            },
                                        )
                                    })
                                    .child(name),
                            )
                            .when(shown, |row| {
                                row.relative().child(
                                    div()
                                        .id(("on-waves", ix))
                                        .absolute()
                                        .left_0()
                                        .top_0()
                                        .bottom_0()
                                        .w(px(2.0))
                                        .bg(t.panel.icon_accent),
                                )
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .max_w(t.px(140.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(t.ui_size_small))
                                    .text_color(colors.text_placeholder)
                                    .child(dims),
                            )
                            .context_menu(move |menu, menu_window, menu_cx| {
                                let owner = owner.clone();
                                let add =
                                    |label: &'static str,
                                     command: fn(Vec<Traced<Member>>) -> Command| {
                                        let owner = owner.clone();
                                        PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                            drop(owner.update(cx, |workspace, cx| {
                                                let list = &workspace.app.variables;
                                                let members = if list.selected.contains(&ix) {
                                                    list.selected
                                                        .iter()
                                                        .filter_map(|&i| list.rows.get(i).copied())
                                                        .collect()
                                                } else {
                                                    vec![traced]
                                                };
                                                workspace.dispatch(
                                                    command(members),
                                                    Some(window),
                                                    cx,
                                                );
                                            }));
                                        })
                                    };
                                let mut menu = menu;
                                if !sampling_items.is_empty() {
                                    let items = sampling_items.clone();
                                    let owner = owner.clone();
                                    let registered = items.iter().any(|item| {
                                        matches!(
                                            item.state,
                                            volna_core::wave::MenuItemState::Choice {
                                                selected: true
                                            }
                                        )
                                    });
                                    let sampling_label = if registered {
                                        "Sampling"
                                    } else {
                                        "Use for sampling"
                                    };
                                    menu = menu.submenu(
                                        sampling_label,
                                        menu_window,
                                        menu_cx,
                                        move |menu, _, _| {
                                            sampling_submenu(menu, &items, &owner, generation)
                                        },
                                    );
                                }
                                let menu = if clock {
                                    menu.item(add("Add as Ruler", Command::AddClockRulers))
                                        .item(add("Add as Waveform", Command::AddToWaves))
                                } else {
                                    menu.item(add("Add to Waves", Command::AddToWaves))
                                };
                                let (selected_tracks, active_pipeline) = owner
                                    .upgrade()
                                    .map(|ws| {
                                        let app = &ws.read(menu_cx).app;
                                        let list = &app.variables;
                                        let members: Vec<_> = if list.selected.contains(&ix) {
                                            list.selected
                                                .iter()
                                                .filter_map(|&i| list.rows.get(i).copied())
                                                .collect()
                                        } else {
                                            vec![traced]
                                        };
                                        let mut tracks = Vec::new();
                                        for m in members {
                                            if app.member_clock(m).is_some() {
                                                continue;
                                            }
                                            if let Some(h) = app.doc.hierarchy(m.trace)
                                                && let Some(t) = h.member_track(m.item)
                                            {
                                                let tr = m.with(t);
                                                if !tracks.contains(&tr) {
                                                    tracks.push(tr);
                                                }
                                            }
                                        }
                                        (tracks, app.panels.active_pipeline())
                                    })
                                    .unwrap_or_default();
                                let menu = if selected_tracks.is_empty() {
                                    menu
                                } else {
                                    let count = selected_tracks.len();
                                    let open_label = if count > 1 {
                                        format!("Open Merged Pipeline ({count})")
                                    } else {
                                        "Open in Pipeline".into()
                                    };
                                    let owner_open = owner.clone();
                                    let tracks_open = selected_tracks.clone();
                                    let mut menu =
                                        menu.item(PopupMenuItem::new(open_label).on_click(
                                            move |_, window, cx| {
                                                drop(owner_open.update(cx, |workspace, cx| {
                                                    let cmd = if tracks_open.len() > 1 {
                                                        Command::OpenMergedPipeline {
                                                            tracks: tracks_open.clone(),
                                                        }
                                                    } else {
                                                        Command::OpenPipeline {
                                                            track: tracks_open[0],
                                                        }
                                                    };
                                                    workspace.dispatch(cmd, Some(window), cx);
                                                }));
                                            },
                                        ));
                                    if let Some(panel) = active_pipeline {
                                        let add_label = if count > 1 {
                                            format!("Add {count} to Active Pipeline")
                                        } else {
                                            "Add to Active Pipeline".into()
                                        };
                                        let owner_add = owner.clone();
                                        let tracks_add = selected_tracks;
                                        menu = menu.item(PopupMenuItem::new(add_label).on_click(
                                            move |_, window, cx| {
                                                drop(owner_add.update(cx, |workspace, cx| {
                                                    workspace.dispatch(
                                                        Command::AddToPipeline {
                                                            panel,
                                                            tracks: tracks_add.clone(),
                                                        },
                                                        Some(window),
                                                        cx,
                                                    );
                                                }));
                                            },
                                        ));
                                    }
                                    menu
                                };
                                menu.item(PopupMenuItem::new("Open in table").on_click(
                                    move |_, window, cx| {
                                        drop(owner.update(cx, |workspace, cx| {
                                            let selected = workspace
                                                .app
                                                .variables
                                                .selected
                                                .iter()
                                                .filter_map(|&index| {
                                                    workspace.app.variables.rows.get(index).copied()
                                                })
                                                .collect();
                                            workspace.dispatch(
                                                Command::OpenTable {
                                                    selected,
                                                    clicked: Some(traced),
                                                },
                                                Some(window),
                                                cx,
                                            );
                                        }));
                                    },
                                ))
                            });
                        Some(row)
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.variables_scroll)
        .flex_1()
        .size_full();

        div()
            .id("variables-panel")
            .track_focus(&self.variables_focus)
            .on_key_down(cx.listener(Self::variables_key))
            .flex()
            .flex_col()
            .size_full()
            .bg(t.panel.bg)
            .child(header)
            .when(!breadcrumb.is_empty(), |el| {
                el.child(
                    div()
                        .px_2()
                        .py_1()
                        .font_family(t.mono_font)
                        .text_size(px(t.ui_size_small))
                        .text_color(colors.text_muted)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(SharedString::from(breadcrumb)),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_none()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .child(div().flex_1().min_w_0().child(self.filter.clone()))
                    .child(
                        icon_button("search-everywhere", IconName::Search, t.panel, t.hover, cx)
                            .selected(everywhere)
                            .accessibility_label("Search everywhere")
                            .tooltip("Search everywhere: variables and generators")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.dispatch(
                                    Command::SetSearchEverywhere(!everywhere),
                                    Some(window),
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(if focused {
                        t.border_focused
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .when(t.islands.is_some(), |el| el.px(t.px(4.)))
                    .child(match placeholder {
                        Some(text) => div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .px_4()
                            .text_align(gpui_kit::TextAlign::Center)
                            .text_size(px(t.ui_size_small))
                            .text_color(colors.text_placeholder)
                            .child(div().w_full().min_w_0().whitespace_normal().child(text))
                            .into_any_element(),
                        None => list.into_any_element(),
                    }),
            )
            .when(truncated, |el| {
                el.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_size(px(t.ui_size_small))
                        .text_color(colors.text_muted)
                        .child("First 5,000 matches · refine your search"),
                )
            })
    }
}

fn sampling_submenu(
    mut menu: PopupMenu,
    items: &[volna_core::wave::MenuItem],
    owner: &gpui_kit::WeakEntity<Workspace>,
    generation: u64,
) -> PopupMenu {
    for item in items {
        let volna_core::wave::model::MenuAction::Sampling(command) = &item.action else {
            continue;
        };
        let command = command.clone();
        let owner = owner.clone();
        let checked = matches!(
            item.state,
            volna_core::wave::MenuItemState::Choice { selected: true }
        );
        menu = menu.item(
            PopupMenuItem::new(item.label.clone())
                .checked(checked)
                .disabled(item.disabled)
                .on_click(move |_, window, cx| {
                    drop(owner.update(cx, |workspace, cx| {
                        workspace.dispatch_if_current(
                            generation,
                            Command::Sampling(command.clone()),
                            Some(window),
                            cx,
                        );
                    }));
                }),
        );
    }
    menu
}
