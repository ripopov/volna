//! `Workspace`: the GPUI root view, a thin adapter over [`volna_core::App`].

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "app_tests.rs"]
mod tests;

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::base::StyledExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt, App, Context, CursorStyle, Entity, FocusHandle, Focusable,
    IntoElement, KeyBinding, Menu, MenuItem, MouseButton, MouseMoveEvent, MouseUpEvent, OsAction,
    ParentElement, Pixels, Render, ShapedLine, SharedString, Styled, Transformation,
    UniformListScrollHandle, Window, actions, anchored, deferred, div, percentage, point, px,
};
use volna_core::app::{Action, ChromeDrag, Command, Event, SettingsCommand};
use volna_core::document::TraceState;
use volna_core::settings::ZoomStep;
use volna_core::trace::{TraceId, Traced};
use volna_core::wave::MenuEntry;
use volna_core::workspace::recent::{RecentCommand, RecentKey, RecentKind};
use volna_core::{App as CoreApp, FontRole, Instant, Scene};
use volna_trace::data::transactions::TrackRef;
use volna_trace::session::Session;

use crate::theme::{ThemePx, theme};
use crate::ui::icon::icon_svg;
use crate::ui::message_dialog::open_message_dialog;
use crate::ui::text_input::TextInputEvent;
use crate::ui::{
    Icon, IconName, Side, Splitter, SplitterAxis, TextInput, app_draws_controls, icon_button,
    popup_at, render_window_controls,
};
use gpui_kit::component::{
    Selectable,
    button::{Button, ButtonVariants},
    menu::{PopupMenu, PopupMenuItem},
    tooltip::Tooltip,
};

actions!(
    workspace,
    [
        OpenFile,
        AddTrace,
        OpenWorkspace,
        SaveWorkspace,
        SaveWorkspaceAs,
        ToggleSidebar,
        CloseTrace,
        Quit,
        OpenSettings,
        CommandPalette,
        UiZoomIn,
        UiZoomOut,
        UiZoomReset,
        FocusPanel1,
        FocusPanel2,
        FocusPanel3,
        FocusPanel4,
        FocusPanel5,
        FocusPanel6,
        FocusPanel7,
        FocusPanel8,
        FocusPanel9,
        NoPipelines,
        CycleFrameOverlay,
        Undo,
        Redo,
        ClearRecent
    ]
);

actions!(
    waves,
    [
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
        RemoveMarkerAtCursor,
        RemoveAllMarkers,
        NextMarker,
        PrevMarker,
        JumpBack,
        SetReference,
        ClearReference,
        ZoomToMeasurement,
        MarkerNavigator,
        RemoveSelected,
        CopySignals,
        CutSignals,
        PasteSignals,
        OpenSignalMenu,
        SelectAll,
        ClearSelection,
        CycleFormat,
        ToggleAnalog,
        ToggleStack,
        ToggleOverlaid,
        IncreaseRowHeight,
        DecreaseRowHeight,
        ResetRowHeight,
        MoveSelectionUp,
        MoveSelectionDown,
        NextCycle,
        PrevCycle,
        ToggleCycleOrigin,
        GroupSelection,
        Ungroup,
        RenameGroup,
        FoldGroupDeep,
        UnfoldGroupDeep,
    ]
);

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = waves, no_json)]
pub struct GoToMarker {
    pub n: u32,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = waves, no_json)]
pub struct SetMarker {
    pub n: u32,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = waves, no_json)]
pub struct SetTint {
    pub tint: Option<volna_core::wave::Tint>,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenRecent {
    pub ix: usize,
}

pub(crate) fn unix_now() -> u64 {
    #[cfg(not(target_family = "wasm"))]
    return std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    #[cfg(target_family = "wasm")]
    0
}

pub(crate) fn home_dir() -> Option<String> {
    #[cfg(not(target_family = "wasm"))]
    return std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| h.replace('\\', "/"));
    #[cfg(target_family = "wasm")]
    None
}

pub(crate) fn recent_labels(app: &CoreApp) -> Option<Vec<String>> {
    app.workspace.recent.enabled.then(|| {
        app.recent_rows(unix_now(), home_dir().as_deref())
            .into_iter()
            .map(|row| format!("{} — {}", row.name, row.folder))
            .collect()
    })
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = waves, no_json)]
pub struct ClockAction {
    pub command: volna_core::app::ClockCommand,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = workspace, no_json)]
pub struct TraceAction {
    pub trace: TraceId,
    pub verb: TraceVerb,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TraceVerb {
    Reveal,
    Rename,
    Close,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = waves, no_json)]
pub struct ShowTransaction;

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = waves, no_json)]
pub struct OpenPipelineTrack {
    pub track: Traced<TrackRef>,
}

pub(crate) fn pipeline_generators(app: &CoreApp) -> Vec<(String, Traced<TrackRef>)> {
    app.pipeline_generators()
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TextStyleKey {
    pub font: FontRole,
    pub size: u32,
    pub color: [u32; 4],
}

#[derive(Default)]
pub(crate) struct ShapedCache {
    lines: HashMap<TextStyleKey, HashMap<String, ShapedLine>>,
    len: usize,
}

impl ShapedCache {
    pub(crate) fn line(
        &mut self,
        style: TextStyleKey,
        text: &str,
        limit: usize,
        shape: impl FnOnce() -> ShapedLine,
    ) -> &ShapedLine {
        let hit = self
            .lines
            .get(&style)
            .is_some_and(|lines| lines.contains_key(text));
        if !hit {
            if self.len >= limit {
                self.lines.clear();
                self.len = 0;
            }
            self.len += 1;
            self.lines
                .entry(style)
                .or_default()
                .insert(text.to_owned(), shape());
        }
        &self.lines[&style][text]
    }
}

type MenuKey = (u64, u64, u64, u64);

pub(crate) type OnWavesKey = (u64, u64);
pub(crate) type OnWaves = std::collections::HashSet<Traced<volna_trace::data::Member>>;

#[expect(
    clippy::struct_excessive_bools,
    reason = "Host mode, theme, focus, and CLI policy are independent state flags."
)]
pub struct Workspace {
    pub app: CoreApp,
    system_dark: bool,
    #[cfg(target_family = "wasm")]
    pub(crate) remote: Option<crate::web_remote::Bridge>,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) native_store: Option<crate::native_workspace::Store>,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) native_imports: crate::native_import::Imports,
    pub(crate) dock: Option<crate::dock::DockHost>,
    panel_focus_pending: bool,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) waves_focus: FocusHandle,
    pub(crate) scopes_focus: FocusHandle,
    pub(crate) variables_focus: FocusHandle,
    pub(crate) filter: Entity<TextInput>,
    pub(crate) scopes_scroll: UniformListScrollHandle,
    pub(crate) variables_scroll: UniformListScrollHandle,
    pub(crate) status_menu: Option<(gpui_kit::Point<Pixels>, Entity<PopupMenu>)>,
    pub(crate) trace_rename: Option<crate::traces::TraceRename>,
    pub(crate) on_waves: Option<(OnWavesKey, std::rc::Rc<OnWaves>)>,
    pub(crate) frame_view: crate::frame_stats::FrameView,
    wave_menu: Option<HostedWaveMenu>,
    pub(crate) rename: Option<HostedRename>,
    pub(crate) scene: Scene,
    pub(crate) shaped: ShapedCache,
    pub(crate) presence_images: crate::canvas::PresenceImages,
    pub embedded: bool,
    pub(crate) cli_policy: bool,
    menu_generation: Option<MenuKey>,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) config_watcher: Option<notify::RecommendedWatcher>,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) opening_workspace: Option<String>,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) recent_checked: Option<(u64, u64)>,
}

pub(crate) struct HostedRename {
    target: volna_core::app::EditTarget,
    pub(crate) input: Entity<TextInput>,
    _subscription: gpui_kit::Subscription,
}

