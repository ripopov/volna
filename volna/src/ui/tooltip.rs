//! Shared hover timing and hierarchy description tooltips.

use std::time::Duration;

use crate::theme::ThemePx;

use gpui_kit::component::tooltip::{Tooltip, TooltipDefaults};
use gpui_kit::{AnyView, App, Global, SharedString, Window, div, prelude::*, px};

struct TooltipDelay(Duration);

impl Global for TooltipDelay {}

pub(crate) fn set_delay(delay: Duration, cx: &mut App) {
    cx.set_global(TooltipDelay(delay));
    TooltipDefaults::new().with_show_delay(delay).install(cx);
}

pub(crate) fn delay(cx: &App) -> Duration {
    cx.global::<TooltipDelay>().0
}

pub(crate) fn hierarchy(
    summary: String,
    description: SharedString,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    if description.is_empty() {
        return Tooltip::new(summary).build(window, cx);
    }
    Tooltip::element(move |window, cx| {
        let t = crate::theme::theme(cx);
        let width = t
            .px(480.0)
            .min((window.viewport_size().width - px(48.0)).max(px(1.0)));
        div()
            .max_w(width)
            .flex()
            .flex_col()
            .gap_2()
            .whitespace_normal()
            .child(summary.clone())
            .child(
                div()
                    .debug_selector(|| "hierarchy-description".into())
                    .child(description.clone()),
            )
    })
    .build(window, cx)
}
