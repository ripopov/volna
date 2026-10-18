//! GPUI's dock is a mirror of the core layout.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Result, bail, ensure};
use gpui_kit::base::StyledExt as _;
use gpui_kit::base::dock as base;
#[cfg(not(target_family = "wasm"))]
use gpui_kit::component::dock::PanelHandle;
use gpui_kit::component::dock::{DockSkin, Panel, panel_handle};
use gpui_kit::component::{
    Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Axis as GAxis, Context, Div, Empty, Entity, EventEmitter,
    FocusHandle, Focusable, IntoElement, Render, Stateful, TestSupportExt, WeakEntity, Window, div,
    px,
};
use volna_core::panels::{
    Axis, DockLayout, Layout, PanelId, PanelsCommand, Placement, SideDock, WindowId,
};
use volna_core::{App as CoreApp, Command};

use crate::app::Workspace;
use crate::settings_panel::SettingsPanelView;
use crate::start_panel::StartPanelView;
use crate::transaction_panel::TransactionPanelView;
use crate::ui::action_button;

pub(crate) enum PanelView {
    Canvas(Entity<CanvasPanelView>),
    Transaction(Entity<TransactionPanelView>),
    Start(Entity<StartPanelView>),
    Settings(Entity<SettingsPanelView>),
}

impl PanelView {
    fn handle(&self) -> Arc<dyn base::PanelView> {
        match self {
            Self::Canvas(view) => panel_handle(view.clone()),
            Self::Transaction(view) => panel_handle(view.clone()),
            Self::Start(view) => panel_handle(view.clone()),
            Self::Settings(view) => panel_handle(view.clone()),
        }
    }
    fn focus(&self, cx: &App) -> FocusHandle {
        match self {
            Self::Canvas(view) => view.read(cx).focus.clone(),
            Self::Transaction(view) => view.read(cx).focus.clone(),
            Self::Start(view) => view.read(cx).focus_handle(cx),
            Self::Settings(view) => view.read(cx).focus_handle(cx),
        }
    }
    fn entity_id(&self) -> gpui_kit::EntityId {
        match self {
            Self::Canvas(view) => view.entity_id(),
            Self::Transaction(view) => view.entity_id(),
            Self::Start(view) => view.entity_id(),
            Self::Settings(view) => view.entity_id(),
        }
    }
}

pub(crate) fn tab_title(
    ws: &WeakEntity<Workspace>,
    id: PanelId,
    generation: Option<u64>,
    cx: &App,
) -> AnyElement {
    use gpui_kit::MouseButton;
    use gpui_kit::assets::IconName;
    let Some(owner) = ws.upgrade() else {
        return Empty.into_any_element();
    };
    let app = &owner.read(cx).app;
    let panels = &app.panels;
    let Some(panel) = panels.get(id) else {
        return Empty.into_any_element();
    };
    let in_titlebar = match panels.layout().placement(id) {
        Some(Placement::Window(window)) => panels
            .layout()
            .windows
            .get(&window)
            .is_some_and(|layout| layout.panels().len() == 1),
        _ => false,
    };
    let letter = panel
        .kind
        .trace()
        .filter(|_| app.doc.traces().is_combined());
    let title = div()
        .flex()
        .items_center()
        .gap_1()
        .when_some(letter, |el, trace| {
            el.child(crate::traces::letter_badge(trace, cx))
        })
        .when(in_titlebar, |el| el.min_w_0().overflow_hidden())
        .child(
            div()
                .when(in_titlebar, gpui_kit::Styled::truncate)
                .child(gpui_kit::SharedString::from(panel.title())),
        );
    let can_close = panels.can_close(id) && !in_titlebar;
    let close = {
        let ws = ws.clone();
        move |window: &mut Window, cx: &mut App| {
            drop(ws.update(cx, |ws, cx| {
                let generation = generation.unwrap_or(ws.app.doc.generation());
                ws.dispatch_if_current(
                    generation,
                    Command::Panels(PanelsCommand::Close(id)),
                    Some(window),
                    cx,
                );
            }));
        }
    };
    let middle = close.clone();
    let menu_owner = ws.clone();
    div()
        .id(("panel-tab", id.0))
        .test_support()
        .flex()
        .items_center()
        .when(in_titlebar, gpui_kit::Styled::min_w_0)
        .gap_1()
        .debug_selector(|| format!("tab-{}", id.0))
        .when(can_close, |el| {
            el.on_mouse_down(MouseButton::Middle, move |_, window, cx| {
                cx.stop_propagation();
                middle(window, cx);
            })
        })
        .child(title)
        .when(can_close, |el| {
            el.child(
                action_button(("close-tab", id.0 as usize), IconName::X, cx)
                    .ghost()
                    .tab_stop(false)
                    .tooltip("Close tab")
                    .debug_selector(|| format!("tab-close-{}", id.0))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        close(window, cx);
                    }),
            )
        })
        .context_menu(move |menu, _, cx| {
            let Some(owner) = menu_owner.upgrade() else {
                return menu;
            };
            let generation = generation.unwrap_or(owner.read(cx).app.doc.generation());
            panel_menu(menu, &menu_owner, id, generation, cx).when(!in_titlebar, |menu| {
                menu.separator().menu(
                    "Toggle panel zoom",
                    Box::new(gpui_kit::component::dock::ToggleZoom),
                )
            })
        })
        .into_any_element()
}

