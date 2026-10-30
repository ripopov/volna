//! Complete immutable transaction data.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::transactions::{
    AttributeSummary, AttributeType, Relation, TrackRef, Transaction, TransactionRef,
};

const SCAN_LEAVES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TransactionLocation {
    pub transaction: TransactionRef,
    pub generator: Option<TrackRef>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoadedRelation {
    pub id: u64,
    pub from_generator: Option<TrackRef>,
    pub to_generator: Option<TrackRef>,
    pub relation: Relation,
}

#[derive(Debug)]
pub struct LoadedGenerator {
    pub(crate) reservation: Option<crate::remote::memory::Reservation>,
    generator: TrackRef,
    transactions: Vec<Transaction>,
    attributes: Vec<AttributeSummary>,
    by_id: HashMap<TransactionRef, (usize, TrackRef)>,
    parents: HashMap<TransactionRef, TransactionLocation>,
    children: HashMap<TransactionLocation, Vec<TransactionRef>>,
    relations: Vec<LoadedRelation>,
    by_endpoint: HashMap<TransactionRef, Vec<u32>>,
    max_end: Vec<u64>,
    leaves: usize,
}

impl LoadedGenerator {
    #[must_use]
    pub fn memory_budget(&self) -> Option<crate::remote::memory::MemoryBudget> {
        self.reservation
            .as_ref()
            .map(super::super::remote::memory::Reservation::budget)
    }

    pub fn resident_bytes(&self) -> u64 {
        let catalogue = self.attribute_catalogue_bytes();
        let fixed = catalogue
            .saturating_add(
                (self.transactions.capacity() as u64)
                    .saturating_mul(std::mem::size_of::<Transaction>() as u64),
            )
            .saturating_add((self.by_id.capacity() as u64).saturating_mul(
                (std::mem::size_of::<TransactionRef>() + std::mem::size_of::<(usize, TrackRef)>())
                    as u64,
            ))
            .saturating_add(
                (self.max_end.capacity() as u64).saturating_mul(std::mem::size_of::<u64>() as u64),
            )
            .saturating_add((self.parents.capacity() as u64).saturating_mul(
                (std::mem::size_of::<TransactionRef>() + std::mem::size_of::<TransactionLocation>())
                    as u64,
            ))
            .saturating_add(
                (self.relations.capacity() as u64)
                    .saturating_mul(std::mem::size_of::<LoadedRelation>() as u64),
            )
            .saturating_add(index_bytes(
                self.children.capacity(),
                std::mem::size_of::<TransactionLocation>(),
                self.children.values().map(Vec::capacity).sum(),
                std::mem::size_of::<TransactionRef>(),
            ))
            .saturating_add(index_bytes(
                self.by_endpoint.capacity(),
                std::mem::size_of::<TransactionRef>(),
                self.by_endpoint.values().map(Vec::capacity).sum(),
                std::mem::size_of::<u32>(),
            ));
        let transactions =
            self.transactions.iter().fold(fixed, |bytes, tx| {
                bytes
                    .saturating_add(transaction_attributes_bytes(&tx.attributes))
                    .saturating_add((tx.events.capacity() as u64).saturating_mul(
                        std::mem::size_of::<super::transactions::TransactionEvent>() as u64,
                    ))
                    .saturating_add(tx.events.iter().fold(0, |sum, event| {
                        sum.saturating_add(event.name.capacity() as u64)
                            .saturating_add(attributes_bytes(&event.attributes))
                    }))
                    .saturating_add((tx.stages.capacity() as u64).saturating_mul(
                        std::mem::size_of::<super::transactions::TransactionStage>() as u64,
                    ))
                    .saturating_add(tx.stages.iter().fold(0, |sum, stage| {
                        sum.saturating_add(stage.name.capacity() as u64)
                            .saturating_add(stage.lane.capacity() as u64)
                            .saturating_add(attributes_bytes(&stage.attributes))
                    }))
            });
        self.relations.iter().fold(transactions, |bytes, edge| {
            bytes
                .saturating_add(edge.relation.kind.capacity() as u64)
                .saturating_add(attributes_bytes(&edge.relation.attributes))
        })
    }

    pub fn new(
        generator: TrackRef,
        mut transactions: Vec<Transaction>,
        parents: HashMap<TransactionRef, TransactionLocation>,
        relations: Vec<LoadedRelation>,
    ) -> anyhow::Result<Self> {
        transactions.sort_by_key(|tx| (tx.begin, tx.end, tx.id.0));
        let mut build = std::pin::pin!(Self::from_sorted(
            generator,
            transactions,
            parents,
            relations,
            || std::future::ready(()),
        ));
        match std::future::Future::poll(
            build.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop()),
        ) {
            std::task::Poll::Ready(result) => result,
            std::task::Poll::Pending => unreachable!("local index construction never yields"),
        }
    }

    pub(crate) async fn from_sorted<F: std::future::Future<Output = ()>>(
        generator: TrackRef,
        transactions: Vec<Transaction>,
        parents: HashMap<TransactionRef, TransactionLocation>,
        relations: Vec<LoadedRelation>,
        mut checkpoint: impl FnMut() -> F,
    ) -> anyhow::Result<Self> {
        let mut by_id = HashMap::with_capacity(transactions.len());
        let mut previous = None;
        let mut attributes: HashMap<&str, (AttributeType, usize)> = HashMap::new();
        for (index, tx) in transactions.iter().enumerate() {
            checkpoint().await;
            for attribute in &tx.attributes {
                checkpoint().await;
                let kind = AttributeType::of(&attribute.value);
                attributes
                    .entry(&attribute.key)
                    .and_modify(|(seen, _)| {
                        if *seen != kind {
                            *seen = AttributeType::Mixed;
                        }
                    })
                    .or_insert((kind, index));
            }
            let key = (tx.begin, tx.end, tx.id.0);
            anyhow::ensure!(
                previous.is_none_or(|last| last <= key),
                "unordered transaction payload"
            );
            previous = Some(key);
            anyhow::ensure!(
                tx.generator == generator,
                "transaction belongs to another generator"
            );
            anyhow::ensure!(tx.begin <= tx.end, "reversed transaction interval");
            anyhow::ensure!(
                by_id.insert(tx.id, (index, generator)).is_none(),
                "duplicate transaction identity"
            );
            for (id, source) in tx
                .events
                .iter()
                .map(|event| (event.id, event.generator.unwrap_or(generator)))
                .chain(
                    tx.stages
                        .iter()
                        .map(|stage| (stage.id, stage.generator.unwrap_or(generator))),
                )
            {
                checkpoint().await;
                anyhow::ensure!(
                    by_id.insert(id, (index, source)).is_none(),
                    "duplicate activity record identity"
                );
            }
            anyhow::ensure!(
                tx.parent == parents.get(&tx.id).map(|p| p.transaction),
                "parent location does not match transaction"
            );
        }
        for id in parents.keys() {
            checkpoint().await;
            anyhow::ensure!(
                by_id
                    .get(id)
                    .is_some_and(|&(index, _)| transactions[index].id == *id),
                "unknown child in parent locations"
            );
        }
        for parent in parents.values() {
            checkpoint().await;
            if parent.generator == Some(generator) {
                anyhow::ensure!(
                    by_id.contains_key(&parent.transaction),
                    "missing parent in complete generator"
                );
            }
        }
        let mut relation_ids = HashSet::with_capacity(relations.len());
        for edge in &relations {
            checkpoint().await;
            anyhow::ensure!(relation_ids.insert(edge.id), "duplicate relation identity");
            let from_here = by_id.contains_key(&edge.relation.from);
            let to_here = by_id.contains_key(&edge.relation.to);
            anyhow::ensure!(from_here || to_here, "relation does not touch generator");
            for (id, source) in [
                (edge.relation.from, edge.from_generator),
                (edge.relation.to, edge.to_generator),
            ] {
                anyhow::ensure!(
                    match by_id.get(&id) {
                        Some(&(_, expected)) => source == Some(expected),
                        None => source != Some(generator),
                    },
                    "missing or misplaced relation endpoint"
                );
            }
        }
        let mut children: HashMap<TransactionLocation, Vec<TransactionRef>> = HashMap::new();
        for tx in &transactions {
            checkpoint().await;
            if let Some(parent) = parents.get(&tx.id) {
                children.entry(*parent).or_default().push(tx.id);
            }
        }
        let mut by_endpoint: HashMap<TransactionRef, Vec<u32>> = HashMap::new();
        for (index, edge) in relations.iter().enumerate() {
            checkpoint().await;
            let index = u32::try_from(index).map_err(|_| anyhow::anyhow!("too many relations"))?;
            let from = by_id
                .contains_key(&edge.relation.from)
                .then_some(edge.relation.from);
            let to = by_id
                .contains_key(&edge.relation.to)
                .then_some(edge.relation.to);
            let keys = [
                from,
                to,
                from.map(|id| transactions[by_id[&id].0].id),
                to.map(|id| transactions[by_id[&id].0].id),
            ];
            for (position, key) in keys.iter().enumerate() {
                if let Some(key) = key
                    && !keys[..position].contains(&Some(*key))
                {
                    by_endpoint.entry(*key).or_default().push(index);
                }
            }
        }
        let leaves = transactions
            .len()
            .max(1)
            .checked_next_power_of_two()
            .ok_or_else(|| anyhow::anyhow!("transaction index too large"))?;
        let tree_len = leaves
            .checked_mul(2)
            .ok_or_else(|| anyhow::anyhow!("transaction index too large"))?;
        let mut max_end = Vec::new();
        max_end.try_reserve_exact(tree_len)?;
        for _ in 0..tree_len {
            checkpoint().await;
            max_end.push(0);
        }
        for (i, tx) in transactions.iter().enumerate() {
            checkpoint().await;
            max_end[leaves + i] = tx.end;
        }
        for i in (1..leaves).rev() {
            checkpoint().await;
            max_end[i] = max_end[i * 2].max(max_end[i * 2 + 1]);
        }
        let mut attributes: Vec<_> = attributes
            .into_iter()
            .map(|(key, (value_type, example_row))| AttributeSummary {
                key: key.to_owned(),
                value_type,
                example_row,
            })
            .collect();
        attributes.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        Ok(Self {
            reservation: None,
            attributes,
            generator,
            transactions,
            by_id,
            parents,
            children,
            relations,
            by_endpoint,
            max_end,
            leaves,
        })
    }

    #[must_use]
    pub fn attribute_catalogue(&self) -> &[AttributeSummary] {
        &self.attributes
    }

    pub(crate) fn attribute_catalogue_bytes(&self) -> u64 {
        (self.attributes.capacity() * std::mem::size_of::<AttributeSummary>()) as u64
            + self
                .attributes
                .iter()
                .map(|a| a.key.capacity() as u64)
                .sum::<u64>()
    }

    #[must_use]
    pub fn generator(&self) -> TrackRef {
        self.generator
    }
    #[must_use]
    pub fn transactions(&self) -> &[Transaction] {
        &self.transactions
    }
    #[must_use]
    pub fn relations(&self) -> &[LoadedRelation] {
        &self.relations
    }
    #[must_use]
    pub fn parent(&self, child: TransactionRef) -> Option<TransactionLocation> {
        self.parents.get(&child).copied()
    }
    pub fn children(&self, parent: TransactionLocation) -> &[TransactionRef] {
        self.children.get(&parent).map_or(&[], Vec::as_slice)
    }
    pub fn relations_of(&self, id: TransactionRef) -> impl Iterator<Item = &LoadedRelation> {
        self.by_endpoint
            .get(&id)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|&index| &self.relations[index as usize])
    }
    /// Activity IDs resolve to their owning row, including projected located records.
    #[must_use]
    pub fn transaction(&self, id: TransactionRef) -> Option<&Transaction> {
        self.by_id
            .get(&id)
            .map(|&(index, _)| &self.transactions[index])
    }

    #[must_use]
    pub fn transaction_ordinal(&self, id: TransactionRef) -> Option<usize> {
        self.by_id.get(&id).map(|&(index, _)| index)
    }

    #[must_use]
    pub fn record_generator(&self, id: TransactionRef) -> Option<TrackRef> {
        self.by_id.get(&id).map(|&(_, source)| source)
    }

    #[must_use]
    pub fn next_boundary(&self, time: u64) -> Option<u64> {
        let next = self.transactions.partition_point(|tx| tx.begin <= time);
        let mut best = self.transactions.get(next).map(|tx| tx.begin);
        drop(self.visit_window(time, time, |tx| {
            if tx.end > time && best.is_none_or(|b| tx.end < b) {
                best = Some(tx.end);
            }
            true
        }));
        best
    }

    #[must_use]
    pub fn prev_boundary(&self, time: u64) -> Option<u64> {
        let before = self.transactions.partition_point(|tx| tx.begin < time);
        let begin = self.transactions[..before].last()?.begin;
        let mut best = begin;
        drop(self.visit_window(begin, time.saturating_sub(1), |tx| {
            if tx.end < time && tx.end > best {
                best = tx.end;
            }
            true
        }));
        Some(best)
    }

    pub fn visit_window<'a>(
        &'a self,
        start: u64,
        end: u64,
        mut visitor: impl FnMut(&'a Transaction) -> bool,
    ) -> anyhow::Result<bool> {
        self.visit_window_ordinals(start, end, |_, tx| visitor(tx))
    }

    pub fn visit_window_ordinals<'a>(
        &'a self,
        start: u64,
        end: u64,
        mut visitor: impl FnMut(usize, &'a Transaction) -> bool,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(start <= end, "transaction window is reversed");
        let limit = self.transactions.partition_point(|tx| tx.begin <= end);
        Ok(self.visit_node(1, 0, self.leaves, start, limit, &mut visitor))
    }

    fn visit_node<'a>(
        &'a self,
        node: usize,
        lo: usize,
        hi: usize,
        start: u64,
        limit: usize,
        visitor: &mut impl FnMut(usize, &'a Transaction) -> bool,
    ) -> bool {
        if lo >= limit || self.max_end[node] < start {
            return true;
        }
        if hi - lo <= SCAN_LEAVES {
            return (lo..hi.min(limit)).all(|i| {
                let tx = &self.transactions[i];
                tx.end < start || visitor(i, tx)
            });
        }
        let mid = lo + (hi - lo) / 2;
        self.visit_node(node * 2, lo, mid, start, limit, visitor)
            && self.visit_node(node * 2 + 1, mid, hi, start, limit, visitor)
    }
}

