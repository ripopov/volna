//! Source indices and display order for logic buses.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::model::{DisplayedSignal, WaveModel, WaveRow};
use super::tree::{self, Entry, Splice};
use crate::data::projection::BitSlice;
use crate::document::Document;
use crate::history::RowSelection;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusExpansion {
    pub collapsed: bool,
    pub initialized: bool,
}

impl Default for BusExpansion {
    fn default() -> Self {
        Self {
            collapsed: true,
            initialized: false,
        }
    }
}

#[derive(Clone, Copy)]
pub struct BusIndices<'a> {
    pub name: &'a str,
    left: i64,
    right: i64,
}

impl<'a> BusIndices<'a> {
    #[must_use]
    pub fn new(name: &'a str, width: u32) -> Self {
        if let Some((base, range)) = name
            .trim_end()
            .strip_suffix(']')
            .and_then(|s| s.rsplit_once('['))
            && let Some((left, right)) = range.split_once(':')
            && let (Ok(left), Ok(right)) = (left.trim().parse::<i64>(), right.trim().parse::<i64>())
            && left.abs_diff(right).checked_add(1) == Some(u64::from(width))
        {
            return Self {
                name: base.trim_end(),
                left,
                right,
            };
        }
        Self {
            name,
            left: i64::from(width) - 1,
            right: 0,
        }
    }

    #[must_use]
    pub fn index(self, offset: u32) -> i64 {
        self.left + i64::from(offset) * if self.left <= self.right { 1 } else { -1 }
    }

    #[must_use]
    pub fn label(self, slice: BitSlice) -> String {
        let (a, b) = (self.index(slice.first), self.index(slice.last));
        if a == b {
            format!("{}[{a}]", self.name)
        } else {
            format!("{}[{a}:{b}]", self.name)
        }
    }

    pub fn parse(self, text: &str) -> Result<BitSlice> {
        let text = text.trim();
        let text = text
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(text);
        let (a, b) = text.split_once(':').unwrap_or((text, text));
        let offset = |s: &str| -> Result<u32> {
            let i = s
                .trim()
                .parse::<i64>()
                .context("Enter a bit index or a slice, such as 7:0")?;
            ensure!(
                (self.left.min(self.right)..=self.left.max(self.right)).contains(&i),
                "Bit {i} is outside [{}:{}]",
                self.left,
                self.right
            );
            Ok(self.left.abs_diff(i) as u32)
        };
        Ok(BitSlice {
            first: offset(a)?,
            last: offset(b)?,
        })
    }
}

impl WaveModel {
    pub(super) fn disclosure_hint(&self) -> Option<(&'static str, crate::geometry::Point)> {
        if self.drag.is_some() {
            return None;
        }
        let pointer = self.pointer?;
        let signal = self.signal(self.chevron_at(pointer)?)?;
        Some((
            if signal.bus?.collapsed {
                "Expand bits"
            } else {
                "Collapse bits"
            },
            pointer,
        ))
    }

    pub(super) fn expand_bits(&mut self, doc: &Document, i: usize) -> bool {
        let entry = &self.items[i];
        if entry.depth + 1 >= tree::MAX_DEPTH
            || self
                .items
                .len()
                .saturating_add(entry.signal().unwrap().shape.width() as usize)
                > crate::workspace::MAX_ROWS
        {
            self.bus_notice = Some("Bus expansion exceeds the row or nesting limit. Use Add bit or slice… to inspect a range.".into());
            return false;
        }
        let mut parent = entry.clone();
        let signal = parent.row.signal_mut().unwrap();
        signal.bus = Some(super::bus::BusExpansion {
            collapsed: false,
            initialized: true,
        });
        let children = (0..signal.shape.width())
            .map(|offset| {
                let slice = crate::data::projection::BitSlice {
                    first: offset,
                    last: offset,
                };
                Entry::new(
                    parent.depth + 1,
                    WaveRow::from(signal.projected(slice, &doc.translators)),
                )
            })
            .collect::<Vec<_>>();
        let added = children.len();
        let shift = |r: usize| if r > i { r + added } else { r };
        let selection = RowSelection {
            selected: self.selected.iter().copied().map(shift).collect(),
            anchor: self.anchor.map(shift),
        };
        let mut insert = vec![parent];
        insert.extend(children);
        self.edit_rows(
            "Expand bus bits".into(),
            vec![Splice {
                at: i,
                remove: 1,
                insert,
            }],
            selection,
        );
        true
    }