pub(crate) struct DockHost {
    window: Option<WindowId>,
    owner: WeakEntity<Workspace>,
    render_owner: Option<gpui_kit::EntityId>,
    pub area: Entity<base::DockArea>,
    views: BTreeMap<PanelId, PanelView>,
    hide_tab_bar: Rc<Cell<bool>>,
    revision: Option<u64>,
    installed: Option<DockLayout>,
    echo: Option<DockLayout>,
    zoom: f32,
    right_width: Option<f32>,
    generation: u64,
}

impl DockHost {
    pub fn new(
        owner: WeakEntity<Workspace>,
        window_id: Option<WindowId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let hide_tab_bar = Rc::new(Cell::new(true));
        let mut skin_handle = None;
        let area = cx.new(|cx| {
            let skin = DockSkin::new(cx);
            skin_handle = Some(skin.clone());
            base::DockArea::new("volna-dock", None, window, cx).with_renderer(Rc::new(
                DockRenderer {
                    skin,
                    hide_tab_bar: hide_tab_bar.clone(),
                    area: cx.weak_entity(),
                    resize: crate::islands::DockResize::default(),
                },
            ))
        });
        skin_handle
            .unwrap()
            .set_panel_style(gpui_kit::component::dock::PanelStyle::TabBar, cx);
        cx.subscribe(&area, |host, area, event, cx| {
            if !matches!(event, base::DockEvent::LayoutChanged) {
                return;
            }
            let snapshot = area.read(cx).dump(cx);
            let measured_width = snapshot
                .right_dock
                .as_ref()
                .map(|dock| f32::from(dock.size()));
            let mut candidate = from_area(&snapshot, host.zoom);
            // Clamping to a narrow window must not overwrite the preferred
            // width when an unrelated tab or focus event arrives.
            if measured_width == host.right_width
                && let Ok(layout) = &mut candidate
                && let (Some(right), Some(previous)) = (
                    &mut layout.right,
                    host.installed
                        .as_ref()
                        .and_then(|layout| layout.right.as_ref()),
                )
            {
                right.width = previous.width;
            }
            match candidate {
                Ok(layout) if host.echo.as_ref() == Some(&layout) => {}
                Ok(layout) => {
                    let revision = host.revision.unwrap_or(0);
                    host.right_width = measured_width;
                    host.installed = Some(layout.clone());
                    host.echo = Some(layout.clone());
                    let owner = host.owner.clone();
                    let window_id = host.window;
                    let weak = cx.weak_entity();
                    cx.defer(move |cx| {
                        let result = owner.update(cx, |ws, cx| {
                            match ws.app.panels.layout().with_window(window_id, layout) {
                                Ok(layout) => {
                                    ws.dispatch(
                                        Command::Panels(PanelsCommand::SetLayout {
                                            layout,
                                            from_revision: revision,
                                        }),
                                        None,
                                        cx,
                                    );
                                    true
                                }
                                Err(error) => {
                                    log::warn!("invalid window layout: {error:#}");
                                    false
                                }
                            }
                        });
                        if matches!(result, Ok(false)) {
                            drop(weak.update(cx, |host, cx| {
                                host.installed = None;
                                if let Some(owner) = host.render_owner {
                                    App::notify(cx, owner);
                                }
                            }));
                        }
                    });
                }
                Err(e) => {
                    log::warn!("invalid dock layout: {e:#}");
                    // A rejected widget proposal must not remain on screen.
                    host.installed = None;
                    if let Some(owner) = host.render_owner {
                        App::notify(cx, owner);
                    }
                }
            }
        })
        .detach();
        Self {
            owner,
            window: window_id,
            render_owner: None,
            area,
            views: BTreeMap::new(),
            hide_tab_bar,
            revision: None,
            installed: None,
            echo: None,
            zoom: 1.0,
            right_width: None,
            generation: 0,
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn titlebar_panel(&self) -> Option<Arc<dyn gpui_kit::component::dock::PanelView>> {
        if self.window.is_none() || self.views.len() != 1 || !self.hide_tab_bar.get() {
            return None;
        }
        let handle = self.views.values().next()?.handle();
        PanelHandle::of(&handle).map(PanelHandle::panel)
    }

    pub fn sync(
        &mut self,
        app: &CoreApp,
        render_owner: gpui_kit::EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.render_owner = Some(render_owner);
        let ws = self.owner.clone();
        if self.generation != app.doc.generation() {
            self.views
                .retain(|_, view| matches!(view, PanelView::Settings(_)));
            self.installed = None;
            self.echo = None;
            self.revision = None;
            self.generation = app.doc.generation();
        }
        let local = app.panels.layout().for_window(self.window);
        let ids = local.panels();
        self.hide_tab_bar.set(
            (self.window.is_some() && ids.len() == 1)
                || (app.panels.len() == 1 && app.panels.focused().kind.is_start()),
        );
        self.views.retain(|id, _| ids.contains(id));
        for panel in app.panels.iter().filter(|panel| ids.contains(&panel.id)) {
            self.views.entry(panel.id).or_insert_with(|| {
                if panel.kind.is_settings() {
                    PanelView::Settings(
                        cx.new(|cx| SettingsPanelView::new(ws.clone(), panel.id, window, cx)),
                    )
                } else if panel.kind.transaction().is_some() {
                    PanelView::Transaction(cx.new(|cx| {
                        TransactionPanelView::new(
                            ws.clone(),
                            panel.id,
                            app.doc.generation(),
                            window,
                            cx,
                        )
                    }))
                } else if panel.kind.is_start() {
                    PanelView::Start(cx.new(|cx| {
                        StartPanelView::new(ws.clone(), panel.id, app.doc.generation(), window, cx)
                    }))
                } else {
                    let name = if panel.kind.pipeline().is_some() {
                        "volna.pipeline"
                    } else if panel.kind.table().is_some() {
                        "volna.table"
                    } else {
                        "volna.waves"
                    };
                    PanelView::Canvas(cx.new(|cx| {
                        CanvasPanelView::new(
                            ws.clone(),
                            panel.id,
                            app.doc.generation(),
                            name,
                            window,
                            cx,
                        )
                    }))
                }
            });
        }
        let layout = &local;
        let zoom = crate::theme::theme(cx).zoom;
        let bounds = self.area.read(cx).bounds();
        let available = f32::from(bounds.size.width);
        let right_width = layout.right.as_ref().map(|dock| {
            let center_min = if layout.center.is_some() {
                (available * 0.5).min(160.0 * zoom)
            } else {
                0.0
            };
            (dock.width * zoom)
                .min((available - center_min).max(100.0))
                .max(100.0)
        });
        if self.installed.as_ref() != Some(layout)
            || self.zoom != zoom
            || self.right_width != right_width
        {
            if (app.panels.len() > 1 || layout.right.is_some())
                && (bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0))
            {
                window.on_next_frame(move |_, cx| cx.notify(render_owner));
                return;
            }
            let same_trees = self.zoom == zoom
                && self.installed.as_ref().is_some_and(|installed| {
                    installed.center == layout.center
                        && installed.right.as_ref().map(|dock| &dock.layout)
                            == layout.right.as_ref().map(|dock| &dock.layout)
                });
            if same_trees {
                // Width and visibility belong to the dock, not its pane tree.
                // Keep Kit's tab groups and split widgets alive for these edits.
                if let Some(dock) = &layout.right {
                    self.area.update(cx, |area, cx| {
                        area.set_dock_size(
                            base::DockPlacement::Right,
                            px(right_width.unwrap()),
                            window,
                            cx,
                        );
                        if area.is_dock_open(base::DockPlacement::Right) != dock.open {
                            area.toggle_dock(base::DockPlacement::Right, window, cx);
                        }
                    });
                }
            } else {
                let occupied = layout
                    .right
                    .as_ref()
                    .filter(|dock| dock.open)
                    .and(right_width)
                    .unwrap_or(0.0);
                let center =
                    layout
                        .center
                        .as_ref()
                        .map_or_else(base::DockLayout::h_split, |tree| {
                            to_dock(
                                tree,
                                &self.views,
                                (f32::from(bounds.size.width) - occupied).max(1.0),
                                f32::from(bounds.size.height),
                                cx,
                            )
                        });
                let right = layout.right.as_ref().map(|dock| {
                    to_dock(
                        &dock.layout,
                        &self.views,
                        right_width.unwrap(),
                        f32::from(bounds.size.height),
                        cx,
                    )
                });
                self.area.update(cx, |area, cx| {
                    // Remove the outgoing region before installing either tree: a
                    // moved panel must never belong to both regions during reconcile.
                    area.remove_dock(base::DockPlacement::Right, window, cx);
                    // Unsupported regions can only arrive in a rejected proposal.
                    area.remove_dock(base::DockPlacement::Left, window, cx);
                    area.remove_dock(base::DockPlacement::Bottom, window, cx);
                    area.set_center(center, window, cx);
                    if let (Some(tree), Some(dock)) = (right, &layout.right) {
                        area.set_dock(base::DockPlacement::Right, tree, window, cx);
                        area.set_dock_size(
                            base::DockPlacement::Right,
                            px(right_width.unwrap()),
                            window,
                            cx,
                        );
                        if !dock.open {
                            area.toggle_dock(base::DockPlacement::Right, window, cx);
                        }
                    }
                });
            }
            self.zoom = zoom;
            self.right_width = right_width;
            self.echo = from_area(&self.area.read(cx).dump(cx), zoom).ok();
            if let (Some(echo), Some(preferred)) = (&mut self.echo, &layout.right)
                && let Some(right) = &mut echo.right
            {
                right.width = preferred.width;
            }
            self.installed = Some(layout.clone());
        }
        if self.revision != Some(app.panels.revision()) {
            self.area.update(cx, |_, cx| cx.notify());
        }
        self.revision = Some(app.panels.revision());
    }

    pub fn focus(&self, panel: PanelId, cx: &App) -> Option<FocusHandle> {
        self.installed.as_ref()?.active_for(panel)?;
        self.views.get(&panel).map(|view| view.focus(cx))
    }

    pub fn settings_view(
        &mut self,
        id: PanelId,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<SettingsPanelView> {
        let ws = self.owner.clone();
        let view = self.views.entry(id).or_insert_with(|| {
            PanelView::Settings(cx.new(|cx| SettingsPanelView::new(ws, id, window, cx)))
        });
        match view {
            PanelView::Settings(view) => view.clone(),
            _ => unreachable!("panel {id:?} is not the settings tab"),
        }
    }

    pub fn invalidate_panels(host: &Entity<Self>, cx: &mut App) {
        let host = host.read(cx);
        let ids: Vec<_> = std::iter::once(host.area.entity_id())
            .chain(host.views.values().map(PanelView::entity_id))
            .collect();
        for id in ids {
            cx.notify(id);
        }
    }
}

fn from_area(state: &base::DockAreaState, zoom: f32) -> Result<DockLayout> {
    ensure!(
        state.left_dock.is_none() && state.bottom_dock.is_none(),
        "unsupported dock region"
    );
    let empty = |state: &base::PanelState| {
        !matches!(state.info, base::PanelInfo::Panel(_)) && state.children.is_empty()
    };
    Ok(DockLayout {
        windows: BTreeMap::new(),
        center: if empty(&state.center) {
            None
        } else {
            Some(from_dock(&state.center)?)
        },
        right: state
            .right_dock
            .as_ref()
            .filter(|dock| !empty(dock.panel()))
            .map(|dock| -> Result<SideDock> {
                Ok(SideDock {
                    layout: from_dock(dock.panel())?,
                    width: f32::from(dock.size()) / zoom,
                    open: dock.open(),
                })
            })
            .transpose()?,
    })
}

/// Panel placement is a core command so menu moves share history and persistence
/// with drag-and-drop proposals from `DockArea`.
fn placement_menu(
    menu: PopupMenu,
    ws: &WeakEntity<Workspace>,
    id: PanelId,
    generation: u64,
    cx: &App,
) -> PopupMenu {
    let placement = ws
        .upgrade()
        .and_then(|ws| ws.read(cx).app.panels.layout().placement(id));
    #[cfg(not(target_family = "wasm"))]
    let menu = {
        let command = if matches!(placement, Some(Placement::Window(_))) {
            PanelsCommand::Move {
                panel: id,
                placement: Placement::Center,
            }
        } else {
            PanelsCommand::MoveToNewWindow(id)
        };
        let label = if matches!(placement, Some(Placement::Window(_))) {
            "Return to main window"
        } else {
            "Move to new window"
        };
        let owner = ws.clone();
        let menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
            drop(owner.update(cx, |ws, cx| {
                ws.dispatch_if_current(
                    generation,
                    Command::Panels(command.clone()),
                    Some(window),
                    cx,
                );
            }));
        }));
        if matches!(placement, Some(Placement::Window(_))) {
            return menu;
        }
        menu
    };
    let (label, destination) = if placement == Some(Placement::Right) {
        ("Move to center", Placement::Center)
    } else {
        ("Move to right sidebar", Placement::Right)
    };
    let ws = ws.clone();
    menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
        drop(ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(
                generation,
                Command::Panels(PanelsCommand::Move {
                    panel: id,
                    placement: destination,
                }),
                Some(window),
                cx,
            );
        }));
    }))
}

