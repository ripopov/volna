//! Linear-space interval paging, with count and identity rank independent of overlap count.

use crate::data::admission::Storage;
use std::ops::Range;
use volna_trace::data::transactions::Transaction;

#[derive(Debug)]
struct Node {
    center: u64,
    begins: Vec<usize>,
    ends: Vec<usize>,
    left: Option<usize>,
    right: Option<usize>,
}

#[derive(Debug, Default)]
pub struct Intervals {
    nodes: Vec<Node>,
}

fn last(tx: &Transaction) -> u64 {
    tx.end.saturating_sub(u64::from(tx.end > tx.begin))
}

impl Intervals {
    pub(crate) fn build(records: &[Transaction], storage: &mut Storage) -> anyhow::Result<Self> {
        let mut out = Self::default();
        let mut ordinals = storage.vec(records.len())?;
        ordinals.extend(0..records.len());
        let mut pending = storage.vec(1)?;
        pending.push((None, false, ordinals));
        while let Some((parent, right, ordinals)) = pending.pop() {
            if ordinals.is_empty() {
                storage.release(ordinals)?;
                continue;
            }
            let center = records[ordinals[ordinals.len() / 2]].begin;
            let mut sizes = [0; 3];
            for &ordinal in &ordinals {
                let tx = &records[ordinal];
                sizes[if last(tx) < center {
                    0
                } else if tx.begin > center {
                    2
                } else {
                    1
                }] += 1;
            }
            let mut left = storage.vec(sizes[0])?;
            let mut crossing = storage.vec(sizes[1])?;
            let mut after = storage.vec(sizes[2])?;
            for &ordinal in &ordinals {
                let tx = &records[ordinal];
                if last(tx) < center {
                    left.push(ordinal);
                } else if tx.begin > center {
                    after.push(ordinal);
                } else {
                    crossing.push(ordinal);
                }
            }
            storage.release(ordinals)?;
            let mut ends = storage.vec(crossing.len())?;
            ends.extend_from_slice(&crossing);
            ends.sort_unstable_by_key(|&i| (last(&records[i]), i));
            let index = out.nodes.len();
            storage.push(
                &mut out.nodes,
                Node {
                    center,
                    begins: crossing,
                    ends,
                    left: None,
                    right: None,
                },
            )?;
            if let Some(parent) = parent {
                let node: &mut Node = &mut out.nodes[parent];
                if right {
                    node.right = Some(index);
                } else {
                    node.left = Some(index);
                }
            }
            storage.push(&mut pending, (Some(index), true, after))?;
            storage.push(&mut pending, (Some(index), false, left))?;
        }
        storage.release(pending)?;
        Ok(out)
    }

    fn matching<'a>(node: &'a Node, records: &[Transaction], time: u64) -> &'a [usize] {
        if time < node.center {
            &node.begins[..node.begins.partition_point(|&i| records[i].begin <= time)]
        } else {
            &node.ends[node.ends.partition_point(|&i| last(&records[i]) < time)..]
        }
    }

    /// A deterministic order along the search path, resumable by rank without scanning overlaps.
    #[must_use]
    pub fn page(&self, records: &[Transaction], time: u64, range: Range<usize>) -> Vec<usize> {
        let mut result = Vec::with_capacity(range.len().min(128));
        let (mut skip, mut remaining) = (range.start, range.len().min(128));
        let mut next = (!self.nodes.is_empty()).then_some(0);
        while let Some(i) = next {
            let node = &self.nodes[i];
            let matches = Self::matching(node, records, time);
            let take = remaining.min(matches.len().saturating_sub(skip));
            if take > 0 {
                result.extend_from_slice(&matches[skip..skip + take]);
            }
            skip = skip.saturating_sub(matches.len());
            remaining -= take;
            if remaining == 0 {
                break;
            }
            next = if time < node.center {
                node.left
            } else {
                node.right
            };
        }
        result
    }

    #[must_use]
    pub fn count(&self, records: &[Transaction], time: u64) -> usize {
        let mut count = 0;
        let mut next = (!self.nodes.is_empty()).then_some(0);
        while let Some(i) = next {
            let node = &self.nodes[i];
            count += Self::matching(node, records, time).len();
            next = if time < node.center {
                node.left
            } else {
                node.right
            };
        }
        count
    }

    #[must_use]
    pub fn rank(&self, records: &[Transaction], time: u64, ordinal: usize) -> Option<usize> {
        let tx = records.get(ordinal)?;
        if tx.begin > time || last(tx) < time {
            return None;
        }
        let mut rank = 0;
        let mut next = (!self.nodes.is_empty()).then_some(0);
        while let Some(i) = next {
            let node = &self.nodes[i];
            let matches = Self::matching(node, records, time);
            if tx.begin <= node.center && last(tx) >= node.center {
                let key = if time < node.center {
                    tx.begin
                } else {
                    last(tx)
                };
                return matches
                    .binary_search_by_key(&(key, ordinal), |&i| {
                        (
                            if time < node.center {
                                records[i].begin
                            } else {
                                last(&records[i])
                            },
                            i,
                        )
                    })
                    .ok()
                    .map(|offset| rank + offset);
            }
            rank += matches.len();
            next = if time < node.center {
                node.left
            } else {
                node.right
            };
        }
        None
    }

    pub(crate) fn bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<Node>()
            + self
                .nodes
                .iter()
                .map(|node| (node.begins.capacity() + node.ends.capacity()) * size_of::<usize>())
                .sum::<usize>()
    }
}
