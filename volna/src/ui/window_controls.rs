//! Window controls (minimize, maximize/restore, close) for the title bar Volna draws
//! itself, after Zed's `platform_title_bar`:

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Decorations, ElementId, IntoElement, MouseButton, ParentElement, RenderOnce,
    Styled, Window, WindowButton, WindowButtonLayout, WindowControlArea, WindowControls, div,
};

use crate::app::CloseWindow;
use crate::theme::{ThemePx, theme};
use crate::ui::{Icon, IconName};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
}

const LINUX: bool = cfg!(any(target_os = "linux", target_os = "freebsd"));
const WINDOWS: bool = cfg!(target_os = "windows");

pub fn app_draws_controls(window: &Window) -> bool {
    (LINUX && matches!(window.window_decorations(), Decorations::Client { .. })) || WINDOWS
}

pub fn render_window_controls(side: Side, window: &Window, cx: &App) -> Option<AnyElement> {
    if !app_draws_controls(window) || window.is_fullscreen() {
        return None;
    }
    if WINDOWS {
        return (side == Side::Right).then(|| {
            WindowsControls {
                buttons: visible_buttons(&linux_default().right, window.window_controls()),
            }
            .into_any_element()
        });
    }
    let layout = cx.button_layout().unwrap_or_else(linux_default);
    let buttons = visible_buttons(
        match side {
            Side::Left => &layout.left,
            Side::Right => &layout.right,
        },
        window.window_controls(),
    );
    (!buttons.is_empty()).then(|| {
        LinuxControls {
            id: match side {
                Side::Left => "left-window-controls",
                Side::Right => "right-window-controls",
            },
            buttons,
        }
        .into_any_element()
    })
}

fn linux_default() -> WindowButtonLayout {
    WindowButtonLayout {
        left: [None; gpui_kit::MAX_BUTTONS_PER_SIDE],
        right: [
            Some(WindowButton::Minimize),
            Some(WindowButton::Maximize),
            Some(WindowButton::Close),
        ],
    }
}

