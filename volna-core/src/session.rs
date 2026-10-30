//! Viewer work: raw trace loads and client-side analysis, tagged by document generation.

use crate::trace::{TraceId, Traced};
use std::sync::Arc;
use volna_trace::data::SignalRef;
use volna_trace::remote::memory::MemoryBudget;
use volna_trace::session::{
    LoadRequest as TraceRequest, LoadResult as TraceResult, OpenSpec, Session, SignalLoads,
};

pub enum LoadRequest {
    Trace(TraceRequest<TraceId>),
    Projection {
        request: u64,
        source: Arc<dyn volna_trace::data::SignalHistory>,
        slice: crate::data::projection::BitSlice,
        budget: volna_trace::remote::memory::MemoryBudget,
    },
    Open {
        trace: TraceId,
        generation: u64,
        spec: OpenSpec,
    },
    Sizes {
        trace: TraceId,
        generation: u64,
        session: Arc<dyn Session>,
    },
    ActivityCounter {
        trace: TraceId,
        generation: u64,
        session: Arc<dyn Session>,
        budget: volna_trace::remote::memory::MemoryBudget,
    },
    Activity {
        trace: TraceId,
        generation: u64,
        window: (u64, u64),
        index: Arc<volna_trace::data::ActivityIndex>,
        counter: Arc<crate::data::ActivityCounter>,
        budget: volna_trace::remote::memory::MemoryBudget,
    },
    Summary {
        generation: u64,
        signal: Traced<SignalRef>,
        history: Arc<dyn volna_trace::data::SignalHistory>,
        kind: crate::data::NumericKind,
        budget: volna_trace::remote::memory::MemoryBudget,
    },
    GroupSummary {
        generation: u64,
        members: Vec<Arc<dyn volna_trace::data::SignalHistory>>,
        range: (u64, u64),
        budget: volna_trace::remote::memory::MemoryBudget,
    },
    Integral {
        generation: u64,
        history: Arc<dyn volna_trace::data::SignalHistory>,
        reading: crate::wave::stack::Reading,
        budget: volna_trace::remote::memory::MemoryBudget,
    },
    StackTotal {
        generation: u64,
        layers: Vec<crate::wave::stack::Layer>,
        range: (u64, u64),
        budget: volna_trace::remote::memory::MemoryBudget,
    },
}

impl LoadRequest {
    #[must_use]
    pub fn remote_id(&self) -> Option<u64> {
        match self {
            Self::Trace(request) => request.remote_id(),
            Self::Projection { .. }
            | Self::Open { .. }
            | Self::Sizes { .. }
            | Self::ActivityCounter { .. }
            | Self::Activity { .. }
            | Self::Summary { .. }
            | Self::GroupSummary { .. }
            | Self::Integral { .. }
            | Self::StackTotal { .. } => None,
        }
    }

    #[must_use]
    pub fn fail(self, error: anyhow::Error) -> LoadResult {
        match self {
            Self::Trace(request) => LoadResult::from_trace(request.fail(error), None),
            Self::Projection { request, .. } => LoadResult::Projection {
                request,
                result: Err(error),
            },
            Self::Open {
                trace, generation, ..
            } => LoadResult::Opened {
                trace,
                generation,
                result: Err(error),
            },
            Self::Sizes {
                trace, generation, ..
            } => LoadResult::Sizes {
                trace,
                generation,
                result: Err(error),
            },
            Self::ActivityCounter {
                trace, generation, ..
            } => LoadResult::ActivityCounter {
                trace,
                generation,
                result: Err(error),
            },
            Self::Activity {
                trace,
                generation,
                window,
                ..
            } => LoadResult::Activity {
                trace,
                generation,
                window,
                result: Err(error),
            },
            Self::Summary {
                generation,
                signal,
                history,
                kind,
                ..
            } => LoadResult::Summary {
                generation,
                signal,
                kind,
                history: crate::wave::analog::history_identity(&history),
                result: Err(error),
            },
            Self::GroupSummary {
                generation,
                members,
                ..
            } => LoadResult::GroupSummary {
                generation,
                key: crate::wave::group::key(&members),
                result: Err(error),
            },
            Self::Integral {
                generation,
                history,
                reading,
                ..
            } => LoadResult::Integral {
                generation,
                history: crate::wave::analog::history_identity(&history),
                reading,
                result: Err(error),
            },
            Self::StackTotal {
                generation, layers, ..
            } => LoadResult::StackTotal {
                generation,
                key: crate::wave::stack::key(&layers),
                result: Err(error),
            },
        }
    }

