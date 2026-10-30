//! `App`: the whole viewer with no toolkit attached.

mod opening;
mod traces;
mod undo;

use crate::frames::FrameStats;
use crate::history::History;
use crate::sidebar::ActivityModel;
use std::collections::HashSet;
pub use traces::TraceChip;

use std::sync::Arc;

use web_time::Instant;

use crate::clock::ClockKey;
use crate::document::{Delivered, Document, TraceState};
use crate::geometry::{Modifiers, Rect};
use crate::geometry::{MouseButton, Point};
use crate::marker::{LaneHit, LaneVerb, Reference};
use crate::panels::{Panel, PanelId, PanelKind, Panels, PanelsCommand};
use crate::pipeline::{PipelineLayout, PipelineModel, TrackSource};
use crate::scene::{Scene, TextCache, TextMeasure};
use crate::session::{LoadRequest, LoadResult};
use crate::settings::{self, Value};
use crate::sidebar::{Key, MemberListModel, ScopeTreeModel, TreeNode};
use crate::theme::Theme;
use crate::trace::{TraceId, Traced};
use crate::transaction::{TransactionCommand, TransactionModel};
use crate::wave::layout::WaveLayout;
use crate::wave::model::{
    MenuAction, MenuEntry, MenuItem, PointerEvent, WaveMenu, WaveMenuKind, WaveRow,
};
use crate::wave::timeline::format_time;
use volna_trace::data::Member;
use volna_trace::data::transactions::{TrackRef, TransactionRef};
use volna_trace::data::{ScopeId, SignalRef, VarId};
use volna_trace::session::{LoadRequest as TraceRequest, OpenSpec, Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    SplitRight,
    SplitDown,
    NewPanel,
    ClosePanel,
    FocusNextPanel,
    FocusPrevPanel,
    ToggleViewportLink,
    ToggleCursorLink,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    ZoomToCursor,
    PanPageLeft,
    PanPageRight,
    GoToStart,
    GoToEnd,
    GoToCursor,
    PanLeft,
    PanRight,
    NextEdge,
    PrevEdge,
    AddOrRenameMarker,
    SetMarker(crate::marker::MarkerId),
    RemoveMarkerAtCursor,
    RemoveAllMarkers,
    NextMarker,
    PrevMarker,
    GoToMarker(crate::marker::MarkerId),
    JumpBack,
    SetReference,
    ClearReference,
    ZoomToMeasurement,
    MarkerNavigator,
    RemoveSelected,
    CopySignals,
    CutSignals,
    PasteSignals,
    SelectAll,
    ClearSelection,
    CycleFormat,
    ToggleAnalog,
    ToggleStack,
    ToggleOverlaid,
    SetTint(Option<crate::wave::Tint>),
    IncreaseRowHeight,
    DecreaseRowHeight,
    ResetRowHeight,
    MoveSelectionUp,
    MoveSelectionDown,
    GroupSelection,
    Ungroup,
    RenameGroup,
    FoldGroupDeep,
    UnfoldGroupDeep,
    NextCycle,
    PrevCycle,
    ToggleCycleOrigin,
}

