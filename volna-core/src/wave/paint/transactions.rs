//! Pixel-bounded transaction geometry, independent of hidden sub-rows and overlap density.

#[cfg(test)]
mod tests;

use super::{
    Document, FontRole, LaneGeometry, Point, Rect, Scene, Status, TextPainter, Theme, TxLane,
    Viewport, fit_mono_prefix, lane, point,
};

const PIXELS: usize = 4096;
const LABELS: usize = 384;

#[derive(Default)]
struct Bars {
    solid: Vec<Rect>,
    failed: Vec<Rect>,
    dense: Vec<Rect>,
    failed_dense: Vec<Rect>,
    edges: Vec<[Point; 2]>,
    failed_edges: Vec<[Point; 2]>,
    labels: Vec<(Point, String, bool)>,
}

pub(super) fn paint(
    lane: &TxLane,
    plan: &lane::RenderPlan<'_>,
    doc: &Document,
    viewport: &Viewport,
    waves: Rect,
    geometry: &LaneGeometry,
    p: &mut TextPainter<'_>,
) {
    let index = plan.index;
    let generator = index.source();
    let t = p.theme;
    let normal = t.transaction_style(false);
    let error = t.transaction_style(true);
    let z = |v: f32| v * t.zoom;
    let width = f64::from(waves.width()).max(1.0);
    let x_of = |time: u64| {
        (waves.left() + viewport.x_of_tick(time, width) as f32).clamp(waves.left(), waves.right())
    };
    let inset = z(2.0);
    let bar_h = (geometry.sub_h - 2.0 * inset).max(1.0);
    let font = (bar_h * 0.9).min(t.mono_size);
    let records = generator.transactions();
    let mut bars = Bars::default();
    let columns = (waves.width().ceil().max(1.0) as usize).min(PIXELS);
    let pixel_time = viewport.width() / columns as f64;

    for (sub, detail) in plan.details.iter().enumerate() {
        let Some(row) = index.row(lane.first_sub_row + sub) else {
            continue;
        };
        let top = geometry.sub_top(sub) + inset;
        if let Some(candidates) = *detail {
            for &ordinal in candidates {
                let tx = &records[ordinal];
                let xa = x_of(tx.begin);
                let xb = x_of(tx.end).max(xa + z(2.0)).min(waves.right());
                let rect = Rect::from_xywh(xa, top, (xb - xa).max(0.0), bar_h);
                let failed = tx.status == Status::Error;
                let solids = if failed {
                    &mut bars.failed
                } else {
                    &mut bars.solid
                };
                append(solids, rect);
                outline(bars.edges(failed), rect);
                if bars.labels.len() < LABELS && bar_h >= z(8.0) && xb - xa > z(16.0) {
                    let text = lane::label(tx);
                    if let Some(label) = fit_mono_prefix(p, &text, font, xb - xa - z(8.0)) {
                        bars.labels.push((point(xa + z(4.0), top), label, failed));
                    }
                }
            }
        } else {
            let mut outlined = None;
            for x in 0..columns {
                let a = viewport.start + x as f64 * pixel_time;
                let (active, failed) = row.activity(a, a + pixel_time);
                let left = waves.left() + waves.width() * x as f32 / columns as f32;
                let w = waves.width() / columns as f32;
                let rect = Rect::from_xywh(left, top, w, bar_h);
                let candidates = row.window(a, a + pixel_time);
                if active {
                    let fills = if failed {
                        &mut bars.failed_dense
                    } else {
                        &mut bars.dense
                    };
                    append(fills, rect);
                }
                if let Some(&ordinal) = candidates.last()
                    && outlined != Some(ordinal)
                {
                    outlined = Some(ordinal);
                    let tx = &records[ordinal];
                    let xa = x_of(tx.begin);
                    let xb = x_of(tx.end);
                    if xb - xa > 2.0 {
                        let rect = Rect::from_xywh(xa, top, xb - xa, bar_h);
                        let failed = tx.status == Status::Error;
                        let fills = if failed {
                            &mut bars.failed
                        } else {
                            &mut bars.solid
                        };
                        append(fills, rect);
                        outline(bars.edges(failed), rect);
                    }
                }
            }
        }
    }
    let occupied_top = geometry.occupancy_top();
    let area = Rect::from_xywh(
        waves.left(),
        occupied_top,
        waves.width(),
        geometry.top + geometry.height - occupied_top,
    );
    p.scene.clipped(
        Rect::from_xywh(waves.left(), geometry.top, waves.width(), geometry.height),
        |scene| {
            scene.rects(bars.dense, normal.edge);
            scene.rects(bars.failed_dense, error.edge);
            scene.rects(bars.solid, normal.fill);
            scene.rects(bars.failed, error.fill);
            scene.lines(bars.edges, normal.edge, 1.0);
            scene.lines(bars.failed_edges, error.edge, 1.0);
            for (origin, label, failed) in bars.labels {
                let color = if failed { error.text } else { normal.text };
                scene.text(origin, bar_h, label, FontRole::Mono, font, color);
            }
            if lane.occupancy {
                occupancy(scene, &index.occupancy, viewport, area, t);
            } else if area.height() > 0.0 {
                occupancy_ribbon(scene, &index.occupancy, viewport, area, t);
            }
            if let Some(selected) = doc.selection().filter(|s| Some(s.track) == lane.track())
                && let Some(ordinal) = generator.transaction_ordinal(selected.id)
            {
                let tx = &records[ordinal];
                if tx.begin as f64 <= viewport.end && tx.end as f64 >= viewport.start {
                    let row = generator.sub_row(ordinal);
                    let visible =
                        row >= lane.first_sub_row && row < lane.first_sub_row + geometry.capacity;
                    let top = if visible {
                        geometry.sub_top(row - lane.first_sub_row) + inset
                    } else {
                        area.top() + z(2.0)
                    };
                    let h = if visible {
                        bar_h
                    } else {
                        (area.height() - z(4.0)).max(1.0)
                    };
                    let xa = x_of(tx.begin);
                    let xb = x_of(tx.end).max(xa + z(2.0));
                    scene.quad(
                        Rect::from_xywh(xa, top, xb - xa, h),
                        t.border_focused
                            .with_alpha(if visible { 0.0 } else { 0.10 }),
                        z(2.0),
                        2.0,
                        t.border_focused,
                    );
                }
            }
        },
    );
}