    pub fn perform(self) -> LoadResult {
        match self {
            Self::Trace(request) => {
                let budget = match &request {
                    TraceRequest::Track { session, .. } => session.memory_budget(),
                    _ => None,
                };
                LoadResult::from_trace(request.perform(), budget.as_ref())
            }
            Self::Projection {
                request,
                source,
                slice,
                budget,
            } => LoadResult::Projection {
                request,
                result: crate::data::projection::ProjectedHistory::build(source, slice, &budget)
                    .map(|h| Arc::new(h) as Arc<dyn volna_trace::data::SignalHistory>),
            },
            LoadRequest::Open {
                trace,
                generation,
                spec,
            } => LoadResult::Opened {
                trace,
                generation,
                result: spec.open(),
            },
            LoadRequest::Sizes {
                trace,
                generation,
                session,
            } => LoadResult::Sizes {
                trace,
                generation,
                result: Ok(Arc::new(volna_trace::data::ScopeSizes::count(
                    session.hierarchy(),
                ))),
            },
            LoadRequest::ActivityCounter {
                trace,
                generation,
                session,
                budget,
            } => LoadResult::ActivityCounter {
                trace,
                generation,
                result: crate::data::ActivityCounter::build(session.hierarchy(), &budget)
                    .map(Arc::new),
            },
            LoadRequest::Activity {
                trace,
                generation,
                window,
                index,
                counter,
                budget,
            } => LoadResult::Activity {
                trace,
                generation,
                window,
                result: crate::data::ActivityCounts::classify(&index, &counter, window, &budget)
                    .map(Arc::new),
            },
            LoadRequest::Summary {
                generation,
                signal,
                history,
                kind,
                budget,
            } => LoadResult::Summary {
                generation,
                signal,
                kind,
                history: crate::wave::analog::history_identity(&history),
                result: crate::wave::analog::AnalogSummary::build(&history, kind, &budget)
                    .map(Arc::new),
            },
            LoadRequest::GroupSummary {
                generation,
                members,
                range,
                budget,
            } => {
                let key = crate::wave::group::key(&members);
                let summary = crate::wave::group::GroupSummary::build(&members, range, &budget);
                LoadResult::GroupSummary {
                    generation,
                    key,
                    result: summary.map(Arc::new),
                }
            }
            LoadRequest::Integral {
                generation,
                history,
                reading,
                budget,
            } => LoadResult::Integral {
                generation,
                history: crate::wave::analog::history_identity(&history),
                reading,
                result: crate::wave::stack::IntegralSummary::build(&history, reading, &budget)
                    .map(Arc::new),
            },
            LoadRequest::StackTotal {
                generation,
                layers,
                range,
                budget,
            } => {
                let key = crate::wave::stack::key(&layers);
                let summary = crate::wave::stack::TotalSummary::build(&layers, range, &budget);
                LoadResult::StackTotal {
                    generation,
                    key,
                    result: summary.map(Arc::new),
                }
            }
        }
    }
}

pub enum LoadResult {
    Projection {
        request: u64,
        result: anyhow::Result<Arc<dyn volna_trace::data::SignalHistory>>,
    },
    ActivityBuilt {
        trace: TraceId,
        generation: u64,
        result: anyhow::Result<()>,
    },
    Track {
        trace: TraceId,
        generation: u64,
        request_id: u64,
        track: volna_trace::data::transactions::TrackRef,
        result: anyhow::Result<crate::data::loaded_tracks::LoadedTrack>,
    },
    Signals {
        trace: TraceId,
        generation: u64,
        results: SignalLoads,
    },
    Opened {
        trace: TraceId,
        generation: u64,
        result: anyhow::Result<Arc<dyn Session>>,
    },
    Sizes {
        trace: TraceId,
        generation: u64,
        result: anyhow::Result<Arc<volna_trace::data::ScopeSizes>>,
    },
    ActivityCounter {
        trace: TraceId,
        generation: u64,
        result: anyhow::Result<Arc<crate::data::ActivityCounter>>,
    },
    Activity {
        trace: TraceId,
        generation: u64,
        window: (u64, u64),
        result: anyhow::Result<Arc<crate::data::ActivityCounts>>,
    },
    Summary {
        generation: u64,
        signal: Traced<SignalRef>,
        kind: crate::data::NumericKind,
        history: usize,
        result: anyhow::Result<Arc<crate::wave::analog::AnalogSummary>>,
    },
    GroupSummary {
        generation: u64,
        key: Vec<usize>,
        result: anyhow::Result<Arc<crate::wave::group::GroupSummary>>,
    },
    Integral {
        generation: u64,
        history: usize,
        reading: crate::wave::stack::Reading,
        result: anyhow::Result<Arc<crate::wave::stack::IntegralSummary>>,
    },
    StackTotal {
        generation: u64,
        key: crate::wave::stack::Key,
        result: anyhow::Result<Arc<crate::wave::stack::TotalSummary>>,
    },
}

impl LoadResult {
    pub(crate) fn from_trace(raw: TraceResult<TraceId>, budget: Option<&MemoryBudget>) -> Self {
        match raw {
            TraceResult::Opened {
                tag,
                generation,
                result,
            } => LoadResult::Opened {
                trace: tag,
                generation,
                result,
            },
            TraceResult::Signals {
                tag,
                generation,
                results,
            } => LoadResult::Signals {
                trace: tag,
                generation,
                results,
            },
            TraceResult::Track {
                tag,
                generation,
                request_id,
                track,
                result,
            } => LoadResult::Track {
                trace: tag,
                generation,
                request_id,
                track,
                result: result
                    .and_then(|raw| crate::data::loaded_tracks::LoadedTrack::prepare(raw, budget)),
            },
            TraceResult::ActivityBuilt {
                tag,
                generation,
                result,
            } => LoadResult::ActivityBuilt {
                trace: tag,
                generation,
                result,
            },
        }
    }
}