pub(crate) fn panel_menu(
    mut menu: PopupMenu,
    ws: &WeakEntity<Workspace>,
    id: PanelId,
    generation: u64,
    cx: &App,
) -> PopupMenu {
    let Some(owner) = ws.upgrade() else {
        return menu;
    };
    let app = &owner.read(cx).app;
    let Some(panel) = app
        .panels
        .get(id)
        .filter(|_| app.doc.generation() == generation)
    else {
        return menu;
    };
    let start = panel.kind.is_start();
    let can_split = start || panel.kind.is_canvas() || panel.kind.transaction().is_some();
    let can_close = app.panels.can_close(id);
    let links = panel.kind.nav().map(|nav| nav.link);
    let pinned = panel
        .kind
        .transaction()
        .map(volna_core::transaction::TransactionModel::pinned);
    let other_content = app
        .panels
        .iter()
        .any(|panel| panel.id != id && panel.kind.is_content());
    let item = |label: &'static str, command: PanelsCommand| {
        let ws = ws.clone();
        PopupMenuItem::new(label).on_click(move |_, window, cx| {
            drop(ws.update(cx, |ws, cx| {
                if start
                    && matches!(
                        &command,
                        PanelsCommand::NewTab { .. } | PanelsCommand::Split { .. }
                    )
                {
                    // Start replacements retain source focus; an inactive source
                    // would otherwise leave the new view unfocused.
                    ws.dispatch_if_current(
                        generation,
                        Command::Panels(PanelsCommand::Focus(id)),
                        Some(window),
                        cx,
                    );
                }
                ws.dispatch_if_current(
                    generation,
                    Command::Panels(command.clone()),
                    Some(window),
                    cx,
                );
            }));
        })
    };
    if !start {
        menu = placement_menu(menu, ws, id, generation, cx);
    }
    if can_split {
        for (label, axis) in [
            ("Split Right", Axis::Horizontal),
            ("Split Down", Axis::Vertical),
        ] {
            menu = menu.item(item(label, PanelsCommand::Split { panel: id, axis }));
        }
    }
    menu = menu.item(item(
        "New waveform tab",
        PanelsCommand::NewTab { group_of: id },
    ));
    if can_close {
        menu = close_menu(menu, ws, id, generation, cx);
    }
    menu = menu
        .item(item("Close Other Panels", PanelsCommand::CloseOthers(id)).disabled(!other_content));
    let rename_owner = ws.clone();
    menu = menu.item(
        PopupMenuItem::new("Rename…").on_click(move |_, window, cx| {
            rename_panel(&rename_owner, id, generation, window, cx);
        }),
    );
    if let Some(links) = links {
        use volna_core::nav::LinkDim;
        menu = menu.separator();
        for (label, dim, linked) in [
            ("Follow shared viewport", LinkDim::Viewport, links.viewport),
            ("Follow shared cursor", LinkDim::Cursor, links.cursor),
        ] {
            menu = menu
                .item(item(label, PanelsCommand::ToggleLink { panel: id, dim }).checked(linked));
        }
    }
    if let Some(pinned) = pinned {
        let ws = ws.clone();
        menu = menu
            .separator()
            .item(PopupMenuItem::new("Pin record").checked(pinned).on_click(
                move |_, window, cx| {
                    drop(ws.update(cx, |ws, cx| {
                        ws.dispatch_if_current(
                            generation,
                            Command::Transaction(
                                id,
                                volna_core::transaction::TransactionCommand::Pin(!pinned),
                            ),
                            Some(window),
                            cx,
                        );
                    }));
                },
            ));
    }
    menu
}

