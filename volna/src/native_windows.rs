//! Auxiliary panel windows share the workspace's models and command dispatch.

use std::collections::BTreeMap;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Context, Entity, FocusHandle, IntoElement, Render,
    SharedString, WeakEntity, Window, WindowControlArea, div, px, size,
};
use volna_core::panels::{PanelId, Panels, PanelsCommand, Placement, WindowId};
use volna_core::{Command, Instant};

use crate::app::Workspace;
use crate::dock::DockHost;
use crate::theme::theme;
use crate::ui::{Side, render_window_controls};

struct Auxiliary {
    handle: AnyWindowHandle,
    view: Entity<AuxiliaryWindow>,
    dock: Entity<DockHost>,
    focus: FocusHandle,
}

pub(crate) struct Windows {
    main: AnyWindowHandle,
    auxiliary: BTreeMap<WindowId, Auxiliary>,
    generation: u64,
    queued: bool,
    closing: bool,
    main_panel: Option<PanelId>,
    focus_target: Option<AnyWindowHandle>,
}

impl Windows {
    pub fn new(main: AnyWindowHandle) -> Self {
        Self {
            main,
            auxiliary: BTreeMap::new(),
            generation: 0,
            queued: false,
            closing: false,
            main_panel: None,
            focus_target: Some(main),
        }
    }

    pub fn main(&self) -> AnyWindowHandle {
        self.main
    }

    pub fn id_for(&self, handle: AnyWindowHandle) -> Option<WindowId> {
        self.auxiliary
            .iter()
            .find_map(|(&id, window)| (window.handle == handle).then_some(id))
    }

    pub fn contains_panel(&self, handle: AnyWindowHandle, panel: PanelId, panels: &Panels) -> bool {
        self.handle_for(panel, panels) == Some(handle)
    }

    pub fn dock(&self, id: WindowId) -> Option<Entity<DockHost>> {
        self.auxiliary.get(&id).map(|window| window.dock.clone())
    }

    pub fn focus_for(&self, handle: AnyWindowHandle) -> Option<FocusHandle> {
        self.auxiliary
            .values()
            .find(|window| window.handle == handle)
            .map(|window| window.focus.clone())
    }

    pub fn handle_for(&self, panel: PanelId, panels: &Panels) -> Option<AnyWindowHandle> {
        match panels.layout().placement(panel)? {
            Placement::Window(id) => self.auxiliary.get(&id).map(|window| window.handle),
            Placement::Center | Placement::Right => Some(self.main),
        }
    }

    pub fn main_panel(&self) -> Option<PanelId> {
        self.main_panel
    }

    pub fn invalidate(&self, cx: &mut App) {
        for window in self.auxiliary.values() {
            DockHost::invalidate_panels(&window.dock, cx);
            cx.notify(window.view.entity_id());
        }
    }

    pub fn close(&mut self, cx: &mut App) {
        self.closing = true;
        for (_, auxiliary) in std::mem::take(&mut self.auxiliary) {
            drop(
                auxiliary
                    .handle
                    .update(cx, |_, window, _| window.remove_window()),
            );
        }
    }
}

pub(crate) fn schedule(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    if ws.windows.queued || ws.windows.closing {
        return;
    }
    if ws.windows.auxiliary.is_empty() && ws.app.panels.layout().windows.is_empty() {
        ws.windows.main_panel = Some(ws.app.panels.focused_id());
        return;
    }
    ws.windows.queued = true;
    let owner = cx.weak_entity();
    cx.defer(move |cx| {
        if let Some(target) = reconcile(owner, cx) {
            drop(target.update(cx, |_, window, _| {
                if !window.is_window_active() {
                    window.activate_window();
                    window.refresh();
                }
            }));
        }
    });
}