impl Bars {
    fn edges(&mut self, failed: bool) -> &mut Vec<[Point; 2]> {
        if failed {
            &mut self.failed_edges
        } else {
            &mut self.edges
        }
    }
}

fn outline(edges: &mut Vec<[Point; 2]>, rect: Rect) {
    // Keep the entire stroke inside the lifetime so one-pixel gaps stay empty.
    let rect = Rect::from_xywh(
        rect.left() + 0.5,
        rect.top() + 0.5,
        (rect.width() - 1.0).max(0.0),
        (rect.height() - 1.0).max(0.0),
    );
    let (a, b, c, d) = (
        rect.origin,
        point(rect.right(), rect.top()),
        point(rect.right(), rect.bottom()),
        point(rect.left(), rect.bottom()),
    );
    edges.extend([[a, b], [b, c], [c, d], [d, a]]);
}

fn append(rects: &mut Vec<Rect>, rect: Rect) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    if let Some(last) = rects.last_mut()
        && last.top() == rect.top()
        && last.height() == rect.height()
        && last.left() <= rect.left()
        && (last.right() - rect.left()).abs() < 0.01
    {
        last.size.width = rect.right() - last.left();
    } else {
        rects.push(rect);
    }
}

fn occupancy(
    scene: &mut Scene,
    summary: &crate::data::occupancy::Occupancy,
    viewport: &Viewport,
    area: Rect,
    t: &Theme,
) {
    let z = |v: f32| v * t.zoom;
    let bottom = area.bottom() - z(3.0);
    let heading = if area.height() >= z(32.0) {
        z(17.0)
    } else {
        0.0
    };
    let height = (area.height() - z(7.0) - heading).max(1.0);
    let peak = summary.maximum().max(1) as f64;
    let columns = (area.width().ceil().max(1.0) as usize).min(PIXELS);
    let step = viewport.width() / columns as f64;
    let w = area.width() / columns as f32;
    let mut fills = Vec::with_capacity(columns);
    let mut peaks = Vec::with_capacity(columns * 2);
    let mut errors = Vec::new();
    let mut previous = None;
    for x in 0..columns {
        let start = viewport.start + x as f64 * step;
        let sample = summary.sample(start, start + step);
        let left = area.left() + x as f32 * w;
        if sample.average > 0.0 {
            let h = (height * (sample.average / peak) as f32).max(0.5);
            append(&mut fills, Rect::from_xywh(left, bottom - h, w, h));
        }
        if sample.peak > 0 {
            let y = bottom - height * (sample.peak as f64 / peak) as f32;
            if let Some(old) = previous.filter(|&old| old != y) {
                peaks.push([point(left, old), point(left, y)]);
            }
            if let Some(last) = peaks
                .last_mut()
                .filter(|last| last[0].y == y && last[1].y == y && (last[1].x - left).abs() < 0.01)
            {
                last[1].x = left + w;
            } else {
                peaks.push([point(left, y), point(left + w, y)]);
            }
            previous = Some(y);
        } else {
            previous = None;
        }
        if sample.failed {
            append(
                &mut errors,
                Rect::from_xywh(left, area.top() + heading + z(1.0), w, z(2.0)),
            );
        }
    }
    scene.fill(area, t.panel.bg.with_alpha(0.55));
    scene.fill(
        Rect::from_xywh(area.left(), area.top(), area.width(), 1.0),
        t.border_variant,
    );
    if let (Some(first), Some(last)) = (fills.first(), fills.last()) {
        let mut points = Vec::with_capacity(fills.len() * 2 + 2);
        points.push(point(first.left(), bottom));
        let mut right = first.left();
        for rect in &fills {
            if rect.left() > right {
                points.push(point(right, bottom));
                points.push(point(rect.left(), bottom));
            }
            points.push(rect.origin);
            points.push(point(rect.right(), rect.top()));
            right = rect.right();
        }
        points.push(point(last.right(), bottom));
        scene.polygon(points, t.wave_signal.with_alpha(0.35));
    }
    scene.lines(peaks, t.wave_signal.with_alpha(0.85), 1.0);
    scene.rects(errors, t.editor.error);
    scene.fill(
        Rect::from_xywh(area.left(), bottom, area.width(), 1.0),
        t.border_variant,
    );
    if area.height() >= z(32.0) {
        let label = if area.width() >= z(240.0) {
            format!("occupancy · avg / peak · 0–{}", summary.maximum())
        } else {
            format!("occupancy · 0–{}", summary.maximum())
        };
        scene.text(
            point(area.left() + z(6.0), area.top() + z(1.0)),
            z(16.0),
            label,
            FontRole::Ui,
            t.ui_size_small,
            t.panel.text_muted,
        );
    }
}

