//! Browser clipboard recovery.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/clipboard.js")]
extern "C" {
    #[wasm_bindgen(js_name = copyText)]
    fn copy_text(text: &str);
}

pub(crate) fn copy(text: &str) {
    copy_text(text);
}
