//! On-demand transaction ancestry. Tracks stay retained until the graph is
//! cleared, so loading another panel cannot evict a parent during traversal.

use std::collections::HashSet;

use crate::data::text::{PREVIEW_BYTES, Radix, format_attribute_radix, join_path_limited};
use crate::document::{Document, TrackLoadState};
use crate::pipeline::model::LABEL_ATTRIBUTE;
use crate::trace::Traced;
use volna_trace::data::loaded_tracks::TransactionLocation;
use volna_trace::data::transactions::{TrackRef, TransactionRef};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

/// State of an explicitly requested parent graph; reading it never queues loads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ParentTraceState {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed(String),
}

/// One graph vertex. Captions are bounded independently of the chain length.
pub struct ParentNode {
    pub track: Traced<TrackRef>,
    pub id: TransactionRef,
    pub label: String,
    pub path: String,
    _reservation: Reservation,
}

// Pay for bounded captions, spare vector capacity, and both hash tables before
// adding a vertex. The recording's full generator data has its own admission.
const NODE_BYTES: u64 = (2 * PREVIEW_BYTES
    + 4 * std::mem::size_of::<ParentNode>()
    + 8 * (std::mem::size_of::<TransactionLocation>() + std::mem::size_of::<Traced<TrackRef>>()))
    as u64;

#[derive(Default)]
pub struct ParentTrace {
    state: ParentTraceState,
    /// Selected record first, then its parent, ending at the root.
    nodes: Vec<ParentNode>,
    seen: HashSet<TransactionLocation>,
    held: HashSet<Traced<TrackRef>>,
    next: Option<(Traced<TrackRef>, TransactionRef)>,
}

impl ParentTrace {
    pub fn state(&self) -> &ParentTraceState {
        &self.state
    }

    /// Child-to-root order; adjacent vertices are connected by a parent edge.
    pub fn nodes(&self) -> &[ParentNode] {
        &self.nodes
    }

    pub(crate) fn clear(&mut self, doc: &mut Document) {
        for track in self.held.drain() {
            doc.release_track(track);
        }
        *self = Self::default();
    }

    pub(crate) fn start(
        &mut self,
        doc: &mut Document,
        budget: &MemoryBudget,
        track: Traced<TrackRef>,
        id: TransactionRef,
    ) {
        self.clear(doc);
        self.state = ParentTraceState::Loading;
        self.next = Some((track, id));
        self.refresh(doc, budget);
    }

    fn fail(&mut self, doc: &mut Document, error: String) {
        self.clear(doc);
        self.state = ParentTraceState::Failed(error);
    }

    /// Continue through resident parents, stopping at the first pending load.
    /// Delivery resumes this walk; it never follows arbitrary relation edges.
    pub(crate) fn refresh(&mut self, doc: &mut Document, budget: &MemoryBudget) {
        if self.state != ParentTraceState::Loading {
            return;
        }
        while let Some((track, id)) = self.next {
            let location = TransactionLocation {
                transaction: id,
                generator: track.item,
            };
            if self.seen.contains(&location) {
                self.fail(doc, format!("Parent cycle at transaction #{}.", id.0));
                return;
            }
            let reservation = match budget.reserve(NODE_BYTES) {
                Ok(reservation) => reservation,
                Err(error) => {
                    self.fail(doc, format!("Cannot admit the parent graph: {error}"));
                    return;
                }
            };
            if !self.held.contains(&track) {
                if let Err(error) = doc.retain_track(track) {
                    self.fail(doc, format!("Cannot load parent #{}: {error}", id.0));
                    return;
                }
                doc.retry_track(track);
                self.held.insert(track);
            }
            let Some(generator) = doc.resident_generator(track) else {
                if let Some(TrackLoadState::Failed(error)) = doc.track(track) {
                    let error = format!("Cannot load parent #{}: {error}", id.0);
                    self.fail(doc, error);
                }
                return;
            };
            let Some(tx) = generator.transaction(id) else {
                self.fail(doc, format!("Parent transaction #{} is missing.", id.0));
                return;
            };
            let label = tx
                .attributes
                .iter()
                .find(|attribute| attribute.key == LABEL_ATTRIBUTE)
                .map(|attribute| {
                    format_attribute_radix(&attribute.value, Radix::Dec, PREVIEW_BYTES)
                })
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| "(no label)".into());
            let path = doc
                .session(track.trace)
                .and_then(|session| session.tracks().iter().find(|t| t.id == track.item))
                .map(|t| join_path_limited(&t.path, PREVIEW_BYTES, &mut false))
                .unwrap_or_default();
            self.nodes.push(ParentNode {
                track,
                id,
                label,
                path,
                _reservation: reservation,
            });
            self.seen.insert(location);
            self.next = generator
                .parent(id)
                .map(|parent| (track.with(parent.generator), parent.transaction));
        }
        self.state = ParentTraceState::Ready;
    }
}