fn index_bytes(entries: usize, key: usize, values: usize, value: usize) -> u64 {
    (entries as u64)
        .saturating_mul((key + std::mem::size_of::<Vec<u8>>()) as u64)
        .saturating_add((values as u64).saturating_mul(value as u64))
}

fn transaction_attributes_bytes(attributes: &[super::transactions::TransactionAttribute]) -> u64 {
    let fixed = (attributes.len() as u64)
        .saturating_mul(std::mem::size_of::<super::transactions::TransactionAttribute>() as u64);
    attributes.iter().fold(fixed, |bytes, attribute| {
        bytes
            .saturating_add(attribute.key.capacity() as u64)
            .saturating_add(attribute_value_bytes(&attribute.value))
    })
}

fn attributes_bytes(attributes: &super::transactions::Attributes) -> u64 {
    let fixed = (attributes.capacity() as u64)
        .saturating_mul(
            std::mem::size_of::<(String, super::transactions::AttributeValue)>() as u64,
        );
    attributes.iter().fold(fixed, |bytes, (key, value)| {
        bytes
            .saturating_add(key.capacity() as u64)
            .saturating_add(attribute_value_bytes(value))
    })
}

fn attribute_value_bytes(value: &super::transactions::AttributeValue) -> u64 {
    use super::transactions::AttributeValue;
    match value {
        AttributeValue::Text(value) => value.capacity() as u64,
        AttributeValue::Bytes(value) => value.capacity() as u64,
        AttributeValue::Logic { data, .. } => data.capacity() as u64,
        AttributeValue::Enum { name, .. } => name.capacity() as u64,
        AttributeValue::List(values) => {
            let fixed = (values.capacity() as u64)
                .saturating_mul(std::mem::size_of::<AttributeValue>() as u64);
            values.iter().fold(fixed, |bytes, value| {
                bytes.saturating_add(attribute_value_bytes(value))
            })
        }
        AttributeValue::Map(values) => attributes_bytes(values),
        AttributeValue::Null
        | AttributeValue::Bool(_)
        | AttributeValue::I64(_)
        | AttributeValue::U64(_)
        | AttributeValue::F64(_)
        | AttributeValue::Time(_)
        | AttributeValue::Pointer(_)
        | AttributeValue::Fixed { .. }
        | AttributeValue::UFixed { .. } => 0,
    }
}

