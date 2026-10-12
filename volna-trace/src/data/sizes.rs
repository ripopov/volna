//! Scope sizes: the distinct signals, variables and scopes at or below each scope.

use super::source::{Hierarchy, ScopeId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScopeSize {
    pub signals: u32,
    pub variables: u32,
    pub scopes: u32,
}

#[derive(Debug, Default)]
pub struct ScopeSizes {
    pub(crate) sizes: Vec<ScopeSize>,
    pub(crate) _ownership: Option<std::sync::Arc<Vec<crate::remote::memory::Reservation>>>,
}

impl ScopeSizes {
    pub fn count(h: &Hierarchy) -> Self {
        let mut census = vtr::Census::new();
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
        let counted = census.finish();
        let mut sizes = vec![ScopeSize::default(); h.scope_count()];
        for (i, &id) in order.iter().enumerate() {
            let i = i as u32;
            sizes[id] = ScopeSize {
                signals: counted.signals(i),
                variables: counted.variables(i),
                scopes: counted.scopes(i),
            };
        }
        Self {
            sizes,
            _ownership: None,
        }
    }

    pub fn get(&self, scope: ScopeId) -> Option<ScopeSize> {
        self.sizes.get(scope).copied()
    }
}
