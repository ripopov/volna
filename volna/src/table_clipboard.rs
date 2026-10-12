//! Browser clipboard recovery.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/table_clipboard.js")]
extern "C" {
    #[wasm_bindgen(js_name = copyTableText)]
    fn copy_table_text(text: &str);
}

pub(crate) fn copy(text: &str) {
    copy_table_text(text);
}