/// Hiding the sidebar preserves all its tabs; closing a tab removes only that
/// panel. Center panels have no sidebar to hide.
pub(crate) fn close_menu(
    mut menu: PopupMenu,
    ws: &WeakEntity<Workspace>,
    id: PanelId,
    generation: u64,
    cx: &App,
) -> PopupMenu {
    let in_sidebar = ws
        .upgrade()
        .is_some_and(|ws| ws.read(cx).app.panels.layout().placement(id) == Some(Placement::Right));
    if in_sidebar {
        let ws = ws.clone();
        menu = menu.item(
            PopupMenuItem::new("Hide sidebar").on_click(move |_, window, cx| {
                drop(ws.update(cx, |ws, cx| {
                    let layout = ws.app.panels.layout();
                    if layout.placement(id) == Some(Placement::Right)
                        && layout.right.as_ref().is_some_and(|dock| dock.open)
                    {
                        ws.dispatch_if_current(
                            generation,
                            Command::Panels(PanelsCommand::ToggleRightDock),
                            Some(window),
                            cx,
                        );
                    }
                }));
            }),
        );
    }
    let ws = ws.clone();
    menu.item(
        PopupMenuItem::new(if in_sidebar {
            "Close tab"
        } else {
            "Close panel"
        })
        .on_click(move |_, window, cx| {
            drop(ws.update(cx, |ws, cx| {
                ws.dispatch_if_current(
                    generation,
                    Command::Panels(PanelsCommand::Close(id)),
                    Some(window),
                    cx,
                );
            }));
        }),
    )
}

