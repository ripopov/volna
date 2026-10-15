//! Shared action-button geometry and surface styling over GPUI Kit's buttons.

use super::IconName;
use crate::theme::{Surface, ThemePx, theme};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonCustomVariant, ButtonVariants},
};
use gpui_kit::{App, ElementId, Styled};

pub fn action_button(
    id: impl Into<ElementId>,
    icon: impl Into<gpui_kit::component::Icon>,
    cx: &App,
) -> Button {
    let t = theme(cx);
    // The toolkit allocates 75% of its custom size to the icon; retain a 24px target.
    Button::new(id)
        .icon(icon.into())
        .with_size(t.px(16.0) / 0.75)
        .min_w(t.px(24.0))
        .h(t.px(24.0))
}

pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    normal: Surface,
    hover: Surface,
    cx: &App,
) -> Button {
    action_button(id, gpui_kit::component::Icon::empty().path(icon.path()), cx).custom(
        ButtonCustomVariant::new(cx)
            .color(gpui_kit::transparent_black())
            .foreground(normal.icon)
            .hover(hover.bg)
            .active(hover.bg),
    )
}
