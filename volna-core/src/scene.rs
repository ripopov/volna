//! The display list a canvas view produces and a frontend paints.
//!
//! Primitives carry resolved colours (the core theme resolves them) and font
//! *roles*; a frontend maps roles to its own font handles. Text widths come back
//! through [`TextMeasure`], cached per string in [`TextCache`] so the core never
//! shapes the same label twice.

use std::collections::HashMap;

use crate::color::Color;
use crate::geometry::{CursorIcon, Point, Rect};
use crate::icons::IconName;
use std::sync::Arc;

mod presence;
pub use presence::PresenceMask;

/// A typeface role. The theme names the family and size for each role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontRole {
    Ui,
    UiMedium,
    UiSemibold,
    Mono,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    /// One mask texel per physical pixel in `rect`, with opaque `color` ink.
    /// Frontends own image conversion, upload and bounded texture lifetime.
    Presence {
        rect: Rect,
        mask: Arc<PresenceMask>,
        color: Color,
    },
    /// Filled rectangle with optional rounded corners and a 1 px-style border.
    Quad {
        rect: Rect,
        fill: Color,
        radius: f32,
        border_width: f32,
        border_color: Color,
    },
    /// Stroked line segments of one colour.
    Lines {
        segments: Vec<[Point; 2]>,
        color: Color,
        width: f32,
    },
    /// One line of text, vertically centred in a box of `height` starting at `origin`.
    Text {
        origin: Point,
        height: f32,
        text: String,
        font: FontRole,
        size: f32,
        color: Color,
    },
    /// A monochrome icon fitted to `rect`.
    Icon {
        name: IconName,
        rect: Rect,
        color: Color,
    },
    /// Clip subsequent primitives until the matching [`Prim::PopClip`].
    PushClip(Rect),
    PopClip,
}

#[derive(Default, Debug)]
pub struct Scene {
    pub prims: Vec<Prim>,
    /// Regions that want a specific pointer shape when hovered.
    pub cursors: Vec<(Rect, CursorIcon)>,
    /// A pointer shape that overrides everything (active drags).
    pub window_cursor: Option<CursorIcon>,
}

impl Scene {
    pub(crate) fn presence(&mut self, rect: Rect, mask: PresenceMask, color: Color) {
        const RUN_LIMIT: usize = 4096;
        let color = color.with_alpha(1.0);
        let mut runs = Vec::new();
        runs.extend(mask.runs().take(RUN_LIMIT + 1));
        if runs.len() > RUN_LIMIT {
            self.prims.push(Prim::Presence {
                rect,
                mask: Arc::new(mask),
                color,
            });
            return;
        }
        let pixel = rect.width() / mask.width() as f32;
        for (y, x) in runs {
            self.fill(
                Rect::from_xywh(
                    rect.left() + x.start as f32 * pixel,
                    rect.top() + y as f32 * pixel,
                    x.len() as f32 * pixel,
                    pixel,
                ),
                color,
            );
        }
    }
    pub fn clear(&mut self) {
        self.prims.clear();
        self.cursors.clear();
        self.window_cursor = None;
    }

    pub fn fill(&mut self, rect: Rect, color: Color) {
        self.prims.push(Prim::Quad {
            rect,
            fill: color,
            radius: 0.0,
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        });
    }

    pub fn quad(
        &mut self,
        rect: Rect,
        fill: Color,
        radius: f32,
        border_width: f32,
        border_color: Color,
    ) {
        self.prims.push(Prim::Quad {
            rect,
            fill,
            radius,
            border_width,
            border_color,
        });
    }

    pub fn text(
        &mut self,
        origin: Point,
        height: f32,
        text: impl Into<String>,
        font: FontRole,
        size: f32,
        color: Color,
    ) {
        self.prims.push(Prim::Text {
            origin,
            height,
            text: text.into(),
            font,
            size,
            color,
        });
    }