#[derive(Clone, Debug)]
pub struct LoadedTrack {
    pub track: TrackRef,
    pub generators: Vec<Arc<LoadedGenerator>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::transactions::{SpanKind, Status};

    fn tx(id: u64, begin: u64, end: u64) -> Transaction {
        Transaction {
            id: TransactionRef(id),
            generator: TrackRef(1),
            begin,
            end,
            status: Status::Ok,
            kind: SpanKind::Producer,
            parent: None,
            attributes: vec![],
            events: vec![],
            stages: vec![],
        }
    }

    #[test]
    fn catalogue_includes_late_keys_merges_types_and_points_to_sorted_examples() {
        use crate::data::transactions::{AttributeValue, TransactionAttribute};
        let mut late = tx(2, 500, 510);
        late.attributes = vec![
            TransactionAttribute {
                key: "z.late".into(),
                value: AttributeValue::Bool(true),
            },
            TransactionAttribute {
                key: "mixed".into(),
                value: AttributeValue::Text("text".into()),
            },
        ];
        let mut early = tx(1, 1, 2);
        early.attributes = vec![
            TransactionAttribute {
                key: "mixed".into(),
                value: AttributeValue::U64(9),
            },
            TransactionAttribute {
                key: "a.map".into(),
                value: AttributeValue::Map(vec![("nested".into(), AttributeValue::Null)]),
            },
        ];
        let loaded =
            LoadedGenerator::new(TrackRef(1), vec![late, early], HashMap::new(), vec![]).unwrap();
        let catalogue = loaded.attribute_catalogue();
        assert_eq!(
            catalogue.iter().map(|a| a.key.as_str()).collect::<Vec<_>>(),
            ["a.map", "mixed", "z.late"]
        );
        assert_eq!(catalogue[1].value_type, AttributeType::Mixed);
        assert_eq!(
            loaded.transactions()[catalogue[1].example_row].id,
            TransactionRef(1)
        );
        assert_eq!(catalogue[2].value_type, AttributeType::Bool);
        assert_eq!(
            loaded.transactions()[catalogue[2].example_row].id,
            TransactionRef(2)
        );
        assert!(loaded.resident_bytes() >= loaded.attribute_catalogue_bytes());
    }