    pub fn reverse_bits(&mut self, row: usize) -> bool {
        let Some(signal) = self.signal(row).filter(|s| s.bus.is_some()) else {
            return false;
        };
        if !signal.bus.unwrap().initialized {
            return false;
        }
        let end = tree::subtree_end(&self.items, row);
        if end <= row + 2 {
            return false;
        }
        let insert = self.items[row + 1..end].iter().rev().cloned().collect();
        let reverse = |r: usize| {
            if (row + 1..end).contains(&r) {
                row + end - r
            } else {
                r
            }
        };
        let selection = RowSelection {
            selected: self.selected.iter().copied().map(reverse).collect(),
            anchor: self.anchor.map(reverse),
        };
        self.edit_rows(
            "Reverse bit order".into(),
            vec![Splice {
                at: row + 1,
                remove: end - row - 1,
                insert,
            }],
            selection,
        );
        true
    }

    pub fn add_slice(&mut self, doc: &Document, row: usize, text: &str) -> anyhow::Result<()> {
        let source = self
            .signal(row)
            .filter(|s| s.bus.is_some())
            .ok_or_else(|| anyhow::anyhow!("Select a bus"))?;
        let slice = super::bus::BusIndices::new(&source.name, source.shape.width()).parse(text)?;
        let projected = source.projected(slice, &doc.translators);
        let at = tree::subtree_end(&self.items, row);
        let insert = vec![Entry::new(self.items[row].depth, WaveRow::from(projected))];
        self.edit_rows(
            "Add signal slice".into(),
            vec![Splice {
                at,
                remove: 0,
                insert,
            }],
            RowSelection {
                selected: BTreeSet::from([at]),
                anchor: Some(at),
            },
        );
        Ok(())
    }

    pub fn add_independent(&mut self, row: usize) -> bool {
        let Some(signal) = self.signal(row).filter(|s| s.projection.is_some()) else {
            return false;
        };
        let mut copy = signal.clone();
        copy.tint = self.ink(row);
        copy.bus = copy.bus.map(|_| super::bus::BusExpansion::default());
        let mut parent = row;
        while let Some(p) =
            tree::parent(&self.items, parent).filter(|&p| self.items[p].signal().is_some())
        {
            parent = p;
        }
        let at = tree::subtree_end(&self.items, parent);
        self.edit_rows(
            "Add independent trace".into(),
            vec![Splice {
                at,
                remove: 0,
                insert: vec![Entry::new(self.items[parent].depth, WaveRow::from(copy))],
            }],
            RowSelection {
                selected: BTreeSet::from([at]),
                anchor: Some(at),
            },
        );
        true
    }

    pub(crate) fn projection_demand(&self) -> impl Iterator<Item = &DisplayedSignal> {
        let row_h = self.last_layout().row_h.max(1.0);
        let height = if self.last_layout().names.height() > 0.0 {
            self.last_layout().names.height()
        } else {
            32.0 * row_h
        };
        let mut top = 0.0;
        let wanted: BTreeSet<_> = self
            .visible()
            .iter()
            .filter_map(|&i| {
                let bottom = top + f32::from(self.items[i as usize].height().multiple()) * row_h;
                let visible = bottom > self.scroll_y && top < self.scroll_y + height;
                top = bottom;
                visible.then_some(i as usize)
            })
            .chain(self.selected.iter().copied())
            .collect();
        self.items
            .iter()
            .enumerate()
            .filter(move |(i, _)| wanted.contains(i))
            .filter_map(|(_, e)| e.signal())
            .filter(|s| s.projection.is_some())
    }

    pub(crate) fn sync_projections(&mut self, projections: &crate::data::projection::Projections) {
        for s in self.signals_mut().filter(|s| s.projection.is_some()) {
            if let Some(result) = s
                .raw_history()
                .and_then(|raw| projections.get(raw, s.projection.unwrap()))
            {
                s.history = result.as_ref().ok().cloned();
                s.error = result.as_ref().err().cloned();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_indices_round_trip_in_both_directions_without_renumbering() {
        for name in [
            "data [7:0]",
            "data[4:11]",
            "data[3:-4]",
            "data[-4:3]",
            "data",
        ] {
            let indices = BusIndices::new(name, 8);
            for first in 0..8 {
                for last in 0..8 {
                    let slice = BitSlice { first, last };
                    let text = format!("{}:{}", indices.index(first), indices.index(last));
                    assert_eq!(indices.parse(&text).unwrap(), slice);
                    assert_eq!(indices.parse(&format!("[{text}]")).unwrap(), slice);
                }
            }
            assert!(indices.parse("42").is_err());
            assert!(indices.parse("1:2:3").is_err());
        }
        let indices = BusIndices::new("data[-9223372036854775808:9223372036854775807]", 8);
        assert!(indices.parse("-1").is_err());
    }
}
