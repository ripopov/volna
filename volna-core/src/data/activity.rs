//! Exact signal and generator activity for hierarchy rows and scope meters.

use volna_trace::data::source::{Hierarchy, ScopeId, SignalRef};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

#[derive(Debug)]
pub struct ActivityCounter {
    contributions: vtr::Contributions,
    order: Vec<ScopeId>,
    totals: Vec<u32>,
    generators: Vec<(u32, u32)>,
    _reservation: Reservation,
}

impl ActivityCounter {
    pub fn build(h: &Hierarchy, budget: &MemoryBudget) -> anyhow::Result<Self> {
        let signals = h.vars().map(|v| v.signal.0 as usize + 1).max().unwrap_or(0);
        let sources = signals
            .checked_add(h.generators().len())
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| anyhow::anyhow!("too many activity sources"))?;
        let upper = super::admission::sum([
            super::admission::bytes::<[u8; 128]>(h.scope_count())?,
            super::admission::bytes::<[u8; 48]>(h.var_count() + h.generators().len())?,
            super::admission::bytes::<[u8; 24]>(sources as usize)?,
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
            for generator in scope.generators {
                census.var(signals as u32 + generator as u32);
            }
            stack.push(None);
            stack.extend(scope.children.iter().rev().map(Some));
        }
        let (sizes, contributions) = census.finish_recorded();
        let mut totals = vec![0; h.scope_count()];
        for (i, &id) in order.iter().enumerate() {
            totals[id] = sizes.signals(i as u32);
        }
        let mut generators: Vec<_> = h
            .generators()
            .iter()
            .enumerate()
            .map(|(id, generator)| (generator.track.0, signals as u32 + id as u32))
            .collect();
        generators.sort_unstable();
        drop(stack);
        let resident = contributions.memory_bytes()
            + ((order.capacity() * std::mem::size_of::<ScopeId>())
                + totals.capacity() * 4
                + generators.capacity() * 8) as u64;
        Ok(Self {
            contributions,
            order,
            totals,
            generators,
            _reservation: super::admission::finish(reservation, resident)?,
        })
    }

    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        self.contributions.memory_bytes()
            + ((self.order.capacity() * std::mem::size_of::<ScopeId>())
                + self.totals.capacity() * 4
                + self.generators.capacity() * 8) as u64
    }

    #[must_use]
    pub fn total(&self, scope: ScopeId) -> Option<u32> {
        self.totals.get(scope).copied()
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
    generators: Vec<u32>,
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
            super::admission::bytes::<[u8; 16]>(
                index.signal_count() as usize + index.generator_count(),
            )?,
            super::admission::bytes::<[u8; 16]>(counter.order.len())?,
            64,
        ])?;
        let reservation = budget.reserve_object("the scope activity", upper)?;
        let signals = index.classify(window.0, window.1);
        let generators = index.classify_generators(window.0, window.1);
        let changing = counter.count(signals.iter().map(|s| s.0).chain(
            generators.iter().filter_map(|id| {
                counter
                    .generators
                    .binary_search_by_key(id, |&(wire, _)| wire)
                    .ok()
                    .map(|i| counter.generators[i].1)
            }),
        ));
        let resident = (changing.capacity() * std::mem::size_of::<u32>()
            + signals.capacity() * std::mem::size_of::<vtr::SignalId>()
            + generators.capacity() * 4) as u64;
        Ok(Self {
            window,
            changing,
            signals,
            generators,
            _reservation: super::admission::finish(reservation, resident)?,
        })
    }

    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        (self.changing.capacity() * std::mem::size_of::<u32>()
            + self.signals.capacity() * std::mem::size_of::<vtr::SignalId>()
            + self.generators.capacity() * 4) as u64
    }

    #[must_use]
    pub fn get(&self, scope: ScopeId) -> Option<u32> {
        self.changing.get(scope).copied()
    }

    #[must_use]
    pub fn generator_active(&self, generator: u32) -> bool {
        self.generators.binary_search(&generator).is_ok()
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
            "{} of {} signals or generators have activity in the view",
            super::sizes::grouped(self.changing),
            super::sizes::grouped(self.total)
        )
    }
}

#[must_use]
pub fn estimated_seconds(info: volna_trace::data::ActivityBuildInfo) -> u64 {
    info.bytes.div_ceil(1 << 30).max(1)
}