fn occupancy_ribbon(
    scene: &mut Scene,
    summary: &crate::data::occupancy::Occupancy,
    viewport: &Viewport,
    area: Rect,
    theme: &Theme,
) {
    let columns = (area.width().ceil().max(1.0) as usize).min(PIXELS);
    let width = area.width() / columns as f32;
    let scale = summary.maximum().max(1) as f64;
    let mut fills: [Vec<Rect>; 16] = std::array::from_fn(|_| Vec::new());
    let mut errors = Vec::new();
    let mut points = Vec::new();
    scene.fill(area, theme.panel.bg);
    for (x, sample) in summary
        .bins(viewport.start, viewport.end, columns)
        .enumerate()
    {
        let rect = Rect::from_xywh(
            area.left() + x as f32 * width,
            area.top() + theme.zoom,
            width,
            (area.height() - theme.zoom).max(1.0),
        );
        if sample.average > 0.0 {
            let density = ((sample.average - 1.0).max(0.0) / (scale - 1.0).max(1.0)).sqrt();
            let level = 1 + (density.clamp(0.0, 1.0) * 14.0).round() as usize;
            append(&mut fills[level], rect);
        }
        if sample.failed {
            append(&mut errors, rect);
        }
        if sample.has_points {
            append(
                &mut points,
                Rect::from_xywh(
                    rect.left(),
                    rect.top(),
                    width.max(theme.zoom),
                    rect.height(),
                ),
            );
        }
    }
    for (level, rects) in fills.into_iter().enumerate().skip(1) {
        scene.rects(
            rects,
            theme
                .wave_signal
                .with_alpha(0.20 + 0.80 * (level - 1) as f32 / 14.0),
        );
    }
    scene.rects(points, theme.wave_event_coalesced);
    scene.rects(errors, theme.editor.error);
    scene.fill(
        Rect::from_xywh(area.left(), area.top(), area.width(), theme.zoom),
        theme.border_variant,
    );
}
