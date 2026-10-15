//! Islands workspace chrome; docking state and actions stay in GPUI-KIT.

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
    AnyElement, App, AppContext, Axis, Context, Empty, IntoElement, MouseMoveEvent, MouseUpEvent,
    Pixels, Render, ScrollHandle, WeakEntity, Window, canvas, deferred, div, px,
};

use crate::theme::{ThemePx, theme};
use crate::ui::action_button;

fn backdrop(bounds: gpui_kit::Bounds<Pixels>, cx: &App) -> [gpui_kit::Background; 2] {
    use gpui_kit::{linear_color_stop, linear_gradient};
    let t = theme(cx);
    let i = t.islands.expect("island backdrop");
    let color_at = |x: Pixels| {
        let x = f32::from(x) / t.zoom;
        let (from, to, amount) = if x < 200. {
            (i.ocean[0], i.ocean[1], x / 200.)
        } else {
            (i.ocean[1], i.ocean[2], (x - 200.) / 700.)
        };
        from.blend(to.opacity(amount.clamp(0., 1.)))
    };
    let fade = |y: Pixels| (f32::from(y) / t.scale(280.)).clamp(0., 1.);
    let end = ((t.px(280.) - bounds.top()) / bounds.size.height).clamp(0., 1.);
    [
        linear_gradient(
            90.,
            linear_color_stop(color_at(bounds.left()), 0.),
            linear_color_stop(color_at(bounds.right()), 1.),
        ),
        linear_gradient(
            180.,
            linear_color_stop(i.background.opacity(fade(bounds.top())), 0.),
            linear_color_stop(i.background.opacity(fade(bounds.bottom())), end),
        ),
    ]
}

pub(crate) fn background() -> AnyElement {
    canvas(
        |_, _, _| (),
        |bounds, (), window, cx| {
            let t = theme(cx);
            let mut left = bounds.left();
            for right in [t.px(200.), t.px(900.), bounds.right()] {
                let right = right.min(bounds.right());
                if right <= left {
                    continue;
                }
                let region = gpui_kit::Bounds::from_corners(
                    gpui_kit::point(left, bounds.top()),
                    gpui_kit::point(right, bounds.bottom()),
                );
                for layer in backdrop(region, cx) {
                    window.paint_quad(gpui_kit::fill(region, layer));
                }
                left = right;
            }
        },
    )
    .absolute()
    .inset_0()
    .size_full()
    .into_any_element()
}

pub(crate) fn resize_thickness(active: bool, cx: &App) -> Pixels {
    if active { theme(cx).px(4.) } else { px(1.) }
}

pub(crate) fn resize_handle(handle: &gpui_kit::base::ResizeHandleContext, cx: &App) -> AnyElement {
    let thickness = resize_thickness(handle.is_active(), cx);
    let line = div()
        .absolute()
        .bg(theme(cx).border_focused)
        .rounded(thickness / 2.)
        .map(|line| match handle.axis() {
            Axis::Horizontal => line.left((px(1.) - thickness) / 2.).w(thickness).h_full(),
            Axis::Vertical => line.top((px(1.) - thickness) / 2.).h(thickness).w_full(),
        });
    div()
        .relative()
        .size_full()
        .when(
            handle.state() != gpui_kit::base::ResizeHandleState::Idle,
            |el| {
                // Dock frames clip at the seam; only the active highlight may overhang it.
                el.child(if handle.edge().is_some() {
                    deferred(line).into_any_element()
                } else {
                    line.into_any_element()
                })
            },
        )
        .into_any_element()
}

#[derive(Default)]
pub(crate) struct DockResize {
    active: Rc<Cell<Option<base::DockPlacement>>>,
}

