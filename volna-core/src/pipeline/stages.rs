//! Primary-lane stage visibility state, dimming rules, and navigation anchor resolution.

use std::collections::BTreeSet;

use super::order::RowSortMeta;
use crate::wave::viewport::Viewport;
use volna_trace::data::transactions::{Transaction, TransactionStage};

/// Fill opacity multiplier applied to hidden `primary_lane` stages so they recede into the
/// background while still indicating a hidden stage rather than an idle gap.
pub const HIDDEN_STAGE_FILL_ALPHA: f32 = 0.08;

/// Border opacity multiplier applied to hidden `primary_lane` stages.
pub const HIDDEN_STAGE_EDGE_ALPHA: f32 = 0.14;

/// Hatch opacity applied to hidden flushed (`Status::Aborted`) `primary_lane` stages.
pub const HIDDEN_STAGE_HATCH_ALPHA: f32 = 0.10;

/// Opacity multiplier applied to the background ghost presence mask in zoomed-out overview mode
/// when one or more `primary_lane` stages are hidden.
pub const HIDDEN_STAGE_OVERVIEW_ALPHA: f32 = 0.12;

/// Hidden `primary_lane` stage names for a pipeline panel.
///
/// Hidden stages remain in the row layout, paint heavily dimmed without text or accent ticks,
/// and are ignored when computing scrolling and `FollowActivity` anchor timestamps.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct StageVisibility {
    hidden: BTreeSet<String>,
}

impl StageVisibility {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty()
    }

    #[must_use]
    pub fn hidden_count(&self) -> usize {
        self.hidden.len()
    }

    #[must_use]
    pub fn hidden(&self) -> &BTreeSet<String> {
        &self.hidden
    }

    #[must_use]
    pub fn is_hidden(&self, stage_name: &str) -> bool {
        self.hidden.contains(stage_name)
    }

    #[must_use]
    pub fn is_stage_hidden(&self, stage: &TransactionStage, primary_lane: &str) -> bool {
        stage.lane == primary_lane && self.hidden.contains(&stage.name)
    }

    pub fn toggle(&mut self, stage: String) -> bool {
        if stage.is_empty() {
            return false;
        }
        if !self.hidden.remove(&stage) {
            self.hidden.insert(stage);
        }
        true
    }

    pub fn set_hidden(&mut self, stage: String, hidden: bool) -> bool {
        if stage.is_empty() {
            return false;
        }
        if hidden {
            self.hidden.insert(stage)
        } else {
            self.hidden.remove(&stage)
        }
    }

    pub fn show_all(&mut self) -> bool {
        if self.hidden.is_empty() {
            return false;
        }
        self.hidden.clear();
        true
    }

    pub fn normalize(&mut self) {
        self.hidden.retain(|s| !s.is_empty());
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        self.hidden.iter().all(|s| !s.is_empty())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageVisibilityCommand {
    Toggle(String),
    SetHidden { stage: String, hidden: bool },
    ShowAll,
}

/// Active primary-lane stage visibility filter passed to navigation and `FollowActivity` queries.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StageAnchorFilter<'a> {
    pub visibility: &'a StageVisibility,
    pub primary_lane: &'a str,
}

impl StageAnchorFilter<'_> {
    #[must_use]
    pub(crate) fn is_stage_hidden(self, stage: &TransactionStage) -> bool {
        self.visibility.is_stage_hidden(stage, self.primary_lane)
    }

    /// Start timestamp of the earliest visible (non-hidden) `primary_lane` stage in `tx`.
    #[must_use]
    pub(crate) fn first_visible_stage_begin(self, tx: &Transaction) -> Option<u64> {
        tx.stages
            .iter()
            .filter(|s| s.lane == self.primary_lane && !self.visibility.is_hidden(&s.name))
            .map(|s| s.begin)
            .min()
    }

    /// End timestamp of the latest visible (non-hidden) `primary_lane` stage in `tx`.
    #[must_use]
    pub(crate) fn last_visible_stage_end(self, tx: &Transaction) -> Option<u64> {
        tx.stages
            .iter()
            .filter(|s| s.lane == self.primary_lane && !self.visibility.is_hidden(&s.name))
            .map(|s| s.end.unwrap_or(tx.end).max(s.begin))
            .max()
    }
}

