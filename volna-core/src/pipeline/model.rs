//! `PipelineModel`: the state of a pipeline panel (the track it shows, its navigation, the
//! row axis, the label column, hover and drags) and every input rule that mutates it.

use std::fmt::Write as _;
use std::sync::Arc;
use web_time::Instant;

use super::layout::{LABEL_W_DEFAULT, LABEL_W_MAX, LABEL_W_MIN, LayoutInput, PipelineLayout};
use super::order::{
    RowOrder, RowOrderCommand, RowOrderMode, StageOrderOption, detect_batch_suggestion,
};
use super::palette::StagePalette;
use super::row_set::{MergedRows, RowStream};
use super::rows::{ROW_PX_DEFAULT, ROW_PX_MAX, ROW_PX_MIN, RowView};
use super::selection::{ZoomPreview, ZoomSelection, ZoomTarget};
use super::sources::{
    FocusStyle, PipelineSource, PipelineSourceCommand, SourceBadges, SourceLoadStatus, clean_alias,
    compute_source_badges, disjoint_stage_notice,
};
use super::stages::{StageAnchorFilter, StageVisibility, StageVisibilityCommand, row_anchor_time};
use super::zoom::{self, CYCLE_PX_MAX, ROW_PX_CAP, ZoomBox, ZoomPoint};
use crate::clock::ClockKey;
use crate::data::loaded_tracks::LoadedGenerator;
use crate::document::{Document, TrackLoadState, TxSelection};
use crate::geometry::{Modifiers, MouseButton, Point, Rect};
use crate::marker::LaneHit;
use crate::nav::{Link, NavState, Tween};
use crate::panels::PanelId;
use crate::theme::Theme;
use crate::trace::{TraceId, Traced};
use crate::wave::model::PointerEvent;
use crate::wave::overlay::SpanClocks;
use crate::wave::viewport::{FIT_MARGIN_PX, Viewport};
use volna_trace::data::transactions::{
    AttributeValue, Status, TrackRef, Transaction, TransactionRef, TransactionStage,
};
use volna_trace::remote::memory::MemoryBudget;