fn to_dock(
    layout: &Layout,
    views: &BTreeMap<PanelId, PanelView>,
    width: f32,
    height: f32,
    cx: &App,
) -> base::DockLayout {
    match layout {
        Layout::Tabs { tabs, active } => {
            let mut out = base::DockLayout::tabs();
            for id in tabs {
                out = out.panel_view(views[id].handle(), cx);
            }
            out.active_index(tabs.iter().position(|id| id == active).unwrap())
        }
        Layout::Split {
            split,
            sizes,
            children,
        } => {
            let horizontal = *split == Axis::Horizontal;
            let mut out = if horizontal {
                base::DockLayout::h_split()
            } else {
                base::DockLayout::v_split()
            };
            for (child, share) in children.iter().zip(sizes) {
                let w = if horizontal { width * share } else { width };
                let h = if horizontal { height } else { height * share };
                out = out.child(
                    to_dock(child, views, w, h, cx),
                    Some(px(if horizontal { w } else { h })),
                );
            }
            out
        }
    }
}

fn from_dock(state: &base::PanelState) -> Result<Layout> {
    match &state.info {
        base::PanelInfo::Stack { sizes, axis } => {
            let children: Vec<_> = state
                .children
                .iter()
                .map(from_dock)
                .collect::<Result<_>>()?;
            if children.len() == 1 {
                return Ok(children.into_iter().next().unwrap());
            }
            ensure!(
                children.len() >= 2 && sizes.len() == children.len(),
                "invalid split"
            );
            let total: f32 = sizes.iter().map(|p| f32::from(*p)).sum();
            ensure!(total.is_finite() && total > 0.0, "invalid sizes");
            Ok(Layout::Split {
                split: if *axis == 0 {
                    Axis::Horizontal
                } else {
                    Axis::Vertical
                },
                sizes: sizes.iter().map(|p| f32::from(*p) / total).collect(),
                children,
            })
        }
        base::PanelInfo::Tabs { active_index } => {
            let tabs: Vec<_> = state.children.iter().map(panel_id).collect::<Result<_>>()?;
            let active = *tabs
                .get(*active_index)
                .ok_or_else(|| anyhow::anyhow!("invalid active tab"))?;
            Ok(Layout::Tabs { tabs, active })
        }
        base::PanelInfo::Panel(_) => Ok(Layout::single(panel_id(state)?)),
    }
}