/// Returns whether `tx` overlaps `view` through either its owner lifetime `[tx.begin, tx.end]`
/// or any recorded stage (including stages that extend outside their owner's lifetime).
#[must_use]
pub(crate) fn tx_overlaps_view(tx: &Transaction, view: Viewport) -> bool {
    if (tx.begin as f64) <= view.end && (tx.end as f64) >= view.start {
        return true;
    }
    tx.stages.iter().any(|s| {
        let end = s.end.unwrap_or(tx.end).max(s.begin);
        (s.begin as f64) <= view.end && (end as f64) >= view.start
    })
}

/// Resolved navigation anchor shared across row scrolling, navigator jumps, and `FollowActivity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowAnchor {
    /// Point timestamp (matched visible sort stage, first visible stage, or transaction fallback).
    Point { time: u64, priority: u8 },
    /// Full transaction lifetime `[begin, end]` (used when no stages are hidden in `Start` order).
    Lifetime { begin: u64, end: u64, priority: u8 },
}

impl RowAnchor {
    #[must_use]
    pub(crate) fn timestamp(self) -> u64 {
        match self {
            Self::Point { time, .. } => time,
            Self::Lifetime { begin, .. } => begin,
        }
    }

    #[must_use]
    pub(crate) fn distance_from(self, anchor: f64) -> (u8, f64) {
        match self {
            Self::Point { time, priority } => (priority, (time as f64 - anchor).abs()),
            Self::Lifetime {
                begin,
                end,
                priority,
            } => (
                priority,
                (begin as f64 - anchor).max(anchor - end as f64).max(0.0),
            ),
        }
    }
}

/// Resolve the navigation anchor for a transaction row.
///
/// When primary-lane stages are hidden, anchors on the matched visible sort stage (if stage-ordered
/// with a visible sort stage) or else the start of the first visible `primary_lane` stage. When
/// all primary-lane stages of `tx` are hidden, falls back cleanly to a non-stage sort timestamp
/// (`tx.end` / unmatched fallback) or `tx.begin` — never reusing a hidden stage's timestamp.
#[must_use]
pub(crate) fn resolve_row_anchor(
    tx: &Transaction,
    meta: Option<RowSortMeta>,
    stage_filter: Option<StageAnchorFilter<'_>>,
) -> RowAnchor {
    if let Some(filter) = stage_filter {
        if let Some(m) = meta
            && m.matched
            && let Some(idx) = m.sort_stage_index()
            && let Some(stage) = tx.stages.get(idx)
            && !filter.is_stage_hidden(stage)
        {
            return RowAnchor::Point {
                time: m.sort_key,
                priority: 0,
            };
        }
        if let Some(vis_begin) = filter.first_visible_stage_begin(tx) {
            return RowAnchor::Point {
                time: vis_begin,
                priority: 0,
            };
        }
        let fallback_time = match meta {
            Some(m) if m.sort_stage_index().is_none() => m.sort_key,
            _ => tx.begin,
        };
        return RowAnchor::Point {
            time: fallback_time,
            priority: 0,
        };
    }
    if let Some(m) = meta {
        RowAnchor::Point {
            time: m.sort_key,
            priority: u8::from(!m.matched),
        }
    } else {
        RowAnchor::Lifetime {
            begin: tx.begin,
            end: tx.end,
            priority: 0,
        }
    }
}

/// Scalar anchor timestamp for a transaction during row scrolling and navigator jumps.
#[must_use]
pub(crate) fn row_anchor_time(
    tx: &Transaction,
    meta: Option<RowSortMeta>,
    stage_filter: Option<StageAnchorFilter<'_>>,
) -> u64 {
    resolve_row_anchor(tx, meta, stage_filter).timestamp()
}