fn visible_buttons(side: &[Option<WindowButton>], supported: WindowControls) -> Vec<WindowButton> {
    side.iter()
        .flatten()
        .copied()
        .filter(|button| match button {
            WindowButton::Minimize => supported.minimize,
            WindowButton::Maximize => supported.maximize,
            WindowButton::Close => true,
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Control {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl Control {
    fn for_button(button: WindowButton, maximized: bool) -> Self {
        match button {
            WindowButton::Minimize => Control::Minimize,
            WindowButton::Maximize if maximized => Control::Restore,
            WindowButton::Maximize => Control::Maximize,
            WindowButton::Close => Control::Close,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Control::Minimize => "minimize",
            Control::Maximize => "maximize",
            Control::Restore => "restore",
            Control::Close => "close",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Control::Minimize => IconName::WindowMinimize,
            Control::Maximize => IconName::WindowMaximize,
            Control::Restore => IconName::WindowRestore,
            Control::Close => IconName::WindowClose,
        }
    }

    fn enabled(self, window: &Window) -> bool {
        match self {
            Control::Minimize => window.is_minimizable(),
            Control::Maximize | Control::Restore => window.is_resizable(),
            Control::Close => true,
        }
    }

    fn area(self) -> WindowControlArea {
        match self {
            Control::Minimize => WindowControlArea::Min,
            Control::Maximize | Control::Restore => WindowControlArea::Max,
            Control::Close => WindowControlArea::Close,
        }
    }

    fn activate(self, window: &mut Window, cx: &mut App) {
        match self {
            Control::Minimize => window.minimize_window(),
            Control::Maximize | Control::Restore => window.zoom_window(),
            Control::Close => window.dispatch_action(Box::new(CloseWindow), cx),
        }
    }
}

#[derive(IntoElement)]
struct LinuxControls {
    id: &'static str,
    buttons: Vec<WindowButton>,
}

impl RenderOnce for LinuxControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let maximized = window.is_maximized();
        let t = *theme(cx);
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .h_full()
            .gap(t.px(12.0))
            .px(t.px(12.0))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(self.buttons.into_iter().map(|button| {
                let control = Control::for_button(button, maximized);
                let enabled = control.enabled(window);
                let size = t.px(24.0);
                div()
                    .id(ElementId::from(control.id()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(size)
                    .h(size)
                    .rounded_full()
                    .text_color(if enabled {
                        t.bar.icon
                    } else {
                        t.bar.text_placeholder
                    })
                    .when(enabled, |el| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(t.bar_hover.bg).text_color(t.bar_hover.icon))
                            .active(|s| s.bg(t.bar_hover.bg).text_color(t.bar_hover.icon))
                    })
                    .child(Icon::new(control.icon()).size(t.px(20.0)).inherit_color())
                    .on_mouse_move(|_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        if enabled {
                            control.activate(window, cx);
                        }
                    })
            }))
    }
}

#[derive(IntoElement)]
struct WindowsControls {
    buttons: Vec<WindowButton>,
}

impl RenderOnce for WindowsControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let maximized = window.is_maximized();
        let t = *theme(cx);
        div()
            .id("windows-window-controls")
            .flex()
            .flex_none()
            .items_stretch()
            .h_full()
            .children(self.buttons.into_iter().map(|button| {
                let control = Control::for_button(button, maximized);
                let enabled = control.enabled(window);
                let close = control == Control::Close;
                let (hover_bg, hover_fg) = if close {
                    (t.bar.error, gpui_kit::white())
                } else {
                    (t.bar_hover.bg, t.bar_hover.icon)
                };
                div()
                    .id(ElementId::from(control.id()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .occlude()
                    .w(t.px(46.0))
                    .h_full()
                    .text_color(if enabled {
                        t.bar.icon
                    } else {
                        t.bar.text_placeholder
                    })
                    .when(enabled, |el| {
                        el.hover(|s| s.bg(hover_bg).text_color(hover_fg))
                            .active(|s| s.bg(hover_bg.opacity(0.8)).text_color(hover_fg))
                    })
                    .window_control_area(control.area())
                    .child(Icon::new(control.icon()).size(t.px(20.0)).inherit_color())
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> WindowControls {
        WindowControls {
            fullscreen: true,
            maximize: true,
            minimize: true,
            window_menu: true,
        }
    }

    #[test]
    fn default_layout_puts_every_control_on_the_right() {
        let layout = linux_default();
        assert!(visible_buttons(&layout.left, all()).is_empty());
        assert_eq!(
            visible_buttons(&layout.right, all()),
            [
                WindowButton::Minimize,
                WindowButton::Maximize,
                WindowButton::Close
            ]
        );
    }

    #[test]
    fn unsupported_controls_are_dropped_but_close_stays() {
        let none = WindowControls {
            fullscreen: false,
            maximize: false,
            minimize: false,
            window_menu: false,
        };
        let layout = linux_default();
        assert_eq!(visible_buttons(&layout.right, none), [WindowButton::Close]);
    }

    #[test]
    fn gnome_left_layout_is_honoured() {
        let layout = WindowButtonLayout {
            left: [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None,
            ],
            right: [Some(WindowButton::Maximize), None, None],
        };
        assert_eq!(
            visible_buttons(&layout.left, all()),
            [WindowButton::Close, WindowButton::Minimize]
        );
        assert_eq!(
            visible_buttons(&layout.right, all()),
            [WindowButton::Maximize]
        );
    }

    #[test]
    fn maximize_becomes_restore_when_maximized() {
        assert_eq!(
            Control::for_button(WindowButton::Maximize, true),
            Control::Restore
        );
        assert_eq!(
            Control::for_button(WindowButton::Maximize, false),
            Control::Maximize
        );
    }
}
