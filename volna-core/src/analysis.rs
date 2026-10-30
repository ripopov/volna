//! Demand-driven analysis results and completion identities.

use std::borrow::Borrow;
use std::collections::{HashMap, hash_map::Entry};
use std::hash::Hash;
use std::sync::Arc;

pub enum AnalysisLoad<T> {
    Building(u64),
    Ready(Arc<T>),
    Failed,
}

pub(crate) struct AnalysisCache<K, V, Owner = ()> {
    entries: HashMap<K, (Owner, AnalysisLoad<V>)>,
    next_request: u64,
}

impl<K, V, Owner> Default for AnalysisCache<K, V, Owner> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            next_request: 0,
        }
    }
}

impl<K: Eq + Hash, V, Owner> AnalysisCache<K, V, Owner> {
    pub fn clear(&mut self) {
        // In-flight completions can outlive a clear and subsequent identical demand.
        self.entries.clear();
    }

    pub fn retain(&mut self, mut wanted: impl FnMut(&K) -> bool) {
        self.entries.retain(|key, _| wanted(key));
    }

    pub fn get<Q: Eq + Hash + ?Sized>(&self, key: &Q) -> Option<&AnalysisLoad<V>>
    where
        K: Borrow<Q>,
    {
        self.entries.get(key).map(|(_, load)| load)
    }

    pub fn sync<I>(
        &mut self,
        wanted: HashMap<K, I>,
        mut start: impl FnMut(&K, I, u64) -> (Owner, AnalysisLoad<V>),
    ) {
        self.retain(|key| wanted.contains_key(key));
        for (key, input) in wanted {
            if let Entry::Vacant(entry) = self.entries.entry(key) {
                self.next_request = self
                    .next_request
                    .checked_add(1)
                    .expect("analysis request identities exhausted");
                let value = start(entry.key(), input, self.next_request);
                entry.insert(value);
            }
        }
    }

    pub fn deliver(&mut self, key: &K, request: u64, result: anyhow::Result<Arc<V>>) -> bool {
        let Some((_, load)) = self.entries.get_mut(key) else {
            return false;
        };
        if !matches!(load, AnalysisLoad::Building(pending) if *pending == request) {
            return false;
        }
        *load = result.map_or(AnalysisLoad::Failed, AnalysisLoad::Ready);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(cache: &mut AnalysisCache<u8, u8>) -> u64 {
        cache.sync(HashMap::from([(0, 1)]), |_, _, id| {
            ((), AnalysisLoad::Building(id))
        });
        let Some(AnalysisLoad::Building(id)) = cache.get(&0) else {
            panic!("expected pending analysis");
        };
        *id
    }

    #[test]
    fn stale_completions_cannot_replace_recreated_demand() {
        for clear in [false, true] {
            let mut cache = AnalysisCache::default();
            let stale = request(&mut cache);
            if clear {
                cache.clear();
            } else {
                cache.sync(HashMap::<u8, u8>::new(), |_, _, _| unreachable!());
            }
            let current = request(&mut cache);
            assert!(!cache.deliver(&0, stale, Err(anyhow::anyhow!("old worker failed"))));
            assert!(!cache.deliver(&0, stale, Ok(Arc::new(2))));
            assert!(cache.deliver(&0, current, Ok(Arc::new(1))));
            assert!(!cache.deliver(&0, current, Err(anyhow::anyhow!("duplicate completion"))));
            assert!(matches!(cache.get(&0), Some(AnalysisLoad::Ready(value)) if **value == 1));
        }
    }
}
