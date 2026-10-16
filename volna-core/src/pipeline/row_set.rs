//! Queries over a track's concatenated immutable generators.

use super::{activity::Activity, occupancy::raster_source, raster::PixelMap};
use crate::{data::loaded_tracks::LoadedGenerator, scene::PresenceMask, wave::viewport::Viewport};
use std::{ops::Range, sync::Arc};
use volna_trace::data::transactions::{TrackRef, Transaction, TransactionRef, TxStatus};

pub struct RowSet<'a> {
    generators: &'a [Arc<LoadedGenerator>],
    starts: Vec<usize>,
    len: usize,
}

impl<'a> RowSet<'a> {
    pub(super) fn new(generators: &'a [Arc<LoadedGenerator>]) -> Self {
        let mut starts = Vec::with_capacity(generators.len());
        let mut len = 0;
        for g in generators {
            starts.push(len);
            len += g.transactions().len();
        }
        Self {
            generators,
            starts,
            len,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn generators(&self) -> &'a [Arc<LoadedGenerator>] {
        self.generators
    }

    #[must_use]
    pub fn row_of(&self, generator: TrackRef, id: TransactionRef) -> Option<usize> {
        let g = self
            .generators
            .iter()
            .position(|c| c.generator() == generator)?;
        Some(self.starts[g] + self.generators[g].transaction_ordinal(id)?)
    }

    #[must_use]
    pub fn get(&self, row: usize) -> Option<(usize, &'a Transaction)> {
        if row >= self.len {
            return None;
        }
        let g = self.starts.partition_point(|&start| start <= row) - 1;
        Some((g, &self.generators[g].transactions()[row - self.starts[g]]))
    }
}

impl RowSet<'_> {
    pub(super) fn presence(&self, map: &PixelMap) -> PresenceMask {
        let mut mask = map.mask();
        for (&offset, generator) in self.starts.iter().zip(self.generators) {
            if let Some(index) = &generator.row_index {
                index.raster(generator.transactions(), offset, map, &mut mask);
            } else {
                raster_source(generator.transactions(), offset, map, &mut mask);
            }
        }
        mask
    }

    pub(super) fn aborted_presence(&self, map: &PixelMap) -> PresenceMask {
        let mut mask = map.aborted_mask();
        for (&offset, generator) in self.starts.iter().zip(self.generators) {
            if let Some(index) = &generator.row_index {
                index.raster_aborted(generator.transactions(), offset, map, &mut mask);
            } else {
                for (row, tx) in generator.transactions().iter().enumerate() {
                    if tx.status == TxStatus::Aborted {
                        map.aborted(offset + row, &mut mask);
                    }
                }
            }
        }
        mask
    }

    pub(super) fn activity(&self, view: Viewport, visible: Range<f64>, anchor: f64) -> Activity {
        let mut result = Activity::default();
        let center = visible.start.midpoint(visible.end);
        let mut best = (f64::INFINITY, f64::INFINITY);
        for (&offset, generator) in self.starts.iter().zip(self.generators) {
            let records = generator.transactions();
            result.earlier |= records
                .first()
                .is_some_and(|tx| (tx.begin as f64) < view.start);
            result.later |= records
                .last()
                .is_some_and(|tx| (tx.begin as f64) > view.end);
            if let Some(index) = &generator.row_index {
                let activity =
                    index.activity(records, offset, view, visible.clone(), anchor, center);
                result.above += activity.above;
                result.below += activity.below;
                result.visible += activity.visible;
                if activity.nearest_above.is_some() {
                    result.nearest_above = activity.nearest_above;
                }
                if result.nearest_below.is_none() {
                    result.nearest_below = activity.nearest_below;
                }
                if let Some(row) = activity.target {
                    let tx = &records[row - offset];
                    let score = (
                        (tx.begin as f64 - anchor)
                            .max(anchor - tx.end as f64)
                            .max(0.0),
                        (row as f64 + 0.5 - center).abs(),
                    );
                    if score < best {
                        best = score;
                        result.target = Some(row);
                    }
                }
                continue;
            }
            generator
                .visit_window_ordinals(
                    view.start.max(0.0).floor() as u64,
                    view.end.max(0.0).ceil() as u64,
                    |ordinal, tx| {
                        if (tx.end as f64) < view.start || (tx.begin as f64) > view.end {
                            return true;
                        }
                        let row = offset + ordinal;
                        if row as f64 + 1.0 <= visible.start {
                            result.above += 1;
                            result.nearest_above = Some(row);
                        } else if row as f64 >= visible.end {
                            result.below += 1;
                            result.nearest_below.get_or_insert(row);
                        } else {
                            result.visible += 1;
                        }
                        let distance = (tx.begin as f64 - anchor)
                            .max(anchor - tx.end as f64)
                            .max(0.0);
                        let score = (distance, (row as f64 + 0.5 - center).abs());
                        if score < best {
                            best = score;
                            result.target = Some(row);
                        }
                        true
                    },
                )
                .expect("ordered viewport");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::Rect,
        pipeline::{raster::oracle, rows::RowView},
    };
    use volna_trace::data::transactions::{TransactionStage, TxKind};

    #[test]
    fn queries_preserve_global_rows_with_mixed_indexes_and_empty_generators() {
        let make = |indexed: u8| {
            [0, 19, 0, 23]
                .into_iter()
                .enumerate()
                .map(|(g, count)| {
                    let records = (0..count)
                        .map(|row| Transaction {
                            id: TransactionRef(row),
                            generator: TrackRef(g as u32),
                            begin: row * 3,
                            end: row * 3 + 8,
                            status: if row % 3 == 0 {
                                TxStatus::Aborted
                            } else {
                                TxStatus::Ok
                            },
                            kind: TxKind::Unspecified,
                            parent: None,
                            attributes: vec![],
                            events: vec![],
                            stages: vec![TransactionStage {
                                name: "stage".into(),
                                lane: "0".into(),
                                begin: row * 3 + 10,
                                end: Some(row * 3 + 12),
                                attributes: vec![],
                            }],
                        })
                        .collect();
                    let mut generator = LoadedGenerator::new(
                        TrackRef(g as u32),
                        records,
                        Default::default(),
                        vec![],
                    )
                    .unwrap();
                    if indexed & (1 << g) == 0 {
                        generator.row_index = None;
                    }
                    Arc::new(generator)
                })
                .collect::<Vec<_>>()
        };
        let source = make(0);
        let source_rows = RowSet::new(&source);
        let records: Vec<_> = source
            .iter()
            .flat_map(|g| g.transactions().iter().cloned())
            .collect();
        let fields = |a: Activity| {
            (
                a.above,
                a.visible,
                a.below,
                a.nearest_above,
                a.nearest_below,
                a.target,
                a.earlier,
                a.later,
            )
        };
        for indexed in [0, 2, 8, 10] {
            let generators = make(indexed);
            let rows = RowSet::new(&generators);
            for top in [0.25, 17.5, 21.75] {
                let view = Viewport {
                    start: 11.25,
                    end: 52.75,
                };
                let map = PixelMap::new(
                    Rect::from_xywh(0.37, 0.23, 18.0, 4.0),
                    view,
                    RowView { top, row_px: 0.3 },
                    1.25,
                );
                assert_eq!(rows.presence(&map), oracle(&records, 0, &map));
                assert_eq!(
                    rows.aborted_presence(&map),
                    source_rows.aborted_presence(&map)
                );
                assert_eq!(
                    fields(rows.activity(view, top..top + 12.0, 30.0)),
                    fields(source_rows.activity(view, top..top + 12.0, 30.0))
                );
            }
        }
    }
}
