//! `TransactionModel`: the state of a Transaction panel.

use std::collections::BTreeMap;

use web_time::Instant;

use super::parents::{ParentTrace, ParentTraceState};
use super::view::{SectionKey, TxView, VIEW_BYTES, ViewPrefs};
use crate::data::text::{COPY_BYTES, Radix};
use crate::document::{Document, TrackLoadState, TxSelection};
use crate::history::{Before, Edit, History, MergeKey};
use crate::pipeline::TrackSource;
use crate::trace::Traced;
use crate::wave::viewport::Viewport;
use volna_trace::data::transactions::{TrackRef, TransactionRef};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

pub const PANEL_BYTES: u64 = VIEW_BYTES as u64;
pub const MAX_HISTORY: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownRecord {
    pub track: TrackSource,
    pub id: TransactionRef,
}

impl ShownRecord {
    #[must_use]
    pub fn selection(&self, origin: crate::panels::PanelId) -> Option<TxSelection> {
        Some(TxSelection {
            track: self.track.track()?,
            id: self.id,
            origin,
        })
    }
}

pub enum TxPanelState {
    Empty,
    Refused(String),
    Loading,
    Failed(String),
    Missing,
    Ready(Box<TxView>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransactionCommand {
    Jump {
        track: Traced<TrackRef>,
        id: TransactionRef,
    },
    Back,
    Forward,
    Pin(bool),
    Cursor(u64),
    FitLifetime,
    Radix(String),
    Collapse(SectionKey, bool),
    Filter(String),
    TraceParents,
    ClearParents,
}

pub struct TransactionModel {
    history: Vec<ShownRecord>,
    cursor: usize,
    pinned: bool,
    prefs: ViewPrefs,
    pin_before: Before<(bool, Option<ShownRecord>)>,
    radix_before: Before<BTreeMap<String, Radix>>,
    retained: Option<Traced<TrackRef>>,
    attached: bool,
    refused: Option<String>,
    budget: MemoryBudget,
    reservation: Option<Reservation>,
    parents: ParentTrace,
}

impl TransactionModel {
    #[must_use]
    pub fn new(budget: MemoryBudget, detail_items: usize) -> Self {
        Self {
            history: Vec::new(),
            cursor: 0,
            pinned: false,
            pin_before: Before::default(),
            radix_before: Before::default(),
            prefs: ViewPrefs {
                detail_items: detail_items.clamp(1, 1000),
                ..ViewPrefs::default()
            },
            retained: None,
            attached: false,
            refused: None,
            budget,
            reservation: None,
            parents: ParentTrace::default(),
        }
    }

    #[must_use]
    pub fn clone_view(&self) -> Self {
        let mut clone = Self::new(self.budget.clone(), self.prefs.detail_items);
        clone.prefs = self.prefs.clone();
        clone.pinned = self.pinned;
        if let Some(shown) = self.shown() {
            clone.history = vec![shown.clone()];
        }
        clone
    }

    #[must_use]
    pub fn shown(&self) -> Option<&ShownRecord> {
        self.history.get(self.cursor)
    }

    #[must_use]
    pub fn parents(&self) -> &ParentTrace {
        &self.parents
    }

    pub(crate) fn refresh_parents(&mut self, doc: &mut Document) {
        self.parents.refresh(doc, &self.budget);
    }

    #[must_use]
    pub fn attribute_key(
        &self,
        doc: &Document,
        id: TransactionRef,
        index: usize,
    ) -> Option<String> {
        let shown = self.shown()?;
        if shown.id != id {
            return None;
        }
        let generator = doc.resident_generator(shown.track.track()?)?;
        let tx = generator.transaction(shown.id)?;
        Some(tx.attributes.get(index)?.key.clone())
    }

    pub(crate) fn forget_trace(&mut self, doc: &mut Document, trace: crate::trace::TraceId) {
        if self
            .shown()
            .is_some_and(|record| record.track.trace() == trace)
        {
            self.parents.clear(doc);
        }
        let shown = self.shown().cloned();
        self.history.retain(|r| r.track.trace() != trace);
        self.cursor = shown
            .and_then(|s| self.history.iter().position(|r| *r == s))
            .unwrap_or(self.history.len().saturating_sub(1));
        let track = self.shown().and_then(|r| r.track.track());
        if !self.hold(doc, track)
            && let Some(track) = self.retained.take()
        {
            doc.release_track(track);
        }
    }

    #[must_use]
    pub fn pinned(&self) -> bool {
        self.pinned
    }

    pub(crate) fn swap_pin(
        &mut self,
        doc: &mut Document,
        (pinned, record): (bool, Option<ShownRecord>),
    ) -> (bool, Option<ShownRecord>) {
        debug_assert!(!self.pin_before.is_held(), "edits were not collected");
        let old = (self.pinned, self.shown().cloned());
        self.pinned = pinned;
        if !pinned {
            self.follow_selection(doc);
        } else if let Some(record) = record {
            self.show(doc, record);
        }
        old
    }

    pub(crate) fn swap_radix(&mut self, radix: BTreeMap<String, Radix>) -> BTreeMap<String, Radix> {
        debug_assert!(!self.radix_before.is_held(), "edits were not collected");
        std::mem::replace(&mut self.prefs.radix, radix)
    }

    #[must_use]
    pub fn prefs(&self) -> &ViewPrefs {
        &self.prefs
    }

    pub(crate) fn take_edits(&mut self, panel: crate::panels::PanelId, history: &mut History) {
        if let Some((pinned, record)) = self.pin_before.take()
            && pinned != self.pinned
        {
            let label = if pinned {
                "Unpin transaction"
            } else {
                "Pin transaction"
            };
            history.record(
                Edit::Prop {
                    panel,
                    prop: crate::panels::content::Prop::Transaction(Prop::Pin(pinned, record)),
                },
                Some(label.into()),
            );
        }
        if let Some(radix) = self.radix_before.take()
            && radix != self.prefs.radix
        {
            let now = &self.prefs.radix;
            let key = radix
                .keys()
                .chain(now.keys())
                .find(|k| radix.get(*k) != now.get(*k))
                .cloned()
                .unwrap_or_default();
            history.record(
                Edit::Prop {
                    panel,
                    prop: crate::panels::content::Prop::Transaction(Prop::Radix(radix)),
                },
                Some(format!("Radix of {key}")),
            );
            history.set_merge(MergeKey::of("radix", &key).in_panel(panel));
        }
    }

    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.cursor > 0
    }

    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        self.cursor + 1 < self.history.len()
    }

