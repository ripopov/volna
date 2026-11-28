//! Paint the bounded prepared window.

use super::format::{BitState, CellBitmask, MAX_BITMASK_BITS};
use super::layout::CELL_PADDING;
use super::model::{RowIdentity, TableModel};
use crate::color::Color;
use crate::geometry::{Rect, point};
use crate::icons::IconName;
use crate::scene::{FontRole, Scene, TextCache, TextMeasure};
use crate::theme::Theme;

const BIT_WIDTH: f32 = 3.0;
const BIT_HEIGHT: f32 = 12.0;
const BIT_GAP: f32 = 1.0;
const NIBBLE_EXTRA_GAP: f32 = 2.0;
const BYTE_EXTRA_GAP: f32 = 4.0;
const BIT_RADIUS: f32 = 1.0;
const ELLIPSIS_GAP: f32 = 2.0;

/// Pixel width of a rendered bitmask strip (excluding cell horizontal padding).
#[must_use]
pub(super) fn bitmask_width(shown: usize, total_width: usize, zoom: f32) -> f32 {
    if shown == 0 {
        return 0.0;
    }
    let mut width = shown as f32 * BIT_WIDTH * zoom;
    for i in 0..shown.saturating_sub(1) {
        width += bit_gap(i, total_width, zoom);
    }
    width
}

/// Content width needed to auto-fit a bitmask column of `total_width` bits.
#[must_use]
pub(super) fn bitmask_fit_width(
    total_width: usize,
    theme: &Theme,
    measure: &mut dyn TextMeasure,
) -> f32 {
    let shown = total_width.min(MAX_BITMASK_BITS);
    let mut width = bitmask_width(shown, total_width, theme.zoom);
    if total_width > shown {
        width +=
            ELLIPSIS_GAP * theme.zoom + measure.text_width("…", FontRole::Mono, theme.mono_size);
    }
    width
}

fn bit_gap(index: usize, total_width: usize, zoom: f32) -> f32 {
    let bit_index = total_width.saturating_sub(1).saturating_sub(index);
    let extra = if bit_index.is_multiple_of(8) {
        BYTE_EXTRA_GAP
    } else if bit_index.is_multiple_of(4) {
        NIBBLE_EXTRA_GAP
    } else {
        0.0
    };
    (BIT_GAP + extra) * zoom
}

fn bit_color(state: BitState, theme: &Theme) -> Color {
    match state {
        BitState::One => theme.wave_signal,
        BitState::Zero => theme.editor.text.with_alpha(0.14),
        BitState::Undef => theme.wave_undef,
        BitState::HighImp => theme.wave_highimp,
        BitState::DontCare => theme.wave_dontcare,
        BitState::Weak => theme.wave_weak,
    }
}

fn paint_bitmask(scene: &mut Scene, rect: Rect, bitmask: &CellBitmask, pad: f32, theme: &Theme) {
    let z = theme.zoom;
    let bit_w = BIT_WIDTH * z;
    let bit_h = (BIT_HEIGHT * z).min((rect.height() - 4.0 * z).max(2.0 * z));
    let bit_y = rect.top() + (rect.height() - bit_h) * 0.5;
    let radius = BIT_RADIUS * z;
    let shown = bitmask.bits.len();

    scene.clipped(rect, |scene| {
        let mut x = rect.left() + pad;
        for (i, &state) in bitmask.bits.iter().enumerate() {
            if x >= rect.right() {
                break;
            }
            scene.quad(
                Rect::from_xywh(x, bit_y, bit_w, bit_h),
                bit_color(state, theme),
                radius,
                0.0,
                Color::TRANSPARENT,
            );
            x += bit_w;
            if i + 1 < shown {
                x += bit_gap(i, bitmask.width, z);
            }
        }
        if bitmask.width > shown && x < rect.right() {
            scene.text(
                point(x + ELLIPSIS_GAP * z, rect.top()),
                rect.height(),
                "…",
                FontRole::Mono,
                theme.mono_size,
                theme.editor.text_muted,
            );
        }
    });
}

