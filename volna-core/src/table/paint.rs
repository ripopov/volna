//! Paint the bounded prepared window.

use super::model::{RowIdentity, TableModel};
use crate::geometry::{Rect, point};
use crate::icons::IconName;
use crate::scene::{FontRole, Scene, TextCache, TextMeasure};
use crate::theme::Theme;

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
    scene.fill(layout.gutter, theme.panel.bg);
    scene.fill(layout.status, theme.panel.bg);
    scene.fill(layout.vertical_bar, theme.panel.bg);
    scene.fill(layout.horizontal_bar, theme.panel.bg);
    scene.fill(layout.vertical_thumb, theme.panel.text_muted);
    scene.fill(layout.horizontal_thumb, theme.panel.text_muted);

    let pad = 7.0 * theme.zoom;
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
            let title = fit_title(
                model.column_title(column.source_index),
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

    let selected = model.selected.as_ref();
    scene.clipped(layout.gutter, |scene| {
        for row in &model.window.rows {
            let y = layout.body.top() + model.viewport.y_of(row.ordinal, layout.row_height);
            label(
                scene,
                Rect::from_xywh(
                    layout.gutter.left(),
                    y,
                    layout.gutter.width(),
                    layout.row_height,
                ),
                &row.ordinal.saturating_add(1).to_string(),
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
            if selected == Some(&row.identity) {
                scene.fill(row_rect, theme.selection.bg);
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
                let color = if cell.missing {
                    theme.editor.text_muted
                } else if cell.changed || selected == Some(&row.identity) {
                    theme.selection.text
                } else {
                    theme.editor.text
                };
                label(scene, rect, &cell.text, color);
            }
        }
    });

    let selected_text = match selected {
        Some(RowIdentity::Transaction(id)) => format!(" · selected ID {}", id.0),
        Some(RowIdentity::SignalTime(time)) => format!(" · selected @{time}"),
        None => String::new(),
    };
    label(
        scene,
        layout.status,
        &format!("{}{}", model.status(), selected_text),
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
