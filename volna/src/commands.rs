//! Shared command definitions for the main menu, native menus, and command palette.

use crate::app::{self as actions, pipeline_generators, recent_labels};
use gpui_kit::{Action, Menu, MenuItem, OsAction};
use std::rc::Rc;
use volna_core::App as CoreApp;
use volna_core::app::ClockCommand;
use volna_core::wave::Tint;

#[derive(Clone, Copy)]
pub(crate) enum Scope {
    Global,
    Recording,
    Panel,
}

#[derive(Clone)]
pub(crate) struct Command {
    pub label: String,
    pub palette_label: String,
    pub action: Rc<dyn Action>,
    pub enabled: bool,
    pub checked: bool,
    pub scope: Scope,
    os_action: Option<OsAction>,
}

impl Command {
    fn new(
        label: impl Into<String>,
        palette_label: impl Into<String>,
        action: impl Action,
        scope: Scope,
        enabled: bool,
    ) -> Self {
        Self {
            label: label.into(),
            palette_label: palette_label.into(),
            action: Rc::new(action),
            scope,
            enabled,
            checked: false,
            os_action: None,
        }
    }
    fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
}

#[derive(Clone)]
pub(crate) enum Entry {
    Command(Command),
    Submenu(&'static str, Vec<Entry>),
    Separator,
    Label(&'static str),
}

fn dynamic(
    label: String,
    palette_label: String,
    action: impl Action,
    scope: Scope,
    enabled: bool,
) -> Entry {
    Entry::Command(Command::new(label, palette_label, action, scope, enabled))
}

struct State<'a> {
    app: &'a CoreApp,
    recent: Option<Vec<String>>,
}

impl<'a> State<'a> {
    fn new(app: &'a CoreApp) -> Self {
        Self {
            app,
            recent: recent_labels(app),
        }
    }
    fn loaded(&self) -> bool {
        self.app.doc.is_loaded()
    }
    fn kind(&self) -> &volna_core::panels::PanelKind {
        &self.app.panels.focused().kind
    }
    fn timed(&self) -> bool {
        self.loaded() && self.kind().nav().is_some()
    }
    fn timeline(&self) -> bool {
        self.loaded() && self.kind().shows_rulers()
    }
    fn waves(&self) -> Option<&volna_core::wave::model::WaveModel> {
        self.kind().waves()
    }
    fn selection(&self) -> bool {
        self.waves().is_some_and(|waves| !waves.selected.is_empty())
    }
    fn has_rows(&self) -> bool {
        self.waves().is_some_and(|waves| !waves.items().is_empty())
    }
    fn cursor(&self) -> Option<u64> {
        self.kind().nav().and_then(|nav| nav.cursor(&self.app.doc))
    }
    fn markers(&self) -> &[volna_core::marker::Marker] {
        self.app.doc.markers()
    }
    fn at_marker(&self) -> bool {
        self.cursor()
            .is_some_and(|time| volna_core::marker::at(self.markers(), time).is_some())
    }
    fn link(&self) -> Option<volna_core::nav::Link> {
        self.kind().nav().map(|nav| nav.link)
    }
}

macro_rules! command_definitions {
    ($($id:ident => ($label:literal, $palette:literal, $scope:ident, $action:expr, $availability:expr)),* $(,)?) => {
        #[derive(Clone, Copy)]
        enum Id { $($id),* }

        impl Id {
            fn resolve(self, state: &State<'_>) -> Command {
                let mut command = match self {
                    $(Self::$id => {
                        let availability: fn(&State<'_>) -> (bool, bool) = $availability;
                        let (enabled, checked) = availability(state);
                        Command::new($label, $palette, $action, Scope::$scope, enabled).checked(checked)
                    }),*
                };
                let history = match self {
                    Self::Undo => Some(("Undo", state.app.undo_label(), OsAction::Undo)),
                    Self::Redo => Some(("Redo", state.app.redo_label(), OsAction::Redo)),
                    _ => None,
                };
                if let Some((verb, label, os_action)) = history {
                    if let Some(label) = label {
                        command.label = format!("{verb} {label}");
                        command.palette_label = format!("{verb}: {label}");
                    }
                    command.os_action = Some(os_action);
                }
                if matches!(self, Self::ToggleCycleOrigin) {
                    command.palette_label = if command.checked { "Reset Cycle Origin" } else { "Set Cycle Origin at Cursor" }.into();
                }
                command
            }
        }
    };
}

command_definitions! {
    OpenFile => ("Open trace…", "Open Trace…", Global, actions::OpenFile, |_| (true, false)),
    AddTrace => ("Add trace…", "Add Trace…", Recording, actions::AddTrace, |s| (s.loaded(), false)),
    ClearRecent => ("Clear recent files", "Clear Recent", Global, actions::ClearRecent, |s| (s.recent.as_ref().is_some_and(|labels| !labels.is_empty()), false)),
    OpenWorkspace => ("Open workspace…", "Open Workspace…", Global, actions::OpenWorkspace, |_| (true, false)),
    SaveWorkspace => ("Save workspace", "Save Workspace", Recording, actions::SaveWorkspace, |s| (s.loaded(), false)),
    SaveWorkspaceAs => ("Save workspace as…", "Save Workspace As…", Recording, actions::SaveWorkspaceAs, |s| (s.loaded(), false)),
    CloseTrace => ("Close trace", "Close Trace", Recording, actions::CloseTrace, |s| (s.loaded(), false)),
    Quit => ("Quit Volna", "Quit Volna", Global, actions::Quit, |_| (true, false)),
    NewPanel => ("New waveform tab", "New Waveform Tab", Panel, actions::NewPanel, |s| (s.loaded(), false)),
    SplitRight => ("Split right", "Split Right", Panel, actions::SplitRight, |s| (s.loaded(), false)),
    SplitDown => ("Split down", "Split Down", Panel, actions::SplitDown, |s| (s.loaded(), false)),
    FocusNextPanel => ("Focus next panel", "Focus Next Panel", Global, actions::FocusNextPanel, |_| (true, false)),
    FocusPrevPanel => ("Focus previous panel", "Focus Previous Panel", Global, actions::FocusPrevPanel, |_| (true, false)),
    ToggleViewportLink => ("Follow shared viewport", "Follow Shared Viewport", Panel, actions::ToggleViewportLink, |s| (s.timed(), s.link().is_some_and(|l| l.viewport))),
    ToggleCursorLink => ("Follow shared cursor", "Follow Shared Cursor", Panel, actions::ToggleCursorLink, |s| (s.timed(), s.link().is_some_and(|l| l.cursor))),
    ClosePanel => ("Close panel", "Close Panel", Panel, actions::ClosePanel, |_| (true, false)),
    ToggleSidebar => ("Show left sidebar", "Toggle Sidebar", Global, actions::ToggleSidebar, |s| (true, s.app.sidebar_visible)),
    ToggleRightSidebar => ("Show right sidebar", "Show right sidebar", Global, actions::ToggleRightSidebar, |s| (true, s.app.panels.layout().right.as_ref().is_some_and(|dock| dock.open))),
    ZoomIn => ("Zoom in", "Zoom In", Panel, actions::ZoomIn, |s| (s.timeline(), false)),
    ZoomOut => ("Zoom out", "Zoom Out", Panel, actions::ZoomOut, |s| (s.timeline(), false)),
    ZoomFit => ("Zoom to fit", "Zoom to Fit", Panel, actions::ZoomFit, |s| (s.timeline(), false)),
    ZoomToCursor => ("Zoom to cursor", "Zoom to Cursor", Panel, actions::ZoomToCursor, |s| (s.timeline() && s.cursor().is_some(), false)),
    ZoomToMeasurement => ("Zoom to measurement", "Zoom to Measurement", Panel, actions::ZoomToMeasurement, |s| (s.timeline() && s.app.doc.reference().is_some() && s.cursor().is_some(), false)),
    IncreaseRowHeight => ("Increase row height", "Increase Row Height", Panel, actions::IncreaseRowHeight, |s| (s.timeline(), false)),
    DecreaseRowHeight => ("Decrease row height", "Decrease Row Height", Panel, actions::DecreaseRowHeight, |s| (s.timeline(), false)),
    ResetRowHeight => ("Reset row height", "Reset Row Height", Panel, actions::ResetRowHeight, |s| (s.timeline(), false)),
    UiZoomIn => ("Increase interface size", "Interface: Zoom In", Global, actions::UiZoomIn, |_| (true, false)),
    UiZoomOut => ("Decrease interface size", "Interface: Zoom Out", Global, actions::UiZoomOut, |_| (true, false)),
    UiZoomReset => ("Reset interface size", "Interface: Reset Zoom", Global, actions::UiZoomReset, |_| (true, false)),
    GoToStart => ("Go to start", "Go to Start", Panel, actions::GoToStart, |s| (s.timed(), false)),
    GoToEnd => ("Go to end", "Go to End", Panel, actions::GoToEnd, |s| (s.timed(), false)),
    GoToCursor => ("Go to cursor", "Go to cursor", Panel, actions::GoToCursor, |s| (s.timeline() && s.cursor().is_some(), false)),
    JumpBack => ("Return from jump", "Return to Before the Last Jump", Panel, actions::JumpBack, |s| (s.timed(), false)),
    PanPageLeft => ("Pan left", "Pan left", Panel, actions::PanPageLeft, |s| (s.timeline(), false)),
    PanPageRight => ("Pan right", "Pan right", Panel, actions::PanPageRight, |s| (s.timeline(), false)),
    PrevEdge => ("Previous transition", "Previous transition", Panel, actions::PrevEdge, |s| (s.loaded() && s.selection(), false)),
    NextEdge => ("Next transition", "Next transition", Panel, actions::NextEdge, |s| (s.loaded() && s.selection(), false)),
    ShowTransaction => ("Show record details", "Show Transaction", Panel, actions::ShowTransaction, |s| (s.timed(), false)),
    PrevCycle => ("Previous clock cycle", "Previous Cycle", Panel, actions::PrevCycle, |s| (s.timed(), false)),
    NextCycle => ("Next clock cycle", "Next Cycle", Panel, actions::NextCycle, |s| (s.timed(), false)),
    ToggleCycleOrigin => ("Cycle origin at cursor", "Cycle origin at cursor", Panel, actions::ToggleCycleOrigin, |s| (s.cursor().is_some(), s.app.clock_choices().is_some_and(|choices| choices.origin))),
    MarkerNavigator => ("Find marker…", "Find marker…", Recording, actions::MarkerNavigator, |s| (s.loaded(), false)),
    AddOrRenameMarker => ("Add or rename marker…", "Add or Name Marker at Cursor", Panel, actions::AddOrRenameMarker, |s| (s.timed() && s.cursor().is_some(), false)),
    RemoveMarkerAtCursor => ("Remove marker at cursor", "Remove Marker at Cursor", Panel, actions::RemoveMarkerAtCursor, |s| (s.at_marker(), false)),
    PrevMarker => ("Previous marker", "Previous Marker", Panel, actions::PrevMarker, |s| (s.timed() && !s.markers().is_empty(), false)),
    NextMarker => ("Next marker", "Next Marker", Panel, actions::NextMarker, |s| (s.timed() && !s.markers().is_empty(), false)),
    SetReference => ("Measure from cursor", "Measure from Cursor", Panel, actions::SetReference, |s| (s.timeline() && s.cursor().is_some(), false)),
    ClearReference => ("Clear measurement reference", "Clear Reference", Panel, actions::ClearReference, |s| (s.timeline() && s.app.doc.reference().is_some(), false)),
    RemoveAllMarkers => ("Remove all markers", "Remove All Markers", Recording, actions::RemoveAllMarkers, |s| (!s.markers().is_empty(), false)),
    CommandPalette => ("Command palette…", "Command palette…", Global, actions::CommandPalette, |_| (true, false)),
    Undo => ("Undo", "Undo", Recording, actions::Undo, |s| (s.app.can_undo(), false)),
    Redo => ("Redo", "Redo", Recording, actions::Redo, |s| (s.app.can_redo(), false)),
    CutSignals => ("Cut signals", "Cut Signals", Panel, actions::CutSignals, |s| (s.loaded() && s.selection(), false)),
    CopySignals => ("Copy signals", "Copy Signals", Panel, actions::CopySignals, |s| (s.loaded() && s.selection(), false)),
    PasteSignals => ("Paste signals", "Paste Signals", Panel, actions::PasteSignals, |s| (s.loaded() && s.waves().is_some(), false)),
    SelectAll => ("Select all signals", "Select all signals", Panel, actions::SelectAll, |s| (s.loaded() && s.has_rows(), false)),
    ClearSelection => ("Clear selection", "Clear selection", Panel, actions::ClearSelection, |s| (s.timed(), false)),
    RemoveSelected => ("Remove selected signals", "Remove selected signals", Panel, actions::RemoveSelected, |s| (s.loaded() && s.selection(), false)),
    OpenSignalMenu => ("Signal options…", "Signal options…", Panel, actions::OpenSignalMenu, |s| (s.loaded() && s.selection(), false)),
    CycleFormat => ("Cycle value format", "Cycle value format", Panel, actions::CycleFormat, |s| (s.loaded() && s.selection(), false)),
    ToggleAnalog => ("Toggle analog drawing", "Toggle Analog Drawing", Panel, actions::ToggleAnalog, |s| (s.loaded() && s.selection(), false)),
    ToggleStack => ("Toggle stacked area", "Toggle Stacked Area", Panel, actions::ToggleStack, |s| (s.loaded() && s.selection(), false)),
    ToggleOverlaid => ("Toggle overlaid lines", "Toggle Overlaid Lines", Panel, actions::ToggleOverlaid, |s| (s.loaded() && s.selection(), false)),
    GroupSelection => ("Group selected signals", "Group selected signals", Panel, actions::GroupSelection, |s| (s.loaded() && s.selection(), false)),
    Ungroup => ("Ungroup", "Ungroup", Panel, actions::Ungroup, |s| (s.loaded() && s.selection(), false)),
    RenameGroup => ("Rename group…", "Rename group…", Panel, actions::RenameGroup, |s| (s.loaded() && s.selection(), false)),
    ToggleScopes => ("Collapse / expand scopes", "Collapse / Expand Scopes", Panel, actions::ToggleScopes, |s| (s.loaded() && s.has_rows(), false)),
    ExpandAllGroups => ("Expand all groups", "Expand All Groups", Panel, actions::ExpandAllGroups, |s| (s.loaded() && s.has_rows(), false)),
    CollapseAllGroups => ("Collapse all groups", "Collapse All Groups", Panel, actions::CollapseAllGroups, |s| (s.loaded() && s.has_rows(), false)),
    OpenSettings => ("Settings…", "Preferences: Open Settings", Global, actions::OpenSettings, |_| (true, false)),
    ToggleSettingsJson => ("Settings (JSON)…", "Preferences: Open Settings (JSON)", Global, crate::settings_panel::ToggleSettingsJson, |_| (true, false)),
    CycleFrameOverlay => ("Cycle GPUI frame overlay", "Developer: Cycle GPUI Frame Overlay", Global, actions::CycleFrameOverlay, |_| (true, false)),
}

pub(crate) fn menu(app: &CoreApp) -> Vec<Entry> {
    use Entry::{Separator as Sep, Submenu as Sub};
    let state = State::new(app);
    let item = |id: Id| Entry::Command(id.resolve(&state));
    let cursor = state.cursor();
    let timed = state.timed();
    let markers = state.markers();
    let mut file = vec![item(Id::OpenFile), item(Id::AddTrace)];
    if let Some(labels) = &state.recent {
        let mut recent: Vec<_> = labels
            .iter()
            .enumerate()
            .map(|(ix, label)| {
                dynamic(
                    label.clone(),
                    format!("Open Recent: {label}"),
                    actions::OpenRecent { ix },
                    Scope::Recording,
                    true,
                )
            })
            .collect();
        let any = !recent.is_empty();
        if !any {
            recent.push(Entry::Label("No recent files"));
        }
        recent.extend([Sep, item(Id::ClearRecent)]);
        file.push(Sub("Open recent", recent));
    }
    file.extend([
        Sep,
        item(Id::OpenWorkspace),
        item(Id::SaveWorkspace),
        item(Id::SaveWorkspaceAs),
        Sep,
        item(Id::CloseTrace),
    ]);
    #[cfg(not(target_family = "wasm"))]
    file.extend([Sep, item(Id::Quit)]);

    let mut panels = vec![
        item(Id::NewPanel),
        item(Id::SplitRight),
        item(Id::SplitDown),
    ];
    let mut pipelines: Vec<_> = pipeline_generators(app)
        .into_iter()
        .map(|(path, track)| {
            dynamic(
                path.clone(),
                format!("Open Pipeline: {path}"),
                actions::OpenPipelineTrack { track },
                Scope::Recording,
                true,
            )
        })
        .collect();
    if pipelines.is_empty() {
        pipelines.push(Entry::Label("No pipeline generators"));
    }
    panels.extend([
        Sub("Open pipeline", pipelines),
        Sep,
        item(Id::FocusNextPanel),
        item(Id::FocusPrevPanel),
        Sep,
        item(Id::ToggleViewportLink),
        item(Id::ToggleCursorLink),
        Sep,
        item(Id::ClosePanel),
    ]);

    let mut view = vec![item(Id::ToggleSidebar)];
    if app.panels.layout().right.is_some() {
        view.push(item(Id::ToggleRightSidebar));
    }
    view.extend([
        Sep,
        item(Id::ZoomIn),
        item(Id::ZoomOut),
        item(Id::ZoomFit),
        item(Id::ZoomToCursor),
        item(Id::ZoomToMeasurement),
        Sep,
        item(Id::IncreaseRowHeight),
        item(Id::DecreaseRowHeight),
        item(Id::ResetRowHeight),
        Sep,
        Sub(
            "Interface size",
            vec![
                item(Id::UiZoomIn),
                item(Id::UiZoomOut),
                item(Id::UiZoomReset),
            ],
        ),
    ]);
    let mut navigate = vec![
        item(Id::GoToStart),
        item(Id::GoToEnd),
        item(Id::GoToCursor),
        item(Id::JumpBack),
        Sep,
        item(Id::PanPageLeft),
        item(Id::PanPageRight),
        Sep,
        item(Id::PrevEdge),
        item(Id::NextEdge),
        item(Id::ShowTransaction),
    ];
    if let Some(choices) = app.clock_choices() {
        let mut clocks = Vec::new();
        for (key, shown) in choices.clocks {
            let label = clock_label(app, &key);
            clocks.push(Entry::Command(
                Command::new(
                    label.clone(),
                    format!(
                        "{} Clock Ruler: {label}",
                        if shown { "Hide" } else { "Show" }
                    ),
                    actions::ClockAction {
                        command: ClockCommand::ToggleRuler(key),
                    },
                    Scope::Panel,
                    true,
                )
                .checked(shown),
            ));
        }
        navigate.extend([
            Sep,
            item(Id::PrevCycle),
            item(Id::NextCycle),
            item(Id::ToggleCycleOrigin),
            Sub("Clock rulers", clocks),
        ]);
    }
    let mut marker_entries = vec![
        item(Id::MarkerNavigator),
        item(Id::AddOrRenameMarker),
        item(Id::RemoveMarkerAtCursor),
        Sep,
        item(Id::PrevMarker),
        item(Id::NextMarker),
    ];
    if !markers.is_empty() {
        marker_entries.push(Sub(
            "Go to marker",
            markers
                .iter()
                .map(|m| {
                    let label = m.label.as_ref().map_or_else(
                        || format!("Marker {}", m.id),
                        |label| format!("{} · {label}", m.id),
                    );
                    dynamic(
                        label.clone(),
                        format!("Go to marker {label}"),
                        actions::GoToMarker { n: m.id.get() },
                        Scope::Panel,
                        true,
                    )
                })
                .collect(),
        ));
    }
    marker_entries.extend([
        Sub(
            "Place numbered marker",
            (1..=6)
                .map(|n| {
                    dynamic(
                        format!("Place or move marker {n}"),
                        format!("Place or Move Marker {n} at Cursor"),
                        actions::SetMarker { n },
                        Scope::Panel,
                        timed && cursor.is_some(),
                    )
                })
                .collect(),
        ),
        Sep,
        item(Id::SetReference),
        item(Id::ClearReference),
        Sep,
        item(Id::RemoveAllMarkers),
    ]);
    vec![
        item(Id::CommandPalette),
        Sep,
        Sub("File", file),
        Sub(
            "Edit",
            vec![
                item(Id::Undo),
                item(Id::Redo),
                Sep,
                item(Id::CutSignals),
                item(Id::CopySignals),
                item(Id::PasteSignals),
                Sep,
                item(Id::SelectAll),
                item(Id::ClearSelection),
                item(Id::RemoveSelected),
            ],
        ),
        Sub("View", view),
        Sub("Navigate", navigate),
        Sub(
            "Signals",
            vec![
                item(Id::OpenSignalMenu),
                item(Id::CycleFormat),
                item(Id::ToggleAnalog),
                item(Id::ToggleStack),
                item(Id::ToggleOverlaid),
                Sep,
                item(Id::GroupSelection),
                item(Id::Ungroup),
                item(Id::RenameGroup),
                Sep,
                item(Id::ToggleScopes),
                item(Id::ExpandAllGroups),
                item(Id::CollapseAllGroups),
            ],
        ),
        Sub("Markers", marker_entries),
        Sub("Panels", panels),
        Sep,
        item(Id::OpenSettings),
        item(Id::ToggleSettingsJson),
    ]
}

fn clock_label(app: &CoreApp, key: &volna_core::clock::ClockKey) -> String {
    if app.doc.traces().is_combined() {
        format!("{} · {}", key.trace, key.item)
    } else {
        key.item.clone()
    }
}

fn colors(enabled: bool) -> Vec<Entry> {
    [
        ("Default", None),
        ("Blue", Some(Tint::Blue)),
        ("Cyan", Some(Tint::Cyan)),
        ("Violet", Some(Tint::Violet)),
        ("Pink", Some(Tint::Pink)),
        ("Grey", Some(Tint::Grey)),
    ]
    .into_iter()
    .map(|(label, tint)| {
        dynamic(
            label.into(),
            format!("Color: {label}"),
            actions::SetTint { tint },
            Scope::Panel,
            enabled,
        )
    })
    .collect()
}

fn flatten(entries: Vec<Entry>, out: &mut Vec<Command>) {
    for entry in entries {
        match entry {
            Entry::Command(command) => out.push(command),
            Entry::Submenu(_, entries) => flatten(entries, out),
            Entry::Label(_) | Entry::Separator => {}
        }
    }
}

pub(crate) fn palette(app: &CoreApp, query: &str) -> Vec<(String, Box<dyn Action>)> {
    let state = State::new(app);
    let mut all = vec![Id::Undo.resolve(&state), Id::Redo.resolve(&state)];
    let mut commands = Vec::new();
    flatten(menu(app), &mut commands);
    flatten(colors(state.selection()), &mut commands);
    commands.push(Id::CycleFrameOverlay.resolve(&state));
    if let Some(choices) = app.clock_choices() {
        for (key, _) in choices.clocks {
            let label = clock_label(app, &key);
            commands.push(Command::new(
                label.clone(),
                format!("Snap and Step to Clock: {label}"),
                actions::ClockAction {
                    command: ClockCommand::Select(key),
                },
                Scope::Panel,
                true,
            ));
        }
    }
    all.extend(commands.into_iter().filter(|command| {
        !command.action.as_any().is::<actions::Undo>()
            && !command.action.as_any().is::<actions::Redo>()
            && !command.action.as_any().is::<actions::MarkerNavigator>()
    }));
    if app.clock_choices().is_some()
        && let Some(cycle) = query
            .split_whitespace()
            .find_map(|word| word.parse::<i64>().ok())
    {
        all.push(Command::new(
            format!("Go to cycle {cycle}"),
            format!("Go to Cycle {cycle}"),
            actions::ClockAction {
                command: ClockCommand::GoToCycle(cycle),
            },
            Scope::Panel,
            true,
        ));
    }
    let chips = app.trace_chips();
    for chip in chips.iter().filter(|_| chips.len() > 1) {
        let what = format!("{} {}", chip.trace, chip.name);
        for (label, verb, enabled) in [
            (
                format!("Reveal Trace {what}"),
                actions::TraceVerb::Reveal,
                true,
            ),
            (
                format!("Rename Trace {what}…"),
                actions::TraceVerb::Rename,
                true,
            ),
            (
                format!("Close Trace {what}"),
                actions::TraceVerb::Close,
                chip.closable,
            ),
        ] {
            if enabled {
                all.push(Command::new(
                    label.clone(),
                    label,
                    actions::TraceAction {
                        trace: chip.trace,
                        verb,
                    },
                    Scope::Recording,
                    true,
                ));
            }
        }
    }
    all.into_iter()
        .map(|command| (command.palette_label, command.action.boxed_clone()))
        .collect()
}

#[derive(PartialEq)]
pub(crate) struct MenuState(Vec<ItemState>);

#[derive(PartialEq)]
enum ItemState {
    Command {
        label: String,
        enabled: bool,
        checked: bool,
    },
    Submenu {
        label: &'static str,
        entries: MenuState,
    },
    Separator,
    Label(&'static str),
}

pub(crate) fn menu_state(entries: &[Entry]) -> MenuState {
    MenuState(
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Command(command) => ItemState::Command {
                    label: command.label.clone(),
                    enabled: command.enabled,
                    checked: command.checked,
                },
                Entry::Submenu(label, entries) => ItemState::Submenu {
                    label,
                    entries: menu_state(entries),
                },
                Entry::Label(label) => ItemState::Label(label),
                Entry::Separator => ItemState::Separator,
            })
            .collect(),
    )
}

fn native_items(entries: Vec<Entry>) -> Vec<MenuItem> {
    entries
        .into_iter()
        .map(|entry| match entry {
            Entry::Command(command) => MenuItem::Action {
                name: command.label.into(),
                action: command.action.boxed_clone(),
                os_action: command.os_action,
                disabled: !command.enabled,
                checked: command.checked,
            },
            Entry::Submenu(name, entries) => MenuItem::submenu(Menu {
                name: name.into(),
                items: native_items(entries),
                disabled: false,
            }),
            Entry::Separator => MenuItem::separator(),
            Entry::Label(label) => MenuItem::action(label, actions::NoPipelines).disabled(true),
        })
        .collect()
}

pub(crate) fn native(app: &CoreApp, entries: Vec<Entry>) -> Vec<Menu> {
    let state = State::new(app);
    let mut application: Vec<_> = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Command(command) => Some(Entry::Command(command.clone())),
            _ => None,
        })
        .collect();
    application.extend([Entry::Separator, Entry::Command(Id::Quit.resolve(&state))]);
    let mut menus = vec![Menu {
        name: "Volna".into(),
        items: native_items(application),
        disabled: false,
    }];
    for entry in entries {
        if let Entry::Submenu(name, entries) = entry {
            menus.push(Menu {
                name: name.into(),
                items: native_items(entries),
                disabled: false,
            });
        }
    }
    menus
}
