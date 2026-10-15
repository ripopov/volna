//! Panel identity, ownership, and docking operations.

pub(crate) mod content;
mod dock;
mod layout;
pub use dock::{DockLayout, Placement, SideDock};
mod timeline;
pub(crate) mod workspace;
pub use layout::{Axis, Layout, MAX_LAYOUT_DEPTH, MAX_PANELS};
pub use timeline::TimelineDivider;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use web_time::Instant;

use crate::document::Document;
use crate::history::{Journaled, Structure};
use crate::nav::{LinkDim, NavState};
use crate::pipeline::PipelineModel;
use crate::table::TableModel;
use crate::transaction::TransactionModel;
use crate::wave::model::{PointerEvent, WaveModel};

#[derive(Clone, Debug, PartialEq)]
pub enum PanelsCommand {
    Move {
        panel: PanelId,
        placement: Placement,
    },
    ToggleRightDock,
    Split {
        panel: PanelId,
        axis: Axis,
    },
    NewTab {
        group_of: PanelId,
    },
    Close(PanelId),
    CloseOthers(PanelId),
    Focus(PanelId),
    FocusNext,
    FocusPrev,
    FocusIndex(usize),
    ToggleLink {
        panel: PanelId,
        dim: LinkDim,
    },
    Rename(PanelId, Option<String>),
    SetLayout {
        layout: DockLayout,
        from_revision: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PanelId(pub u64);

pub enum PanelKind {
    Waves(Box<WaveModel>),
    Pipeline(Box<PipelineModel>),
    Table(Box<TableModel>),
    Transaction(Box<TransactionModel>),
    Start,
    Settings,
    Unsupported(Box<serde_json::value::RawValue>),
}

impl PanelKind {
    #[must_use]
    pub fn waves(&self) -> Option<&WaveModel> {
        match self {
            Self::Waves(w) => Some(w),
            _ => None,
        }
    }

    pub fn waves_mut(&mut self) -> Option<&mut WaveModel> {
        match self {
            Self::Waves(w) => Some(w),
            _ => None,
        }
    }

    #[must_use]
    pub fn pipeline(&self) -> Option<&PipelineModel> {
        match self {
            Self::Pipeline(p) => Some(p),
            _ => None,
        }
    }

    pub fn pipeline_mut(&mut self) -> Option<&mut PipelineModel> {
        match self {
            Self::Pipeline(p) => Some(p),
            _ => None,
        }
    }

    #[must_use]
    pub fn table(&self) -> Option<&TableModel> {
        match self {
            Self::Table(table) => Some(table),
            _ => None,
        }
    }

    pub fn table_mut(&mut self) -> Option<&mut TableModel> {
        match self {
            Self::Table(table) => Some(table),
            _ => None,
        }
    }

    #[must_use]
    pub fn transaction(&self) -> Option<&TransactionModel> {
        match self {
            Self::Transaction(model) => Some(model),
            _ => None,
        }
    }

    pub fn transaction_mut(&mut self) -> Option<&mut TransactionModel> {
        match self {
            Self::Transaction(model) => Some(model),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_settings(&self) -> bool {
        matches!(self, Self::Settings)
    }

    #[must_use]
    pub fn is_start(&self) -> bool {
        matches!(self, Self::Start)
    }

    #[must_use]
    pub fn is_content(&self) -> bool {
        !self.is_settings()
    }

    #[must_use]
    pub fn is_canvas(&self) -> bool {
        matches!(self, Self::Waves(_) | Self::Pipeline(_) | Self::Table(_))
    }

    #[must_use]
    pub fn shows_rulers(&self) -> bool {
        matches!(self, Self::Waves(_) | Self::Pipeline(_))
    }

    #[must_use]
    pub fn nav(&self) -> Option<&NavState> {
        match self {
            Self::Waves(w) => Some(&w.nav),
            Self::Pipeline(p) => Some(&p.nav),
            Self::Table(t) => Some(&t.nav),
            _ => None,
        }
    }

    pub fn nav_mut(&mut self) -> Option<&mut NavState> {
        match self {
            Self::Waves(w) => Some(&mut w.nav),
            Self::Pipeline(p) => Some(&mut p.nav),
            Self::Table(t) => Some(&mut t.nav),
            _ => None,
        }
    }

    #[must_use]
    pub fn trace(&self) -> Option<crate::trace::TraceId> {
        match self {
            Self::Pipeline(p) => Some(p.track.trace()),
            Self::Table(t) => {
                let mut traces = t.source.traces();
                let first = traces.next()?;
                traces.all(|t| t == first).then_some(first)
            }
            Self::Transaction(m) => m.shown().map(|r| r.track.trace()),
            Self::Waves(_) | Self::Start | Self::Settings | Self::Unsupported(_) => None,
        }
    }

    #[must_use]
    pub fn shows_trace(&self, trace: crate::trace::TraceId) -> bool {
        match self {
            Self::Pipeline(p) => p.track.trace() == trace,
            Self::Table(t) => t.source.traces().any(|t| t == trace),
            Self::Transaction(m) => {
                m.pinned() && m.shown().is_some_and(|r| r.track.trace() == trace)
            }
            Self::Waves(_) | Self::Start | Self::Settings | Self::Unsupported(_) => false,
        }
    }

    #[must_use]
    pub fn marker_lane(&self) -> Option<&crate::wave::overlay::MarkerLane> {
        match self {
            Self::Waves(w) => Some(&w.last_layout().marker_lane),
            Self::Pipeline(p) => Some(&p.last_layout().marker_lane),
            _ => None,
        }
    }

    fn clone_view(&self, split: bool) -> Result<PanelKind> {
        Ok(match (self, split) {
            (Self::Waves(w), _) => Self::Waves(Box::new(w.clone_view(split))),
            (Self::Pipeline(p), true) => Self::Pipeline(Box::new(p.clone_view())),
            (Self::Table(t), true) => Self::Table(Box::new(t.clone_view())),
            (Self::Transaction(model), true) => Self::Transaction(Box::new(model.clone_view())),
            (Self::Start, _) => bail!("the start panel has no content to copy"),
            (_, false) => Self::Waves(Box::default()),
            _ => bail!("cannot split this panel"),
        })
    }
}

impl crate::trace::Retime for PanelKind {
    fn retime(&mut self, by: crate::trace::Rescale) {
        if let Some(nav) = self.nav_mut() {
            nav.retime(by);
        }
        if let Self::Table(t) = self {
            t.retime(by);
        }
    }
}

pub struct Panel {
    pub id: PanelId,
    pub title: Journaled<Option<String>>,
    pub kind: PanelKind,
}

impl Panel {
    fn new(id: PanelId, kind: PanelKind) -> Self {
        Self {
            id,
            title: Journaled::default(),
            kind,
        }
    }

    #[must_use]
    pub fn title(&self) -> String {
        self.title
            .get()
            .clone()
            .unwrap_or_else(|| match &self.kind {
                PanelKind::Waves(_) => format!("Waves {}", self.id.0),
                PanelKind::Pipeline(p) => {
                    format!("Pipeline {} · {}", self.id.0, p.track.path().join("."))
                }
                PanelKind::Table(t) => match &t.source {
                    crate::table::TableSource::Generator(source) => {
                        format!("Table {} · {}", self.id.0, source.path().join("."))
                    }
                    crate::table::TableSource::Signals(signals) => {
                        format!("Table {} · {} signals", self.id.0, signals.len())
                    }
                },
                PanelKind::Transaction(model) => match model.shown() {
                    Some(shown) => format!("Transaction {} · #{}", self.id.0, shown.id.0),
                    None => format!("Transaction {}", self.id.0),
                },
                PanelKind::Start => "Start".into(),
                PanelKind::Settings => "Settings".into(),
                PanelKind::Unsupported(_) => format!("Unsupported panel {}", self.id.0),
            })
    }

    pub fn pointer(&mut self, doc: &mut Document, event: PointerEvent, now: Instant) -> bool {
        let id = self.id;
        match &mut self.kind {
            PanelKind::Waves(w) => w.pointer(doc, id, event, now),
            PanelKind::Pipeline(p) => p.pointer(doc, id, event, now),
            PanelKind::Table(t) => t.pointer(doc, id, event, now),
            _ => false,
        }
    }

    #[must_use]
    pub fn dragging(&self) -> bool {
        match &self.kind {
            PanelKind::Waves(w) => w.drag.is_some(),
            PanelKind::Pipeline(p) => p.drag.is_some(),
            PanelKind::Table(t) => t.dragging(),
            _ => false,
        }
    }

    pub(crate) fn take_edits(
        &mut self,
        clocks: &crate::clock::Clocks,
        history: &mut crate::history::History,
    ) {
        let id = self.id;
        if let Some(title) = self.title.take_before() {
            history.record(
                crate::history::Edit::Prop {
                    panel: id,
                    prop: crate::panels::content::Prop::Title(title),
                },
                Some("Rename panel".into()),
            );
        }
        if let Some(nav) = self.kind.nav_mut() {
            nav.take_edits(id, clocks, history);
        }
        match &mut self.kind {
            PanelKind::Waves(w) => w.take_edits(id, history),
            PanelKind::Table(t) => t.take_edits(id, history),
            PanelKind::Transaction(t) => t.take_edits(id, history),
            PanelKind::Pipeline(_)
            | PanelKind::Start
            | PanelKind::Settings
            | PanelKind::Unsupported(_) => {}
        }
    }

    pub(crate) fn cancel_drag(&mut self) {
        match &mut self.kind {
            PanelKind::Waves(w) => w.drag = None,
            PanelKind::Pipeline(p) => p.drag = None,
            PanelKind::Table(t) => t.cancel_drag(),
            _ => {}
        }
    }

    pub fn tick(&mut self, doc: &crate::document::Document, now: Instant) -> bool {
        match &mut self.kind {
            PanelKind::Waves(w) => w.tick(doc, now),
            PanelKind::Pipeline(p) => p.tick(now),
            PanelKind::Table(t) => t.tick(now),
            _ => false,
        }
    }

    #[must_use]
    pub fn is_animating(&self) -> bool {
        match &self.kind {
            PanelKind::Waves(w) => w.is_animating(),
            PanelKind::Pipeline(p) => p.is_animating(),
            PanelKind::Table(t) => t.is_animating(),
            _ => false,
        }
    }

    pub fn record_paint(&mut self) {
        match &mut self.kind {
            PanelKind::Waves(w) => w.frames_painted += 1,
            PanelKind::Pipeline(p) => p.frames_painted += 1,
            _ => {}
        }
    }
}

pub struct Panels {
    layout: DockLayout,
    pub(crate) timeline: Vec<TimelineDivider>,
    timeline_revision: Option<u64>,
    panels: BTreeMap<PanelId, Panel>,
    focused: PanelId,
    next_id: u64,
    revision: u64,
}

impl Default for Panels {
    fn default() -> Self {
        Self::new()
    }
}

impl Panels {
    #[must_use]
    pub fn new() -> Self {
        let id = PanelId(1);
        Self {
            layout: DockLayout::single(id),
            timeline: Vec::new(),
            timeline_revision: None,
            panels: BTreeMap::from([(id, Panel::new(id, PanelKind::Start))]),
            focused: id,
            next_id: 2,
            revision: 0,
        }
    }

    pub(crate) fn restore(
        layout: DockLayout,
        panels: Vec<Panel>,
        focused: PanelId,
        previous: &Self,
    ) -> Result<Self> {
        let count = panels.len();
        let panels: BTreeMap<_, _> = panels.into_iter().map(|p| (p.id, p)).collect();
        ensure!(panels.len() == count, "duplicate panel ID");
        let next_id = panels
            .keys()
            .map(|p| p.0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("panel IDs exhausted"))?;
        let mut result = Self {
            layout,
            timeline: Vec::new(),
            timeline_revision: None,
            panels,
            focused,
            next_id: next_id.max(previous.next_id),
            revision: previous
                .revision
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("layout revision exhausted"))?,
        };
        result.validate()?;
        result.layout = result.layout.normalized().expect("validated layout");
        result.validate()?;
        if let Some(id) = previous.settings_id() {
            result.readd_settings(
                id,
                previous.layout.placement(id).unwrap(),
                previous.focused == id,
            )?;
        }
        Ok(result)
    }

    #[must_use]
    pub fn settings_id(&self) -> Option<PanelId> {
        self.iter().find(|p| p.kind.is_settings()).map(|p| p.id)
    }

    #[must_use]
    pub fn can_close(&self, id: PanelId) -> bool {
        self.get(id)
            .is_some_and(|panel| !(panel.kind.is_start() && self.content_count() == 1))
    }

    fn content_count(&self) -> usize {
        self.iter().filter(|p| p.kind.is_content()).count()
    }

    #[must_use]
    pub fn first_waves(&self) -> Option<PanelId> {
        self.first(|kind| kind.waves().is_some())
    }

    #[must_use]
    pub fn first_start(&self) -> Option<PanelId> {
        self.layout
            .center
            .as_ref()?
            .panels()
            .into_iter()
            .find(|id| self.panels[id].kind.is_start())
    }

    fn first(&self, pred: impl Fn(&PanelKind) -> bool) -> Option<PanelId> {
        self.layout
            .panels()
            .into_iter()
            .find(|id| pred(&self.panels[id].kind))
    }

    pub fn open_settings(&mut self) -> Result<bool> {
        if let Some(id) = self.settings_id() {
            return self.focus(id);
        }
        if let Some(beside) = self.center_target(self.focused) {
            self.open(PanelKind::Settings, beside, None)?;
        } else {
            self.open_in(PanelKind::Settings, Placement::Center)?;
        }
        Ok(true)
    }

    fn insert_settings(
        &mut self,
        id: PanelId,
        preferred: PanelId,
        placement: Placement,
        focus: bool,
    ) {
        let beside = self.placement_target(preferred, placement);
        if let Some(Layout::Tabs { tabs, active }) =
            beside.and_then(|beside| self.layout.group_mut(beside))
        {
            tabs.push(id);
            if focus {
                *active = id;
            }
        } else {
            self.layout.insert(id, placement);
        }
        self.panels.insert(id, Panel::new(id, PanelKind::Settings));
        if focus {
            self.focused = id;
            self.layout.activate(id);
        }
    }

    fn readd_settings(&mut self, id: PanelId, placement: Placement, focus: bool) -> Result<()> {
        ensure!(!self.panels.contains_key(&id), "settings id reused");
        ensure!(self.len() < MAX_PANELS, "panel limit reached");
        self.next_id = self.next_id.max(id.0 + 1);
        let beside = self.focused;
        self.insert_settings(id, beside, placement, focus);
        self.validate()
    }

    #[must_use]
    pub fn layout(&self) -> &DockLayout {
        &self.layout
    }
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub fn focused_id(&self) -> PanelId {
        self.focused
    }
    #[must_use]
    pub fn focused(&self) -> &Panel {
        &self.panels[&self.focused]
    }
    pub fn focused_mut(&mut self) -> &mut Panel {
        self.panels.get_mut(&self.focused).unwrap()
    }

    #[must_use]
    pub fn waves(&self, id: PanelId) -> Option<&WaveModel> {
        self.get(id)?.kind.waves()
    }

    #[must_use]
    pub fn focused_waves(&self) -> Option<&WaveModel> {
        self.waves(self.focused)
    }

    pub fn focused_waves_mut(&mut self) -> Option<&mut WaveModel> {
        self.waves_mut(self.focused)
    }

    pub fn waves_mut(&mut self, id: PanelId) -> Option<&mut WaveModel> {
        self.get_mut(id)?.kind.waves_mut()
    }

    #[must_use]
    pub fn pipeline(&self, id: PanelId) -> Option<&PipelineModel> {
        self.get(id)?.kind.pipeline()
    }

    pub fn pipeline_mut(&mut self, id: PanelId) -> Option<&mut PipelineModel> {
        self.get_mut(id)?.kind.pipeline_mut()
    }

    pub fn pipelines_mut(&mut self) -> impl Iterator<Item = &mut PipelineModel> {
        self.panels
            .values_mut()
            .filter_map(|p| p.kind.pipeline_mut())
    }

    #[must_use]
    pub fn transaction(&self, id: PanelId) -> Option<&TransactionModel> {
        self.get(id)?.kind.transaction()
    }

    pub fn transaction_mut(&mut self, id: PanelId) -> Option<&mut TransactionModel> {
        self.get_mut(id)?.kind.transaction_mut()
    }

    pub fn transactions_mut(&mut self) -> impl Iterator<Item = (PanelId, &mut TransactionModel)> {
        self.panels
            .values_mut()
            .filter_map(|p| Some((p.id, p.kind.transaction_mut()?)))
    }

    #[must_use]
    pub fn first_free_transaction(&self) -> Option<PanelId> {
        self.layout.panels().into_iter().find(|id| {
            self.panels[id]
                .kind
                .transaction()
                .is_some_and(|model| !model.pinned())
        })
    }

    #[must_use]
    pub fn pipeline_for_track(
        &self,
        track: crate::trace::Traced<volna_trace::data::transactions::TrackRef>,
    ) -> Option<PanelId> {
        self.layout.panels().into_iter().find(|id| {
            self.panels[id]
                .kind
                .pipeline()
                .is_some_and(|p| p.track.track() == Some(track))
        })
    }
    #[must_use]
    pub fn pipeline_showing(
        &self,
        doc: &Document,
        track: crate::trace::Traced<volna_trace::data::transactions::TrackRef>,
        id: volna_trace::data::transactions::TransactionRef,
    ) -> Option<PanelId> {
        self.layout.panels().into_iter().find(|panel| {
            self.panels[panel].kind.pipeline().is_some_and(|p| {
                p.track.trace() == track.trace
                    && matches!(p.rows(doc), crate::pipeline::Rows::Ready(set)
                        if set.row_of(track.item, id).is_some())
            })
        })
    }

    #[must_use]
    pub fn get(&self, id: PanelId) -> Option<&Panel> {
        self.panels.get(&id)
    }
    pub fn get_mut(&mut self, id: PanelId) -> Option<&mut Panel> {
        self.panels.get_mut(&id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &Panel> {
        self.panels.values()
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Panel> {
        self.panels.values_mut()
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.panels.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.panels.is_empty()
    }

    fn ids(&self) -> BTreeSet<PanelId> {
        self.panels.keys().copied().collect()
    }

    fn unused_id(&self) -> Result<PanelId> {
        self.next_id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("panel IDs exhausted"))?;
        Ok(PanelId(self.next_id))
    }

    pub fn validate(&self) -> Result<()> {
        self.layout.validate(&self.ids())?;
        self.validate_start_placement(&self.layout)?;
        ensure!(
            self.layout.active_for(self.focused) == Some(self.focused),
            "focused panel is not an active tab"
        );
        ensure!(
            self.content_count() > 0,
            "workspace requires a content panel"
        );
        Ok(())
    }

    fn validate_start_placement(&self, layout: &DockLayout) -> Result<()> {
        ensure!(
            self.iter()
                .filter(|p| p.kind.is_start())
                .all(|p| layout.placement(p.id) == Some(Placement::Center)),
            "the start panel belongs in the center"
        );
        Ok(())
    }

    fn advance(&mut self) -> Result<()> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("layout revision exhausted"))?;
        Ok(())
    }

    pub fn set_layout(&mut self, layout: DockLayout, from_revision: u64) -> Result<bool> {
        ensure!(from_revision == self.revision, "stale layout proposal");
        layout.validate(&self.ids())?;
        self.validate_start_placement(&layout)?;
        let layout = layout.normalized().expect("validated nonempty tree");
        layout.validate(&self.ids())?;
        if layout == self.layout {
            return Ok(false);
        }
        self.advance()?;
        let visible = layout.visible();
        if !visible.contains(&self.focused) {
            let active = layout.active_for(self.focused).unwrap();
            self.focused = if visible.contains(&active) {
                active
            } else {
                visible.first().copied().unwrap_or(active)
            };
        }
        self.layout = layout;
        Ok(true)
    }

    pub fn move_panel(&mut self, id: PanelId, placement: Placement) -> Result<bool> {
        ensure!(self.panels.contains_key(&id), "unknown panel");
        if self.layout.placement(id) == Some(placement) {
            return self.focus(id);
        }
        let mut layout = self.layout.clone();
        layout.remove(id);
        layout = layout.normalized().unwrap_or(DockLayout {
            center: None,
            right: None,
        });
        layout.insert(id, placement);
        self.set_layout(layout, self.revision)?;
        self.focus(id)?;
        Ok(true)
    }

    pub fn toggle_right_dock(&mut self) -> Result<bool> {
        let Some(dock) = self.layout.right.as_ref() else {
            return Ok(false);
        };
        let mut layout = self.layout.clone();
        layout.right.as_mut().unwrap().open = !dock.open;
        self.set_layout(layout, self.revision)
    }

    /// Reveal the reusable inspector in its current region, or create one in
    /// the right dock. Pinned inspectors keep their records and are not reused.
    pub(crate) fn ensure_transaction(&mut self, model: TransactionModel) -> Result<PanelId> {
        if let Some(id) = self.first_free_transaction() {
            self.focus(id)?;
            Ok(id)
        } else {
            self.open_in(PanelKind::Transaction(Box::new(model)), Placement::Right)
        }
    }

    pub(crate) fn open_in(&mut self, kind: PanelKind, placement: Placement) -> Result<PanelId> {
        ensure!(self.len() < MAX_PANELS, "panel limit reached");
        let id = self.unused_id()?;
        self.advance()?;
        self.next_id = id.0 + 1;
        self.layout.insert(id, placement);
        self.focused = id;
        self.layout.activate(id);
        self.panels.insert(id, Panel::new(id, kind));
        Ok(id)
    }

    /// Prefer the requested central group, falling back to the first visible
    /// central panel. Sidebar focus must not determine a new viewer's region.
    pub(crate) fn center_target(&self, preferred: PanelId) -> Option<PanelId> {
        self.placement_target(preferred, Placement::Center)
    }

    fn placement_target(&self, preferred: PanelId, placement: Placement) -> Option<PanelId> {
        let tree = match placement {
            Placement::Center => self.layout.center.as_ref(),
            Placement::Right => self.layout.right.as_ref().map(|dock| &dock.layout),
        }?;
        tree.active_for(preferred)
            .or_else(|| tree.visible().first().copied())
    }

    /// Open a viewer centrally, replacing a start placeholder when present.
    /// Explicit tab/split actions use `open` to keep the requested region.
    pub(crate) fn open_center(
        &mut self,
        kind: PanelKind,
        preferred: PanelId,
        split: Option<Axis>,
    ) -> Result<(PanelId, Vec<Panel>)> {
        let id = match self.center_target(preferred) {
            Some(beside) if self.panels[&beside].kind.is_start() => {
                let result = self.replace(beside, kind)?;
                self.focus(result.0)?;
                return Ok(result);
            }
            Some(beside) => self.open(kind, beside, split)?,
            None => self.open_in(kind, Placement::Center)?,
        };
        Ok((id, Vec::new()))
    }

    pub fn focus(&mut self, id: PanelId) -> Result<bool> {
        ensure!(self.panels.contains_key(&id), "unknown panel");
        if self.focused == id && self.layout.visible().contains(&id) {
            return Ok(false);
        }
        self.advance()?;
        self.layout.activate(id);
        self.focused = id;
        Ok(true)
    }

    pub fn focus_next(&mut self, backwards: bool) -> Result<bool> {
        let ids = self.layout.panels();
        let ix = ids.iter().position(|p| *p == self.focused).unwrap();
        let next = if backwards {
            (ix + ids.len() - 1) % ids.len()
        } else {
            (ix + 1) % ids.len()
        };
        self.focus(ids[next])
    }

    pub fn focus_index(&mut self, ix: usize) -> Result<bool> {
        match self.layout.panels().get(ix) {
            Some(&id) => self.focus(id),
            None => Ok(false),
        }
    }

    pub fn rename(&mut self, id: PanelId, title: Option<String>) -> Result<bool> {
        let title = title.filter(|s| !s.trim().is_empty());
        if self.get(id).is_some_and(|panel| *panel.title == title) {
            return Ok(false);
        }
        self.retitle(id, title, true).map(|_| true)
    }

    pub(crate) fn swap_title(
        &mut self,
        id: PanelId,
        title: Option<String>,
    ) -> Result<Option<String>> {
        self.retitle(id, title, false)
    }

    fn retitle(
        &mut self,
        id: PanelId,
        title: Option<String>,
        edit: bool,
    ) -> Result<Option<String>> {
        let panel = self
            .panels
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown panel"))?;
        let opaque = if let PanelKind::Unsupported(raw) = &panel.kind {
            let mut fields: BTreeMap<String, Box<serde_json::value::RawValue>> =
                serde_json::from_str(raw.get())?;
            fields.insert("title".into(), serde_json::value::to_raw_value(&title)?);
            Some(serde_json::value::to_raw_value(&fields)?)
        } else {
            None
        };
        self.advance()?;
        let panel = self.panels.get_mut(&id).unwrap();
        let old = panel.title.get().clone();
        if edit {
            panel.title.set(title);
        } else {
            panel.title.swap(title);
        }
        if let Some(raw) = opaque {
            panel.kind = PanelKind::Unsupported(raw);
        }
        Ok(old)
    }

    pub fn toggle_link(&mut self, id: PanelId, doc: &crate::Document, dim: LinkDim) -> Result<()> {
        let nav = self
            .get_mut(id)
            .and_then(|p| p.kind.nav_mut())
            .ok_or_else(|| anyhow::anyhow!("not a timed panel"))?;
        nav.toggle_link(doc, dim);
        self.advance()
    }

    pub fn create(&mut self, source: PanelId, split: Option<Axis>) -> Result<PanelId> {
        let kind = self
            .panels
            .get(&source)
            .ok_or_else(|| anyhow::anyhow!("unknown panel"))?
            .kind
            .clone_view(split.is_some())?;
        self.place(kind, source, split)
    }

    pub fn open(
        &mut self,
        kind: PanelKind,
        beside: PanelId,
        split: Option<Axis>,
    ) -> Result<PanelId> {
        ensure!(self.get(beside).is_some(), "unknown panel");
        self.place(kind, beside, split)
    }

    pub fn replace(&mut self, id: PanelId, kind: PanelKind) -> Result<(PanelId, Vec<Panel>)> {
        ensure!(self.panels.contains_key(&id), "unknown panel");
        let new = self.unused_id()?;
        let mut layout = self.layout.clone();
        if kind.is_start() && layout.placement(id) == Some(Placement::Right) {
            layout.remove(id);
            layout = layout.normalized().unwrap_or(DockLayout {
                center: None,
                right: None,
            });
            layout.insert(new, Placement::Center);
            if self.focused != id {
                layout.activate(self.focused);
            }
        } else {
            layout.replace(id, new);
        }
        let mut ids = self.ids();
        ids.remove(&id);
        ids.insert(new);
        layout.validate(&ids)?;
        self.advance()?;
        self.next_id = new.0 + 1;
        self.layout = layout;
        if self.focused == id {
            self.focused = new;
        }
        let removed = self.panels.remove(&id).expect("checked above");
        self.panels.insert(new, Panel::new(new, kind));
        Ok((new, vec![removed]))
    }

    fn place(&mut self, kind: PanelKind, beside: PanelId, split: Option<Axis>) -> Result<PanelId> {
        ensure!(self.len() < MAX_PANELS, "panel limit reached");
        let id = self.unused_id()?;
        let mut layout = self.layout.clone();
        if let Some(axis) = split {
            layout.split(beside, id, axis);
        } else if let Some(Layout::Tabs { tabs, active }) = layout.group_mut(beside) {
            tabs.push(id);
            *active = id;
        }
        let layout = layout.normalized().unwrap();
        let mut ids = self.ids();
        ids.insert(id);
        layout.validate(&ids)?;
        self.advance()?;
        self.next_id = id.0 + 1;
        self.layout = layout;
        self.focused = id;
        self.layout.activate(id);
        self.panels.insert(id, Panel::new(id, kind));
        Ok(id)
    }

    pub fn close(&mut self, id: PanelId) -> Result<Vec<Panel>> {
        let panel = self
            .panels
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown panel"))?;
        if panel.kind.is_content() && self.content_count() == 1 {
            if panel.kind.is_start() {
                return Ok(Vec::new());
            }
            return self
                .replace(id, PanelKind::Start)
                .map(|(_, removed)| removed);
        }
        let ids = self.layout.panels();
        let ix = ids.iter().position(|p| *p == id).unwrap();
        let mut layout = self.layout.clone();
        let neighbor = layout.remove(id);
        let layout = layout.normalized().unwrap();
        let mut survivors = self.ids();
        survivors.remove(&id);
        layout.validate(&survivors)?;
        self.advance()?;
        self.layout = layout;
        let removed = self.panels.remove(&id).expect("checked above");
        if self.focused == id {
            self.focused = neighbor.unwrap_or_else(|| ids[(ix + 1) % ids.len()]);
            self.layout.activate(self.focused);
        }
        Ok(vec![removed])
    }

    pub fn close_others(&mut self, id: PanelId) -> Result<Vec<Panel>> {
        let panel = self
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("unknown panel"))?;
        if self.len() == 1 {
            return Ok(Vec::new());
        }
        let start = (!panel.kind.is_content())
            .then(|| self.unused_id())
            .transpose()?;
        self.advance()?;
        let (kept, removed): (Vec<_>, Vec<_>) = std::mem::take(&mut self.panels)
            .into_iter()
            .partition(|(other, _)| *other == id);
        self.panels = kept.into_iter().collect();
        let mut tabs = vec![id];
        let in_right = self.layout.placement(id) == Some(Placement::Right);
        if let Some(start) = start {
            self.next_id = start.0 + 1;
            self.panels
                .insert(start, Panel::new(start, PanelKind::Start));
            if !in_right {
                tabs.push(start);
            }
        }
        let kept = Layout::Tabs { tabs, active: id };
        self.layout = if in_right {
            DockLayout {
                center: start.map(Layout::single),
                right: Some(SideDock {
                    layout: kept,
                    width: self.layout.right.as_ref().unwrap().width,
                    open: true,
                }),
            }
        } else {
            kept.into()
        };
        self.focused = id;
        Ok(removed.into_iter().map(|(_, panel)| panel).collect())
    }

    #[must_use]
    pub fn saved_view(&self) -> (DockLayout, PanelId, Vec<&Panel>) {
        let mut layout = self.layout.clone();
        for id in self.iter().filter(|p| p.kind.is_settings()).map(|p| p.id) {
            layout.remove(id);
        }
        let layout = layout.normalized().expect("a content panel remains");
        let focused = if self.panels[&self.focused].kind.is_settings() {
            layout
                .active_for(self.focused)
                .unwrap_or_else(|| layout.active_for(layout.panels()[0]).unwrap())
        } else {
            self.focused
        };
        let panels = self.iter().filter(|p| !p.kind.is_settings()).collect();
        (layout, focused, panels)
    }

    pub(crate) fn structure(&self) -> (DockLayout, PanelId, BTreeSet<PanelId>) {
        let (layout, focused, panels) = self.saved_view();
        let ids = panels.iter().map(|p| p.id).collect();
        (layout, focused, ids)
    }

    pub(crate) fn install(&mut self, structure: Structure) -> Result<Structure> {
        let (layout, focused, mut ids) = self.structure();
        for id in &structure.close {
            ensure!(ids.remove(id), "no panel {} to close", id.0);
        }
        for panel in &structure.reopen {
            ensure!(ids.insert(panel.id), "panel {} is open", panel.id.0);
        }
        structure.layout.validate(&ids)?;
        ensure!(
            structure.layout.active_for(structure.focused) == Some(structure.focused),
            "focused panel is not an active tab"
        );
        let settings = self
            .settings_id()
            .map(|id| (id, self.layout.placement(id).unwrap()));
        self.advance()?;
        let closed = structure
            .close
            .iter()
            .map(|id| self.panels.remove(id).expect("checked above"))
            .collect();
        let reopened = structure.reopen.iter().map(|p| p.id).collect();
        for panel in structure.reopen {
            self.next_id = self.next_id.max(panel.id.0 + 1);
            self.panels.insert(panel.id, panel);
        }
        self.layout = structure.layout;
        self.focused = structure.focused;
        if let Some((id, placement)) = settings {
            self.insert_settings(id, structure.focused, placement, false);
        }
        self.validate()?;
        Ok(Structure {
            layout,
            focused,
            close: reopened,
            reopen: closed,
        })
    }

    pub fn reset(&mut self) -> Result<()> {
        let id = self.unused_id()?;
        self.advance()?;
        let settings = self
            .settings_id()
            .map(|id| (id, self.layout.placement(id).unwrap(), self.focused == id));
        self.next_id = id.0 + 1;
        self.panels = BTreeMap::from([(id, Panel::new(id, PanelKind::Start))]);
        self.layout = DockLayout::single(id);
        self.focused = id;
        if let Some((settings, placement, focus)) = settings {
            self.readd_settings(settings, placement, focus)?;
        }
        Ok(())
    }
}
