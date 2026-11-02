//! Bounded instruction-stream sampling, picking, and lifetime-relative time navigation.

use super::{row_set::RowSet, rows::RowView};
use crate::{
    geometry::{Rect, point, size},
    scene::{Prim, Scene},
    theme::Theme,
    wave::viewport::Viewport,
};
use volna_trace::data::transactions::{Status, Transaction};

pub const MIN_WIDTH: f32 = 32.0;
pub const MAX_WIDTH: f32 = 512.0;
const LATENCY_MAGNIFICATION: f64 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NavigatorSettings {
    pub visible: bool,
    pub width: f32,
}

impl Default for NavigatorSettings {
    fn default() -> Self {
        Self {
            visible: true,
            width: 64.0,
        }
    }
}

impl NavigatorSettings {
    #[must_use]
    pub fn width(self) -> f32 {
        if self.visible {
            self.width.clamp(MIN_WIDTH, MAX_WIDTH)
        } else {
            0.0
        }
    }

    pub(crate) fn valid(self) -> bool {
        self.width.is_finite() && (MIN_WIDTH..=MAX_WIDTH).contains(&self.width)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigatorCommand {
    Toggle,
    Widen,
    Narrow,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Navigator {
    pub rect: Rect,
    rows: usize,
    pixels: usize,
    samples: usize,
}

impl Navigator {
    #[must_use]
    pub fn new(rect: Rect, rows: usize) -> Self {
        let pixels = rect.height().floor().max(0.0) as usize;
        let samples = if rect.width() > 0.0 {
            rows.min(pixels)
        } else {
            0
        };
        Self {
            rect,
            rows,
            pixels,
            samples,
        }
    }

    fn row(self, sample: usize) -> usize {
        (((sample as u128 * 2 + 1) * self.rows as u128) / (self.samples as u128 * 2)) as usize
    }

    fn band(self, sample: usize) -> Rect {
        let edge = |s: usize| (s as u128 * self.pixels as u128 / self.samples as u128) as f32;
        let top = edge(sample);
        Rect::new(
            point(self.rect.left(), self.rect.top() + top),
            size(self.rect.width(), edge(sample + 1) - top),
        )
    }

    #[must_use]
    pub fn row_at(self, y: f32) -> Option<usize> {
        if self.samples == 0 || !y.is_finite() {
            return None;
        }
        let pixel = (y - self.rect.top())
            .floor()
            .max(0.0)
            .min((self.pixels - 1) as f32) as usize;
        let sample =
            ((pixel as u128 + 1) * self.samples as u128).div_ceil(self.pixels as u128) as usize - 1;
        Some(self.row(sample))
    }

    #[must_use]
    pub fn viewport(self, rows: RowView) -> Option<Rect> {
        if self.samples == 0 {
            return None;
        }
        let visible = f64::from(self.rect.height() / rows.row_px);
        let height = (self.rect.height() * (visible / self.rows as f64) as f32)
            .max(8.0)
            .min(self.rect.height());
        let maximum = (self.rows as f64 - visible).max(0.0);
        let fraction = if maximum > 0.0 {
            (rows.top / maximum).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some(Rect::new(
            point(
                self.rect.left(),
                self.rect.top() + (self.rect.height() - height) * fraction as f32,
            ),
            size(self.rect.width(), height),
        ))
    }

    pub(super) fn paint(
        self,
        set: &RowSet<'_>,
        rows: RowView,
        viewport: Viewport,
        cells_width: f64,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        if self.samples == 0 {
            return;
        }
        let units = (viewport.width() / cells_width * self.rows as f64
            / f64::from(self.rect.height())
            * f64::from(rows.row_px))
        .max(f64::MIN_POSITIVE);
        let mut normal = Vec::with_capacity(self.samples * 2);
        let mut aborted = Vec::new();
        for sample in 0..self.samples {
            let Some((_, tx)) = set.get(self.row(sample)) else {
                continue;
            };
            let band = self.band(sample);
            let width = f64::from(band.width());
            let start = (tx.begin as f64 / units).floor().rem_euclid(width) as f32;
            let length = ((tx.end.saturating_sub(tx.begin) as f64 * LATENCY_MAGNIFICATION / units)
                .ceil() as f32)
                .max(1.0)
                .min(band.width());
            let rects = if tx.status == Status::Aborted {
                let marker_width = (5.0 * theme.zoom).min(band.width());
                aborted.push(Rect::new(
                    point(band.right() - marker_width, band.top()),
                    size(marker_width, band.height()),
                ));
                &mut aborted
            } else {
                &mut normal
            };
            if length >= band.width() {
                rects.push(band);
            } else {
                let first = length.min(band.width() - start);
                rects.push(Rect::new(
                    point(band.left() + start, band.top()),
                    size(first, band.height()),
                ));
                if first < length {
                    rects.push(Rect::new(band.origin, size(length - first, band.height())));
                }
            }
        }
        let style = theme.navigator_style();
        let thumb = self.viewport(rows);
        scene.clipped(self.rect, |scene| {
            scene.fill(self.rect, style.background);
            if let Some(thumb) = thumb {
                scene.fill(thumb, style.viewport);
            }
            for (rects, color) in [(normal, style.stream), (aborted, style.aborted)] {
                if !rects.is_empty() {
                    scene.prims.push(Prim::Rects { rects, color });
                }
            }
            if let Some(thumb) = thumb {
                // The surface-colored halo separates the outline from any transaction ink.
                scene.quad(
                    thumb,
                    crate::color::Color::TRANSPARENT,
                    0.0,
                    3.0 * theme.zoom,
                    style.background,
                );
                let inset = theme
                    .zoom
                    .min(thumb.width() * 0.25)
                    .min(thumb.height() * 0.25);
                let outline = Rect::from_xywh(
                    thumb.left() + inset,
                    thumb.top() + inset,
                    thumb.width() - 2.0 * inset,
                    thumb.height() - 2.0 * inset,
                );
                scene.quad(
                    outline,
                    crate::color::Color::TRANSPARENT,
                    0.0,
                    theme.zoom,
                    style.edge,
                );
            }
        });
    }
}

pub(super) fn follow_shift(viewport: Viewport, from: &Transaction, to: &Transaction) -> f64 {
    // A virtual tick keeps phase mapping invertible when either record is instantaneous.
    let from_length = from.end.saturating_sub(from.begin).max(1) as f64;
    let to_length = to.end.saturating_sub(to.begin).max(1) as f64;
    let relative = viewport.start - from.begin as f64 + viewport.width() * 0.5;
    let begin_shift = (i128::from(to.begin) - i128::from(from.begin)) as f64;
    if relative <= 0.0 {
        begin_shift
    } else if relative >= from_length {
        begin_shift + to_length - from_length
    } else {
        begin_shift + relative * (to_length / from_length - 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::loaded_tracks::LoadedGenerator;
    use std::sync::Arc;
    use volna_trace::data::transactions::{SpanKind, TrackRef, TransactionRef};

    fn transaction(row: u64, begin: u64, end: u64) -> Transaction {
        Transaction {
            id: TransactionRef(row),
            generator: TrackRef(0),
            begin,
            end,
            status: if row.is_multiple_of(2) {
                Status::Ok
            } else {
                Status::Aborted
            },
            kind: SpanKind::Unspecified,
            parent: None,
            attributes: vec![],
            events: vec![],
            stages: vec![],
        }
    }

    #[test]
    fn picking_inverts_drawn_bands_at_sparse_and_dense_resolutions() {
        for count in [1, 3, 16, 1000, usize::MAX] {
            for height in [1.0, 7.5, 31.0] {
                let map = Navigator::new(Rect::from_xywh(20.0, 13.25, 64.0, height), count);
                for sample in 0..map.samples {
                    let band = map.band(sample);
                    assert!(band.height() >= 1.0);
                    assert!(map.row(sample) < count);
                    for offset in [0.0, 0.25, band.height() - 0.01] {
                        assert_eq!(map.row_at(band.top() + offset), Some(map.row(sample)));
                    }
                }
                assert_eq!(map.row_at(-1000.0), Some(map.row(0)));
                assert_eq!(map.row_at(1e6), Some(map.row(map.samples - 1)));
            }
        }
        assert_eq!(
            Navigator::new(Rect::from_xywh(0.0, 0.0, 64.0, 0.0), 10).row_at(0.0),
            None
        );
        assert_eq!(
            Navigator::new(Rect::from_xywh(0.0, 0.0, 64.0, 10.0), 0).row_at(0.0),
            None
        );
    }

    #[test]
    fn lifetime_mapping_is_reversible_before_during_and_after_execution() {
        let from = transaction(0, 100, 1000);
        for to in [transaction(1, 200, 220), transaction(2, 300, 300)] {
            for center in [50.0, 100.0, 550.0, 1000.0, 1050.0] {
                let view = Viewport {
                    start: center - 10.0,
                    end: center + 10.0,
                };
                let delta = follow_shift(view, &from, &to);
                let mapped = Viewport {
                    start: view.start + delta,
                    end: view.end + delta,
                };
                let reverse = follow_shift(mapped, &to, &from);
                assert!((delta + reverse).abs() < 1e-9, "{center}, {to:?}");
                assert_eq!(mapped.width(), view.width());
            }
        }
        let view = Viewport {
            start: 540.0,
            end: 560.0,
        };
        assert_eq!(
            view.start + 10.0 + follow_shift(view, &from, &transaction(1, 200, 220)),
            210.0
        );
        let late = 1 << 60;
        let from = transaction(0, late, late + 1000);
        let to = transaction(1, late + 500, late + 1500);
        assert_eq!(
            follow_shift(
                Viewport {
                    start: late as f64,
                    end: late as f64 + 4096.0
                },
                &from,
                &to
            ),
            500.0
        );
    }

    #[test]
    fn aborted_samples_have_a_right_edge_mark_independent_of_bar_length() {
        let generators = [Arc::new(
            LoadedGenerator::new(
                TrackRef(0),
                (0..3)
                    .map(|row| transaction(row, row * 100, row * 100))
                    .collect(),
                Default::default(),
                vec![],
            )
            .unwrap(),
        )];
        let rows = RowSet::new(&generators);
        let map = Navigator::new(Rect::from_xywh(0.0, 0.0, 64.0, 30.0), rows.len());
        for theme in [
            Theme::volna(true),
            Theme::builtin("dracula").unwrap(),
            Theme::volna(false),
        ] {
            let mut scene = Scene::default();
            map.paint(
                &rows,
                RowView::default(),
                Viewport {
                    start: 0.0,
                    end: 1000.0,
                },
                1000.0,
                &theme,
                &mut scene,
            );
            let aborted = theme.navigator_style().aborted;
            let marks: Vec<_> = scene
                .prims
                .iter()
                .filter_map(|prim| match prim {
                    Prim::Rects { rects, color } if *color == aborted => Some(rects),
                    _ => None,
                })
                .flatten()
                .filter(|rect| rect.width() > 1.0)
                .copied()
                .collect();
            assert_eq!(marks, [Rect::from_xywh(59.0, 10.0, 5.0, 10.0)]);
        }
    }

    #[test]
    fn painting_batches_height_bounded_output_independent_of_record_count() {
        for count in [32, 100_000] {
            let generators = [Arc::new(
                LoadedGenerator::new(
                    TrackRef(0),
                    (0..count)
                        .map(|row| transaction(row, row * 100, row * 100 + row % 7))
                        .collect(),
                    Default::default(),
                    vec![],
                )
                .unwrap(),
            )];
            let set = RowSet::new(&generators);
            for width in [0.5, 32.0, 128.0] {
                let map = Navigator::new(Rect::from_xywh(10.0, 20.0, width, 31.0), set.len());
                let mut scene = Scene::default();
                map.paint(
                    &set,
                    RowView::default(),
                    Viewport {
                        start: 0.0,
                        end: 1000.0,
                    },
                    1000.0,
                    &Theme::volna(true),
                    &mut scene,
                );
                let batches: Vec<_> = scene
                    .prims
                    .iter()
                    .filter_map(|p| match p {
                        Prim::Rects { rects, .. } => Some(rects),
                        _ => None,
                    })
                    .collect();
                assert!(batches.len() <= 2);
                assert!(scene.prims.len() <= 8);
                let rectangles: Vec<_> = batches.iter().flat_map(|rs| rs.iter()).collect();
                assert!(rectangles.len() <= 3 * map.samples);
                for rect in rectangles {
                    if count == 32 && width >= 32.0 && rect.right() < map.rect.right() {
                        assert!(rect.width() <= 3.0);
                    }
                    assert!(rect.width() > 0.0 && rect.height() > 0.0);
                    assert!(rect.left() >= map.rect.left() && rect.right() <= map.rect.right());
                    assert!(map.row_at(rect.top()).is_some());
                }
            }
        }
    }
}
