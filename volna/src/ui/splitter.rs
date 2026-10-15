use gpui_kit::{
    App, CursorStyle, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, RenderOnce, Styled, Window, deferred, div, px,
};

use crate::theme::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplitterAxis {
    Vertical,
    Horizontal,
}

#[derive(IntoElement)]
pub struct Splitter {
    id: ElementId,
    axis: SplitterAxis,
    dragging: bool,
    line_width: f32,
    on_drag_start: Option<Box<DragStartHandler>>,
}

impl Splitter {
    pub fn new(id: impl Into<ElementId>, axis: SplitterAxis) -> Self {
        Splitter {
            id: id.into(),
            axis,
            dragging: false,
            line_width: 1.0,
            on_drag_start: None,
        }
    }

    pub fn line_width(mut self, width: f32) -> Self {
        self.line_width = width;
        self
    }

    pub fn dragging(mut self, dragging: bool) -> Self {
        self.dragging = dragging;
        self
    }

    pub fn on_drag_start(
        mut self,
        f: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_drag_start = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Splitter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let grab = px(t.splitter_grab);
        let base_color = t.border;
        let active_color = t.border_focused;
        let cursor = match self.axis {
            SplitterAxis::Vertical => CursorStyle::ResizeLeftRight,
            SplitterAxis::Horizontal => CursorStyle::ResizeUpDown,
        };
        let highlight = div().group_hover("splitter", move |s| s.bg(active_color));
        let highlight = if self.dragging {
            highlight.bg(active_color)
        } else {
            highlight
        };
        let thickness = if t.islands.is_some() {
            crate::islands::resize_thickness(self.dragging, cx)
        } else {
            px(1.0)
        };
        let highlight = if t.islands.is_some() {
            highlight.rounded(thickness / 2.0)
        } else {
            highlight
        };
        let highlight = match self.axis {
            SplitterAxis::Vertical => highlight.w(thickness).h_full(),
            SplitterAxis::Horizontal => highlight.h(thickness).w_full(),
        };
        let mut grab_zone = div()
            .id(self.id)
            .group("splitter")
            .absolute()
            .cursor(cursor)
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .child(highlight);
        grab_zone = match self.axis {
            SplitterAxis::Vertical => grab_zone.top_0().bottom_0().left(-(grab / 2.0)).w(grab),
            SplitterAxis::Horizontal => grab_zone.left_0().right_0().top(-(grab / 2.0)).h(grab),
        };
        if let Some(f) = self.on_drag_start {
            grab_zone = grab_zone.on_mouse_down(MouseButton::Left, f);
        }
        let line = div().bg(base_color);
        let width = px(self.line_width);
        let line = match self.axis {
            SplitterAxis::Vertical => line.w(width).h_full(),
            SplitterAxis::Horizontal => line.h(width).w_full(),
        };
        let zone = div().relative().flex_none();
        let zone = match self.axis {
            SplitterAxis::Vertical => zone.w(width).h_full(),
            SplitterAxis::Horizontal => zone.h(width).w_full(),
        };
        zone.child(line).child(deferred(grab_zone).with_priority(1))
    }
}

type DragStartHandler = dyn Fn(&MouseDownEvent, &mut Window, &mut App);
