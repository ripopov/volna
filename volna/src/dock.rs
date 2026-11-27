//! GPUI's dock is a mirror of the core layout.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Result, bail, ensure};
use gpui_kit::base::Disableable;
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
    Axis, BottomDock, DockLayout, Layout, PanelId, PanelsCommand, Placement, SideDock, WindowId,
};
use volna_core::{App as CoreApp, Command};

use crate::app::Workspace;
use crate::cursor_panel::CursorRecordsPanelView;
use crate::settings_panel::SettingsPanelView;
use crate::start_panel::StartPanelView;
use crate::theme::{ThemePx, theme};
use crate::transaction_panel::TransactionPanelView;
use crate::ui::action_button;

pub(crate) enum PanelView {
    Canvas(Entity<CanvasPanelView>),
    Transaction(Entity<TransactionPanelView>),
    CursorRecords(Entity<CursorRecordsPanelView>),
    Start(Entity<StartPanelView>),
    Settings(Entity<SettingsPanelView>),
}

impl PanelView {
    fn handle(&self) -> Arc<dyn base::PanelView> {
        match self {
            Self::Canvas(view) => panel_handle(view.clone()),
            Self::Transaction(view) => panel_handle(view.clone()),
            Self::CursorRecords(view) => panel_handle(view.clone()),
            Self::Start(view) => panel_handle(view.clone()),
            Self::Settings(view) => panel_handle(view.clone()),
        }
    }
    fn focus(&self, cx: &App) -> FocusHandle {
        match self {
            Self::Canvas(view) => view.read(cx).focus.clone(),
            Self::Transaction(view) => view.read(cx).focus.clone(),
            Self::CursorRecords(view) => view.read(cx).focus.clone(),
            Self::Start(view) => view.read(cx).focus_handle(cx),
            Self::Settings(view) => view.read(cx).focus_handle(cx),
        }
    }
    fn entity_id(&self) -> gpui_kit::EntityId {
        match self {
            Self::Canvas(view) => view.entity_id(),
            Self::Transaction(view) => view.entity_id(),
            Self::CursorRecords(view) => view.entity_id(),
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
    let t = *theme(cx);
    let panel_title = gpui_kit::SharedString::from(panel.title());
    let title = div()
        .flex()
        .items_center()
        .gap_1()
        .when(t.compact_chrome, |el| {
            use crate::ui::{Icon, IconName};
            use volna_core::panels::PanelKind;
            let (icon, color) = match &panel.kind {
                PanelKind::Waves(_) => (IconName::AudioWaveform, t.wave_signal),
                PanelKind::Pipeline(_) => (IconName::ChartNoAxesGantt, t.markers[0].stroke),
                PanelKind::Table(_) => (IconName::TableColumns, t.panel.icon_accent),
                PanelKind::Transaction(_) => (IconName::TableDetails, t.tx_relation_out),
                PanelKind::CursorRecords => (IconName::ListTree, t.panel.icon_accent),
                PanelKind::Settings => (IconName::Settings, t.panel.icon_muted),
                PanelKind::Start | PanelKind::Unsupported(_) => {
                    (IconName::Layers, t.panel.icon_muted)
                }
            };
            el.gap(t.px(6.))
                .min_w_0()
                .child(
                    Icon::new(icon)
                        .size(t.px(14.))
                        .color(if t.islands.is_some() {
                            color
                        } else {
                            t.panel.text_muted
                        }),
                )
        })
        .when_some(letter, |el, trace| {
            el.child(crate::traces::letter_badge(trace, cx))
        })
        .when(in_titlebar, |el| el.min_w_0().overflow_hidden())
        .child(
            div()
                .when(in_titlebar || t.compact_chrome, gpui_kit::Styled::truncate)
                .when(t.compact_chrome, |el| el.max_w(t.px(188.)))
                .child(if t.compact_chrome {
                    let numbered = t.islands.is_some()
                        || panels.iter().any(|other| {
                            other.id != id
                                && std::mem::discriminant(&other.kind)
                                    == std::mem::discriminant(&panel.kind)
                        });
                    panel.tab_title(numbered).into()
                } else {
                    panel_title.clone()
                }),
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
        .when(in_titlebar || t.compact_chrome, |el| {
            el.min_w_0().max_w_full()
        })
        .gap_1()
        .debug_selector(|| format!("tab-{}", id.0))
        .when(can_close, |el| {
            el.on_mouse_down(MouseButton::Middle, move |_, window, cx| {
                cx.stop_propagation();
                middle(window, cx);
            })
        })
        .when(t.compact_chrome, |tab| {
            tab.tooltip_show_delay(crate::ui::tooltip::delay(cx))
                .tooltip(move |w, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(panel_title.clone()).build(w, cx)
                })
        })
        .child(title.when(t.compact_chrome && !in_titlebar, |title| {
            let ws = ws.clone();
            let generation = generation.unwrap_or(app.doc.generation());
            title.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                drop(ws.update(cx, |ws, cx| {
                    if generation != ws.app.doc.generation() {
                        cx.stop_propagation();
                        return;
                    }
                    ws.dispatch(Command::Panels(PanelsCommand::Focus(id)), Some(window), cx);
                    if let Some(focus) = ws.panel_focus(id, cx) {
                        window.focus(&focus, cx);
                    }
                }));
            })
        }))
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
    reconciling: Rc<Cell<bool>>,
    revision: Option<u64>,
    installed_revision: Option<u64>,
    installed: Option<DockLayout>,
    echo: Option<DockLayout>,
    zoom: f32,
    right_width: Option<f32>,
    bottom_height: Option<f32>,
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
        let reconciling = Rc::new(Cell::new(false));
        let mut skin_handle = None;
        let area = cx.new(|cx| {
            let skin = DockSkin::new(cx);
            skin_handle = Some(skin.clone());
            base::DockArea::new("volna-dock", None, window, cx).with_renderer(Rc::new(
                DockRenderer {
                    main_window: window_id.is_none(),
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
        if window_id.is_none() {
            let mut zoomed = false;
            cx.observe(&area, move |host, area, cx| {
                let next = area.read(cx).is_zoomed();
                if next != zoomed {
                    zoomed = next;
                    if let Some(owner) = host.render_owner {
                        App::notify(cx, owner);
                    }
                }
            })
            .detach();
        }
        cx.subscribe(&area, |host, area, event, cx| {
            if host.reconciling.get() || !matches!(event, base::DockEvent::LayoutChanged) {
                return;
            }
            let snapshot = area.read(cx).dump(cx);
            let measured_width = snapshot
                .right_dock
                .as_ref()
                .map(|dock| f32::from(dock.size()));
            let measured_height = snapshot
                .bottom_dock
                .as_ref()
                .map(|dock| f32::from(dock.size()));
            let mut candidate = from_area(&snapshot, host.zoom);
            // Clamping to a narrow window must not overwrite the preferred
            // size when an unrelated tab or focus event arrives.
            if px_matches(measured_width, host.right_width)
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
            if px_matches(measured_height, host.bottom_height)
                && let Ok(layout) = &mut candidate
                && let (Some(bottom), Some(previous)) = (
                    &mut layout.bottom,
                    host.installed
                        .as_ref()
                        .and_then(|layout| layout.bottom.as_ref()),
                )
            {
                bottom.height = previous.height;
            }
            match candidate {
                Ok(layout)
                    if host
                        .echo
                        .as_ref()
                        .is_some_and(|echo| dock_layout_echo_matches(echo, &layout)) => {}
                Ok(layout) => {
                    let revision = host.revision.unwrap_or(0);
                    host.right_width = measured_width;
                    host.bottom_height = measured_height;
                    host.echo = Some(layout.clone());
                    let owner = host.owner.clone();
                    let window_id = host.window;
                    let weak = cx.weak_entity();
                    cx.defer(move |cx| {
                        let applied = owner.update(cx, |ws, cx| {
                            let before = ws.app.panels.revision();
                            match ws
                                .app
                                .panels
                                .layout()
                                .with_window(window_id, layout.clone())
                            {
                                Ok(proposed) => {
                                    ws.dispatch(
                                        Command::Panels(PanelsCommand::SetLayout {
                                            layout: proposed,
                                            from_revision: revision,
                                        }),
                                        None,
                                        cx,
                                    );
                                    let after = ws.app.panels.revision();
                                    let current = ws.app.panels.layout().for_window(window_id);
                                    if after != before
                                        || (before == revision
                                            && dock_layout_echo_matches(&current, &layout))
                                    {
                                        Some((after, current))
                                    } else {
                                        None
                                    }
                                }
                                Err(error) => {
                                    log::warn!("invalid window layout: {error:#}");
                                    None
                                }
                            }
                        });
                        match applied {
                            Ok(Some((after, local))) => {
                                drop(weak.update(cx, |host, _| {
                                    host.revision = Some(after);
                                    host.installed_revision = Some(after);
                                    host.installed = Some(local);
                                }));
                            }
                            Ok(None) => {
                                drop(weak.update(cx, |host, cx| {
                                    host.installed_revision = None;
                                    host.installed = None;
                                    if let Some(owner) = host.render_owner {
                                        App::notify(cx, owner);
                                    }
                                }));
                            }
                            Err(_) => {}
                        }
                    });
                }
                Err(e) => {
                    log::warn!("invalid dock layout: {e:#}");
                    // A rejected widget proposal must not remain on screen.
                    host.installed_revision = None;
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
            reconciling,
            revision: None,
            installed_revision: None,
            installed: None,
            echo: None,
            zoom: 1.0,
            right_width: None,
            bottom_height: None,
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
            self.installed_revision = None;
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
                } else if panel.kind.is_cursor_records() {
                    PanelView::CursorRecords(cx.new(|cx| {
                        CursorRecordsPanelView::new(
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
        let available_h = f32::from(bounds.size.height);
        let right_width = layout.right.as_ref().map(|dock| {
            if self.window.is_none() && crate::theme::compact_workspace(window, cx) {
                return f32::from(window.viewport_size().width);
            }
            let center_min = if layout.center.is_some() {
                (available * 0.5).min(320.0 * zoom)
            } else {
                0.0
            };
            (dock.width * zoom)
                .min((available - center_min).max(100.0))
                .max(100.0)
        });
        let bottom_height = layout.bottom.as_ref().map(|dock| {
            let center_min = if layout.center.is_some() {
                (available_h * 0.4).min(160.0 * zoom)
            } else {
                0.0
            };
            (dock.height * zoom)
                .min((available_h - center_min).max(96.0))
                .max(96.0)
        });
        let core_revision = app.panels.revision();
        let revision_changed = self.installed_revision != Some(core_revision);
        let zoom_changed = self.zoom.to_bits() != zoom.to_bits();
        let dock_size_changed = !px_matches(self.right_width, right_width)
            || !px_matches(self.bottom_height, bottom_height);
        if revision_changed || zoom_changed || dock_size_changed {
            if (app.panels.len() > 1 || layout.right.is_some() || layout.bottom.is_some())
                && (bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0))
            {
                window.on_next_frame(move |_, cx| cx.notify(render_owner));
                return;
            }
            let same_trees = !zoom_changed
                && (!revision_changed
                    || self.installed.as_ref().is_some_and(|installed| {
                        installed.center == layout.center
                            && installed.right.as_ref().map(|dock| &dock.layout)
                                == layout.right.as_ref().map(|dock| &dock.layout)
                            && installed.bottom.as_ref().map(|dock| &dock.layout)
                                == layout.bottom.as_ref().map(|dock| &dock.layout)
                    }));
            if same_trees {
                // Size and visibility belong to the dock, not its pane tree.
                // Keep Kit's tab groups and split widgets alive for these edits.
                self.reconciling.set(true);
                self.area.update(cx, |area, cx| {
                    if let Some(dock) = &layout.right {
                        area.set_dock_size(
                            base::DockPlacement::Right,
                            px(right_width.unwrap()),
                            window,
                            cx,
                        );
                        if area.is_dock_open(base::DockPlacement::Right) != dock.open {
                            area.toggle_dock(base::DockPlacement::Right, window, cx);
                        }
                    }
                    if let Some(dock) = &layout.bottom {
                        area.set_dock_size(
                            base::DockPlacement::Bottom,
                            px(bottom_height.unwrap()),
                            window,
                            cx,
                        );
                        if area.is_dock_open(base::DockPlacement::Bottom) != dock.open {
                            area.toggle_dock(base::DockPlacement::Bottom, window, cx);
                        }
                    }
                });
                self.reconciling.set(false);
            } else {
                let occupied_w = layout
                    .right
                    .as_ref()
                    .filter(|dock| dock.open)
                    .and(right_width)
                    .unwrap_or(0.0);
                let occupied_h = layout
                    .bottom
                    .as_ref()
                    .filter(|dock| dock.open)
                    .and(bottom_height)
                    .unwrap_or(0.0);
                let center_w = (available - occupied_w).max(1.0);
                let center_h = (available_h - occupied_h).max(1.0);
                let center = layout
                    .center
                    .as_ref()
                    .map_or_else(base::DockLayout::h_split, |tree| {
                        to_dock(tree, &self.views, center_w, center_h, cx)
                    });
                let right = layout.right.as_ref().map(|dock| {
                    to_dock(
                        &dock.layout,
                        &self.views,
                        right_width.unwrap(),
                        available_h,
                        cx,
                    )
                });
                let bottom = layout.bottom.as_ref().map(|dock| {
                    to_dock(
                        &dock.layout,
                        &self.views,
                        center_w,
                        bottom_height.unwrap(),
                        cx,
                    )
                });
                self.reconciling.set(true);
                self.area.update(cx, |area, cx| {
                    // Remove outgoing regions before installing trees: a moved
                    // panel must never belong to two regions during reconcile.
                    area.remove_dock(base::DockPlacement::Right, window, cx);
                    area.remove_dock(base::DockPlacement::Bottom, window, cx);
                    // Unsupported left dock can only arrive in a rejected proposal.
                    area.remove_dock(base::DockPlacement::Left, window, cx);
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
                    if let (Some(tree), Some(dock)) = (bottom, &layout.bottom) {
                        area.set_dock(base::DockPlacement::Bottom, tree, window, cx);
                        area.set_dock_size(
                            base::DockPlacement::Bottom,
                            px(bottom_height.unwrap()),
                            window,
                            cx,
                        );
                        if !dock.open {
                            area.toggle_dock(base::DockPlacement::Bottom, window, cx);
                        }
                    }
                });
                self.reconciling.set(false);
            }
            self.zoom = zoom;
            self.right_width = right_width;
            self.bottom_height = bottom_height;
            self.echo = from_area(&self.area.read(cx).dump(cx), zoom).ok();
            if let (Some(echo), Some(preferred)) = (&mut self.echo, &layout.right)
                && let Some(right) = &mut echo.right
            {
                right.width = preferred.width;
            }
            if let (Some(echo), Some(preferred)) = (&mut self.echo, &layout.bottom)
                && let Some(bottom) = &mut echo.bottom
            {
                bottom.height = preferred.height;
            }
            self.installed_revision = Some(core_revision);
            self.installed = Some(layout.clone());
        }
        if self.revision != Some(core_revision) {
            self.area.update(cx, |_, cx| cx.notify());
        }
        self.revision = Some(core_revision);
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

fn px_matches(a: Option<f32>, b: Option<f32>) -> bool {
    const PX_EPS: f32 = 0.5;
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => (a - b).abs() <= PX_EPS,
        _ => false,
    }
}

// Suppress spurious `DockEvent::LayoutChanged` echoes that GPUI emits after
// `set_center`/`set_dock` lays out the same tree with sub-pixel rounding, so
// they do not record bogus undo entries in `volna-core`.
fn tree_echo_matches(a: &Layout, b: &Layout) -> bool {
    const SHARE_EPS: f32 = 1e-4;
    match (a, b) {
        (
            Layout::Tabs {
                tabs: ta,
                active: aa,
            },
            Layout::Tabs {
                tabs: tb,
                active: ab,
            },
        ) => ta == tb && aa == ab,
        (
            Layout::Split {
                split: sa,
                sizes: sza,
                children: ca,
            },
            Layout::Split {
                split: sb,
                sizes: szb,
                children: cb,
            },
        ) => {
            sa == sb
                && sza.len() == szb.len()
                && ca.len() == cb.len()
                && sza.iter().zip(szb).all(|(x, y)| (x - y).abs() <= SHARE_EPS)
                && ca.iter().zip(cb).all(|(x, y)| tree_echo_matches(x, y))
        }
        _ => false,
    }
}

fn opt_tree_echo_matches(a: Option<&Layout>, b: Option<&Layout>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => tree_echo_matches(a, b),
        _ => false,
    }
}

fn dock_layout_echo_matches(a: &DockLayout, b: &DockLayout) -> bool {
    opt_tree_echo_matches(a.center.as_ref(), b.center.as_ref())
        && match (&a.right, &b.right) {
            (None, None) => true,
            (Some(ra), Some(rb)) => {
                ra.open == rb.open
                    && px_matches(Some(ra.width), Some(rb.width))
                    && tree_echo_matches(&ra.layout, &rb.layout)
            }
            _ => false,
        }
        && match (&a.bottom, &b.bottom) {
            (None, None) => true,
            (Some(ba), Some(bb)) => {
                ba.open == bb.open
                    && px_matches(Some(ba.height), Some(bb.height))
                    && tree_echo_matches(&ba.layout, &bb.layout)
            }
            _ => false,
        }
        && a.windows.len() == b.windows.len()
        && a.windows
            .iter()
            .zip(&b.windows)
            .all(|((ka, va), (kb, vb))| ka == kb && tree_echo_matches(va, vb))
}

fn from_area(state: &base::DockAreaState, zoom: f32) -> Result<DockLayout> {
    ensure!(state.left_dock.is_none(), "unsupported dock region");
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
        bottom: state
            .bottom_dock
            .as_ref()
            .filter(|dock| !empty(dock.panel()))
            .map(|dock| -> Result<BottomDock> {
                Ok(BottomDock {
                    layout: from_dock(dock.panel())?,
                    height: f32::from(dock.size()) / zoom,
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
    let mut menu = menu;
    let destinations: &[(&str, Placement)] = match placement {
        Some(Placement::Right) => &[
            ("Move to center", Placement::Center),
            ("Move to bottom dock", Placement::Bottom),
        ],
        Some(Placement::Bottom) => &[
            ("Move to center", Placement::Center),
            ("Move to right sidebar", Placement::Right),
        ],
        _ => &[
            ("Move to right sidebar", Placement::Right),
            ("Move to bottom dock", Placement::Bottom),
        ],
    };
    for &(label, destination) in destinations {
        let ws = ws.clone();
        menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
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
        }));
    }
    menu
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
            (
                "Follow shared transaction",
                LinkDim::Transaction,
                links.transaction,
            ),
        ] {
            menu = menu
                .item(item(label, PanelsCommand::ToggleLink { panel: id, dim }).checked(linked));
        }
    }
    if let Some(settings) = ws
        .upgrade()
        .and_then(|owner| owner.read(cx).app.panels.pipeline(id).map(|p| p.navigator))
    {
        use volna_core::pipeline::NavigatorCommand;
        menu = menu.separator();
        for (label, command) in [
            ("Instruction navigator", NavigatorCommand::Toggle),
            ("Widen instruction navigator", NavigatorCommand::Widen),
            ("Narrow instruction navigator", NavigatorCommand::Narrow),
        ] {
            let owner = ws.clone();
            menu = menu.item(
                PopupMenuItem::new(label)
                    .checked(command == NavigatorCommand::Toggle && settings.visible)
                    .on_click(move |_, window, cx| {
                        drop(owner.update(cx, |ws, cx| {
                            ws.dispatch_if_current(
                                generation,
                                Command::PipelineNavigator(id, command),
                                Some(window),
                                cx,
                            );
                        }));
                    }),
            );
        }
    }
    if let Some(settings) = ws
        .upgrade()
        .and_then(|owner| owner.read(cx).app.panels.pipeline(id).map(|p| p.occupancy))
    {
        use volna_core::pipeline::OccupancyCommand;
        menu = menu.separator();
        for (label, command, checked) in [
            ("Occupancy plot", OccupancyCommand::Toggle, settings.visible),
            ("Taller occupancy plot", OccupancyCommand::Taller, false),
            ("Shorter occupancy plot", OccupancyCommand::Shorter, false),
        ] {
            let owner = ws.clone();
            menu = menu.item(PopupMenuItem::new(label).checked(checked).on_click(
                move |_, window, cx| {
                    drop(owner.update(cx, |ws, cx| {
                        ws.dispatch_if_current(
                            generation,
                            Command::PipelineOccupancy(id, command),
                            Some(window),
                            cx,
                        );
                    }));
                },
            ));
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
    let placement = ws
        .upgrade()
        .and_then(|ws| ws.read(cx).app.panels.layout().placement(id));
    let in_sidebar = placement == Some(Placement::Right);
    let in_bottom = placement == Some(Placement::Bottom);
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
    if in_bottom {
        let ws = ws.clone();
        menu = menu.item(
            PopupMenuItem::new("Hide bottom dock").on_click(move |_, window, cx| {
                drop(ws.update(cx, |ws, cx| {
                    let layout = ws.app.panels.layout();
                    if layout.placement(id) == Some(Placement::Bottom)
                        && layout.bottom.as_ref().is_some_and(|dock| dock.open)
                    {
                        ws.dispatch_if_current(
                            generation,
                            Command::Panels(PanelsCommand::ToggleBottomDock),
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
        PopupMenuItem::new(if in_sidebar || in_bottom {
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
            let total: f64 = sizes.iter().map(|p| f64::from(f32::from(*p))).sum();
            ensure!(total.is_finite() && total > 0.0, "invalid sizes");
            Ok(Layout::Split {
                split: if *axis == 0 {
                    Axis::Horizontal
                } else {
                    Axis::Vertical
                },
                sizes: sizes
                    .iter()
                    .map(|p| (f64::from(f32::from(*p)) / total) as f32)
                    .collect(),
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
                | "volna.cursor"
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
    derived_signal: Option<Entity<crate::derived::DerivedEditor>>,
    pub(crate) pipeline_sources: Option<Entity<crate::pipeline_sources::PipelineSourcesView>>,
    pub(crate) pipeline_order: Option<Entity<crate::pipeline_order::PipelineOrderView>>,
    pub(crate) table_columns: Option<Entity<crate::table_columns::ColumnPicker>>,
    pub(crate) table_filters: Option<Entity<crate::table_filters::FilterRow>>,
    pub(crate) table_copy: Option<gpui_kit::Task<()>>,
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
            derived_signal: None,
            pipeline_sources: None,
            pipeline_order: None,
            ws,
            id,
            generation,
            name,
            focus,
            table_columns: None,
            table_filters: None,
            table_copy: None,
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
        use gpui_kit::{assets::IconName, component::Icon};
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
        let link_button = |name, icon: Icon, linked, dim, tooltip: &'static str| {
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
        let compact_chrome = theme(cx).compact_chrome;
        let mut buttons = if compact_chrome {
            Vec::new()
        } else {
            vec![
                link_button(
                    "link-view",
                    IconName::Scan.into(),
                    link.viewport,
                    LinkDim::Viewport,
                    "Follow shared viewport (L)",
                ),
                link_button(
                    "link-cursor",
                    IconName::Crosshair.into(),
                    link.cursor,
                    LinkDim::Cursor,
                    "Follow shared cursor (Shift+L)",
                ),
                link_button(
                    "link-transaction",
                    Icon::empty().path(volna_core::icons::IconName::TransactionSelect.path()),
                    link.transaction,
                    LinkDim::Transaction,
                    "Follow shared transaction",
                ),
            ]
        };
        buttons.push(
            action_button("new-tab", IconName::Plus, cx)
                .ghost()
                .tooltip("New waveform tab")
                .debug_selector({
                    let id = self.id.0;
                    move || format!("new-tab-{id}")
                })
                .on_click(cx.listener(|view, _, window, cx| {
                    view.dispatch(PanelsCommand::NewTab { group_of: view.id }, window, cx);
                })),
        );
        let workspace = self.ws.upgrade()?;
        let app = &workspace.read(cx).app;
        if let Some(waves) = app.panels.waves(self.id) {
            use volna_core::computed::Command as Formula;
            let eligible = waves.can_derive_signal();
            let resume = app
                .derived_editor
                .as_ref()
                .is_some_and(|e| e.panel == self.id && e.has_suspended_formula());
            let selected = waves
                .selected
                .first()
                .copied()
                .filter(|_| eligible && waves.selected.len() == 1);
            let commands = [
                (
                    "derive-signal",
                    if resume { "Resume formula" } else { "Derive" },
                    if resume {
                        Some(Formula::ResumeFormula)
                    } else {
                        eligible.then_some(Formula::Create(None))
                    },
                ),
                ("measure-signal", "Measure", selected.map(Formula::Measure)),
            ];
            for (index, (id, label, command)) in commands.into_iter().enumerate() {
                let owner = self.ws.clone();
                let panel = self.id;
                let generation = self.generation;
                buttons.insert(
                    index,
                    Button::new(id)
                        .label(label)
                        .ghost()
                        .small()
                        .disabled(command.is_none())
                        .debug_selector(move || id.into())
                        .on_click(move |_, window, cx| {
                            if let Some(command) = command.clone() {
                                drop(owner.update(cx, |ws, cx| {
                                    ws.dispatch_if_current(
                                        generation,
                                        Command::Derived(panel, command),
                                        Some(window),
                                        cx,
                                    );
                                }));
                            }
                        }),
                );
            }
        }
        if let Some((
            follow,
            activity,
            navigator_visible,
            occupancy_visible,
            source_count,
            order_label,
            order_is_custom,
            hidden_stage_count,
            suggested_stage,
        )) = self.ws.upgrade().and_then(|owner| {
            let ws = owner.read(cx);
            let pipeline = ws.app.panels.pipeline(self.id)?;
            Some((
                pipeline.follow,
                pipeline.activity(&ws.app.doc),
                pipeline.navigator.visible,
                pipeline.occupancy.visible,
                pipeline.sources.len(),
                pipeline.order.short_label(),
                !pipeline.order.is_start(),
                pipeline.stage_visibility.hidden_count(),
                pipeline.suggested_order_stage().map(str::to_owned),
            ))
        }) {
            use crate::ui::{Icon as VolnaIcon, IconName as VolnaIconName};
            use volna_core::pipeline::{ActivityCommand, FollowActivity};
            let generation = self.generation;
            let panel = self.id;
            let drawer_open = self
                .pipeline_sources
                .as_ref()
                .is_some_and(|view| view.read(cx).is_drawer_open());
            let order_open = self
                .pipeline_order
                .as_ref()
                .is_some_and(|view| view.read(cx).is_open());
            let order_tooltip = if hidden_stage_count > 0 {
                let stage_word = if hidden_stage_count == 1 {
                    "stage"
                } else {
                    "stages"
                };
                format!(
                    "Stages & Order · {hidden_stage_count} {stage_word} hidden · Order: {order_label} · Click to show/hide stages or order rows"
                )
            } else if let Some(stage) = &suggested_stage {
                format!(
                    "Stages & Order · Order: Start · Batched fetches detected — try ordering by \"{stage}\""
                )
            } else {
                format!(
                    "Stages & Order · Order: {order_label} · Click to show/hide stages or order rows"
                )
            };
            let t = *theme(cx);
            let mut order_btn = Button::new("pipeline-order-btn")
                .debug_selector(|| "pipeline-order-btn".into())
                .ghost()
                .small()
                .selected(order_is_custom || hidden_stage_count > 0 || order_open)
                .tooltip(order_tooltip)
                .on_click(cx.listener(|view, ev: &gpui_kit::ClickEvent, window, cx| {
                    let entity = view
                        .pipeline_order
                        .get_or_insert_with(|| {
                            cx.new(|cx| {
                                crate::pipeline_order::PipelineOrderView::new(
                                    view.ws.clone(),
                                    view.id,
                                    view.generation,
                                    cx,
                                )
                            })
                        })
                        .clone();
                    let pos = ev.position();
                    entity.update(cx, |order_view, cx| {
                        order_view.toggle_at(pos, window, cx);
                    });
                    cx.notify();
                }));
            let base_label = if order_is_custom {
                format!("Stages & Order · {order_label}")
            } else {
                "Stages & Order".into()
            };
            if hidden_stage_count > 0 || (suggested_stage.is_some() && !order_is_custom) {
                let mut content = div().flex().items_center().gap(t.px(5.0));
                if suggested_stage.is_some() && !order_is_custom && hidden_stage_count == 0 {
                    content = content.child(
                        div()
                            .debug_selector(|| "pipeline-order-suggest-dot".into())
                            .size(t.px(6.0))
                            .rounded_full()
                            .bg(t.panel.icon_accent),
                    );
                }
                content = content.child(gpui_kit::SharedString::from(base_label));
                if hidden_stage_count > 0 {
                    content = content.child(
                        div()
                            .debug_selector(|| "pipeline-stages-hidden-badge".into())
                            .flex()
                            .items_center()
                            .gap(t.px(3.0))
                            .px(t.px(4.0))
                            .py(t.px(1.0))
                            .rounded(t.px(3.0))
                            .border_1()
                            .border_color(t.border_focused.opacity(0.55))
                            .bg(t.selection.bg)
                            .font_family(t.mono_font)
                            .text_size(px(t.ui_size_small * 0.88))
                            .text_color(t.panel.icon_accent)
                            .child(
                                VolnaIcon::new(VolnaIconName::EyeOff)
                                    .size(t.px(11.0))
                                    .color(t.panel.icon_accent),
                            )
                            .child(gpui_kit::SharedString::from(format!(
                                "{hidden_stage_count} hidden"
                            ))),
                    );
                }
                order_btn = order_btn.child(content);
            } else {
                order_btn = order_btn.label(base_label);
            }
            buttons.insert(0, order_btn);
            buttons.insert(
                1,
                Button::new("pipeline-sources-btn")
                    .debug_selector(|| "pipeline-sources-btn".into())
                    .label(if source_count > 1 {
                        format!("Sources ({source_count})")
                    } else {
                        "Sources".into()
                    })
                    .ghost()
                    .small()
                    .selected(drawer_open)
                    .tooltip("Add, remove, reorder, or alias merged pipeline sources")
                    .on_click(cx.listener(|view, _, window, cx| {
                        let entity = view
                            .pipeline_sources
                            .get_or_insert_with(|| {
                                cx.new(|cx| {
                                    crate::pipeline_sources::PipelineSourcesView::new(
                                        view.ws.clone(),
                                        view.id,
                                        view.generation,
                                        window,
                                        cx,
                                    )
                                })
                            })
                            .clone();
                        entity.update(
                            cx,
                            crate::pipeline_sources::PipelineSourcesView::toggle_drawer,
                        );
                        cx.notify();
                    })),
            );
            if !compact_chrome {
                let owner = self.ws.clone();
                let navigator_hint = if navigator_visible {
                    "Hide instruction navigator. Drag its left edge to resize."
                } else {
                    "Show instruction navigator"
                };
                buttons.insert(
                    0,
                    Button::new("instruction-navigator")
                        .label("Navigator")
                        .ghost()
                        .small()
                        .selected(navigator_visible)
                        .tooltip(navigator_hint)
                        .on_click(move |_, window, cx| {
                            drop(owner.update(cx, |ws, cx| {
                                ws.dispatch_if_current(
                                    generation,
                                    Command::PipelineNavigator(
                                        panel,
                                        volna_core::pipeline::NavigatorCommand::Toggle,
                                    ),
                                    Some(window),
                                    cx,
                                );
                            }));
                        }),
                );
                let owner = self.ws.clone();
                buttons.insert(
                    1,
                    Button::new("pipeline-occupancy")
                        .label("Occupancy")
                        .ghost()
                        .small()
                        .selected(occupancy_visible)
                        .tooltip("Show or hide occupancy; drag its top edge to resize")
                        .on_click(move |_, window, cx| {
                            drop(owner.update(cx, |ws, cx| {
                                ws.dispatch_if_current(
                                    generation,
                                    Command::PipelineOccupancy(
                                        panel,
                                        volna_core::pipeline::OccupancyCommand::Toggle,
                                    ),
                                    Some(window),
                                    cx,
                                );
                            }));
                        }),
                );
            }
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
                        .small()
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
                    .small()
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
                    .small()
                    .tooltip("Show the selected record in Record details (⏎)")
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
                let pipeline = workspace.app.panels.get(self.id)?.kind.pipeline()?;
                let track = pipeline.sources.iter().find_map(|s| s.track.track())?;
                Some(
                    workspace
                        .app
                        .doc
                        .session(track.trace)?
                        .tracks()
                        .iter()
                        .any(|declaration| declaration.id == track.item),
                )
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
                    .small()
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
        let editor = ws
            .read(cx)
            .app
            .derived_editor
            .as_ref()
            .filter(|e| e.panel == self.id)
            .map(|e| e.id);
        if let Some(editor_id) = editor {
            if self
                .derived_signal
                .as_ref()
                .is_none_or(|view| view.read(cx).id != editor_id)
            {
                self.derived_signal = Some(cx.new(|cx| {
                    crate::derived::DerivedEditor::new(self.ws.clone(), self.id, window, cx)
                }));
            }
        } else {
            self.derived_signal = None;
        }
        let is_pipeline = ws.read(cx).app.panels.pipeline(self.id).is_some();
        if is_pipeline && self.pipeline_sources.is_none() {
            self.pipeline_sources = Some(cx.new(|cx| {
                crate::pipeline_sources::PipelineSourcesView::new(
                    self.ws.clone(),
                    self.id,
                    self.generation,
                    window,
                    cx,
                )
            }));
        } else if !is_pipeline {
            self.pipeline_sources = None;
        }
        if is_pipeline && self.pipeline_order.is_none() {
            self.pipeline_order = Some(cx.new(|cx| {
                crate::pipeline_order::PipelineOrderView::new(
                    self.ws.clone(),
                    self.id,
                    self.generation,
                    cx,
                )
            }));
        } else if !is_pipeline {
            self.pipeline_order = None;
        }
        div()
            .id(("wave-panel", self.id.0))
            .size_full()
            .relative()
            .flex()
            .flex_col()
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
                el.when_some(
                    self.pipeline_sources.clone(),
                    gpui_kit::ParentElement::child,
                )
                .child(div().flex_1().min_h_0().w_full().child(
                    crate::canvas::PanelCanvas::new(ws, self.id, self.generation).waves(waves),
                ))
            })
            .when_some(self.pipeline_order.clone(), gpui_kit::ParentElement::child)
            .when_some(self.derived_signal.clone(), gpui_kit::ParentElement::child)
            .into_any_element()
    }
}

struct DockRenderer {
    main_window: bool,
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
        let t = *crate::theme::theme(cx);
        self.skin.center_frame(w, cx).when_some(t.islands, |el, i| {
            let insets = crate::islands::insets(t.scale(i.gap), w.scale_factor());
            el.relative()
                .paddings(insets)
                .child(crate::islands::frame_corners(insets, cx))
        })
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
        let t = *crate::theme::theme(cx);
        let content = if let Some(i) = t.islands {
            div()
                .size_full()
                .paddings(crate::islands::insets(t.scale(i.gap), w.scale_factor()))
                .child(crate::islands::rounded(content, cx))
                .into_any_element()
        } else {
            content
        };
        if self.main_window && crate::theme::compact_workspace(w, cx) {
            return content;
        }
        self.resize.render(dock, content, self.area.clone(), cx)
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
        })
    }
}
struct TabRenderer {
    skin: Rc<dyn base::TabGroupRenderer>,
    hide_tab_bar: Rc<Cell<bool>>,
}
impl base::TabGroupRenderer for TabRenderer {
    fn frame(&self, group: &base::TabGroupContext, w: &mut Window, cx: &mut App) -> Stateful<Div> {
        let t = *crate::theme::theme(cx);
        self.skin.frame(group, w, cx).when_some(t.islands, |el, i| {
            el.bg(t.panel.bg).p_0().when(group.is_zoomed(), |el| {
                let insets = crate::islands::insets(t.scale(i.gap), w.scale_factor());
                el.relative()
                    .bg(gpui_kit::transparent_black())
                    .paddings(insets)
                    .child(crate::islands::frame_corners(insets, cx))
            })
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
        } else if crate::theme::theme(cx).compact_chrome {
            crate::dock_tabs::render(group, w, cx)
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
        self.skin.render_active_panel(panel, group, w, cx)
    }

    fn render_drop_indicator(
        &self,
        indicator: base::DropIndicator,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let t = *crate::theme::theme(cx);
        if t.islands.is_some() {
            let target = indicator.to();
            return Some(
                div()
                    .absolute()
                    .left(target.origin().x)
                    .top(target.origin().y)
                    .w(target.size().width)
                    .h(target.size().height)
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
