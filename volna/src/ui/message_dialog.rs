//! Selectable, wrapping diagnostic details.

use gpui_kit::component::{
    WindowExt,
    button::Button,
    input::{Textarea, TextareaState},
};
use gpui_kit::prelude::*;
use gpui_kit::{App, ClipboardItem, Focusable, SharedString, Window, div};

use crate::theme::{ThemePx, theme};

pub(crate) fn open_message_dialog(
    title: &'static str,
    message: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let message = message.into();
    let text = cx.new(|cx| {
        TextareaState::new(window, cx)
            .soft_wrap(true)
            .default_value(message.clone())
    });
    let focus = text.read(cx).focus_handle(cx);
    window.open_dialog(cx, move |dialog, window, cx| {
        let t = theme(cx);
        let height = t
            .px(280.0)
            .min((window.viewport_size().height - t.px(180.0)).max(t.px(60.0)));
        let message = message.clone();
        dialog
            .title(title)
            .child(
                div()
                    .debug_selector(|| "message-details".into())
                    .w_full()
                    .min_w_0()
                    .child(
                        Textarea::new(&text)
                            .readonly(true)
                            .aria_label(title)
                            .w_full()
                            .h(height),
                    ),
            )
            .footer(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("copy-message")
                            .debug_selector(|| "copy-message".into())
                            .label("Copy all")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    message.to_string(),
                                ));
                            }),
                    )
                    .child(
                        Button::new("close-message")
                            .debug_selector(|| "close-message".into())
                            .label("Close")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
    window.focus(&focus, cx);
}