fn panel_id(state: &base::PanelState) -> Result<PanelId> {
    ensure!(
        matches!(
            state.panel_name.as_ref(),
            "volna.waves"
                | "volna.pipeline"
                | "volna.table"
                | "volna.transaction"
                | "volna.start"
                | "volna.settings"
        ),
        "unknown dock widget"
    );
    match &state.info {
        base::PanelInfo::Panel(value) => value
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .map(PanelId)
            .ok_or_else(|| anyhow::anyhow!("missing panel ID")),
        _ => bail!("expected panel payload"),
    }
}

fn rename_panel(
    ws: &WeakEntity<Workspace>,
    id: PanelId,
    generation: u64,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(owner) = ws.upgrade() else {
        return;
    };
    if owner.read(cx).app.doc.generation() != generation {
        return;
    }
    let Some(panel) = owner.read(cx).app.panels.get(id) else {
        return;
    };
    let title = panel.title.get().clone().unwrap_or_default();
    let input = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(title)
            .placeholder("Default panel title")
    });
    let input_focus = input.read(cx).focus_handle(cx);
    window.open_dialog(cx, move |dialog, _, _| {
        let owner = owner.clone();
        let value = input.clone();
        dialog
            .title("Rename panel")
            .child(Input::new(&input))
            .footer(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-rename")
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("confirm-rename")
                            .label("Rename")
                            .primary()
                            .on_click(|_, window, cx| {
                                window.dispatch_action(
                                    Box::new(gpui_kit::component::dialog::Confirm {
                                        secondary: false,
                                    }),
                                    cx,
                                );
                            }),
                    ),
            )
            .on_ok(move |_, window, cx| {
                let title = value.read(cx).value().to_string();
                owner.update(cx, |ws, cx| {
                    ws.dispatch_if_current(
                        generation,
                        Command::Panels(PanelsCommand::Rename(id, Some(title))),
                        Some(window),
                        cx,
                    );
                });
                true
            })
    });
    window.focus(&input_focus, cx);
}

pub(crate) struct CanvasPanelView {
    pub(crate) table_columns: Option<Entity<crate::table_columns::ColumnPicker>>,
    pub(crate) table_filters: Option<Entity<crate::table_filters::FilterRow>>,
    pub(crate) ws: WeakEntity<Workspace>,
    pub(crate) id: PanelId,
    pub(crate) generation: u64,
    name: &'static str,
    pub(crate) focus: FocusHandle,
}

impl CanvasPanelView {
    fn dispatch(&self, command: PanelsCommand, window: &mut Window, cx: &mut Context<Self>) {
        drop(self.ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(self.generation, Command::Panels(command), Some(window), cx);
        }));
    }
    fn new(
        ws: WeakEntity<Workspace>,
        id: PanelId,
        generation: u64,
        name: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        cx.on_focus_in(&focus, window, |view, window, cx| {
            view.dispatch(PanelsCommand::Focus(view.id), window, cx);
        })
        .detach();
        Self {
            ws,
            id,
            generation,
            name,
            focus,
            table_columns: None,
            table_filters: None,
        }
    }
}

