use gpui_kit::{App, Div, ParentElement, SharedString, Styled, div, px};

use crate::theme::{ThemePx, theme};

pub fn panel_header(title: impl Into<SharedString>, cx: &App) -> Div {
    let t = *theme(cx);
    let colors = t.panel;
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .h(px(t.header_height))
        .px(t.px(12.))
        .bg(t.islands.map_or(t.panel.bg, |islands| islands.heading))
        .border_b_1()
        .border_color(t.border_variant)
        .child(
            div()
                .font_family(t.ui_font)
                .text_size(px(t.ui_size))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(title.into()),
        )
}