impl DockResize {
    pub(crate) fn render(
        &self,
        dock: &base::DockContext,
        content: AnyElement,
        area: WeakEntity<base::DockArea>,
    ) -> AnyElement {
        use gpui_kit::base::HandleEdge;
        let placement = dock.placement();
        let (id, axis, edge) = match placement {
            base::DockPlacement::Left => {
                ("island-resize-left", Axis::Horizontal, HandleEdge::Trailing)
            }
            base::DockPlacement::Right => {
                ("island-resize-right", Axis::Horizontal, HandleEdge::Leading)
            }
            base::DockPlacement::Bottom => {
                ("island-resize-bottom", Axis::Vertical, HandleEdge::Leading)
            }
            base::DockPlacement::Center => return content,
        };
        let handle = gpui_kit::base::resize_handle(id, axis)
            .inside(edge)
            .with_appearance(Rc::new(|handle, _, cx| Some(resize_handle(handle, cx))))
            .on_drag(ResizeDrag, {
                let active = self.active.clone();
                move |_, _, _, cx| {
                    cx.stop_propagation();
                    active.set(Some(placement));
                    cx.new(|_| ResizeDrag)
                }
            });
        let dock = dock.clone();
        let active = self.active.clone();
        let tracker = canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                // Dragging leaves the handle's hit band, so track the window's mouse stream.
                window.on_mouse_event({
                    let dock = dock.clone();
                    let active = active.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase.bubble() && active.get() == Some(placement) {
                            dock.resize_to(event.position, window, cx);
                        }
                    }
                });
                window.on_mouse_event({
                    let dock = dock.clone();
                    let active = active.clone();
                    let area = area.clone();
                    move |_: &MouseUpEvent, phase, window, cx| {
                        if phase.bubble() && active.get() == Some(placement) {
                            active.set(None);
                            dock.end_resize(window, cx);
                            drop(area.update(cx, |_, cx| cx.emit(base::DockEvent::LayoutChanged)));
                        }
                    }
                });
            },
        )
        .absolute()
        .size_full();
        div()
            .flex()
            .relative()
            .size_full()
            .child(content)
            .child(handle)
            .child(tracker)
            .into_any_element()
    }
}

#[derive(Clone)]
struct ResizeDrag;

impl Render for ResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Complementary insets avoid rounding both half-gutters down at fractional scale.
pub(crate) fn insets(gap: f32, display_scale: f32) -> gpui_kit::Edges<gpui_kit::Pixels> {
    let physical = (gap * display_scale).round();
    let leading = (physical / 2.0).floor();
    let trailing = physical - leading;
    gpui_kit::Edges {
        left: px(leading / display_scale),
        top: px(leading / display_scale),
        right: px(trailing / display_scale),
        bottom: px(trailing / display_scale),
    }
}

pub(crate) fn rounded(content: AnyElement, cx: &App) -> AnyElement {
    let t = theme(cx);
    let Some(i) = t.islands else { return content };
    div()
        .relative()
        .size_full()
        .overflow_hidden()
        .rounded(t.px(i.radius))
        .child(div().flex().flex_col().size_full().child(content))
        .child(corners(true, cx))
        .child(corners(false, cx))
        .into_any_element()
}

// GPUI clips descendants rectangularly. Repaint each corner notch with the
// window-coordinate backdrop so scrolled content cannot cover the island arc.
pub(crate) fn corners(top: bool, cx: &App) -> AnyElement {
    let t = theme(cx);
    let radius = t.px(t.islands.expect("island renderer").radius);
    canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            use gpui_kit::{Bounds, PathBuilder, point};
            let y = if top { bounds.top() } else { bounds.bottom() };
            let dy = if top { 1. } else { -1. };
            for (x, dx) in [(bounds.left(), 1.), (bounds.right(), -1.)] {
                let corner = point(x, y);
                let mut path = PathBuilder::fill();
                path.move_to(corner);
                path.line_to(point(x + radius * dx, y));
                path.arc_to(
                    point(radius, radius),
                    px(0.),
                    false,
                    dx * dy < 0.,
                    point(x, y + radius * dy),
                );
                path.close();
                let region = Bounds::from_corners(
                    point(x.min(x + radius * dx), y.min(y + radius * dy)),
                    point(x.max(x + radius * dx), y.max(y + radius * dy)),
                );
                if let Ok(path) = path.build() {
                    for layer in backdrop(region, cx) {
                        window.paint_path(path.clone(), layer);
                    }
                }
            }
        },
    )
    .absolute()
    .inset_0()
    .size_full()
    .into_any_element()
}

#[derive(Default)]
pub(crate) struct Tabs {
    scroll: ScrollHandle,
    active: Cell<Option<usize>>,
    narrow: Rc<Cell<bool>>,
}

