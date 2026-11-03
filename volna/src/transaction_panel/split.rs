//! Current-frame layout and interaction for the record inspector's two panes.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AvailableSpace, Bounds, Context, CursorStyle, Element, ElementId, Entity,
    FocusHandle, GlobalElementId, InspectorElementId, KeyDownEvent, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Style, Window, deferred, div, px,
    relative, size,
};
use volna_core::transaction::view::{DEFAULT_CURSOR_HEIGHT, MAX_CURSOR_HEIGHT, MIN_CURSOR_HEIGHT};

use crate::theme::{ThemePx, theme};
use crate::ui::{Splitter, SplitterAxis};

type ListBuilder = dyn FnOnce(f32, &mut Window, &mut App) -> AnyElement;
type HeightChanged = dyn Fn(f32, &mut Window, &mut App);

pub(super) struct RecordSplit {
    focus: FocusHandle,
    preferred: f32,
    geometry: Geometry,
    drag: Option<Drag>,
    on_change: Rc<HeightChanged>,
}

#[derive(Clone, Copy)]
struct Geometry {
    height: f32,
    min: f32,
    max: f32,
    zoom: f32,
}

impl Geometry {
    fn new(available: Pixels, zoom: f32, preferred: f32) -> Self {
        let available = ((f32::from(available) - 1.0) / zoom).max(0.0);
        let min = MIN_CURSOR_HEIGHT.min(available / 2.0);
        let max = (available - 160.0).max(min).min(MAX_CURSOR_HEIGHT);
        Self {
            height: preferred.clamp(min, max),
            min,
            max,
            zoom,
        }
    }
}

struct Drag {
    start: Pixels,
    height: f32,
    original: f32,
}

impl RecordSplit {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle().tab_stop(true),
            preferred: DEFAULT_CURSOR_HEIGHT,
            geometry: Geometry::new(px(0.0), 1.0, DEFAULT_CURSOR_HEIGHT),
            drag: None,
            on_change: Rc::new(|_, _, _| {}),
        }
    }

    fn set_height(&mut self, height: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.preferred = height.clamp(MIN_CURSOR_HEIGHT, MAX_CURSOR_HEIGHT);
        (self.on_change)(self.preferred, window, cx);
        cx.notify();
    }

    fn start_drag(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        if event.click_count == 2 {
            self.drag = None;
            self.set_height(DEFAULT_CURSOR_HEIGHT, window, cx);
        } else {
            self.drag = Some(Drag {
                start: event.position.y,
                height: self.geometry.height,
                original: self.preferred,
            });
            cx.notify();
        }
    }

    fn drag_to(&mut self, y: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = &self.drag else { return };
        let height = drag.height + f32::from(y - drag.start) / self.geometry.zoom;
        self.set_height(
            height.clamp(self.geometry.min, self.geometry.max),
            window,
            cx,
        );
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Geometry {
            height, min, max, ..
        } = self.geometry;
        let step = if event.keystroke.modifiers.shift {
            64.0
        } else {
            16.0
        };
        let next = match event.keystroke.key.as_str() {
            "up" => height - step,
            "down" => height + step,
            "home" => min,
            "end" => max,
            "enter" => DEFAULT_CURSOR_HEIGHT,
            "escape" => {
                if let Some(drag) = self.drag.take() {
                    self.set_height(drag.original, window, cx);
                }
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        self.set_height(next.clamp(min, max), window, cx);
        cx.stop_propagation();
    }

    fn content(
        &self,
        list: AnyElement,
        details: AnyElement,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let t = *theme(cx);
        let Geometry {
            height, min, max, ..
        } = self.geometry;
        let divider = div()
            .id("tx-cursor-divider")
            .debug_selector(|| "tx-cursor-divider".into())
            .track_focus(&self.focus)
            .tab_stop(true)
            .role(gpui_kit::accesskit::Role::Splitter)
            .aria_label("Resize transactions at cursor")
            .aria_description("Up and Down resize. Shift resizes faster. Enter or double-click resets. Escape cancels a drag.")
            .aria_orientation(gpui_kit::accesskit::Orientation::Horizontal)
            .aria_numeric_value(f64::from(height))
            .aria_min_numeric_value(f64::from(min))
            .aria_max_numeric_value(f64::from(max))
            .aria_numeric_value_step(16.0)
            .w_full().h(px(1.0)).flex_none()
            .focus_visible(|s| s.bg(t.border_focused))
            .on_key_down(cx.listener(Self::key_down))
            .child(Splitter::new("tx-cursor-grab", SplitterAxis::Horizontal)
                .dragging(self.drag.is_some() || self.focus.is_focused(window))
                .on_drag_start(cx.listener(Self::start_drag)));
        div()
            .id("tx-cursor-split")
            .debug_selector(|| "tx-cursor-split".into())
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .h(t.px(height))
                    .flex_none()
                    .min_h_0()
                    .overflow_hidden()
                    .child(list),
            )
            .child(divider)
            .child(
                div()
                    .debug_selector(|| "tx-details-viewport".into())
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(details),
            )
            .when(self.drag.is_some(), |split| {
                split.child(
                    deferred(
                        div()
                            .id("tx-cursor-drag-surface")
                            .absolute()
                            .inset_0()
                            .occlude()
                            .cursor(CursorStyle::ResizeUpDown),
                    )
                    .with_priority(2),
                )
            })
            .into_any_element()
    }

    fn track_drag(state: &Entity<Self>, window: &mut Window, cx: &App) {
        if state.read(cx).drag.is_none() {
            return;
        }
        window.set_window_cursor_style(CursorStyle::ResizeUpDown);
        let owner = state.downgrade();
        window.on_mouse_event({
            let owner = owner.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase.bubble() {
                    drop(owner.update(cx, |split, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            split.drag_to(event.position.y, window, cx);
                        } else if split.drag.take().is_some() {
                            cx.notify();
                        }
                    }));
                }
            }
        });
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase.bubble() && event.button == MouseButton::Left {
                drop(owner.update(cx, |split, cx| {
                    split.drag_to(event.position.y, window, cx);
                    if split.drag.take().is_some() {
                        cx.notify();
                    }
                }));
            }
        });
    }
}

