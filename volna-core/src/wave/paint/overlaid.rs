//! Shared-scale line painting and exact-value probes for numeric groups.

use super::*;
use crate::wave::model::GroupStyle;
use crate::wave::overlaid;

pub(super) struct OverlaidRow {
    plot: Plot,
    clip: Rect,
    entries: Vec<usize>,
    colors: Vec<Color>,
    probe: Option<(Point, u64, Option<usize>)>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_row(
    entries: &[usize],
    colors: &[Color],
    model: &WaveModel,
    doc: &Document,
    cells: &LaneCells<'_>,
    reading: Option<Point>,
    p: &mut TextPainter<'_>,
) -> Option<OverlaidRow> {
    let t = p.theme;
    let z = |v: f32| v * t.zoom;
    let layout = cells.layout;
    let y = layout.row_y(cells.pos);
    let height = layout.row_height(cells.pos);
    let group = layout.entry(cells.pos)?;
    let GroupStyle::Overlaid { draw, .. } = model.items()[group].group()?.style else {
        return None;
    };
    let shown = entries
        .iter()
        .filter(|&&i| !model.signal(i).unwrap().overlay_hidden)
        .count();
    if height >= 2.0 * layout.row_h - 0.5 {
        p.scene.clipped(layout.names, |scene| {
            scene.text(
                point(cells.name_x + z(CHEVRON_W), y + layout.row_h - z(4.0)),
                layout.row_h,
                format!("overlaid · {shown}/{} lines", entries.len()),
                FontRole::Ui,
                t.ui_size_small,
                cells.muted,
            );
        });
    }
    p.scene.clipped(layout.values, |scene| {
        scene.text(
            point(layout.values.left() + z(8.0), y),
            layout.row_h,
            format!("{shown} line{}", if shown == 1 { "" } else { "s" }),
            FontRole::Ui,
            t.ui_size_small,
            cells.muted,
        );
    });
    let wave_row = Rect::from_xywh(layout.waves.left(), y, layout.waves.width(), height);
    let clip = intersect(wave_row, layout.waves);
    let scale = model.overlaid_ranges.get(&group).map(|r| &r.scale)?;
    let plot = analog_plot(scale, wave_row, t)?;
    if plot.width <= 0.0 || plot.bottom <= plot.top {
        return None;
    }
    let reading = reading.filter(|pt| clip.contains(*pt));
    let mut hot = None;
    let mut distance = z(8.0).powi(2);
    let mut geometries = Vec::new();
    let mut waiting = false;
    for (k, &entry) in entries.iter().enumerate() {
        let item = model.signal(entry)?;
        if item.overlay_hidden {
            continue;
        }
        let Some(history) = &item.history else {
            waiting |= item.error.is_none();
            continue;
        };
        let kind = item.numeric_kind()?;
        let series = Series::of(doc, item.source.signal(), history, kind);
        let geometry = analog::geometry(&series, draw, &cells.viewport, &plot, t.zoom);
        waiting |= geometry.waiting;
        if let Some(pt) = reading {
            for pair in geometry.runs.iter().flat_map(|run| run.windows(2)) {
                let (a, b) = (pair[0], pair[1]);
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let length = dx * dx + dy * dy;
                let f = if length == 0.0 {
                    0.0
                } else {
                    ((pt.x - a.x) * dx + (pt.y - a.y) * dy) / length
                }
                .clamp(0.0, 1.0);
                let d = (pt.x - a.x - f * dx).powi(2) + (pt.y - a.y - f * dy).powi(2);
                if d < distance {
                    distance = d;
                    hot = Some(k);
                }
            }
        }
        geometries.push((k, geometry));
    }
    let focused = hot.or_else(|| {
        entries
            .iter()
            .position(|i| model.selected.contains(i) && !model.signal(*i).unwrap().overlay_hidden)
    });
    // Draw the focused member last so coincident lines remain inspectable.
    geometries.sort_by_key(|(k, _)| Some(*k) == focused);
    p.scene.clipped(clip, |scene| {
        for y in [plot.top, plot.bottom] {
            scene.fill(
                Rect::from_xywh(plot.left, snap(y), plot.width, 1.0),
                t.wave_tick,
            );
        }
        if plot.lo < 0.0 && plot.hi > 0.0 {
            scene.lines(
                vec![[
                    point(plot.left, plot.y_of(0.0)),
                    point(plot.left + plot.width, plot.y_of(0.0)),
                ]],
                t.wave_tick_text.with_alpha(0.35),
                1.0,
            );
        }
        for (k, geometry) in geometries {
            let alpha = if focused.is_none() || focused == Some(k) {
                1.0
            } else {
                0.4
            };
            let color = colors[k].with_alpha(alpha);
            let width = if focused == Some(k) {
                z(1.75)
            } else {
                z(t.analog_wave_width).max(1.0)
            };
            scene.lines(overlaid::strokes(&geometry.runs, clip), color, width);
            // Unknowns are member-specific lanes along the bottom, rather
            // than a fill that could cover another member's valid line.
            let unknown_y = plot.bottom - z(2.0) - (k % 4) as f32 * z(3.0);
            let unknowns: Vec<_> = geometry
                .undefined
                .iter()
                .map(|&(a, b)| vec![point(a, unknown_y), point(b.max(a + 1.0), unknown_y)])
                .collect();
            scene.lines(
                overlaid::strokes(&unknowns, clip),
                color.with_alpha(0.55),
                z(1.0),
            );
        }
        let message = if entries.is_empty() {
            Some("No numeric members")
        } else if shown == 0 {
            Some("All overlay lines hidden")
        } else if waiting {
            Some("Summarizing / loading…")
        } else {
            None
        };
        if let Some(message) = message {
            scene.text(
                point(plot.left + z(8.0), y),
                layout.row_h,
                message,
                FontRole::Ui,
                t.ui_size_small,
                cells.muted,
            );
        }
    });
    if height >= 2.0 * layout.row_h - 0.5 {
        for (value, ly, bound) in [
            (plot.hi, plot.top, "max"),
            (plot.lo, plot.bottom - z(14.0), "min"),
        ] {
            let text = format!("{bound} {}", stack::number(value));
            let width = p.width(&text, FontRole::Mono, t.ui_size_small) + z(8.0);
            let chip = Rect::from_xywh(plot.left + plot.width - width - z(4.0), ly, width, z(14.0));
            p.scene.clipped(clip, |scene| {
                scene.quad(
                    chip,
                    t.editor.bg.with_alpha(0.85),
                    z(3.0),
                    0.0,
                    Color::TRANSPARENT,
                );
                scene.text(
                    point(chip.left() + z(4.0), chip.top()),
                    chip.height(),
                    text,
                    FontRole::Mono,
                    t.ui_size_small,
                    cells.muted,
                );
            });
        }
    }
    let probe = reading.map(|pt| {
        let at = cells
            .viewport
            .time_at(f64::from(pt.x - plot.left), f64::from(plot.width))
            .floor()
            .max(0.0) as u64;
        (pt, at, hot)
    });
    Some(OverlaidRow {
        plot,
        clip,
        entries: entries.to_vec(),
        colors: colors.to_vec(),
        probe,
    })
}

pub(super) fn paint_probes(
    rows: &[OverlaidRow],
    model: &WaveModel,
    doc: &Document,
    layout: &WaveLayout,
    vp: &Viewport,
    cursor: Option<u64>,
    p: &mut TextPainter<'_>,
) {
    let t = p.theme;
    let z = |v: f32| v * t.zoom;
    for row in rows {
        // Cursor dots and probe values always use held trace samples, even
        // when the visible stroke uses linear interpolation or envelopes.
        for time in cursor.into_iter().chain(row.probe.map(|(_, at, _)| at)) {
            if (time as f64) < vp.start || (time as f64) > vp.end {
                continue;
            }
            let x = row.plot.left + vp.x_of(time as f64, f64::from(row.plot.width)) as f32;
            for (k, &entry) in row.entries.iter().enumerate() {
                let s = model.signal(entry).unwrap();
                if s.overlay_hidden {
                    continue;
                }
                let Some(h) = &s.history else {
                    continue;
                };
                let sample =
                    analog::sample(h.as_ref(), s.numeric_kind().unwrap(), h.index_at(time));
                let analog::Sample::Value(value) = sample else {
                    continue;
                };
                let y = row.plot.y_of(value);
                let r = z(2.75);
                p.scene.clipped(row.clip, |scene| {
                    scene.quad(
                        Rect::from_xywh(x - r, y - r, r * 2.0, r * 2.0),
                        t.editor.bg,
                        r,
                        z(1.25),
                        row.colors[k],
                    );
                });
            }
        }
        let Some((pointer, time, hot)) = row.probe else {
            continue;
        };
        let lines: Vec<_> = row
            .entries
            .iter()
            .take(STACK_READOUT_LAYERS)
            .enumerate()
            .map(|(k, &entry)| {
                let s = model.signal(entry).unwrap();
                let value = s
                    .history
                    .as_ref()
                    .map(|h| s.translator.translate(&h.value(h.index_at(time))).text)
                    .unwrap_or_else(|| {
                        if s.error.is_some() {
                            "load failed"
                        } else {
                            "loading…"
                        }
                        .into()
                    });
                (
                    k,
                    format!(
                        "{}{}",
                        s.name,
                        if s.overlay_hidden { " (hidden)" } else { "" }
                    ),
                    value,
                )
            })
            .collect();
        let footer = format!(
            "{} · exact held values",
            format_time(time as f64, doc.time_base())
        );
        let name_w = lines
            .iter()
            .map(|(_, name, _)| p.width(name, FontRole::Mono, t.mono_size))
            .fold(0.0f32, f32::max);
        let value_w = lines
            .iter()
            .map(|(_, _, value)| p.width(value, FontRole::Mono, t.mono_size))
            .fold(0.0f32, f32::max);
        let omitted = row.entries.len().saturating_sub(lines.len());
        let line_h = z(18.0);
        let width = (z(SWATCH_W + 28.0) + name_w + value_w)
            .max(p.width(&footer, FontRole::Ui, t.ui_size_small) + z(14.0));
        let height = line_h * (lines.len() + 1 + usize::from(omitted > 0)) as f32 + z(8.0);
        let waves = layout.waves;
        let x = (pointer.x + z(14.0))
            .min(waves.right() - width - z(4.0))
            .max(waves.left());
        let y = (pointer.y + z(14.0))
            .min(waves.bottom() - height - z(4.0))
            .max(waves.top());
        let chip = Rect::from_xywh(snap(x), snap(y), width, height);
        p.scene.clipped(waves, |scene| {
            scene.quad(chip, t.tooltip.bg, z(4.0), 1.0, t.border);
            let left = chip.left() + z(7.0);
            let mut ly = chip.top() + z(4.0);
            for (k, name, value) in lines {
                if hot == Some(k) {
                    scene.fill(
                        Rect::from_xywh(chip.left() + 1.0, ly, width - 2.0, line_h),
                        t.hover.bg,
                    );
                }
                paint_member_swatch(scene, point(left, ly), line_h, row.colors[k], t);
                scene.text(
                    point(left + z(SWATCH_W), ly),
                    line_h,
                    name,
                    FontRole::Mono,
                    t.mono_size,
                    if hot == Some(k) {
                        t.tooltip.text
                    } else {
                        t.editor.text_placeholder
                    },
                );
                scene.text(
                    point(left + z(SWATCH_W + 14.0) + name_w, ly),
                    line_h,
                    value,
                    FontRole::Mono,
                    t.mono_size,
                    t.tooltip.text,
                );
                ly += line_h;
            }
            if omitted > 0 {
                scene.text(
                    point(left, ly),
                    line_h,
                    format!("+{omitted} more lines"),
                    FontRole::Ui,
                    t.ui_size_small,
                    t.editor.text_placeholder,
                );
                ly += line_h;
            }
            scene.text(
                point(left, ly),
                line_h,
                footer,
                FontRole::Ui,
                t.ui_size_small,
                t.editor.text_placeholder,
            );
        });
    }
}