pub fn paint(
    model: &TableModel,
    theme: &Theme,
    cache: &mut TextCache,
    measure: &mut dyn TextMeasure,
    scene: &mut Scene,
) {
    let layout = &model.layout;
    scene.fill(layout.bounds, theme.editor.bg);
    scene.fill(layout.header, theme.panel.bg);
    scene.fill(layout.filter_row, theme.panel.bg);
    scene.fill(layout.gutter, theme.panel.bg);
    scene.fill(layout.status, theme.panel.bg);
    for (thumb, hovered) in [
        (layout.vertical_thumb, model.vertical_scrollbar_hovered()),
        (
            layout.horizontal_thumb,
            model.horizontal_scrollbar_hovered(),
        ),
    ] {
        if !thumb.is_empty() {
            scene.quad(
                thumb,
                if hovered {
                    theme.scrollbar_thumb_hover
                } else {
                    theme.scrollbar_thumb
                },
                3.0 * theme.zoom,
                0.0,
                Color::TRANSPARENT,
            );
        }
    }

    let pad = CELL_PADDING * theme.zoom;
    let digit_width = cache.width(measure, "0", FontRole::Mono, theme.mono_size);
    let label = |scene: &mut Scene, rect: Rect, text: &str, color| {
        scene.clipped(rect, |scene| {
            scene.text(
                point(rect.left() + pad, rect.top()),
                rect.height(),
                text,
                FontRole::Mono,
                theme.mono_size,
                color,
            );
        });
    };

    scene.clipped(layout.header, |scene| {
        for column in &layout.columns {
            let header_title = model.column_header_title(column.source_index);
            let title = fit_title(
                &header_title,
                column.header.title.width(),
                theme,
                cache,
                measure,
            );
            scene.clipped(column.header.title, |scene| {
                scene.text(
                    column.header.title.origin,
                    column.header.title.height(),
                    title,
                    FontRole::Mono,
                    theme.mono_size,
                    theme.panel.text,
                );
            });
            let enabled = model.can_sort_column(column.source_index);
            let active = model
                .applied_sort()
                .filter(|sort| sort.column == column.source_index);
            let (sort_bg, sort_border) = if !enabled {
                (Color::TRANSPARENT, theme.border.with_alpha(0.4))
            } else if active.is_some() {
                (theme.selection.bg.with_alpha(0.35), theme.panel.icon_accent)
            } else {
                (theme.editor.bg.with_alpha(0.5), theme.border)
            };
            scene.quad(
                column.header.sort_target,
                sort_bg,
                4.0 * theme.zoom,
                theme.zoom,
                sort_border,
            );
            scene.icon(
                active.map_or(IconName::ArrowUpDown, |sort| {
                    if sort.descending {
                        IconName::ArrowDown
                    } else {
                        IconName::ArrowUp
                    }
                }),
                column.header.sort_icon,
                if !enabled {
                    theme.panel.text_muted.with_alpha(0.45)
                } else if active.is_some() {
                    theme.panel.icon_accent
                } else {
                    theme.panel.text_muted
                },
            );
        }
        scene.fill(
            Rect::from_xywh(
                layout.header.left(),
                layout.header.bottom() - theme.zoom,
                layout.header.width(),
                theme.zoom,
            ),
            theme.border,
        );
    });

    let active = model.active.as_ref();
    scene.clipped(layout.gutter, |scene| {
        for row in &model.window.rows {
            let y = layout.body.top() + model.viewport.y_of(row.ordinal, layout.row_height);
            let text = row.ordinal.saturating_add(1).to_string();
            scene.text(
                point(
                    layout.gutter.right() - pad - text.len() as f32 * digit_width,
                    y,
                ),
                layout.row_height,
                text,
                FontRole::Mono,
                theme.mono_size,
                theme.panel.text_muted,
            );
        }
    });
    scene.clipped(layout.body, |scene| {
        for row in &model.window.rows {
            let y = layout.body.top() + model.viewport.y_of(row.ordinal, layout.row_height);
            let row_rect = Rect::from_xywh(
                layout.body.left(),
                y,
                layout.body.width(),
                layout.row_height,
            );
            let selected = model.row_selected(row.ordinal);
            if selected {
                scene.fill(row_rect, theme.selection.bg);
            }
            if active == Some(&row.identity) {
                scene.quad(
                    row_rect,
                    Color::TRANSPARENT,
                    0.0,
                    theme.zoom,
                    theme.border_focused,
                );
            }
            for column in &layout.columns {
                let Some(cell) = row.cells.get(column.prepared_index) else {
                    continue;
                };
                let rect = Rect::from_xywh(
                    column.rect.left(),
                    y,
                    column.rect.width(),
                    layout.row_height,
                );
                if let Some(bitmask) = &cell.bitmask
                    && !cell.missing
                {
                    paint_bitmask(scene, rect, bitmask, pad, theme);
                } else {
                    let color = if cell.missing {
                        theme.editor.text_muted
                    } else if let Some(status) = cell.status {
                        match status {
                            volna_trace::data::transactions::Status::Ok => theme.wave_signal,
                            volna_trace::data::transactions::Status::Error => theme.editor.error,
                            volna_trace::data::transactions::Status::Aborted => theme.wave_weak,
                            volna_trace::data::transactions::Status::Open => {
                                theme.panel.icon_accent
                            }
                            volna_trace::data::transactions::Status::Unset => {
                                theme.editor.text_muted
                            }
                        }
                    } else if cell.changed || selected {
                        theme.selection.text
                    } else {
                        theme.editor.text
                    };
                    label(scene, rect, &cell.text, color);
                }
            }
        }
    });

    let selected_text = match active {
        Some(RowIdentity::Transaction(id)) => format!(" · active ID {}", id.0),
        Some(RowIdentity::SignalTime(time)) => format!(" · active @{time}"),
        None => String::new(),
    };
    label(
        scene,
        layout.status,
        &format!(
            "{} selected · {}{}{}",
            model.selection_count(),
            model.status(),
            if model.applied_filter_count() > 0 {
                " (visible rows copied)"
            } else {
                ""
            },
            selected_text
        ),
        theme.panel.text_muted,
    );
}

fn fit_title(
    text: &str,
    width: f32,
    theme: &Theme,
    cache: &mut TextCache,
    measure: &mut dyn TextMeasure,
) -> String {
    if cache.width(measure, text, FontRole::Mono, theme.mono_size) <= width {
        return text.into();
    }
    if cache.width(measure, "…", FontRole::Mono, theme.mono_size) > width {
        return String::new();
    }
    let cuts: Vec<_> = text.char_indices().map(|(i, _)| i).collect();
    let end = cuts.partition_point(|&cut| {
        cache.width(
            measure,
            &format!("{}…", &text[..cut]),
            FontRole::Mono,
            theme.mono_size,
        ) <= width
    });
    format!("{}…", &text[..cuts[end.saturating_sub(1)]])
}