    #[test]
    fn interval_index_matches_scan_including_long_and_point_records() {
        let mut records = vec![tx(0, 0, u64::MAX), tx(1, u64::MAX, u64::MAX)];
        for id in 2..202 {
            let begin = (id * 37) % 100;
            records.push(tx(id, begin, begin + (id * 19) % 50));
        }
        let loaded = LoadedGenerator::new(TrackRef(1), records, HashMap::new(), vec![]).unwrap();
        for start in 0..160 {
            for end in [start, start + 7, u64::MAX] {
                let expected: Vec<_> = loaded
                    .transactions()
                    .iter()
                    .filter(|tx| tx.begin <= end && tx.end >= start)
                    .map(|tx| tx.id)
                    .collect();
                let mut actual = vec![];
                assert!(
                    loaded
                        .visit_window(start, end, |tx| {
                            actual.push(tx.id);
                            true
                        })
                        .unwrap()
                );
                assert_eq!(actual, expected);
            }
        }
        let mut count = 0;
        assert!(
            !loaded
                .visit_window(0, u64::MAX, |_| {
                    count += 1;
                    false
                })
                .unwrap()
        );
        assert_eq!(count, 1);
        assert!(loaded.visit_window(2, 1, |_| true).is_err());
    }

    #[test]
    fn cooperative_index_yields_and_matches_local_queries() {
        use std::cell::Cell;
        use std::future::{Future, poll_fn};
        use std::task::{Context, Poll, Waker};
        let records: Vec<_> = (0..2000)
            .map(|id| tx(id, id, if id == 0 { 10000 } else { id + 3 }))
            .collect();
        let local =
            LoadedGenerator::new(TrackRef(1), records.clone(), HashMap::new(), vec![]).unwrap();
        let credits = Cell::new(0);
        let checkpoint = || {
            poll_fn(|_| {
                if credits.get() == 0 {
                    Poll::Pending
                } else {
                    credits.set(credits.get() - 1);
                    Poll::Ready(())
                }
            })
        };
        let mut build = std::pin::pin!(LoadedGenerator::from_sorted(
            TrackRef(1),
            records,
            HashMap::new(),
            vec![],
            checkpoint
        ));
        let mut polls = 0;
        let remote = loop {
            credits.set(32);
            polls += 1;
            match build.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                Poll::Pending => assert_eq!(credits.get(), 0),
                Poll::Ready(result) => break result.unwrap(),
            }
        };
        assert!(polls > 100);
        assert_eq!(local.transactions(), remote.transactions());
        assert_eq!(local.by_id, remote.by_id);
        assert_eq!(local.max_end, remote.max_end);
        let mut overlap = Vec::new();
        remote
            .visit_window(1000, 1000, |tx| {
                overlap.push(tx.id);
                true
            })
            .unwrap();
        assert!(overlap.contains(&TransactionRef(0)));
        assert!(overlap.contains(&TransactionRef(1000)));

