//! Labels and timestamp anchors for log and located-event waveform tracks.

use super::{LaneCells, truncate_chars};
use crate::document::Document;
use crate::geometry::{Rect, point};
use crate::scene::FontRole;
use crate::wave::lane::TxLane;
use crate::wave::overlay::TextPainter;
use crate::wave::timeline::format_time;

pub(super) fn paint(
    lane: &TxLane,
    generator: &crate::data::loaded_tracks::LoadedGenerator,
    index: &crate::data::points::PointIndex,
    doc: &Document,
    cells: &LaneCells<'_>,
    p: &mut TextPainter<'_>,
) {
    use crate::data::points::PointKind;
    use crate::wave::points;
    let t = p.theme;
    let layout = cells.layout;
    let area = Rect::from_xywh(
        layout.waves.left(),
        layout.row_y(cells.pos),
        layout.waves.width(),
        layout.row_height(cells.pos),
    );
    let groups = points::layout(
        generator.transactions(),
        index,
        cells.viewport,
        area,
        t.zoom,
    );
    let color_of = |group: &points::PointGroup| {
        if index.kind == PointKind::Event {
            t.wave_signal
        } else if group.counts[4] + group.counts[5] > 0 {
            t.editor.error
        } else if group.counts[3] > 0 {
            t.editor.warning
        } else {
            t.editor.icon_accent
        }
    };
    p.scene.clipped(area, |scene| {
        for group in &groups {
            let color = color_of(group);
            scene.lines(
                vec![[
                    point(group.anchor, area.top()),
                    point(group.anchor, group.rect.top()),
                ]],
                color,
                1.0,
            );
            if group.first != group.last {
                scene.lines(
                    vec![[
                        point(group.anchor, group.rect.top() - 2.0),
                        point(group.end_anchor, group.rect.top() - 2.0),
                    ]],
                    color,
                    1.0,
                );
            }
        }
    });
    for group in groups {
        let count = group.records.len();
        let record = &generator.transactions()[group.records.start];
        let selected = doc.selection().is_some_and(|s| {
            Some(s.track) == lane.track()
                && generator
                    .transaction_ordinal(s.id)
                    .is_some_and(|i| group.records.contains(&i))
        });
        let color = color_of(&group);
        let title = if count == 1 {
            index.kind.label(record).map_or_else(
                || format!("#{}", record.id.0),
                |v| crate::data::text::format_attribute(v, 256),
            )
        } else {
            format!(
                "{count} {}",
                if index.kind == PointKind::Log {
                    "messages"
                } else {
                    "events"
                }
            )
        };
        let char_w = p.width("0", FontRole::Mono, t.mono_size).max(1.0);
        let title = truncate_chars(
            &title,
            ((group.rect.width() - 12.0 * t.zoom) / char_w).max(0.0) as usize,
        )
        .unwrap_or_default();
        let time = format_time(group.first as f64, doc.time_base());
        let time = if group.first == group.last {
            time
        } else {
            format!("{time}–{}", format_time(group.last as f64, doc.time_base()))
        };
        let summary = if count > 1 && index.kind == PointKind::Log {
            [
                (5, "F"),
                (4, "E"),
                (3, "W"),
                (2, "I"),
                (1, "D"),
                (0, "T"),
                (6, "?"),
            ]
            .into_iter()
            .filter(|(level, _)| group.counts[*level] > 0)
            .map(|(level, name)| {
                let color = match level {
                    4 | 5 => t.editor.error,
                    3 => t.editor.warning,
                    2 => t.editor.icon_accent,
                    _ => t.editor.text_muted,
                };
                (format!("{name} {}", group.counts[level]), color)
            })
            .collect::<Vec<_>>()
        } else if group.first == group.last {
            vec![("same timestamp".into(), color)]
        } else {
            vec![("time range".into(), color)]
        };
        let mut offset = 6.0 * t.zoom;
        let summary: Vec<_> = summary
            .into_iter()
            .map(|(text, color)| {
                let x = group.rect.left() + offset;
                offset += p.width(&text, FontRole::Mono, t.ui_size_small) + 8.0 * t.zoom;
                (x, text, color)
            })
            .collect();
        let line_h = group.rect.height() / 3.0;
        p.scene.clipped(area, |scene| {
            scene.quad(
                group.rect,
                t.panel.bg,
                3.0 * t.zoom,
                if selected { 2.0 } else { 1.0 },
                if selected { t.border_focused } else { t.border },
            );
            scene.fill(
                Rect::from_xywh(
                    group.rect.left(),
                    group.rect.top(),
                    2.0 * t.zoom,
                    group.rect.height(),
                ),
                color,
            );
            scene.clipped(group.rect, |scene| {
                scene.text(
                    point(group.rect.left() + 6.0 * t.zoom, group.rect.top()),
                    line_h,
                    title,
                    FontRole::Mono,
                    t.mono_size,
                    t.editor.text,
                );
                scene.text(
                    point(group.rect.left() + 6.0 * t.zoom, group.rect.top() + line_h),
                    line_h,
                    time,
                    FontRole::Mono,
                    t.ui_size_small,
                    t.editor.text_muted,
                );
                if count > 1 {
                    for (x, text, color) in summary {
                        scene.text(
                            point(x, group.rect.top() + 2.0 * line_h),
                            line_h,
                            text,
                            FontRole::Mono,
                            t.ui_size_small,
                            color,
                        );
                    }
                }
            });
        });
    }
}