    pub fn set_detail_items(&mut self, value: usize) {
        self.prefs.detail_items = value.clamp(1, 1000);
    }

    pub(crate) fn restore(&mut self, record: Option<ShownRecord>, pinned: bool, prefs: ViewPrefs) {
        self.history = record.into_iter().collect();
        self.cursor = 0;
        self.pinned = pinned;
        self.restore_prefs(prefs);
    }

    fn restore_prefs(&mut self, prefs: ViewPrefs) {
        let detail_items = self.prefs.detail_items;
        self.prefs = ViewPrefs {
            detail_items,
            ..prefs
        };
    }

    #[must_use]
    pub fn is_attached(&self) -> bool {
        self.attached
    }

    pub fn attach(&mut self, doc: &mut Document) -> anyhow::Result<()> {
        if self.attached {
            return Ok(());
        }
        self.attached = true;
        if self.reservation.is_none() {
            match self.budget.reserve(PANEL_BYTES) {
                Ok(reservation) => {
                    self.reservation = Some(reservation);
                    self.refused = None;
                }
                Err(error) => {
                    self.refused = Some(format!(
                        "Transaction panel needs {PANEL_BYTES} bytes; admission failed: {error}"
                    ));
                }
            }
        }
        if self.refused.is_some() {
            return Ok(());
        }
        if let Some(track) = self.shown().and_then(|shown| shown.track.track()) {
            doc.retain_track(track)?;
            self.retained = Some(track);
        }
        Ok(())
    }

    pub fn detach(&mut self, doc: &mut Document) {
        if !self.attached {
            return;
        }
        self.parents.clear(doc);
        if let Some(track) = self.retained.take() {
            doc.release_track(track);
        }
        self.attached = false;
    }