pub const LABEL_ATTRIBUTE: &str = "vtr.label";
const DRAG_THRESHOLD_PX: f32 = 3.0;
pub(super) const ROW_JUMP_MARGIN_PX: f64 = 32.0;
pub const SCROLL_ROWS: f64 = 3.0;
const ROW_HEIGHT_STEP: f64 = std::f64::consts::SQRT_2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowBoundary {
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackSource {
    Resolved {
        track: Traced<TrackRef>,
        path: Vec<String>,
    },
    Unresolved {
        trace: TraceId,
        path: Vec<String>,
    },
}

impl TrackSource {
    #[must_use]
    pub fn path(&self) -> &[String] {
        match self {
            Self::Resolved { path, .. } | Self::Unresolved { path, .. } => path,
        }
    }

    #[must_use]
    pub fn track(&self) -> Option<Traced<TrackRef>> {
        match self {
            Self::Resolved { track, .. } => Some(*track),
            Self::Unresolved { .. } => None,
        }
    }

    #[must_use]
    pub fn trace(&self) -> TraceId {
        match self {
            Self::Resolved { track, .. } => track.trace,
            Self::Unresolved { trace, .. } => *trace,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Occupancy,
    OccupancySplit,
    Overview,
    Navigator,
    NavigatorSplit,
    Activity(super::ActivityCommand),
    SourceBadge { row: usize, source: usize },
    Row(usize),
    Cell { row: usize, stage: usize },
    LabelSplit,
    Header,
    Marker(usize),
    Span(usize),
    Live,
    ReferenceTag,
    ClearReference,
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    Occupancy,
    OccupancySplit {
        start: f32,
        height: f32,
        moved: bool,
    },
    Pan {
        button: MouseButton,
        start: Point,
        last: Point,
        moved: bool,
    },
    BoxZoom(ZoomSelection),
    Cursor,
    Navigator,
    NavigatorSplit {
        start: f32,
        width: f32,
        saved_width: f32,
        moved: bool,
    },
    LabelSplit {
        grab: f32,
    },
    Marker(crate::marker::MarkerDrag),
}

pub enum Rows<'a> {
    Ready(RowSet<'a>),
    Loading,
    Failed(&'a str),
    Unresolved,
    Unavailable,
}

pub use super::row_set::RowSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineCellContext {
    pub row: usize,
    pub source_index: usize,
    pub stage_name: Option<String>,
    pub is_merged: bool,
    pub source_label: Option<super::SourceLabel>,
    pub is_focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourcesSnapshot {
    pub(crate) sources: Vec<PipelineSource>,
    pub(crate) focused_source: Option<usize>,
    pub(crate) focus_style: FocusStyle,
    pub(crate) next_color_slot: usize,
}

pub(crate) enum Prop {
    Sources(SourcesSnapshot),
    Order(RowOrder),
    StageVisibility(StageVisibility),
}

impl Prop {
    pub(crate) fn kind(&self) -> u8 {
        match self {
            Self::Sources(_) => 0,
            Self::Order(_) => 1,
            Self::StageVisibility(_) => 2,
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        match self {
            Self::Sources(snap) => snap
                .sources
                .iter()
                .map(|s| {
                    s.track.path().iter().map(|p| p.len() + 24).sum::<usize>()
                        + s.alias.as_ref().map_or(0, String::len)
                        + 64
                })
                .sum(),
            Self::Order(order) => match &order.mode {
                RowOrderMode::Start | RowOrderMode::End => 32,
                RowOrderMode::Stages { stages } => {
                    32 + stages.iter().map(|s| s.len() + 24).sum::<usize>()
                }
                RowOrderMode::FromStage { stage } => 32 + stage.len() + 24,
            },
            Self::StageVisibility(vis) => {
                32 + vis.hidden().iter().map(|s| s.len() + 24).sum::<usize>()
            }
        }
    }
}

enum VisibleStatus<'a> {
    ReadyMatched,
    ReadyNeedsRebuild,
    Loading,
    Failed(&'a str),
    Unresolved,
    AllHidden,
    Unavailable,
}

pub struct PipelineModel {
    pub follow: super::FollowActivity,
    pub order: RowOrder,
    pub stage_visibility: StageVisibility,
    pub sources: Vec<PipelineSource>,
    pub focused_source: Option<usize>,
    pub focus_style: FocusStyle,
    pub hovered_source: Option<usize>,
    pub nav: NavState,
    pub rows: Tween<RowView>,
    pub row_cap: f32,
    pub label_width: f32,
    pub navigator: super::NavigatorSettings,
    pub occupancy: super::OccupancySettings,
    pub hover: Option<Hit>,
    pub drag: Option<Drag>,
    pub pointer: Option<Point>,
    pub frames_painted: u64,
    attached: bool,
    next_color_slot: usize,
    sources_before: crate::history::Before<SourcesSnapshot>,
    sources_edit_label: Option<String>,
    order_before: crate::history::Before<RowOrder>,
    order_edit_label: Option<String>,
    stage_visibility_before: crate::history::Before<StageVisibility>,
    stage_visibility_edit_label: Option<String>,
    suggested_order_stage: Option<String>,
    suggestion_cache_key: Option<(u64, u64, u64)>,
    opening: Option<super::opening::OpeningView>,
    palette: StagePalette,
    badges: SourceBadges,
    merged: MergedRows,
    disjoint_notice: Option<String>,
    budget: Option<MemoryBudget>,
    layout: PipelineLayout,
}

impl PipelineModel {
    #[must_use]
    pub fn new(track: TrackSource, link: Link) -> Self {
        Self::from_sources(vec![PipelineSource::new(track, 0)], link)
    }

    #[must_use]
    pub fn new_merged(tracks: Vec<TrackSource>, link: Link) -> Self {
        let sources = tracks
            .into_iter()
            .enumerate()
            .map(|(slot, track)| PipelineSource::new(track, slot))
            .collect();
        Self::from_sources(sources, link)
    }

    #[must_use]
    pub fn from_sources(sources: Vec<PipelineSource>, link: Link) -> Self {
        let mut nav = NavState::new();
        nav.link = link;
        let next_color_slot = sources
            .iter()
            .map(|s| s.color_slot.saturating_add(1))
            .max()
            .unwrap_or(0);
        let badges = compute_source_badges(&sources);
        Self {
            sources,
            focused_source: None,
            focus_style: FocusStyle::default(),
            hovered_source: None,
            follow: super::FollowActivity::default(),
            order: RowOrder::default(),
            stage_visibility: StageVisibility::default(),
            nav,
            rows: Tween::new(RowView::default()),
            row_cap: ROW_PX_CAP,
            label_width: LABEL_W_DEFAULT,
            navigator: super::NavigatorSettings::default(),
            occupancy: super::OccupancySettings::default(),
            hover: None,
            drag: None,
            pointer: None,
            frames_painted: 0,
            attached: false,
            next_color_slot,
            sources_before: crate::history::Before::default(),
            sources_edit_label: None,
            order_before: crate::history::Before::default(),
            order_edit_label: None,
            stage_visibility_before: crate::history::Before::default(),
            stage_visibility_edit_label: None,
            suggested_order_stage: None,
            suggestion_cache_key: None,
            opening: None,
            palette: StagePalette::default(),
            badges,
            merged: MergedRows::default(),
            disjoint_notice: None,
            budget: None,
            layout: PipelineLayout::default(),
        }
    }

    pub(crate) fn open_merged(
        tracks: Vec<TrackSource>,
        link: Link,
        doc: &Document,
        budget: Option<MemoryBudget>,
    ) -> Self {
        let mut model = Self::new_merged(tracks, link);
        model.budget = budget;
        model.nav.reset(Some(doc.limits()));
        model.opening = Some(super::opening::OpeningView::default());
        model
    }

    pub(crate) fn set_memory_budget(&mut self, budget: Option<MemoryBudget>) {
        self.budget = budget;
    }

    #[must_use]
    pub fn is_merged(&self) -> bool {
        self.sources.len() > 1
    }

    #[must_use]
    pub fn primary_trace(&self) -> Option<TraceId> {
        self.sources.first().map(|s| s.track.trace())
    }

    #[must_use]
    pub fn single_trace(&self) -> Option<TraceId> {
        let first = self.primary_trace()?;
        self.sources
            .iter()
            .all(|s| s.track.trace() == first)
            .then_some(first)
    }

    #[must_use]
    pub fn uses_trace(&self, trace: TraceId) -> bool {
        self.sources.iter().any(|s| s.track.trace() == trace)
    }

    #[must_use]
    pub fn has_only_trace(&self, trace: TraceId) -> bool {
        !self.sources.is_empty() && self.sources.iter().all(|s| s.track.trace() == trace)
    }

    #[must_use]
    pub fn badges(&self) -> &SourceBadges {
        &self.badges
    }

    #[must_use]
    pub fn disjoint_notice(&self) -> Option<&str> {
        self.disjoint_notice.as_deref()
    }

    /// The source index currently emphasized (hovered chip or soloed source in Dim mode).
    #[must_use]
    pub fn highlight_source(&self) -> Option<usize> {
        if !self.is_merged() {
            return None;
        }
        self.hovered_source
            .filter(|&i| i < self.sources.len())
            .or_else(|| {
                (self.focus_style == FocusStyle::Dim)
                    .then_some(self.focused_source)
                    .flatten()
                    .filter(|&i| i < self.sources.len())
            })
    }

    #[must_use]
    pub fn is_source_dimmed(&self, source_index: usize) -> bool {
        self.highlight_source()
            .is_some_and(|focused| focused != source_index)
    }

    #[must_use]
    pub fn is_source_effectively_visible(&self, source_index: usize) -> bool {
        let Some(source) = self.sources.get(source_index) else {
            return false;
        };
        if let Some(focused) = self.focused_source
            && self.focus_style == FocusStyle::Isolate
            && focused < self.sources.len()
        {
            return source_index == focused;
        }
        source.visible
    }

    #[must_use]
    pub fn source_status(&self, doc: &Document, index: usize) -> SourceLoadStatus {
        let Some(source) = self.sources.get(index) else {
            return SourceLoadStatus::Unavailable;
        };
        let Some(track) = source.track.track() else {
            return SourceLoadStatus::Unresolved;
        };
        if !self.attached {
            return SourceLoadStatus::Unavailable;
        }
        match doc.track(track) {
            Some(TrackLoadState::Ready(loaded)) => SourceLoadStatus::Ready {
                count: loaded
                    .generators
                    .iter()
                    .map(|g| g.transactions().len())
                    .sum(),
            },
            Some(TrackLoadState::Loading) => SourceLoadStatus::Loading,
            Some(TrackLoadState::Failed(err)) => SourceLoadStatus::Failed(err.clone()),
            None => SourceLoadStatus::Unavailable,
        }
    }

    pub(crate) fn source_generators(
        &self,
        doc: &Document,
        source_index: usize,
    ) -> Option<(TraceId, Vec<Arc<LoadedGenerator>>)> {
        let (trace, gens) = self.source_generators_ref(doc, source_index)?;
        Some((trace, gens.to_vec()))
    }

    pub(crate) fn source_generators_ref<'a>(
        &self,
        doc: &'a Document,
        source_index: usize,
    ) -> Option<(TraceId, &'a [Arc<LoadedGenerator>])> {
        let source = self.sources.get(source_index)?;
        let track = source.track.track()?;
        match doc.track(track) {
            Some(TrackLoadState::Ready(loaded)) => Some((track.trace, &loaded.generators)),
            _ => None,
        }
    }

    pub(crate) fn primary_table_track(&self, doc: &Document) -> Option<Traced<TrackRef>> {
        if let Some(row) = self.selected_row(doc)
            && let Rows::Ready(set) = self.rows(doc)
            && let Some((src_idx, _, _)) = set.get_with_trace(row)
            && let Some(track) = self.sources.get(src_idx).and_then(|s| s.track.track())
        {
            return Some(track);
        }
        if let Some(focused) = self.focused_source
            && let Some(track) = self.sources.get(focused).and_then(|s| s.track.track())
        {
            return Some(track);
        }
        self.sources.iter().find_map(|s| s.track.track())
    }

    pub(crate) fn opening_measurement(&self) -> Option<super::ViewportSize> {
        let opening = self.opening.as_ref()?;
        let size = super::ViewportSize {
            width: f64::from(self.layout.cells.width()),
            zoom: self.layout.zoom,
        };
        (size.is_valid() && opening.size != Some(size)).then_some(size)
    }

    pub(crate) fn measure_viewport(&mut self, size: super::ViewportSize) {
        if size.is_valid()
            && let Some(opening) = &mut self.opening
        {
            opening.size = Some(size);
        }
    }

    pub(crate) fn cancel_opening(&mut self) {
        self.opening = None;
    }

    pub(crate) fn initialize_view(&mut self, doc: &mut Document) -> bool {
        let Some(size) = self.opening.as_ref().and_then(|opening| opening.size) else {
            return false;
        };
        if self
            .clock(doc)
            .is_some_and(|clock| matches!(clock.state, crate::clock::ClockState::Loading))
        {
            return false;
        }
        // Wait for all resolved visible sources to finish loading before framing the first record
        // so opening a merged panel frames the true earliest merged transaction.
        if self.sources.iter().enumerate().any(|(i, s)| {
            self.is_source_effectively_visible(i)
                && s.track
                    .track()
                    .is_some_and(|t| matches!(doc.track(t), Some(TrackLoadState::Loading)))
        }) {
            return false;
        }
        self.sync_merged_cache(doc);
        let first_info = {
            let Rows::Ready(set) = self.rows(doc) else {
                return false;
            };
            set.get(0)
                .map(|(_, first)| (first.begin, super::opening::fallback_time_unit(first)))
        };
        self.opening = None;
        let Some((first_begin, fallback_unit)) = first_info else {
            return false;
        };
        let unit = self
            .clock(doc)
            .and_then(|clock| clock.timeline())
            .map_or(fallback_unit, |_| self.cycle_units(doc, first_begin as f64));
        let viewport = super::opening::viewport(first_begin, unit, size, doc.limits());
        self.nav.viewport_state_mut(doc).set(viewport);
        self.rows.set(RowView::default());
        // Following the viewport center would immediately scroll past the first record.
        self.suspend_follow();
        true
    }

    #[must_use]
    pub fn clone_view(&self) -> Self {
        let mut cloned = Self::from_sources(self.sources.clone(), self.nav.link);
        cloned.focused_source = self.focused_source;
        cloned.focus_style = self.focus_style;
        cloned.follow = self.follow;
        cloned.order = self.order.clone();
        cloned.stage_visibility = self.stage_visibility.clone();
        cloned.nav = self.nav.clone_view();
        cloned.rows = Tween::new(self.rows.target());
        cloned.row_cap = self.row_cap;
        cloned.label_width = self.label_width;
        cloned.navigator = self.navigator;
        cloned.occupancy = self.occupancy;
        cloned.palette = self.palette.clone();
        cloned.disjoint_notice.clone_from(&self.disjoint_notice);
        cloned.budget.clone_from(&self.budget);
        cloned.next_color_slot = self.next_color_slot;
        cloned
    }

    pub fn attach(&mut self, doc: &mut Document) -> anyhow::Result<()> {
        if self.attached {
            return Ok(());
        }
        let mut retained = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            if let Some(track) = source.track.track() {
                if let Err(err) = doc.retain_track(track) {
                    for prev in retained {
                        doc.release_track(prev);
                    }
                    return Err(err);
                }
                retained.push(track);
            }
        }
        self.attached = true;
        self.refresh(doc);
        Ok(())
    }

    pub fn detach(&mut self, doc: &mut Document) {
        if !self.attached {
            return;
        }
        for source in &self.sources {
            if let Some(track) = source.track.track() {
                doc.release_track(track);
            }
        }
        self.attached = false;
        self.merged.clear();
    }

    #[must_use]
    pub fn is_attached(&self) -> bool {
        self.attached
    }

    fn snapshot_sources(&self) -> SourcesSnapshot {
        SourcesSnapshot {
            sources: self.sources.clone(),
            focused_source: self.focused_source,
            focus_style: self.focus_style,
            next_color_slot: self.next_color_slot,
        }
    }

    fn note_sources_before(&mut self, label: impl Into<String>) {
        let snap = self.snapshot_sources();
        self.sources_before.note(|| snap);
        if self.sources_edit_label.is_none() {
            self.sources_edit_label = Some(label.into());
        }
    }

    fn note_order_before(&mut self, label: impl Into<String>) {
        let before = self.order.clone();
        self.order_before.note(|| before);
        if self.order_edit_label.is_none() {
            self.order_edit_label = Some(label.into());
        }
    }

    fn note_stage_visibility_before(&mut self, label: impl Into<String>) {
        let before = self.stage_visibility.clone();
        self.stage_visibility_before.note(|| before);
        if self.stage_visibility_edit_label.is_none() {
            self.stage_visibility_edit_label = Some(label.into());
        }
    }

    pub(crate) fn take_edits(&mut self, panel: PanelId, history: &mut crate::history::History) {
        let label = self.sources_edit_label.take();
        if let Some(before) = self.sources_before.take() {
            history.record(
                crate::history::Edit::Prop {
                    panel,
                    prop: crate::panels::content::Prop::Pipeline(Prop::Sources(before)),
                },
                label.or_else(|| Some("Edit pipeline sources".into())),
            );
        }
        let order_label = self.order_edit_label.take();
        if let Some(before) = self.order_before.take() {
            history.record(
                crate::history::Edit::Prop {
                    panel,
                    prop: crate::panels::content::Prop::Pipeline(Prop::Order(before)),
                },
                order_label.or_else(|| Some("Change pipeline row order".into())),
            );
        }
        let vis_label = self.stage_visibility_edit_label.take();
        if let Some(before) = self.stage_visibility_before.take() {
            history.record(
                crate::history::Edit::Prop {
                    panel,
                    prop: crate::panels::content::Prop::Pipeline(Prop::StageVisibility(before)),
                },
                vis_label.or_else(|| Some("Toggle pipeline stage visibility".into())),
            );
        }
    }

    pub(crate) fn matches_prop(&self, prop: &Prop) -> bool {
        match prop {
            Prop::Sources(snap) => {
                self.sources == snap.sources
                    && self.focused_source == snap.focused_source
                    && self.focus_style == snap.focus_style
            }
            Prop::Order(order) => self.order == *order,
            Prop::StageVisibility(vis) => self.stage_visibility == *vis,
        }
    }

    pub(crate) fn swap_prop(&mut self, doc: &mut Document, prop: Prop) -> anyhow::Result<Prop> {
        match prop {
            Prop::Sources(next) => {
                let old = self.snapshot_sources();
                if self.attached {
                    let old_tracks: Vec<Traced<TrackRef>> =
                        old.sources.iter().filter_map(|s| s.track.track()).collect();
                    let new_tracks: Vec<Traced<TrackRef>> = next
                        .sources
                        .iter()
                        .filter_map(|s| s.track.track())
                        .collect();
                    let mut newly_retained = Vec::new();
                    for &track in &new_tracks {
                        if !old_tracks.contains(&track) {
                            if let Err(err) = doc.retain_track(track) {
                                for prev in newly_retained {
                                    doc.release_track(prev);
                                }
                                return Err(err);
                            }
                            newly_retained.push(track);
                        }
                    }
                    for &track in &old_tracks {
                        if !new_tracks.contains(&track) {
                            doc.release_track(track);
                        }
                    }
                }
                self.sources = next.sources;
                self.focused_source = next.focused_source;
                self.focus_style = next.focus_style;
                self.next_color_slot = next.next_color_slot;
                self.hovered_source = None;
                self.hover = None;
                self.suggestion_cache_key = None;
                self.refresh(doc);
                self.clamp_and_follow_after_visibility_change(doc);
                Ok(Prop::Sources(old))
            }
            Prop::Order(mut next) => {
                next.normalize();
                let old = std::mem::replace(&mut self.order, next);
                self.hover = None;
                self.suggestion_cache_key = None;
                if !self.order.is_start() {
                    self.suggested_order_stage = None;
                }
                self.sync_merged_cache(doc);
                self.clamp_and_follow_after_visibility_change(doc);
                Ok(Prop::Order(old))
            }
            Prop::StageVisibility(mut next) => {
                next.normalize();
                let old = std::mem::replace(&mut self.stage_visibility, next);
                self.merged.clear_visible_index();
                self.hover = None;
                self.follow_after_stage_visibility_change(doc);
                Ok(Prop::StageVisibility(old))
            }
        }
    }

    pub fn add_source(&mut self, doc: &mut Document, track: TrackSource) -> anyhow::Result<bool> {
        let duplicate = self
            .sources
            .iter()
            .any(|existing| match (&existing.track, &track) {
                (
                    TrackSource::Resolved { track: a, .. },
                    TrackSource::Resolved { track: b, .. },
                ) => a == b,
                _ => {
                    existing.track.trace() == track.trace() && existing.track.path() == track.path()
                }
            });
        if duplicate {
            return Ok(false);
        }
        if self.attached
            && let Some(resolved) = track.track()
        {
            doc.retain_track(resolved)?;
        }
        self.note_sources_before("Add pipeline source");
        let slot = self.next_color_slot;
        self.next_color_slot += 1;
        self.sources.push(PipelineSource::new(track, slot));
        self.hover = None;
        self.badges = compute_source_badges(&self.sources);
        self.refresh(doc);
        self.clamp_and_follow_after_visibility_change(doc);
        Ok(true)
    }

    pub fn remove_source(&mut self, doc: &mut Document, index: usize) -> bool {
        if self.sources.len() <= 1 || index >= self.sources.len() {
            return false;
        }
        self.note_sources_before("Remove pipeline source");
        self.remove_source_inner(doc, index);
        self.badges = compute_source_badges(&self.sources);
        self.refresh(doc);
        self.clamp_and_follow_after_visibility_change(doc);
        true
    }

    fn remove_source_inner(&mut self, doc: &mut Document, index: usize) {
        let removed = self.sources.remove(index);
        if self.attached
            && let Some(track) = removed.track.track()
        {
            doc.release_track(track);
        }
        self.focused_source = match self.focused_source {
            Some(f) if f == index => {
                self.focus_style = FocusStyle::Dim;
                None
            }
            Some(f) if f > index => Some(f - 1),
            other => other,
        };
        self.hovered_source = match self.hovered_source {
            Some(h) if h == index => None,
            Some(h) if h > index => Some(h - 1),
            other => other,
        };
        // A panel reduced back to one source hides the chip strip, so ensure the sole source
        // cannot be stranded in hidden or isolated focus state.
        if self.sources.len() == 1 {
            self.focused_source = None;
            self.focus_style = FocusStyle::Dim;
            self.hovered_source = None;
            self.sources[0].visible = true;
        }
        self.hover = None;
    }

    pub fn reorder_source(&mut self, doc: &Document, from: usize, to: usize) -> bool {
        if from == to || from >= self.sources.len() || to >= self.sources.len() {
            return false;
        }
        self.note_sources_before("Reorder pipeline sources");
        let item = self.sources.remove(from);
        self.sources.insert(to, item);
        let remap = |idx: usize| {
            if idx == from {
                to
            } else if from < to && idx > from && idx <= to {
                idx - 1
            } else if to < from && idx >= to && idx < from {
                idx + 1
            } else {
                idx
            }
        };
        self.focused_source = self.focused_source.map(remap);
        self.hovered_source = self.hovered_source.map(remap);
        self.hover = None;
        self.badges = compute_source_badges(&self.sources);
        self.refresh(doc);
        true
    }

    pub fn forget_trace(&mut self, doc: &mut Document, trace: TraceId, record_undo: bool) {
        if self.has_only_trace(trace) {
            if !record_undo {
                self.detach(doc);
            }
            return;
        }
        if !self.uses_trace(trace) {
            return;
        }
        if record_undo {
            self.note_sources_before(format!("Close trace {trace}"));
        }
        let mut removed_any = false;
        let mut idx = 0;
        while idx < self.sources.len() {
            if self.sources[idx].track.trace() == trace {
                if self.sources.len() <= 1 {
                    break;
                }
                self.remove_source_inner(doc, idx);
                removed_any = true;
            } else {
                idx += 1;
            }
        }
        if removed_any {
            self.badges = compute_source_badges(&self.sources);
            self.refresh(doc);
            self.clamp_and_follow_after_visibility_change(doc);
        }
    }

    pub fn source_command(
        &mut self,
        doc: &mut Document,
        command: PipelineSourceCommand,
    ) -> anyhow::Result<bool> {
        match command {
            PipelineSourceCommand::Add(track) => {
                let Some(session) = doc.session(track.trace) else {
                    return Ok(false);
                };
                let Some(declaration) = session.tracks().iter().find(|t| t.id == track.item) else {
                    return Ok(false);
                };
                let path = declaration.path.clone();
                self.add_source(doc, TrackSource::Resolved { track, path })
            }
            PipelineSourceCommand::Remove(index) => Ok(self.remove_source(doc, index)),
            PipelineSourceCommand::Reorder { from, to } => Ok(self.reorder_source(doc, from, to)),
            PipelineSourceCommand::ToggleVisible(index) => {
                if index >= self.sources.len() {
                    return Ok(false);
                }
                if let Some(focused) = self.focused_source
                    && self.focus_style == FocusStyle::Isolate
                {
                    // Exit isolate into explicit multi-source visibility toggling.
                    self.focused_source = None;
                    self.focus_style = FocusStyle::Dim;
                    for (i, s) in self.sources.iter_mut().enumerate() {
                        s.visible = i == focused || i == index;
                    }
                    if focused == index {
                        self.sources[index].visible = false;
                    }
                } else {
                    self.sources[index].visible = !self.sources[index].visible;
                    if !self.sources[index].visible && self.focused_source == Some(index) {
                        self.focused_source = None;
                        self.focus_style = FocusStyle::Dim;
                    }
                }
                self.hover = None;
                self.sync_merged_cache(doc);
                self.clamp_and_follow_after_visibility_change(doc);
                Ok(true)
            }
            PipelineSourceCommand::SetVisible(index, visible) => {
                if index >= self.sources.len() {
                    return Ok(false);
                }
                if let Some(focused) = self.focused_source
                    && self.focus_style == FocusStyle::Isolate
                {
                    if visible == (index == focused) {
                        return Ok(false);
                    }
                    self.focused_source = None;
                    self.focus_style = FocusStyle::Dim;
                    for (i, s) in self.sources.iter_mut().enumerate() {
                        s.visible = i == focused;
                    }
                    self.sources[index].visible = visible;
                } else {
                    let source = &mut self.sources[index];
                    if source.visible == visible {
                        return Ok(false);
                    }
                    source.visible = visible;
                    if !visible && self.focused_source == Some(index) {
                        self.focused_source = None;
                        self.focus_style = FocusStyle::Dim;
                    }
                }
                self.hover = None;
                self.sync_merged_cache(doc);
                self.clamp_and_follow_after_visibility_change(doc);
                Ok(true)
            }
            PipelineSourceCommand::SetAlias(index, alias) => {
                let Some(source) = self.sources.get(index) else {
                    return Ok(false);
                };
                let cleaned = clean_alias(alias.as_deref());
                if source.alias == cleaned {
                    return Ok(false);
                }
                self.note_sources_before("Rename pipeline source badge");
                self.sources[index].alias = cleaned;
                self.badges = compute_source_badges(&self.sources);
                Ok(true)
            }
            PipelineSourceCommand::ToggleFocus(index) => {
                if index >= self.sources.len() {
                    return Ok(false);
                }
                let was_isolate =
                    self.focus_style == FocusStyle::Isolate && self.focused_source.is_some();
                let mut visibility_changed = false;
                if !self.sources[index].visible {
                    self.sources[index].visible = true;
                    self.focused_source = Some(index);
                    visibility_changed = true;
                } else if self.focused_source == Some(index) {
                    self.focused_source = None;
                    self.focus_style = FocusStyle::Dim;
                } else {
                    self.focused_source = Some(index);
                }
                let is_isolate =
                    self.focus_style == FocusStyle::Isolate && self.focused_source.is_some();
                if was_isolate || is_isolate || visibility_changed {
                    self.hover = None;
                    self.sync_merged_cache(doc);
                }
                self.clamp_and_follow_after_visibility_change(doc);
                Ok(true)
            }
            PipelineSourceCommand::Isolate(index) => {
                if index >= self.sources.len() {
                    return Ok(false);
                }
                self.sources[index].visible = true;
                self.focused_source = Some(index);
                self.focus_style = FocusStyle::Isolate;
                self.hover = None;
                self.sync_merged_cache(doc);
                self.clamp_and_follow_after_visibility_change(doc);
                Ok(true)
            }
            PipelineSourceCommand::SetFocusStyle(style) => {
                if self.focus_style == style
                    || (self.focused_source.is_none() && style != FocusStyle::Dim)
                {
                    return Ok(false);
                }
                self.focus_style = style;
                self.hover = None;
                self.sync_merged_cache(doc);
                self.clamp_and_follow_after_visibility_change(doc);
                Ok(true)
            }
            PipelineSourceCommand::ClearFocusAndShowAll => {
                let changed = self.focused_source.is_some()
                    || self.focus_style != FocusStyle::Dim
                    || self.sources.iter().any(|s| !s.visible);
                if !changed {
                    return Ok(false);
                }
                self.focused_source = None;
                self.focus_style = FocusStyle::Dim;
                for s in &mut self.sources {
                    s.visible = true;
                }
                self.hover = None;
                self.sync_merged_cache(doc);
                self.clamp_and_follow_after_visibility_change(doc);
                Ok(true)
            }
            PipelineSourceCommand::Hover(index) => {
                let next = index.filter(|&i| i < self.sources.len());
                if self.hovered_source == next {
                    return Ok(false);
                }
                self.hovered_source = next;
                Ok(true)
            }
            PipelineSourceCommand::RetryFailed => Ok(self.retry(doc)),
        }
    }

    pub fn order_command(&mut self, doc: &Document, command: RowOrderCommand) -> bool {
        let mut next = self.order.clone();
        let label = match command {
            RowOrderCommand::Set(order) => {
                next = order;
                "Change pipeline row order"
            }
            RowOrderCommand::SetMode(mode) => {
                next.mode = mode;
                "Change pipeline row order"
            }
            RowOrderCommand::ToggleStage(stage) => {
                let mut stages: Vec<String> = match &next.mode {
                    RowOrderMode::Start | RowOrderMode::End => Vec::new(),
                    RowOrderMode::Stages { stages } => stages.clone(),
                    RowOrderMode::FromStage { .. } => next.resolved_stages(&self.palette).to_vec(),
                };
                if let Some(pos) = stages.iter().position(|s| s == &stage) {
                    stages.remove(pos);
                } else {
                    stages.push(stage);
                }
                next.mode = if stages.is_empty() {
                    RowOrderMode::Start
                } else {
                    RowOrderMode::Stages { stages }
                };
                "Toggle stage in pipeline row order"
            }
            RowOrderCommand::SetUnmatched(policy) => {
                next.unmatched = policy;
                "Change unmatched row policy"
            }
            RowOrderCommand::SetFollowSortKey(follow) => {
                next.follow_sort_key = follow;
                "Toggle follow sort-stage anchor"
            }
            RowOrderCommand::Reset => {
                next.mode = RowOrderMode::Start;
                "Reset pipeline row order"
            }
        };
        next.normalize();
        if next == self.order {
            return false;
        }
        let rows_changed = next.mode != self.order.mode || next.unmatched != self.order.unmatched;
        self.note_order_before(label);
        self.order = next;
        self.hover = None;
        self.suggestion_cache_key = None;
        if !self.order.is_start() {
            self.suggested_order_stage = None;
        }
        if rows_changed {
            self.sync_merged_cache(doc);
        }
        self.clamp_and_follow_after_visibility_change(doc);
        true
    }

    pub fn stage_visibility_command(
        &mut self,
        doc: &Document,
        command: StageVisibilityCommand,
    ) -> bool {
        let mut next = self.stage_visibility.clone();
        let (changed, label) = match command {
            StageVisibilityCommand::Toggle(stage) => {
                let changed = next.toggle(stage);
                (changed, "Toggle pipeline stage visibility")
            }
            StageVisibilityCommand::SetHidden { stage, hidden } => {
                let changed = next.set_hidden(stage, hidden);
                let label = if hidden {
                    "Hide pipeline stage"
                } else {
                    "Show pipeline stage"
                };
                (changed, label)
            }
            StageVisibilityCommand::ShowAll => {
                let changed = next.show_all();
                (changed, "Show all pipeline stages")
            }
        };
        next.normalize();
        if !changed || next == self.stage_visibility {
            return false;
        }
        self.note_stage_visibility_before(label);
        self.stage_visibility = next;
        self.merged.clear_visible_index();
        self.hover = None;
        self.follow_after_stage_visibility_change(doc);
        true
    }

    #[must_use]
    pub(crate) fn stage_anchor_filter(&self) -> Option<StageAnchorFilter<'_>> {
        (!self.stage_visibility.is_empty()).then(|| StageAnchorFilter {
            visibility: &self.stage_visibility,
            primary_lane: self.palette.primary_lane(),
        })
    }

    fn row_anchor_time(&self, set: &RowSet<'_>, row: usize) -> Option<u64> {
        let (_, tx) = set.get(row)?;
        let filter = self.stage_anchor_filter();
        let use_sort_meta =
            !self.order.is_start() && (filter.is_none() || self.order.follow_sort_key);
        let meta = use_sort_meta.then(|| set.sort_meta(row)).flatten();
        Some(row_anchor_time(tx, meta, filter))
    }

    #[must_use]
    pub fn stages_order_status_label(&self, doc: &Document) -> Option<String> {
        let order_part = self.order.status_label(self.unmatched_row_count(doc));
        let hidden = self.stage_visibility.hidden_count();
        match (order_part, hidden) {
            (None, 0) => None,
            (Some(order), 0) => Some(order),
            (None, n) => Some(format!(
                "Stages: {n} {} hidden",
                if n == 1 { "stage" } else { "stages" }
            )),
            (Some(order), n) => Some(format!("{order} · {n} hidden")),
        }
    }

    #[must_use]
    pub fn stage_order_options(&self, doc: &Document) -> Vec<StageOrderOption> {
        let names = self.palette.names();
        if names.is_empty() {
            return Vec::new();
        }
        let primary = self.palette.primary_lane();
        let mut counts = vec![0u64; names.len()];
        let mut total_tx = 0u64;
        for only_visible in [true, false] {
            for (i, source) in self.sources.iter().enumerate() {
                if only_visible && !self.is_source_effectively_visible(i) {
                    continue;
                }
                let Some(track) = source.track.track() else {
                    continue;
                };
                let Some(TrackLoadState::Ready(loaded)) = doc.track(track) else {
                    continue;
                };
                for g in &loaded.generators {
                    total_tx += g.transactions().len() as u64;
                    if let Some(lane) = g.stage_census().lanes.iter().find(|l| l.lane == primary) {
                        for entry in &lane.names {
                            if let Some(rank) = self.palette.rank(&entry.name) {
                                counts[rank] += entry.count;
                            }
                        }
                    }
                }
            }
            if total_tx > 0 {
                break;
            }
        }
        names
            .iter()
            .enumerate()
            .map(|(rank, name)| {
                let count = counts[rank];
                let coverage = if total_tx > 0 {
                    (count as f64 / total_tx as f64).min(1.0) as f32
                } else {
                    0.0
                };
                StageOrderOption {
                    name: name.clone(),
                    count,
                    total_transactions: total_tx,
                    coverage,
                }
            })
            .collect()
    }

    #[must_use]
    pub fn unmatched_row_count(&self, doc: &Document) -> usize {
        match self.rows(doc) {
            Rows::Ready(set) => set.unmatched_count(),
            _ => 0,
        }
    }

    #[must_use]
    pub fn suggested_order_stage(&self) -> Option<&str> {
        if self.order.is_start() {
            self.suggested_order_stage.as_deref()
        } else {
            None
        }
    }

    #[must_use]
    pub fn cell_context_at(&self, doc: &Document, p: Point) -> Option<PipelineCellContext> {
        if !self.layout.cells.contains(p) || self.layout.is_overview() {
            return None;
        }
        let (row, stage) = match self.hit_at(p, doc)? {
            Hit::Cell { row, stage } => (row, Some(stage)),
            Hit::Row(row) => (row, None),
            _ => return None,
        };
        let Rows::Ready(set) = self.rows(doc) else {
            return None;
        };
        let (source_index, tx) = set.get(row)?;
        let stage_name = stage
            .and_then(|idx| tx.stages.get(idx))
            .filter(|s| s.lane == self.palette.primary_lane())
            .map(|s| s.name.clone());
        let is_merged = self.is_merged();
        let source_label = is_merged
            .then(|| self.badges.labels.get(source_index).cloned())
            .flatten();
        Some(PipelineCellContext {
            row,
            source_index,
            stage_name,
            is_merged,
            source_label,
            is_focused: self.focused_source == Some(source_index),
        })
    }

    #[must_use]
    pub(crate) fn pending_cell_context_click(&self) -> Option<Point> {
        match self.drag {
            Some(Drag::Pan {
                button: MouseButton::Right,
                start,
                moved: false,
                ..
            }) => Some(start),
            _ => None,
        }
    }

    fn follow_after_stage_visibility_change(&mut self, doc: &Document) {
        if self.laid_out() && self.follow == super::FollowActivity::Following {
            let _ = self.follow_activity(doc);
        }
    }

    fn clamp_and_follow_after_visibility_change(&mut self, doc: &Document) {
        if !self.laid_out() {
            return;
        }
        let zoom = self.layout.zoom;
        let mut rows = self.rows.target().zoomed(zoom);
        rows.clamp(self.cells_h(), self.row_count(doc), zoom);
        self.rows.set(rows.unzoomed(zoom));
        if self.follow == super::FollowActivity::Following {
            let _ = self.follow_activity(doc);
        } else if let Some(target) = self.activity(doc).target {
            let rows = self.rows.target().zoomed(zoom);
            let visible = f64::from(self.cells_h() / rows.row_px);
            let pos = target as f64 + 0.5;
            if pos < rows.top || pos > rows.top + visible {
                self.center_row(doc, target);
            }
        }
    }

    fn merged_cache_status<'a>(&self, doc: &'a Document) -> VisibleStatus<'a> {
        let mut visible_sources = 0usize;
        let mut ready_gens = 0usize;
        let mut total_rows = 0usize;
        let mut any_loading = false;
        let mut first_failed: Option<&'a str> = None;
        let mut any_unresolved = false;
        let mut any_unavailable = false;

        let fp = self.merged.fingerprint();
        let mut fp_idx = 0usize;
        let mut fp_ok = true;
        for (i, source) in self.sources.iter().enumerate() {
            if !self.is_source_effectively_visible(i) {
                continue;
            }
            visible_sources += 1;
            let Some(track) = source.track.track() else {
                any_unresolved = true;
                continue;
            };
            match doc.track(track) {
                Some(TrackLoadState::Ready(loaded)) => {
                    ready_gens += loaded.generators.len();
                    for g in &loaded.generators {
                        total_rows += g.transactions().len();
                        let entry = (i, track.trace, Arc::as_ptr(g) as usize);
                        if fp.get(fp_idx) != Some(&entry) {
                            fp_ok = false;
                        }
                        fp_idx += 1;
                    }
                }
                Some(TrackLoadState::Loading) => any_loading = true,
                Some(TrackLoadState::Failed(err)) => {
                    first_failed.get_or_insert(err.as_str());
                }
                None => any_unavailable = true,
            }
        }
        let resolved = self.order.resolved_stages(&self.palette);
        let primary_lane = self.palette.primary_lane();
        let matched = fp_ok
            && fp_idx == fp.len()
            && self
                .merged
                .order_matches(&self.order, resolved, primary_lane);

        if ready_gens > 0 && total_rows > 0 {
            return if matched {
                VisibleStatus::ReadyMatched
            } else {
                VisibleStatus::ReadyNeedsRebuild
            };
        }
        if any_loading {
            return VisibleStatus::Loading;
        }
        if let Some(error) = first_failed {
            return VisibleStatus::Failed(error);
        }
        if ready_gens > 0 {
            return if matched {
                VisibleStatus::ReadyMatched
            } else {
                VisibleStatus::ReadyNeedsRebuild
            };
        }
        if any_unresolved {
            return VisibleStatus::Unresolved;
        }
        if visible_sources == 0 {
            return VisibleStatus::AllHidden;
        }
        if any_unavailable {
            return VisibleStatus::Unavailable;
        }
        VisibleStatus::Unavailable
    }

    fn collect_visible_streams(&self, doc: &Document) -> Vec<RowStream> {
        let mut streams = Vec::new();
        for (i, source) in self.sources.iter().enumerate() {
            if !self.is_source_effectively_visible(i) {
                continue;
            }
            let Some(track) = source.track.track() else {
                continue;
            };
            if let Some(TrackLoadState::Ready(loaded)) = doc.track(track) {
                for g in &loaded.generators {
                    streams.push(RowStream {
                        source_index: i,
                        trace: track.trace,
                        generator: Arc::clone(g),
                    });
                }
            }
        }
        streams
    }

    pub(crate) fn sync_merged_cache(&mut self, doc: &Document) {
        if !self.attached {
            self.merged.clear();
            return;
        }
        let resolved = self.order.resolved_stages(&self.palette);
        let primary_lane = self.palette.primary_lane();
        match self.merged_cache_status(doc) {
            VisibleStatus::ReadyMatched => {}
            VisibleStatus::AllHidden
                if self.merged.is_empty()
                    && self
                        .merged
                        .matches(&[], &self.order, resolved, primary_lane) => {}
            _ => {
                let streams = self.collect_visible_streams(doc);
                let resolved = self.order.resolved_stages(&self.palette);
                let primary_lane = self.palette.primary_lane();
                self.merged.rebuild(
                    streams,
                    &self.order,
                    resolved,
                    primary_lane,
                    self.budget.as_ref(),
                );
            }
        }
    }

    pub fn refresh(&mut self, doc: &Document) {
        self.badges = compute_source_badges(&self.sources);
        if !self.attached {
            self.merged.clear();
            return;
        }
        let mut all_gens = Vec::new();
        let mut by_source = Vec::new();
        for (i, source) in self.sources.iter().enumerate() {
            if let Some(track) = source.track.track()
                && let Some(TrackLoadState::Ready(loaded)) = doc.track(track)
            {
                all_gens.extend(loaded.generators.iter().cloned());
                by_source.push((i, loaded.generators.clone()));
            }
        }
        if !all_gens.is_empty() {
            self.palette = StagePalette::build(&all_gens);
        }
        self.disjoint_notice = disjoint_stage_notice(&self.sources, &by_source);
        self.suggestion_cache_key = None;
        self.sync_merged_cache(doc);
    }

    pub fn retry(&mut self, doc: &mut Document) -> bool {
        let mut retried = false;
        for source in &self.sources {
            if let Some(track) = source.track.track() {
                retried |= doc.retry_track(track);
            }
        }
        retried
    }

    #[must_use]
    pub fn palette(&self) -> &StagePalette {
        &self.palette
    }

    #[must_use]
    pub fn rows<'a>(&'a self, doc: &'a Document) -> Rows<'a> {
        if self.sources.is_empty() || !self.attached {
            return Rows::Unavailable;
        }
        let resolved = self.order.resolved_stages(&self.palette);
        let primary_lane = self.palette.primary_lane();
        match self.merged_cache_status(doc) {
            VisibleStatus::ReadyMatched => Rows::Ready(RowSet::borrowed(&self.merged)),
            VisibleStatus::ReadyNeedsRebuild => {
                let streams = self.collect_visible_streams(doc);
                Rows::Ready(RowSet::owned(MergedRows::build(
                    streams,
                    &self.order,
                    resolved,
                    primary_lane,
                    self.budget.as_ref(),
                )))
            }
            VisibleStatus::Loading => Rows::Loading,
            VisibleStatus::Failed(error) => Rows::Failed(error),
            VisibleStatus::Unresolved => Rows::Unresolved,
            VisibleStatus::AllHidden => {
                if self
                    .merged
                    .matches(&[], &self.order, resolved, primary_lane)
                {
                    Rows::Ready(RowSet::borrowed(&self.merged))
                } else {
                    Rows::Ready(RowSet::owned(MergedRows::default()))
                }
            }
            VisibleStatus::Unavailable => Rows::Unavailable,
        }
    }

    #[must_use]
    pub fn row_count(&self, doc: &Document) -> usize {
        match self.rows(doc) {
            Rows::Ready(set) => set.len(),
            _ => 0,
        }
    }

    #[must_use]
    pub fn label(tx: &Transaction) -> &str {
        tx.attributes
            .iter()
            .find_map(|attribute| match (&*attribute.key, &attribute.value) {
                (LABEL_ATTRIBUTE, AttributeValue::Text(text)) => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub fn stage_end(tx: &Transaction, stage: &TransactionStage) -> u64 {
        stage.end.unwrap_or(tx.end).max(stage.begin)
    }

    #[must_use]
    pub fn last_layout(&self) -> &PipelineLayout {
        &self.layout
    }

    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.nav.is_animating() || self.rows.is_animating()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let nav = self.nav.tick(now);
        let rows = self.rows.tick(now);
        nav || rows
    }

    #[must_use]
    pub fn selected_row(&self, doc: &Document) -> Option<usize> {
        let selection = self.nav.selection(doc)?;
        let Rows::Ready(set) = self.rows(doc) else {
            return None;
        };
        set.row_of(selection.track.trace, selection.track.item, selection.id)
    }

    #[must_use]
    pub fn label_row_at(&self, doc: &Document, p: Point) -> Option<usize> {
        if !self.layout.labels.contains(p) || self.layout.is_overview() {
            return None;
        }
        match self.hit_at(p, doc) {
            Some(Hit::Row(row) | Hit::SourceBadge { row, .. }) if row < self.row_count(doc) => {
                Some(row)
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn label_badge_at(&self, doc: &Document, p: Point) -> Option<usize> {
        if !self.is_merged() || self.layout.is_overview() || !self.layout.badge_column_contains(p) {
            return None;
        }
        match self.hit_at(p, doc) {
            Some(Hit::SourceBadge { source, .. }) if source < self.sources.len() => Some(source),
            _ => None,
        }
    }

    pub fn row_transaction(
        &self,
        doc: &Document,
        row: usize,
    ) -> Option<(
        Traced<TrackRef>,
        volna_trace::data::transactions::TransactionRef,
    )> {
        let Rows::Ready(set) = self.rows(doc) else {
            return None;
        };
        let (_, trace, tx) = set.get_with_trace(row)?;
        Some((Traced::new(trace, tx.generator), tx.id))
    }

    pub fn select_row(&mut self, doc: &mut Document, panel: PanelId, row: usize) -> bool {
        let Some((track, id)) = self.row_transaction(doc, row) else {
            return false;
        };
        self.nav.clear_ruler_selection();
        self.nav.select(
            doc,
            Some(TxSelection {
                track,
                id,
                origin: panel,
            }),
        )
    }

    pub fn move_selection(
        &mut self,
        doc: &mut Document,
        panel: PanelId,
        delta: isize,
        now: Instant,
    ) {
        let Some(row) = self.selected_row(doc) else {
            self.scroll_rows(doc, delta as f64 * SCROLL_ROWS, now);
            return;
        };
        let count = self.row_count(doc);
        let next = row
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1));
        if !self.select_row(doc, panel, next) {
            return;
        }
        self.reveal_row(doc, next);
    }

    pub(crate) fn reveal_selection(&mut self, doc: &mut Document, now: Instant) -> bool {
        let Some(row) = self.selected_row(doc) else {
            return false;
        };
        if self.layout.cells.height() <= 0.0 {
            return false;
        }
        let lifetime = match self.rows(doc) {
            Rows::Ready(set) => set.get(row).map(|(_, tx)| (tx.begin, tx.end)),
            _ => None,
        };
        let Some((begin, end)) = lifetime else {
            return false;
        };
        self.suspend_follow();
        self.reveal_row(doc, row);
        self.nav.reveal_record(doc, begin, end, now);
        true
    }

    pub fn reveal_record(
        &mut self,
        doc: &mut Document,
        track: Traced<TrackRef>,
        id: TransactionRef,
        now: Instant,
    ) -> bool {
        self.sync_merged_cache(doc);
        let Some((row, begin, end)) = ({
            let Rows::Ready(set) = self.rows(doc) else {
                return false;
            };
            set.row_of(track.trace, track.item, id).and_then(|row| {
                let (_, tx) = set.get(row)?;
                Some((row, tx.begin, tx.end))
            })
        }) else {
            return false;
        };
        self.suspend_follow();
        self.center_row(doc, row);
        let pad = ((end.saturating_sub(begin)) as f64 * 0.05).max(1.0);
        self.nav.animate_to(
            doc,
            crate::wave::viewport::Viewport {
                start: begin as f64 - pad,
                end: end as f64 + pad,
            },
            now,
        );
        true
    }

    pub(crate) fn pan_to_row_boundary(
        &mut self,
        doc: &mut Document,
        row: usize,
        boundary: RowBoundary,
        now: Instant,
    ) -> bool {
        let filter = self.stage_anchor_filter();
        let tick = match self.rows(doc) {
            Rows::Ready(set) => set.get(row).map(|(_, tx)| match boundary {
                RowBoundary::Start => filter
                    .and_then(|f| f.first_visible_stage_begin(tx))
                    .unwrap_or(tx.begin),
                RowBoundary::End => filter
                    .and_then(|f| f.last_visible_stage_end(tx))
                    .unwrap_or(tx.end),
            }),
            _ => None,
        };
        let Some(tick) = tick else {
            return false;
        };
        let width_px = self.cells_w();
        let margin_px = (ROW_JUMP_MARGIN_PX * f64::from(self.layout.zoom)).min(width_px * 0.1);
        let mut target = self.nav.viewport_state(doc).target();
        let width = target.width();
        let margin = width * margin_px / width_px;
        target.start = match boundary {
            RowBoundary::Start => tick as f64 - margin,
            RowBoundary::End => tick as f64 + margin - width,
        };
        target.end = target.start + width;
        target.clamp_preserving_width(doc.limits());
        self.suspend_follow();
        let mode = doc.navigation.animation;
        self.nav
            .viewport_state_mut(doc)
            .animate_to(target, now, mode);
        true
    }

    fn center_row(&mut self, doc: &Document, row: usize) {
        let zoom = self.layout.zoom.max(f32::EPSILON);
        let height = self.layout.cells.height();
        if height <= 0.0 {
            return;
        }
        let mut rows = self.rows.target().zoomed(zoom);
        rows.top = row as f64 + 0.5 - f64::from(height / rows.row_px) * 0.5;
        rows.clamp(height, self.row_count(doc), zoom);
        self.rows.set(rows.unzoomed(zoom));
    }

    fn reveal_row(&mut self, doc: &Document, row: usize) {
        let zoom = self.layout.zoom.max(f32::EPSILON);
        let height = self.layout.cells.height();
        if height <= 0.0 {
            return;
        }
        let mut rows = self.rows.target().zoomed(zoom);
        let visible = f64::from(height / rows.row_px);
        let position = row as f64;
        if position < rows.top {
            rows.top = position;
        } else if position + 1.0 > rows.top + visible {
            rows.top = position + 1.0 - visible;
        } else {
            return;
        }
        rows.clamp(height, self.row_count(doc), zoom);
        self.rows.set(rows.unzoomed(zoom));
    }

    #[must_use]
    pub fn hover_text(&self, doc: &Document) -> Option<String> {
        match self.hover {
            Some(Hit::Occupancy) => return self.occupancy_hover(doc),
            Some(Hit::OccupancySplit) => {
                return Some("Drag to resize occupancy; click to show or hide".into());
            }
            _ => {}
        }
        if self.hover == Some(Hit::Overview) {
            return Some("Pipeline presence · zoom in to inspect records".into());
        }
        if let Some(Hit::SourceBadge { source, .. }) = self.hover
            && let Some(lbl) = self.badges.labels.get(source)
        {
            return Some(format!(
                "{} [{}] · Click to Solo, double-click to Isolate",
                lbl.distinguishing, lbl.badge
            ));
        }
        let (row, stage) = match self.hover? {
            Hit::Row(row) => (row, None),
            Hit::Cell { row, stage } => (row, Some(stage)),
            _ => return None,
        };
        let Rows::Ready(set) = self.rows(doc) else {
            return None;
        };
        let (source_index, tx) = set.get(row)?;
        let mut text = if self.is_merged()
            && let Some(lbl) = self.badges.labels.get(source_index)
        {
            format!("[{}] #{row}", lbl.badge)
        } else {
            format!("#{row}")
        };
        match tx.status {
            Status::Aborted => text.push_str(" flushed"),
            Status::Open => text.push_str(" open"),
            Status::Error => text.push_str(" error"),
            _ => {}
        }
        match stage.and_then(|s| tx.stages.get(s)) {
            Some(s) => {
                write!(
                    text,
                    " · {} {}",
                    s.name,
                    self.span_text(doc, s.begin, Self::stage_end(tx, s))
                )
                .unwrap();
                if s.lane != self.palette.primary_lane() {
                    write!(text, " lane {}", s.lane).unwrap();
                } else if self.stage_visibility.is_hidden(&s.name) {
                    text.push_str(" (hidden)");
                }
            }
            None => write!(text, " · {}", self.span_text(doc, tx.begin, tx.end)).unwrap(),
        }
        let label = Self::label(tx);
        if !label.is_empty() {
            text.push_str(" · ");
            text.push_str(label);
        }
        if self.order.is_stage_based()
            && let Some(meta) = set.sort_meta(row)
        {
            if !meta.matched {
                text.push_str(" · fallback");
            } else if stage.is_some() && stage == meta.sort_stage_index() {
                text.push_str(" · sort key");
            }
        }
        Some(text)
    }

    pub fn layout(
        &mut self,
        bounds: crate::geometry::Rect,
        doc: &Document,
        theme: &Theme,
        margins: crate::panels::TimelineMargins,
        device_scale: f64,
    ) -> &PipelineLayout {
        self.sync_merged_cache(doc);
        let (row_count, failed) = match self.rows(doc) {
            Rows::Ready(set) => (set.len(), false),
            Rows::Failed(_) => (0, true),
            _ => (0, false),
        };
        let rulers = self.nav.clocks().rulers(&doc.clocks);
        let input = LayoutInput {
            bounds,
            margins,
            header_h: theme.timeline_height,
            ruler_h: rulers.len() as f32 * crate::wave::overlay::RULER_H * theme.zoom,
            zoom: theme.zoom,
            label_width: self.label_width,
            navigator_width: self.navigator.width(),
            occupancy: self.occupancy,
            rows: self.rows.value,
            row_count,
            markers: doc.markers(),
            spans: SpanClocks::of(&rulers, self.nav.clocks(), &doc.clocks, doc.time_base()),
            measuring: doc
                .reference_time()
                .map(|reference| crate::wave::overlay::Measuring {
                    reference,
                    cursor: self.nav.cursor(doc),
                }),
            viewport: self.nav.viewport(doc),
            failed,
            device_scale,
            is_merged: self.is_merged(),
            max_badge_chars: self.badges.max_badge_chars,
            mono_size: theme.mono_size,
        };
        self.layout = PipelineLayout::compute(input);
        if let Some(Drag::BoxZoom(selection)) = self.drag
            && !selection.matches_layout(&self.layout)
        {
            self.drag = None;
        }
        let followed = if self.opening.is_some() || matches!(self.drag, Some(Drag::BoxZoom(_))) {
            None
        } else {
            self.follow_activity(doc)
        };
        let rows_before = self.rows.target();
        if self.rows.value != input.rows {
            self.layout = PipelineLayout::compute(LayoutInput {
                rows: self.rows.value,
                ..input
            });
        }
        if !self.rows.is_animating() {
            let unzoomed = self.layout.rows.unzoomed(self.layout.zoom);
            self.rows.set(unzoomed);
        }
        if self.stage_visibility.is_empty() {
            self.merged.clear_visible_index();
        } else if self.layout.is_overview() {
            self.merged.sync_visible_index(
                &self.stage_visibility,
                self.palette.primary_lane(),
                self.budget.as_ref(),
            );
        }
        let activity = match followed {
            Some(activity) if self.rows.target() == rows_before => activity,
            _ => self.activity(doc),
        };
        self.layout.activity = activity;
        let area = self.layout.cells;
        let z = theme.zoom;
        let width = (170.0 * z).min(area.width());
        let x = area.left() + (area.width() - width) * 0.5;
        if area.height() >= 80.0 * z {
            for (command, count, y, direction) in [
                (
                    super::ActivityCommand::RevealAbove,
                    activity.above,
                    area.top() + 6.0 * z,
                    "↑",
                ),
                (
                    super::ActivityCommand::RevealBelow,
                    activity.below,
                    area.bottom() - 28.0 * z,
                    "↓",
                ),
            ] {
                if count > 0 {
                    self.layout.activity_controls.push((
                        command,
                        crate::geometry::Rect::from_xywh(x, y, width, 22.0 * z),
                        format!("{direction} {count} relevant rows"),
                    ));
                }
            }
        }
        self.hover = self.pointer.and_then(|p| self.hit_at(p, doc));
        if self.order.is_start() {
            let vp = self.nav.viewport(doc);
            let key = (vp.start.to_bits(), vp.end.to_bits(), self.merged.version());
            if self.suggestion_cache_key != Some(key) {
                self.suggestion_cache_key = Some(key);
                let options = self.stage_order_options(doc);
                self.suggested_order_stage = match self.rows(doc) {
                    Rows::Ready(set) => {
                        let samples = set.sample_overlapping(vp, 256);
                        detect_batch_suggestion(&samples, &self.palette, &options)
                    }
                    _ => None,
                };
            }
        } else {
            self.suggested_order_stage = None;
            self.suggestion_cache_key = None;
        }
        &self.layout
    }

    fn hit_at(&self, p: Point, doc: &Document) -> Option<Hit> {
        let layout = &self.layout;
        if !layout.bounds.contains(p) {
            return None;
        }
        if layout.occupancy_split.contains(p) {
            return Some(Hit::OccupancySplit);
        }
        if layout.occupancy.contains(p) {
            return Some(Hit::Occupancy);
        }
        if layout.navigator_split.contains(p) || layout.navigator_grip_target.contains(p) {
            return Some(Hit::NavigatorSplit);
        }
        if let Some((command, _, _)) = layout
            .activity_controls
            .iter()
            .find(|(_, rect, _)| rect.contains(p))
        {
            return Some(Hit::Activity(*command));
        }
        if layout.navigator.rect.contains(p) {
            return Some(Hit::Navigator);
        }
        if layout.gutter.contains(p) {
            return None;
        }
        if layout.label_split.contains(p) {
            return Some(Hit::LabelSplit);
        }
        if layout.retry.is_some_and(|r| r.contains(p)) {
            return Some(Hit::Retry);
        }
        match layout.marker_lane.hit(p) {
            Some(LaneHit::Live) => return Some(Hit::Live),
            Some(LaneHit::Tag) => return Some(Hit::ReferenceTag),
            Some(LaneHit::ClearReference) => return Some(Hit::ClearReference),
            _ => {}
        }
        if let Some(chip) = layout.marker_lane.chip_at(p) {
            return Some(Hit::Marker(chip));
        }
        if let Some(span) = layout.marker_lane.span_at(p) {
            return Some(Hit::Span(span));
        }
        if layout.header.contains(p) || layout.marker_lane.strip().contains(p) {
            return Some(Hit::Header);
        }
        let row = layout.row_at(p.y)?;
        if layout.is_overview() {
            return Some(Hit::Overview);
        }
        if layout.labels.contains(p) {
            if self.is_merged()
                && layout.badge_column_contains(p)
                && let Rows::Ready(set) = self.rows(doc)
                && let Some((source, _)) = set.get(row)
            {
                return Some(Hit::SourceBadge { row, source });
            }
            return Some(Hit::Row(row));
        }
        let Rows::Ready(set) = self.rows(doc) else {
            return Some(Hit::Row(row));
        };
        let (_, tx) = set.get(row)?;
        let t = layout.time_at(&self.nav.viewport(doc), p.x);
        let primary = self.palette.primary_lane();
        let bands = super::stage_layout::stage_bands(tx, primary);
        let row_fraction = (p.y - layout.row_y(row)) / layout.rows.row_px;
        let overlay_band =
            super::stage_layout::overlay_band(layout.row_y(row), layout.rows.row_px, layout.zoom);
        let mut best: Option<(usize, bool)> = None;
        for (ix, stage) in tx.stages.iter().enumerate() {
            let end = Self::stage_end(tx, stage);
            let inside = t >= stage.begin as f64 && (t < end as f64 || stage.begin == end);
            if !inside {
                continue;
            }
            let overlay = stage.lane != primary;
            let band = bands[ix];
            if (overlay && !overlay_band.contains(&p.y))
                || (!overlay && (row_fraction * band.count as f32).floor() as usize != band.index)
            {
                continue;
            }
            if best.is_none_or(|(_, was_overlay)| overlay || !was_overlay) {
                best = Some((ix, overlay));
            }
        }
        Some(match best {
            Some((stage, _)) => Hit::Cell { row, stage },
            None => Hit::Row(row),
        })
    }

    fn cells_w(&self) -> f64 {
        self.layout.cells_width_f64()
    }

    fn cells_h(&self) -> f32 {
        self.layout.cells.height().max(1.0)
    }

    fn rows_target(&self) -> RowView {
        self.rows.target().zoomed(self.layout.zoom)
    }

    fn set_rows(&mut self, doc: &Document, mut rows: RowView, animate: Option<Instant>) {
        rows.clamp(self.cells_h(), self.row_count(doc), self.layout.zoom);
        let rows = rows.unzoomed(self.layout.zoom);
        match animate {
            Some(now) => self.rows.animate_to(rows, now, doc.navigation.animation),
            None => self.rows.set(rows),
        }
    }

    fn rows_base(&self, animated: bool) -> RowView {
        if animated {
            self.rows_target()
        } else {
            self.rows.value.zoomed(self.layout.zoom)
        }
    }

    fn laid_out(&self) -> bool {
        self.layout.zoom > 0.0
    }

    fn fit_margin_px(&self) -> f64 {
        FIT_MARGIN_PX * f64::from(self.layout.zoom)
    }

    fn fit_viewport(&self, doc: &Document) -> Viewport {
        Viewport::fit_px(doc.limits(), self.cells_w(), self.fit_margin_px())
    }

    fn fit_row_px(&self, doc: &Document) -> f64 {
        let n = self.row_count(doc);
        if n == 0 {
            return f64::INFINITY;
        }
        let h = f64::from(self.cells_h());
        let inner = h - 2.0 * self.fit_margin_px();
        let inner = if inner >= h * 0.5 { inner } else { h };
        (inner / n as f64).max(f64::from(ROW_PX_MIN * self.layout.zoom))
    }

    fn cycle_units(&self, doc: &Document, t: f64) -> f64 {
        let (a, b) = doc.limits();
        let t = t.clamp(a as f64, b as f64) as u64;
        self.clock(doc)
            .and_then(|c| c.timeline())
            .and_then(|tl| {
                let at = tl.cycle_at(t)?;
                let period = match at.next_edge {
                    Some(next) => next - at.edge,
                    None => at.edge - tl.prev_edge(at.edge)?,
                };
                (period > 0).then_some(period as f64)
            })
            .unwrap_or(1.0)
    }

    #[must_use]
    pub fn zoom_box(&self, doc: &Document, viewport: &Viewport) -> ZoomBox {
        let z = f64::from(self.layout.zoom);
        let w = self.cells_w();
        let t_lo = (w / self.fit_viewport(doc).width()).ln();
        let center = viewport.start.midpoint(viewport.end);
        let t_hi = (CYCLE_PX_MAX * z / self.cycle_units(doc, center))
            .ln()
            .max(t_lo);
        let cap = f64::from(self.row_cap) * z;
        let r_lo = self
            .fit_row_px(doc)
            .min(f64::from(ROW_PX_DEFAULT) * z)
            .min(cap);
        ZoomBox {
            time: (t_lo, t_hi),
            rows: (r_lo.ln(), cap.ln()),
        }
    }

    fn zoom_point(&self, viewport: &Viewport, rows: RowView) -> ZoomPoint {
        ZoomPoint {
            time: (self.cells_w() / viewport.width()).ln(),
            rows: f64::from(rows.row_px).ln(),
        }
    }

    fn square_size_limits(&self, doc: &Document, cycle_units: f64) -> (f64, f64) {
        let z = f64::from(self.layout.zoom);
        let cap = f64::from(self.row_cap) * z;
        let time_fit = self.cells_w() / self.fit_viewport(doc).width() * cycle_units;
        let row_fit = self.fit_row_px(doc).min(f64::from(ROW_PX_DEFAULT) * z);
        let lo = time_fit.min(row_fit).clamp(f64::from(ROW_PX_MIN) * z, cap);
        (lo.ln(), cap.ln())
    }

    pub(super) fn selection_box(&self) -> Option<ZoomPreview> {
        match self.drag {
            Some(Drag::BoxZoom(selection)) => Some(selection.preview()),
            _ => None,
        }
    }

    fn begin_selection(&mut self, doc: &mut Document, start: Point, target: ZoomTarget) {
        let viewport = self.nav.viewport(doc);
        self.nav.viewport_state_mut(doc).set(viewport);
        if target == ZoomTarget::Cells {
            self.rows.set(self.layout.rows.unzoomed(self.layout.zoom));
        }
        self.drag = Some(Drag::BoxZoom(ZoomSelection::new(
            target,
            start,
            &self.layout,
        )));
    }

    fn fit_selection(&mut self, doc: &mut Document, rect: Rect, target: ZoomTarget, now: Instant) {
        let viewport = self.nav.viewport(doc);
        if target == ZoomTarget::Occupancy {
            let target = Viewport {
                start: self.layout.time_at(&viewport, rect.left()),
                end: self.layout.time_at(&viewport, rect.right()),
            };
            self.suspend_follow();
            self.nav.animate_to(doc, target, now);
            return;
        }
        let mut rows = self.rows.value.zoomed(self.layout.zoom);
        let bounds = self.zoom_box(doc, &viewport);
        let from = self.zoom_point(&viewport, rows);
        let factor = (self.cells_w() / f64::from(rect.width()))
            .min(f64::from(self.cells_h()) / f64::from(rect.height()))
            .min((bounds.time.1 - from.time).exp().max(1.0))
            .min((bounds.rows.1 - from.rows).exp().max(1.0));
        let center = Point {
            x: rect.left() + rect.width() * 0.5,
            y: rect.top() + rect.height() * 0.5,
        };
        let time = self.layout.time_at(&viewport, center.x);
        let row = rows.row_at(center.y - self.layout.cells.top());
        let width = viewport.width() / factor;
        let mut target = Viewport {
            start: time - width * 0.5,
            end: time + width * 0.5,
        };
        target.clamp(doc.limits());
        let factor = factor.min(viewport.width() / target.width());
        self.suspend_follow();
        self.nav.animate_to(doc, target, now);
        rows.row_px = (f64::from(rows.row_px) * factor) as f32;
        rows.top = row - f64::from(self.cells_h()) / (2.0 * f64::from(rows.row_px));
        self.set_rows(doc, rows, Some(now));
    }

    pub fn zoom_about(&mut self, doc: &mut Document, p: Point, factor: f64, now: Option<Instant>) {
        let x = f64::from((p.x - self.layout.cells.left()).max(0.0));
        let y = (p.y - self.layout.cells.top()).max(0.0);
        self.zoom_square(doc, Some((x, y)), factor, now);
    }

    fn zoom_square(
        &mut self,
        doc: &mut Document,
        anchor: Option<(f64, f32)>,
        factor: f64,
        now: Option<Instant>,
    ) {
        if !(self.laid_out() && factor.is_finite() && factor > 0.0) || factor == 1.0 {
            return;
        }
        self.suspend_follow();
        let animated = now.is_some();
        let viewport = if animated {
            self.nav.viewport_state(doc).target()
        } else {
            self.nav.viewport(doc)
        };
        let rows = self.rows_base(animated);
        let w = self.cells_w();
        let (x, y) = anchor.unwrap_or_else(|| {
            let x = self
                .nav
                .cursor(doc)
                .map(|t| viewport.x_of(t as f64, w))
                .filter(|x| (0.0..=w).contains(x))
                .unwrap_or(w * 0.5);
            (x, self.cells_h() * 0.5)
        });
        let cycle_units = self.cycle_units(doc, viewport.time_at(x, w));
        let size = self.square_size_limits(doc, cycle_units);
        let from = self.zoom_point(&viewport, rows);
        let to = zoom::square_zoom(from, cycle_units, size, factor.ln());
        if factor < 1.0 && to.rows <= size.0 + 1e-9 {
            self.fit_square(doc, now);
            return;
        }
        self.set_square_view(
            doc,
            (viewport, rows),
            to.rows.exp(),
            cycle_units,
            (x, y),
            now,
        );
    }

    fn set_square_view(
        &mut self,
        doc: &mut Document,
        from: (Viewport, RowView),
        side: f64,
        cycle_units: f64,
        anchor: (f64, f32),
        now: Option<Instant>,
    ) {
        let (viewport, mut rows) = from;
        let w = self.cells_w();
        let (x, y) = anchor;
        let width = w * cycle_units / side;
        let time = viewport.time_at(x, w);
        let start = time - width * (x / w);
        let mut target = Viewport {
            start,
            end: start + width,
        };
        target.clamp_preserving_width(doc.limits());
        let side = w * cycle_units / target.width();
        rows.zoom_about(
            y,
            side / f64::from(rows.row_px),
            self.cells_h(),
            self.row_count(doc),
            self.layout.zoom,
        );
        let mode = doc.navigation.animation;
        let state = self.nav.viewport_state_mut(doc);
        match now {
            Some(now) => state.animate_to(target, now, mode),
            None => state.set(target),
        }
        self.set_rows(doc, rows, now);
    }

    fn zoom_rows_at(&mut self, doc: &mut Document, y: f32, factor: f64, now: Option<Instant>) {
        if !(self.laid_out() && factor.is_finite() && factor > 0.0) {
            return;
        }
        self.suspend_follow();
        let z = self.layout.zoom;
        let mut rows = self.rows_base(now.is_some());
        let current = f64::from(rows.row_px);
        let lo = self
            .fit_row_px(doc)
            .min(f64::from(ROW_PX_DEFAULT * z))
            .min(current);
        let hi = f64::from(ROW_PX_MAX * z);
        let target = (current * factor).clamp(lo, hi);
        rows.zoom_about(y, target / current, self.cells_h(), self.row_count(doc), z);
        self.row_cap = (rows.row_px / z).max(ROW_PX_CAP);
        self.set_rows(doc, rows, now);
    }

    pub fn step_row_height(&mut self, doc: &mut Document, delta: isize, now: Instant) {
        if delta == 0 {
            return self.reset_row_height(doc, now);
        }
        let factor = ROW_HEIGHT_STEP.powi(delta.signum() as i32);
        let y = self.cells_h() * 0.5;
        self.zoom_rows_at(doc, y, factor, Some(now));
    }

    pub fn reset_row_height(&mut self, doc: &mut Document, now: Instant) {
        if !self.laid_out() {
            return;
        }
        self.suspend_follow();
        let z = self.layout.zoom;
        let mut rows = self.rows_target();
        let h = self.cells_h();
        let factor = f64::from(ROW_PX_DEFAULT * z) / f64::from(rows.row_px);
        rows.zoom_about(h * 0.5, factor, h, self.row_count(doc), z);
        self.row_cap = ROW_PX_CAP;
        self.set_rows(doc, rows, Some(now));
    }

    pub fn zoom_in(&mut self, doc: &mut Document, now: Instant) {
        self.zoom_square(doc, None, 2.0, Some(now));
    }

    pub fn zoom_out(&mut self, doc: &mut Document, now: Instant) {
        self.zoom_square(doc, None, 0.5, Some(now));
    }

    pub fn zoom_fit(&mut self, doc: &mut Document, now: Instant) {
        if !self.laid_out() {
            return self.nav.zoom_fit(doc, self.cells_w(), 0.0, now);
        }
        self.fit_square(doc, Some(now));
    }

    fn fit_square(&mut self, doc: &mut Document, now: Option<Instant>) {
        self.suspend_follow();
        let fit = self.fit_viewport(doc);
        let cycle_units = self.cycle_units(doc, fit.start.midpoint(fit.end));
        let side = self.square_size_limits(doc, cycle_units).0.exp();
        let h = self.cells_h();
        let mut rows = self.rows_target();
        rows.top = self.row_count(doc) as f64 * 0.5 - f64::from(h) / (2.0 * f64::from(rows.row_px));
        self.set_square_view(
            doc,
            (fit, rows),
            side,
            cycle_units,
            (self.cells_w() * 0.5, h * 0.5),
            now,
        );
    }

    pub fn zoom_to_cursor(&mut self, doc: &mut Document, now: Instant) {
        self.nav.zoom_to_cursor(doc, now);
    }

    pub fn go_to_start(&mut self, doc: &mut Document, now: Instant) {
        self.nav.go_to_start(doc, now);
    }

    pub fn go_to_end(&mut self, doc: &mut Document, now: Instant) {
        self.nav.go_to_end(doc, now);
    }

    pub fn go_to_cursor(&mut self, doc: &mut Document, now: Instant) {
        self.nav.go_to_cursor(doc, now);
    }

    pub fn pan_fraction(&mut self, doc: &mut Document, frac: f64, now: Instant) {
        self.nav.pan_fraction(doc, frac, now);
    }

    pub fn scroll_rows(&mut self, doc: &mut Document, rows: f64, now: Instant) {
        if rows == 0.0 {
            return;
        }
        let mut target = self.rows_target();
        target.top += rows;
        target.clamp(self.cells_h(), self.row_count(doc), self.layout.zoom);
        self.move_stream(doc, target, Some(now));
    }

    pub fn navigator_command(&mut self, command: super::NavigatorCommand) {
        use super::NavigatorCommand;
        match command {
            NavigatorCommand::Toggle => self.navigator.visible = !self.navigator.visible,
            NavigatorCommand::Widen | NavigatorCommand::Narrow => {
                let delta = if command == NavigatorCommand::Widen {
                    16.0
                } else {
                    -16.0
                };
                self.navigator.visible = true;
                self.navigator.width = (self.navigator.width + delta)
                    .clamp(super::navigator::MIN_WIDTH, super::navigator::MAX_WIDTH);
            }
        }
    }

    fn navigate_at(&mut self, doc: &mut Document, y: f32) {
        let Some(row) = self.layout.navigator.row_at(y) else {
            return;
        };
        let anchor_time = match self.rows(doc) {
            Rows::Ready(set) => self.row_anchor_time(&set, row),
            _ => None,
        };
        let Some(begin) = anchor_time else {
            return;
        };
        self.suspend_follow();
        let mut rows = self.rows.value.zoomed(self.layout.zoom);
        rows.top = row as f64;
        self.set_rows(doc, rows, None);
        let mut viewport = self.nav.viewport(doc);
        let width = viewport.width();
        viewport.start = begin as f64;
        viewport.end = viewport.start + width;
        viewport.clamp_preserving_width(doc.limits());
        self.nav.viewport_state_mut(doc).set(viewport);
    }

    fn move_stream(&mut self, doc: &mut Document, rows: RowView, now: Option<Instant>) {
        self.suspend_follow();
        let old = if now.is_some() {
            self.rows_target()
        } else {
            self.rows.value.zoomed(self.layout.zoom)
        };
        let mut viewport = if now.is_some() {
            self.nav.viewport_state(doc).target()
        } else {
            self.nav.viewport(doc)
        };
        let shift = match self.rows(doc) {
            Rows::Ready(set) if !set.is_empty() => {
                let row = |top: f64| top.floor().max(0.0).min((set.len() - 1) as f64) as usize;
                let from_idx = row(old.top);
                let to_idx = row(rows.top);
                if self.order.is_start() && self.stage_visibility.is_empty() {
                    match (set.get(from_idx), set.get(to_idx)) {
                        (Some((_, from)), Some((_, to))) => {
                            super::navigator::follow_shift(viewport, from, to)
                        }
                        _ => 0.0,
                    }
                } else {
                    match (
                        self.row_anchor_time(&set, from_idx),
                        self.row_anchor_time(&set, to_idx),
                    ) {
                        (Some(from_key), Some(to_key)) => {
                            (i128::from(to_key) - i128::from(from_key)) as f64
                        }
                        _ => 0.0,
                    }
                }
            }
            _ => 0.0,
        };
        viewport.start += shift;
        viewport.end += shift;
        viewport.clamp_preserving_width(doc.limits());
        match now {
            Some(now) => self.nav.animate_to(doc, viewport, now),
            None => self.nav.viewport_state_mut(doc).set(viewport),
        }
        self.set_rows(doc, rows, now);
    }

    fn pan_px(&mut self, doc: &mut Document, dx: f32, dy: f32) {
        if dx != 0.0 {
            let w = self.cells_w();
            let mut viewport = self.nav.viewport(doc);
            let dt = f64::from(dx) / w * viewport.width();
            viewport.start += dt;
            viewport.end += dt;
            viewport.clamp_preserving_width(doc.limits());
            self.nav.viewport_state_mut(doc).set(viewport);
        }
        if dy != 0.0 {
            self.suspend_follow();
            let mut rows = self.rows.value.zoomed(self.layout.zoom);
            rows.pan_px(dy, self.cells_h(), self.row_count(doc), self.layout.zoom);
            self.set_rows(doc, rows, None);
        }
    }

    pub fn escape(&mut self, doc: &mut Document) -> bool {
        if self.drag.is_some() {
            self.drag = None;
            false
        } else if self.selected_row(doc).is_some() || self.nav.ruler_selection().is_some() {
            self.nav.clear_ruler_selection();
            self.nav.select(doc, None);
            false
        } else if self.focused_source.is_some() || self.sources.iter().any(|s| !s.visible) {
            self.source_command(doc, PipelineSourceCommand::ClearFocusAndShowAll)
                .unwrap_or(false)
        } else {
            self.nav.set_cursor(doc, None);
            false
        }
    }

    #[must_use]
    pub fn clock<'a>(&self, doc: &'a Document) -> Option<&'a crate::clock::Clock> {
        self.sources
            .iter()
            .find_map(|s| doc.clocks.linked(s.track.track()?))
    }

    pub fn clocks<'a>(&self, doc: &'a Document) -> Vec<&'a crate::clock::Clock> {
        let mut clocks = Vec::new();
        for source in &self.sources {
            if let Some(track) = source.track.track()
                && let Some(clock) = doc.clocks.linked(track)
                && !clocks
                    .iter()
                    .any(|c: &&crate::clock::Clock| c.key() == clock.key())
            {
                clocks.push(clock);
            }
        }
        clocks
    }

    fn cycle_at(&self, doc: &Document, x: f32) -> u64 {
        let t = self.layout.time_at(&self.nav.viewport(doc), x).max(0.0);
        if let Some(at) = self
            .clock(doc)
            .and_then(|c| c.timeline())
            .and_then(|tl| tl.cycle_at(t as u64))
        {
            return at.edge;
        }
        let (a, b) = doc.limits();
        (t.floor() as u64).clamp(a, b)
    }

    fn span_text(&self, doc: &Document, begin: u64, end: u64) -> String {
        let Some(tl) = self.clock(doc).and_then(|c| c.timeline()) else {
            return format!("[{begin}, {end})");
        };
        let view = self.nav.clocks();
        let cycle = |t: u64| tl.cycle_at(t).map(|c| view.display_cycle(tl, c.cycle));
        match (cycle(begin), cycle(end)) {
            (Some(b), Some(e)) => {
                let n = e - b;
                format!(
                    "cycles {b}–{e} ({n} cycle{})",
                    if n == 1 { "" } else { "s" }
                )
            }
            _ => format!("[{begin}, {end})"),
        }
    }

    pub fn pointer(
        &mut self,
        doc: &mut Document,
        panel: PanelId,
        event: PointerEvent,
        now: Instant,
    ) -> bool {
        if matches!(self.drag, Some(Drag::BoxZoom(_)))
            && matches!(
                event,
                PointerEvent::Wheel { .. } | PointerEvent::Pinch { .. }
            )
        {
            return false;
        }
        match event {
            PointerEvent::Down {
                position,
                button,
                modifiers,
            } => {
                self.pointer_down(doc, panel, position, button, modifiers, now);
                true
            }
            PointerEvent::Move { position } => self.pointer_move(doc, position),
            PointerEvent::Up => {
                let drag = self.drag.take();
                if let Some(Drag::BoxZoom(selection)) = drag {
                    let preview = selection.preview();
                    if preview.valid {
                        self.fit_selection(doc, preview.rect, selection.target(), now);
                    }
                    return true;
                }
                if let Some(Drag::OccupancySplit { moved: false, .. }) = drag {
                    self.occupancy.visible = !self.occupancy.visible;
                    return true;
                }
                if let Some(Drag::NavigatorSplit { moved: false, .. }) = drag {
                    self.navigator.visible = !self.navigator.visible;
                    return true;
                }
                if let Some(Drag::Marker(held)) = drag {
                    held.release(doc, &mut self.nav, now);
                    return true;
                }
                if let Some(Drag::Pan {
                    button: MouseButton::Middle,
                    start,
                    moved: false,
                    ..
                }) = drag
                {
                    self.measure_from(doc, start);
                    return true;
                }
                if let Some(Drag::Pan {
                    button: MouseButton::Left,
                    start,
                    moved: false,
                    ..
                }) = drag
                    && self.layout.cells.contains(start)
                {
                    let cycle = self.cycle_at(doc, start.x);
                    self.nav.set_cursor(doc, Some(cycle));
                    match self
                        .layout
                        .row_at(start.y)
                        .filter(|_| !self.layout.is_overview())
                    {
                        Some(row) => {
                            self.select_row(doc, panel, row);
                        }
                        None => {
                            self.nav.select(doc, None);
                        }
                    }
                }
                drag.is_some()
            }
            PointerEvent::Leave => {
                let had = self.pointer.is_some() || self.hover.is_some();
                self.pointer = None;
                self.hover = None;
                had
            }
            PointerEvent::Wheel {
                position,
                dx,
                dy,
                modifiers,
                ..
            } => {
                self.wheel(doc, position, dx, dy, modifiers);
                true
            }
            PointerEvent::Pinch { position, delta } => {
                let factor = f64::from(1.0 + delta).clamp(0.2, 5.0);
                if self.layout.occupancy.contains(position) {
                    self.nav.zoom_at(
                        doc,
                        f64::from(position.x - self.layout.cells.left()),
                        self.cells_w(),
                        factor,
                    );
                } else {
                    self.zoom_about(doc, position, factor, None);
                }
                true
            }
        }
    }

    fn measure_from(&mut self, doc: &mut Document, p: Point) {
        let x =
            p.x.clamp(self.layout.cells.left(), self.layout.cells.right());
        let t = self.cycle_at(doc, x);
        let viewport = self.nav.viewport(doc);
        let width = self.layout.cells_width_f64();
        let raw = self.layout.time_at(&viewport, x);
        let tolerance =
            doc.navigation.snap_px * f64::from(self.layout.zoom) / viewport.px_per_unit(width);
        let reference = crate::marker::reference_near(doc.markers(), raw, tolerance, t);
        doc.set_reference(Some(reference));
    }

    fn pointer_down(
        &mut self,
        doc: &mut Document,
        panel: PanelId,
        p: Point,
        button: MouseButton,
        modifiers: Modifiers,
        now: Instant,
    ) {
        self.pointer = Some(p);
        if self.layout.occupancy_split.contains(p) {
            if button == MouseButton::Left {
                self.drag = Some(Drag::OccupancySplit {
                    start: p.y,
                    height: if self.occupancy.visible {
                        self.layout.occupancy.height() / self.layout.zoom
                    } else {
                        0.0
                    },
                    moved: false,
                });
            }
            return;
        }
        if self.layout.occupancy_labels.contains(p) {
            return;
        }
        if self.layout.occupancy.contains(p) {
            if button == MouseButton::Left {
                if modifiers.control || modifiers.platform {
                    self.begin_selection(doc, p, ZoomTarget::Occupancy);
                } else {
                    self.drag = Some(Drag::Occupancy);
                    self.occupancy_at(doc, p.x);
                }
            }
            return;
        }
        if button == MouseButton::Left
            && let Some(Hit::Activity(command)) = self.hit_at(p, doc)
        {
            self.activity_command(doc, command);
            return;
        }
        if self.layout.navigator_split.contains(p) || self.layout.navigator_grip_target.contains(p)
        {
            if button == MouseButton::Left {
                self.drag = Some(Drag::NavigatorSplit {
                    start: p.x,
                    width: self.layout.navigator.rect.width() / self.layout.zoom,
                    saved_width: self.navigator.width,
                    moved: false,
                });
            }
            return;
        }
        if self.layout.navigator.rect.contains(p) {
            if button == MouseButton::Left {
                self.drag = Some(Drag::Navigator);
                self.navigate_at(doc, p.y);
            }
            return;
        }
        if self.layout.gutter.contains(p) {
            return;
        }
        let lane_hit = self.layout.marker_lane.hit(p);
        let layout = &self.layout;
        let in_time = p.x >= layout.cells.left() && layout.bounds.contains(p);
        if button == MouseButton::Left && modifiers.alt && in_time && lane_hit.is_none() {
            self.measure_from(doc, p);
            return;
        }
        if button == MouseButton::Left {
            if layout.label_split.contains(p) {
                self.drag = Some(Drag::LabelSplit {
                    grab: p.x - layout.cells.left(),
                });
                return;
            }
            if layout.retry.is_some_and(|r| r.contains(p)) {
                self.retry(doc);
                return;
            }
            match lane_hit {
                Some(LaneHit::Chip(ix)) => {
                    let marker_x = doc.markers().get(ix).map_or(p.x, |m| {
                        layout.cells.left()
                            + self
                                .nav
                                .viewport(doc)
                                .x_of(m.time as f64, layout.cells_width_f64())
                                as f32
                    });
                    self.drag =
                        crate::marker::MarkerDrag::begin(doc, ix, p, marker_x).map(Drag::Marker);
                    return;
                }
                Some(LaneHit::Span(_)) | None => {}
                Some(hit) => {
                    crate::marker::press(doc, &mut self.nav, hit, now);
                    return;
                }
            }
            let rulers: Vec<ClockKey> = self
                .nav
                .clocks()
                .rulers(&doc.clocks)
                .iter()
                .map(|c| c.key())
                .collect();
            let ruler = layout.ruler_at(p, rulers.len());
            if let Some(ix) = ruler {
                self.nav.select(doc, None);
                if modifiers.secondary() && self.nav.ruler_selection() == Some(&rulers[ix]) {
                    self.nav.clear_ruler_selection();
                } else {
                    self.nav.select_ruler(rulers[ix].clone());
                }
            }
            let strip = layout.header.contains(p) || layout.marker_lane.strip().contains(p);
            if (strip || ruler.is_some()) && p.x >= layout.cells.left() {
                let cycle = self.cycle_at(doc, p.x);
                self.nav.set_cursor(doc, Some(cycle));
                self.drag = Some(Drag::Cursor);
                return;
            }
            if ruler.is_some() {
                return;
            }
            if let Some(source) = self.label_badge_at(doc, p) {
                drop(self.source_command(doc, PipelineSourceCommand::ToggleFocus(source)));
                return;
            }
            if let Some(row) = self.label_row_at(doc, p) {
                self.select_row(doc, panel, row);
                return;
            }
        }
        if button == MouseButton::Left
            && (modifiers.control || modifiers.platform)
            && layout.cells.contains(p)
        {
            self.begin_selection(doc, p, ZoomTarget::Cells);
            return;
        }
        if layout.cells.contains(p) || (button == MouseButton::Middle && in_time) {
            self.drag = Some(Drag::Pan {
                button,
                start: p,
                last: p,
                moved: false,
            });
        }
    }

    fn pointer_move(&mut self, doc: &mut Document, p: Point) -> bool {
        self.pointer = Some(p);
        match self.drag {
            Some(Drag::Occupancy) => {
                self.occupancy_at(doc, p.x);
                true
            }
            Some(Drag::OccupancySplit {
                start,
                height,
                moved,
            }) => {
                let delta = (start - p.y) / self.layout.zoom;
                let moved = moved || delta.abs() >= DRAG_THRESHOLD_PX;
                self.drag = Some(Drag::OccupancySplit {
                    start,
                    height,
                    moved,
                });
                if moved {
                    self.occupancy.visible = height + delta >= super::plot::MIN_HEIGHT * 0.5;
                    if self.occupancy.visible {
                        self.occupancy.height = (height + delta)
                            .clamp(super::plot::MIN_HEIGHT, super::plot::MAX_HEIGHT);
                    }
                }
                true
            }
            Some(Drag::BoxZoom(mut selection)) => {
                selection.move_to(p);
                self.drag = Some(Drag::BoxZoom(selection));
                true
            }
            Some(Drag::Pan {
                button,
                start,
                last,
                moved,
            }) => {
                let threshold = DRAG_THRESHOLD_PX * self.layout.zoom;
                let moved = moved || (p.x - start.x).hypot(p.y - start.y) > threshold;
                self.drag = Some(Drag::Pan {
                    button,
                    start,
                    last: p,
                    moved,
                });
                if moved {
                    self.pan_px(doc, last.x - p.x, last.y - p.y);
                }
                moved
            }
            Some(Drag::Marker(mut held)) => {
                let Some(x) = held.target_x(p, self.layout.zoom) else {
                    return false;
                };
                let x = x.clamp(self.layout.cells.left(), self.layout.cells.right());
                let cycle = self.cycle_at(doc, x);
                self.drag = Some(Drag::Marker(held));
                doc.move_marker(held.id, cycle);
                true
            }
            Some(Drag::Cursor) => {
                let x =
                    p.x.clamp(self.layout.cells.left(), self.layout.cells.right());
                let cycle = self.cycle_at(doc, x);
                self.nav.set_cursor(doc, Some(cycle))
            }
            Some(Drag::Navigator) => {
                self.navigate_at(doc, p.y);
                true
            }
            Some(Drag::NavigatorSplit {
                start,
                width,
                saved_width,
                moved,
            }) => {
                let delta = (start - p.x) / self.layout.zoom;
                let moved = moved || delta.abs() >= DRAG_THRESHOLD_PX;
                self.drag = Some(Drag::NavigatorSplit {
                    start,
                    width,
                    saved_width,
                    moved,
                });
                if moved {
                    let requested = width + delta;
                    self.navigator.visible = requested >= super::navigator::MIN_WIDTH * 0.5;
                    if self.navigator.visible {
                        self.navigator.width = requested
                            .clamp(super::navigator::MIN_WIDTH, super::navigator::MAX_WIDTH);
                    } else {
                        self.navigator.width = saved_width;
                    }
                }
                true
            }
            Some(Drag::LabelSplit { grab }) => {
                self.label_width = ((p.x - grab - self.layout.bounds.left()) / self.layout.zoom)
                    .clamp(LABEL_W_MIN, LABEL_W_MAX);
                true
            }
            None => {
                let before = self.hover;
                self.hover = self.hit_at(p, doc);
                self.hover != before
            }
        }
    }

    fn wheel(&mut self, doc: &mut Document, p: Point, dx: f32, dy: f32, modifiers: Modifiers) {
        if self.layout.occupancy.contains(p) {
            if modifiers.control || modifiers.platform {
                self.nav.zoom_at(
                    doc,
                    f64::from(p.x - self.layout.cells.left()),
                    self.cells_w(),
                    2f64.powf(f64::from(dy) / 120.0),
                );
            } else {
                self.pan_px(doc, -(dx + dy), 0.0);
            }
            return;
        }
        if self.layout.occupancy_split.contains(p) || self.layout.occupancy_labels.contains(p) {
            return;
        }
        if self.layout.gutter.contains(p)
            || self.layout.navigator_split.contains(p)
            || self.layout.navigator_grip_target.contains(p)
        {
            return;
        }
        let factor = 2f64.powf(f64::from(dy) / 120.0);
        if modifiers.control || modifiers.platform {
            self.zoom_about(doc, p, factor, None);
        } else if modifiers.alt {
            let y = (p.y - self.layout.cells.top()).max(0.0);
            self.zoom_rows_at(doc, y, factor, None);
        } else if modifiers.shift {
            self.pan_px(doc, -(dx + dy), 0.0);
        } else {
            self.pan_px(doc, -dx, 0.0);
            if dy != 0.0 {
                let mut rows = self.rows.value.zoomed(self.layout.zoom);
                rows.pan_px(-dy, self.cells_h(), self.row_count(doc), self.layout.zoom);
                self.move_stream(doc, rows, None);
            }
        }
    }
}