impl Command {
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        let action = match name {
            "openWorkspace" => return Some(Self::RequestOpenWorkspace),
            "undo" => return Some(Self::Undo),
            "redo" => return Some(Self::Redo),
            "saveWorkspace" => return Some(Self::SaveWorkspace),
            "saveWorkspaceAs" => return Some(Self::RequestSaveWorkspaceAs),
            "splitRight" => Action::SplitRight,
            "splitDown" => Action::SplitDown,
            "newPanel" => Action::NewPanel,
            "closePanel" => Action::ClosePanel,
            "focusNextPanel" => Action::FocusNextPanel,
            "focusPrevPanel" => Action::FocusPrevPanel,
            "toggleViewportLink" => Action::ToggleViewportLink,
            "toggleCursorLink" => Action::ToggleCursorLink,
            "nextCycle" => Action::NextCycle,
            "prevCycle" => Action::PrevCycle,
            "toggleCycleOrigin" => Action::ToggleCycleOrigin,
            "nextMarker" => Action::NextMarker,
            "prevMarker" => Action::PrevMarker,
            "jumpBack" => Action::JumpBack,
            "setReference" => Action::SetReference,
            "clearReference" => Action::ClearReference,
            "zoomToMeasurement" => Action::ZoomToMeasurement,
            "markerNavigator" => Action::MarkerNavigator,
            _ if name.starts_with("color") => {
                let tint = match name.strip_prefix("color")? {
                    "Default" => None,
                    rest => Some(crate::wave::Tint::from_id(&rest.to_lowercase())?),
                };
                Action::SetTint(tint)
            }
            _ if name.starts_with("goToMarker") => {
                let n = name.strip_prefix("goToMarker")?.parse::<u32>().ok()?;
                Action::GoToMarker(crate::marker::MarkerId::new(n)?)
            }
            _ if name.starts_with("setMarker") => {
                let n = name.strip_prefix("setMarker")?.parse::<u32>().ok()?;
                Action::SetMarker(crate::marker::MarkerId::new(n)?)
            }
            _ => {
                let index = name.strip_prefix("focusPanel")?.parse::<usize>().ok()?;
                return (1..=9)
                    .contains(&index)
                    .then_some(Self::Panels(PanelsCommand::FocusIndex(index - 1)));
            }
        };
        Some(Self::Action(action))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditTarget {
    Group {
        panel: PanelId,
        entry: usize,
    },
    Slice {
        panel: PanelId,
        entry: usize,
    },
    Marker {
        panel: PanelId,
        id: crate::marker::MarkerId,
    },
}

impl EditTarget {
    #[must_use]
    pub fn panel(self) -> PanelId {
        match self {
            Self::Group { panel, .. } | Self::Slice { panel, .. } | Self::Marker { panel, .. } => {
                panel
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextEdit {
    pub target: EditTarget,
    pub rect: Option<Rect>,
    pub text: String,
    pub select_all: bool,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeDrag {
    Sidebar,
    ScopesSplit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Notice(String),
    BuildActivity(TraceId),
    DismissActivity(TraceId),
    CancelActivity(TraceId),
    Open(OpenSpec),
    AddTrace(OpenSpec),
    RequestOpenDialog,
    RequestAddTraceDialog,
    RemoveTrace(TraceId),
    RenameTrace(TraceId, Option<String>),
    RevealTrace(TraceId),
    RequestOpenWorkspace,
    RequestSaveWorkspaceAs,
    SaveWorkspace,
    RequestQuit,
    CloseTrace,
    ToggleSidebar,
    Undo,
    Redo,
    Action(Action),
    Pointer(PanelId, PointerEvent),
    Panels(PanelsCommand),
    AddVars(Vec<Traced<VarId>>),
    AddToWaves(Vec<Traced<Member>>),
    AddClockRulers(Vec<Traced<Member>>),
    ActivateMembers(Vec<Traced<Member>>),
    OpenPipeline {
        track: Traced<TrackRef>,
    },
    OpenTable {
        selected: Vec<Traced<Member>>,
        clicked: Option<Traced<Member>>,
    },
    OpenTableFromPanel {
        panel: PanelId,
        row: Option<usize>,
    },
    Table(PanelId, crate::table::TableCommand),
    SelectTransaction {
        panel: PanelId,
        track: Traced<TrackRef>,
        id: TransactionRef,
        cursor: Option<u64>,
    },
    ShowTransaction {
        from: PanelId,
    },
    ShowAttributeColumn {
        from: PanelId,
        key: String,
    },
    PanToTransactionStart {
        panel: PanelId,
        row: usize,
    },
    Transaction(PanelId, TransactionCommand),
    Clocks(ClockCommand),
    RevealTransaction {
        panel: PanelId,
    },
    PipelineActivity(PanelId, crate::pipeline::ActivityCommand),
    PipelineMeasured(PanelId, crate::pipeline::ViewportSize),
    SetSearchEverywhere(bool),
    SelectScope(TreeNode),
    ToggleScope(TreeNode),
    ExpandAllScopes(bool),
    ScopesKey(Key),
    SetFilter(String),
    SelectVar {
        ix: usize,
        modifiers: Modifiers,
    },
    AddAllVars,
    AddSelectedOrAllVars,
    VariablesKey(Key, Modifiers),
    AddScopeAsGroup {
        scope: Traced<ScopeId>,
        recursive: bool,
    },
    CommitText(EditTarget, Option<String>),
    RenameMarker(PanelId, crate::marker::MarkerId),
    Lane(crate::marker::LaneVerb),
    ZoomToSpan(PanelId, crate::marker::MarkerId, crate::marker::MarkerId),
    OpenSignalMenu(PanelId),
    MenuSelect(PanelId, MenuAction),
    MenuDismiss(PanelId),
    SetSidebarWidth(f32),
    SetScopesFraction(f32),
    ChromeDragStart(ChromeDrag),
    ChromeDragEnd,
    Settings(SettingsCommand),
    Recent(crate::workspace::recent::RecentCommand),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockChoices {
    pub clocks: Vec<(ClockKey, bool)>,
    pub origin: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClockCommand {
    ToggleRuler(ClockKey),
    Select(ClockKey),
    GoToCycle(i64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsCommand {
    Open,
    Close,
    Set { id: String, value: Value },
    Reset { id: String },
    ReplaceText(String),
    Query(String),
    ToggleJson,
    Reveal { id: String },
    Zoom(settings::ZoomStep),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Changed,
    LayoutChanged {
        revision: u64,
    },
    Notice(String),
    Announce(String),
    LoadWorkspace {
        trace_uri: String,
    },
    TraceOpened {
        trace_uri: String,
    },
    OpenRecent(crate::workspace::recent::Recent),
    RecentChanged,
    PersistWorkspace {
        ticket: crate::workspace::persistence::SaveTicket,
        bytes: Vec<u8>,
    },
    OpenWorkspaceDialog,
    SaveWorkspaceDialog,
    TraceClosed {
        trace_uri: String,
    },
    Quit,
    OpenFileDialog,
    AddTraceDialog,
    RevealScopeRow(usize),
    RevealVarRow(usize),
    FocusFilter,
    WriteSettings {
        ticket: u64,
        bytes: Vec<u8>,
    },
    SettingsChanged {
        keys: Vec<&'static str>,
    },
    FocusSettingsSearch,
    CopyText(String),
    OpenMarkerNavigator,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsView {
    pub query: String,
    pub json: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub sidebar_notice: Option<String>,
    pub workspace_notice: Option<String>,
    pub panel: Option<String>,
    pub hover: Option<String>,
    pub file: Option<String>,
    pub time_range: Option<String>,
    pub signals: Option<String>,
    pub changes: Option<String>,
    pub px_per: Option<String>,
    pub cursor: Option<String>,
    pub clocks: Vec<String>,
    pub measure: Option<String>,
    pub markers: Option<String>,
    pub announcement: Option<String>,
    pub frames: Option<crate::frames::FrameStatus>,
    pub memory: Option<MemoryStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryStatus {
    pub used: u64,
    pub limit: u64,
}

impl MemoryStatus {
    pub const NEARLY_FULL: f32 = 0.9;

    #[must_use]
    pub fn fraction(self) -> f32 {
        if self.limit == 0 {
            return 1.0;
        }
        (self.used as f64 / self.limit as f64).clamp(0.0, 1.0) as f32
    }

    #[must_use]
    pub fn nearly_full(self) -> bool {
        self.fraction() >= Self::NEARLY_FULL
    }

    #[must_use]
    pub fn label(self) -> String {
        let (unit, scale) = byte_unit(self.limit);
        format!(
            "{} / {} {unit}",
            short_amount(self.used as f64 / scale),
            short_amount(self.limit as f64 / scale)
        )
    }

    #[must_use]
    pub fn detail(self) -> String {
        let (unit, scale) = byte_unit(self.limit);
        format!(
            "Memory budget: {:.1} of {} {unit} used ({:.0}%) by loaded trace data. \
             Loads that would exceed the budget fail. Click to change the limits.",
            self.used as f64 / scale,
            short_amount(self.limit as f64 / scale),
            f64::from(self.fraction()) * 100.0
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryChoice {
    pub id: &'static str,
    pub mib: u64,
    pub label: String,
    pub checked: bool,
}

impl MemoryChoice {
    #[must_use]
    pub fn command(&self) -> Command {
        Command::Settings(SettingsCommand::Set {
            id: self.id.into(),
            value: settings::Value::Integer(self.mib as i64),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryMenu {
    pub budget_title: String,
    pub budget: Vec<MemoryChoice>,
    pub object_title: String,
    pub object: Vec<MemoryChoice>,
    pub object_note: Option<&'static str>,
    pub locked: Option<&'static str>,
}

pub const BUDGET_PRESETS_MIB: [u64; 8] = [256, 512, 1024, 2048, 4096, 8192, 16384, 32768];
pub const OBJECT_PRESETS_MIB: [u64; 7] = [64, 128, 256, 512, 1024, 2048, 4096];

#[must_use]
pub fn format_mib(mib: u64) -> String {
    if mib >= 1024 {
        format!("{} GiB", short_amount(mib as f64 / 1024.0))
    } else {
        format!("{mib} MiB")
    }
}

fn memory_choices(id: &'static str, presets: &[u64], current: u64) -> Vec<MemoryChoice> {
    let mut values = presets.to_vec();
    if !values.contains(&current) {
        values.push(current);
        values.sort_unstable();
    }
    values
        .into_iter()
        .map(|mib| MemoryChoice {
            id,
            mib,
            label: format_mib(mib),
            checked: mib == current,
        })
        .collect()
}

fn byte_unit(bytes: u64) -> (&'static str, f64) {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= 1024.0 * MIB {
        ("GiB", 1024.0 * MIB)
    } else {
        ("MiB", MIB)
    }
}

fn short_amount(value: f64) -> String {
    let text = if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    };
    text.strip_suffix(".0").map(str::to_owned).unwrap_or(text)
}

pub const SIDEBAR_FRACTION_MIN: f32 = 0.15;

#[derive(Clone, Debug, PartialEq)]
pub struct StartSummary {
    pub name: String,
    pub time_range: String,
    pub variables: usize,
    pub tracks: usize,
    pub pipelines: Vec<(String, Traced<TrackRef>)>,
}

pub enum PanelLayout<'a> {
    Waves(&'a WaveLayout),
    Pipeline(&'a PipelineLayout),
    Table(&'a crate::table::TableLayout),
}

pub struct App {
    pub doc: Document,
    pub workspace: crate::workspace::State,
    pub panels: Panels,
    pub scopes: ScopeTreeModel,
    pub variables: MemberListModel,
    pub activity: crate::sidebar::ActivityModel,
    pub sidebar_width: f32,
    pub sidebar_visible: bool,
    pub scopes_fraction: f32,
    pub drag: Option<ChromeDrag>,
    pub settings: settings::Store,
    pub settings_view: SettingsView,
    pub history: crate::history::History,
    announcement: Option<String>,
    marker_edit: Option<(PanelId, crate::marker::MarkerId)>,
    transient_menu: Option<(PanelId, WaveMenu)>,
    transient_menus: usize,
    pub frames: crate::frames::FrameStats,
    pub(crate) events: Vec<Event>,
    text: TextCache,
    scene: Scene,
    projections: crate::data::projection::Projections,
    table_budget: volna_trace::remote::memory::MemoryBudget,
    lane_tracks: (u64, std::collections::HashSet<Traced<TrackRef>>),
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        App {
            doc: Document::new(),
            workspace: crate::workspace::State::default(),
            panels: Panels::new(),
            scopes: ScopeTreeModel::default(),
            variables: MemberListModel::default(),
            activity: ActivityModel::default(),
            sidebar_width: 280.0,
            sidebar_visible: true,
            scopes_fraction: 0.42,
            drag: None,
            settings: settings::Store::new(settings::Host::Native),
            settings_view: SettingsView::default(),
            history: History::default(),
            announcement: None,
            marker_edit: None,
            transient_menu: None,
            transient_menus: 0,
            frames: FrameStats::default(),
            events: Vec::new(),
            text: TextCache::default(),
            scene: Scene::default(),
            projections: crate::data::projection::Projections::default(),
            table_budget: {
                let (limit, object_limit) = settings::Settings::default()
                    .limits()
                    .bytes()
                    .expect("valid defaults");
                let budget = volna_trace::remote::memory::MemoryBudget::new(limit);
                budget.set_object_limit(object_limit);
                budget
            },
            lane_tracks: Default::default(),
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn open_path(&mut self, path: std::path::PathBuf) {
        self.open(OpenSpec::Path(path));
    }

    pub fn open_bytes(&mut self, name: String, bytes: Vec<u8>) {
        self.open(OpenSpec::Bytes { name, bytes });
    }

    pub(crate) fn open_now(&mut self, spec: OpenSpec) {
        self.doc.open(spec);
        self.changed();
    }

    pub fn set_session(&mut self, session: Arc<dyn Session>) {
        let session = if session.memory_budget().is_none() {
            match self.account_local_session(session) {
                Ok(session) => session,
                Err(error) => {
                    self.events.push(Event::Notice(format!(
                        "Trace exceeds the memory budget: {error:#}"
                    )));
                    return;
                }
            }
        } else {
            session
        };
        self.doc.set_session(session);
        self.on_session_changed();
    }

    pub(crate) fn close_now(&mut self) {
        self.doc.close();
        self.on_session_changed();
    }

    fn on_session_changed(&mut self) {
        self.projections.clear();
        self.activity = ActivityModel::default();
        self.reset_recent_view();
        self.history.clear();
        self.announcement = None;
        self.marker_edit = None;
        self.transient_menu = None;
        if let Err(e) = self.panels.reset() {
            self.events.push(Event::Notice(e.to_string()));
        }
        self.layout_changed();
        let traces = self.doc.traces();
        self.scopes.reset(traces);
        self.variables.reset(traces);
        let scope = self.scopes.selected_scope();
        self.variables.set_scope(self.doc.traces(), scope);
        self.changed();
    }

    pub(crate) fn workspace_restored(&mut self) {
        self.history.clear();
        self.announcement = None;
        self.marker_edit = None;
        self.transient_menu = None;
        self.sync_lane_tracks();
        self.sync_analog_summaries();
        self.drag = None;
        self.layout_changed();
        self.changed();
    }

    pub fn take_requests(&mut self) -> Vec<LoadRequest> {
        let budget = self.table_memory_budget();
        let wanted = self
            .panels
            .iter()
            .filter_map(|p| p.kind.waves())
            .flat_map(crate::wave::WaveModel::projection_demand)
            .filter_map(|s| {
                Some((
                    crate::data::projection::Projections::key(s.raw_history()?, s.projection?),
                    s.raw_history()?.clone(),
                ))
            })
            .collect();
        let retained = self
            .panels
            .iter()
            .filter_map(|p| p.kind.waves())
            .flat_map(crate::wave::WaveModel::items)
            .filter_map(|e| e.signal())
            .filter_map(|s| {
                Some((
                    crate::data::projection::Projections::key(s.raw_history()?, s.projection?),
                    s.raw_history()?.clone(),
                ))
            })
            .collect();
        let mut requests = self.projections.sync(retained, wanted, &budget);
        for w in self.panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
            w.sync_projections(&self.projections);
        }
        requests.extend(self.doc.take_requests());
        {
            let budget = self.table_memory_budget();
            requests.extend(self.activity.sync(
                self.doc.traces(),
                self.activity_viewport(),
                &budget,
                self.settings.resolved().hierarchy.activity_index,
                self.sidebar_visible,
            ));
        }
        if requests
            .iter()
            .any(|r| matches!(r, LoadRequest::Trace(TraceRequest::Signals { .. })))
        {
            let wanted = self.signal_demand();
            requests.retain_mut(|request| {
                if let LoadRequest::Trace(TraceRequest::Signals {
                    tag: trace,
                    generation,
                    signals,
                    ..
                }) = request
                {
                    self.doc
                        .retain_queued_signals(*trace, *generation, signals, &wanted)
                } else {
                    true
                }
            });
        }
        requests
    }

    pub(crate) fn signal_demand(
        &self,
    ) -> std::collections::HashSet<Traced<volna_trace::data::SignalRef>> {
        self.panels
            .iter()
            .filter_map(|panel| panel.kind.waves())
            .flat_map(|waves| waves.items().iter().map(|e| &e.row))
            .filter_map(WaveRow::signal_ref)
            .chain(
                self.panels
                    .iter()
                    .filter_map(|panel| panel.kind.table())
                    .flat_map(|table| table.signal_demand()),
            )
            .collect()
    }

    #[must_use]
    pub fn activity_viewport(&self) -> crate::wave::Viewport {
        self.panels
            .focused()
            .kind
            .nav()
            .map_or(self.doc.shared.viewport.value, |nav| {
                nav.viewport(&self.doc)
            })
    }

    #[must_use]
    pub fn scope_activity(
        &self,
        node: crate::sidebar::TreeNode,
    ) -> Option<crate::data::ScopeActivity> {
        let total = self.activity.total(node.trace, node.scope?)?;
        let slot = self.doc.traces().get(node.trace)?;
        let window =
            crate::sidebar::activity::trace_window(self.activity_viewport(), slot.placement());
        let changing = self
            .activity
            .get(node.trace, slot.generation(), window, node.scope?)?;
        Some(crate::data::ScopeActivity { changing, total })
    }

    #[must_use]
    pub fn signal_activity(&self, signal: Traced<SignalRef>) -> Option<bool> {
        let slot = self.doc.traces().get(signal.trace)?;
        let window =
            crate::sidebar::activity::trace_window(self.activity_viewport(), slot.placement());
        self.activity
            .signal_active(signal.trace, slot.generation(), window, signal.item)
    }

    #[must_use]
    pub fn member_activity(&self, member: Traced<Member>) -> Option<bool> {
        let h = self.doc.hierarchy(member.trace)?;
        match member.item {
            Member::Var(id) => self.signal_activity(Traced::new(member.trace, h.signal(id))),
            Member::Generator(id) => {
                let slot = self.doc.traces().get(member.trace)?;
                let window = crate::sidebar::activity::trace_window(
                    self.activity_viewport(),
                    slot.placement(),
                );
                self.activity.generator_active(
                    member.trace,
                    slot.generation(),
                    window,
                    h.generators().get(id)?.track.0,
                )
            }
        }
    }

    pub fn deliver(&mut self, mut result: LoadResult) {
        if let LoadResult::Projection { request, result } = result {
            if self.projections.deliver(request, result) {
                for w in self.panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
                    w.sync_projections(&self.projections);
                }
                self.sync_analog_summaries();
                self.changed();
            }
            return;
        }
        if matches!(
            result,
            LoadResult::ActivityBuilt { .. }
                | LoadResult::ActivityCounter { .. }
                | LoadResult::Activity { .. }
        ) {
            let viewport = self.activity_viewport();
            match self.activity.deliver(result, self.doc.traces(), viewport) {
                Ok(true) => self.changed(),
                Ok(false) => {}
                Err(error) => {
                    self.events.push(Event::Notice(format!("{error:#}")));
                    self.changed();
                }
            }
            return;
        }
        if let LoadResult::Opened { result: opened, .. } = &mut result
            && opened
                .as_ref()
                .is_ok_and(|session| session.memory_budget().is_none())
        {
            let session = opened.as_ref().expect("checked successful open").clone();
            *opened = self.account_local_session(session);
        }
        match self.doc.deliver(result) {
            Some(Delivered::Summary | Delivered::Sizes) => self.changed(),
            Some(Delivered::Track) => {
                self.initialize_pipelines();
                for pipeline in self.panels.pipelines_mut() {
                    pipeline.refresh(&self.doc);
                }
                let Self { panels, doc, .. } = self;
                for waves in panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
                    waves.fit_lanes(doc);
                }
                for table in self.panels.iter_mut().filter_map(|p| p.kind.table_mut()) {
                    table.refresh(&self.doc);
                }
                for (_, model) in self.panels.transactions_mut() {
                    model.refresh_parents(&mut self.doc);
                }
                self.changed();
            }
            Some(Delivered::Signals(trace, results)) => {
                for (signal, result) in results {
                    let signal = Traced::new(trace, signal);
                    let result = result.map_err(|e| e.to_string());
                    for panel in self.panels.iter_mut() {
                        if let Some(w) = panel.kind.waves_mut() {
                            w.finish_signal(signal, result.clone().map_err(anyhow::Error::msg));
                        }
                        if let Some(table) = panel.kind.table_mut() {
                            table.finish_signal(signal, result.clone());
                            table.refresh(&self.doc);
                        }
                    }
                }
                self.sync_analog_summaries();
                self.changed();
            }
            Some(Delivered::Opened {
                trace,
                result: Ok(_),
                refined,
            }) => {
                let first = self.doc.traces().loaded().count() == 1;
                if first {
                    self.on_session_changed();
                }
                if trace.is_a() {
                    if !first {
                        self.trace_visible(trace, refined, Instant::now());
                    }
                    self.session_ready_for_workspace();
                } else {
                    self.trace_joined(trace, refined, Instant::now());
                    self.workspace_trace_joined(trace);
                }
            }
            Some(Delivered::Opened {
                trace,
                result: Err(error),
                ..
            }) => {
                if self.doc.is_loaded() {
                    self.events.push(Event::Notice(format!(
                        "Trace {trace} was not added: {error}"
                    )));
                }
                if trace.is_a() {
                    self.workspace.loading = false;
                }
                self.workspace_trace_failed(trace, error);
                self.changed();
            }
            None => {}
        }
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub(crate) fn changed(&mut self) {
        if !self.events.contains(&Event::Changed) {
            self.events.push(Event::Changed);
        }
    }

    pub(crate) fn layout_changed(&mut self) {
        self.events
            .retain(|e| !matches!(e, Event::LayoutChanged { .. }));
        self.events.push(Event::LayoutChanged {
            revision: self.panels.revision(),
        });
        self.changed();
    }

    pub(crate) fn adopt_panels(&mut self, ids: &[PanelId]) -> Vec<String> {
        if ids.is_empty() {
            return Vec::new();
        }
        let resident = self.resident_histories();
        let mut notices = Vec::new();
        for &id in ids {
            if let Some(panel) = self.panels.get_mut(id)
                && let Err(error) = panel.kind.adopt(&mut self.doc, &resident)
            {
                notices.push(format!("{}: {error:#}", panel.title()));
            }
        }
        notices
    }

    pub(crate) fn retire_panels(&mut self) {
        for panel in self.panels.iter_mut() {
            panel.kind.retire(&mut self.doc);
        }
    }

    fn fresh_waves(&self) -> PanelKind {
        let mut waves = crate::wave::model::WaveModel::new();
        waves.reset(self.doc.is_loaded().then(|| self.doc.limits()));
        waves.nav.link = self.settings.resolved().link_by_default();
        PanelKind::Waves(Box::new(waves))
    }

    fn replace_panel(
        &mut self,
        id: PanelId,
        kind: PanelKind,
        label: Option<String>,
    ) -> Result<PanelId, String> {
        match self.restructure(label, |panels| panels.replace(id, kind)) {
            Ok(new) => Ok(new),
            Err(error) => {
                self.events.push(Event::Notice(error.to_string()));
                self.changed();
                Err(error.to_string())
            }
        }
    }

    fn waves_target(&mut self) -> Option<PanelId> {
        let focused = self.panels.focused_id();
        let kind = &self.panels.focused().kind;
        if kind.waves().is_some() {
            self.panel_command(PanelsCommand::Focus(focused));
            return Some(focused);
        }
        if kind.is_start()
            && self.panels.layout().placement(focused) == Some(crate::panels::Placement::Center)
        {
            let waves = self.fresh_waves();
            return self.replace_panel(focused, waves, None).ok();
        }
        if let Some(id) = self.panels.first_waves() {
            self.panel_command(PanelsCommand::Focus(id));
            return Some(id);
        }
        if let Some(start) = self.panels.first_start() {
            let waves = self.fresh_waves();
            let id = self.replace_panel(start, waves, None).ok()?;
            self.panel_command(PanelsCommand::Focus(id));
            return Some(id);
        }
        let waves = self.fresh_waves();
        match self.restructure(None, |panels| panels.open_center(waves, focused, None)) {
            Ok(id) => Some(id),
            Err(error) => {
                self.events.push(Event::Notice(error.to_string()));
                self.changed();
                None
            }
        }
    }

    fn panel_command(&mut self, command: PanelsCommand) {
        let revision = self.panels.revision();
        let old_focus = self.panels.focused_id();
        let start =
            |panels: &Panels, id: PanelId| panels.get(id).is_some_and(|p| p.kind.is_start());
        let result = match command {
            PanelsCommand::MoveToNewWindow(panel) => self
                .restructure(Some("Move panel to new window".into()), |panels| {
                    Ok((panels.move_to_new_window(panel)?, Vec::new()))
                })
                .map(|_| ()),
            PanelsCommand::ReturnWindow(window) => self
                .restructure(Some("Return window to main window".into()), |panels| {
                    Ok((panels.return_window(window)?, Vec::new()))
                })
                .map(|_| ()),
            PanelsCommand::Split { panel, .. } | PanelsCommand::NewTab { group_of: panel }
                if start(&self.panels, panel) =>
            {
                let waves = self.fresh_waves();
                drop(self.replace_panel(panel, waves, Some("New waves panel".into())));
                return;
            }
            PanelsCommand::Split { panel, axis } => {
                let label = self
                    .panels
                    .get(panel)
                    .map(|p| format!("Split {}", p.title()));
                self.restructure(label, |panels| {
                    Ok((panels.create(panel, Some(axis))?, Vec::new()))
                })
                .map(|_| ())
            }
            PanelsCommand::NewTab { group_of } => self
                .restructure(Some("New panel".into()), |panels| {
                    Ok((panels.create(group_of, None)?, Vec::new()))
                })
                .map(|_| ()),
            PanelsCommand::Close(id) => {
                let label = self.panels.get(id).map(|p| format!("Close {}", p.title()));
                self.restructure(label, |panels| Ok(((), panels.close(id)?)))
            }
            PanelsCommand::CloseOthers(id) => {
                let others = self
                    .panels
                    .iter()
                    .filter(|p| p.id != id && p.kind.is_content())
                    .count();
                let label = format!("Close {}", crate::history::count(others, "panel", "panels"));
                self.restructure(Some(label), |panels| Ok(((), panels.close_others(id)?)))
            }
            PanelsCommand::Move { panel, placement } => self
                .restructure(Some("Move panel".into()), |panels| {
                    Ok((panels.move_panel(panel, placement)?, Vec::new()))
                })
                .map(|_| ()),
            PanelsCommand::ToggleRightDock => self.panels.toggle_right_dock().map(|_| ()),
            PanelsCommand::Focus(id) => self.panels.focus(id).map(|_| ()),
            PanelsCommand::FocusNext => self.panels.focus_next(false).map(|_| ()),
            PanelsCommand::FocusPrev => self.panels.focus_next(true).map(|_| ()),
            PanelsCommand::FocusIndex(ix) => self.panels.focus_index(ix).map(|_| ()),
            PanelsCommand::Rename(id, title) => self.panels.rename(id, title).map(|_| ()),
            PanelsCommand::SetLayout {
                layout,
                from_revision,
            } => self
                .restructure(Some("Rearrange panels".into()), |panels| {
                    Ok((panels.set_layout(layout, from_revision)?, Vec::new()))
                })
                .map(|_| ()),
            PanelsCommand::ToggleLink { panel, dim } => {
                self.panels.toggle_link(panel, &self.doc, dim)
            }
        };
        if old_focus != self.panels.focused_id()
            && let Some(w) = self.panels.waves_mut(old_focus)
        {
            w.menu_dismiss();
        }
        if let Err(e) = result {
            self.events.push(Event::Notice(e.to_string()));
            self.layout_changed();
        } else if revision != self.panels.revision() {
            self.layout_changed();
        } else {
            self.changed();
        }
    }

    pub fn handle_if_current(&mut self, generation: u64, command: Command) -> bool {
        if generation != self.doc.generation() {
            return false;
        }
        self.handle(command);
        true
    }

    pub fn handle(&mut self, command: Command) {
        self.handle_at(command, Instant::now());
    }

    pub fn handle_at(&mut self, command: Command, now: Instant) {
        let before = crate::workspace::Stamp::capture(self, &command);
        let tracked = before.as_ref().map(|_| command.clone());
        let selection = self.doc.selection();
        let pointer = matches!(command, Command::Pointer(..));
        let press = matches!(
            command,
            Command::Pointer(_, PointerEvent::Down { .. } | PointerEvent::Up)
        );
        self.begin_step(now);
        let key_or_click = matches!(
            command,
            Command::Action(_) | Command::Pointer(_, PointerEvent::Down { .. })
        );
        if key_or_click && self.announcement.take().is_some() {
            self.changed();
        }
        if matches!(command, Command::Action(_)) && self.marker_edit.take().is_some() {
            self.changed();
        }
        self.cancel_pipeline_opening(&command);
        self.dispatch(command, now);
        for waves in self.panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
            if let Some(notice) = waves.bus_notice.take() {
                self.events.push(Event::Notice(notice));
            }
        }
        self.panels.sync_timeline_dividers();
        self.activity.prune(self.doc.traces());
        if !pointer {
            self.sync_lane_tracks();
            self.sync_analog_summaries();
        } else if press {
            self.sync_group_summaries();
        }
        if self.doc.selection() != selection {
            self.sync_selection();
        }
        self.end_step(now);
        if let Some(command) = tracked
            && before != crate::workspace::Stamp::capture(self, &command)
        {
            self.workspace.scheduler.changed(now);
        }
    }

    fn dispatch(&mut self, command: Command, now: Instant) {
        match command {
            Command::Notice(message) => {
                self.events.push(Event::Notice(message));
                self.changed();
            }
            Command::Open(spec) => self.open(spec),
            Command::AddTrace(spec) => self.add_trace(spec, None),
            Command::RequestOpenDialog => self.events.push(Event::OpenFileDialog),
            Command::RequestAddTraceDialog => self.events.push(Event::AddTraceDialog),
            Command::RemoveTrace(trace) => self.remove_trace(trace),
            Command::RenameTrace(trace, name) => self.rename_trace(trace, name.as_deref()),
            Command::RevealTrace(trace) => self.reveal_trace(trace),
            Command::RequestOpenWorkspace | Command::RequestSaveWorkspaceAs => {
                if self.workspace.scheduler.enabled() {
                    self.events
                        .push(if matches!(command, Command::RequestOpenWorkspace) {
                            Event::OpenWorkspaceDialog
                        } else {
                            Event::SaveWorkspaceDialog
                        });
                } else {
                    self.report_workspace_error(
                        "Workspace persistence is disabled for this session".into(),
                    );
                }
            }
            Command::SaveWorkspace => self.save_workspace(None),
            Command::RequestQuit => self.request_quit(),
            Command::CloseTrace => self.close_trace(),
            Command::Undo => self.undo(now),
            Command::Redo => self.redo(now),
            Command::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                self.changed();
            }
            Command::Action(a) => self.action(a, now),
            Command::Panels(command) => self.panel_command(command),
            Command::Pointer(id, ev) => {
                if matches!(ev, PointerEvent::Down { .. }) && self.panels.get(id).is_some() {
                    self.panel_command(PanelsCommand::Focus(id));
                    if self.transient_menu.take().is_some() {
                        self.changed();
                    }
                    if self.lane_press(id, ev) {
                        self.changed();
                        return;
                    }
                    if self.pipeline_label_press(id, ev) {
                        self.changed();
                        return;
                    }
                }
                if let PointerEvent::Move { position } = ev
                    && self.panels.resize_timeline(id, position)
                {
                    self.changed();
                    return;
                }
                if let Some(panel) = self.panels.get_mut(id)
                    && panel.pointer(&mut self.doc, ev, now)
                {
                    self.changed();
                }
            }
            Command::AddVars(vars) => self.add_vars(&vars),
            Command::AddToWaves(members) => self.add_to_waves(&members),
            Command::AddClockRulers(members) => self.add_clock_rulers(&members),
            Command::ActivateMembers(members) => self.activate_members(&members),
            Command::OpenPipeline { track } => self.open_pipeline(track),
            Command::OpenTable { selected, clicked } => self.open_table(&selected, clicked),
            Command::OpenTableFromPanel { panel, row } => self.open_table_from_panel(panel, row),
            Command::PipelineMeasured(panel, size) => {
                if let Some(pipeline) = self.panels.pipeline_mut(panel) {
                    pipeline.measure_viewport(size);
                }
                self.initialize_pipelines();
            }
            Command::PipelineActivity(panel, command) => {
                if let Some(pipeline) = self.panels.pipeline_mut(panel) {
                    pipeline.activity_command(&self.doc, command);
                    self.changed();
                }
            }
            Command::Table(panel, command) => {
                let Self { panels, doc, .. } = self;
                if let Some(table) = panels.get_mut(panel).and_then(|p| p.kind.table_mut())
                    && table.command(doc, panel, command, now)
                {
                    self.changed();
                }
            }
            Command::SelectTransaction {
                panel,
                track,
                id,
                cursor,
            } => self.select_transaction(panel, track, id, cursor),
            Command::ShowTransaction { from } => self.show_transaction(from),
            Command::ShowAttributeColumn { from, key } => {
                self.show_attribute_column(from, key, now);
            }
            Command::PanToTransactionStart { panel, row } => {
                if let Some(pipeline) = self.panels.pipeline_mut(panel)
                    && pipeline.pan_to_row_start(&mut self.doc, row, now)
                {
                    self.changed();
                }
            }
            Command::Clocks(command) => self.clock_command(command),
            Command::Transaction(panel, command) => {
                let Self { panels, doc, .. } = self;
                if let Some(model) = panels.transaction_mut(panel)
                    && model.command(doc, panel, command, now)
                {
                    self.changed();
                }
            }
            Command::RevealTransaction { panel } => self.reveal_transaction(panel, now),
            Command::SetSearchEverywhere(search) => {
                self.variables.search_everywhere = search;
                self.variables.rebuild(self.doc.traces());
                self.events.push(Event::FocusFilter);
                self.changed();
            }
            Command::SelectScope(node) => {
                if self.scopes.select(node) || self.variables.search_everywhere {
                    self.variables
                        .set_scope(self.doc.traces(), node.traced_scope());
                }
                self.changed();
            }
            Command::ToggleScope(node) => {
                self.scopes.toggle(self.doc.traces(), node);
                self.changed();
            }
            Command::ExpandAllScopes(expand) => {
                self.scopes.set_all(self.doc.traces(), expand);
                self.changed();
            }
            Command::ScopesKey(key) => {
                if !self.doc.is_loaded() {
                    return;
                }
                let out = self.scopes.key(self.doc.traces(), &key);
                if out.changed {
                    let scope = self.scopes.selected_scope();
                    self.variables.set_scope(self.doc.traces(), scope);
                }
                if let Some(row) = out.reveal {
                    self.events.push(Event::RevealScopeRow(row));
                }
                if let Some(member) = out.activate {
                    self.activate_members(&[member]);
                }
                self.changed();
            }
            Command::SetFilter(text) => {
                self.variables.set_filter(self.doc.traces(), &text);
                self.changed();
            }
            Command::SelectVar { ix, modifiers } => {
                self.variables.select(ix, modifiers);
                self.changed();
            }
            Command::AddAllVars => {
                let vars = self.variables.listed_vars();
                self.add_vars(&vars);
            }
            Command::AddSelectedOrAllVars => {
                let vars = self.variables.selected_or_all();
                self.activate_members(&vars);
            }
            Command::VariablesKey(key, modifiers) => {
                if key == Key::Escape && self.variables.selected.is_empty() {
                    self.variables.set_filter(self.doc.traces(), "");
                    self.changed();
                }
                let out = self.variables.key(&key, modifiers);
                if let Some(members) = out.activate {
                    self.activate_members(&members);
                }
                if let Some(row) = out.reveal {
                    self.events.push(Event::RevealVarRow(row));
                }
                if out.focus_filter {
                    self.events.push(Event::FocusFilter);
                }
                if out.changed {
                    self.changed();
                }
            }
            Command::AddScopeAsGroup { scope, recursive } => self.add_scope_group(scope, recursive),
            Command::CommitText(target, text) => self.commit_text(target, text),
            Command::RenameMarker(panel, id) => self.open_marker_name(panel, id, now),
            Command::ZoomToSpan(panel, from, to) => self.zoom_to_span(panel, from, to, now),
            Command::OpenSignalMenu(panel) => {
                if let Some(waves) = self.panels.waves_mut(panel) {
                    waves.open_selected_signal_menu(&self.doc);
                    if waves.menu.is_some() {
                        self.changed();
                    }
                }
            }
            Command::Lane(verb) => {
                let panel = self.panels.focused_id();
                self.lane_verb(panel, verb, now);
                self.changed();
            }
            Command::MenuSelect(panel, MenuAction::Lane(verb)) => {
                let valid = self.transient_menu.as_ref().is_some_and(|(owner, menu)| {
                    *owner == panel
                        && menu.kind == WaveMenuKind::Lane
                        && menu
                            .items()
                            .any(|item| item.action == MenuAction::Lane(verb))
                });
                if valid {
                    self.transient_menu = None;
                    self.lane_verb(panel, verb, now);
                    self.changed();
                }
            }
            Command::MenuSelect(panel, MenuAction::PipelineStart(row)) => {
                let valid = self.transient_menu.as_ref().is_some_and(|(owner, menu)| {
                    *owner == panel
                        && menu.kind == WaveMenuKind::PipelineLabel
                        && menu
                            .items()
                            .any(|item| item.action == MenuAction::PipelineStart(row))
                });
                if valid {
                    self.transient_menu = None;
                    if let Some(pipeline) = self.panels.pipeline_mut(panel) {
                        pipeline.pan_to_row_start(&mut self.doc, row, now);
                    }
                    self.changed();
                }
            }
            Command::MenuSelect(panel, action) => {
                if matches!(action, MenuAction::OpenTable) {
                    let row = self
                        .panels
                        .waves(panel)
                        .and_then(|waves| waves.menu.as_ref())
                        .filter(|menu| menu.kind == WaveMenuKind::Signal)
                        .map(|menu| menu.row);
                    let Some(row) = row else { return };
                    self.open_table_from_panel(panel, Some(row));
                    if let Some(waves) = self.panels.waves_mut(panel) {
                        waves.menu_dismiss();
                    }
                    self.changed();
                    return;
                }
                if let Some(clipboard) = match action {
                    MenuAction::CopySignals => Some(Action::CopySignals),
                    MenuAction::CutSignals => Some(Action::CutSignals),
                    MenuAction::PasteSignals => Some(Action::PasteSignals),
                    _ => None,
                } {
                    let Some(waves) = self.panels.waves_mut(panel).filter(|w| {
                        w.menu
                            .as_ref()
                            .is_some_and(|m| m.kind == WaveMenuKind::Signal)
                    }) else {
                        return;
                    };
                    waves.menu_dismiss();
                    match clipboard {
                        Action::CopySignals => waves.copy_selected(&mut self.doc),
                        Action::CutSignals => waves.cut_selected(&mut self.doc),
                        _ => self.paste_signals(panel),
                    }
                    self.changed();
                    return;
                }
                if matches!(action, MenuAction::RemoveSignals) {
                    if let Some(waves) = self.panels.waves_mut(panel)
                        && waves
                            .menu
                            .as_ref()
                            .is_some_and(|menu| menu.kind == WaveMenuKind::Signal)
                    {
                        waves.menu_dismiss();
                        waves.remove_selected();
                        self.changed();
                    }
                    return;
                }
                if action == MenuAction::RetryLoad {
                    self.projections.retry_failed();
                }
                let retry = self
                    .panels
                    .waves_mut(panel)
                    .and_then(|w| w.menu_select(&self.doc, &action));
                if let Some(signal) = retry
                    && self.doc.request_signal(signal)
                {
                    for waves in self.panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
                        waves.clear_error(signal);
                    }
                }
                self.changed();
            }
            Command::MenuDismiss(panel) => {
                if let Some(w) = self.panels.waves_mut(panel) {
                    w.menu_dismiss();
                }
                if self
                    .transient_menu
                    .as_ref()
                    .is_some_and(|(p, _)| *p == panel)
                {
                    self.transient_menu = None;
                }
                self.changed();
            }
            Command::SetSidebarWidth(w) => {
                self.sidebar_width = w.clamp(160.0, 640.0);
                self.changed();
            }
            Command::SetScopesFraction(f) => {
                self.scopes_fraction = f.clamp(SIDEBAR_FRACTION_MIN, 1.0 - SIDEBAR_FRACTION_MIN);
                self.changed();
            }
            Command::ChromeDragStart(d) => {
                self.drag = Some(d);
                self.changed();
            }
            Command::ChromeDragEnd => {
                self.drag = None;
                self.changed();
            }
            Command::Settings(command) => self.settings_command(command, now),
            Command::BuildActivity(trace) => {
                self.activity.start_build(trace);
                self.changed();
            }
            Command::DismissActivity(trace) => {
                self.activity.dismiss_build(trace);
                self.changed();
            }
            Command::CancelActivity(trace) => {
                self.activity.cancel_build(trace);
                self.changed();
            }
            Command::Recent(command) => self.recent_command(command),
        }
    }

    pub(crate) fn sync_selection(&mut self) {
        let Self { panels, doc, .. } = self;
        for (_, model) in panels.transactions_mut() {
            model.follow_selection(doc);
        }
        let Self { panels, doc, .. } = self;
        for table in panels.iter_mut().filter_map(|p| p.kind.table_mut()) {
            table.follow_selection(doc);
        }
        self.changed();
    }

    fn select_transaction(
        &mut self,
        panel: PanelId,
        track: Traced<TrackRef>,
        id: TransactionRef,
        cursor: Option<u64>,
    ) {
        let Self { panels, doc, .. } = self;
        doc.select(Some(crate::document::TxSelection {
            track,
            id,
            origin: panel,
        }));
        if let Some(time) = cursor
            && let Some(nav) = panels.get_mut(panel).and_then(|p| p.kind.nav_mut())
        {
            nav.set_cursor(doc, Some(time));
        }
        self.changed();
    }

    fn show_transaction(&mut self, from: PanelId) {
        if self.panels.get(from).is_none() {
            return;
        }
        if self.doc.selection().is_none() {
            self.events
                .push(Event::Notice("Select a record first.".into()));
            self.changed();
            return;
        }
        let model = TransactionModel::new(
            self.table_memory_budget(),
            self.settings.resolved().transaction.detail_items,
        );
        let label = Some("Open transaction panel".to_owned());
        match self.restructure(label, |panels| {
            Ok((panels.ensure_transaction(model)?, Vec::new()))
        }) {
            Ok(_) => self.sync_selection(),
            Err(error) => {
                self.events.push(Event::Notice(error.to_string()));
                self.changed();
            }
        }
    }

    fn reveal_transaction(&mut self, panel: PanelId, now: Instant) {
        let Some(shown) = self
            .panels
            .transaction(panel)
            .and_then(|model| model.shown().cloned())
        else {
            return;
        };
        let Some(track) = shown.track.track() else {
            return;
        };
        let target = self.panels.pipeline_showing(&self.doc, track, shown.id);
        let Some(target) = target else {
            self.open_pipeline(track);
            return;
        };
        self.panel_command(PanelsCommand::Focus(target));
        let Self { panels, doc, .. } = self;
        if let Some(pipeline) = panels.pipeline_mut(target) {
            pipeline.reveal_record(doc, track.item, shown.id, now);
        }
        self.changed();
    }

    fn activate_members(&mut self, members: &[Traced<Member>]) {
        let mut vars = Vec::new();
        let mut tracks = Vec::new();
        let mut logs = Vec::new();
        for &member in members {
            let Some(h) = self.doc.hierarchy(member.trace) else {
                continue;
            };
            if let Member::Var(id) = member.item {
                if id < h.var_count() {
                    vars.push(member.with(id));
                }
            } else if let Some(track) = h.member_track(member.item) {
                let track = member.with(track);
                if h.is_log(member.item) {
                    if !logs.contains(&member) {
                        logs.push(member);
                    }
                } else if !tracks.contains(&track) {
                    tracks.push(track);
                }
            }
        }
        if !vars.is_empty() {
            self.add_vars(&vars);
        }
        self.variables.notice = None;
        for track in tracks {
            self.open_pipeline(track);
        }
        for member in logs {
            let track = self
                .doc
                .hierarchy(member.trace)
                .and_then(|h| h.member_track(member.item));
            let existing = self
                .panels
                .iter()
                .find(|panel| {
                    panel.kind.table().is_some_and(|table| {
                        table.source.generator_track() == track.map(|track| member.with(track))
                    })
                })
                .map(|panel| panel.id);
            if let Some(panel) = existing {
                self.panel_command(PanelsCommand::Focus(panel));
            } else {
                self.open_table(&[member], None);
            }
        }
        self.changed();
    }

    fn resident_histories(&self) -> crate::wave::model::Resident {
        self.panels
            .iter()
            .filter_map(|panel| panel.kind.waves())
            .flat_map(|waves| waves.items().iter().map(|e| &e.row))
            .filter_map(WaveRow::signal)
            .filter_map(|row| Some((row.source.signal()?, row.raw_history()?.clone())))
            .chain(
                self.panels
                    .iter()
                    .filter_map(|panel| panel.kind.table())
                    .flat_map(super::table::model::TableModel::histories),
            )
            .collect()
    }

    fn account_local_session(&self, session: Arc<dyn Session>) -> anyhow::Result<Arc<dyn Session>> {
        volna_trace::session::account_local_session(session, self.table_budget.clone())
    }

    #[must_use]
    pub fn memory_menu(&self) -> MemoryMenu {
        let memory = &self.settings.resolved().memory;
        let locked = ["memory.budgetMiB", "memory.objectMiB"]
            .iter()
            .any(|id| self.settings.is_overridden(id));
        MemoryMenu {
            budget_title: format!("Memory budget: {}", format_mib(memory.budget_mib)),
            budget: memory_choices("memory.budgetMiB", &BUDGET_PRESETS_MIB, memory.budget_mib),
            object_title: format!("Object size limit: {}", format_mib(memory.object_mib)),
            object: memory_choices("memory.objectMiB", &OBJECT_PRESETS_MIB, memory.object_mib),
            object_note: self
                .doc
                .traces()
                .loaded()
                .any(|(_, s)| s.remote_id().is_some())
                .then_some("A remote trace applies it when reopened"),
            locked: locked.then_some("Set in VS Code settings (volna.memory.*)"),
        }
    }

    pub(crate) fn apply_memory_limits(&self) {
        let Ok((limit, object_limit)) = self.settings.resolved().limits().bytes() else {
            return;
        };
        for budget in [self.table_budget.clone(), self.table_memory_budget()] {
            budget.set_limit(limit);
            budget.set_object_limit(object_limit);
        }
    }

    pub(crate) fn table_memory_budget(&self) -> volna_trace::remote::memory::MemoryBudget {
        self.doc
            .traces()
            .first_loaded()
            .and_then(|(_, session)| session.memory_budget())
            .unwrap_or_else(|| self.table_budget.clone())
    }

    fn open_table(&mut self, selected: &[Traced<Member>], clicked: Option<Traced<Member>>) {
        let clicked_only;
        let members = match clicked {
            Some(member) if !selected.contains(&member) => {
                clicked_only = [member];
                &clicked_only[..]
            }
            _ => selected,
        };
        let vars: Vec<Traced<VarId>> = members
            .iter()
            .filter_map(|m| Some(m.with(m.item.var()?)))
            .collect();
        let source = if !members.is_empty() && vars.len() == members.len() {
            crate::table::TableSource::signals(&self.doc, &vars)
        } else if let [member] = members {
            self.generator_table(*member)
        } else {
            Err(anyhow::anyhow!("Choose one generator, or only signals."))
        };
        match source {
            Ok(source) => self.open_table_source(source, self.panels.focused_id()),
            Err(error) => {
                self.events.push(Event::Notice(error.to_string()));
                self.changed();
            }
        }
    }

    fn generator_table(&self, member: Traced<Member>) -> anyhow::Result<crate::table::TableSource> {
        let session = self
            .doc
            .session(member.trace)
            .ok_or_else(|| anyhow::anyhow!("trace {} is not open", member.trace))?;
        let track = session
            .hierarchy()
            .member_track(member.item)
            .ok_or_else(|| anyhow::anyhow!("Choose one generator, or only signals."))?;
        self.track_table(member.with(track))
    }

    fn track_table(&self, track: Traced<TrackRef>) -> anyhow::Result<crate::table::TableSource> {
        let session = self
            .doc
            .session(track.trace)
            .ok_or_else(|| anyhow::anyhow!("trace {} is not open", track.trace))?;
        crate::table::TableSource::generator(session.hierarchy(), session.tracks(), track)
    }

    fn open_table_from_panel(&mut self, panel: PanelId, row: Option<usize>) {
        if !self.doc.is_loaded() {
            return;
        }
        let source = if let Some(waves) = self.panels.get(panel).and_then(|p| p.kind.waves()) {
            let rows = match row {
                Some(row) if !waves.selected.contains(&row) => {
                    std::collections::BTreeSet::from([row])
                }
                _ => waves.selected.clone(),
            };
            let rows = crate::wave::tree::selected_leaves(waves.items(), &rows);
            let vars = rows
                .iter()
                .filter_map(|&index| waves.signal(index)?.source.var())
                .collect::<Vec<_>>();
            let lane = rows
                .iter()
                .find_map(|&index| waves.items().get(index)?.lane_track());
            match lane {
                Some(track) if vars.is_empty() => self.track_table(track),
                _ => crate::table::TableSource::signals(&self.doc, &vars),
            }
        } else if let Some(track) = self
            .panels
            .get(panel)
            .and_then(|p| p.kind.pipeline())
            .and_then(|pipeline| pipeline.track.track())
        {
            self.track_table(track)
        } else {
            return;
        };
        match source {
            Ok(source) => self.open_table_source(source, panel),
            Err(error) => {
                self.events.push(Event::Notice(error.to_string()));
                self.changed();
            }
        }
    }

    fn show_attribute_column(&mut self, from: PanelId, key: String, now: Instant) {
        let Some(track) = self
            .panels
            .transaction(from)
            .and_then(|model| model.shown())
            .and_then(|shown| shown.track.track())
        else {
            return;
        };
        let existing = self
            .panels
            .iter()
            .find(|panel| {
                panel
                    .kind
                    .table()
                    .is_some_and(|t| t.source.generator_track() == Some(track))
            })
            .map(|p| p.id);
        let panel = if let Some(panel) = existing {
            drop(self.panels.focus(panel));
            panel
        } else {
            let Ok(source) = self.track_table(track) else {
                return;
            };
            self.open_table_source(source, self.panels.focused_id());
            let Some(panel) = self
                .panels
                .iter()
                .find(|panel| {
                    panel
                        .kind
                        .table()
                        .is_some_and(|t| t.source.generator_track() == Some(track))
                })
                .map(|p| p.id)
            else {
                return;
            };
            panel
        };
        let column = if key == "vtr.label" {
            crate::table::columns::TransactionColumn::Label
        } else {
            crate::table::columns::TransactionColumn::Attribute(key)
        };
        if let Some(table) = self.panels.get_mut(panel).and_then(|p| p.kind.table_mut()) {
            table.command(
                &mut self.doc,
                panel,
                crate::table::TableCommand::ShowTransactionColumn(column),
                now,
            );
        }
        self.changed();
    }

    fn open_table_source(&mut self, source: crate::table::TableSource, beside: PanelId) {
        let mut table = crate::table::TableModel::for_document(
            source,
            self.settings.resolved().link_by_default(),
            self.table_memory_budget(),
            &self.doc,
        );
        table.nav.reset(Some(self.doc.limits()));
        let kind = PanelKind::Table(Box::new(table));
        let label = Some("Open table".to_owned());
        match self.restructure(label, |panels| {
            panels.open_center(kind, beside, Some(crate::panels::Axis::Vertical))
        }) {
            Ok(_) => {}
            Err(error) => {
                self.events.push(Event::Notice(error.to_string()));
                self.changed();
            }
        }
    }

    fn open_pipeline(&mut self, track: Traced<TrackRef>) {
        let Some(session) = self.doc.session(track.trace).cloned() else {
            return;
        };
        if !session.capabilities().transactions {
            self.events
                .push(Event::Notice("This trace records no transactions.".into()));
            return;
        }
        let Some(declaration) = session.tracks().iter().find(|t| t.id == track.item) else {
            self.events.push(Event::Notice(format!(
                "Unknown transaction track {}",
                track.item.0
            )));
            return;
        };
        let existing = self.panels.pipeline_for_track(track);
        let model = PipelineModel::open(
            TrackSource::Resolved {
                track,
                path: declaration.path.clone(),
            },
            self.settings.resolved().link_by_default(),
            &self.doc,
        );
        let inspector = TransactionModel::new(
            self.table_memory_budget(),
            self.settings.resolved().transaction.detail_items,
        );
        let focused = self.panels.focused_id();
        let label = Some(format!("Open pipeline {}", declaration.path.join(".")));
        let beside = self.panels.center_target(focused);
        match self.restructure(label, |panels| {
            let replace_start = existing.is_none()
                && beside
                    .is_some_and(|id| panels.get(id).is_some_and(|panel| panel.kind.is_start()));
            let added = usize::from(existing.is_none() && !replace_start)
                + usize::from(panels.first_free_transaction().is_none());
            anyhow::ensure!(
                panels.len() + added <= crate::panels::MAX_PANELS,
                "panel limit reached"
            );
            let (pipeline, removed) = if let Some(id) = existing {
                (id, Vec::new())
            } else {
                panels.open_center(
                    PanelKind::Pipeline(Box::new(model)),
                    focused,
                    Some(crate::panels::Axis::Vertical),
                )?
            };
            panels.ensure_transaction(inspector)?;
            // Selecting records remains the pipeline's keyboard interaction.
            panels.focus(pipeline)?;
            Ok((pipeline, removed))
        }) {
            Ok(_) => {
                if let Some(w) = self.panels.waves_mut(focused) {
                    w.menu_dismiss();
                }
                self.sync_selection();
            }
            Err(error) => {
                self.events.push(Event::Notice(error.to_string()));
                self.changed();
            }
        }
    }

    fn add_scope_group(&mut self, scope: Traced<ScopeId>, recursive: bool) {
        let Some(target) = self.waves_target() else {
            return;
        };
        let loaded = self.resident_histories();
        if let Some(w) = self.panels.waves_mut(target)
            && w.add_scope_group(&mut self.doc, scope, recursive, loaded)
        {
            self.changed();
        }
    }

    fn add_vars(&mut self, vars: &[Traced<VarId>]) {
        if vars.is_empty() {
            return;
        }
        let Some(target) = self.waves_target() else {
            return;
        };
        let loaded = self.resident_histories();
        if let Some(w) = self.panels.waves_mut(target) {
            w.add_vars(&mut self.doc, vars, loaded);
        }
        self.changed();
    }

    fn add_to_waves(&mut self, members: &[Traced<Member>]) {
        let vars: Vec<Traced<VarId>> = members
            .iter()
            .filter_map(|m| Some(m.with(m.item.var()?)))
            .collect();
        let mut tracks = Vec::new();
        let mut clocks: Vec<ClockKey> = Vec::new();
        for &member in members {
            let Some(h) = self.doc.hierarchy(member.trace) else {
                continue;
            };
            let Some(track) = h.member_track(member.item) else {
                continue;
            };
            let track = member.with(track);
            if let Some(key) = self.member_clock(member) {
                if !clocks.contains(&key) {
                    clocks.push(key);
                }
            } else if matches!(member.item, Member::Generator(_))
                && !h.is_log(member.item)
                && !tracks.contains(&track)
            {
                tracks.push(track);
            }
        }
        self.add_vars(&vars);
        if tracks.is_empty() && clocks.is_empty() {
            return;
        }
        let Some(target) = self.waves_target() else {
            return;
        };
        let Self { panels, doc, .. } = self;
        if let Some(w) = panels.waves_mut(target) {
            w.add_lanes(doc, &tracks);
            w.add_clocks(&clocks);
        }
        self.sync_lane_tracks();
        self.changed();
    }

    #[must_use]
    pub fn member_clock(&self, member: Traced<Member>) -> Option<ClockKey> {
        let session = self.doc.session(member.trace)?;
        let track = session.hierarchy().member_track(member.item)?;
        let clock = self
            .doc
            .clocks
            .of_track(member.with(track), session.tracks())?;
        Some(clock.key())
    }

    fn add_clock_rulers(&mut self, members: &[Traced<Member>]) {
        let mut keys: Vec<ClockKey> = Vec::new();
        for &member in members {
            if let Some(key) = self.member_clock(member)
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        if keys.is_empty() {
            return;
        }
        let Some(target) = self.waves_target() else {
            return;
        };
        let Self { panels, doc, .. } = self;
        if let Some(w) = panels.waves_mut(target) {
            for key in &keys {
                w.nav.show_ruler(&doc.clocks, key);
            }
        }
        self.changed();
    }

    pub(crate) fn sync_analog_summaries(&mut self) {
        let mut wanted = std::collections::HashMap::new();
        for waves in self.panels.iter().filter_map(|panel| panel.kind.waves()) {
            let mut rows: std::collections::BTreeSet<usize> = waves
                .items()
                .iter()
                .enumerate()
                .filter_map(|(i, e)| e.signal().filter(|s| s.analog.is_some()).map(|_| i))
                .collect();
            for &i in waves.visible().iter() {
                if waves.is_overlaid(i as usize) {
                    rows.extend(
                        waves
                            .numeric_members(i as usize)
                            .into_iter()
                            .filter(|&j| !waves.signal(j).unwrap().overlay_hidden),
                    );
                }
            }
            for i in rows {
                let s = waves.signal(i).unwrap();
                let (Some(kind), Some(history), Some(signal)) =
                    (s.numeric_kind(), s.history.as_ref(), s.source.signal())
                else {
                    continue;
                };
                if history.len() >= crate::wave::analog::SUMMARY_MIN_CHANGES {
                    wanted.insert(
                        (signal, kind, crate::wave::analog::history_identity(history)),
                        history.clone(),
                    );
                }
            }
        }
        let budget = self.table_memory_budget();
        self.doc.sync_summaries(wanted, &budget);
        self.sync_group_summaries();
    }

    pub(crate) fn sync_group_summaries(&mut self) {
        let mut wanted = std::collections::HashMap::new();
        for waves in self.panels.iter().filter_map(|panel| panel.kind.waves()) {
            for &i in waves.visible().iter() {
                let i = i as usize;
                if !waves.items()[i].group().is_some_and(|g| g.collapsed) {
                    continue;
                }
                let members = waves.group_histories(i);
                if members.iter().map(|h| h.len()).sum::<usize>() > crate::wave::group::WALK_MAX {
                    wanted.insert(crate::wave::group::key(&members), members);
                }
            }
        }
        let budget = self.table_memory_budget();
        self.doc.sync_group_summaries(wanted, &budget);
        self.sync_stack_totals();
    }

    pub(crate) fn sync_stack_totals(&mut self) {
        let mut wanted = std::collections::HashMap::new();
        let mut integrals = std::collections::HashMap::new();
        for waves in self.panels.iter().filter_map(|panel| panel.kind.waves()) {
            for &i in waves.visible().iter() {
                if !waves.is_stacked(i as usize) {
                    continue;
                }
                let layers = waves.stack_layers(i as usize);
                if crate::wave::stack::changes(&layers) > crate::wave::stack::WALK_MAX {
                    for l in &layers {
                        if l.history.len() >= crate::wave::stack::INTEGRAL_MIN_CHANGES {
                            let identity = crate::wave::analog::history_identity(&l.history);
                            integrals.insert((identity, l.reading), l.history.clone());
                        }
                    }
                }
                if !layers.is_empty() {
                    wanted.insert(crate::wave::stack::key(&layers), layers);
                }
            }
        }
        let budget = self.table_memory_budget();
        self.doc.sync_stack_totals(wanted, &budget);
        self.doc.sync_integrals(integrals, &budget);
    }

    pub(crate) fn sync_lane_tracks(&mut self) {
        let generation = self.doc.generation();
        if self.lane_tracks.0 != generation {
            self.lane_tracks = (generation, HashSet::default());
        }
        let wanted: std::collections::HashSet<Traced<TrackRef>> = self
            .panels
            .iter()
            .filter_map(|panel| panel.kind.waves())
            .flat_map(|waves| waves.items().iter().map(|e| &e.row))
            .filter_map(WaveRow::lane_track)
            .collect();
        let held = &mut self.lane_tracks.1;
        if *held == wanted {
            return;
        }
        for &track in held.difference(&wanted) {
            self.doc.release_track(track);
        }
        held.retain(|track| wanted.contains(track));
        let mut added = false;
        for &track in &wanted {
            if !held.contains(&track) && self.doc.retain_track(track).is_ok() {
                held.insert(track);
                added = true;
            }
        }
        if added {
            let Self { panels, doc, .. } = self;
            for waves in panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
                waves.fit_lanes(doc);
            }
        }
    }

    fn paste_signals(&mut self, panel: PanelId) {
        if self.doc.copied_rows.is_empty() || self.panels.waves(panel).is_none() {
            return;
        }
        let loaded = self.resident_histories();
        if let Some(w) = self.panels.waves_mut(panel) {
            w.paste(&mut self.doc, &loaded);
        }
        self.changed();
    }

    fn clock_command(&mut self, command: ClockCommand) {
        let now = Instant::now();
        let doc = &mut self.doc;
        let kind = &mut self.panels.focused_mut().kind;
        let rulers = kind.shows_rulers();
        let Some(nav) = kind.nav_mut() else {
            return;
        };
        match command {
            ClockCommand::ToggleRuler(key) if rulers => nav.toggle_ruler(&doc.clocks, &key),
            ClockCommand::ToggleRuler(_) => return,
            ClockCommand::Select(key) => nav.select_clock(key),
            ClockCommand::GoToCycle(cycle) => {
                if let Err(message) = nav.go_to_cycle(doc, cycle, now) {
                    self.events.push(Event::Notice(message));
                }
            }
        }
        self.changed();
    }

    #[must_use]
    pub fn clock_choices(&self) -> Option<ClockChoices> {
        let clocks = &self.doc.clocks;
        if clocks.is_empty() {
            return None;
        }
        let kind = &self.panels.focused().kind;
        let nav = kind.nav().filter(|_| kind.shows_rulers())?;
        let rulers = nav.clocks().ruler_keys(clocks);
        Some(ClockChoices {
            clocks: clocks
                .iter()
                .map(|c| {
                    let key = c.key();
                    let shown = rulers.contains(&key);
                    (key, shown)
                })
                .collect(),
            origin: nav.clocks().origin.is_some(),
        })
    }

    fn pipeline_clocks(&self) -> Vec<ClockKey> {
        let mut keys: Vec<ClockKey> = Vec::new();
        for p in self.panels.iter().filter_map(|p| p.kind.pipeline()) {
            if let Some(c) = p.clock(&self.doc)
                && !keys.iter().any(|k| c.is(k))
            {
                keys.push(c.key());
            }
        }
        keys
    }

    fn action(&mut self, action: Action, now: Instant) {
        use crate::panels::Axis;
        use crate::wave::model::LinkDim;
        let panel = self.panels.focused_id();
        let panel_command = match action {
            Action::SplitRight => Some(PanelsCommand::Split {
                panel,
                axis: Axis::Horizontal,
            }),
            Action::SplitDown => Some(PanelsCommand::Split {
                panel,
                axis: Axis::Vertical,
            }),
            Action::NewPanel => Some(PanelsCommand::NewTab { group_of: panel }),
            Action::ClosePanel
                if self.panels.focused().kind.is_start()
                    && self
                        .panels
                        .iter()
                        .all(|p| !p.kind.is_content() || p.id == panel) =>
            {
                self.close_trace();
                return;
            }
            Action::ClosePanel => Some(PanelsCommand::Close(panel)),
            Action::FocusNextPanel => Some(PanelsCommand::FocusNext),
            Action::FocusPrevPanel => Some(PanelsCommand::FocusPrev),
            Action::ToggleViewportLink => Some(PanelsCommand::ToggleLink {
                panel,
                dim: LinkDim::Viewport,
            }),
            Action::ToggleCursorLink => Some(PanelsCommand::ToggleLink {
                panel,
                dim: LinkDim::Cursor,
            }),
            _ => None,
        };
        if let Some(command) = panel_command {
            self.panel_command(command);
            return;
        }
        if action == Action::ClearSelection && self.cancel_gesture() {
            return;
        }
        if action == Action::PasteSignals {
            self.paste_signals(panel);
            return;
        }
        if matches!(
            action,
            Action::AddOrRenameMarker
                | Action::SetMarker(_)
                | Action::RemoveMarkerAtCursor
                | Action::RemoveAllMarkers
        ) {
            let cursor = self.panels.focused().kind.nav().map_or_else(
                || {
                    matches!(action, Action::SetMarker(_))
                        .then_some(self.doc.shared.cursor)
                        .flatten()
                },
                |n| n.cursor(&self.doc),
            );
            let changed = match (action, cursor) {
                (Action::RemoveAllMarkers, _) => self.doc.remove_all_markers(),
                (Action::SetMarker(id), Some(c)) => {
                    if crate::marker::at(self.doc.markers(), c).is_some_and(|m| m.id != id) {
                        self.announce("Another marker is at the cursor".into());
                        false
                    } else {
                        self.doc.set_marker(id, c)
                    }
                }
                (Action::SetMarker(_), None) => {
                    self.announce("No cursor in this panel".into());
                    false
                }
                (Action::AddOrRenameMarker, Some(c)) => {
                    match crate::marker::at(self.doc.markers(), c).map(|m| m.id) {
                        Some(id) => {
                            self.open_marker_name(panel, id, now);
                            false
                        }
                        None => self.doc.add_marker(c).is_some(),
                    }
                }
                (Action::RemoveMarkerAtCursor, Some(c)) => {
                    if let Some(id) = crate::marker::at(self.doc.markers(), c).map(|m| m.id) {
                        self.doc.remove_marker(id)
                    } else {
                        self.announce("No marker at the cursor".into());
                        false
                    }
                }
                _ => false,
            };
            if changed {
                self.changed();
            }
            return;
        }
        if matches!(
            action,
            Action::NextMarker | Action::PrevMarker | Action::GoToMarker(_) | Action::JumpBack
        ) {
            let target = self.panels.focused().kind.nav().map(|_| panel).or_else(|| {
                matches!(action, Action::GoToMarker(_))
                    .then(|| {
                        self.panels
                            .iter()
                            .find_map(|p| p.kind.nav().filter(|n| n.link.cursor).map(|_| p.id))
                    })
                    .flatten()
            });
            let mut shared_nav = crate::nav::NavState::new();
            let nav = match target {
                Some(id) => self.panels.get_mut(id).unwrap().kind.nav_mut().unwrap(),
                None if matches!(action, Action::GoToMarker(_)) => &mut shared_nav,
                None => return,
            };
            let doc = &mut self.doc;
            let walk = match action {
                Action::NextMarker => crate::marker::Walk::Next,
                Action::PrevMarker => crate::marker::Walk::Prev,
                Action::GoToMarker(id) => crate::marker::Walk::To(id),
                _ => {
                    if nav.back(doc, now) {
                        self.changed();
                    } else {
                        self.announce("No jump to return from".into());
                    }
                    return;
                }
            };
            let text = crate::marker::walk(doc, nav, walk, now).unwrap_or_else(|missing| missing);
            self.announce(text);
            return;
        }
        if action == Action::MarkerNavigator {
            self.events.push(Event::OpenMarkerNavigator);
            return;
        }
        if matches!(
            action,
            Action::SetReference | Action::ClearReference | Action::ZoomToMeasurement
        ) {
            self.reference_action(action, now);
            return;
        }
        if matches!(
            action,
            Action::NextCycle | Action::PrevCycle | Action::ToggleCycleOrigin
        ) {
            let doc = &mut self.doc;
            let kind = &mut self.panels.focused_mut().kind;
            let rulers = kind.shows_rulers();
            if let Some(nav) = kind.nav_mut() {
                match action {
                    Action::NextCycle => _ = nav.step_cycle(doc, true, now),
                    Action::PrevCycle => _ = nav.step_cycle(doc, false, now),
                    _ if rulers => nav.toggle_cycle_origin(doc),
                    _ => return,
                }
                self.changed();
            }
            return;
        }
        let doc = &mut self.doc;
        match &mut self.panels.focused_mut().kind {
            PanelKind::Table(table) => match action {
                Action::GoToStart => {
                    table.command(doc, panel, crate::table::TableCommand::First, now);
                }
                Action::GoToEnd => {
                    table.command(doc, panel, crate::table::TableCommand::Last, now);
                }
                Action::MoveSelectionUp => {
                    table.command(doc, panel, crate::table::TableCommand::Previous, now);
                }
                Action::MoveSelectionDown => {
                    table.command(doc, panel, crate::table::TableCommand::Next, now);
                }
                Action::ClearSelection => {
                    table.command(doc, panel, crate::table::TableCommand::ClearSelection, now);
                }
                Action::GoToCursor
                | Action::ZoomIn
                | Action::ZoomOut
                | Action::ZoomFit
                | Action::ZoomToCursor
                | Action::PanPageLeft
                | Action::PanPageRight
                | Action::PanLeft
                | Action::PanRight
                | Action::NextEdge
                | Action::PrevEdge
                | Action::RemoveSelected
                | Action::CopySignals
                | Action::CutSignals
                | Action::PasteSignals
                | Action::SelectAll
                | Action::CycleFormat
                | Action::ToggleAnalog
                | Action::ToggleStack
                | Action::ToggleOverlaid
                | Action::SetTint(_)
                | Action::IncreaseRowHeight
                | Action::DecreaseRowHeight
                | Action::ResetRowHeight
                | Action::GroupSelection
                | Action::Ungroup
                | Action::RenameGroup
                | Action::FoldGroupDeep
                | Action::UnfoldGroupDeep => return,
                Action::SplitRight
                | Action::SplitDown
                | Action::NewPanel
                | Action::ClosePanel
                | Action::FocusNextPanel
                | Action::FocusPrevPanel
                | Action::ToggleViewportLink
                | Action::ToggleCursorLink
                | Action::NextCycle
                | Action::PrevCycle
                | Action::ToggleCycleOrigin
                | Action::AddOrRenameMarker
                | Action::SetMarker(_)
                | Action::RemoveMarkerAtCursor
                | Action::RemoveAllMarkers
                | Action::NextMarker
                | Action::PrevMarker
                | Action::GoToMarker(_)
                | Action::JumpBack
                | Action::SetReference
                | Action::ClearReference
                | Action::ZoomToMeasurement
                | Action::MarkerNavigator => unreachable!(),
            },
            PanelKind::Waves(w) => match action {
                Action::ZoomIn => w.zoom_in(doc, now),
                Action::ZoomOut => w.zoom_out(doc, now),
                Action::ZoomFit => w.zoom_fit(doc, now),
                Action::ZoomToCursor => w.zoom_to_cursor(doc, now),
                Action::PanPageLeft => w.pan_fraction(doc, -1.0, now),
                Action::PanPageRight => w.pan_fraction(doc, 1.0, now),
                Action::GoToStart => w.go_to_start(doc, now),
                Action::GoToEnd => w.go_to_end(doc, now),
                Action::GoToCursor => w.go_to_cursor(doc, now),
                Action::PanLeft => {
                    if !w.fold_key(doc, false, false) {
                        w.pan_fraction(doc, -0.25, now);
                    }
                }
                Action::PanRight => {
                    if !w.fold_key(doc, true, false) {
                        w.pan_fraction(doc, 0.25, now);
                    }
                }
                Action::FoldGroupDeep => _ = w.fold_key(doc, false, true),
                Action::UnfoldGroupDeep => _ = w.fold_key(doc, true, true),
                Action::GroupSelection => _ = w.group_selected(),
                Action::Ungroup => _ = w.ungroup_selected(),
                Action::RenameGroup => _ = w.start_rename(),
                Action::NextEdge => w.next_edge(doc, now),
                Action::PrevEdge => w.prev_edge(doc, now),
                Action::RemoveSelected => w.remove_selected(),
                Action::CopySignals => w.copy_selected(doc),
                Action::CutSignals => w.cut_selected(doc),
                Action::SelectAll => w.select_all(),
                Action::ClearSelection => w.clear_selection(doc),
                Action::CycleFormat => w.cycle_format(doc),
                Action::ToggleAnalog => w.toggle_analog(),
                Action::ToggleStack => _ = w.toggle_stack(),
                Action::ToggleOverlaid => _ = w.toggle_overlaid(),
                Action::SetTint(tint) => _ = w.tint_selected(tint),
                Action::IncreaseRowHeight => w.step_row_height(1),
                Action::DecreaseRowHeight => w.step_row_height(-1),
                Action::ResetRowHeight => w.step_row_height(0),
                Action::MoveSelectionUp => w.move_selection(-1),
                Action::MoveSelectionDown => w.move_selection(1),
                Action::PasteSignals
                | Action::SplitRight
                | Action::SplitDown
                | Action::NewPanel
                | Action::ClosePanel
                | Action::FocusNextPanel
                | Action::FocusPrevPanel
                | Action::ToggleViewportLink
                | Action::ToggleCursorLink
                | Action::NextCycle
                | Action::PrevCycle
                | Action::ToggleCycleOrigin
                | Action::AddOrRenameMarker
                | Action::SetMarker(_)
                | Action::RemoveMarkerAtCursor
                | Action::RemoveAllMarkers
                | Action::NextMarker
                | Action::PrevMarker
                | Action::GoToMarker(_)
                | Action::JumpBack
                | Action::SetReference
                | Action::ClearReference
                | Action::ZoomToMeasurement
                | Action::MarkerNavigator => unreachable!(),
            },
            PanelKind::Pipeline(p) => match action {
                Action::ZoomIn => p.zoom_in(doc, now),
                Action::ZoomOut => p.zoom_out(doc, now),
                Action::ZoomFit => p.zoom_fit(doc, now),
                Action::ZoomToCursor => p.zoom_to_cursor(doc, now),
                Action::PanPageLeft => p.pan_fraction(doc, -1.0, now),
                Action::PanPageRight => p.pan_fraction(doc, 1.0, now),
                Action::GoToStart => p.go_to_start(doc, now),
                Action::GoToEnd => p.go_to_end(doc, now),
                Action::GoToCursor => p.go_to_cursor(doc, now),
                Action::PanLeft => p.pan_fraction(doc, -0.25, now),
                Action::PanRight => p.pan_fraction(doc, 0.25, now),
                Action::ClearSelection => p.escape(doc),
                Action::MoveSelectionUp => p.move_selection(doc, panel, -1, now),
                Action::MoveSelectionDown => p.move_selection(doc, panel, 1, now),
                Action::IncreaseRowHeight => p.step_row_height(doc, 1, now),
                Action::DecreaseRowHeight => p.step_row_height(doc, -1, now),
                Action::ResetRowHeight => p.reset_row_height(doc, now),
                Action::NextEdge
                | Action::PrevEdge
                | Action::RemoveSelected
                | Action::CopySignals
                | Action::CutSignals
                | Action::PasteSignals
                | Action::SelectAll
                | Action::CycleFormat
                | Action::ToggleAnalog
                | Action::ToggleStack
                | Action::ToggleOverlaid
                | Action::SetTint(_)
                | Action::GroupSelection
                | Action::Ungroup
                | Action::RenameGroup
                | Action::FoldGroupDeep
                | Action::UnfoldGroupDeep => return,
                Action::SplitRight
                | Action::SplitDown
                | Action::NewPanel
                | Action::ClosePanel
                | Action::FocusNextPanel
                | Action::FocusPrevPanel
                | Action::ToggleViewportLink
                | Action::ToggleCursorLink
                | Action::NextCycle
                | Action::PrevCycle
                | Action::ToggleCycleOrigin
                | Action::AddOrRenameMarker
                | Action::SetMarker(_)
                | Action::RemoveMarkerAtCursor
                | Action::RemoveAllMarkers
                | Action::NextMarker
                | Action::PrevMarker
                | Action::GoToMarker(_)
                | Action::JumpBack
                | Action::SetReference
                | Action::ClearReference
                | Action::ZoomToMeasurement
                | Action::MarkerNavigator => unreachable!(),
            },
            _ => return,
        }
        self.changed();
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let mut animating = self.doc.shared.viewport.tick(now);
        for panel in self.panels.iter_mut() {
            animating |= panel.tick(&self.doc, now);
        }
        let waiting_to_save = self.workspace_tick(now);
        let waiting_for_settings = self.settings_tick(now, false);
        animating || waiting_to_save || waiting_for_settings
    }

    pub fn is_animating(&self) -> bool {
        self.doc.shared.viewport.is_animating() || self.panels.iter().any(Panel::is_animating)
    }

    pub fn layout_panel(
        &mut self,
        id: PanelId,
        bounds: Rect,
        theme: &Theme,
        device_scale: f64,
        measure: &mut dyn TextMeasure,
    ) -> Option<PanelLayout<'_>> {
        let device_scale = if device_scale.is_finite() && device_scale > 0.0 {
            device_scale
        } else {
            1.0
        };
        self.doc.clocks.defaults = self.pipeline_clocks();
        self.panels.sync_timeline_dividers();
        let metadata = self.panels.timeline_metadata(id, bounds, theme.zoom);
        let doc = &self.doc;
        Some(match &mut self.panels.get_mut(id)?.kind {
            PanelKind::Waves(w) => PanelLayout::Waves(w.layout(bounds, doc, theme, metadata)),
            PanelKind::Pipeline(p) => {
                PanelLayout::Pipeline(p.layout(bounds, doc, theme, metadata, device_scale))
            }
            PanelKind::Table(table) => PanelLayout::Table(table.layout(bounds, theme, measure)),
            _ => return None,
        })
    }

    pub fn render_panel(
        &mut self,
        id: PanelId,
        theme: &Theme,
        measure: &mut dyn TextMeasure,
    ) -> &Scene {
        let mut scene = std::mem::take(&mut self.scene);
        self.render_panel_into(id, theme, measure, &mut scene);
        self.scene = scene;
        &self.scene
    }

    pub fn render_panel_into(
        &mut self,
        id: PanelId,
        theme: &Theme,
        measure: &mut dyn TextMeasure,
        scene: &mut Scene,
    ) {
        scene.clear();
        let Some(panel) = self.panels.get(id) else {
            return;
        };
        let focused = id == self.panels.focused_id();
        let bounds = match &panel.kind {
            PanelKind::Waves(w) => {
                let mut theme = *theme;
                let appearance = &self.settings.resolved().appearance;
                theme.digital_wave_width = appearance.digital_wave_width as f32;
                theme.analog_wave_width = appearance.analog_wave_width as f32;
                theme.wave_spacing = appearance.wave_spacing as f32;
                crate::wave::paint::paint(
                    w,
                    &self.doc,
                    &theme,
                    &mut self.text,
                    measure,
                    scene,
                    focused,
                );
                w.last_layout().bounds
            }
            PanelKind::Pipeline(p) => {
                crate::pipeline::paint::paint(
                    p,
                    &self.doc,
                    theme,
                    &mut self.text,
                    measure,
                    scene,
                    focused,
                );
                p.last_layout().bounds
            }
            PanelKind::Table(table) => {
                crate::table::paint::paint(table, theme, &mut self.text, measure, scene);
                table.layout.bounds
            }
            _ => return,
        };
        if focused && self.panels.len() > 1 && theme.islands.is_none() {
            scene.quad(
                bounds,
                crate::Color::TRANSPARENT,
                0.0,
                1.0,
                theme.border_focused,
            );
        }
    }

    #[must_use]
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    #[must_use]
    pub fn pipeline_generators(&self) -> Vec<(String, Traced<TrackRef>)> {
        let traces = self.doc.traces();
        traces
            .loaded()
            .flat_map(|(trace, session)| {
                session
                    .tracks()
                    .iter()
                    .filter(|t| t.kind == "PIPELINE")
                    .map(move |t| {
                        let path = t.path.join(".");
                        let label = if traces.is_combined() {
                            format!("{trace} · {path}")
                        } else {
                            path
                        };
                        (label, Traced::new(trace, t.id))
                    })
            })
            .collect()
    }

    fn time_range_text(&self) -> String {
        let base = self.doc.time_base();
        let (a, b) = self.doc.limits();
        format!(
            "{} – {}",
            format_time(a as f64, base),
            format_time(b as f64, base)
        )
    }

    #[must_use]
    pub fn start_summary(&self) -> Option<StartSummary> {
        let traces = self.doc.traces();
        traces.first_loaded()?;
        let sessions = || traces.loaded().map(|(_, s)| s);
        Some(StartSummary {
            name: self.doc.name()?,
            time_range: self.time_range_text(),
            variables: sessions().map(|s| s.hierarchy().var_count()).sum(),
            tracks: sessions().map(|s| s.tracks().len()).sum(),
            pipelines: self.pipeline_generators(),
        })
    }

    #[must_use]
    pub fn text_edit(&self) -> Option<TextEdit> {
        let panel = self.panels.focused_id();
        if let Some((p, id)) = self.marker_edit.filter(|(p, _)| *p == panel) {
            let ix = self.doc.markers().iter().position(|m| m.id == id)?;
            let rect = self
                .panels
                .get(p)?
                .kind
                .marker_lane()
                .and_then(|lane| lane.name_field(ix, id));
            return Some(TextEdit {
                target: EditTarget::Marker { panel: p, id },
                rect,
                text: self.doc.markers()[ix].label.clone().unwrap_or_default(),
                select_all: true,
                label: format!("Name of marker {id}"),
            });
        }
        let waves = self.panels.focused_waves()?;
        let editing = waves.row_edit?;
        let entry = editing.row();
        let (target, text, label) = match editing {
            crate::wave::model::RowEdit::Group(_) => (
                EditTarget::Group { panel, entry },
                waves.items().get(entry)?.name().to_owned(),
                "Group name",
            ),
            crate::wave::model::RowEdit::Slice(_) => (
                EditTarget::Slice { panel, entry },
                String::new(),
                "Bit index or slice (for example 7:0)",
            ),
        };
        Some(TextEdit {
            target,
            rect: waves.text_edit_rect(),
            text,
            select_all: true,
            label: label.into(),
        })
    }

    fn open_marker_name(&mut self, panel: PanelId, id: crate::marker::MarkerId, now: Instant) {
        let Some(time) = self
            .doc
            .markers()
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.time)
        else {
            return;
        };
        let doc = &mut self.doc;
        let Some(nav) = self.panels.get_mut(panel).and_then(|p| p.kind.nav_mut()) else {
            return;
        };
        nav.reveal(doc, time, now);
        self.marker_edit = Some((panel, id));
        self.changed();
    }

    fn reference_action(&mut self, action: Action, now: Instant) {
        let doc = &mut self.doc;
        let Some(nav) = self.panels.focused_mut().kind.nav_mut() else {
            return;
        };
        let cursor = nav.cursor(doc);
        let text = match (action, cursor) {
            (Action::ClearReference, _) => {
                if !doc.set_reference(None) {
                    return;
                }
                "Reference cleared".into()
            }
            (Action::SetReference, Some(c)) => {
                let reference = crate::marker::reference_at(doc.markers(), c);
                if !doc.set_reference(Some(reference)) {
                    return;
                }
                match reference {
                    crate::marker::Reference::Marker(id) => {
                        format!("Measuring from marker {id} · ⇧R clears")
                    }
                    crate::marker::Reference::Time(t) => format!(
                        "Measuring from {} · ⇧R clears",
                        format_time(t as f64, doc.time_base())
                    ),
                }
            }
            (Action::SetReference, None) => "Place the cursor first".into(),
            (_, cursor) => match (doc.reference_time(), cursor) {
                (Some(r), Some(c)) => {
                    crate::marker::zoom_between(doc, nav, r, c, now);
                    self.changed();
                    return;
                }
                _ => "Nothing to zoom to · R measures from the cursor".into(),
            },
        };
        self.announce(text);
        self.changed();
    }

    #[must_use]
    pub fn navigator_rows(&self, query: &str) -> Vec<crate::marker::NavigatorRow> {
        let selected = self
            .panels
            .focused()
            .kind
            .nav()
            .and_then(|nav| nav.clocks().selected(&self.doc.clocks));
        crate::marker::navigator_rows(&self.doc, query, selected)
    }

    #[must_use]
    pub fn menu(&self) -> Option<&WaveMenu> {
        let panel = self.panels.focused_id();
        match &self.transient_menu {
            Some((p, menu)) if *p == panel => Some(menu),
            _ => self.panels.focused_waves()?.menu.as_ref(),
        }
    }

    #[must_use]
    pub fn adds_marker_at(&self, panel: PanelId, p: Point) -> bool {
        let Some(panel) = self.panels.get(panel) else {
            return false;
        };
        let (header, lane) = match &panel.kind {
            PanelKind::Waves(w) => (w.last_layout().header, &w.last_layout().marker_lane),
            PanelKind::Pipeline(p) => (p.last_layout().header, &p.last_layout().marker_lane),
            _ => return false,
        };
        p.x >= lane.time_left
            && (header.contains(p) || (lane.band.contains(p) && lane.hit(p).is_none()))
    }

    fn lane_press(&mut self, panel: PanelId, ev: PointerEvent) -> bool {
        let PointerEvent::Down {
            position,
            button,
            modifiers,
        } = ev
        else {
            return false;
        };
        let Some(hit) = self
            .panels
            .get(panel)
            .and_then(|p| p.kind.marker_lane())
            .and_then(|lane| lane.hit(position))
        else {
            return false;
        };
        match (button, hit) {
            (MouseButton::Right, hit) => {
                let Some(entries) = crate::marker::menu(&self.doc, &hit) else {
                    return false;
                };
                if let Some(w) = self.panels.waves_mut(panel) {
                    w.menu_dismiss();
                }
                self.transient_menus += 1;
                self.transient_menu = Some((
                    panel,
                    WaveMenu {
                        kind: WaveMenuKind::Lane,
                        row: self.transient_menus,
                        position,
                        entries,
                    },
                ));
                true
            }
            (MouseButton::Middle | MouseButton::Left, LaneHit::Chip(ix))
                if button == MouseButton::Middle || modifiers.alt =>
            {
                match self.doc.markers().get(ix).map(|m| m.id) {
                    Some(id) => {
                        self.doc.set_reference(Some(Reference::Marker(id)));
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    fn pipeline_label_press(&mut self, panel: PanelId, ev: PointerEvent) -> bool {
        let PointerEvent::Down {
            position,
            button: MouseButton::Right,
            ..
        } = ev
        else {
            return false;
        };
        let Some(row) = self
            .panels
            .pipeline(panel)
            .and_then(|pipeline| pipeline.label_row_at(&self.doc, position))
        else {
            return false;
        };
        if let Some(pipeline) = self.panels.pipeline_mut(panel) {
            pipeline.select_row(&mut self.doc, panel, row);
        }
        self.transient_menus += 1;
        self.transient_menu = Some((
            panel,
            WaveMenu {
                kind: WaveMenuKind::PipelineLabel,
                row: self.transient_menus,
                position,
                entries: vec![MenuEntry::Item(MenuItem {
                    action: MenuAction::PipelineStart(row),
                    label: "Go to start".into(),
                    badge: Some("double-click".into()),
                    checked: false,
                })],
            },
        ));
        true
    }

    fn lane_verb(&mut self, panel: PanelId, verb: LaneVerb, now: Instant) {
        let find = |doc: &Document, id| doc.markers().iter().find(|m| m.id == id).cloned();
        match verb {
            LaneVerb::GoTo(id) => {
                let doc = &mut self.doc;
                if let Some(nav) = self.panels.get_mut(panel).and_then(|p| p.kind.nav_mut()) {
                    let text = crate::marker::walk(doc, nav, crate::marker::Walk::To(id), now)
                        .unwrap_or_else(|missing| missing);
                    self.announce(text);
                }
            }
            LaneVerb::Rename(id) => self.open_marker_name(panel, id, now),
            LaneVerb::MeasureFrom(id) => {
                self.doc.set_reference(Some(Reference::Marker(id)));
            }
            LaneVerb::MoveToCursor(id) => {
                let cursor = self
                    .panels
                    .get(panel)
                    .and_then(|p| p.kind.nav())
                    .and_then(|n| n.cursor(&self.doc));
                match cursor {
                    Some(c) if self.doc.move_marker(id, c) => {}
                    Some(_) => self.announce("Another marker is at the cursor".into()),
                    None => self.announce("Place the cursor first".into()),
                }
            }
            LaneVerb::Remove(id) => {
                self.doc.remove_marker(id);
            }
            LaneVerb::MeasureSpan(a, b) => {
                let Some(to) = find(&self.doc, b) else { return };
                self.doc.set_reference(Some(Reference::Marker(a)));
                let doc = &mut self.doc;
                if let Some(nav) = self.panels.get_mut(panel).and_then(|p| p.kind.nav_mut()) {
                    nav.jump_cursor(doc, to.time, now);
                }
            }
            LaneVerb::ZoomToSpan(a, b) => self.zoom_to_span(panel, a, b, now),
            LaneVerb::Copy(_) | LaneVerb::CopySpan(..) => {
                let Some(nav) = self.panels.get(panel).and_then(|p| p.kind.nav()) else {
                    return;
                };
                let rulers = nav.clocks().rulers(&self.doc.clocks);
                let trace = self.doc.name().unwrap_or_default();
                let base = self.doc.time_base();
                let text = match verb {
                    LaneVerb::Copy(id) => find(&self.doc, id)
                        .map(|m| crate::marker::text(&trace, &m, base, nav.clocks(), &rulers)),
                    LaneVerb::CopySpan(a, b) => find(&self.doc, a)
                        .zip(find(&self.doc, b))
                        .map(|(a, b)| crate::marker::span_text(&trace, &a, &b, base, &rulers)),
                    _ => None,
                };
                if let Some(text) = text {
                    self.events.push(Event::CopyText(text));
                    self.announce("Copied".into());
                }
            }
            LaneVerb::ZoomToMeasurement => self.reference_action(Action::ZoomToMeasurement, now),
            LaneVerb::ClearReference => self.reference_action(Action::ClearReference, now),
        }
    }

    fn zoom_to_span(
        &mut self,
        panel: PanelId,
        from: crate::marker::MarkerId,
        to: crate::marker::MarkerId,
        now: Instant,
    ) {
        let ix = |id| self.doc.markers().iter().position(|m| m.id == id);
        let (Some(a), Some(b)) = (ix(from), ix(to)) else {
            return;
        };
        let doc = &mut self.doc;
        let Some(nav) = self.panels.get_mut(panel).and_then(|p| p.kind.nav_mut()) else {
            return;
        };
        if crate::marker::zoom_to(doc, nav, a.min(b), a.max(b), now) {
            self.changed();
        }
    }

    fn commit_text(&mut self, target: EditTarget, text: Option<String>) {
        match target {
            EditTarget::Slice { panel, entry } => {
                if let Some(waves) = self.panels.waves_mut(panel)
                    && waves.row_edit == Some(crate::wave::model::RowEdit::Slice(entry))
                {
                    if let Some(text) = text {
                        if let Err(error) = waves.add_slice(&self.doc, entry, &text) {
                            self.events.push(Event::Notice(error.to_string()));
                        }
                    } else {
                        waves.row_edit = None;
                    }
                    self.changed();
                }
            }
            EditTarget::Group { panel, entry } => {
                if let Some(waves) = self.panels.waves_mut(panel)
                    && waves.row_edit == Some(crate::wave::model::RowEdit::Group(entry))
                {
                    waves.finish_rename(text.as_deref());
                    self.changed();
                }
            }
            EditTarget::Marker { panel, id } => {
                if self.marker_edit != Some((panel, id)) {
                    return;
                }
                self.marker_edit = None;
                if let Some(text) = text {
                    self.doc.rename_marker(id, &text);
                }
                self.changed();
            }
        }
    }

    pub fn status(&self) -> Status {
        let mut s = Status {
            sidebar_notice: self.variables.notice.clone(),
            workspace_notice: self
                .workspace
                .scheduler
                .error()
                .map(str::to_owned)
                .or_else(|| self.workspace.notices.last().cloned()),
            file: self.doc.name(),
            frames: self.frames.status(),
            announcement: self.announcement.clone(),
            ..Default::default()
        };
        if self.doc.is_loaded() {
            let budget = self.table_memory_budget();
            s.memory = Some(MemoryStatus {
                used: budget.used(),
                limit: budget.limit(),
            });
            let base = self.doc.time_base();
            s.time_range = Some(self.time_range_text());
            let infos: Vec<_> = self.doc.traces().loaded().map(|(_, s)| s.info()).collect();
            let signals: usize = infos.iter().map(|i| i.signal_count).sum();
            s.signals = Some(format!("{signals} signals"));
            s.changes = infos
                .iter()
                .map(|i| i.change_count)
                .sum::<Option<u64>>()
                .map(|n| format!("{n} changes"));
            s.panel = (self.panels.len() > 1).then(|| self.panels.focused().title());
            let focused = self.panels.focused();
            let (nav, width) = match &focused.kind {
                PanelKind::Waves(w) => (&w.nav, f64::from(w.wave_width)),
                PanelKind::Pipeline(p) => {
                    s.hover = p.hover_text(&self.doc);
                    (&p.nav, p.last_layout().cells_width_f64())
                }
                PanelKind::Table(table) => (&table.nav, f64::from(table.layout.body.width())),
                _ => return s,
            };
            let vp = nav.viewport(&self.doc);
            let px_per = vp.width() / width.max(1.0);
            s.px_per = Some(format!("1 px = {}", format_time(px_per, base)));
            let cursor = nav.cursor(&self.doc);
            s.cursor = cursor.map(|c| format_time(c as f64, base));
            let rulers = nav.clocks().rulers(&self.doc.clocks);
            if let Some(c) = cursor {
                s.clocks = rulers
                    .iter()
                    .map(|clock| crate::clock::position_at(nav.clocks(), clock, c))
                    .collect();
                if let Some(r) = self.doc.reference_time() {
                    let measured = crate::measure::measure(&rulers, r, c);
                    let parts: Vec<String> =
                        std::iter::once(crate::wave::overlay::signed_time(measured.dt(), base))
                            .chain(
                                measured
                                    .clocks
                                    .iter()
                                    .map(|count| format!("{} {}", count.cycles, count.name)),
                            )
                            .collect();
                    s.measure = Some(format!("R → cursor {}", parts.join(" · ")));
                }
            }
            if !self.doc.markers().is_empty() {
                s.markers = Some(format!("{} markers", self.doc.markers().len()));
            }
        }
        s
    }

    #[must_use]
    pub fn trace_state(&self) -> TraceState {
        self.doc.state()
    }

    #[must_use]
    pub fn debug_state(&self) -> String {
        self.panels.layout().panels().into_iter().map(|id| {
        let panel = self.panels.get(id).unwrap();
        if let Some(p) = panel.kind.pipeline() {
            let rows = p.rows.target();
            let (state, count) = match p.rows(&self.doc) {
                crate::pipeline::Rows::Ready(set) => ("ready", set.len()),
                crate::pipeline::Rows::Loading => ("loading", 0),
                crate::pipeline::Rows::Failed(_) => ("failed", 0),
                crate::pipeline::Rows::Unresolved => ("unresolved", 0),
                crate::pipeline::Rows::Unavailable => ("unavailable", 0),
            };
            return format!(
                "panel={} pipeline track={} focused={} linked=({},{}) {state} rows={count} top={:.2} row_px={:.2} label_w={} cursor={:?} markers={} viewport=({:.0},{:.0}) hover={:?} drag={:?}",
                id.0, p.track.path().join("."), id == self.panels.focused_id(), p.nav.link.viewport, p.nav.link.cursor,
                rows.top, rows.row_px, p.label_width,
                p.nav.cursor(&self.doc), self.doc.markers().len(),
                p.nav.viewport(&self.doc).start, p.nav.viewport(&self.doc).end,
                p.hover, p.drag,
            );
        }
        if let Some(model) = panel.kind.transaction() {
            let state = match model.state(&self.doc) {
                crate::transaction::TxPanelState::Empty => "empty",
                crate::transaction::TxPanelState::Refused(_) => "refused",
                crate::transaction::TxPanelState::Loading => "loading",
                crate::transaction::TxPanelState::Failed(_) => "failed",
                crate::transaction::TxPanelState::Missing => "missing",
                crate::transaction::TxPanelState::Ready(_) => "ready",
            };
            return format!(
                "panel={} transaction focused={} {state} record={:?} pinned={} back={} forward={} selection={:?}",
                id.0,
                id == self.panels.focused_id(),
                model.shown().map(|shown| (shown.track.path().join("."), shown.id.0)),
                model.pinned(),
                model.can_go_back(),
                model.can_go_forward(),
                self.doc.selection().map(|s| s.id.0),
            );
        }
        if let Some(table) = panel.kind.table() {
            return format!(
                "panel={} table focused={} rows={} top={} selected={:?} sort={:?} first_time={:?} status={}",
                id.0,
                id == self.panels.focused_id(),
                table.len(),
                table.viewport.top,
                table.selected,
                table.applied_sort(),
                table.row_time(0),
                table.status()
            );
        }
        let Some(w) = panel.kind.waves() else {
            let kind = match &panel.kind {
                PanelKind::Start => "start",
                PanelKind::Settings => "settings",
                _ => "unsupported",
            };
            return format!(
                "panel={} {kind} focused={} drag={:?} sidebar_w={}px scopes_frac={:.2}",
                id.0,
                id == self.panels.focused_id(),
                self.drag,
                self.sidebar_width,
                self.scopes_fraction
            );
        };
        format!(
            "panel={} focused={} linked=({},{}) items={} loaded={} selected={:?} anchor={:?} cursor={:?} markers={} viewport=({:.0},{:.0}) menu={} drag={:?} sidebar_w={}px scopes_frac={:.2}",
            id.0, id == self.panels.focused_id(), w.nav.link.viewport, w.nav.link.cursor,
            w.items().len(),
            w.loaded_count(),
            w.selected,
            w.anchor,
            w.cursor(&self.doc),
            self.doc.markers().len(),
            w.viewport(&self.doc).start,
            w.viewport(&self.doc).end,
            w.menu.is_some(),
            self.drag,
            self.sidebar_width,
            self.scopes_fraction
        )
        }).collect::<Vec<_>>().join("\n")
    }
}