    pub(crate) fn park(&mut self, doc: &mut Document) {
        self.detach(doc);
        self.reservation = None;
    }

    fn hold(&mut self, doc: &mut Document, track: Option<Traced<TrackRef>>) -> bool {
        if !self.attached || self.retained == track {
            return true;
        }
        if let Some(track) = track
            && doc.retain_track(track).is_err()
        {
            return false;
        }
        if let Some(previous) = std::mem::replace(&mut self.retained, track) {
            doc.release_track(previous);
        }
        true
    }

    fn show(&mut self, doc: &mut Document, record: ShownRecord) -> bool {
        if self.shown() == Some(&record) || !self.hold(doc, record.track.track()) {
            return false;
        }
        self.parents.clear(doc);
        self.history.truncate(self.cursor + 1);
        self.history.push(record);
        if self.history.len() > MAX_HISTORY {
            let excess = self.history.len() - MAX_HISTORY;
            self.history.drain(..excess);
        }
        self.cursor = self.history.len() - 1;
        true
    }

    fn step(&mut self, doc: &mut Document, panel: crate::panels::PanelId, cursor: usize) -> bool {
        let Some(track) = self.history.get(cursor).map(|r| r.track.track()) else {
            return false;
        };
        if !self.hold(doc, track) {
            return false;
        }
        self.parents.clear(doc);
        self.cursor = cursor;
        if let Some(selection) = self.shown().and_then(|shown| shown.selection(panel)) {
            doc.select(Some(selection));
        }
        true
    }

    pub fn follow_selection(&mut self, doc: &mut Document) -> bool {
        if self.pinned {
            return false;
        }
        let Some(selection) = doc.selection() else {
            return false;
        };
        let Some(path) = track_path(doc, selection.track) else {
            return false;
        };
        self.show(
            doc,
            ShownRecord {
                track: TrackSource::Resolved {
                    track: selection.track,
                    path,
                },
                id: selection.id,
            },
        )
    }

    #[must_use]
    pub fn state(&self, doc: &Document) -> TxPanelState {
        if let Some(refusal) = &self.refused {
            return TxPanelState::Refused(refusal.clone());
        }
        let Some(shown) = self.shown() else {
            return TxPanelState::Empty;
        };
        let Some(track) = shown.track.track() else {
            return TxPanelState::Missing;
        };
        match doc.track(track) {
            Some(TrackLoadState::Loading) => return TxPanelState::Loading,
            Some(TrackLoadState::Failed(error)) => return TxPanelState::Failed(error.clone()),
            _ => {}
        }
        match super::view::view(doc, track, shown.id, &self.prefs) {
            Some(view) => TxPanelState::Ready(Box::new(view)),
            None if doc.track(track).is_none() => TxPanelState::Loading,
            None => TxPanelState::Missing,
        }
    }

    pub fn copy_tsv(&self, doc: &Document) -> anyhow::Result<String> {
        let shown = self
            .shown()
            .ok_or_else(|| anyhow::anyhow!("No record is shown."))?;
        let track = shown
            .track
            .track()
            .ok_or_else(|| anyhow::anyhow!("Record is not loaded."))?;
        super::view::copy_tsv(doc, track, shown.id, COPY_BYTES)
    }

    #[must_use]
    pub fn lifetime(&self, doc: &Document) -> Option<(u64, u64)> {
        let shown = self.shown()?;
        let generator = doc.resident_generator(shown.track.track()?)?;
        let tx = generator.transaction(shown.id)?;
        Some((tx.begin, tx.end))
    }

