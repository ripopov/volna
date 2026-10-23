//! On-demand transaction ancestry.

use std::collections::HashSet;

use crate::data::text::{PREVIEW_BYTES, Radix, format_attribute_radix, join_path_limited};
use crate::document::{Document, TrackLoadState};
use crate::pipeline::model::LABEL_ATTRIBUTE;
use crate::trace::Traced;
use volna_trace::data::loaded_tracks::TransactionLocation;
use volna_trace::data::transactions::{TrackRef, TransactionRef};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ParentTraceState {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed(String),
}

pub struct ParentNode {
    pub track: Traced<TrackRef>,
    pub id: TransactionRef,
    pub label: String,
    pub path: String,
    _reservation: Reservation,
}

const NODE_BYTES: u64 = (2 * PREVIEW_BYTES
    + 4 * std::mem::size_of::<ParentNode>()
    + 8 * (std::mem::size_of::<TransactionLocation>() + std::mem::size_of::<Traced<TrackRef>>()))
    as u64;

#[derive(Default)]
pub struct ParentTrace {
    state: ParentTraceState,
    nodes: Vec<ParentNode>,
    seen: HashSet<TransactionLocation>,
    held: HashSet<Traced<TrackRef>>,
    next: Option<(Traced<TrackRef>, TransactionRef)>,
}

impl ParentTrace {
    #[must_use]
    pub fn state(&self) -> &ParentTraceState {
        &self.state
    }

    #[must_use]
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

    pub(crate) fn refresh(&mut self, doc: &mut Document, budget: &MemoryBudget) {
        if self.state != ParentTraceState::Loading {
            return;
        }
        while let Some((track, id)) = self.next {
            let location = TransactionLocation {
                transaction: id,
                generator: Some(track.item),
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
            self.next = generator.parent(id).and_then(|parent| {
                parent
                    .generator
                    .map(|generator| (track.with(generator), parent.transaction))
            });
        }
        self.state = ParentTraceState::Ready;
    }
}
