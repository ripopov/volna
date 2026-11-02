//! Shared hover timing for toolkit components and direct GPUI tooltips.

use std::time::Duration;

use gpui_kit::component::tooltip::TooltipDefaults;
use gpui_kit::{App, Global};

struct TooltipDelay(Duration);

impl Global for TooltipDelay {}

pub(crate) fn set_delay(delay: Duration, cx: &mut App) {
    cx.set_global(TooltipDelay(delay));
    TooltipDefaults::new().with_show_delay(delay).install(cx);
}

pub(crate) fn delay(cx: &App) -> Duration {
    cx.global::<TooltipDelay>().0
}
