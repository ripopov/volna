//! Stage bands shared by painting and hit testing.

use volna_trace::data::transactions::Transaction;

use super::PipelineModel;

pub(super) fn row_padding(row_px: f32, zoom: f32) -> f32 {
    if row_px >= 8.0 {
        1.5 * zoom
    } else if row_px >= 4.0 {
        0.5 * zoom
    } else {
        0.0
    }
}

pub(super) fn overlay_band(row_y: f32, row_px: f32, zoom: f32) -> std::ops::Range<f32> {
    let top = row_y + row_px * 0.55;
    let bottom = row_y + row_px - row_padding(row_px, zoom);
    top..bottom.max(top + 1.0)
}

#[derive(Clone, Copy)]
pub(super) struct StageBand {
    pub index: usize,
    pub count: usize,
}

pub(super) fn stage_bands(tx: &Transaction, primary: &str) -> Vec<StageBand> {
    let mut bands = vec![StageBand { index: 0, count: 1 }; tx.stages.len()];
    let mut order: Vec<_> = tx
        .stages
        .iter()
        .enumerate()
        .filter(|(_, stage)| stage.lane == primary)
        .map(|(index, _)| index)
        .collect();
    order.sort_by_key(|&index| (tx.stages[index].begin, index));
    // A connected overlap group keeps one subdivision while panning through it.
    let mut ends = Vec::new();
    let mut group: Vec<usize> = Vec::new();
    let mut group_end = 0;
    for index in order {
        let stage = &tx.stages[index];
        if stage.begin >= group_end {
            for member in group.drain(..) {
                bands[member].count = ends.len();
            }
            ends.clear();
        }
        let end = PipelineModel::stage_end(tx, stage);
        let band = ends
            .iter()
            .position(|&end| end <= stage.begin)
            .unwrap_or(ends.len());
        if band == ends.len() {
            ends.push(end);
        } else {
            ends[band] = end;
        }
        bands[index].index = band;
        group.push(index);
        group_end = end.max(group_end);
    }
    for member in group {
        bands[member].count = ends.len();
    }
    bands
}
