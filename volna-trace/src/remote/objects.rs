//! Serializable complete objects, separate from reader internals and UI state.

use crate::data::loaded_tracks::{
    LoadedGenerator, LoadedRelation, LoadedTrack, TransactionLocation,
};
use crate::data::transactions::{Track, TrackRef, Transaction, TransactionRef};
use crate::data::{Hierarchy, TraceInfo};
use crate::session::{Capabilities, Session};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Metadata {
    pub info: TraceInfo,
    pub hierarchy: Hierarchy,
    pub capabilities: Capabilities,
    pub tracks: Vec<Track>,
    pub activity: Option<super::activity::Descriptor>,
    pub server: String,
}

impl Metadata {
    pub fn from_session(session: &dyn Session) -> Self {
        Self {
            info: session.info().clone(),
            hierarchy: session.hierarchy().clone(),
            capabilities: session.capabilities(),
            tracks: session.tracks().to_vec(),
            activity: super::activity::Descriptor::from_session(session),
            server: session.activity_host().unwrap_or("remote server").into(),
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let mut validation = std::pin::pin!(self.validate_with(|| std::future::ready(())));
        match std::future::Future::poll(
            validation.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop()),
        ) {
            std::task::Poll::Ready(result) => result,
            std::task::Poll::Pending => unreachable!("synchronous validation never yields"),
        }
    }

