//! Paint the bounded prepared window.

use super::model::{RowIdentity, TableModel};
use crate::geometry::{Rect, point};
use crate::scene::{FontRole, Scene};
use crate::theme::Theme;

pub fn paint(model: &TableModel, theme: &Theme, scene: &mut Scene) {
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
            label(
                scene,
                Rect::from_xywh(
                    column.rect.left(),
                    column.rect.top(),
                    (column.rect.width() - 22.0 * theme.zoom).max(0.0),
                    column.rect.height(),
                ),
                model.column_title(column.source_index),
                theme.panel.text,
            );
            let enabled = model.can_sort_column(column.source_index);
            let active = model
                .applied_sort()
                .filter(|sort| sort.column == column.source_index);
            let x = column.rect.right() - 14.0 * theme.zoom;
            let y = column.rect.top() + column.rect.height() * 0.5;
            let d = 3.0 * theme.zoom;
            for (descending, offset) in [(false, -4.0), (true, 4.0)] {
                let selected = active.is_some_and(|sort| sort.descending == descending);
                let tip = y + offset * theme.zoom;
                let base = tip + if descending { -d } else { d };
                scene.lines(
                    vec![
                        [point(x - d, base), point(x, tip)],
                        [point(x, tip), point(x + d, base)],
                    ],
                    if !enabled {
                        theme.panel.text_muted.with_alpha(0.45)
                    } else if selected {
                        theme.panel.icon_accent
                    } else {
                        theme.panel.text
                    },
                    if selected {
                        2.0 * theme.zoom
                    } else {
                        theme.zoom
                    },
                );
            }
        }
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
