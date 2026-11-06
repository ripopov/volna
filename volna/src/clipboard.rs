//! Platform clipboard delivery with browser recovery.

use std::borrow::Cow;

#[cfg(target_family = "wasm")]
use wasm_bindgen::prelude::*;

#[cfg(target_family = "wasm")]
#[wasm_bindgen(module = "/src/clipboard.js")]
extern "C" {
    #[wasm_bindgen(js_name = copyText)]
    fn copy_text(text: &str);
}

pub(crate) fn copy<'a>(text: impl Into<Cow<'a, str>>, cx: &mut gpui_kit::App) {
    let text = text.into();
    #[cfg(target_family = "wasm")]
    {
        let _ = cx;
        copy_text(&text);
    }
    #[cfg(not(target_family = "wasm"))]
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text.into_owned()));
}