    pub fn command(
        &mut self,
        doc: &mut Document,
        panel: crate::panels::PanelId,
        command: TransactionCommand,
        now: Instant,
    ) -> bool {
        match command {
            TransactionCommand::TraceParents => {
                if !self.attached
                    || self.refused.is_some()
                    || matches!(
                        self.parents.state(),
                        ParentTraceState::Loading | ParentTraceState::Ready
                    )
                {
                    return false;
                }
                let Some((track, id)) = self
                    .shown()
                    .and_then(|shown| Some((shown.track.track()?, shown.id)))
                else {
                    return false;
                };
                if doc.resident_generator(track).is_none() {
                    return false;
                }
                self.parents.start(doc, &self.budget, track, id);
                true
            }
            TransactionCommand::ClearParents => {
                let changed = *self.parents.state() != ParentTraceState::Idle;
                self.parents.clear(doc);
                changed
            }
            TransactionCommand::Jump { track, id } => {
                let Some(path) = track_path(doc, track) else {
                    return false;
                };
                let changed = self.show(
                    doc,
                    ShownRecord {
                        track: TrackSource::Resolved { track, path },
                        id,
                    },
                );
                doc.select(Some(TxSelection {
                    track,
                    id,
                    origin: panel,
                }));
                changed
            }
            TransactionCommand::Back => {
                self.can_go_back() && self.step(doc, panel, self.cursor - 1)
            }
            TransactionCommand::Forward => {
                self.can_go_forward() && self.step(doc, panel, self.cursor + 1)
            }
            TransactionCommand::Pin(pinned) => {
                let changed = self.pinned != pinned;
                if changed {
                    let (was, shown) = (self.pinned, self.shown().cloned());
                    self.pin_before.note(|| (was, shown));
                }
                self.pinned = pinned;
                if changed && !pinned {
                    self.follow_selection(doc);
                }
                changed
            }
            TransactionCommand::Cursor(time) => {
                let changed = doc.shared.cursor != Some(time);
                doc.shared.cursor = Some(time);
                changed
            }
            TransactionCommand::FitLifetime => {
                let Some((begin, end)) = self.lifetime(doc) else {
                    return false;
                };
                let pad = ((end.saturating_sub(begin)) as f64 * 0.05).max(1.0);
                let mut target = Viewport {
                    start: begin as f64 - pad,
                    end: end as f64 + pad,
                };
                target.clamp(doc.limits());
                let animation = doc.navigation.animation;
                doc.shared.viewport.animate_to(target, now, animation);
                true
            }
            TransactionCommand::Radix(key) => {
                let radix = &self.prefs.radix;
                self.radix_before.note(|| radix.clone());
                let next = self.prefs.radix_of(&key).next();
                if next == Radix::default() {
                    self.prefs.radix.remove(&key);
                } else {
                    self.prefs.radix.insert(key, next);
                }
                true
            }
            TransactionCommand::Collapse(section, collapsed) => {
                if collapsed {
                    self.prefs.collapsed.insert(section)
                } else {
                    self.prefs.collapsed.remove(&section)
                }
            }
            TransactionCommand::Filter(text) => {
                let changed = self.prefs.filter != text;
                self.prefs.filter = text;
                changed
            }
        }
    }
}

fn track_path(doc: &Document, track: Traced<TrackRef>) -> Option<Vec<String>> {
    doc.session(track.trace)?
        .tracks()
        .iter()
        .find(|t| t.id == track.item)
        .map(|t| t.path.clone())
}

pub(crate) enum Prop {
    Pin(bool, Option<ShownRecord>),
    Radix(BTreeMap<String, Radix>),
}

impl Prop {
    pub(crate) fn kind(&self) -> u8 {
        match self {
            Self::Pin(..) => 0,
            Self::Radix(_) => 1,
        }
    }
    pub(crate) fn bytes(&self) -> usize {
        match self {
            Self::Pin(..) => 64,
            Self::Radix(map) => map.keys().map(|k| k.len() + 32).sum(),
        }
    }
}

impl TransactionModel {
    pub(crate) fn matches_prop(&self, prop: &Prop) -> bool {
        match prop {
            Prop::Pin(pinned, _) => self.pinned() == *pinned,
            Prop::Radix(radix) => self.prefs().radix == *radix,
        }
    }

    pub(crate) fn swap_prop(&mut self, doc: &mut Document, prop: Prop) -> Prop {
        match prop {
            Prop::Pin(pinned, record) => {
                let (pinned, record) = self.swap_pin(doc, (pinned, record));
                Prop::Pin(pinned, record)
            }
            Prop::Radix(radix) => Prop::Radix(self.swap_radix(radix)),
        }
    }
}