    pub fn icon(&mut self, name: IconName, rect: Rect, color: Color) {
        self.prims.push(Prim::Icon { name, rect, color });
    }

    pub fn lines(&mut self, segments: Vec<[Point; 2]>, color: Color, width: f32) {
        if !segments.is_empty() {
            self.prims.push(Prim::Lines {
                segments,
                color,
                width,
            });
        }
    }

    pub fn clipped(&mut self, rect: Rect, f: impl FnOnce(&mut Scene)) {
        self.prims.push(Prim::PushClip(rect));
        f(self);
        self.prims.push(Prim::PopClip);
    }

    /// Filled quads only, for tests.
    pub fn quads(&self) -> impl Iterator<Item = (Rect, Color)> + '_ {
        self.prims.iter().filter_map(|p| match p {
            Prim::Quad { rect, fill, .. } => Some((*rect, *fill)),
            _ => None,
        })
    }

    /// Text runs only, for tests.
    pub fn texts(&self) -> impl Iterator<Item = &str> + '_ {
        self.prims.iter().filter_map(|p| match p {
            Prim::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
    }
}

/// Implemented by a frontend with its text system.
pub trait TextMeasure {
    /// Advance width in logical pixels of `text` set in `font` at `size`.
    fn text_width(&mut self, text: &str, font: FontRole, size: f32) -> f32;
}

/// Width measurements keyed by string, role and size. Bounded: it is cleared
/// when it grows past a few thousand entries, which only happens with a
/// stream of unique labels (tick labels while panning), all cheap to remeasure.
#[derive(Default)]
pub struct TextCache {
    /// By font and size bits, then by text, so a hit looks up the borrowed
    /// text without allocating a key.
    map: HashMap<(FontRole, u32), HashMap<Box<str>, f32>>,
    len: usize,
}

const TEXT_CACHE_LIMIT: usize = 8192;

impl TextCache {
    pub fn width(
        &mut self,
        measure: &mut dyn TextMeasure,
        text: &str,
        font: FontRole,
        size: f32,
    ) -> f32 {
        let style = (font, size.to_bits());
        if let Some(w) = self.map.get(&style).and_then(|m| m.get(text)) {
            return *w;
        }
        if self.len >= TEXT_CACHE_LIMIT {
            self.map.clear();
            self.len = 0;
        }
        let w = measure.text_width(text, font, size);
        self.map.entry(style).or_default().insert(text.into(), w);
        self.len += 1;
        w
    }
}

/// Fixed-advance measurement for tests and headless use: every character is
/// `0.6 × size` wide, which is close to a monospace face.
#[derive(Clone, Copy, Debug, Default)]
pub struct MonoMeasure;

impl TextMeasure for MonoMeasure {
    fn text_width(&mut self, text: &str, _font: FontRole, size: f32) -> f32 {
        text.chars().count() as f32 * size * 0.6
    }
}

#[cfg(test)]
mod presence_tests {
    use super::*;

    #[test]
    fn fragmented_output_is_one_mask_and_solid_output_is_bounded_runs() {
        let rect = Rect::from_xywh(0.0, 0.0, 128.0, 32.0);
        let color = Color::rgb(0x123456);
        let mut scene = Scene::default();
        let mut mask = PresenceMask::new(256, 64);
        mask.fill(0..256, 0..64);
        scene.presence(rect, mask, color);
        assert_eq!(scene.prims.len(), 64);
        assert!(scene.quads().all(|(r, c)| r.height() == 0.5 && c == color));
        scene.clear();
        let mut mask = PresenceMask::new(256, 64);
        for y in 0..64 {
            for x in (0..256).step_by(2) {
                mask.fill(x..x + 1, y..y + 1);
            }
        }
        scene.presence(rect, mask, color);
        assert!(
            matches!(scene.prims.as_slice(),[Prim::Presence { rect:r,mask,color:c }] if *r==rect && *c==color && mask.runs().count()==8192)
        );
    }
}
