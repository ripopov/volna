//! Theme-coloured application marks, distinct from trace semantic icons.

use gpui_kit::{Hsla, Pixels, Styled, Svg, svg};

const MARK_PATH: &str = "branding/volna-mark.svg";
pub(crate) const ASSETS: [(&str, &[u8]); 1] = [(
    MARK_PATH,
    include_bytes!("../../assets/app-icon/volna-mark.svg"),
)];

pub(crate) fn mark(size: Pixels, color: Hsla) -> Svg {
    svg()
        .path(MARK_PATH)
        .size(size)
        .flex_none()
        .text_color(color)
}
