//! Theme-derived surfaces and ink for the instruction navigator.

use super::{Theme, contrast, fallback_text, over, stroke};
use crate::color::Color;

pub(crate) struct NavigatorStyle {
    pub background: Color,
    pub stream: Color,
    pub aborted: Color,
    pub viewport: Color,
    pub edge: Color,
}

fn viewport_tint(accent: Color, background: Color) -> Color {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..16 {
        let alpha = f32::midpoint(lo, hi);
        if contrast(over(accent.with_alpha(alpha), background), background) >= 1.5 {
            hi = alpha;
        } else {
            lo = alpha;
        }
    }
    over(accent.with_alpha(hi), background)
}

impl Theme {
    pub(crate) fn navigator_style(&self) -> NavigatorStyle {
        let accent = self.editor.icon_accent.with_alpha(1.0);
        let surface = over(self.panel.bg.with_alpha(0.45), self.editor.bg);
        let background = over(accent.with_alpha(0.025), surface);
        let viewport = viewport_tint(stroke(accent, &[background]), background);
        let backgrounds = [background, viewport];
        NavigatorStyle {
            background,
            stream: stroke(over(accent.with_alpha(0.60), background), &backgrounds),
            aborted: stroke(self.editor.error.with_alpha(1.0), &backgrounds),
            viewport,
            edge: fallback_text(self.editor.text.with_alpha(1.0), background),
        }
    }
}