struct HostedWaveMenu {
    panel: volna_core::panels::PanelId,
    row: usize,
    kind: volna_core::wave::WaveMenuKind,
    position: gpui_kit::Point<Pixels>,
    popup: Entity<PopupMenu>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub fn init(cx: &mut App) {
    cx.bind_keys(vec![
        KeyBinding::new("cmd-z", Undo, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-z", Undo, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-shift-z", Redo, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-shift-z", Redo, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-y", Redo, Some("Workspace && !Embedded")),
    ]);
    crate::ui::text_input::bind_keys(cx);
    cx.bind_keys(crate::palette::key_bindings());
    {
        use gpui_kit::component::input::{Redo as InputRedo, Undo as InputUndo};
        cx.bind_keys(vec![
            KeyBinding::new("cmd-z", InputUndo, Some("Input")),
            KeyBinding::new("ctrl-z", InputUndo, Some("Input")),
            KeyBinding::new("cmd-shift-z", InputRedo, Some("Input")),
            KeyBinding::new("ctrl-shift-z", InputRedo, Some("Input")),
            KeyBinding::new("ctrl-y", InputRedo, Some("Input")),
        ]);
    }
    for n in 1..=9 {
        for key in [n.to_string(), format!("ctrl-{n}")] {
            cx.bind_keys(vec![KeyBinding::new(
                &key,
                gpui_kit::NoAction {},
                Some("TextInput || Input || Command || Dialog || StartPage"),
            )]);
        }
    }
    cx.bind_keys(vec![
        KeyBinding::new("cmd-1", FocusPanel1, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-alt-1", FocusPanel1, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-2", FocusPanel2, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-alt-2", FocusPanel2, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-3", FocusPanel3, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-alt-3", FocusPanel3, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-4", FocusPanel4, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-alt-4", FocusPanel4, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-5", FocusPanel5, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-alt-5", FocusPanel5, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-6", FocusPanel6, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-alt-6", FocusPanel6, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-7", FocusPanel7, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-alt-7", FocusPanel7, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-8", FocusPanel8, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-alt-8", FocusPanel8, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-9", FocusPanel9, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-alt-9", FocusPanel9, Some("Workspace && !Embedded")),
    ]);
    cx.bind_keys(vec![
        KeyBinding::new("cmd-s", SaveWorkspace, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-s", SaveWorkspace, Some("Workspace && !Embedded")),
        KeyBinding::new(
            "cmd-shift-s",
            SaveWorkspaceAs,
            Some("Workspace && !Embedded"),
        ),
        KeyBinding::new(
            "ctrl-shift-s",
            SaveWorkspaceAs,
            Some("Workspace && !Embedded"),
        ),
        KeyBinding::new("cmd-o", OpenFile, None),
        KeyBinding::new("ctrl-o", OpenFile, None),
        KeyBinding::new("cmd-shift-o", AddTrace, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-shift-o", AddTrace, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("ctrl-b", ToggleSidebar, None),
        KeyBinding::new("cmd-w", ClosePanel, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-w", ClosePanel, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-\\", SplitRight, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-\\", SplitRight, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-shift-\\", SplitDown, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-shift-\\", SplitDown, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-n", NewPanel, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-n", NewPanel, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-tab", FocusNextPanel, Some("Workspace && !Embedded")),
        KeyBinding::new(
            "ctrl-shift-tab",
            FocusPrevPanel,
            Some("Workspace && !Embedded"),
        ),
        KeyBinding::new("l", ToggleViewportLink, Some("Waves && !Input")),
        KeyBinding::new("shift-l", ToggleCursorLink, Some("Waves && !Input")),
        KeyBinding::new(
            "shift-escape",
            gpui_kit::component::dock::ToggleZoom,
            Some("Waves && !Input"),
        ),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("cmd-k", CommandPalette, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-k", CommandPalette, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-=", UiZoomIn, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-=", UiZoomIn, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-shift-=", UiZoomIn, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-shift-=", UiZoomIn, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-+", UiZoomIn, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-+", UiZoomIn, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd--", UiZoomOut, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl--", UiZoomOut, Some("Workspace && !Embedded")),
        KeyBinding::new("cmd-0", UiZoomReset, Some("Workspace && !Embedded")),
        KeyBinding::new("ctrl-0", UiZoomReset, Some("Workspace && !Embedded")),
        KeyBinding::new(
            "cmd-shift-,",
            crate::settings_panel::ToggleSettingsJson,
            None,
        ),
        KeyBinding::new(
            "ctrl-shift-,",
            crate::settings_panel::ToggleSettingsJson,
            None,
        ),
        KeyBinding::new(
            "escape",
            crate::settings_panel::SettingsEscape,
            Some("Settings"),
        ),
        KeyBinding::new(
            "cmd-s",
            crate::settings_panel::ApplySettingsJson,
            Some("Settings"),
        ),
        KeyBinding::new(
            "ctrl-s",
            crate::settings_panel::ApplySettingsJson,
            Some("Settings"),
        ),
        KeyBinding::new("=", ZoomIn, Some("Waves && !Input")),
        KeyBinding::new("shift-=", ZoomIn, Some("Waves && !Input")),
        KeyBinding::new("-", ZoomOut, Some("Waves && !Input")),
        KeyBinding::new("f", ZoomFit, Some("Waves && !Input")),
        KeyBinding::new("shift-f", ZoomFit, Some("Waves && !Input")),
        KeyBinding::new("shift-z", ZoomToCursor, Some("Waves && !Input")),
        KeyBinding::new("pageup", PanPageRight, Some("Waves && !Table && !Input")),
        KeyBinding::new("pagedown", PanPageLeft, Some("Waves && !Table && !Input")),
        KeyBinding::new("home", GoToStart, Some("Waves && !Input")),
        KeyBinding::new("s", GoToStart, Some("Waves && !Input")),
        KeyBinding::new("end", GoToEnd, Some("Waves && !Input")),
        KeyBinding::new("e", GoToEnd, Some("Waves && !Input")),
        KeyBinding::new("c", GoToCursor, Some("Waves && !Input")),
        KeyBinding::new("left", PanLeft, Some("Waves && !Input")),
        KeyBinding::new("right", PanRight, Some("Waves && !Input")),
        KeyBinding::new("shift-right", NextEdge, Some("Waves && !Input")),
        KeyBinding::new("shift-left", PrevEdge, Some("Waves && !Input")),
        KeyBinding::new("m", AddOrRenameMarker, Some("Waves && !Input")),
        KeyBinding::new("ctrl-1", SetMarker { n: 1 }, Some("Workspace")),
        KeyBinding::new("ctrl-2", SetMarker { n: 2 }, Some("Workspace")),
        KeyBinding::new("ctrl-3", SetMarker { n: 3 }, Some("Workspace")),
        KeyBinding::new("ctrl-4", SetMarker { n: 4 }, Some("Workspace")),
        KeyBinding::new("ctrl-5", SetMarker { n: 5 }, Some("Workspace")),
        KeyBinding::new("ctrl-6", SetMarker { n: 6 }, Some("Workspace")),
        KeyBinding::new("shift-m", RemoveMarkerAtCursor, Some("Waves && !Input")),
        KeyBinding::new("r", SetReference, Some("Waves && !Table && !Input")),
        KeyBinding::new("shift-r", ClearReference, Some("Waves && !Table && !Input")),
        KeyBinding::new("z", ZoomToMeasurement, Some("Waves && !Table && !Input")),
        KeyBinding::new("'", MarkerNavigator, Some("Waves && !Table && !Input")),
        KeyBinding::new(".", NextMarker, Some("Waves && !Table && !Input")),
        KeyBinding::new(",", PrevMarker, Some("Waves && !Table && !Input")),
        KeyBinding::new("`", JumpBack, Some("Waves && !Table && !Input")),
        KeyBinding::new("1", GoToMarker { n: 1 }, Some("Workspace")),
        KeyBinding::new("2", GoToMarker { n: 2 }, Some("Workspace")),
        KeyBinding::new("3", GoToMarker { n: 3 }, Some("Workspace")),
        KeyBinding::new("4", GoToMarker { n: 4 }, Some("Workspace")),
        KeyBinding::new("5", GoToMarker { n: 5 }, Some("Workspace")),
        KeyBinding::new("6", GoToMarker { n: 6 }, Some("Workspace")),
        KeyBinding::new("7", GoToMarker { n: 7 }, Some("Workspace")),
        KeyBinding::new("8", GoToMarker { n: 8 }, Some("Workspace")),
        KeyBinding::new("9", GoToMarker { n: 9 }, Some("Workspace")),
        KeyBinding::new("backspace", RemoveSelected, Some("Waves && !Input")),
        KeyBinding::new("delete", RemoveSelected, Some("Waves && !Input")),
        KeyBinding::new("shift-f10", OpenSignalMenu, Some("Waves && !Input")),
        KeyBinding::new("cmd-c", CopySignals, Some("Waves && !Table && !Input")),
        KeyBinding::new("ctrl-c", CopySignals, Some("Waves && !Table && !Input")),
        KeyBinding::new("cmd-x", CutSignals, Some("Waves && !Table && !Input")),
        KeyBinding::new("ctrl-x", CutSignals, Some("Waves && !Table && !Input")),
        KeyBinding::new("cmd-v", PasteSignals, Some("Waves && !Table && !Input")),
        KeyBinding::new("ctrl-v", PasteSignals, Some("Waves && !Table && !Input")),
        KeyBinding::new("cmd-a", SelectAll, Some("Waves && !Input")),
        KeyBinding::new("ctrl-a", SelectAll, Some("Waves && !Input")),
        KeyBinding::new("escape", ClearSelection, Some("Waves && !Input")),
        KeyBinding::new("t", CycleFormat, Some("Waves && !Input")),
        KeyBinding::new("a", ToggleAnalog, Some("Waves && !Input")),
        KeyBinding::new("shift-a", ToggleStack, Some("Waves && !Input")),
        KeyBinding::new("shift-o", ToggleOverlaid, Some("Waves && !Input")),
        KeyBinding::new("enter", ShowTransaction, Some("Waves && !Input")),
        KeyBinding::new("up", MoveSelectionUp, Some("Waves && !Input")),
        KeyBinding::new("down", MoveSelectionDown, Some("Waves && !Input")),
        KeyBinding::new("]", NextCycle, Some("Waves && !Input")),
        KeyBinding::new("[", PrevCycle, Some("Waves && !Input")),
        KeyBinding::new("g", GroupSelection, Some("Waves && !Input")),
        KeyBinding::new("shift-g", Ungroup, Some("Waves && !Input")),
        KeyBinding::new("f2", RenameGroup, Some("Waves && !Input")),
        KeyBinding::new("alt-left", FoldGroupDeep, Some("Waves && !Input")),
        KeyBinding::new("alt-right", UnfoldGroupDeep, Some("Waves && !Input")),
        KeyBinding::new(
            "alt-left",
            crate::transaction_panel::Back,
            Some("Transaction"),
        ),
        KeyBinding::new(
            "alt-right",
            crate::transaction_panel::Forward,
            Some("Transaction"),
        ),
        KeyBinding::new("cmd-[", crate::transaction_panel::Back, Some("Transaction")),
        KeyBinding::new(
            "cmd-]",
            crate::transaction_panel::Forward,
            Some("Transaction"),
        ),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.set_menus(menus(&[], None, None, None));
}

pub(crate) fn edit_menu_title(verb: &str, label: Option<&str>) -> String {
    label.map_or_else(|| verb.to_owned(), |label| format!("{verb} {label}"))
}

pub(crate) fn menus(
    pipelines: &[(String, Traced<TrackRef>)],
    undo: Option<&str>,
    redo: Option<&str>,
    recent: Option<&[String]>,
) -> Vec<Menu> {
    let mut file = vec![
        MenuItem::action("Open…", OpenFile),
        MenuItem::action("Add Trace…", AddTrace),
    ];
    if let Some(recent) = recent {
        let mut items: Vec<MenuItem> = recent
            .iter()
            .enumerate()
            .map(|(ix, label)| MenuItem::action(label.clone(), OpenRecent { ix }))
            .collect();
        if items.is_empty() {
            items.push(MenuItem::action("No Recent Files", ClearRecent).disabled(true));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::action("Clear Recent", ClearRecent).disabled(recent.is_empty()));
        file.push(MenuItem::submenu(Menu {
            name: "Open Recent".into(),
            items,
            disabled: false,
        }));
    }
    file.extend([
        MenuItem::action("Close Trace", CloseTrace),
        MenuItem::separator(),
        MenuItem::action("Open Workspace…", OpenWorkspace),
        MenuItem::action("Save Workspace", SaveWorkspace),
        MenuItem::action("Save Workspace As…", SaveWorkspaceAs),
    ]);
    let pipeline_items = if pipelines.is_empty() {
        vec![MenuItem::action(
            "No pipeline generators in the trace",
            NoPipelines,
        )]
    } else {
        pipelines
            .iter()
            .map(|(path, track)| {
                MenuItem::action(path.clone(), OpenPipelineTrack { track: *track })
            })
            .collect()
    };
    vec![
        Menu {
            name: "Volna".into(),
            items: vec![
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit", Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: file,
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action(edit_menu_title("Undo", undo), Undo, OsAction::Undo)
                    .disabled(undo.is_none()),
                MenuItem::os_action(edit_menu_title("Redo", redo), Redo, OsAction::Redo)
                    .disabled(redo.is_none()),
                MenuItem::separator(),
                MenuItem::action("Cut Signals", CutSignals),
                MenuItem::action("Copy Signals", CopySignals),
                MenuItem::action("Paste Signals", PasteSignals),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Command Palette…", CommandPalette),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Split Right", SplitRight),
                MenuItem::action("Split Down", SplitDown),
                MenuItem::action("New Waveform Tab", NewPanel),
                MenuItem::submenu(Menu {
                    name: "Pipeline".into(),
                    items: pipeline_items,
                    disabled: false,
                }),
                MenuItem::action("Close Panel", ClosePanel),
                MenuItem::action("Follow Shared Viewport", ToggleViewportLink),
                MenuItem::action("Follow Shared Cursor", ToggleCursorLink),
                MenuItem::separator(),
                MenuItem::action("Zoom In", ZoomIn),
                MenuItem::action("Zoom Out", ZoomOut),
                MenuItem::action("Zoom to Fit", ZoomFit),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: "Appearance".into(),
                    items: vec![
                        MenuItem::action("Zoom In", UiZoomIn),
                        MenuItem::action("Zoom Out", UiZoomOut),
                        MenuItem::action("Reset Zoom", UiZoomReset),
                    ],
                    disabled: false,
                }),
            ],
            disabled: false,
        },
    ]
}

macro_rules! focus_panel_actions {
    ($el:expr, $cx:expr, [$(($name:ident, $index:expr)),* $(,)?]) => {
        $el $(.on_action($cx.listener(|this: &mut Workspace, _: &$name, window, cx| {
            this.dispatch(
                Command::Panels(volna_core::panels::PanelsCommand::FocusIndex($index)),
                Some(window),
                cx,
            )
        })))*
    };
}

macro_rules! wave_actions {
    ($el:expr, $cx:expr, [$($name:ident),* $(,)?]) => {
        $el $(.on_action($cx.listener(|this: &mut Workspace, _: &$name, window, cx| {
            this.dispatch(Command::Action(Action::$name), Some(window), cx)
        })))*
    };
}

pub(crate) const MEMORY_PILL_W: f32 = 112.0;

fn memory_meter(
    memory: volna_core::app::MemoryStatus,
    t: &crate::theme::Theme,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let full = memory.nearly_full();
    let (tint, border, text) = if full {
        (
            t.editor.error.alpha(0.28),
            t.editor.error.alpha(0.7),
            t.editor.error,
        )
    } else {
        (t.bar.icon_accent.alpha(0.24), t.border, t.bar.text)
    };
    let filled = if memory.used == 0 {
        0.0
    } else {
        (memory.fraction() * MEMORY_PILL_W).max(3.0)
    };
    let detail = SharedString::from(memory.detail());
    div()
        .id("status-memory")
        .debug_selector(|| "status-memory".into())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(t.px(MEMORY_PILL_W))
        .h(t.px(16.0))
        .rounded_full()
        .overflow_hidden()
        .border_1()
        .border_color(border)
        .bg(t.bar.text.alpha(0.05))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(t.bar.text.alpha(0.1)).border_color(t.border_focused))
        .tooltip(move |w, cx| Tooltip::new(detail.clone()).build(w, cx))
        .child(
            div()
                .debug_selector(|| "status-memory-fill".into())
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(t.px(filled))
                .bg(tint),
        )
        .child(
            div()
                .debug_selector(|| "status-memory-label".into())
                .relative()
                .font_family(t.mono_font)
                .text_size(px(t.ui_size_small * 0.92))
                .line_height(t.px(14.0))
                .text_color(text)
                .child(SharedString::from(memory.label())),
        )
}

pub(crate) fn to_modifiers(m: gpui_kit::Modifiers) -> volna_core::geometry::Modifiers {
    volna_core::geometry::Modifiers {
        shift: m.shift,
        control: m.control,
        alt: m.alt,
        platform: m.platform,
    }
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(not(target_family = "wasm"))]
        cx.on_app_quit(|this, cx| {
            this.app.persist_workspace(Instant::now(), true);
            this.app.flush_settings(Instant::now());
            this.after(None, cx);
            async {}
        })
        .detach();
        let filter = cx.new(|cx| TextInput::new("Filter members", cx));
        cx.subscribe(&filter, |this, filter, event, cx| match event {
            TextInputEvent::Changed => {
                let text = filter.read(cx).text().to_owned();
                this.dispatch(Command::SetFilter(text), None, cx);
            }
            TextInputEvent::Submit => this.dispatch(Command::AddSelectedOrAllVars, None, cx),
            TextInputEvent::Cancel => {}
        })
        .detach();
        cx.observe_window_appearance(window, |this, window, cx| {
            this.set_system_dark(crate::theme::is_dark(window.appearance()), cx);
        })
        .detach();
        let waves_focus = cx.focus_handle();
        window.focus(&waves_focus, cx);
        let workspace = Workspace {
            app: CoreApp::new(),
            #[cfg(target_family = "wasm")]
            remote: None,
            #[cfg(not(target_family = "wasm"))]
            native_store: None,
            #[cfg(not(target_family = "wasm"))]
            native_imports: Default::default(),
            dock: None,
            panel_focus_pending: false,
            system_dark: crate::theme::is_dark(window.appearance()),
            focus_handle: cx.focus_handle(),
            waves_focus,
            scopes_focus: cx.focus_handle(),
            variables_focus: cx.focus_handle(),
            filter,
            scopes_scroll: UniformListScrollHandle::new(),
            variables_scroll: UniformListScrollHandle::new(),
            status_menu: None,
            frame_view: Default::default(),
            wave_menu: None,
            rename: None,
            trace_rename: None,
            on_waves: None,
            scene: Scene::default(),
            shaped: ShapedCache::default(),
            presence_images: Default::default(),
            embedded: false,
            cli_policy: false,
            menu_generation: None,
            #[cfg(not(target_family = "wasm"))]
            config_watcher: None,
            #[cfg(not(target_family = "wasm"))]
            opening_workspace: None,
            #[cfg(not(target_family = "wasm"))]
            recent_checked: None,
        };
        Self::start_frame_sampling(window, cx);
        workspace
    }

    pub fn dispatch(
        &mut self,
        command: Command,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_if_current(self.app.doc.generation(), command, window, cx);
    }

    pub(crate) fn dispatch_if_current(
        &mut self,
        generation: u64,
        command: Command,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let before = self.app.panels.focused_id();
        if !self.app.handle_if_current(generation, command) {
            return;
        }
        self.panel_focus_pending |= before != self.app.panels.focused_id();
        self.after(window, cx);
    }

    pub(crate) fn after(&mut self, mut window: Option<&mut Window>, cx: &mut Context<Self>) {
        #[cfg(not(target_family = "wasm"))]
        self.check_recent();
        let panel = self.app.panels.focused_id();
        if let Some(h) = self.rename.as_ref().filter(|h| h.target.panel() != panel) {
            let text = h.input.read(cx).text().to_owned();
            self.app.handle(Command::CommitText(h.target, Some(text)));
        }
        loop {
            let events = self.app.take_events();
            if events.is_empty() {
                break;
            }
            for event in events {
                match event {
                    Event::LoadWorkspace { trace_uri } => {
                        #[cfg(not(target_family = "wasm"))]
                        self.load_workspace_candidates(trace_uri);
                        #[cfg(target_family = "wasm")]
                        crate::web::request_workspace_candidates(&trace_uri);
                    }
                    Event::PersistWorkspace { ticket, bytes } => {
                        #[cfg(not(target_family = "wasm"))]
                        self.write_workspace(ticket, bytes);
                        #[cfg(target_family = "wasm")]
                        crate::web::write_workspace(ticket, bytes);
                    }
                    Event::OpenWorkspaceDialog | Event::SaveWorkspaceDialog => {
                        let save = matches!(event, Event::SaveWorkspaceDialog);
                        #[cfg(not(target_family = "wasm"))]
                        self.workspace_dialog(save, cx);
                        #[cfg(target_family = "wasm")]
                        crate::web::workspace_dialog(save);
                    }
                    Event::TraceOpened { trace_uri } => {
                        #[cfg(not(target_family = "wasm"))]
                        self.trace_opened(trace_uri);
                        #[cfg(target_family = "wasm")]
                        let _ = trace_uri;
                    }
                    Event::OpenRecent(entry) => {
                        #[cfg(not(target_family = "wasm"))]
                        self.open_recent(entry, cx);
                        #[cfg(target_family = "wasm")]
                        let _ = entry;
                    }
                    Event::RecentChanged => {
                        #[cfg(not(target_family = "wasm"))]
                        self.save_recent();
                    }
                    Event::TraceClosed { trace_uri } => {
                        #[cfg(target_family = "wasm")]
                        crate::web::trace_closed(&trace_uri);
                        #[cfg(not(target_family = "wasm"))]
                        drop(trace_uri);
                    }
                    Event::Quit => cx.quit(),
                    Event::Changed => {
                        if let Some(dock) = &self.dock {
                            dock.invalidate_panels(cx);
                        }
                        cx.notify();
                    }
                    Event::LayoutChanged { .. } => {
                        self.panel_focus_pending = true;
                        cx.notify();
                    }
                    Event::Notice(text) => {
                        log::warn!("{text}");
                        #[cfg(not(target_family = "wasm"))]
                        if !self.app.workspace.notices.contains(&text) {
                            self.app.workspace.notices.push(text);
                            cx.notify();
                        }
                        #[cfg(target_family = "wasm")]
                        crate::web::notice(&text);
                    }
                    Event::Announce(text) => {
                        log::info!("{text}");
                        #[cfg(target_family = "wasm")]
                        crate::web::announce(&text);
                    }
                    Event::OpenFileDialog => Self::open_file_dialog(cx),
                    Event::AddTraceDialog => self.add_trace_dialog(cx),
                    Event::RevealScopeRow(ix) => self
                        .scopes_scroll
                        .scroll_to_item(ix, gpui_kit::ScrollStrategy::Nearest),
                    Event::RevealVarRow(ix) => self
                        .variables_scroll
                        .scroll_to_item(ix, gpui_kit::ScrollStrategy::Nearest),
                    Event::FocusFilter => {
                        if let Some(window) = window.as_deref_mut() {
                            let handle = self.filter.read(cx).focus_handle(cx);
                            window.focus(&handle, cx);
                        }
                    }
                    Event::WriteSettings { ticket, bytes } => {
                        #[cfg(not(target_family = "wasm"))]
                        self.write_settings(ticket, bytes);
                        #[cfg(target_family = "wasm")]
                        {
                            let error = crate::web::write_settings(&bytes).err();
                            self.app.settings_saved(ticket, error, Instant::now());
                        }
                    }
                    Event::SettingsChanged { keys } => self.settings_changed(&keys, cx),
                    Event::OpenMarkerNavigator => {
                        if let Some(window) = window.as_deref_mut() {
                            cx.defer_in(window, |ws, window, cx| {
                                ws.open_palette_with("@", window, cx);
                            });
                        }
                    }
                    Event::CopyText(text) => {
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
                    }
                    Event::FocusSettingsSearch => {
                        if let (Some(window), Some(id)) =
                            (window.as_deref_mut(), self.app.panels.settings_id())
                        {
                            let view = self.settings_view(id, window, cx);
                            view.update(cx, |view, cx| view.focus_search(window, cx));
                        }
                    }
                }
            }
        }
        self.sync_wave_menu(window.as_deref_mut(), cx);
        self.sync_rename(window, cx);
        self.sync_filter(cx);
        self.sync_menus(cx);
        self.run_requests(cx);
        #[cfg(target_family = "wasm")]
        if self.app.doc.is_loaded() {
            crate::web::viewer_state(&self.debug_state());
        }
    }

    fn sync_menus(&mut self, cx: &mut Context<Self>) {
        let key = (
            self.app.doc.generation(),
            self.app.history.revision(),
            self.app.workspace.recent.revision,
            self.app.doc.traces().revision(),
        );
        if self.embedded || self.menu_generation == Some(key) {
            return;
        }
        self.menu_generation = Some(key);
        cx.set_menus(menus(
            &pipeline_generators(&self.app),
            self.app.undo_label(),
            self.app.redo_label(),
            recent_labels(&self.app).as_deref(),
        ));
    }

    fn run_requests(&mut self, cx: &mut Context<Self>) {
        #[cfg(not(target_family = "wasm"))]
        self.native_imports.retain_current(&self.app.doc);
        #[cfg(target_family = "wasm")]
        self.sync_remote();
        for request in self.app.take_requests() {
            #[cfg(not(target_family = "wasm"))]
            if let volna_core::session::LoadRequest::Open {
                trace, generation, ..
            } = &request
                && self
                    .app
                    .doc
                    .traces()
                    .get(*trace)
                    .is_none_or(|slot| slot.generation() != *generation)
            {
                continue;
            }
            #[cfg(target_family = "wasm")]
            let Some(request) = self.route_remote(request, cx) else {
                continue;
            };
            #[cfg(not(target_family = "wasm"))]
            {
                let work = self.native_imports.prepare(request);
                cx.spawn(async move |this, cx| {
                    let result = cx.background_spawn(async move { work() }).await;
                    this.update(cx, |this, cx| {
                        this.native_imports.finish(&result);
                        this.app.deliver(result);
                        this.after(None, cx);
                    })
                    .ok();
                })
                .detach();
            }
            #[cfg(target_family = "wasm")]
            Self::spawn_request(request, cx);
        }
    }

    #[cfg(any(test, target_family = "wasm"))]
    pub(crate) fn spawn_request(request: volna_core::session::LoadRequest, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { request.perform() }).await;
            this.update(cx, |this, cx| {
                this.app.deliver(result);
                this.after(None, cx);
            })
            .ok();
        })
        .detach();
    }

    fn sync_wave_menu(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let panel = self.app.panels.focused_id();
        let Some(m) = self.app.menu() else {
            self.wave_menu = None;
            return;
        };
        if self
            .wave_menu
            .as_ref()
            .is_some_and(|menu| menu.panel == panel && menu.row == m.row && menu.kind == m.kind)
        {
            return;
        }
        let Some(window) = window else { return };
        let row = m.row;
        let generation = self.app.doc.generation();
        let position = point(px(m.position.x), px(m.position.y));
        let workspace = cx.entity().downgrade();
        let item = move |item: &volna_core::wave::MenuItem| {
            let workspace = workspace.clone();
            let id = item.action.clone();
            let label = match &item.badge {
                Some(badge) => format!("{} ({badge})", item.label),
                None => item.label.clone(),
            };
            let entry = match &item.action {
                volna_core::wave::model::MenuAction::Tint(tint) => {
                    let tint = *tint;
                    PopupMenuItem::element(move |_, cx| {
                        let t = theme(cx);
                        let swatch = t.px(10.0);
                        div()
                            .flex()
                            .items_center()
                            .gap(t.px(8.0))
                            .child(div().size(swatch).rounded(t.px(2.0)).bg(t.ink(tint)))
                            .child(label.clone())
                    })
                }
                _ => PopupMenuItem::new(label),
            };
            entry.checked(item.checked).on_click(move |_, window, cx| {
                let command = Command::MenuSelect(panel, id.clone());
                workspace
                    .update(cx, |this, cx| {
                        this.dispatch_if_current(generation, command, Some(window), cx);
                    })
                    .ok();
            })
        };
        let entries = m.entries.clone();
        let focus = self.waves_focus.clone();
        let min_w = theme(cx).px(200.0);
        let menu = PopupMenu::build(window, cx, move |mut menu, window, cx| {
            for entry in &entries {
                menu = match entry {
                    MenuEntry::Item(entry) => menu.item(item(entry)),
                    MenuEntry::Separator => menu.separator(),
                    MenuEntry::Label(label) => menu.label(label.clone()),
                    MenuEntry::Submenu { label, items } => {
                        let (items, item, focus) = (items.clone(), item.clone(), focus.clone());
                        menu.submenu(label.clone(), window, cx, move |mut sub, _, _| {
                            for entry in &items {
                                sub = sub.item(item(entry));
                            }
                            sub.action_context(focus.clone())
                        })
                    }
                };
            }
            menu.min_w(min_w).action_context(focus.clone())
        });
        cx.subscribe(&menu, move |this, _, _: &gpui_kit::DismissEvent, cx| {
            this.dispatch_if_current(generation, Command::MenuDismiss(panel), None, cx);
        })
        .detach();
        self.wave_menu = Some(HostedWaveMenu {
            panel,
            row,
            kind: m.kind,
            position,
            popup: menu,
        });
        cx.notify();
    }

    fn sync_rename(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let panel = self.app.panels.focused_id();
        let want = self.app.text_edit();
        if self.rename.as_ref().map(|h| h.target) == want.as_ref().map(|e| e.target) {
            return;
        }
        let Some(window) = window else {
            if self.rename.take().is_some() {
                self.panel_focus_pending = true;
                cx.notify();
            }
            return;
        };
        if self.rename.take().is_some() {
            let focus = self
                .dock
                .as_ref()
                .and_then(|dock| dock.focus(panel, cx))
                .unwrap_or_else(|| self.waves_focus.clone());
            window.focus(&focus, cx);
            cx.notify();
        }
        let Some(edit) = want else { return };
        let accent = match edit.target {
            volna_core::app::EditTarget::Marker { id, .. } => Some(crate::theme::hsla(
                crate::theme::canvas_theme(cx)
                    .marker(id.palette_index())
                    .background
                    .with_alpha(1.0),
            )),
            volna_core::app::EditTarget::Group { .. } => None,
        };
        let input = cx.new(|cx| {
            let input = TextInput::new(edit.label.clone(), cx).plain();
            let mut input = match accent {
                Some(accent) => input.accent(accent),
                None => input,
            };
            input.set_text(edit.text.clone(), cx);
            if edit.select_all {
                input.select_all(cx);
            }
            input
        });
        let generation = self.app.doc.generation();
        let target = edit.target;
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &TextInputEvent, window, cx| {
                let text = match event {
                    TextInputEvent::Submit => Some(input.read(cx).text().to_owned()),
                    TextInputEvent::Cancel => None,
                    TextInputEvent::Changed => return,
                };
                this.dispatch_if_current(
                    generation,
                    Command::CommitText(target, text),
                    Some(window),
                    cx,
                );
            },
        );
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.rename = Some(HostedRename {
            target,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(crate) fn focus_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(h) = &self.rename {
            window.focus(&h.input.read(cx).focus_handle(cx), cx);
        }
    }

    pub(crate) fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(h) = &self.rename {
            let command = Command::CommitText(h.target, Some(h.input.read(cx).text().to_owned()));
            self.dispatch(command, Some(window), cx);
        }
    }

    fn sync_filter(&mut self, cx: &mut Context<Self>) {
        let text = self.app.variables.filter.clone();
        if self.filter.read(cx).text() != text {
            self.filter.update(cx, |f, cx| f.set_text(text, cx));
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn open_path(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        if path.to_string_lossy().ends_with(".volna.json") {
            self.open_workspace_path(path, cx);
            return;
        }
        self.opening_workspace = None;
        if !self.cli_policy
            && !self.embedded
            && matches!(
                self.app.workspace.scheduler.policy,
                volna_core::workspace::persistence::Persistence::Explicit(_)
            )
        {
            let policy = self.autosave_policy();
            self.app.configure_persistence(policy);
        }
        match path
            .canonicalize()
            .map_err(anyhow::Error::from)
            .and_then(|path| Ok((crate::native_workspace::file_uri(&path)?, path)))
        {
            Ok((uri, path)) => self
                .app
                .open_resource(volna_trace::session::OpenSpec::Path(path), uri),
            Err(_) => self.app.open_path(path),
        }
        self.after(None, cx);
    }

    pub fn open_bytes(&mut self, name: String, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.app.open_bytes(name, bytes);
        self.after(None, cx);
    }

    pub fn set_session(&mut self, session: std::sync::Arc<dyn Session>, cx: &mut Context<Self>) {
        self.app.set_session(session);
        self.after(None, cx);
    }

    fn open_file_dialog(cx: &mut Context<Self>) {
        #[cfg(not(target_family = "wasm"))]
        {
            let rx = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Open".into()),
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(paths))) = rx.await
                    && let Some(path) = paths.into_iter().next()
                {
                    this.update(cx, |this, cx| this.open_path(path, cx)).ok();
                }
            })
            .detach();
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = cx;
            crate::web::request_open_dialog();
        }
    }

    fn open_file(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Command::RequestOpenDialog, Some(window), cx);
    }

    fn add_trace(&mut self, _: &AddTrace, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Command::RequestAddTraceDialog, Some(window), cx);
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Command::ToggleSidebar, Some(window), cx);
    }

    fn close_trace(&mut self, _: &CloseTrace, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Command::CloseTrace, Some(window), cx);
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.embedded {
            #[cfg(target_family = "wasm")]
            crate::web::open_host_settings();
            return;
        }
        self.dispatch(Command::Settings(SettingsCommand::Open), Some(window), cx);
    }

    fn command_palette(&mut self, _: &CommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(window, cx);
    }

    pub(crate) fn settings_view(
        &mut self,
        id: volna_core::panels::PanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<crate::settings_panel::SettingsPanelView> {
        let mut dock = self
            .dock
            .take()
            .unwrap_or_else(|| crate::dock::DockHost::new(window, cx));
        let view = dock.settings_view(id, cx.weak_entity(), window, cx);
        self.dock = Some(dock);
        view
    }

    fn settings_changed(&mut self, keys: &[&'static str], cx: &mut Context<Self>) {
        if keys.contains(&"appearance.theme") {
            self.apply_theme_setting(cx);
        }
        if keys.contains(&"appearance.zoom") {
            let zoom = self.app.settings.resolved().appearance.zoom as f32;
            log::debug!("applying interface zoom {zoom}");
            crate::theme::set_zoom(zoom, cx);
        }
        if keys.contains(&"workspace.autosave") && !self.embedded && !self.cli_policy {
            let policy = self.autosave_policy();
            self.app.configure_persistence(policy);
        }
        cx.notify();
    }

    fn autosave_policy(&self) -> volna_core::workspace::persistence::Persistence {
        use volna_core::settings::Autosave;
        use volna_core::workspace::persistence::Persistence;
        match self.app.settings.resolved().workspace.autosave {
            Autosave::Off => Persistence::Disabled,
            _ => Persistence::Auto,
        }
    }

    pub(crate) fn set_system_dark(&mut self, dark: bool, cx: &mut Context<Self>) {
        if self.system_dark != dark {
            self.system_dark = dark;
            self.apply_theme_setting(cx);
        }
    }

    pub(crate) fn apply_theme_setting(&mut self, cx: &mut Context<Self>) {
        if self.embedded {
            return;
        }
        let name = self.app.settings.resolved().appearance.theme.clone();
        let dark = self.system_dark;
        log::debug!("applying theme setting {name:?} (system dark: {dark})");
        #[cfg(not(target_family = "wasm"))]
        let themes = match &self.native_store {
            Some(store) => store.theme(&name, dark),
            None => volna_core::theme::builtin::resolve(&name, dark)
                .ok_or_else(|| anyhow::anyhow!("no theme directory")),
        };
        #[cfg(target_family = "wasm")]
        let themes = volna_core::theme::builtin::resolve(&name, dark)
            .ok_or_else(|| anyhow::anyhow!("palette files are not available in the browser"));
        match themes {
            Ok(themes) => crate::theme::install_themes(themes, cx),
            Err(error) => {
                self.app
                    .report_workspace_error(format!("Theme '{name}': {error:#}; using Volna"));
                let fallback =
                    volna_core::theme::builtin::resolve(volna_core::theme::builtin::VOLNA, dark)
                        .expect("volna is built in");
                crate::theme::install_themes(fallback, cx);
            }
        }
    }

    pub(crate) fn open_memory_menu(
        &mut self,
        position: gpui_kit::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let model = self.app.memory_menu();
        let workspace = cx.entity().downgrade();
        let run = move |command: Command| {
            let workspace = workspace.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                let command = command.clone();
                workspace
                    .update(cx, |this, cx| {
                        this.status_menu = None;
                        this.dispatch(command, Some(window), cx);
                    })
                    .ok();
            }
        };
        let focus = self.focus_handle.clone();
        let min_w = theme(cx).px(200.0);
        let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
            let presets = |choices: Vec<volna_core::app::MemoryChoice>,
                           note: Option<&'static str>| {
                let (run, locked, focus) = (run.clone(), model.locked.is_some(), focus.clone());
                move |mut sub: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                    for choice in &choices {
                        sub = sub.item(
                            PopupMenuItem::new(choice.label.clone())
                                .checked(choice.checked)
                                .disabled(locked)
                                .on_click(run(choice.command())),
                        );
                    }
                    if let Some(note) = note {
                        sub = sub.separator().item(PopupMenuItem::label(note));
                    }
                    sub.action_context(focus.clone())
                }
            };
            let mut menu = menu
                .submenu(
                    model.budget_title.clone(),
                    window,
                    cx,
                    presets(model.budget.clone(), None),
                )
                .submenu(
                    model.object_title.clone(),
                    window,
                    cx,
                    presets(model.object.clone(), model.object_note),
                )
                .separator();
            if let Some(locked) = model.locked {
                menu = menu.item(PopupMenuItem::label(locked));
            }
            menu.item(
                PopupMenuItem::new("Memory Settings…").on_click(run(Command::Settings(
                    SettingsCommand::Reveal {
                        id: "memory".into(),
                    },
                ))),
            )
            .min_w(min_w)
            .action_context(focus.clone())
        });
        cx.subscribe(&menu, move |this, _, _: &gpui_kit::DismissEvent, cx| {
            this.status_menu = None;
            cx.notify();
        })
        .detach();
        self.status_menu = Some((position, menu));
        cx.notify();
    }

    #[must_use]
    pub fn debug_state(&self) -> String {
        self.app.debug_state()
    }

    fn render_drag_surface(drag: ChromeDrag, cx: &mut Context<Self>) -> impl IntoElement {
        let cursor = match drag {
            ChromeDrag::Sidebar => CursorStyle::ResizeLeftRight,
            ChromeDrag::ScopesSplit => CursorStyle::ResizeUpDown,
        };
        gpui_kit::deferred(
            div()
                .id("drag-surface")
                .absolute()
                .inset_0()
                .occlude()
                .cursor(cursor)
                .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, window, cx| {
                    let command = match drag {
                        ChromeDrag::Sidebar => {
                            Command::SetSidebarWidth(f32::from(ev.position.x) / theme(cx).zoom)
                        }
                        ChromeDrag::ScopesSplit => {
                            let t = theme(cx);
                            let top = if this.embedded {
                                0.0
                            } else {
                                t.titlebar_height
                            };
                            let total = (f32::from(window.viewport_size().height)
                                - top
                                - t.statusbar_height)
                                .max(1.0);
                            Command::SetScopesFraction((f32::from(ev.position.y) - top) / total)
                        }
                    };
                    this.dispatch(command, Some(window), cx);
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, window, cx| {
                        this.dispatch(Command::ChromeDragEnd, Some(window), cx);
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, window, cx| {
                        this.dispatch(Command::ChromeDragEnd, Some(window), cx);
                    }),
                ),
        )
        .with_priority(100)
    }

    fn render_titlebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme(cx);
        let colors = t.bar;
        let chips = self.render_trace_chips(cx);
        let file: Option<SharedString> = chips
            .is_none()
            .then(|| self.app.doc.name())
            .flatten()
            .map(Into::into);
        let fullscreen = window.is_fullscreen();
        let left_controls = render_window_controls(Side::Left, window, cx);
        let right_controls = render_window_controls(Side::Right, window, cx);
        let is_macos = cfg!(target_os = "macos");
        let is_windows = cfg!(target_os = "windows");
        let supported = window.window_controls();
        let window_menu = app_draws_controls(window) && !is_windows && supported.window_menu;
        let zoom_on_double_click = is_macos || (supported.maximize && window.is_resizable());
        let left_pad = if fullscreen || left_controls.is_some() {
            t.px(8.0)
        } else if is_macos {
            t.px(80.0)
        } else {
            t.px(12.0)
        };
        div()
            .id("titlebar")
            .flex()
            .flex_none()
            .items_center()
            .h(px(t.titlebar_height))
            .w_full()
            .pl(left_pad)
            .when(right_controls.is_none(), gpui_kit::Styled::pr_2)
            .gap_2()
            .bg(crate::theme::chrome_background(cx))
            .border_b_1()
            .border_color(crate::theme::chrome_border(cx))
            .font_family(t.ui_font)
            .text_size(px(t.ui_size))
            .when_some(left_controls, gpui_kit::ParentElement::child)
            .child(
                div()
                    .id("titlebar-drag")
                    .flex()
                    .flex_1()
                    .h_full()
                    .items_center()
                    .gap_2()
                    .window_control_area(gpui_kit::WindowControlArea::Drag)
                    .when(!is_windows, |el| {
                        el.on_mouse_down(MouseButton::Left, move |ev, window, _| {
                            if ev.click_count == 2 {
                                if is_macos {
                                    window.titlebar_double_click();
                                } else if zoom_on_double_click {
                                    window.zoom_window();
                                }
                            } else {
                                window.start_window_move();
                            }
                        })
                    })
                    .when(window_menu, |el| {
                        el.on_mouse_down(MouseButton::Right, |ev, window, _| {
                            window.show_window_menu(ev.position);
                        })
                    })
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(colors.text)
                            .child("Volna"),
                    )
                    .when_some(file, |el, name| {
                        el.child(div().text_color(colors.text_placeholder).child("—"))
                            .child(div().text_color(colors.text_muted).child(name))
                    })
                    .when_some(chips, gpui_kit::ParentElement::child),
            )
            .child(
                icon_button(
                    "toggle-sidebar",
                    IconName::PanelLeft,
                    t.bar,
                    t.bar_hover,
                    cx,
                )
                .selected(self.app.sidebar_visible)
                .tooltip("Toggle sidebar (⌘B)")
                .on_click(cx.listener(|this, _, w, cx| {
                    this.dispatch(Command::ToggleSidebar, Some(w), cx);
                })),
            )
            .when_some(self.app.panels.layout().right.as_ref(), |el, dock| {
                el.child(
                    icon_button(
                        "toggle-right-sidebar",
                        IconName::PanelRight,
                        t.bar,
                        t.bar_hover,
                        cx,
                    )
                    .selected(dock.open)
                    .tooltip("Toggle right sidebar")
                    .debug_selector(|| "toggle-right-sidebar".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.dispatch(
                            Command::Panels(volna_core::panels::PanelsCommand::ToggleRightDock),
                            Some(window),
                            cx,
                        );
                    })),
                )
            })
            .child(
                icon_button("open-file", IconName::FolderOpen, t.bar, t.bar_hover, cx)
                    .tooltip("Open trace (⌘O)")
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.dispatch(Command::RequestOpenDialog, Some(w), cx);
                    })),
            )
            .when(self.app.doc.is_loaded(), |el| {
                el.child(
                    icon_button("add-trace", IconName::Plus, t.bar, t.bar_hover, cx)
                        .tooltip("Add trace beside the open ones (⇧⌘O)")
                        .on_click(cx.listener(|this, _, w, cx| {
                            this.dispatch(Command::RequestAddTraceDialog, Some(w), cx);
                        })),
                )
            })
            .child(
                icon_button("open-settings", IconName::Settings, t.bar, t.bar_hover, cx)
                    .selected(self.app.panels.settings_id().is_some())
                    .tooltip("Settings (⌘,)")
                    .on_click(
                        cx.listener(|this, _, w, cx| this.open_settings(&OpenSettings, w, cx)),
                    ),
            )
            .when_some(right_controls, gpui_kit::ParentElement::child)
    }

    fn render_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme(cx);
        let frac = self.app.scopes_fraction;
        let scopes = self.render_scopes(window, cx).into_any_element();
        let variables = self.render_variables(window, cx).into_any_element();
        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .h_full()
            .map(|el| {
                if t.islands.is_some() {
                    el.w_full()
                } else {
                    el.w(t.px(self.app.sidebar_width))
                }
            })
            .bg(t.panel.bg)
            .child(
                div()
                    .flex_none()
                    .h(gpui_kit::relative(frac))
                    .min_h(t.px(96.0))
                    .overflow_hidden()
                    .child(scopes),
            )
            .child(
                Splitter::new("scopes-split", SplitterAxis::Horizontal)
                    .dragging(self.app.drag == Some(ChromeDrag::ScopesSplit))
                    .on_drag_start(cx.listener(|this, _, window, cx| {
                        this.dispatch(
                            Command::ChromeDragStart(ChromeDrag::ScopesSplit),
                            Some(window),
                            cx,
                        );
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(t.px(96.0))
                    .overflow_hidden()
                    .child(variables),
            )
    }

    fn render_center(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let t = *theme(cx);
        let colors = t.editor;
        let settings_only = match self.app.trace_state() {
            TraceState::Loaded(_) | TraceState::Loading { .. } => None,
            _ => self.app.panels.settings_id(),
        };
        if let Some(id) = settings_only {
            let view = self.settings_view(id, window, cx);
            return div().size_full().child(view).into_any_element();
        }
        match self.app.trace_state() {
            TraceState::Loaded(_) => self.render_loaded(window, cx),
            TraceState::Loading { name } => {
                #[cfg(not(target_family = "wasm"))]
                let progress = self.render_import_progress(TraceId::A, false, cx);
                #[cfg(target_family = "wasm")]
                let progress: Option<gpui_kit::AnyElement> = None;
                div()
                    .id("trace-loading")
                    .debug_selector(|| "trace-loading".into())
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .bg(t.editor.bg)
                    .child(
                        icon_svg(IconName::LoaderCircle, t.px(28.0), colors.icon_accent)
                            .with_animation(
                                "spinner",
                                Animation::new(Duration::from_millis(900)).repeat(),
                                |svg, delta| {
                                    svg.with_transformation(Transformation::rotate(percentage(
                                        delta,
                                    )))
                                },
                            ),
                    )
                    .child(
                        div()
                            .font_family(t.ui_font)
                            .text_size(px(t.ui_size))
                            .text_color(colors.text_muted)
                            .id("trace-loading-message")
                            .debug_selector(|| "trace-loading-message".into())
                            .max_w_full()
                            .min_w_0()
                            .px_4()
                            .whitespace_normal()
                            .text_center()
                            .child(SharedString::from(self.loading_message(&name))),
                    )
                    .children(progress)
                    .into_any_element()
            }
            TraceState::Empty | TraceState::Error(_) => self.render_empty(cx).into_any_element(),
        }
    }

    fn render_loaded(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let dock = self.render_dock(window, cx).into_any_element();
        #[cfg(not(target_family = "wasm"))]
        {
            let active = self.native_imports.active();
            if !active.is_empty() {
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("conversion-banners")
                            .flex_none()
                            .w_full()
                            .max_h(gpui_kit::relative(0.4))
                            .overflow_y_scroll()
                            .children(
                                active.into_iter().filter_map(|trace| {
                                    self.render_import_progress(trace, true, cx)
                                }),
                            ),
                    )
                    .child(div().flex_1().min_h_0().child(dock))
                    .into_any_element();
            }
        }
        dock
    }

    fn loading_message(&self, name: &str) -> String {
        #[cfg(not(target_family = "wasm"))]
        if self
            .native_imports
            .converting(volna_core::trace::TraceId::A)
        {
            return format!("Converting {name} to VTR…");
        }
        format!("Loading {name}…")
    }

    fn render_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let el = div().id("wave-view").size_full().relative();
        let mut dock = self
            .dock
            .take()
            .unwrap_or_else(|| crate::dock::DockHost::new(window, cx));
        dock.sync(&self.app, cx.weak_entity(), window, cx);
        if let Some(focus) = dock.focus(self.app.panels.focused_id(), cx) {
            if self.panel_focus_pending {
                window.focus(&focus, cx);
                self.panel_focus_pending = false;
            }
            self.waves_focus = focus;
        }
        let area = dock.area.clone();
        self.dock = Some(dock);
        el.child(area)
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme(cx);
        let colors = t.editor;
        let error: Option<SharedString> = match self.app.trace_state() {
            TraceState::Error(e) => Some(e.into()),
            _ => None,
        };
        let badge = |keys: &'static str| {
            div()
                .px_1p5()
                .py_0p5()
                .rounded_sm()
                .bg(t.badge_hover.bg)
                .font_family(t.mono_font)
                .text_size(px(t.ui_size_small))
                .text_color(t.badge_hover.text)
                .child(keys)
        };
        let hint = |keys: &'static str, label: &'static str| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .w(t.px(260.0))
                .child(div().text_color(colors.text_muted).child(label))
                .child(badge(keys))
        };
        let shown = self.app.recent_shown();
        let rows = self.app.recent_rows(unix_now(), home_dir().as_deref());
        let total = rows.len();
        let selected = self.app.workspace.recent.selected;
        let recent = rows.into_iter().take(shown).enumerate().map(|(ix, row)| {
            let selected = ix == selected;
            let detail = match &row.trace {
                Some(trace) => format!("{trace} · {}", row.folder),
                None => row.folder.clone(),
            };
            let (icon, icon_color) = match row.kind {
                RecentKind::Trace => (IconName::AudioWaveform, colors.icon_muted),
                RecentKind::Workspace => (IconName::Layers, colors.icon_accent),
            };
            let group: SharedString = format!("recent-{ix}").into();
            let path: SharedString = row.path.into();
            div()
                .id(("recent", ix))
                .debug_selector(move || format!("recent-{ix}"))
                .group(group.clone())
                .flex()
                .items_center()
                .gap_2()
                .w_full()
                .h(t.px(28.0))
                .pl_1()
                .pr_0p5()
                .rounded_md()
                .cursor_pointer()
                .when(selected, |el| el.bg(t.selection.bg))
                .when(!selected, |el| el.hover(|s| s.bg(t.hover.bg)))
                .tooltip(move |w, cx| Tooltip::new(path.clone()).build(w, cx))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Command::Recent(RecentCommand::Open(ix)), Some(window), cx);
                }))
                .child(
                    div()
                        .w(t.px(10.0))
                        .flex_none()
                        .font_family(t.mono_font)
                        .text_size(px(t.ui_size_small))
                        .text_color(colors.text_placeholder)
                        .when(ix < 9, |el| {
                            el.child(SharedString::from((ix + 1).to_string()))
                        }),
                )
                .child(
                    Icon::new(icon)
                        .size(t.px(14.0))
                        .color(icon_color)
                        .when(row.missing, |i| i.color(colors.text_placeholder)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(if row.missing {
                            colors.text_placeholder
                        } else {
                            colors.text
                        })
                        .child(SharedString::from(row.name))
                        .child(
                            div()
                                .ml_2()
                                .text_size(px(t.ui_size_small))
                                .text_color(if row.missing {
                                    colors.text_placeholder
                                } else {
                                    colors.text_muted
                                })
                                .child(SharedString::from(detail)),
                        )
                        .flex()
                        .items_baseline(),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(t.ui_size_small))
                        .text_color(if row.missing {
                            colors.error
                        } else {
                            colors.text_muted
                        })
                        .child(SharedString::from(row.when)),
                )
                .child(
                    div()
                        .flex_none()
                        .when(!selected, |el| {
                            el.opacity(0.0).group_hover(group, |s| s.opacity(1.0))
                        })
                        .child(
                            icon_button(("recent-remove", ix), IconName::X, colors, t.hover, cx)
                                .tooltip("Remove from Recent (Delete)")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.dispatch(
                                        Command::Recent(RecentCommand::Forget(ix)),
                                        Some(window),
                                        cx,
                                    );
                                })),
                        ),
                )
        });
        let recent: Vec<_> = recent.collect();
        let has_recent = !recent.is_empty();
        div()
            .id("start-page")
            .key_context("StartPage")
            .track_focus(&self.waves_focus)
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    let ks = &event.keystroke;
                    let m = ks.modifiers;
                    if m.control || m.alt || m.platform || m.function {
                        return;
                    }
                    let key = match ks.key.as_str() {
                        "up" => RecentKey::Up,
                        "down" => RecentKey::Down,
                        "home" => RecentKey::Home,
                        "end" => RecentKey::End,
                        "enter" => RecentKey::Enter,
                        "delete" | "backspace" => RecentKey::Delete,
                        k if k.len() == 1 && !m.shift => match k.as_bytes()[0] {
                            d @ b'1'..=b'9' => RecentKey::Digit(d - b'0'),
                            _ => return,
                        },
                        _ => return,
                    };
                    if this.app.recent_shown() == 0 {
                        return;
                    }
                    cx.stop_propagation();
                    this.dispatch(Command::Recent(RecentCommand::Key(key)), Some(window), cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| window.focus(&this.waves_focus, cx)),
            )
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .px_4()
            .bg(t.editor.bg)
            .font_family(t.ui_font)
            .text_size(px(t.ui_size))
            .child(crate::ui::branding::seal(
                t.px(96.0),
                colors.text_placeholder,
            ))
            .child(
                div()
                    .mt_2()
                    .text_size(t.px(16.0))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child("No trace open"),
            )
            .child(
                div()
                    .text_color(colors.text_muted)
                    .max_w_full()
                    .whitespace_normal()
                    .text_center()
                    .child(
                        "Open a VTR, FST, or Kanata trace to browse its signals and transactions",
                    ),
            )
            .when_some(error, |el, e| {
                let summary = e.lines().next().unwrap_or_default().to_owned();
                el.child(
                    div()
                        .id("trace-error")
                        .mt_1()
                        .w_full()
                        .max_w_full()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(t.panel.bg)
                        .border_1()
                        .border_color(t.panel.error)
                        .text_color(t.panel.error)
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .size(t.px(14.0))
                                .color(t.panel.error),
                        )
                        .child(
                            div().flex_1().min_w_0().whitespace_normal().child(
                                div()
                                    .line_clamp(3)
                                    .child(gpui_kit::base::SelectableText::new(
                                        "trace-error-text",
                                        summary,
                                    )),
                            ),
                        )
                        .child(
                            Button::new("trace-error-details")
                                .label("Details")
                                .on_click(move |_, window, cx| {
                                    open_message_dialog("Trace error", e.clone(), window, cx);
                                }),
                        ),
                )
            })
            .child(
                div().mt_3().flex().gap_2().child(
                    Button::new("open")
                        .label("Open File…")
                        .icon(gpui_kit::component::Icon::empty().path(IconName::FolderOpen.path()))
                        .primary()
                        .on_click(cx.listener(|this, _, w, cx| {
                            this.dispatch(Command::RequestOpenDialog, Some(w), cx);
                        })),
                ),
            )
            .when(has_recent, |el| {
                el.child(
                    div()
                        .id("recent-list")
                        .mt_4()
                        .w_full()
                        .max_w(t.px(480.0))
                        .max_h(t.px(320.0))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_size(px(t.ui_size_small))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(colors.text_muted)
                                .child("Recent"),
                        )
                        .children(recent),
                )
                .when(total > volna_core::workspace::recent::SHOWN, |el| {
                    el.child(
                        Button::new("recent-all")
                            .label(if shown < total {
                                format!("Show all {total}")
                            } else {
                                "Show fewer".into()
                            })
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dispatch(
                                    Command::Recent(RecentCommand::ToggleAll),
                                    Some(window),
                                    cx,
                                );
                            })),
                    )
                })
                .child(
                    div()
                        .mt_2()
                        .flex()
                        .flex_wrap()
                        .justify_center()
                        .items_center()
                        .gap_1()
                        .text_size(px(t.ui_size_small))
                        .text_color(colors.text_placeholder)
                        .child(badge("⏎"))
                        .child("open ·")
                        .child(badge("↑↓"))
                        .child("select ·")
                        .child(badge("1–9"))
                        .child("open by number ·")
                        .child(badge("Del"))
                        .child("remove ·")
                        .child(badge("⌘O"))
                        .child("browse"),
                )
            })
            .when(!has_recent, |el| {
                el.child(
                    div()
                        .mt_6()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .text_size(px(t.ui_size_small))
                        .child(hint("⌘O", "Open a trace"))
                        .child(hint("⏎", "Add selected variables"))
                        .child(hint("= / -", "Zoom in / out"))
                        .child(hint("F", "Zoom to fit"))
                        .child(hint("M", "Add marker at cursor"))
                        .child(hint("T", "Cycle value format")),
                )
            })
    }

    fn render_statusbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme(cx);
        let colors = t.bar;
        let status = self.app.status();
        let mono = |text: String, color: gpui_kit::Hsla| {
            div()
                .flex_none()
                .font_family(t.mono_font)
                .text_size(px(t.ui_size_small))
                .text_color(color)
                .child(SharedString::from(text))
        };
        let mono_w = |chars: f32| px(t.ui_size_small * 0.62 * chars);
        let sep = || div().flex_none().w(px(1.0)).h(t.px(12.0)).bg(t.border);
        let group = || div().flex().flex_none().items_center().gap_3();

        let mut context = group();
        if let Some(panel) = status.panel {
            context = context.child(mono(panel, colors.text));
        }
        if let Some(range) = status.time_range {
            context = context.child(mono(range, colors.text_muted));
        }
        if let Some(s) = status.signals {
            context = context.child(mono(s, colors.text_placeholder));
        }
        if let Some(s) = status.changes {
            context = context.child(mono(s, colors.text_placeholder));
        }
        let mut position = group();
        let has_position = status.cursor.is_some() || status.markers.is_some();
        if let Some(c) = status.cursor {
            position = position.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .child(
                        Icon::new(IconName::Locate)
                            .size(t.px(12.0))
                            .color(colors.icon_accent),
                    )
                    .child(mono(c, colors.text)),
            );
        }
        for c in status.clocks {
            position = position.child(mono(c, colors.text_muted));
        }
        if let Some(d) = status.measure {
            position = position.child(mono(d, colors.text));
        }
        if let Some(m) = status.markers {
            position = position.child(mono(m, colors.text_placeholder));
        }
        let mut left = div()
            .debug_selector(|| "status-left".into())
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .overflow_hidden()
            .child(context);
        if has_position {
            left = left.child(sep()).child(position);
        }

        let crop = |text: String, color: gpui_kit::Hsla, mono_text: bool| {
            div()
                .min_w_0()
                .truncate()
                .when(mono_text, |d| d.font_family(t.mono_font))
                .text_size(px(t.ui_size_small))
                .text_color(color)
                .child(SharedString::from(text))
        };
        let has_message = status.hover.is_some()
            || status.sidebar_notice.is_some()
            || status.workspace_notice.is_some()
            || status.announcement.is_some();
        let mut message = div()
            .debug_selector(|| "status-message".into())
            .flex()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .gap_3();
        if has_message {
            message = message.child(sep());
        }
        if let Some(notice) = status.workspace_notice {
            let mut details = self.app.workspace.notices.clone();
            if !details.contains(&notice) {
                details.push(notice.clone());
            }
            let details = details.join("\n\n");
            message = message.child(
                div()
                    .id("workspace-notice")
                    .debug_selector(|| "workspace-notice".into())
                    .flex_none()
                    .cursor(CursorStyle::PointingHand)
                    .text_color(t.editor.error)
                    .text_size(px(t.ui_size_small))
                    .child("Workspace notice")
                    .tooltip(move |w, cx| Tooltip::new(notice.clone()).build(w, cx))
                    .on_click(move |_, window, cx| {
                        open_message_dialog("Workspace details", details.clone(), window, cx);
                    }),
            );
        }
        let notice_details = |id: &'static str, text: String| {
            let details = text.clone();
            div()
                .id(id)
                .debug_selector(move || id.into())
                .min_w_0()
                .cursor(CursorStyle::PointingHand)
                .child(crop(text, colors.text_muted, false))
                .tooltip(|w, cx| Tooltip::new("Click for message details").build(w, cx))
                .on_click(move |_, window, cx| {
                    open_message_dialog("Message details", details.clone(), window, cx);
                })
        };
        if let Some(hover) = status.hover {
            message = message.child(crop(hover, colors.text_muted, true));
        }
        if let Some(notice) = status.sidebar_notice {
            message = message.child(notice_details("sidebar-notice", notice));
        }
        if let Some(announcement) = status.announcement {
            message = message.child(notice_details("announcement", announcement));
        }

        let has_nav = status.px_per.is_some();
        let has_meters = status.memory.is_some() || status.frames.is_some();
        let mut nav = group();
        if let Some(s) = status.px_per {
            nav = nav.child(mono(s, colors.text_placeholder).min_w(mono_w(16.0)));
        }
        let mut meters = group();
        if let Some(memory) = status.memory {
            meters = meters.child(memory_meter(memory, &t).on_click(cx.listener(
                |this, ev: &gpui_kit::ClickEvent, window, cx| {
                    let p = ev.position();
                    let t = theme(cx);
                    this.open_memory_menu(point(p.x - t.px(100.0), p.y - t.px(96.0)), window, cx);
                },
            )));
        }
        if let Some(frames) = status.frames {
            meters = meters.child(
                div()
                    .flex_none()
                    .min_w(mono_w(22.0))
                    .child(self.render_frame_status(frames, cx)),
            );
        }
        let right = div()
            .debug_selector(|| "status-right".into())
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .when(has_nav, |d| d.child(nav))
            .when(has_nav && has_meters, |d| d.child(sep()))
            .when(has_meters, |d| d.child(meters));
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h(px(t.statusbar_height))
            .w_full()
            .px_2()
            .overflow_hidden()
            .bg(crate::theme::chrome_background(cx))
            .border_t_1()
            .border_color(crate::theme::chrome_border(cx))
            .font_family(t.ui_font)
            .when(t.islands.is_some(), |el| {
                el.border_t_0()
                    .px(t.px(12.0))
                    .py(t.px(4.0))
                    .line_height(t.px(16.0))
            })
            .child(left)
            .child(message)
            .child(right)
    }
}