fn reconcile(owner: WeakEntity<Workspace>, cx: &mut App) -> Option<AnyWindowHandle> {
    if owner.upgrade().is_none_or(|ws| ws.read(cx).windows.closing) {
        return None;
    }
    let Ok((generation, stale, missing)) = owner.update(cx, |ws, _| {
        let generation = ws.app.doc.generation();
        let stale: Vec<_> = ws
            .windows
            .auxiliary
            .keys()
            .filter(|id| {
                ws.windows.generation != generation
                    || !ws.app.panels.layout().windows.contains_key(id)
            })
            .copied()
            .collect();
        let stale: Vec<_> = stale
            .into_iter()
            .map(|id| ws.windows.auxiliary.remove(&id).unwrap())
            .collect();
        let missing: Vec<_> = ws
            .app
            .panels
            .layout()
            .windows
            .keys()
            .filter(|id| !ws.windows.auxiliary.contains_key(id))
            .copied()
            .collect();
        ws.windows.generation = generation;
        (generation, stale, missing)
    }) else {
        return None;
    };
    // Creating a GPUI window immediately renders its root. Release the session
    // borrow first so that the new view can read and dispatch through its owner.
    for auxiliary in stale {
        drop(
            auxiliary
                .handle
                .update(cx, |_, window, _| window.remove_window()),
        );
    }
    let mut failed = Vec::new();
    for id in missing {
        let mut options = crate::window_options(cx);
        options.focus = false;
        options.window_min_size = Some(size(px(360.0), px(240.0)));
        let result = gpui_kit::open_window(options, cx, |window, cx| {
            let dock = cx.new(|cx| DockHost::new(owner.clone(), Some(id), window, cx));
            let weak = owner.clone();
            window.on_window_should_close(cx, move |window, cx| {
                drop(weak.update(cx, |ws, cx| {
                    if ws.app.doc.generation() != generation
                        || ws
                            .windows
                            .auxiliary
                            .get(&id)
                            .is_none_or(|entry| entry.handle != window.window_handle())
                    {
                        return;
                    }
                    ws.windows.auxiliary.remove(&id);
                    ws.dispatch_if_current(
                        generation,
                        Command::Panels(PanelsCommand::ReturnWindow(id)),
                        Some(window),
                        cx,
                    );
                }));
                true
            });
            let view =
                cx.new(|cx| AuxiliaryWindow::new(owner.clone(), id, dock.clone(), window, cx));
            let focus = view.read(cx).focus.clone();
            let handle = window.window_handle();
            drop(owner.update(cx, |ws, _| {
                ws.windows.auxiliary.insert(
                    id,
                    Auxiliary {
                        handle,
                        view: view.clone(),
                        dock,
                        focus,
                    },
                );
            }));
            view
        });
        if let Err(error) = result {
            log::error!("failed to open panel window: {error:#}");
            failed.push(id);
        }
    }
    owner
        .update(cx, |ws, cx| {
            ws.windows.queued = false;
            for id in failed {
                ws.windows.auxiliary.remove(&id);
                ws.dispatch(Command::Panels(PanelsCommand::ReturnWindow(id)), None, cx);
            }
            let focused = ws.app.panels.focused_id();
            let target = ws.windows.handle_for(focused, &ws.app.panels);
            // Wayland activation is asynchronous. Repeating a request before focus
            // arrives can orphan GPUI tokens and leave desktop startup feedback active.
            let activation = target.filter(|_| target != ws.windows.focus_target);
            ws.windows.focus_target = target;
            if target == Some(ws.windows.main) {
                ws.windows.main_panel = Some(focused);
            }
            let open: Vec<_> = ws
                .windows
                .auxiliary
                .values()
                .map(|window| window.handle.window_id())
                .chain([ws.windows.main.window_id()])
                .collect();
            ws.canvas.retain(|id, _| open.contains(id));
            if ws.app.doc.generation() != generation
                || ws
                    .app
                    .panels
                    .layout()
                    .windows
                    .keys()
                    .ne(ws.windows.auxiliary.keys())
            {
                schedule(ws, cx);
            }
            activation
        })
        .ok()
        .flatten()
}

struct AuxiliaryWindow {
    owner: WeakEntity<Workspace>,
    id: WindowId,
    dock: Entity<DockHost>,
    focused: Option<PanelId>,
    focus: FocusHandle,
}