/// Ranking score `(priority, time_distance, row_distance, row)` for `FollowActivity` target selection.
/// Lower tuples win.
#[must_use]
pub(crate) fn activity_target_score(
    row: usize,
    tx: &Transaction,
    meta: Option<RowSortMeta>,
    stage_filter: Option<StageAnchorFilter<'_>>,
    anchor: f64,
    center: f64,
) -> (u8, f64, f64, usize) {
    let row_dist = (row as f64 + 0.5 - center).abs();
    let (priority, time_dist) = resolve_row_anchor(tx, meta, stage_filter).distance_from(anchor);
    (priority, time_dist, row_dist, row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_trace::data::transactions::{SpanKind, Status, TrackRef, TransactionRef};

    fn tx_with_stages(begin: u64, end: u64, stages: &[(&str, &str, u64, u64)]) -> Transaction {
        Transaction {
            id: TransactionRef(1),
            generator: TrackRef(1),
            begin,
            end,
            status: Status::Ok,
            kind: SpanKind::Unspecified,
            parent: None,
            attributes: vec![],
            events: vec![],
            stages: stages
                .iter()
                .enumerate()
                .map(|(k, &(name, lane, sb, se))| TransactionStage {
                    id: TransactionRef(100 + k as u64),
                    generator: None,
                    name: name.into(),
                    lane: lane.into(),
                    begin: sb,
                    end: Some(se),
                    attributes: vec![],
                })
                .collect(),
        }
    }

    // User scenario: an engineer hides early primary-lane stages while keeping overlay stages
    // and later primary-lane stages visible, or hides every stage of a transaction.
    // Requirement: navigation anchors on the start of the first visible primary-lane stage,
    // ignores overlay lanes, and falls back to the normal transaction timestamp when all stages
    // are hidden.
    #[test]
    fn anchor_uses_first_visible_primary_stage_and_falls_back_when_all_hidden() {
        let tx = tx_with_stages(
            100,
            160,
            &[
                ("fetch", "0", 100, 120),
                ("stall", "1", 105, 115),
                ("decode", "0", 120, 135),
                ("exec", "0", 135, 160),
            ],
        );

        let mut vis = StageVisibility::default();
        assert!(vis.is_empty());
        assert_eq!(row_anchor_time(&tx, None, None), 100);

        // Hiding `fetch` skips both `fetch` and overlay `stall`, anchoring on `decode` at 120.
        assert!(vis.toggle("fetch".into()));
        let filter = StageAnchorFilter {
            visibility: &vis,
            primary_lane: "0",
        };
        assert_eq!(row_anchor_time(&tx, None, Some(filter)), 120);
        // Overlay lane stages with the same name as a hidden stage are never considered hidden.
        let overlay_same_name = tx_with_stages(
            100,
            160,
            &[("fetch", "1", 100, 110), ("exec", "0", 140, 160)],
        );
        assert!(!filter.is_stage_hidden(&overlay_same_name.stages[0]));
        assert_eq!(row_anchor_time(&overlay_same_name, None, Some(filter)), 140);

        // Hiding `decode` as well advances the anchor to `exec` at 135.
        assert!(vis.toggle("decode".into()));
        let filter = StageAnchorFilter {
            visibility: &vis,
            primary_lane: "0",
        };
        assert_eq!(row_anchor_time(&tx, None, Some(filter)), 135);

        // Hiding `exec` leaves all primary stages hidden -> falls back cleanly to `tx.begin` (100).
        assert!(vis.toggle("exec".into()));
        let filter = StageAnchorFilter {
            visibility: &vis,
            primary_lane: "0",
        };
        assert_eq!(row_anchor_time(&tx, None, Some(filter)), 100);

        // Show all clears hidden set and returns false if already empty.
        assert!(vis.show_all());
        assert!(vis.is_empty());
        assert!(!vis.show_all());
    }
}
