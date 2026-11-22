//! Paint the pipeline panel into a [`crate::scene::Scene`]: the label column, one row per transaction
//! with a cell per stage on the primary lane and a band per overlay stage, plus the shared
//! timeline header, cursor and markers.

use crate::color::Color;
use crate::document::Document;
use crate::geometry::{CursorIcon, Point, Rect, point, size, snap};
use crate::icons::IconName;
use crate::scene::FontRole;
use crate::theme::contrast;
use crate::wave::marks;
use crate::wave::overlay::{self, TextPainter, TimeColumn};
use volna_trace::data::transactions::Status;

use super::model::{Drag, Hit, PipelineModel, Rows};
use super::stages::{
    HIDDEN_STAGE_EDGE_ALPHA, HIDDEN_STAGE_FILL_ALPHA, HIDDEN_STAGE_HATCH_ALPHA,
    HIDDEN_STAGE_OVERVIEW_ALPHA,
};

const STAGE_TICK_ALPHA: f32 = 0.13;
const EDGE_MIN_PX: f32 = 6.0;
const TEXT_MIN_PX: f32 = 10.0;
const LABEL_MIN_PX: f32 = 7.0;
const DASH_PX: f32 = 3.0;
const HOLLOW_HATCH_PX: f32 = 3.0;
const CELL_TEXT_PAD_PX: f32 = 4.0;

fn fitting_stage_label(
    p: &mut TextPainter<'_>,
    name: &str,
    font_size: f32,
    available: f32,
) -> Option<(String, f32)> {
    if available <= 0.0 {
        return None;
    }
    let first_end = name.chars().next()?.len_utf8();
    let first_width = p.width(&name[..first_end], FontRole::Mono, font_size);
    if first_width > available {
        return None;
    }
    if first_end == name.len() {
        return Some((name.to_owned(), first_width));
    }
    let full_width = p.width(name, FontRole::Mono, font_size);
    if full_width <= available {
        return Some((name.to_owned(), full_width));
    }
    let mut fit = (name[..first_end].to_owned(), first_width);
    for (start, ch) in name.char_indices().skip(1) {
        let prefix = &name[..start + ch.len_utf8()];
        let width = p.width(prefix, FontRole::Mono, font_size);
        if width > available {
            break;
        }
        fit = (prefix.to_owned(), width);
    }
    Some(fit)
}

fn arrow_head(from: Point, to: Point, size: f32) -> [[Point; 2]; 2] {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let length = dx.hypot(dy).max(f32::EPSILON);
    let (ux, uy) = (dx / length, dy / length);
    let base = point(to.x - ux * size, to.y - uy * size);
    let half = size * 0.45;
    [
        [to, point(base.x - uy * half, base.y + ux * half)],
        [to, point(base.x + uy * half, base.y - ux * half)],
    ]
}

fn stage_ticks(scene: &mut crate::scene::Scene, rect: Rect, xs: &[f32], ink: Color) {
    let first = xs.partition_point(|&x| x < rect.left() + 1.0);
    for &x in xs[first..].iter().take_while(|&&x| x + 1.0 < rect.right()) {
        scene.fill(Rect::from_xywh(x, rect.top(), 1.0, rect.height()), ink);
    }
}

fn row_grids(
    model: &PipelineModel,
    clock_ticks: &[Option<Vec<f32>>],
    set: &super::model::RowSet<'_>,
    p: &mut TextPainter<'_>,
    column: &TimeColumn,
    ticks: &[crate::wave::timeline::Tick],
) {
    let layout = model.last_layout();
    let range = layout.row_range.clone();
    if range.is_empty() {
        return;
    }
    let cells = column.area;
    let top = layout.row_y(range.start).max(cells.top());
    let bottom = layout.row_y(range.end).min(cells.bottom());
    let mut paint_band = |top: f32, bottom: f32, source: usize| {
        let band = TimeColumn {
            area: Rect::from_xywh(cells.left(), top, cells.width(), bottom - top),
            ..*column
        };
        if let Some(xs) = &clock_ticks[source] {
            overlay::cycle_grid_lines(p, band.area, xs);
        } else {
            overlay::grid(p, &band, ticks);
        }
    };
    if clock_ticks.len() == 1 {
        paint_band(top, bottom, 0);
        return;
    }

    let mut band = None;
    let mut visit = |row, y| {
        if let Some((source, _)) = set.get(row) {
            match band {
                Some((previous, _)) if previous == source => return,
                Some((previous, start)) => paint_band(start, y, previous),
                None => {}
            }
            band = Some((source, y));
        }
    };
    let pixel = (1.0 / layout.device_scale) as f32;
    if layout.rows.row_px < pixel {
        // Several transactions share one screen pixel. Sample its center to bound
        // overview painting by the viewport height.
        for i in 0..((bottom - top) / pixel).ceil() as usize {
            let y = top + i as f32 * pixel;
            if let Some(row) = layout.row_at(y.midpoint((y + pixel).min(bottom))) {
                visit(row, y);
            }
        }
    } else {
        for row in range {
            visit(row, layout.row_y(row).max(top));
        }
    }
    if let Some((source, start)) = band {
        paint_band(start, bottom, source);
    }
}