impl Workspace {
    fn render_rename(&self) -> Option<impl IntoElement> {
        let hosted = self.rename.as_ref()?;
        let rect = self
            .app
            .text_edit()
            .filter(|e| e.target == hosted.target)?
            .rect?;
        let id = match hosted.target {
            volna_core::app::EditTarget::Group { .. } => "group-rename",
            volna_core::app::EditTarget::Marker { .. } => "marker-name",
        };
        Some(
            deferred(
                anchored()
                    .position(point(px(rect.left()), px(rect.top())))
                    .child(
                        div()
                            .id(id)
                            .debug_selector(move || id.into())
                            .w(px(rect.width()))
                            .h(px(rect.height()))
                            .child(hosted.input.clone()),
                    ),
            )
            .with_priority(5),
        )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_wave_menu(Some(window), cx);
        let was_animating = self.app.is_animating();
        if self.app.tick(Instant::now()) {
            window.request_animation_frame();
        }
        self.after(Some(window), cx);
        #[cfg(not(target_family = "wasm"))]
        if !self.native_imports.active().is_empty() {
            window.request_animation_frame();
        }
        if was_animating && let Some(dock) = &self.dock {
            dock.invalidate_panels(cx);
        }
        let t = *theme(cx);
        let colors = t.editor;
        let drag = self.app.drag;
        let sidebar_visible = self.app.sidebar_visible;
        let mut root = div()
            .id("workspace")
            .key_context(if self.embedded {
                "Workspace Embedded"
            } else {
                "Workspace"
            })
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(t.editor.bg)
            .text_color(colors.text)
            .font_family(t.ui_font)
            .text_size(px(t.ui_size))
            .on_action(cx.listener(Self::open_file))
            .on_action(cx.listener(Self::add_trace))
            .on_action(cx.listener(|this, _: &OpenWorkspace, window, cx| {
                this.dispatch(Command::RequestOpenWorkspace, Some(window), cx);
            }))
            .on_action(cx.listener(|this, _: &SaveWorkspace, window, cx| {
                this.dispatch(Command::SaveWorkspace, Some(window), cx);
            }))
            .on_action(cx.listener(|this, _: &SaveWorkspaceAs, window, cx| {
                this.dispatch(Command::RequestSaveWorkspaceAs, Some(window), cx);
            }))
            .on_action(cx.listener(|this, _: &Quit, window, cx| {
                this.dispatch(Command::RequestQuit, Some(window), cx);
            }))
            .on_action(cx.listener(|this, _: &Undo, window, cx| {
                this.dispatch(Command::Undo, Some(window), cx);
            }))
            .on_action(cx.listener(|this, _: &Redo, window, cx| {
                this.dispatch(Command::Redo, Some(window), cx);
            }))
            .on_action(cx.listener(|this, action: &OpenRecent, window, cx| {
                this.dispatch(
                    Command::Recent(RecentCommand::Open(action.ix)),
                    Some(window),
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &ClearRecent, window, cx| {
                this.dispatch(Command::Recent(RecentCommand::Clear), Some(window), cx);
            }))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::close_trace))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::command_palette))
            .on_action(cx.listener(|this, action: &GoToMarker, window, cx| {
                if let Some(id) = volna_core::marker::MarkerId::new(action.n) {
                    this.dispatch(Command::Action(Action::GoToMarker(id)), Some(window), cx);
                }
            }))
            .on_action(cx.listener(|this, action: &SetMarker, window, cx| {
                if let Some(id) = volna_core::marker::MarkerId::new(action.n) {
                    this.dispatch(Command::Action(Action::SetMarker(id)), Some(window), cx);
                }
            }))
            .on_action(cx.listener(|this, action: &SetTint, window, cx| {
                this.dispatch(
                    Command::Action(Action::SetTint(action.tint)),
                    Some(window),
                    cx,
                );
            }))
            .on_action(cx.listener(|this, action: &ClockAction, window, cx| {
                this.dispatch(Command::Clocks(action.command.clone()), Some(window), cx);
            }))
            .on_action(cx.listener(|this, action: &TraceAction, window, cx| {
                let trace = action.trace;
                match action.verb {
                    TraceVerb::Reveal => {
                        this.dispatch(Command::RevealTrace(trace), Some(window), cx);
                    }
                    TraceVerb::Rename => this.start_trace_rename(trace, window, cx),
                    TraceVerb::Close => {
                        this.dispatch(Command::RemoveTrace(trace), Some(window), cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, action: &OpenPipelineTrack, window, cx| {
                this.dispatch(
                    Command::OpenPipeline {
                        track: action.track,
                    },
                    Some(window),
                    cx,
                );
            }))
            .on_action(cx.listener(
                |this, _: &crate::settings_panel::ToggleSettingsJson, window, cx| {
                    if this.embedded {
                        return;
                    }
                    if this.app.panels.settings_id().is_none() {
                        this.dispatch(Command::Settings(SettingsCommand::Open), Some(window), cx);
                    }
                    if !this.app.settings_view.json {
                        this.dispatch(
                            Command::Settings(SettingsCommand::ToggleJson),
                            Some(window),
                            cx,
                        );
                    }
                },
            ));
        root = focus_panel_actions!(
            root,
            cx,
            [
                (FocusPanel1, 0),
                (FocusPanel2, 1),
                (FocusPanel3, 2),
                (FocusPanel4, 3),
                (FocusPanel5, 4),
                (FocusPanel6, 5),
                (FocusPanel7, 6),
                (FocusPanel8, 7),
                (FocusPanel9, 8)
            ]
        );
        root = root
            .on_action(cx.listener(|this, _: &UiZoomIn, window, cx| {
                this.dispatch(
                    Command::Settings(SettingsCommand::Zoom(ZoomStep::In)),
                    Some(window),
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &UiZoomOut, window, cx| {
                this.dispatch(
                    Command::Settings(SettingsCommand::Zoom(ZoomStep::Out)),
                    Some(window),
                    cx,
                );
            }))
            .on_action(|_: &CycleFrameOverlay, window, _| {
                window.cycle_debug_frame_overlay_mode();
            })
            .on_action(cx.listener(|this, _: &UiZoomReset, window, cx| {
                this.dispatch(
                    Command::Settings(SettingsCommand::Zoom(ZoomStep::Reset)),
                    Some(window),
                    cx,
                );
            }));
        root = wave_actions!(
            root,
            cx,
            [
                SplitRight,
                SplitDown,
                NewPanel,
                ClosePanel,
                FocusNextPanel,
                FocusPrevPanel,
                ToggleViewportLink,
                ToggleCursorLink
            ]
        );
        root = root.on_action(cx.listener(|this, _: &OpenSignalMenu, window, cx| {
            let panel = this.app.panels.focused_id();
            this.dispatch(Command::OpenSignalMenu(panel), Some(window), cx);
        }));
        root = root.on_action(cx.listener(|this, _: &ShowTransaction, window, cx| {
            let panel = this.app.panels.focused_id();
            this.dispatch(Command::ShowTransaction { from: panel }, Some(window), cx);
        }));
        root = wave_actions!(
            root,
            cx,
            [
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
                RemoveMarkerAtCursor,
                RemoveAllMarkers,
                NextMarker,
                PrevMarker,
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
                IncreaseRowHeight,
                DecreaseRowHeight,
                ResetRowHeight,
                MoveSelectionUp,
                MoveSelectionDown,
                NextCycle,
                PrevCycle,
                ToggleCycleOrigin,
                GroupSelection,
                Ungroup,
                RenameGroup,
                FoldGroupDeep,
                UnfoldGroupDeep,
            ]
        );
        #[cfg(not(target_family = "wasm"))]
        {
            root = root.on_drop(cx.listener(
                |this, paths: &gpui_kit::ExternalPaths, window, cx| {
                    let position = window.mouse_position();
                    this.drop_paths(paths.paths().to_vec(), position, window, cx);
                },
            ));
        }
        if let Some(i) = t.islands {
            root = root
                .relative()
                .bg(i.background)
                .child(crate::islands::background());
        }
        if !self.embedded {
            root = root.child(self.render_titlebar(window, cx));
        }
        let island_insets = t
            .islands
            .map(|i| crate::islands::insets(t.scale(i.gap), window.scale_factor()));
        let sidebar = sidebar_visible.then(|| {
            let sidebar = self.render_sidebar(window, cx).into_any_element();
            if let Some(insets) = island_insets {
                div()
                    .flex_none()
                    .h_full()
                    .w(t.px(self.app.sidebar_width))
                    .paddings(insets)
                    .child(crate::islands::rounded(sidebar, cx))
                    .into_any_element()
            } else {
                sidebar
            }
        });
        let center = self.render_center(window, cx);
        let center = if let Some(insets) = island_insets
            && !matches!(self.app.trace_state(), TraceState::Loaded(_))
        {
            div()
                .size_full()
                .paddings(insets)
                .child(crate::islands::rounded(center, cx))
                .into_any_element()
        } else {
            center
        };
        root.child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .when_some(t.islands, |el, _| {
                    el.paddings({
                        let p = island_insets.unwrap();
                        gpui_kit::Edges {
                            left: p.right,
                            right: p.left,
                            top: p.bottom,
                            bottom: p.top,
                        }
                    })
                })
                .when_some(sidebar, |el, sidebar| {
                    el.child(sidebar).child(
                        Splitter::new("sidebar-split", SplitterAxis::Vertical)
                            .line_width(if t.islands.is_some() { 0.0 } else { 1.0 })
                            .dragging(drag == Some(ChromeDrag::Sidebar))
                            .on_drag_start(cx.listener(|this, _, window, cx| {
                                this.dispatch(
                                    Command::ChromeDragStart(ChromeDrag::Sidebar),
                                    Some(window),
                                    cx,
                                );
                            })),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_hidden()
                        .child(center),
                ),
        )
        .child(self.render_statusbar(cx))
        .children(
            self.wave_menu
                .as_ref()
                .map(|menu| popup_at(menu.position, menu.popup.clone(), window, cx)),
        )
        .children(
            self.status_menu
                .as_ref()
                .map(|(p, m)| popup_at(*p, m.clone(), window, cx)),
        )
        .children(self.render_frame_details(cx))
        .children(self.render_rename())
        .children(drag.map(|d| Self::render_drag_surface(d, cx)))
    }
}