        let mut unordered = std::pin::pin!(LoadedGenerator::from_sorted(
            TrackRef(1),
            vec![tx(1, 2, 3), tx(2, 1, 2)],
            HashMap::new(),
            vec![],
            || std::future::ready(())
        ));
        assert!(matches!(
            unordered
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(_))
        ));
    }

    #[test]
    fn validates_complete_objects_and_keeps_parallel_relations() {
        let mut child = tx(2, 10, 10);
        child.parent = Some(TransactionRef(9));
        let parent = TransactionLocation {
            transaction: TransactionRef(9),
            generator: Some(TrackRef(3)),
        };
        let edge = LoadedRelation {
            id: 0,
            from_generator: Some(TrackRef(3)),
            to_generator: Some(TrackRef(1)),
            relation: Relation {
                from: parent.transaction,
                to: child.id,
                kind: "causes".into(),
                attributes: vec![],
            },
        };
        let parents = HashMap::from([(child.id, parent)]);
        assert!(
            LoadedGenerator::new(TrackRef(1), vec![child.clone()], HashMap::new(), vec![]).is_err()
        );
        assert!(
            LoadedGenerator::new(
                TrackRef(1),
                vec![child.clone(), child.clone()],
                parents.clone(),
                vec![]
            )
            .is_err()
        );
        assert!(
            LoadedGenerator::new(
                TrackRef(1),
                vec![child.clone()],
                parents.clone(),
                vec![edge.clone(), edge.clone()]
            )
            .is_err()
        );
        let mut parallel = edge.clone();
        parallel.id = 1;
        let loaded = Arc::new(
            LoadedGenerator::new(TrackRef(1), vec![child], parents, vec![edge, parallel]).unwrap(),
        );
        assert_eq!(loaded.relations().len(), 2);
        assert_eq!(loaded.parent(TransactionRef(2)), Some(parent));
        assert!(loaded.transaction(TransactionRef(9)).is_none());
        let stream = LoadedTrack {
            track: TrackRef(0),
            generators: vec![loaded.clone()],
        };
        let generator = LoadedTrack {
            track: TrackRef(1),
            generators: vec![loaded.clone()],
        };
        assert!(Arc::ptr_eq(&stream.generators[0], &generator.generators[0]));
        let weak = Arc::downgrade(&loaded);
        drop(loaded);
        drop(stream);
        drop(generator);
        assert!(weak.upgrade().is_none());
        let empty = LoadedGenerator::new(TrackRef(1), vec![], HashMap::new(), vec![]).unwrap();
        assert!(
            empty
                .visit_window(0, u64::MAX, |_| panic!("empty"))
                .unwrap()
        );
    }
}
