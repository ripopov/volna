//! Timeline presentation of interval results; measurement metadata stays on the row.

use crate::computed::MeasurementResult;
use std::sync::Arc;
use volna_trace::data::{Bit, SignalHistory, SignalShape, WaveValue};

enum MeasurementHistory {
    Scalar([(u64, Option<f64>); 2]),
    Table(Arc<crate::computed::MeasurementTable>),
}

impl MeasurementHistory {
    fn events(&self) -> &[(u64, Option<f64>)] {
        match self {
            Self::Scalar(events) => events,
            Self::Table(rows) => rows.timeline(),
        }
    }
}

pub(super) fn history(result: &MeasurementResult) -> Arc<dyn SignalHistory> {
    Arc::new(match result {
        MeasurementResult::Scalar(value) => {
            MeasurementHistory::Scalar([(value.bounds.0, value.value), (value.bounds.1, None)])
        }
        MeasurementResult::Table(rows) => MeasurementHistory::Table(rows.clone()),
    })
}

impl SignalHistory for MeasurementHistory {
    fn shape(&self) -> SignalShape {
        SignalShape::Real
    }
    fn len(&self) -> usize {
        self.events().len()
    }
    fn time(&self, index: usize) -> u64 {
        self.events()[index].0
    }
    fn value(&self, index: Option<usize>) -> WaveValue {
        index
            .and_then(|index| self.events().get(index))
            .and_then(|(_, value)| *value)
            .map_or(WaveValue::Unavailable, WaveValue::Real)
    }
    fn bit(&self, _: Option<usize>) -> Bit {
        Bit::Unavailable
    }
}