impl Tabs {
    pub(crate) fn render(
        &self,
        group: &base::TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let t = *theme(cx);
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
        if self.active.replace(Some(group.active_ix())) != Some(group.active_ix())
            && let Some(ix) = visible.iter().position(|(ix, _)| *ix == group.active_ix())
        {
            self.scroll.scroll_to_item(ix);
        }
        let selected = group.active_panel().map(|p| p.panel_id(cx));
        let visible_indices = visible.iter().map(|(ix, _)| *ix).collect::<Vec<_>>();
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
                    .h(t.px(32.0))
                    .px(t.px(12.0))
                    .flex_none()
                    .whitespace_nowrap()
                    .tab_stop(true)
                    .focus_visible(move |s| s.border_color(t.border_focused))
                    .hover(move |s| s.bg(t.panel.bg).text_color(t.panel.text))
                    .on_key_down({
                        let group = group.clone();
                        let indices = visible_indices.clone();
                        move |event, window, cx| {
                            let next = match event.keystroke.key.as_str() {
                                "left" => Some((position + indices.len() - 1) % indices.len()),
                                "right" => Some((position + 1) % indices.len()),
                                "home" => Some(0),
                                "end" => Some(indices.len() - 1),
                                "enter" | "space" => Some(position),
                                _ => None,
                            };
                            if let Some(next) = next {
                                cx.stop_propagation();
                                group.select_tab(indices[next], window, cx);
                            }
                        }
                    })
                    .child(title)
                    .selected(active)
                    .rounded_t(t.px(10.0))
                    .border_1()
                    .border_color(t.border_variant)
                    .when(active, gpui_kit::Styled::border_b_0)
                    .bg(if active {
                        t.panel.bg
                    } else {
                        t.panel.bg.blend(gpui_kit::black().opacity(0.12))
                    })
                    .text_color(if active && focused {
                        t.panel.text
                    } else {
                        t.panel.text_muted
                    })
                    .debug_selector(move || format!("dock-tab-{ix}"))
                    .on_click({
                        let group = group.clone();
                        move |_, window, cx| group.select_tab(ix, window, cx)
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
        let dense_toolbar = buttons.len() > 3;
        let stacked = dense_toolbar && self.narrow.get();
        let toolbar = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .children(handle.and_then(|h| h.title_suffix(window, cx)))
            .children(buttons.into_iter().map(ButtonVariants::ghost))
            .when(zoomable || group.is_zoomed(), |el| {
                el.child(
                    action_button(
                        "island-zoom",
                        if group.is_zoomed() {
                            IconName::Minimize
                        } else {
                            IconName::Maximize
                        },
                        cx,
                    )
                    .ghost()
                    .tooltip("Zoom panel")
                    .on_click({
                        let group = group.clone();
                        move |_, w, cx| group.toggle_zoom(w, cx)
                    }),
                )
            })
            .child(
                action_button("island-menu", IconName::Ellipsis, cx)
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
            .id("island-tab-end")
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
                TabList::new("island-tabs")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0()
                    .overflow_x_scroll()
                    .track_scroll(&self.scroll)
                    .children(tabs)
                    .child(trailing),
            )
            .when(tab_count > 1, |el| {
                el.child(
                    action_button("island-tab-list", IconName::ChevronDown, cx)
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
                                        .on_click(move |_, w, cx| group.select_tab(ix, w, cx)),
                                    );
                                }
                                menu
                            }
                        }),
                )
            });
        let header = div().relative().flex_none().bg(t.panel.bg).on_prepaint({
            let narrow = self.narrow.clone();
            move |bounds, window, _| {
                let next = bounds.size.width < t.px(640.);
                if narrow.replace(next) != next && dense_toolbar {
                    window.refresh();
                }
            }
        });
        let header = if stacked {
            header.child(row).child(
                div()
                    .id("island-toolbar")
                    .mt(t.px(3.))
                    .w_full()
                    .overflow_x_scroll()
                    .child(toolbar),
            )
        } else {
            header.child(row.child(toolbar))
        };
        header.child(corners(true, cx)).into_any_element()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_insets_share_one_device_rounded_gap() {
        for zoom in [0.75, 1.0, 1.25, 1.5, 2.0] {
            for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
                let gap = 4.0 * zoom;
                let i = insets(gap, scale);
                let leading = f32::from(i.left) * scale;
                let trailing = f32::from(i.right) * scale;
                assert!((leading - leading.round()).abs() < 0.0001);
                assert!((trailing - trailing.round()).abs() < 0.0001);
                assert!((leading + trailing - (gap * scale).round()).abs() < 0.0001);
                assert!((leading - trailing).abs() <= 1.0);
                assert_eq!(i.top, i.left);
                assert_eq!(i.bottom, i.right);
            }
        }
    }
}
