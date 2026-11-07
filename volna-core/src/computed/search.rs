//! Bounded recorded-source search over immutable session snapshots.

use crate::{
    Document,
    trace::{TraceId, Traced},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use volna_trace::{
    data::{SignalShape, VarId},
    session::Session,
};

const MAX_MATCHES: usize = 20;

pub struct RecordedMatch {
    pub var: Traced<VarId>,
    pub label: String,
}
pub struct RecordedSearch {
    sessions: Vec<(TraceId, Arc<dyn Session>)>,
    query: String,
    sampling: bool,
}
impl RecordedSearch {
    #[must_use]
    pub fn new(doc: &Document, query: &str) -> Self {
        Self {
            sessions: doc
                .traces()
                .loaded()
                .map(|(id, session)| (id, session.clone()))
                .collect(),
            query: query.to_lowercase(),
            sampling: false,
        }
    }
    #[must_use]
    pub fn sampling_only(mut self) -> Self {
        self.sampling = true;
        self
    }

    #[must_use]
    pub fn run(self, cancelled: &AtomicBool) -> Vec<RecordedMatch> {
        let mut matches = Vec::new();
        for (trace, session) in &self.sessions {
            let h = session.hierarchy();
            for scope in 0..h.scope_count() {
                if cancelled.load(Ordering::Relaxed) {
                    return Vec::new();
                }
                let path = h.scope_path(scope).join(".");
                let prefix = format!("{trace}: {path}.");
                for var in h.scope(scope).vars {
                    if cancelled.load(Ordering::Relaxed) {
                        return Vec::new();
                    }
                    let v = h.var(var);
                    if self.sampling
                        && !crate::sampling::SamplingMode::ValueChanges.accepts(v.shape)
                    {
                        continue;
                    }
                    if !matches!(
                        v.shape,
                        SignalShape::Bit | SignalShape::Vector { width: 2.. } | SignalShape::Real
                    ) {
                        continue;
                    }
                    let mut label = prefix.clone();
                    label.push_str(v.name);
                    if !label.to_lowercase().contains(&self.query) {
                        continue;
                    }
                    // Ordinal lookup scans siblings, so perform it only for the bounded result set.
                    if let Some(nth) = h.var_path(var).1 {
                        use std::fmt::Write;
                        write!(label, " (occurrence {})", nth + 1).expect("write to String");
                    }
                    matches.push(RecordedMatch {
                        var: Traced::new(*trace, var),
                        label,
                    });
                    if matches.len() == MAX_MATCHES {
                        return matches;
                    }
                }
            }
        }
        matches
    }
}
