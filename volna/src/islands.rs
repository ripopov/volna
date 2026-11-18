//! Islands workspace chrome; docking state and actions stay in GPUI-KIT.

use std::{cell::Cell, rc::Rc};

use gpui_kit::base::StyledExt as _;
use gpui_kit::base::dock as base;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AppContext, Axis, Context, Empty, IntoElement, MouseMoveEvent, MouseUpEvent,
    Pixels, Render, WeakEntity, Window, canvas, deferred, div, px,
};

use crate::theme::{ThemePx, theme};

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
    let t = theme(cx);
    let flat = t.compact_chrome && t.islands.is_none();
    if !flat && handle.edge().is_some() && handle.state() == gpui_kit::base::ResizeHandleState::Idle
    {
        return div().into_any_element();
    }
    let thickness = resize_thickness(handle.is_active(), cx);
    let color = if handle.state() == gpui_kit::base::ResizeHandleState::Idle {
        if flat {
            t.border.blend(t.panel.text_muted.opacity(0.32))
        } else {
            t.border_variant
        }
    } else {
        theme(cx).border_focused
    };
    let line = div()
        .absolute()
        .bg(color)
        .rounded(thickness / 2.)
        .map(|line| match handle.axis() {
            Axis::Horizontal => line.left((px(1.) - thickness) / 2.).w(thickness).h_full(),
            Axis::Vertical => line.top((px(1.) - thickness) / 2.).h(thickness).w_full(),
        });
    div()
        .relative()
        .size_full()
        // Dock frames clip at the seam; the resize highlight may overhang it.
        .child(if handle.edge().is_some() {
            deferred(line).into_any_element()
        } else {
            line.into_any_element()
        })
        .into_any_element()
}

#[derive(Default)]
pub(crate) struct DockResize {
    active: Rc<Cell<Option<base::DockPlacement>>>,
    restore_width: Rc<Cell<Option<Pixels>>>,
    collapse: Rc<Cell<bool>>,
}

impl DockResize {
    pub(crate) fn render(
        &self,
        dock: &base::DockContext,
        content: AnyElement,
        area: WeakEntity<base::DockArea>,
        cx: &App,
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
        let min_width = theme(cx).px(280.);
        let handle = gpui_kit::base::resize_handle(id, axis)
            .inside(edge)
            .with_appearance(Rc::new(|handle, _, cx| Some(resize_handle(handle, cx))))
            .on_drag(ResizeDrag, {
                let active = self.active.clone();
                let restore_width = self.restore_width.clone();
                let collapse = self.collapse.clone();
                let area = area.clone();
                move |_, _, _, cx| {
                    cx.stop_propagation();
                    restore_width.set(
                        area.upgrade()
                            .and_then(|area| area.read(cx).dock_size(placement)),
                    );
                    collapse.set(false);
                    active.set(Some(placement));
                    cx.new(|_| ResizeDrag)
                }
            });
        let dock = dock.clone();
        let active = self.active.clone();
        let restore_width = self.restore_width.clone();
        let collapse = self.collapse.clone();
        let tracker = canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                // Dragging leaves the handle's hit band, so track the window's mouse stream.
                window.on_mouse_event({
                    let dock = dock.clone();
                    let active = active.clone();
                    let area = area.clone();
                    let collapse = collapse.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase.bubble() && active.get() == Some(placement) {
                            let mut position = event.position;
                            if placement == base::DockPlacement::Right
                                && let Some(area) = area.upgrade()
                            {
                                let right = area.read(cx).bounds().right();
                                let width = right - position.x;
                                collapse.set(width < min_width / 2.);
                                position.x = position.x.min(right - min_width);
                            }
                            dock.resize_to(position, window, cx);
                        }
                    }
                });
                window.on_mouse_event({
                    let dock = dock.clone();
                    let active = active.clone();
                    let area = area.clone();
                    let collapse = collapse.clone();
                    let restore_width = restore_width.clone();
                    move |_: &MouseUpEvent, phase, window, cx| {
                        if phase.bubble() && active.get() == Some(placement) {
                            active.set(None);
                            dock.end_resize(window, cx);
                            drop(area.update(cx, |area, cx| {
                                if collapse.replace(false) {
                                    area.set_dock_size(
                                        placement,
                                        restore_width.get().unwrap_or(min_width).max(min_width),
                                        window,
                                        cx,
                                    );
                                    if area.is_dock_open(placement) {
                                        area.toggle_dock(placement, window, cx);
                                    }
                                }
                                cx.emit(base::DockEvent::LayoutChanged);
                            }));
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

// Frame hooks receive their children later, so defer the corner masks until
// the region's panels have painted.
pub(crate) fn frame_corners(insets: gpui_kit::Edges<Pixels>, cx: &App) -> AnyElement {
    deferred(
        div().absolute().inset_0().paddings(insets).child(
            div()
                .relative()
                .size_full()
                .child(corners(true, cx))
                .child(corners(false, cx)),
        ),
    )
    .into_any_element()
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
