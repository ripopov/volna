//! Scope activity: how many of each scope's
//! signals change in a window, from the trace's activity index at once, and
//! exactly once the signals the index leaves undecided are read.
//!
//! [`ActivityCounter`] holds every signal's census weights over the trace's
//! scopes, built once per trace off the UI thread. [`ActivityCounts`] is one
//! window's answer per scope; a frame costs one classification of every
//! signal and one pass over the changing signals' weights and the scopes.

use volna_trace::data::SignalRef;
use volna_trace::data::source::{Hierarchy, ScopeId};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

/// Every signal's census weights over one trace's scopes, so that the
/// distinct signals of any set below every scope cost the set's variables
/// plus one pass over the scopes.
#[derive(Debug)]
pub struct ActivityCounter {
    contributions: vtr::Contributions,
    /// Census index → scope.
    order: Vec<ScopeId>,
    _reservation: Reservation,
}

impl ActivityCounter {
    /// One recording census over the hierarchy in preorder.
    pub fn build(h: &Hierarchy, budget: &MemoryBudget) -> anyhow::Result<Self> {
        let signals = h.vars().map(|v| v.signal.0 as usize + 1).max().unwrap_or(0);
        // Census owns scope/union-find stacks and up to two recorded weights
        // per variable. Allow vector growth and the overlapping arrays made
        // while grouping weights by signal; discard that scratch after build.
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

    pub fn resident_bytes(&self) -> u64 {
        self.contributions.memory_bytes()
            + (self.order.capacity() * std::mem::size_of::<ScopeId>()) as u64
    }

    /// Distinct signals of `signals` at or below every scope, by scope.
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

fn counts_bytes(changing: &Vec<u32>, upper: &Vec<u32>, undecided: &Vec<SignalRef>) -> u64 {
    ((changing.capacity() + upper.capacity()) * std::mem::size_of::<u32>()
        + undecided.capacity() * std::mem::size_of::<SignalRef>()) as u64
}

/// One window's activity per scope: the signals that change, and at most
/// how many may while some are undecided.
#[derive(Debug)]
pub struct ActivityCounts {
    /// The window in trace times, both ends included.
    pub window: (u64, u64),
    /// Signals below each scope known to change, by scope.
    changing: Vec<u32>,
    /// Changing or undecided, by scope; empty once exact.
    upper: Vec<u32>,
    /// Signals the index cannot decide in this window, to read from the
    /// trace; empty once exact.
    pub undecided: Vec<SignalRef>,
    _reservation: Reservation,
}

impl ActivityCounts {
    /// What the index says about `window` (trace times).
    pub fn classify(
        index: &vtr::activity::Index,
        counter: &ActivityCounter,
        window: (u64, u64),
        budget: &MemoryBudget,
    ) -> anyhow::Result<Self> {
        // Two classification lists can grow to twice their logical length;
        // per-scope count scratch overlaps the changing and upper arrays.
        let upper = super::admission::sum([
            super::admission::bytes::<[u8; 16]>(index.signal_count() as usize)?,
            super::admission::bytes::<[u8; 16]>(counter.order.len())?,
            64,
        ])?;
        let reservation = budget.reserve_object("the scope activity", upper)?;
        let c = index.classify(window.0, window.1);
        let changing = counter.count(c.active.iter().map(|s| s.0));
        let upper = if c.undecided.is_empty() {
            Vec::new()
        } else {
            let may = counter.count(c.undecided.iter().map(|s| s.0));
            changing.iter().zip(may).map(|(a, b)| a + b).collect()
        };
        let undecided: Vec<_> = c.undecided.into_iter().map(|s| SignalRef(s.0)).collect();
        drop(c.active);
        let resident = counts_bytes(&changing, &upper, &undecided);
        Ok(Self {
            window,
            changing,
            upper,
            undecided,
            _reservation: super::admission::finish(reservation, resident)?,
        })
    }

    /// Admit the count arrays before resolving histories or projecting a
    /// remote answer. The returned owner pays for both scratch and output.
    pub(crate) fn admit_resolution(
        &self,
        counter: &ActivityCounter,
        budget: &MemoryBudget,
    ) -> anyhow::Result<Reservation> {
        budget.reserve_object(
            "the scope activity",
            super::admission::sum([
                super::admission::bytes::<[u8; 12]>(counter.order.len())?,
                super::admission::bytes::<[SignalRef; 2]>(self.undecided.len())?,
                32,
            ])?,
        )
    }

    /// The exact counts once the undecided signals were read: `changed` are
    /// those of them that change in the window. Admission covers count scratch.
    pub(crate) fn resolved(
        &self,
        counter: &ActivityCounter,
        changed: Vec<SignalRef>,
        reservation: Reservation,
    ) -> anyhow::Result<Self> {
        let more = counter.count(changed.iter().map(|s| s.0));
        let changing: Vec<_> = self.changing.iter().zip(more).map(|(a, b)| a + b).collect();
        drop(changed);
        let resident = counts_bytes(&changing, &Vec::new(), &Vec::new());
        Ok(Self {
            window: self.window,
            changing,
            upper: Vec::new(),
            undecided: Vec::new(),
            _reservation: super::admission::finish(reservation, resident)?,
        })
    }

    pub fn resident_bytes(&self) -> u64 {
        counts_bytes(&self.changing, &self.upper, &self.undecided)
    }

    /// Whether every signal is decided.
    pub fn exact(&self) -> bool {
        self.undecided.is_empty()
    }

    /// `(changing, upper)` below `scope`.
    pub fn get(&self, scope: ScopeId) -> Option<(u32, u32)> {
        let changing = *self.changing.get(scope)?;
        Some((changing, self.upper.get(scope).copied().unwrap_or(changing)))
    }
}

/// What a scope row shows of the activity in the viewport: of its `total`
/// signals, `changing` change for sure and at most `upper` may.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeActivity {
    pub changing: u32,
    pub upper: u32,
    pub total: u32,
}

impl ScopeActivity {
    /// Nothing is left to read.
    pub fn exact(&self) -> bool {
        self.changing == self.upper
    }

