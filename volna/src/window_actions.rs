//! Commands are routed through the shared workspace from every window.

use super::*;
use gpui_kit::{Div, Stateful, WeakEntity};

macro_rules! focus_panel_actions {
    ($el:expr, $owner:expr, [$(($name:ident, $index:expr)),* $(,)?]) => {
        $el $(.on_action(workspace_listener($owner.clone(), |this: &mut Workspace, _: &$name, window, cx| {
            this.dispatch(
                Command::Panels(volna_core::panels::PanelsCommand::FocusIndex($index)),
                Some(window),
                cx,
            )
        })))*
    };
}

macro_rules! wave_actions {
    ($el:expr, $owner:expr, [$($name:ident),* $(,)?]) => {
        $el $(.on_action(workspace_listener($owner.clone(), |this: &mut Workspace, _: &$name, window, cx| {
            this.dispatch(Command::Action(Action::$name), Some(window), cx)
        })))*
    };
}

fn workspace_listener<E: 'static>(
    owner: WeakEntity<Workspace>,
    listener: impl Fn(&mut Workspace, &E, &mut Window, &mut Context<Workspace>) + 'static,
) -> impl Fn(&E, &mut Window, &mut App) + 'static {
    move |event, window, cx| {
        drop(owner.update(cx, |ws, cx| listener(ws, event, window, cx)));
    }
}

impl Workspace {
    pub(crate) fn panel_actions(mut root: Stateful<Div>, owner: WeakEntity<Self>) -> Stateful<Div> {
        root = root
            .on_action(workspace_listener(owner.clone(), Self::open_file))
            .on_action(workspace_listener(owner.clone(), Self::add_trace))
            .on_action(workspace_listener(
                owner.clone(),
                |this, _: &OpenWorkspace, window, cx| {
                    this.dispatch(Command::RequestOpenWorkspace, Some(window), cx);
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, _: &SaveWorkspace, window, cx| {
                    this.dispatch(Command::SaveWorkspace, Some(window), cx);
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, _: &SaveWorkspaceAs, window, cx| {
                    this.dispatch(Command::RequestSaveWorkspaceAs, Some(window), cx);
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, _: &Quit, window, cx| {
                    this.dispatch(Command::RequestQuit, Some(window), cx);
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, _: &Undo, window, cx| {
                    this.dispatch(Command::Undo, Some(window), cx);
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, _: &Redo, window, cx| {
                    this.dispatch(Command::Redo, Some(window), cx);
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, action: &OpenRecent, window, cx| {
                    this.dispatch(
                        Command::Recent(RecentCommand::Open(action.ix)),
                        Some(window),
                        cx,
                    );
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, _: &ClearRecent, window, cx| {
                    this.dispatch(Command::Recent(RecentCommand::Clear), Some(window), cx);
                },
            ))
            .on_action(workspace_listener(owner.clone(), Self::toggle_sidebar))
            .on_action(workspace_listener(owner.clone(), Self::close_trace))
            .on_action(workspace_listener(owner.clone(), Self::open_settings))
            .on_action(workspace_listener(owner.clone(), Self::command_palette))
            .on_action(workspace_listener(
                owner.clone(),
                |this, action: &GoToMarker, window, cx| {
                    if let Some(id) = volna_core::marker::MarkerId::new(action.n) {
                        this.dispatch(Command::Action(Action::GoToMarker(id)), Some(window), cx);
                    }
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, action: &SetMarker, window, cx| {
                    if let Some(id) = volna_core::marker::MarkerId::new(action.n) {
                        this.dispatch(Command::Action(Action::SetMarker(id)), Some(window), cx);
                    }
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, action: &SetTint, window, cx| {
                    this.dispatch(
                        Command::Action(Action::SetTint(action.tint)),
                        Some(window),
                        cx,
                    );
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, action: &ClockAction, window, cx| {
                    this.dispatch(Command::Clocks(action.command.clone()), Some(window), cx);
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, action: &TraceAction, window, cx| {
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
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, action: &OpenPipelineTrack, window, cx| {
                    this.dispatch(
                        Command::OpenPipeline {
                            track: action.track,
                        },
                        Some(window),
                        cx,
                    );
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
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
            owner,
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
            .on_action(workspace_listener(
                owner.clone(),
                |this, _: &UiZoomIn, window, cx| {
                    this.dispatch(
                        Command::Settings(SettingsCommand::Zoom(ZoomStep::In)),
                        Some(window),
                        cx,
                    );
                },
            ))
            .on_action(workspace_listener(
                owner.clone(),
                |this, _: &UiZoomOut, window, cx| {
                    this.dispatch(
                        Command::Settings(SettingsCommand::Zoom(ZoomStep::Out)),
                        Some(window),
                        cx,
                    );
                },
            ))
            .on_action(|_: &CycleFrameOverlay, window, _| {
                window.cycle_debug_frame_overlay_mode();
            })
            .on_action(workspace_listener(
                owner.clone(),
                |this, _: &UiZoomReset, window, cx| {
                    this.dispatch(
                        Command::Settings(SettingsCommand::Zoom(ZoomStep::Reset)),
                        Some(window),
                        cx,
                    );
                },
            ));
        root = wave_actions!(
            root,
            owner,
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
        root = root.on_action(workspace_listener(
            owner.clone(),
            |this, _: &OpenSignalMenu, window, cx| {
                let panel = this.app.panels.focused_id();
                this.dispatch(Command::OpenSignalMenu(panel), Some(window), cx);
            },
        ));
        root = root.on_action(workspace_listener(
            owner.clone(),
            |this, _: &ShowTransaction, window, cx| {
                let panel = this.app.panels.focused_id();
                this.dispatch(Command::ShowTransaction { from: panel }, Some(window), cx);
            },
        ));
        root = wave_actions!(
            root,
            owner,
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
            root = root.on_drop(workspace_listener(
                owner.clone(),
                |this, paths: &gpui_kit::ExternalPaths, window, cx| {
                    let position = window.mouse_position();
                    this.drop_paths(paths.paths().to_vec(), position, window, cx);
                },
            ));
        }

        root.on_action(workspace_listener(owner, Self::close_window))
    }
}
