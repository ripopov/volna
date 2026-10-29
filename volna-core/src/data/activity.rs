//! Exact signal activity and scope change counts from the recording’s activity index.

use volna_trace::data::source::{Hierarchy, ScopeId, SignalRef};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

#[derive(Debug)]
pub struct ActivityCounter {
    contributions: vtr::Contributions,
    order: Vec<ScopeId>,
    _reservation: Reservation,
}

impl ActivityCounter {
    pub fn build(h: &Hierarchy, budget: &MemoryBudget) -> anyhow::Result<Self> {
        let signals = h.vars().map(|v| v.signal.0 as usize + 1).max().unwrap_or(0);
        let upper = super::admission::sum([
            super::admission::bytes::<[u8; 128]>(h.scope_count())?,
            super::admission::bytes::<[u8; 48]>(h.var_count())?,
            super::admission::bytes::<[u8; 16]>(signals)?,
            512,
        ])?;
        let reservation = budget.reserve_object("the activity counter", upper)?;
        let mut census = vtr::Census::recording();
        let mut order = Vec::with_capacity(h.scope_count());
        let mut stack: Vec<Option<ScopeId>> = h.roots().iter().rev().map(Some).collect();
        while let Some(entry) = stack.pop() {
            let Some(id) = entry else {
                census.leave();
                continue;
            };
            census.enter();
            order.push(id);
            let scope = h.scope(id);
            for v in scope.vars {
                census.var(h.signal(v).0);
            }
            stack.push(None);
            stack.extend(scope.children.iter().rev().map(Some));
        }
        let (_, contributions) = census.finish_recorded();
        drop(stack);
        let resident = contributions.memory_bytes()
            + (order.capacity() * std::mem::size_of::<ScopeId>()) as u64;
        Ok(Self {
            contributions,
            order,
            _reservation: super::admission::finish(reservation, resident)?,
        })
    }

    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        self.contributions.memory_bytes()
            + (self.order.capacity() * std::mem::size_of::<ScopeId>()) as u64
    }

    fn count(&self, signals: impl IntoIterator<Item = u32>) -> Vec<u32> {
        let mut by_census = Vec::new();
        self.contributions.count(signals, &mut by_census);
        let mut by_scope = vec![0; self.order.len()];
        for (i, &scope) in self.order.iter().enumerate() {
            by_scope[scope] = by_census[i];
        }
        by_scope
    }
}

#[derive(Debug)]
pub struct ActivityCounts {
    pub window: (u64, u64),
    changing: Vec<u32>,
    signals: Vec<vtr::SignalId>,
    _reservation: Reservation,
}

impl ActivityCounts {
    pub fn classify(
        index: &vtr::activity::Index,
        counter: &ActivityCounter,
        window: (u64, u64),
        budget: &MemoryBudget,
    ) -> anyhow::Result<Self> {
        let upper = super::admission::sum([
            super::admission::bytes::<[u8; 16]>(index.signal_count() as usize)?,
            super::admission::bytes::<[u8; 16]>(counter.order.len())?,
            64,
        ])?;
        let reservation = budget.reserve_object("the scope activity", upper)?;
        let signals = index.classify(window.0, window.1);
        let changing = counter.count(signals.iter().map(|s| s.0));
        let resident = (changing.capacity() * std::mem::size_of::<u32>()
            + signals.capacity() * std::mem::size_of::<vtr::SignalId>())
            as u64;
        Ok(Self {
            window,
            changing,
            signals,
            _reservation: super::admission::finish(reservation, resident)?,
        })
    }

    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        (self.changing.capacity() * std::mem::size_of::<u32>()
            + self.signals.capacity() * std::mem::size_of::<vtr::SignalId>()) as u64
    }

    #[must_use]
    pub fn get(&self, scope: ScopeId) -> Option<u32> {
        self.changing.get(scope).copied()
    }

    #[must_use]
    pub fn signal_active(&self, signal: SignalRef) -> bool {
        self.signals
            .binary_search_by_key(&signal.0, |s| s.0)
            .is_ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeActivity {
    pub changing: u32,
    pub total: u32,
}

impl ScopeActivity {
    #[must_use]
    pub fn quiet(&self) -> bool {
        self.total > 0 && self.changing == 0
    }

    #[must_use]
    pub fn label(&self) -> String {
        format!(
            "{} / {}",
            super::sizes::grouped(self.changing),
            super::sizes::grouped(self.total)
        )
    }

    #[must_use]
    pub fn share(&self) -> f32 {
        match self.total {
            0 => 0.0,
            total => self.changing as f32 / total as f32,
        }
    }

    #[must_use]
    pub fn detail(&self) -> String {
        format!(
            "{} of {} signals change in the view",
            super::sizes::grouped(self.changing),
            super::sizes::grouped(self.total)
        )
    }
}

#[must_use]
pub fn estimated_seconds(info: volna_trace::data::ActivityBuildInfo) -> u64 {
    info.bytes.div_ceil(1 << 30).max(1)
}