    /// No signal below the scope can change: the row is drawn faint.
    pub fn quiet(&self) -> bool {
        self.total > 0 && self.upper == 0
    }

    /// The count column: `2,483 / 6,779`, or a range while signals are
    /// read: `1,462–2,210 / 6,779`.
    pub fn label(&self) -> String {
        let total = super::sizes::grouped(self.total);
        match self.exact() {
            true => format!("{} / {total}", super::sizes::grouped(self.changing)),
            false => format!(
                "{}–{} / {total}",
                super::sizes::grouped(self.changing),
                super::sizes::grouped(self.upper)
            ),
        }
    }

    /// Shares of the scope's signals that change and that may: the
    /// meter's solid and hatched lengths, in `0..=1`.
    pub fn shares(&self) -> (f32, f32) {
        match self.total {
            0 => (0.0, 0.0),
            t => (
                self.changing as f32 / t as f32,
                self.upper as f32 / t as f32,
            ),
        }
    }

    /// The tooltip line: `2,483 of 6,779 signals change in the view`.
    pub fn detail(&self) -> String {
        let total = super::sizes::grouped(self.total);
        match self.exact() {
            true => format!(
                "{} of {total} signals change in the view",
                super::sizes::grouped(self.changing)
            ),
            false => format!(
                "{} to {} of {total} signals change in the view; reading the rest from the trace",
                super::sizes::grouped(self.changing),
                super::sizes::grouped(self.upper)
            ),
        }
    }
}

/// Estimated build duration from the compressed recording size.
pub fn estimated_seconds(info: volna_trace::data::ActivityBuildInfo) -> u64 {
    let rate = match info.format {
        vtr::activity::SourceFormat::Vtr => 1 << 30,
        vtr::activity::SourceFormat::Fst => 400 << 20,
    };
    info.bytes.div_ceil(rate).max(1)
}
