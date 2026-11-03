//! Average concurrency and peak envelopes shared by transaction and pipeline views.

use crate::{
    geometry::{Rect, point},
    scene::{FontRole, Scene},
    theme::Theme,
    wave::viewport::Viewport,
};

const PIXELS: usize = 4096;

pub(crate) fn paint(
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
    let minimum = z(1.0).min(height);
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
        let h = (height * (sample.average / peak) as f32).max(minimum);
        append(&mut fills, Rect::from_xywh(left, bottom - h, w, h));
        let y = bottom - (height * (sample.peak as f64 / peak) as f32).max(minimum);
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
        for rect in &fills {
            points.push(rect.origin);
            points.push(point(rect.right(), rect.top()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{data::occupancy::Occupancy, scene::Prim};
    use volna_trace::data::transactions::{
        SpanKind, Status, TrackRef, Transaction, TransactionRef,
    };

    #[test]
    fn zero_occupancy_draws_a_continuous_minimum_through_empty_windows_and_gaps() {
        let records: Vec<_> = [(2, 4), (6, 8)]
            .into_iter()
            .enumerate()
            .map(|(id, (begin, end))| Transaction {
                id: TransactionRef(id as u64),
                generator: TrackRef(1),
                begin,
                end,
                status: Status::Ok,
                kind: SpanKind::Unspecified,
                parent: None,
                attributes: vec![],
                events: vec![],
                stages: vec![],
            })
            .collect();
        for records in [&[][..], records.as_slice()] {
            let summary = Occupancy::build(records, None).unwrap();
            for dark in [true, false] {
                for zoom in [1.0, 2.0] {
                    let theme = Theme::volna(dark).zoomed(zoom);
                    let area = Rect::from_xywh(10.0, 20.0, 100.0, 80.0 * zoom);
                    let viewport = Viewport {
                        start: 0.0,
                        end: 10.0,
                    };
                    let mut scene = Scene::default();
                    paint(&mut scene, &summary, &viewport, area, &theme);
                    let minimum_y = area.bottom() - 4.0 * zoom;
                    let segments = scene
                        .prims
                        .iter()
                        .find_map(|p| match p {
                            Prim::Lines { segments, .. } => Some(segments),
                            _ => None,
                        })
                        .expect("peak outline remains visible");
                    let fill = scene
                        .prims
                        .iter()
                        .find_map(|p| match p {
                            Prim::Polygon { points, .. } => Some(points),
                            _ => None,
                        })
                        .expect("average remains visible");
                    for offset in [0.5, 45.5, 99.5] {
                        let x = area.left() + offset;
                        assert!(segments.iter().any(|[a, b]| a.x <= x
                            && b.x >= x
                            && a.y == minimum_y
                            && b.y == minimum_y));
                        assert!(fill.windows(2).any(|pair| pair[0].x <= x
                            && pair[1].x >= x
                            && pair[0].y == minimum_y
                            && pair[1].y == minimum_y));
                    }
                    if !records.is_empty() {
                        let x = area.left() + 30.5;
                        assert!(segments.iter().any(|[a, b]| a.x <= x
                            && b.x >= x
                            && a.y < minimum_y
                            && b.y < minimum_y));
                    }
                    assert_eq!(summary.sample(4.0, 6.0).average, 0.0);
                    assert_eq!(summary.sample(4.0, 6.0).peak, 0);
                }
            }
        }
    }
}