pub fn paint(
    model: &PipelineModel,
    doc: &Document,
    mut p: TextPainter<'_>,
    focused: bool,
    marker_tooltip_visible: bool,
) {
    let layout = model.last_layout();
    let bounds = layout.bounds;
    let t = p.theme;
    let z = |v: f32| v * t.zoom;
    let viewport = model.nav.viewport(doc);
    let cursor = model.nav.cursor(doc);
    let base = doc.time_base();
    let cells = layout.cells;
    let labels = layout.labels;
    let header = layout.header;
    let column = TimeColumn {
        header: Rect::new(
            point(cells.left(), header.top()),
            size(cells.width(), header.height()),
        ),
        rulers: Rect::new(
            point(cells.left(), layout.rulers.top()),
            size(cells.width(), layout.rulers.height()),
        ),
        lane: Rect::new(
            point(cells.left(), layout.marker_lane.strip().top()),
            size(cells.width(), layout.marker_lane.strip().height()),
        ),
        area: cells,
        viewport,
    };
    let clocks = model.nav.clocks();

    p.scene.fill(bounds, t.editor.bg);
    p.scene.fill(labels, t.panel.bg);
    p.scene.fill(header, t.panel.bg);
    let (tick_list, unit) = column.ticks(base, t.zoom);
    let rows = model.rows(doc);
    let clock_ticks: Vec<_> = model
        .sources
        .iter()
        .map(|source| {
            source
                .track
                .track()
                .and_then(|track| doc.clocks.linked(track))
                .map(|clock| overlay::cycle_positions(&column, clock, t.zoom))
        })
        .collect();
    if let Rows::Ready(set) = &rows
        && doc.is_loaded()
    {
        row_grids(model, &clock_ticks, set, &mut p, &column, &tick_list);
        let inside = overlay::outside_time(&mut p, &column, doc.limits());
        let first = layout.row_y(0).clamp(cells.top(), cells.bottom());
        let last = layout.row_y(set.len()).clamp(cells.top(), cells.bottom());
        let width = inside.end - inside.start;
        if width > 0.0 {
            for (top, bottom) in [(cells.top(), first), (last, cells.bottom())] {
                if bottom > top {
                    p.scene.fill(
                        Rect::new(point(inside.start, top), size(width, bottom - top)),
                        t.wave_outside,
                    );
                }
            }
        }
    }

    let hover_row = match model.hover {
        Some(Hit::Row(row) | Hit::Cell { row, .. } | Hit::SourceBadge { row, .. }) => Some(row),
        _ => None,
    };
    let hover_badge_row = match model.hover {
        Some(Hit::SourceBadge { row, .. }) => Some(row),
        _ => None,
    };
    let selected_row = model.selected_row(doc);
    match &rows {
        Rows::Ready(set) if !set.is_empty() => {
            let row_px = layout.rows.row_px;
            let pad = super::stage_layout::row_padding(row_px, t.zoom);
            let gap = 2.0 * pad;
            let wave_wf = column.width_f64();
            let x_of = |time: u64| cells.left() + viewport.x_of_tick(time, wave_wf) as f32;
            let map =
                super::raster::PixelMap::new(cells, viewport, layout.rows, layout.device_scale);
            if layout.is_overview() {
                if let Some(filter) = model.stage_anchor_filter() {
                    let ghost = set.presence(&map);
                    let visible = set.visible_presence(&map, filter);
                    let ghost_color = t
                        .editor
                        .text
                        .with_alpha(t.editor.text.a * HIDDEN_STAGE_OVERVIEW_ALPHA);
                    p.scene.clipped(cells, |scene| {
                        scene.presence(map.rect, ghost, ghost_color);
                        scene.presence(map.rect, visible, t.editor.text);
                    });
                } else {
                    let mask = set.presence(&map);
                    p.scene
                        .clipped(cells, |scene| scene.presence(map.rect, mask, t.editor.text));
                }
            } else {
                let border = row_px >= EDGE_MIN_PX;
                let palette = model.palette();
                let primary = palette.primary_lane();
                let merged = model.is_merged();
                let stage_ordered = model.order.is_stage_based();
                let band_base = t.editor.text.with_alpha(0.28);
                let band_edge_base = t.editor.text.with_alpha(0.5);
                let mut cell_texts = Vec::new();
                let mut open_edges = Vec::new();
                for r in layout.row_range.clone() {
                    let Some((source_index, tx)) = set.get(r) else {
                        continue;
                    };
                    let y = layout.row_y(r);
                    let row_rect = Rect::new(point(cells.left(), y), size(cells.width(), row_px));
                    let dimmed = model.is_source_dimmed(source_index);
                    let dim_alpha = if dimmed { 0.18 } else { 1.0 };
                    let ticks = clock_ticks[source_index].as_deref().unwrap_or_default();
                    let band = band_base.with_alpha(band_base.a * dim_alpha);
                    let band_edge = band_edge_base.with_alpha(band_edge_base.a * dim_alpha);
                    let sort_stage_idx = if stage_ordered {
                        set.sort_meta(r)
                            .filter(|m| m.matched)
                            .and_then(super::order::RowSortMeta::sort_stage_index)
                    } else {
                        None
                    };
                    let mut sort_tick: Option<Rect> = None;
                    if tx.status == Status::Aborted {
                        p.scene.clipped(cells, |scene| {
                            scene.fill(
                                row_rect,
                                t.wave_flush.with_alpha(t.wave_flush.a * dim_alpha),
                            );
                        });
                    }
                    if hover_row == Some(r) {
                        p.scene
                            .clipped(cells, |scene| scene.fill(row_rect, t.wave_row_hover));
                    }
                    if tx.stages.is_empty()
                        && ((tx.end as f64) < viewport.start || (tx.begin as f64) > viewport.end)
                    {
                        continue;
                    }
                    let mut last_right: Option<f32> = None;
                    let bands = super::stage_layout::stage_bands(tx, primary);
                    for (stage_idx, (stage, stage_band)) in tx.stages.iter().zip(bands).enumerate()
                    {
                        let sb = stage.begin;
                        let se = PipelineModel::stage_end(tx, stage);
                        let visible = if sb == se {
                            (sb as f64) >= viewport.start && (sb as f64) <= viewport.end
                        } else {
                            (se as f64) > viewport.start && (sb as f64) < viewport.end
                        };
                        if !visible {
                            continue;
                        }
                        let mut xa = x_of(sb) + gap / 2.0;
                        let mut xb = x_of(se) - gap / 2.0;
                        if xb - xa < 1.0 {
                            xa = xa.min(xb);
                            xb = xa + 1.0;
                        }
                        if stage.lane != primary {
                            let band_y = super::stage_layout::overlay_band(y, row_px, t.zoom);
                            let ya = band_y.start;
                            let rect = Rect::from_xywh(xa, ya, xb - xa, band_y.end - ya);
                            p.scene.clipped(cells, |scene| {
                                scene.fill(rect, band);
                                stage_ticks(
                                    scene,
                                    rect,
                                    ticks,
                                    t.editor.text.with_alpha(STAGE_TICK_ALPHA * dim_alpha),
                                );
                                if border {
                                    scene.fill(Rect::from_xywh(xa, ya, xb - xa, 1.0), band_edge);
                                }
                            });
                            if sort_stage_idx == Some(stage_idx) && !dimmed && border {
                                let tick_w = z(2.0).max(1.5);
                                sort_tick = Some(Rect::from_xywh(
                                    snap(xa) - tick_w * 0.5,
                                    (ya - 1.0).max(y),
                                    tick_w,
                                    (rect.height() + 2.0).min(row_px),
                                ));
                            }
                            continue;
                        }
                        let style = palette.style(&stage.name, t);
                        let stage_hidden = model.stage_visibility.is_hidden(&stage.name);
                        let stage_fill_alpha = if stage_hidden {
                            HIDDEN_STAGE_FILL_ALPHA * dim_alpha
                        } else {
                            dim_alpha
                        };
                        let stage_edge_alpha = if stage_hidden {
                            HIDDEN_STAGE_EDGE_ALPHA * dim_alpha
                        } else {
                            dim_alpha
                        };
                        let band_height = row_px / stage_band.count as f32;
                        let band_y = y + band_height * stage_band.index as f32;
                        let band_pad = if stage_band.count == 1 {
                            pad
                        } else {
                            pad.min(band_height * 0.15)
                        };
                        let font_ratio = if stage_band.count == 1 { 0.62 } else { 0.72 };
                        let cell_font = (band_height * font_ratio).clamp(z(6.0), t.mono_size);
                        let rect = Rect::from_xywh(
                            xa,
                            band_y + band_pad,
                            xb - xa,
                            (band_height - 2.0 * band_pad).max(1.0),
                        );
                        let flushed = tx.status == Status::Aborted;
                        if flushed {
                            let ink = if contrast(style.fill, t.editor.bg)
                                >= contrast(style.edge, t.editor.bg)
                            {
                                style.fill
                            } else {
                                style.edge
                            };
                            let border_ink = ink.with_alpha(ink.a * stage_edge_alpha);
                            let hatch_alpha = if stage_hidden {
                                HIDDEN_STAGE_HATCH_ALPHA * dim_alpha
                            } else {
                                0.55 * dim_alpha
                            };
                            p.scene.clipped(cells, |scene| {
                                scene.quad(rect, Color::TRANSPARENT, 0.0, 1.0, border_ink);
                                // Keep strokes that straddle the viewport edge in the hatch path.
                                let left = rect.left().max(cells.left() - 1.0);
                                let right = rect.right().min(cells.right() + 1.0);
                                let hatch =
                                    Rect::from_xywh(left, rect.top(), right - left, rect.height());
                                marks::hatch_with(
                                    scene,
                                    hatch,
                                    cells.left(),
                                    z(HOLLOW_HATCH_PX).max(2.0),
                                    ink.with_alpha(hatch_alpha),
                                );
                                stage_ticks(
                                    scene,
                                    rect,
                                    ticks,
                                    ink.with_alpha(STAGE_TICK_ALPHA * stage_edge_alpha),
                                );
                            });
                        } else {
                            let fill_c = style.fill.with_alpha(style.fill.a * stage_fill_alpha);
                            let (border_w, border_c) = if border && xb - xa >= 3.0 {
                                (1.0, style.edge.with_alpha(style.edge.a * stage_edge_alpha))
                            } else {
                                (0.0, Color::TRANSPARENT)
                            };
                            p.scene.clipped(cells, |scene| {
                                scene.quad(rect, fill_c, 0.0, border_w, border_c);
                                stage_ticks(
                                    scene,
                                    rect,
                                    ticks,
                                    style.text.with_alpha(STAGE_TICK_ALPHA * stage_fill_alpha),
                                );
                            });
                        }
                        if sort_stage_idx == Some(stage_idx) && !dimmed && !stage_hidden && border {
                            let tick_w = z(2.0).max(1.5);
                            sort_tick = Some(Rect::from_xywh(
                                snap(xa) - tick_w * 0.5,
                                (rect.top() - 1.0).max(y),
                                tick_w,
                                (rect.height() + 2.0).min(row_px),
                            ));
                        }
                        last_right = Some(last_right.map_or(xb, |r: f32| r.max(xb)));
                        let label_left = xa.max(cells.left());
                        let label_right = xb.min(cells.right());
                        if !dimmed
                            && !stage_hidden
                            && band_height >= TEXT_MIN_PX
                            && let Some((label, w)) = fitting_stage_label(
                                &mut p,
                                &stage.name,
                                cell_font,
                                label_right - label_left - z(CELL_TEXT_PAD_PX),
                            )
                        {
                            cell_texts.push((
                                point(snap((label_left + label_right - w) / 2.0), band_y),
                                band_height,
                                label,
                                if flushed { t.editor.text } else { style.text },
                                cell_font,
                            ));
                        }
                    }
                    if tx.stages.is_empty() {
                        let style = t.stage_fallback();
                        let mut xa = x_of(tx.begin) + gap / 2.0;
                        let mut xb = x_of(tx.end) - gap / 2.0;
                        if xb - xa < 1.0 {
                            xa = xa.min(xb);
                            xb = xa + 1.0;
                        }
                        let rect =
                            Rect::from_xywh(xa, y + pad, xb - xa, (row_px - 2.0 * pad).max(1.0));
                        let fill_c = style.fill.with_alpha(style.fill.a * dim_alpha);
                        let (border_w, border_c) = if border && xb - xa >= 3.0 {
                            (1.0, style.edge.with_alpha(style.edge.a * dim_alpha))
                        } else {
                            (0.0, Color::TRANSPARENT)
                        };
                        p.scene.clipped(cells, |scene| {
                            scene.quad(rect, fill_c, 0.0, border_w, border_c);
                        });
                        last_right = Some(xb);
                    }
                    // 3px source-accent stripe on the true left edge of the transaction bar.
                    if merged
                        && !dimmed
                        && border
                        && let Some(xr) = last_right
                        && (tx.begin as f64) >= viewport.start
                        && (tx.begin as f64) <= viewport.end
                        && let Some(source) = model.sources.get(source_index)
                    {
                        let xl = x_of(tx.begin) + gap / 2.0;
                        let tick_w = z(3.0).min((xr - xl).max(1.0));
                        let accent = t.source_accent(source.color_slot);
                        let tick =
                            Rect::from_xywh(xl, y + pad, tick_w, (row_px - 2.0 * pad).max(1.0));
                        p.scene.clipped(cells, |scene| scene.fill(tick, accent));
                    }
                    // Subtle accent tick on the matched sort-stage left edge when ordered by stage.
                    if let Some(tick) = sort_tick {
                        p.scene
                            .clipped(cells, |scene| scene.fill(tick, t.border_focused));
                    }
                    if tx.status == Status::Open
                        && !dimmed
                        && let Some(xr) = last_right
                    {
                        let x = snap(xr) + 0.5;
                        let mut yy = y + pad;
                        let bottom = y + row_px - pad;
                        while yy < bottom {
                            let end = (yy + z(DASH_PX)).min(bottom);
                            open_edges.push([point(x, yy), point(x, end)]);
                            yy = end + z(DASH_PX);
                        }
                    }
                }
                p.scene.clipped(cells, |scene| {
                    for (origin, height, text, color, cell_font) in cell_texts {
                        scene.text(origin, height, text, FontRole::Mono, cell_font, color);
                    }
                    scene.lines(open_edges, t.editor.text, 1.0);
                });
            }

            if let Some(row) = selected_row {
                let y = layout.row_y(row);
                let rect = Rect::new(point(cells.left(), y), size(cells.width(), row_px));
                let focus = t.border_focused;
                let (into, out) = (t.tx_relation_in, t.tx_relation_out);
                let selection = model.nav.selection(doc);
                p.scene.clipped(cells, |scene| {
                    scene.quad(rect, Color::TRANSPARENT, 0.0, 1.0, focus);
                    let Some(selection) = selection else { return };
                    let Some(generator) = doc.resident_generator(selection.track) else {
                        return;
                    };
                    let center = |r: usize| layout.row_y(r) + row_px * 0.5;
                    let anchor = |r: usize, time: u64| point(x_of(time), center(r));
                    let here = anchor(row, set.get(row).map_or(0, |(_, tx)| tx.begin));
                    let mut incoming = Vec::new();
                    let mut outgoing = Vec::new();
                    for edge in generator.relations_of(selection.id) {
                        let forward = match edge.from_generator {
                            Some(from_gen) => {
                                from_gen == selection.track.item
                                    && edge.relation.from == selection.id
                            }
                            None => {
                                generator.transaction_ordinal(edge.relation.from)
                                    == generator.transaction_ordinal(selection.id)
                            }
                        };
                        let (other, other_generator) = if forward {
                            (edge.relation.to, edge.to_generator)
                        } else {
                            (edge.relation.from, edge.from_generator)
                        };
                        let Some(other_row) = other_generator.and_then(|generator| {
                            set.row_of(selection.track.trace, generator, other)
                        }) else {
                            continue;
                        };
                        let Some((_, tx)) = set.get(other_row) else {
                            continue;
                        };
                        let far = anchor(other_row, tx.begin);
                        let (from, to) = if forward { (here, far) } else { (far, here) };
                        let target = if forward {
                            &mut outgoing
                        } else {
                            &mut incoming
                        };
                        target.push([from, to]);
                        target.extend(arrow_head(from, to, z(5.0)));
                    }
                    scene.lines(incoming, into, 1.0);
                    scene.lines(outgoing, out, 1.0);
                });
            }

            let merged = model.is_merged();
            let badges = model.badges();
            let label_font = (row_px * 0.62).clamp(z(7.0), t.mono_size);
            let badge_font = layout.badge_font;
            let digits = (set.len().max(1)).ilog10() as usize + 1;
            let digit_w = p.width("0", FontRole::Mono, label_font);
            let badge_left = layout.badge_left;
            let badge_w = layout.badge_width;
            let index_right = if merged {
                layout.badge_right() + z(6.0) + digit_w * digits as f32
            } else {
                labels.left() + z(8.0) + digit_w * digits as f32
            };
            let text_left = index_right + z(8.0);
            if row_px >= LABEL_MIN_PX {
                let badge_widths: Vec<f32> = if merged {
                    badges
                        .labels
                        .iter()
                        .map(|lbl| p.width(&lbl.badge, FontRole::Mono, badge_font))
                        .collect()
                } else {
                    Vec::new()
                };
                let is_dark = contrast(Color::rgb(0x00ff_ffff), t.panel.bg)
                    > contrast(Color::rgb(0x0000_0000), t.panel.bg);
                let mut texts: Vec<(Point, String, Color, f32)> = Vec::new();
                let mut fills: Vec<(Rect, Color)> = Vec::new();
                let mut quads: Vec<(Rect, Color, f32, f32, Color)> = Vec::new();
                for r in layout.row_range.clone() {
                    let Some((source_index, tx)) = set.get(r) else {
                        continue;
                    };
                    let y = layout.row_y(r);
                    let dimmed = model.is_source_dimmed(source_index);
                    let dim_alpha = if dimmed { 0.25 } else { 1.0 };
                    if hover_row == Some(r) {
                        fills.push((
                            Rect::new(point(labels.left(), y), size(labels.width(), row_px)),
                            t.hover.bg,
                        ));
                    }
                    if selected_row == Some(r) {
                        fills.push((
                            Rect::new(point(labels.left(), y), size(labels.width(), row_px)),
                            t.wave_row_selected,
                        ));
                    }
                    if merged && let Some(source) = model.sources.get(source_index) {
                        let accent = t.source_accent(source.color_slot);
                        let stripe_alpha = if dimmed { 0.22 } else { 1.0 };
                        fills.push((
                            Rect::from_xywh(
                                labels.left(),
                                y + 1.0,
                                z(3.0),
                                (row_px - 2.0).max(1.0),
                            ),
                            accent.with_alpha(accent.a * stripe_alpha),
                        ));
                        if let Some(lbl) = badges.labels.get(source_index) {
                            let pill_pad = (row_px * 0.14).clamp(1.0, z(3.0));
                            let pill_h = (row_px - 2.0 * pill_pad).max(1.0);
                            let badge_hovered = hover_badge_row == Some(r);
                            let source_focused = model.focused_source == Some(source_index);
                            let base_alpha = if is_dark { 0.14 } else { 0.10 };
                            let fill_alpha = if badge_hovered || source_focused {
                                (base_alpha + 0.08) * dim_alpha
                            } else {
                                base_alpha * dim_alpha
                            };
                            let (border_w, border_c) = if badge_hovered || source_focused {
                                (1.0, accent.with_alpha(0.75 * dim_alpha))
                            } else {
                                (0.0, Color::TRANSPARENT)
                            };
                            quads.push((
                                Rect::from_xywh(badge_left, y + pill_pad, badge_w, pill_h),
                                accent.with_alpha(fill_alpha),
                                z(3.0),
                                border_w,
                                border_c,
                            ));
                            let bw = badge_widths.get(source_index).copied().unwrap_or(0.0);
                            texts.push((
                                point(snap(badge_left + ((badge_w - bw) * 0.5).max(z(2.0))), y),
                                lbl.badge.clone(),
                                accent.with_alpha(accent.a * dim_alpha),
                                badge_font,
                            ));
                        }
                    }
                    let index = r.to_string();
                    let w = digit_w * index.len() as f32;
                    let idx_color = t
                        .panel
                        .text_placeholder
                        .with_alpha(t.panel.text_placeholder.a * dim_alpha);
                    texts.push((
                        point(snap(index_right - w), y),
                        index,
                        idx_color,
                        label_font,
                    ));
                    let label = PipelineModel::label(tx);
                    let color = match tx.status {
                        Status::Aborted => {
                            let stripe_x = if merged {
                                labels.left() + z(3.0)
                            } else {
                                labels.left()
                            };
                            fills.push((
                                Rect::new(
                                    point(stripe_x, y + pad),
                                    size(z(2.0), row_px - 2.0 * pad),
                                ),
                                t.editor.error.with_alpha(t.editor.error.a * dim_alpha),
                            ));
                            t.panel.text_muted
                        }
                        Status::Open => t.panel.text_muted,
                        _ => t.panel.text,
                    };
                    if !label.is_empty() {
                        texts.push((
                            point(text_left, y),
                            label.to_owned(),
                            color.with_alpha(color.a * dim_alpha),
                            label_font,
                        ));
                    }
                }
                p.scene.clipped(labels, |scene| {
                    for (rect, color) in fills {
                        scene.fill(rect, color);
                    }
                    for (rect, color, radius, border_w, border_c) in quads {
                        scene.quad(rect, color, radius, border_w, border_c);
                    }
                    for (origin, text, color, font_size) in texts {
                        scene.text(origin, row_px, text, FontRole::Mono, font_size, color);
                    }
                });
                if merged && !layout.is_overview() {
                    let badge_col = Rect::from_xywh(
                        labels.left(),
                        labels.top(),
                        (layout.badge_right() + z(2.0) - labels.left()).min(labels.width()),
                        labels.height(),
                    );
                    p.scene.cursors.push((badge_col, CursorIcon::PointingHand));
                }
            } else {
                let ticks = set.aborted_presence(&map);
                p.scene.clipped(labels, |scene| {
                    for (y, _) in ticks.runs() {
                        scene.fill(
                            Rect::from_xywh(
                                labels.left(),
                                map.rect.top() + y as f32 / layout.device_scale as f32,
                                z(2.0),
                                1.0 / layout.device_scale as f32,
                            ),
                            t.editor.error,
                        );
                    }
                });
            }
        }
        _ => {
            let area = Rect::new(
                point(bounds.left(), cells.top()),
                size(bounds.width(), cells.height()),
            );
            p.scene.fill(area, t.editor.bg);
            let path = if model.is_merged() {
                model.badges().panel_title.clone()
            } else {
                model
                    .sources
                    .first()
                    .map_or_else(String::new, |s| s.track.path().join("."))
            };
            let all_hidden = !model.sources.is_empty()
                && (0..model.sources.len()).all(|i| !model.is_source_effectively_visible(i));
            let (title, hint, error): (String, String, bool) = match &rows {
                Rows::Loading => (format!("Loading {path}…"), String::new(), false),
                Rows::Failed(e) => (format!("Cannot load {path}"), (*e).to_owned(), true),
                Rows::Unresolved => (
                    "Not in this trace".into(),
                    format!("{path} was saved in the workspace but is not a track here"),
                    false,
                ),
                Rows::Unavailable => ("No trace open".into(), String::new(), false),
                Rows::Ready(_) if all_hidden => (
                    "All sources hidden".into(),
                    "Use Show All or toggle a source chip to display transactions".into(),
                    false,
                ),
                Rows::Ready(set) if set.unmatched_count() > 0 => (
                    "All rows hidden by row order".into(),
                    format!(
                        "{} unmatched rows hidden · switch to Fallback or reset order to Start",
                        set.unmatched_count()
                    ),
                    false,
                ),
                Rows::Ready(_) => (
                    "No transactions recorded".into(),
                    format!("{path} has no records"),
                    false,
                ),
            };
            let cx_ = area.left() + area.width() / 2.0;
            let cy = area.top() + area.height() / 2.0;
            let icon = Rect::new(point(cx_ - z(16.0), cy - z(44.0)), size(z(32.0), z(32.0)));
            p.scene.icon(
                IconName::Workflow,
                icon,
                if error {
                    t.editor.error
                } else {
                    t.editor.text_placeholder
                },
            );
            let title_w = p.width(&title, FontRole::UiMedium, t.ui_size);
            p.scene.text(
                point(snap(cx_ - title_w / 2.0), cy - z(4.0)),
                z(20.0),
                title,
                FontRole::UiMedium,
                t.ui_size,
                t.editor.text_muted,
            );
            if !hint.is_empty() {
                let hint_w = p.width(&hint, FontRole::Ui, t.ui_size_small);
                p.scene.text(
                    point(snap(cx_ - hint_w / 2.0), cy + z(18.0)),
                    z(18.0),
                    hint,
                    FontRole::Ui,
                    t.ui_size_small,
                    if error {
                        t.editor.error
                    } else {
                        t.editor.text_placeholder
                    },
                );
            }
            if let Some(retry) = layout.retry {
                let hovered = model.hover == Some(Hit::Retry);
                let surface = if hovered { t.button_hover } else { t.button };
                p.scene
                    .quad(retry, surface.bg, z(4.0), 1.0, t.border_variant);
                let label = "Retry";
                let w = p.width(label, FontRole::UiMedium, t.ui_size_small);
                p.scene.text(
                    point(snap(retry.left() + (retry.width() - w) / 2.0), retry.top()),
                    retry.height(),
                    label,
                    FontRole::UiMedium,
                    t.ui_size_small,
                    surface.text,
                );
                p.scene.cursors.push((retry, CursorIcon::PointingHand));
            }
        }
    }

    if matches!(&rows, Rows::Ready(set) if !set.is_empty()) {
        let activity = model.last_layout().activity;
        if activity.visible == 0 {
            let message = if activity.above + activity.below == 0 {
                match (activity.earlier, activity.later) {
                    (true, true) => "No pipeline activity in this time window · activity ← and →",
                    (true, false) => "No pipeline activity in this time window · activity ←",
                    (false, true) => "No pipeline activity in this time window · activity →",
                    _ => "No pipeline activity in this time window",
                }
            } else {
                "No activity in these rows · reveal rows above/below or resume follow"
            };
            p.scene.clipped(cells, |scene| {
                scene.text(
                    point(cells.left() + z(12.0), cells.top() + cells.height() * 0.5),
                    z(22.0),
                    message,
                    FontRole::Ui,
                    t.ui_size_small,
                    t.editor.text_muted,
                );
            });
        }
    }

    for (_, rect, label) in &layout.activity_controls {
        p.scene
            .quad(*rect, t.button.bg, z(6.0), 1.0, t.border_variant);
        let width = p.width(label, FontRole::UiMedium, t.ui_size_small);
        p.scene.text(
            point(rect.left() + (rect.width() - width) * 0.5, rect.top()),
            rect.height(),
            label.clone(),
            FontRole::UiMedium,
            t.ui_size_small,
            t.button.text,
        );
        p.scene.cursors.push((*rect, CursorIcon::PointingHand));
    }

    p.scene.clipped(
        Rect::new(header.origin, size(labels.width(), header.height())),
        |scene| {
            if model.is_merged() {
                scene.text(
                    point(layout.badge_left, header.top()),
                    header.height(),
                    "Src",
                    FontRole::UiSemibold,
                    t.ui_size_small,
                    t.panel.text_muted,
                );
                scene.text(
                    point(layout.badge_right() + z(6.0), header.top()),
                    header.height(),
                    "# · Label",
                    FontRole::UiSemibold,
                    t.ui_size_small,
                    t.panel.text_muted,
                );
            } else {
                scene.text(
                    point(labels.left() + z(8.0), header.top()),
                    header.height(),
                    "#",
                    FontRole::UiSemibold,
                    t.ui_size_small,
                    t.panel.text_muted,
                );
                scene.text(
                    point(labels.left() + z(36.0), header.top()),
                    header.height(),
                    "Label",
                    FontRole::UiSemibold,
                    t.ui_size_small,
                    t.panel.text_muted,
                );
            }
        },
    );
    overlay::header_ticks(&mut p, &column, &tick_list, unit);
    overlay::clock_rulers(
        &mut p,
        &column,
        overlay::RulerCells {
            scopes: None,
            names: Rect::new(
                point(bounds.left(), layout.rulers.top()),
                size(labels.width(), layout.rulers.height()),
            ),
            values: None,
            selected: model.nav.ruler_selection(),
        },
        clocks,
        &doc.clocks,
        base,
        cursor,
    );

    overlay::marker_lane_paint(
        &mut p,
        &column,
        &layout.marker_lane,
        labels,
        None,
        doc,
        cursor,
        model.pointer,
        marker_tooltip_visible,
        match model.drag {
            Some(super::model::Drag::Marker(held)) if held.moved => Some(held.from),
            _ => None,
        },
    );
    overlay::cursor(&mut p, &column, cursor, base, focused, z(4.0));

    let near_split = model.hover == Some(Hit::LabelSplit);
    let split_border = if near_split || matches!(model.drag, Some(Drag::LabelSplit { .. })) {
        t.border_focused
    } else {
        t.border
    };
    p.scene.fill(
        Rect::new(
            point(labels.right() - 1.0, bounds.top()),
            size(1.0, bounds.height()),
        ),
        split_border,
    );
    p.scene.fill(
        Rect::new(
            point(bounds.left(), header.bottom() - 1.0),
            size(bounds.width(), 1.0),
        ),
        t.border_variant,
    );
    p.scene.fill(layout.gutter, t.panel.bg);
    super::plot::paint(model, doc, p.scene, t, focused);
    if let Some(selection) = model.selection_box() {
        let area = selection.area;
        let rect = selection.rect;
        let label = "Zoom to selected area · Esc to cancel";
        let label_width = p.width(label, FontRole::Ui, t.ui_size);
        let pointer = model.pointer.unwrap_or(rect.origin);
        let label_x = (pointer.x + z(8.0))
            .min(area.right() - label_width - z(8.0))
            .max(area.left() + z(8.0));
        let label_y = (pointer.y + z(8.0))
            .min(area.bottom() - z(20.0))
            .max(area.top());
        p.scene.clipped(area, |scene| {
            scene.quad(
                rect,
                if selection.valid {
                    t.selection.bg.with_alpha(0.35)
                } else {
                    Color::TRANSPARENT
                },
                0.0,
                1.0,
                t.editor.text,
            );
            scene.text(
                point(label_x, label_y),
                z(20.0),
                label,
                FontRole::Ui,
                t.ui_size,
                t.editor.text,
            );
        });
    }

    if let Rows::Ready(set) = &rows {
        layout.navigator.paint(
            set,
            layout.rows,
            viewport,
            layout.cells_width_f64(),
            t,
            p.scene,
        );
    }
    let resizing_navigator = matches!(model.drag, Some(Drag::NavigatorSplit { .. }));
    let splitter = layout.navigator_split;
    let highlight = model.hover == Some(Hit::NavigatorSplit) || resizing_navigator;
    let x = layout
        .navigator
        .rect
        .left()
        .min(bounds.right() - z(2.0))
        .max(bounds.left());
    p.scene.fill(
        Rect::from_xywh(
            x,
            splitter.top(),
            if resizing_navigator { z(4.0) } else { 1.0 },
            splitter.height(),
        ),
        if highlight {
            t.border_focused
        } else {
            t.border_variant
        },
    );
    let target = layout.navigator_grip_target;
    let grip_width = z(12.0).min(target.width());
    let grip_height = z(40.0).min(target.height());
    let grip = Rect::from_xywh(
        (layout.navigator.rect.left() - grip_width * 0.5)
            .clamp(bounds.left(), bounds.right() - grip_width),
        target.top() + (target.height() - grip_height) * 0.5,
        grip_width,
        grip_height,
    );
    p.scene.quad(
        grip,
        if highlight { t.hover.bg } else { t.elevated.bg },
        z(4.0),
        1.0,
        if highlight {
            t.border_focused
        } else {
            t.border
        },
    );
    let icon_size = z(10.0).min(grip.width()).min(grip.height());
    p.scene.icon(
        if model.navigator.visible {
            IconName::ChevronsRight
        } else {
            IconName::ChevronsLeft
        },
        Rect::from_xywh(
            grip.left() + (grip.width() - icon_size) * 0.5,
            grip.top() + (grip.height() - icon_size) * 0.5,
            icon_size,
            icon_size,
        ),
        if highlight {
            t.border_focused
        } else {
            t.panel.text_muted
        },
    );
    p.scene
        .cursors
        .push((layout.navigator.rect, CursorIcon::PointingHand));
    p.scene
        .cursors
        .push((splitter, CursorIcon::ResizeLeftRight));
    p.scene.cursors.push((target, CursorIcon::ResizeLeftRight));
    match model.drag {
        Some(Drag::NavigatorSplit { .. } | Drag::LabelSplit { .. }) => {
            p.scene.window_cursor = Some(CursorIcon::ResizeLeftRight);
        }
        Some(Drag::Navigator | Drag::Pan { moved: true, .. }) => {
            p.scene.window_cursor = Some(CursorIcon::Grabbing);
        }
        _ => p
            .scene
            .cursors
            .push((layout.label_split, CursorIcon::ResizeLeftRight)),
    }
}
