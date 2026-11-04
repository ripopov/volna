//! Job ownership cancels abandoned work; request identities reject late completions.

use super::{Interpretation, MeanHistory, Settings};
use crate::{session::LoadRequest, trace::Traced};
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use volna_trace::{
    data::{SignalHistory, SignalRef},
    remote::memory::MemoryBudget,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub generation: u64,
    pub signal: Traced<SignalRef>,
    pub settings: Settings,
    pub interpretation: Interpretation,
}

pub enum Status {
    Waiting,
    Building,
    Ready(Arc<MeanHistory>),
    Failed(String),
}

struct Job {
    request: u64,
    cancelled: Arc<AtomicBool>,
    status: Status,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub(crate) struct Computations {
    jobs: HashMap<Key, Job>,
    sources: HashMap<Traced<SignalRef>, Result<Arc<dyn SignalHistory>, String>>,
    next_request: u64,
    generation: Option<u64>,
    wanted_sources: HashSet<Traced<SignalRef>>,
}

impl Computations {
    pub fn clear(&mut self) {
        self.jobs.clear();
        self.sources.clear();
        self.wanted_sources.clear();
    }

    pub fn retain(&mut self, wanted: &HashSet<Key>) {
        self.wanted_sources = wanted.iter().map(|k| k.signal).collect();
        self.jobs.retain(|key, _| wanted.contains(key));
        self.sources
            .retain(|signal, _| self.wanted_sources.contains(signal));
    }

    pub fn needs_source(&self, signal: Traced<SignalRef>) -> bool {
        !self.sources.contains_key(&signal)
    }

    pub fn status(&self, key: &Key) -> Option<&Status> {
        self.jobs.get(key).map(|j| &j.status)
    }

    pub fn source(&self, signal: Traced<SignalRef>) -> Option<&Arc<dyn SignalHistory>> {
        self.sources.get(&signal)?.as_ref().ok()
    }

    pub fn deliver_source(
        &mut self,
        signal: Traced<SignalRef>,
        result: Result<Arc<dyn SignalHistory>, String>,
    ) {
        if self.wanted_sources.contains(&signal) {
            self.sources.insert(signal, result);
        }
    }

    pub fn retry(&mut self) {
        self.sources.retain(|_, s| s.is_ok());
        self.jobs
            .retain(|_, j| !matches!(j.status, Status::Failed(_)));
    }

    pub fn sync(
        &mut self,
        wanted: HashSet<Key>,
        resident: &crate::wave::model::Resident,
        generation: u64,
        budget: &MemoryBudget,
    ) -> Vec<LoadRequest> {
        if self.generation != Some(generation) {
            self.clear();
            self.generation = Some(generation);
        }
        self.retain(&wanted);
        let mut requests = Vec::new();
        for key in wanted {
            if let Some(source) = resident.get(&key.signal) {
                self.sources.insert(key.signal, Ok(source.clone()));
            }
            let job = self.jobs.entry(key).or_insert_with(|| Job {
                request: 0,
                cancelled: Arc::new(AtomicBool::new(false)),
                status: Status::Waiting,
            });
            if !matches!(job.status, Status::Waiting) {
                continue;
            }
            match self.sources.get(&key.signal) {
                Some(Ok(source)) => {
                    self.next_request = self
                        .next_request
                        .checked_add(1)
                        .expect("moving-average request identities exhausted");
                    job.request = self.next_request;
                    job.status = Status::Building;
                    requests.push(LoadRequest::MovingAverage {
                        key,
                        request: job.request,
                        source: source.clone(),
                        budget: budget.clone(),
                        cancelled: job.cancelled.clone(),
                    });
                }
                Some(Err(error)) => job.status = Status::Failed(error.clone()),
                None => {}
            }
        }
        requests
    }

    pub fn deliver(
        &mut self,
        key: Key,
        request: u64,
        result: anyhow::Result<Arc<MeanHistory>>,
    ) -> bool {
        let Some(job) = self.jobs.get_mut(&key) else {
            return false;
        };
        if job.request != request || !matches!(job.status, Status::Building) {
            return false;
        }
        job.status = match result {
            Ok(history) => Status::Ready(history),
            Err(error) => Status::Failed(format!("{error:#}")),
        };
        true
    }
}

pub use Status as ComputationStatus;
