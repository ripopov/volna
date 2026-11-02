//! A virtual list over the prepared interval index; only requested previews are materialized.

use crate::{data::transaction_index::TransactionIndex, trace::Traced};
use std::{ops::Range, sync::Arc};
use volna_trace::data::transactions::{Status, TrackRef, TransactionRef};

pub enum CursorRecordsState {
    Hidden,
    Preparing,
    Unavailable,
    Ready(CursorRecords),
}

impl CursorRecordsState {
    #[must_use]
    pub fn ready(self) -> Option<CursorRecords> {
        match self {
            Self::Ready(records) => Some(records),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct CursorRecords {
    pub track: Traced<TrackRef>,
    pub time: u64,
    pub count: usize,
    pub selected_rank: Option<usize>,
    pub selected: Option<TransactionRef>,
    index: Arc<TransactionIndex>,
}

pub struct CursorRecord {
    pub id: TransactionRef,
    pub label: String,
    pub begin: u64,
    pub end: u64,
    pub failed: bool,
}

impl CursorRecords {
    #[must_use]
    pub fn new(
        index: Arc<TransactionIndex>,
        track: Traced<TrackRef>,
        time: u64,
        selected: Option<TransactionRef>,
    ) -> Self {
        let generator = index.source();
        let selected = selected.map(|id| generator.transaction(id).map_or(id, |record| record.id));
        Self {
            track,
            time,
            count: index.active_count(time),
            selected_rank: selected
                .and_then(|id| generator.transaction_ordinal(id))
                .and_then(|i| index.active_rank(time, i)),
            selected,
            index,
        }
    }

    #[must_use]
    pub fn page(&self, range: Range<usize>) -> Vec<CursorRecord> {
        let records = self.index.source().transactions();
        self.index
            .active_page(self.time, range)
            .into_iter()
            .map(|i| {
                let tx = &records[i];
                let label = crate::pipeline::PipelineModel::label(tx);
                CursorRecord {
                    id: tx.id,
                    label: crate::data::text::truncate_ref(label, 256),
                    begin: tx.begin,
                    end: tx.end,
                    failed: tx.status == Status::Error,
                }
            })
            .collect()
    }
}
