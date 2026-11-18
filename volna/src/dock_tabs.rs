//! Responsive dock tabs and panel actions for the Volna themes.

use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui_kit::assets::IconName;
use gpui_kit::base::{ElementExt as _, Tab, Tabs as TabList, dock as base};
use gpui_kit::component::{
    button::ButtonVariants,
    dock::{ClosePanel, PanelHandle, ToggleZoom},
    menu::DropdownMenu,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AppContext, Context, FocusHandle, IntoElement, Pixels, Render, ScrollHandle,
    TestSupportExt, Window, div, px,
};

use crate::theme::{ThemePx, theme};
use crate::ui::action_button;

#[derive(Default)]
pub(crate) struct Tabs {
    scroll: ScrollHandle,
    active: Cell<Option<usize>>,
    header_width: Rc<Cell<Pixels>>,
    toolbar_width: Rc<Cell<Pixels>>,
}

fn activate(group: &base::TabGroupContext, ix: usize, window: &mut Window, cx: &mut App) {
    let focus = group.panels()[ix].focus_handle(cx);
    group.select_tab(ix, window, cx);
    window.focus(&focus, cx);
}

impl Tabs {
    pub(crate) fn render(
        &self,
        group: &base::TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        // TabGroupContext owns its panel list. Share one snapshot across the tab handlers.
        let group = Rc::new(group.clone());
        let t = *theme(cx);
        let heading = t.islands.map_or(t.panel.bg, |i| i.heading);
        let focused = window.is_window_active()
            && group
                .active_panel()
                .is_some_and(|p| p.focus_handle(cx).contains_focused(window, cx));
        let visible: Vec<_> = group
            .panels()
            .iter()
            .enumerate()
            .filter(|(_, panel)| panel.visible(cx))
            .collect();
        let active_position = visible.iter().position(|(ix, _)| *ix == group.active_ix());
        if self.active.replace(Some(group.active_ix())) != Some(group.active_ix())
            && let Some(ix) = active_position
        {
            self.scroll.scroll_to_item(ix);
        }
        let selected = group.active_panel().map(|p| p.panel_id(cx));
        let tab_focus: Rc<[FocusHandle]> = visible
            .iter()
            .map(|(_, panel)| {
                let id = ("dock-tab-focus", panel.panel_id(cx).as_u64());
                let state = window.use_keyed_state(id, cx, |_, cx| cx.focus_handle());
                state.read(cx).clone()
            })
            .collect();
        let tab_stop = tab_focus
            .iter()
            .position(|h| h.is_focused(window))
            .or(active_position)
            .unwrap_or(0);
        let tab_count = visible.len();
        let tabs = visible
            .into_iter()
            .enumerate()
            .map(|(position, (ix, panel))| {
                let active = Some(panel.panel_id(cx)) == selected && !group.is_collapsed();
                let title = match PanelHandle::of(panel) {
                    Some(handle) => handle.title(window, cx),
                    None => div().child(panel.panel_name(cx)).into_any_element(),
                };
                Tab::new(ix)
                    .set_position(position + 1, tab_count)
                    .h(if t.islands.is_some() {
                        t.px(26.)
                    } else {
                        px(t.header_height) - px(1.)
                    })
                    .px(t.px(8.0))
                    .flex_none()
                    .max_w_full()
                    .whitespace_nowrap()
                    .track_focus(&tab_focus[position].clone().tab_stop(position == tab_stop))
                    .focus_visible(move |s| {
                        s.border_color(t.border_focused).shadow(vec![
                            gpui_kit::BoxShadow::new(px(0.), px(0.), t.border_focused)
                                .spread_radius(px(1.))
                                .inset(),
                        ])
                    })
                    .when(!active, |tab| {
                        tab.hover(move |s| s.bg(t.hover.bg).text_color(t.panel.text))
                    })
                    .on_key_down({
                        let group = group.clone();
                        let tab_focus = tab_focus.clone();
                        let scroll = self.scroll.clone();
                        move |event, window, cx| {
                            let next = match event.keystroke.key.as_str() {
                                "left" => (position + tab_count - 1) % tab_count,
                                "right" => (position + 1) % tab_count,
                                "home" => 0,
                                "end" => tab_count - 1,
                                "enter" | "space" => {
                                    cx.stop_propagation();
                                    activate(&group, ix, window, cx);
                                    return;
                                }
                                _ => return,
                            };
                            cx.stop_propagation();
                            scroll.scroll_to_item(next);
                            window.focus(&tab_focus[next], cx);
                        }
                    })
                    .relative()
                    .child(title)
                    .selected(active)
                    .border_0()
                    .map(|tab| match t.islands {
                        Some(_) => tab.rounded(t.px(4.)),
                        None => {
                            tab.rounded_none()
                                .border_b_2()
                                .border_color(if active && focused {
                                    t.border_focused
                                } else {
                                    gpui_kit::transparent_black()
                                })
                        }
                    })
                    .bg(if active && focused {
                        t.islands.map_or(heading, |i| i.active_tab)
                    } else if active {
                        t.islands.map_or(heading, |i| i.tab)
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .text_color(if active {
                        t.panel.text
                    } else {
                        t.panel.text_muted
                    })
                    .when(active && t.islands.is_none(), |tab| {
                        tab.font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    })
                    .debug_selector(move || format!("dock-tab-{ix}"))
                    .on_click({
                        let group = group.clone();
                        move |_, window, cx| activate(&group, ix, window, cx)
                    })
                    .when(group.is_draggable() && !group.is_collapsed(), |tab| {
                        tab.when_some(group.drag_panel(ix, cx), |tab, drag| {
                            let panel = panel.clone();
                            tab.on_drag(drag, move |drag, offset, _, cx| {
                                cx.stop_propagation();
                                drag.set_drag_offset(offset);
                                drag.set_preview_size(gpui_kit::size(px(180.), px(32.)));
                                cx.new(|_| Preview {
                                    panel: panel.clone(),
                                })
                            })
                        })
                    })
                    .when(group.is_droppable(), |tab| {
                        tab.drag_over::<base::DragPanel>(move |s, _, _, _| {
                            s.border_l_2().border_color(t.border_focused)
                        })
                        .on_drop({
                            let group = group.clone();
                            move |drag: &base::DragPanel, window, cx| {
                                group.drop_panel(drag.clone(), Some(ix), true, window, cx);
                            }
                        })
                        .on_drop({
                            let group = group.clone();
                            move |drag: &base::AnyDrag, window, cx| {
                                group.drop_item(drag.clone(), None, window, cx);
                            }
                        })
                    })
            })
            .collect::<Vec<_>>();
        let handle = group.active_panel().and_then(PanelHandle::of);
        let panel = handle.map(PanelHandle::panel);
        let zoomable = group.active_panel().is_some_and(|p| p.zoomable(cx))
            && handle.is_some_and(|h| h.zoom_control(cx).is_some());
        let buttons = handle
            .and_then(|h| h.toolbar_buttons(window, cx))
            .unwrap_or_default();
        // Keep a readable title beside the actions, including after zoom or a panel switch.
        let suffix = handle.and_then(|h| h.title_suffix(window, cx));
        let has_tools = suffix.is_some() || !buttons.is_empty();
        let stacked = has_tools && self.header_width.get() < self.toolbar_width.get() + t.px(224.);

        let toolbar = div()
            .id("dock-tools")
            .on_prepaint({
                let width = self.toolbar_width.clone();
                move |bounds, window, _| {
                    if width.replace(bounds.size.width) != bounds.size.width {
                        window.refresh();
                    }
                }
            })
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .children(suffix)
            .children(buttons);
        let panel_controls = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(t.px(2.))
            .when(zoomable || group.is_zoomed(), |el| {
                el.child(
                    action_button(
                        "dock-zoom",
                        if group.is_zoomed() {
                            IconName::Minimize
                        } else {
                            IconName::Maximize
                        },
                        cx,
                    )
                    .ghost()
                    .tooltip(if group.is_zoomed() {
                        "Restore panel size"
                    } else {
                        "Maximize panel"
                    })
                    .on_click({
                        let group = group.clone();
                        move |_, w, cx| group.toggle_zoom(w, cx)
                    }),
                )
            })
            .child(
                action_button("dock-menu", IconName::Ellipsis, cx)
                    .ghost()
                    .tooltip("Panel menu")
                    .dropdown_menu({
                        let closable = group.is_closable();
                        let zoomed = group.is_zoomed();
                        move |menu, w, cx| {
                            menu.when_some(panel.clone(), |menu, panel| {
                                panel.dropdown_menu(menu, w, cx)
                            })
                            .separator()
                            .menu_with_disabled(
                                if zoomed { "Zoom out" } else { "Zoom in" },
                                Box::new(ToggleZoom),
                                !zoomable && !zoomed,
                            )
                            .when(closable, |menu| {
                                menu.menu("Close panel", Box::new(ClosePanel))
                            })
                        }
                    }),
            );
        let trailing = div()
            .id("dock-tab-end")
            .h_full()
            .min_w(t.px(12.))
            .flex_1()
            .when(group.is_droppable(), |el| {
                el.on_drop({
                    let group = group.clone();
                    move |drag: &base::DragPanel, window, cx| {
                        group.drop_panel(drag.clone(), None, true, window, cx);
                    }
                })
            });
        let row = div().flex().items_center().w_full().min_w_0();
        let row = row
            .child(
                TabList::new("dock-tabs")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(t.px(2.))
                    .overflow_x_scroll()
                    .track_scroll(&self.scroll)
                    .children(tabs)
                    .child(trailing),
            )
            .when(tab_count > 1, |el| {
                el.child(
                    action_button("dock-tab-list", IconName::ChevronDown, cx)
                        .ghost()
                        .tooltip("All tabs")
                        .dropdown_menu({
                            let group = group.clone();
                            move |mut menu, _, cx| {
                                for (ix, panel) in group
                                    .panels()
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, p)| p.visible(cx))
                                {
                                    let panel = panel.clone();
                                    let group = group.clone();
                                    menu = menu.item(
                                        gpui_kit::component::menu::PopupMenuItem::element(
                                            move |w, cx| match PanelHandle::of(&panel) {
                                                Some(handle) => handle.title(w, cx),
                                                None => div()
                                                    .child(panel.panel_name(cx))
                                                    .into_any_element(),
                                            },
                                        )
                                        .on_click(move |_, w, cx| activate(&group, ix, w, cx)),
                                    );
                                }
                                menu
                            }
                        }),
                )
            });
        let header = div()
            .id(format!("dock-header-{}", group.node().as_u64()))
            .test_support()
            .relative()
            .flex_none()
            .flex()
            .flex_col()
            .justify_center()
            .min_h(px(t.header_height))
            .bg(heading)
            .px(t.px(4.))
            .when(t.islands.is_some(), |el| el.py(t.px(4.)))
            .border_b_1()
            .border_color(t.border_variant)
            .on_prepaint({
                let width = self.header_width.clone();
                move |bounds, window, _| {
                    if width.replace(bounds.size.width) != bounds.size.width {
                        window.refresh();
                    }
                }
            });
        let header = if stacked {
            header.child(row.child(panel_controls)).child(
                div()
                    .id("dock-toolbar")
                    .flex()
                    .w_full()
                    .when(t.islands.is_some(), |el| el.pt(t.px(3.)))
                    .when(t.islands.is_none(), |el| el.pb(t.px(4.)))
                    .overflow_x_scroll()
                    .child(toolbar),
            )
        } else {
            header.child(row.child(toolbar).child(panel_controls))
        };
        header.into_any_element()
    }
}

struct Preview {
    panel: Arc<dyn base::PanelView>,
}
impl Render for Preview {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme(cx);
        div()
            .py_1()
            .px_3()
            .rounded(t.px(6.))
            .bg(t.panel.bg)
            .border_1()
            .border_color(t.border_focused)
            .text_color(t.panel.text)
            .children(PanelHandle::of(&self.panel).map(|h| h.title(w, cx)))
    }
}