impl EventEmitter<base::PanelEvent> for CanvasPanelView {}
impl Focusable for CanvasPanelView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl base::Panel for CanvasPanelView {
    fn panel_name(&self) -> &'static str {
        self.name
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn dump(&self, _: &App) -> base::PanelState {
        let mut state = base::PanelState::new(self.name);
        state.info = base::PanelInfo::panel(serde_json::json!({"id": self.id.0}));
        state
    }
}
impl Panel for CanvasPanelView {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        tab_title(&self.ws, self.id, Some(self.generation), cx)
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn toolbar_buttons(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Option<Vec<Button>> {
        use gpui_kit::assets::IconName;
        use volna_core::nav::LinkDim;
        let link = self
            .ws
            .upgrade()?
            .read(cx)
            .app
            .panels
            .get(self.id)?
            .kind
            .nav()?
            .link;
        let link_button = |name, icon, linked, dim, tooltip: &'static str| {
            action_button(name, icon, cx)
                .ghost()
                .selected(linked)
                .tooltip(tooltip)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.dispatch(
                        PanelsCommand::ToggleLink {
                            panel: view.id,
                            dim,
                        },
                        window,
                        cx,
                    );
                }))
        };
        let mut buttons = vec![
            link_button(
                "link-view",
                IconName::Scan,
                link.viewport,
                LinkDim::Viewport,
                "Follow shared viewport (L)",
            ),
            link_button(
                "link-cursor",
                IconName::Crosshair,
                link.cursor,
                LinkDim::Cursor,
                "Follow shared cursor (Shift+L)",
            ),
            action_button("new-tab", IconName::Plus, cx)
                .ghost()
                .tooltip("New waveform tab")
                .on_click(cx.listener(|view, _, window, cx| {
                    view.dispatch(PanelsCommand::NewTab { group_of: view.id }, window, cx);
                })),
        ];
        if let Some((follow, activity)) = self.ws.upgrade().and_then(|owner| {
            let ws = owner.read(cx);
            let pipeline = ws.app.panels.pipeline(self.id)?;
            Some((pipeline.follow, pipeline.activity(&ws.app.doc)))
        }) {
            use volna_core::pipeline::{ActivityCommand, FollowActivity};
            let mut controls = vec![(
                "follow-activity",
                match follow {
                    FollowActivity::Following => "Following activity",
                    FollowActivity::Suspended => "Resume follow",
                    FollowActivity::Off => "Follow activity",
                }
                .to_string(),
                ActivityCommand::Toggle,
            )];
            if activity.above > 0 {
                controls.push((
                    "activity-above",
                    format!("↑ {} above", activity.above),
                    ActivityCommand::RevealAbove,
                ));
            }
            if activity.below > 0 {
                controls.push((
                    "activity-below",
                    format!("↓ {} below", activity.below),
                    ActivityCommand::RevealBelow,
                ));
            }
            for (name, label, command) in controls.into_iter().rev() {
                let owner = self.ws.clone();
                let generation = self.generation;
                let panel = self.id;
                buttons.insert(
                    0,
                    Button::new(name)
                        .label(label)
                        .ghost()
                        .when(
                            name == "follow-activity" && follow == FollowActivity::Suspended,
                            gpui_kit::component::button::ButtonVariants::warning,
                        )
                        .xsmall()
                        .selected(name == "follow-activity" && follow == FollowActivity::Following)
                        .tooltip(if name == "follow-activity" {
                            "Follow activity; manual vertical navigation suspends following"
                        } else {
                            "Reveal relevant rows without enabling follow"
                        })
                        .on_click(move |_, window, cx| {
                            drop(owner.update(cx, |ws, cx| {
                                ws.dispatch_if_current(
                                    generation,
                                    Command::PipelineActivity(panel, command),
                                    Some(window),
                                    cx,
                                );
                            }));
                        }),
                );
            }
        }
        let timed = self.ws.upgrade().is_some_and(|owner| {
            let ws = owner.read(cx);
            ws.app
                .panels
                .get(self.id)
                .is_some_and(|p| p.kind.marker_lane().is_some() && !ws.app.doc.markers().is_empty())
        });
        if timed {
            let id = self.id;
            let generation = self.generation;
            let owner = self.ws.clone();
            buttons.insert(
                0,
                Button::new("markers")
                    .label("Markers…")
                    .ghost()
                    .xsmall()
                    .tooltip("Find a marker by name or number (')")
                    .on_click(move |_, window, cx| {
                        drop(owner.update(cx, |ws, cx| {
                            ws.dispatch_if_current(
                                generation,
                                Command::Panels(PanelsCommand::Focus(id)),
                                Some(window),
                                cx,
                            );
                            ws.dispatch_if_current(
                                generation,
                                Command::Action(volna_core::app::Action::MarkerNavigator),
                                Some(window),
                                cx,
                            );
                        }));
                    }),
            );
        }
        let selected_here = self
            .ws
            .upgrade()
            .and_then(|owner| {
                let ws = owner.read(cx);
                let pipeline = ws.app.panels.pipeline(self.id)?;
                Some(pipeline.selected_row(&ws.app.doc).is_some())
            })
            .unwrap_or(false);
        if selected_here {
            let id = self.id;
            let generation = self.generation;
            let owner = self.ws.clone();
            buttons.insert(
                0,
                Button::new("show-transaction")
                    .label("Details")
                    .ghost()
                    .xsmall()
                    .tooltip("Show the selected record in a Transaction panel (⏎)")
                    .on_click(move |_, window, cx| {
                        drop(owner.update(cx, |ws, cx| {
                            ws.dispatch_if_current(
                                generation,
                                Command::ShowTransaction { from: id },
                                Some(window),
                                cx,
                            );
                        }));
                    }),
            );
        }
        let table_eligible = self
            .ws
            .upgrade()
            .and_then(|owner| {
                let workspace = owner.read(cx);
                let track = workspace
                    .app
                    .panels
                    .get(self.id)?
                    .kind
                    .pipeline()?
                    .track
                    .track()?;
                workspace
                    .app
                    .doc
                    .session(track.trace)?
                    .tracks()
                    .iter()
                    .find(|declaration| declaration.id == track.item)
                    .map(|declaration| {
                        matches!(
                            declaration.kind,
                            volna_trace::data::transactions::TrackKind::Generator { .. }
                        )
                    })
            })
            .unwrap_or(false);
        if table_eligible {
            let id = self.id;
            let generation = self.generation;
            let owner = self.ws.clone();
            buttons.insert(
                0,
                Button::new("open-table")
                    .label("Table")
                    .ghost()
                    .xsmall()
                    .tooltip("Open this generator in a table")
                    .on_click(move |_, window, cx| {
                        drop(owner.update(cx, |ws, cx| {
                            ws.dispatch_if_current(
                                generation,
                                Command::OpenTableFromPanel {
                                    panel: id,
                                    row: None,
                                },
                                Some(window),
                                cx,
                            );
                        }));
                    }),
            );
        }
        Some(buttons)
    }

    fn dropdown_menu(
        &mut self,
        menu: PopupMenu,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> PopupMenu {
        panel_menu(menu, &self.ws, self.id, self.generation, cx)
    }
}
impl Render for CanvasPanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ws) = self.ws.upgrade() else {
            return Empty.into_any_element();
        };
        let unsupported = ws
            .read(cx)
            .app
            .panels
            .get(self.id)
            .is_none_or(|p| !p.kind.is_canvas());
        if ws
            .read(cx)
            .app
            .panels
            .get(self.id)
            .is_some_and(|panel| panel.kind.table().is_some())
        {
            return self.render_table(window, cx);
        }
        div()
            .id(("wave-panel", self.id.0))
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .key_context("Waves")
            .when(unsupported, |el| {
                el.flex()
                    .items_center()
                    .justify_center()
                    .child("Unsupported panel — saved content is preserved")
            })
            .when(!unsupported, |el| {
                let waves = ws.read(cx).app.panels.waves(self.id).is_some();
                el.child(crate::canvas::PanelCanvas::new(ws, self.id, self.generation).waves(waves))
            })
            .into_any_element()
    }
}