pub(super) fn element(
    state: Entity<RecordSplit>,
    preferred: f32,
    details: AnyElement,
    list: impl FnOnce(f32, &mut Window, &mut App) -> AnyElement + 'static,
    on_change: impl Fn(f32, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_hidden()
        .child(SplitLayout {
            state,
            preferred,
            list: Some(Box::new(list)),
            details: Some(details),
            on_change: Rc::new(on_change),
        })
        .into_any_element()
}

struct SplitLayout {
    state: Entity<RecordSplit>,
    preferred: f32,
    list: Option<Box<ListBuilder>>,
    details: Option<AnyElement>,
    on_change: Rc<HeightChanged>,
}

impl IntoElement for SplitLayout {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for SplitLayout {
    type RequestLayoutState = ();
    type PrepaintState = AnyElement;

    fn id(&self) -> Option<ElementId> {
        Some(("record-split", self.state.entity_id()).into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let geometry = Geometry::new(bounds.size.height, theme(cx).zoom, self.preferred);
        // Final pane space is known here; child layout must use it in this frame.
        let list = self.list.take().expect("split list is built once")(geometry.height, window, cx);
        let details = self.details.take().expect("split details are built once");
        let mut content = self.state.update(cx, |split, cx| {
            split.preferred = self.preferred;
            split.geometry = geometry;
            split.on_change = self.on_change.clone();
            split.content(list, details, window, cx)
        });
        content.prepaint_as_root(
            bounds.origin,
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::Definite(bounds.size.height),
            ),
            window,
            cx,
        );
        content
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        (): &mut (),
        content: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        content.paint(window, cx);
        RecordSplit::track_drag(&self.state, window, cx);
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;
    use gpui_kit::{Render, Size, TestAppContext, VisualTestContext};
    use std::cell::RefCell;

    struct Inspector {
        focus: FocusHandle,
        split: Option<Entity<RecordSplit>>,
        allocated: Size<Pixels>,
        preferred: f32,
        list_heights: Rc<RefCell<Vec<f32>>>,
    }

    impl Render for Inspector {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let Some(split) = &self.split else {
                return div().into_any_element();
            };
            let list_heights = self.list_heights.clone();
            let owner = cx.entity().downgrade();
            div()
                .id("split-test-inspector")
                .track_focus(&self.focus)
                .w(self.allocated.width)
                .h(self.allocated.height)
                .flex()
                .flex_col()
                .child(div().h(theme(cx).px(40.0)).flex_none())
                .child(element(
                    split.clone(),
                    self.preferred,
                    div().size_full().into_any_element(),
                    move |height, _, _| {
                        list_heights.borrow_mut().push(height);
                        div()
                            .debug_selector(|| "split-test-list".into())
                            .size_full()
                            .into_any_element()
                    },
                    move |height, _, cx| {
                        drop(owner.update(cx, |view, cx| {
                            view.preferred = height;
                            cx.notify();
                        }));
                    },
                ))
                .into_any_element()
        }
    }

    #[gpui_kit::test]
    fn first_frame_uses_current_panel_space_on_open_resize_and_zoom(cx: &mut TestAppContext) {
        cx.update(crate::init_app);
        let list_heights = Rc::new(RefCell::new(Vec::new()));
        let host = cx.add_window(|_, cx| Inspector {
            focus: cx.focus_handle(),
            split: None,
            allocated: size(px(360.0), px(600.0)),
            preferred: DEFAULT_CURSOR_HEIGHT,
            list_heights: list_heights.clone(),
        });
        let mut vcx = VisualTestContext::from_window(host.into(), cx);
        vcx.simulate_resize(size(px(1200.0), px(1000.0)));
        for dark in [true, false] {
            for zoom in [1.0, 1.5] {
                host.update(&mut vcx, |view, _, cx| {
                    crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
                    crate::theme::set_zoom(zoom, cx);
                    view.split = Some(cx.new(RecordSplit::new));
                    view.allocated = size(px(360.0 * zoom), px(600.0 * zoom));
                    view.preferred = DEFAULT_CURSOR_HEIGHT;
                    list_heights.borrow_mut().clear();
                    cx.notify();
                })
                .unwrap();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                assert_eq!(
                    list_heights.borrow()[0],
                    DEFAULT_CURSOR_HEIGHT,
                    "the first frame must reserve the preferred list height"
                );
                let list = vcx.debug_bounds("split-test-list").unwrap();
                assert!((f32::from(list.size.height) - DEFAULT_CURSOR_HEIGHT * zoom).abs() <= 1.0);

                let next_zoom = if zoom == 1.0 { 1.5 } else { 1.0 };
                host.update(&mut vcx, |_, _, cx| {
                    list_heights.borrow_mut().clear();
                    crate::theme::set_zoom(next_zoom, cx);
                })
                .unwrap();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                assert_eq!(list_heights.borrow()[0], DEFAULT_CURSOR_HEIGHT);
                assert!(
                    (f32::from(vcx.debug_bounds("split-test-list").unwrap().size.height)
                        - DEFAULT_CURSOR_HEIGHT * next_zoom)
                        .abs()
                        <= 1.0
                );

                host.update(&mut vcx, |view, _, cx| {
                    list_heights.borrow_mut().clear();
                    crate::theme::set_zoom(zoom, cx);
                    view.preferred = MAX_CURSOR_HEIGHT;
                    cx.notify();
                })
                .unwrap();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                let expanded = vcx.debug_bounds("split-test-list").unwrap();
                let details = vcx.debug_bounds("tx-details-viewport").unwrap();
                assert!(details.size.height >= px(160.0 * zoom - 1.0));

                host.update(&mut vcx, |view, _, cx| {
                    list_heights.borrow_mut().clear();
                    view.allocated.height = px(200.0 * zoom);
                    cx.notify();
                })
                .unwrap();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                assert!(
                    list_heights.borrow()[0] > 0.0 && list_heights.borrow()[0] <= 80.0,
                    "the first shrinking frame must use the new panel space"
                );
                let split = vcx.debug_bounds("tx-cursor-split").unwrap();
                let list = vcx.debug_bounds("split-test-list").unwrap();
                let details = vcx.debug_bounds("tx-details-viewport").unwrap();
                assert!(list.size.height > px(0.0) && details.size.height > px(0.0));
                assert!(
                    list.bottom() < details.top() && details.bottom() <= split.bottom() + px(1.0),
                    "both panes must fit on the first frame after shrinking"
                );
                assert!(list.size.height < expanded.size.height);

                host.update(&mut vcx, |view, _, cx| {
                    assert_eq!(view.preferred, MAX_CURSOR_HEIGHT);
                    list_heights.borrow_mut().clear();
                    view.allocated.height = px(600.0 * zoom);
                    cx.notify();
                })
                .unwrap();
                vcx.update(|window, cx| window.draw(cx).clear(cx));
                assert!(
                    (list_heights.borrow()[0] * zoom - f32::from(expanded.size.height)).abs()
                        <= 1.0
                );
                assert_eq!(
                    vcx.debug_bounds("split-test-list").unwrap(),
                    expanded,
                    "enlarging restores the split on the first frame"
                );
            }
        }
    }

    #[gpui_kit::test]
    fn divider_is_reachable_by_tab_and_keeps_keyboard_resize_local(cx: &mut TestAppContext) {
        cx.update(crate::init_app);
        let mut inspector = None;
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| Inspector {
                focus: cx.focus_handle(),
                split: Some(cx.new(RecordSplit::new)),
                allocated: size(px(360.0), px(600.0)),
                preferred: DEFAULT_CURSOR_HEIGHT,
                list_heights: Rc::default(),
            });
            inspector = Some(view.clone());
            gpui_kit::component::Root::new(view, window, cx)
        });
        let host = inspector.unwrap();
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.simulate_resize(size(px(800.0), px(800.0)));
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        vcx.update(|window, cx| {
            let focus = host.read(cx).focus.clone();
            window.focus(&focus, cx);
        });
        for _ in 0..4 {
            vcx.simulate_keystrokes("tab");
            vcx.run_until_parked();
            vcx.simulate_keystrokes("down");
            vcx.run_until_parked();
            if host.read_with(&vcx, |view, _| view.preferred) != DEFAULT_CURSOR_HEIGHT {
                break;
            }
        }
        host.update(&mut vcx, |view, _| {
            assert_eq!(view.preferred, DEFAULT_CURSOR_HEIGHT + 16.0);
        });
        vcx.simulate_keystrokes("shift-up");
        host.update(&mut vcx, |view, _| {
            assert_eq!(view.preferred, DEFAULT_CURSOR_HEIGHT - 48.0);
        });
        vcx.simulate_keystrokes("enter");
        host.update(&mut vcx, |view, _| {
            assert_eq!(view.preferred, DEFAULT_CURSOR_HEIGHT);
        });
    }
}
