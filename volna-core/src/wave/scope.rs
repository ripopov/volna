//! Contiguous scope bands and their shared presentation for waveform rows and rulers.

use super::{
    layout::MIN_COLUMN,
    overlay::{TextPainter, fit_scope},
};
use crate::{
    color::Color,
    geometry::{Point, Rect, point},
    scene::FontRole,
    theme::Theme,
    trace::TraceId,
};

pub(super) struct Scope<'a> {
    trace: TraceId,
    id: Option<volna_trace::data::ScopeId>,
    path: &'a str,
}

impl<'a> Scope<'a> {
    pub(super) fn new(
        trace: TraceId,
        id: Option<volna_trace::data::ScopeId>,
        path: &'a str,
    ) -> Self {
        Self { trace, id, path }
    }
}

pub(super) struct Band<'a> {
    scope: Scope<'a>,
    top: f32,
    bottom: f32,
}

pub(super) fn bands<'a>(
    rows: impl IntoIterator<Item = (Option<Scope<'a>>, f32, f32)>,
) -> Vec<Band<'a>> {
    let mut bands: Vec<Band<'a>> = Vec::new();
    let mut adjacent = false;
    for (scope, top, bottom) in rows {
        let Some(scope) = scope.filter(|scope| !scope.path.is_empty()) else {
            adjacent = false;
            continue;
        };
        if adjacent
            && let Some(last) = bands.last_mut()
            && scope.id.is_some()
            && last.scope.id == scope.id
            && last.scope.trace == scope.trace
        {
            last.bottom = bottom;
        } else {
            bands.push(Band { scope, top, bottom });
        }
        adjacent = true;
    }
    bands
}

pub(super) fn paint(
    p: &mut TextPainter<'_>,
    bands: &[Band<'_>],
    area: Rect,
    pointer: Option<Point>,
    row_h: f32,
) {
    let theme = p.theme;
    let z = |value: f32| value * theme.zoom;
    if area.width() <= z(4.0) {
        return;
    }
    for band in bands {
        let rect = Rect::from_xywh(
            area.left() + z(2.0),
            band.top + z(2.0),
            area.width() - z(4.0),
            (band.bottom - band.top - z(4.0)).max(0.0),
        );
        let hovered = pointer.is_some_and(|point| {
            area.contains(point) && point.y >= band.top && point.y < band.bottom
        });
        let color = color(band.scope.path, band.scope.trace, theme);
        let edge = Rect::from_xywh(
            rect.left(),
            rect.top(),
            z(2.0).min(rect.width()),
            rect.height(),
        );
        p.scene.clipped(area, |scene| {
            scene.quad(
                rect,
                color.with_alpha(if hovered { 0.12 } else { 0.055 }),
                z(3.0),
                if theme.appearance.is_high_contrast() {
                    z(1.0)
                } else {
                    0.0
                },
                theme.panel.text_muted,
            );
            scene.quad(
                edge,
                color.with_alpha(if hovered { 1.0 } else { 0.65 }),
                z(1.0),
                0.0,
                Color::TRANSPARENT,
            );
        });
        if area.width() < z(MIN_COLUMN) {
            continue;
        }
        let visible_top = band.top.max(area.top());
        let visible_bottom = band.bottom.min(area.bottom());
        let parent = band.scope.path.rsplit_once('.');
        let detailed = parent.is_some() && visible_bottom - visible_top >= 2.0 * row_h;
        let lines = if detailed { 2.0 } else { 1.0 };
        let top = visible_top.min((band.bottom - lines * row_h).max(band.top));
        let max_width = area.width() - z(24.0);
        let (primary, secondary) = if detailed {
            let (parent, name) = parent.unwrap();
            (name, Some(parent))
        } else {
            (band.scope.path, None)
        };
        let clip = Rect::from_xywh(
            area.left(),
            visible_top,
            area.width(),
            (visible_bottom - visible_top).max(0.0),
        );
        if let Some(label) = fit_scope(p, primary, max_width) {
            let width = p.width(&label, FontRole::Ui, theme.ui_size);
            p.scene.clipped(clip, |scene| {
                scene.text(
                    point(area.right() - z(8.0) - width, top),
                    row_h,
                    label,
                    FontRole::Ui,
                    theme.ui_size,
                    if hovered {
                        theme.panel.text
                    } else {
                        theme.panel.text_muted
                    },
                );
            });
        }
        if let Some(parent) = secondary
            && let Some(label) = fit_scope(p, parent, max_width)
        {
            let width = p.width(&label, FontRole::Ui, theme.ui_size_small);
            p.scene.clipped(clip, |scene| {
                scene.text(
                    point(area.right() - z(8.0) - width, top + row_h),
                    row_h,
                    label,
                    FontRole::Ui,
                    theme.ui_size_small,
                    theme.panel.text_placeholder,
                );
            });
        }
    }
}

fn color(scope: &str, trace: TraceId, theme: &Theme) -> Color {
    let hash = scope
        .bytes()
        .chain([trace.index() as u8])
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        });
    let index = (hash % 10) as usize;
    if index < 4 {
        theme.wave_tints[index]
    } else {
        theme.markers[index - 4].stroke
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping_requires_adjacent_rows_with_the_same_scope_and_trace() {
        let a = TraceId::A;
        let b = TraceId::new(1).unwrap();
        let bands = bands([
            (Some(Scope::new(a, Some(0), "top.cpu")), 0.0, 24.0),
            (Some(Scope::new(a, Some(0), "top.cpu")), 24.0, 120.0),
            (None, 120.0, 144.0),
            (Some(Scope::new(a, Some(0), "top.cpu")), 144.0, 168.0),
            (Some(Scope::new(a, Some(0), "")), 168.0, 192.0),
            (Some(Scope::new(a, Some(0), "top.cpu")), 192.0, 216.0),
            (Some(Scope::new(b, Some(0), "top.cpu")), 216.0, 240.0),
            (Some(Scope::new(a, Some(1), "top.dma")), 240.0, 264.0),
            (Some(Scope::new(a, Some(0), "top.cpu")), 264.0, 288.0),
        ]);
        assert_eq!(bands.len(), 6);
        assert_eq!((bands[0].top, bands[0].bottom), (0.0, 120.0));
        assert_eq!(
            bands
                .iter()
                .map(|band| band.scope.trace)
                .collect::<Vec<_>>(),
            [a, a, a, b, a, a]
        );
        assert_eq!(bands[5].scope.path, "top.cpu");
    }

    #[test]
    fn ambiguous_scope_paths_are_drawn_but_never_merged() {
        let bands = bands([
            (Some(Scope::new(TraceId::A, Some(0), "top.cpu")), 0.0, 24.0),
            (Some(Scope::new(TraceId::A, Some(1), "top.cpu")), 24.0, 48.0),
            (Some(Scope::new(TraceId::A, None, "top.cpu")), 48.0, 72.0),
            (Some(Scope::new(TraceId::A, None, "top.cpu")), 72.0, 96.0),
        ]);
        assert_eq!(bands.len(), 4);
    }
}