    pub(super) async fn validate_with<F: std::future::Future<Output = ()>>(
        &self,
        mut checkpoint: impl FnMut() -> F,
    ) -> anyhow::Result<()> {
        if let Some(activity) = &self.activity {
            activity.identity()?;
        }
        anyhow::ensure!(
            self.info.time_range.0 <= self.info.time_range.1,
            "reversed trace time range"
        );
        let h = &self.hierarchy;
        for (id, scope) in h.scopes().enumerate() {
            checkpoint().await;
            anyhow::ensure!(scope.parent.is_none_or(|p| p < id), "invalid scope parent");
        }
        let mut signals = HashMap::new();
        for var in h.vars() {
            checkpoint().await;
            anyhow::ensure!(var.scope < h.scope_count(), "invalid variable scope");
            anyhow::ensure!(
                (var.signal.0 as usize) < self.info.signal_count,
                "invalid signal reference"
            );
            anyhow::ensure!(
                !matches!(var.shape, crate::data::SignalShape::Vector { width: 0 | 1 }),
                "invalid vector width"
            );
            if let Some(shape) = signals.insert(var.signal, var.shape) {
                anyhow::ensure!(shape == var.shape, "inconsistent alias shape");
            }
        }
        let mut tracks = HashMap::new();
        for track in &self.tracks {
            checkpoint().await;
            anyhow::ensure!(
                tracks.insert(track.id, &track.kind).is_none(),
                "duplicate track identity"
            );
        }
        let mut generators = HashSet::new();
        let mut hierarchy_tracks = HashSet::new();
        for (id, scope) in h.scopes().enumerate() {
            checkpoint().await;
            for g in scope.generators {
                checkpoint().await;
                anyhow::ensure!(generators.insert(g), "duplicate generator declaration");
                let g = h
                    .generators
                    .get(g)
                    .ok_or_else(|| anyhow::anyhow!("invalid generator reference"))?;
                anyhow::ensure!(g.scope == id, "inconsistent generator parent");
                anyhow::ensure!(
                    hierarchy_tracks.insert(g.track),
                    "duplicate hierarchy track"
                );
                anyhow::ensure!(
                    tracks.get(&g.track).is_some_and(|kind| **kind == g.kind),
                    "invalid hierarchy generator"
                );
            }
        }
        anyhow::ensure!(
            generators.len() == h.generators.len(),
            "unreachable generators"
        );
        anyhow::ensure!(
            self.capabilities.transactions || self.tracks.is_empty(),
            "tracks without transaction capability"
        );
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct GeneratorPayload {
    #[serde(skip)]
    pub(super) reservation: Option<super::memory::Reservation>,
    pub generator: TrackRef,
    pub transactions: Vec<Transaction>,
    pub parents: Vec<(TransactionRef, TransactionLocation)>,
    pub relations: Vec<LoadedRelation>,
}

#[derive(Debug, Serialize)]
pub struct TrackPayload {
    pub track: TrackRef,
    pub generators: Vec<GeneratorPayload>,
}

impl TrackPayload {
    #[must_use]
    pub fn from_loaded(track: &LoadedTrack) -> impl Serialize + '_ {
        #[derive(Serialize)]
        struct Generator<'a> {
            generator: TrackRef,
            transactions: &'a [Transaction],
            parents: Vec<(TransactionRef, TransactionLocation)>,
            relations: &'a [LoadedRelation],
        }
        #[derive(Serialize)]
        struct Track<'a> {
            track: TrackRef,
            generators: Vec<Generator<'a>>,
        }
        Track {
            track: track.track,
            generators: track
                .generators
                .iter()
                .map(|generator| Generator {
                    generator: generator.generator(),
                    transactions: generator.transactions(),
                    parents: generator
                        .transactions()
                        .iter()
                        .filter_map(|tx| generator.parent(tx.id).map(|p| (tx.id, p)))
                        .collect(),
                    relations: generator.relations(),
                })
                .collect(),
        }
    }

    pub(super) async fn into_loaded<F: std::future::Future<Output = ()>>(
        self,
        catalog: &[Track],
        mut checkpoint: impl FnMut() -> F,
    ) -> anyhow::Result<LoadedTrack> {
        let mut declaration = None;
        let mut expected = HashSet::new();
        let mut catalog_generators = HashSet::new();
        for track in catalog {
            checkpoint().await;
            if track.id == self.track {
                declaration = Some(track);
            }
            catalog_generators.insert(track.id);
        }
        declaration.ok_or_else(|| anyhow::anyhow!("unknown loaded track"))?;
        expected.insert(self.track);
        let mut seen = HashSet::new();
        let mut result = vec![];
        for payload in self.generators {
            checkpoint().await;
            anyhow::ensure!(
                expected.contains(&payload.generator) && seen.insert(payload.generator),
                "unexpected or duplicate generator"
            );
            let mut parents = HashMap::new();
            for (child, parent) in payload.parents {
                checkpoint().await;
                anyhow::ensure!(
                    parents.insert(child, parent).is_none(),
                    "duplicate parent reference"
                );
            }
            let mut generator = LoadedGenerator::from_sorted(
                payload.generator,
                payload.transactions,
                parents,
                payload.relations,
                &mut checkpoint,
            )
            .await?;
            if let Some(mut reservation) = payload.reservation {
                reservation.grow_object(
                    "transaction attribute catalogue",
                    generator.attribute_catalogue_bytes(),
                )?;
                generator.reservation = Some(reservation);
            }
            result.push(Arc::new(generator));
        }
        anyhow::ensure!(seen == expected, "incomplete generator payload");
        let mut transactions = HashMap::new();
        let mut relations = HashMap::new();
        for generator in &result {
            for tx in generator.transactions() {
                checkpoint().await;
                for id in std::iter::once(tx.id)
                    .chain(tx.events.iter().map(|event| event.id))
                    .chain(tx.stages.iter().map(|stage| stage.id))
                {
                    anyhow::ensure!(
                        transactions.insert(id, generator.generator()).is_none(),
                        "duplicate record across generators"
                    );
                }
            }
        }
        let endpoint = |id: TransactionRef, owner: Option<TrackRef>| -> anyhow::Result<()> {
            let Some(owner) = owner else {
                anyhow::ensure!(
                    !transactions.contains_key(&id),
                    "known endpoint has no generator"
                );
                return Ok(());
            };
            anyhow::ensure!(
                catalog_generators.contains(&owner),
                "unknown endpoint generator"
            );
            if expected.contains(&owner) {
                anyhow::ensure!(
                    transactions.get(&id) == Some(&owner),
                    "missing or misplaced endpoint in complete track"
                );
            }
            Ok(())
        };
        for generator in &result {
            for tx in generator.transactions() {
                checkpoint().await;
                if let Some(parent) = generator.parent(tx.id) {
                    endpoint(parent.transaction, parent.generator)?;
                }
            }
            for edge in generator.relations() {
                checkpoint().await;
                endpoint(edge.relation.from, edge.from_generator)?;
                endpoint(edge.relation.to, edge.to_generator)?;
                if let Some(previous) = relations.insert(edge.id, edge) {
                    anyhow::ensure!(
                        super::equality::relation(previous, edge, &mut checkpoint).await,
                        "inconsistent relation identity"
                    );
                }
            }
        }
        Ok(LoadedTrack {
            track: self.track,
            generators: result,
        })
    }
}
