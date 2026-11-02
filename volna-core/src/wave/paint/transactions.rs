//! Pixel-bounded transaction geometry, independent of hidden sub-rows and overlap density.

#[cfg(test)]
mod tests;

use super::{
    Document, FontRole, LaneGeometry, PipelineModel, Point, Rect, Scene, Status, TextPainter,
    Theme, TxLane, Viewport, fit_mono_prefix, lane, point,
};

const PIXELS: usize = 4096;
const LABELS: usize = 384;

#[derive(Default)]
struct Bars {
    background: Vec<Rect>,
    solid: Vec<Rect>,
    failed: Vec<Rect>,
    edges: Vec<[Point; 2]>,
    failed_edges: Vec<[Point; 2]>,
    labels: Vec<(Point, String)>,
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
                append(&mut bars.background, rect);
                let solids = if failed {
                    &mut bars.failed
                } else {
                    &mut bars.solid
                };
                if tx.stages.is_empty() {
                    append(solids, rect);
                } else {
                    for stage in &tx.stages {
                        let a = x_of(stage.begin).max(xa);
                        let b = x_of(PipelineModel::stage_end(tx, stage)).min(xb);
                        if b >= a {
                            append(
                                solids,
                                Rect::from_xywh(a, top, (b - a).max(1.0).min(xb - a), bar_h),
                            );
                        }
                    }
                }
                let edges = if failed {
                    &mut bars.failed_edges
                } else {
                    &mut bars.edges
                };
                outline(edges, rect);
                if bars.labels.len() < LABELS && bar_h >= z(8.0) && xb - xa > z(16.0) {
                    let text = lane::label(tx);
                    if let Some(label) = fit_mono_prefix(p, &text, font, xb - xa - z(8.0)) {
                        bars.labels.push((point(xa + z(4.0), top), label));
                    }
                }
            }
        } else {
            for x in 0..columns {
                let a = viewport.start + x as f64 * pixel_time;
                let (active, stage, failed) = row.activity(a, a + pixel_time);
                let left = waves.left() + waves.width() * x as f32 / columns as f32;
                let w = waves.width() / columns as f32;
                let rect = Rect::from_xywh(left, top, w, bar_h);
                if active {
                    append(&mut bars.background, rect);
                }
                if stage {
                    if failed {
                        append(&mut bars.failed, rect);
                    } else {
                        append(&mut bars.solid, rect);
                    }
                } else if failed {
                    append(&mut bars.failed, Rect::from_xywh(left, top, w, z(2.0)));
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
            scene.rects(bars.background, t.wave_signal.with_alpha(0.25));
            scene.rects(bars.solid, t.wave_signal);
            scene.rects(bars.failed, t.editor.error);
            scene.lines(bars.edges, t.wave_signal, 1.0);
            scene.lines(bars.failed_edges, t.editor.error, 1.0);
            for (origin, label) in bars.labels {
                scene.text(origin, bar_h, label, FontRole::Mono, font, t.editor.bg);
            }
            occupancy(scene, &index.occupancy, viewport, area, t);
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

fn outline(edges: &mut Vec<[Point; 2]>, rect: Rect) {
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