struct DockRenderer {
    skin: Rc<DockSkin>,
    hide_tab_bar: Rc<Cell<bool>>,
    area: WeakEntity<base::DockArea>,
    resize: crate::islands::DockResize,
}
impl base::DockAreaRenderer for DockRenderer {
    fn frame(&self, w: &mut Window, cx: &mut App) -> Stateful<Div> {
        let t = *crate::theme::theme(cx);
        self.skin
            .frame(w, cx)
            .when_some(t.islands, |el, _| el.bg(gpui_kit::transparent_black()))
    }
    fn center_frame(&self, w: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.skin.center_frame(w, cx)
    }
    fn split_frame(
        &self,
        node: base::NodeId,
        axis: GAxis,
        w: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        self.skin
            .split_frame(node, axis, w, cx)
            .when(crate::theme::theme(cx).islands.is_some(), |el| {
                el.bg(gpui_kit::transparent_black())
            })
    }
    fn render_split_handle(
        &self,
        handle: &gpui_kit::base::ResizeHandleContext,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        if crate::theme::theme(cx).islands.is_some() {
            return Some(crate::islands::resize_handle(handle, cx));
        }
        self.skin.render_split_handle(handle, w, cx)
    }
    fn render_dock(
        &self,
        dock: &base::DockContext,
        content: AnyElement,
        w: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        if crate::theme::theme(cx).islands.is_some() {
            self.resize.render(dock, content, self.area.clone())
        } else {
            self.skin.render_dock(dock, content, w, cx)
        }
    }

    fn build_placeholder(
        &self,
        state: &base::PanelState,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn base::PanelView>> {
        self.skin.build_placeholder(state, w, cx)
    }
    fn tab_group_renderer(&self) -> Rc<dyn base::TabGroupRenderer> {
        Rc::new(TabRenderer {
            skin: self.skin.tab_group_renderer(),
            hide_tab_bar: self.hide_tab_bar.clone(),
            islands: crate::islands::Tabs::default(),
        })
    }
}
struct TabRenderer {
    islands: crate::islands::Tabs,
    skin: Rc<dyn base::TabGroupRenderer>,
    hide_tab_bar: Rc<Cell<bool>>,
}
impl base::TabGroupRenderer for TabRenderer {
    fn frame(&self, group: &base::TabGroupContext, w: &mut Window, cx: &mut App) -> Stateful<Div> {
        let t = *crate::theme::theme(cx);
        self.skin.frame(group, w, cx).when_some(t.islands, |el, i| {
            el.bg(gpui_kit::transparent_black())
                .paddings(crate::islands::insets(t.scale(i.gap), w.scale_factor()))
        })
    }
    fn content_frame(
        &self,
        group: &base::TabGroupContext,
        w: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let t = *crate::theme::theme(cx);
        self.skin
            .content_frame(group, w, cx)
            .when(t.islands.is_some(), |el| {
                el.relative().pt_0().bg(t.panel.bg)
            })
    }
    fn render_tab_bar(
        &self,
        group: &base::TabGroupContext,
        w: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        if self.hide_tab_bar.get() {
            Empty.into_any_element()
        } else if crate::theme::theme(cx).islands.is_some() {
            self.islands.render(group, w, cx)
        } else {
            self.skin.render_tab_bar(group, w, cx)
        }
    }
    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &base::TabGroupContext,
        w: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let content = self.skin.render_active_panel(panel, group, w, cx);
        if crate::theme::theme(cx).islands.is_none() {
            return content;
        }
        if self.hide_tab_bar.get() {
            return crate::islands::rounded(content, cx);
        }
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(content)
            .child(crate::islands::corners(false, cx))
            .into_any_element()
    }
    fn render_drop_indicator(
        &self,
        indicator: base::DropIndicator,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        use crate::theme::ThemePx;
        let t = *crate::theme::theme(cx);
        if let Some(i) = t.islands {
            let target = indicator.to();
            return Some(
                div()
                    .absolute()
                    .left(target.origin().x)
                    .top(target.origin().y)
                    .w(target.size().width)
                    .h(target.size().height)
                    .rounded(t.px(i.radius))
                    .bg(t.selection.bg.opacity(0.8))
                    .border_1()
                    .border_color(t.border_focused)
                    .into_any_element(),
            );
        }
        self.skin.render_drop_indicator(indicator, w, cx)
    }
    fn render_empty(
        &self,
        group: &base::TabGroupContext,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.skin.render_empty(group, w, cx)
    }
}
