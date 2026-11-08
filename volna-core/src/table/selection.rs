//! Local copy selection over immutable row orders, independent of shared navigation.

use std::{collections::BTreeSet, ops::Range, sync::Arc};

use super::query::Order;

#[derive(Clone)]
pub(super) struct RowOrder {
    pub len: u64,
    pub query: Option<Arc<Order>>,
}

impl RowOrder {
    pub fn source_row(&self, rank: u64) -> Option<u64> {
        if rank >= self.len {
            return None;
        }
        self.query
            .as_ref()
            .map_or(Some(rank), |q| q.source_row(rank))
    }

    pub fn rank(&self, source: u64) -> Option<u64> {
        self.query
            .as_ref()
            .map_or_else(|| (source < self.len).then_some(source), |q| q.rank(source))
    }
}

impl Drop for RowOrder {
    fn drop(&mut self) {
        if let Some(query) = self.query.take() {
            super::query::retire(query);
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum SelectionMode {
    Replace,
    Toggle,
    Extend,
}

#[derive(Clone, Default)]
pub(super) struct RowSelection {
    // Ranks identify different rows after sorting; retain the order in which the range was selected.
    base: Option<(RowOrder, Range<u64>)>,
    toggled: BTreeSet<u64>,
    anchor: Option<u64>,
    focus: Option<u64>,
    // An explicitly empty selection must not fall back to copying the active transaction.
    explicit: bool,
    count: u64,
}

impl RowSelection {
    pub fn clear(&mut self) {
        *self = Self {
            explicit: true,
            ..Self::default()
        };
    }

    pub fn len(&self) -> u64 {
        self.count
    }

    pub fn select(
        &mut self,
        order: RowOrder,
        ordinal: u64,
        mode: SelectionMode,
        active: Option<u64>,
    ) -> bool {
        let Some(source) = order.source_row(ordinal) else {
            return false;
        };
        match mode {
            SelectionMode::Extend => {
                let (anchor, rank) = self
                    .anchor
                    .or_else(|| active.and_then(|rank| order.source_row(rank)))
                    .and_then(|source| order.rank(source).map(|rank| (source, rank)))
                    .unwrap_or((source, ordinal));
                self.replace(order, rank.min(ordinal)..rank.max(ordinal) + 1);
                self.anchor = Some(anchor);
            }
            SelectionMode::Toggle => {
                self.toggle(source);
                self.anchor = Some(source);
            }
            SelectionMode::Replace => {
                self.replace(order, ordinal..ordinal + 1);
                self.anchor = Some(source);
            }
        }
        self.focus = Some(source);
        true
    }

    pub fn select_all(&mut self, order: RowOrder) -> bool {
        if order.len == 0 {
            return false;
        }
        self.anchor = order.source_row(0);
        let len = order.len;
        self.replace(order, 0..len);
        true
    }

    pub fn focus_rank(&self, order: &RowOrder) -> Option<u64> {
        order.rank(self.focus?)
    }

    /// Returns None if the selected visible rows exceed the limit.
    pub fn visible_ranks(
        &self,
        order: &RowOrder,
        active: Option<u64>,
        limit: usize,
    ) -> Option<Vec<u64>> {
        let mut ranks = Vec::new();
        let mut push = |rank| {
            if ranks.len() >= limit {
                return None;
            }
            ranks.push(rank);
            Some(())
        };
        if !self.explicit {
            if let Some(rank) = active.filter(|&rank| rank < order.len) {
                push(rank)?;
            }
        } else if order.len <= self.count {
            for rank in 0..order.len {
                if order
                    .source_row(rank)
                    .is_some_and(|source| self.contains(source))
                {
                    push(rank)?;
                }
            }
        } else {
            for source in self.sources() {
                if let Some(rank) = order.rank(source) {
                    push(rank)?;
                }
            }
            ranks.sort_unstable();
        }
        Some(ranks)
    }

    fn in_base(&self, source: u64) -> bool {
        self.base.as_ref().is_some_and(|(order, range)| {
            order.rank(source).is_some_and(|rank| range.contains(&rank))
        })
    }

    pub fn contains(&self, source: u64) -> bool {
        self.in_base(source) ^ self.toggled.contains(&source)
    }

    fn replace(&mut self, order: RowOrder, range: Range<u64>) {
        self.count = range.end - range.start;
        self.base = Some((order, range));
        self.toggled.clear();
        self.explicit = true;
    }

    fn toggle(&mut self, source: u64) {
        if self.contains(source) {
            self.count -= 1;
        } else {
            self.count += 1;
        }
        if !self.toggled.remove(&source) {
            self.toggled.insert(source);
        }
        if self.count == 0 {
            self.base = None;
            self.toggled.clear();
        }
        self.explicit = true;
    }

    fn sources(&self) -> impl Iterator<Item = u64> + '_ {
        self.base
            .iter()
            .flat_map(|(order, range)| range.clone().filter_map(|rank| order.source_row(rank)))
            .filter(|source| !self.toggled.contains(source))
            .chain(
                self.toggled
                    .iter()
                    .copied()
                    .filter(|&source| !self.in_base(source)),
            )
    }
}