impl AuxiliaryWindow {
    fn new(
        owner: WeakEntity<Workspace>,
        id: WindowId,
        dock: Entity<DockHost>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                return;
            }
            drop(this.owner.update(cx, |ws, cx| {
                let panel = this
                    .focused
                    .filter(|panel| {
                        ws.app.panels.layout().placement(*panel) == Some(Placement::Window(this.id))
                    })
                    .or_else(|| {
                        ws.app
                            .panels
                            .layout()
                            .windows
                            .get(&this.id)?
                            .visible()
                            .first()
                            .copied()
                    });
                if let Some(panel) = panel {
                    ws.activate_panel(panel, window, cx);
                }
            }));
        })
        .detach();
        Self {
            owner,
            id,
            dock,
            focused: None,
            focus: cx.focus_handle(),
        }
    }
}

impl Render for AuxiliaryWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            return gpui_kit::Empty.into_any_element();
        };
        let dock = self.dock.clone();
        let render_owner = cx.entity_id();
        let (title, overlays) = owner.update(cx, |ws, cx| {
            let animating = ws.app.is_animating();
            if ws.app.tick(Instant::now()) {
                window.request_animation_frame();
            }
            ws.after(Some(window), cx);
            if animating {
                if let Some(main) = &ws.dock {
                    DockHost::invalidate_panels(main, cx);
                }
                ws.windows.invalidate(cx);
                cx.notify();
            }
            dock.update(cx, |dock, cx| {
                dock.sync(&ws.app, render_owner, window, cx);
            });
            let focused = ws.app.panels.focused_id();
            let local =
                ws.app.panels.layout().placement(focused) == Some(Placement::Window(self.id));
            let focus = local.then(|| dock.read(cx).focus(focused, cx)).flatten();
            if local {
                self.focused = Some(focused);
                if ws.panel_focus_pending
                    && let Some(focus) = &focus
                {
                    window.focus(focus, cx);
                    ws.panel_focus_pending = false;
                }
            }
            let title = self
                .focused
                .and_then(|id| ws.app.panels.get(id))
                .map_or_else(
                    || "Volna".to_owned(),
                    |panel| format!("{} — Volna", panel.title()),
                );
            (title, ws.window_overlays(window, cx))
        });
        window.set_window_title(&title);
        let t = *theme(cx);
        let header = div()
            .id("auxiliary-titlebar")
            .flex()
            .flex_none()
            .items_center()
            .h(px(t.titlebar_height))
            .bg(crate::theme::chrome_background(cx))
            .text_color(t.bar.text)
            .when(cfg!(target_os = "macos") && !window.is_fullscreen(), |el| {
                el.pl(px(80.0 * t.zoom))
            })
            .children(render_window_controls(Side::Left, window, cx))
            .child(
                div()
                    .id("auxiliary-titlebar-drag")
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .px_3()
                    .overflow_hidden()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(gpui_kit::MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    })
                    .child(SharedString::from(title)),
            )
            .children(render_window_controls(Side::Right, window, cx));
        let root = Workspace::panel_actions(
            div()
                .id("auxiliary-workspace")
                .key_context("Workspace")
                .track_focus(&self.focus)
                .size_full()
                .flex()
                .flex_col()
                .bg(t.editor.bg)
                .text_color(t.editor.text)
                .font_family(t.ui_font)
                .text_size(px(t.ui_size)),
            self.owner.clone(),
        );
        root.child(header)
            .child(div().flex_1().min_h_0().child(dock.read(cx).area.clone()))
            .children(overlays)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use std::sync::Arc;
    use volna_core::testing::{ProceduralTrace, a_all};

    #[gpui_kit::test]
    fn restored_windows_preserve_the_saved_panel_and_activate_its_host(cx: &mut TestAppContext) {
        use volna_core::workspace::Workspace as SavedWorkspace;

        cx.update(crate::init_app);
        let session = Arc::new(ProceduralTrace::new(100));
        let mut source = volna_core::App::new();
        source.set_session(session.clone());
        source.handle(Command::AddVars(a_all(vec![0])));
        let main_panel = source.panels.focused_id();
        for _ in 0..2 {
            source.handle(Command::Panels(PanelsCommand::NewTab {
                group_of: main_panel,
            }));
            source.handle(Command::Panels(PanelsCommand::MoveToNewWindow(
                source.panels.focused_id(),
            )));
        }
        let detached_panel = source.panels.focused_id();
        let trace_uri = "file:///recording.vtr";
        let mut saved =
            SavedWorkspace::capture(&source, |_| Ok(Some(trace_uri.into())), None).unwrap();
        for focused in [main_panel, detached_panel] {
            saved.focused = focused;
            let owner = cx.update(|cx| crate::open_main_window(cx, false).unwrap());
            let main = owner.read_with(cx, |ws, _| ws.windows.main);
            main.update(cx, |_, window, _| window.activate_window())
                .unwrap();
            cx.run_until_parked();
            owner.update(cx, |ws, cx| {
                ws.set_session(session.clone(), cx);
                saved
                    .prepare(&ws.app, trace_uri, "file:///recording.volna.json")
                    .unwrap()
                    .commit(&mut ws.app)
                    .unwrap();
                ws.after(None, cx);
            });
            cx.run_until_parked();
            let target = owner.read_with(cx, |ws, _| {
                assert_eq!(ws.app.panels.focused_id(), focused);
                assert_eq!(ws.windows.auxiliary.len(), 2);
                ws.app.panels.validate().unwrap();
                ws.windows.handle_for(focused, &ws.app.panels).unwrap()
            });
            assert_eq!(cx.update(|cx| cx.active_window()), Some(target));
            if focused == main_panel {
                assert_eq!(target, main);
            } else {
                assert_ne!(target, main);
            }
            assert!(cx.update(|cx| reconcile(owner.downgrade(), cx)).is_none());
            owner.update(cx, |ws, cx| ws.windows.close(cx));
            main.update(cx, |_, window, _| window.remove_window())
                .unwrap();
        }
    }

    #[gpui_kit::test]
    fn window_activation_is_requested_once_without_waiting_for_desktop_focus(
        cx: &mut TestAppContext,
    ) {
        cx.update(crate::init_app);
        let owner = cx.update(|cx| crate::open_main_window(cx, false).unwrap());
        let main = owner.read_with(cx, |ws, _| ws.windows.main);
        owner.update(cx, |ws, cx| {
            ws.set_session(Arc::new(ProceduralTrace::new(100)), cx);
            ws.dispatch(Command::AddVars(a_all(vec![0])), None, cx);
        });
        cx.run_until_parked();
        let panel = owner.read_with(cx, |ws, _| ws.app.panels.focused_id());
        for restore in [false, true] {
            owner.update(cx, |ws, cx| {
                ws.windows.queued = true;
                ws.dispatch(
                    if restore {
                        Command::Undo
                    } else {
                        Command::Panels(PanelsCommand::MoveToNewWindow(panel))
                    },
                    None,
                    cx,
                );
                ws.panel_focus_pending = false;
            });
            let auxiliary = cx.update(|cx| reconcile(owner.downgrade(), cx)).unwrap();
            assert_ne!(auxiliary, main);
            owner.read_with(cx, |ws, _| {
                assert_eq!(
                    ws.windows.handle_for(panel, &ws.app.panels),
                    Some(auxiliary)
                );
            });
            // Leave desktop activation unacknowledged while more frames reconcile.
            for _ in 0..3 {
                owner.update(cx, |ws, _| {
                    ws.windows.queued = true;
                    ws.panel_focus_pending = true;
                });
                assert!(cx.update(|cx| reconcile(owner.downgrade(), cx)).is_none());
            }
            owner.update(cx, |ws, cx| {
                ws.windows.queued = true;
                let id = ws.windows.id_for(auxiliary).unwrap();
                ws.dispatch(Command::Panels(PanelsCommand::ReturnWindow(id)), None, cx);
                ws.panel_focus_pending = false;
            });
            assert_eq!(cx.update(|cx| reconcile(owner.downgrade(), cx)), Some(main));
            assert!(cx.update(|cx| reconcile(owner.downgrade(), cx)).is_none());
        }
        main.update(cx, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
    }
}
