//! Window main menu, using the workspace's existing action routes.

use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::commands::{self, Entry, Scope};
use gpui_kit::{Bounds, Point, canvas};
use volna_core::panels::PanelsCommand;

#[derive(Default)]
pub(super) struct MainMenu {
    trigger: Rc<Cell<Bounds<Pixels>>>,
    open: Option<OpenMenu>,
}

struct OpenMenu {
    hosted: HostedMenu,
    return_focus: FocusHandle,
}

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("f10", OpenMainMenu, Some("MainWindow"))]);
}

impl Workspace {
    pub(super) fn render_main_menu_button(
        &self,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let t = *theme(cx);
        let trigger = self.main_menu.trigger.clone();
        let open = self.main_menu.open.is_some();
        div()
            .id("main-menu-control")
            .relative()
            .flex_none()
            .child(
                icon_button("main-menu", IconName::Menu, t.bar, t.bar_hover, cx)
                    .w(t.px(32.0))
                    .h(t.px(32.0))
                    .selected(open)
                    .accessibility_label("Main menu")
                    .tooltip("Main menu (F10)")
                    .debug_selector(|| "main-menu".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_main_menu(&OpenMainMenu, window, cx);
                    })),
            )
            .child(
                canvas(move |bounds, _, _| trigger.set(bounds), |_, (), _, _| {})
                    .absolute()
                    .size_full(),
            )
    }

    pub(super) fn open_main_menu(
        &mut self,
        _: &OpenMainMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.embedded {
            return;
        }
        #[cfg(not(target_family = "wasm"))]
        if self.windows.id_for(window.window_handle()).is_some() {
            return;
        }
        let id = window.window_handle().window_id();
        if let Some(open) = self.main_menu.open.take()
            && open.hosted.window == id
        {
            window.focus(&open.return_focus, cx);
            cx.notify();
            return;
        }
        let return_focus = window
            .focused(cx)
            .unwrap_or_else(|| self.window_focus(window));
        let panel = self.app.panels.focused_id();
        let target = self
            .panel_focus(panel, cx)
            .unwrap_or_else(|| self.window_focus(window));
        let generation = self.app.doc.generation();
        let entries = commands::menu(&self.app);
        let dismiss_target = target.clone();
        let target = MenuTarget {
            owner: cx.weak_entity(),
            focus: target,
            generation,
            panel,
        };
        let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
            populate(menu, entries, &target, window, cx)
        });
        let popup_focus = menu.read(cx).focus_handle(cx);
        let handle = window.window_handle();
        cx.subscribe(&menu, move |this, menu, _: &gpui_kit::DismissEvent, cx| {
            if this
                .main_menu
                .open
                .as_ref()
                .is_none_or(|open| open.hosted.popup != menu)
            {
                return;
            }
            let open = this.main_menu.open.take().unwrap();
            let popup_focus = popup_focus.clone();
            let dismiss_target = dismiss_target.clone();
            // PopupMenu restores its action target; Escape must return to the interrupted control.
            cx.defer(move |cx| {
                drop(handle.update(cx, |_, window, cx| {
                    if popup_focus.contains_focused(window, cx)
                        || (window.last_input_was_keyboard() && dismiss_target.is_focused(window))
                    {
                        window.focus(&open.return_focus, cx);
                    }
                }));
            });
            cx.notify();
        })
        .detach();
        let bounds = self.main_menu.trigger.get();
        let position = Point::new(
            bounds.left(),
            px(self.workspace_header_height(cx)) + theme(cx).px(4.0),
        );
        self.status_menu = None;
        self.wave_menu = None;
        self.main_menu.open = Some(OpenMenu {
            hosted: HostedMenu {
                trace: None,
                window: id,
                position,
                popup: menu,
            },
            return_focus,
        });
        cx.notify();
    }

    pub(super) fn main_menu_overlay(
        &self,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui_kit::AnyElement> {
        let open = self.main_menu.open.as_ref()?;
        (open.hosted.window == window.window_handle().window_id()).then(|| {
            popup_at(open.hosted.position, open.hosted.popup.clone(), window, cx).into_any_element()
        })
    }

    pub(super) fn toggle_right_sidebar(
        &mut self,
        _: &ToggleRightSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            Command::Panels(PanelsCommand::ToggleRightDock),
            Some(window),
            cx,
        );
    }
}

#[derive(Clone)]
struct MenuTarget {
    owner: gpui_kit::WeakEntity<Workspace>,
    focus: FocusHandle,
    generation: u64,
    panel: volna_core::panels::PanelId,
}

fn populate(
    mut menu: PopupMenu,
    entries: Vec<Entry>,
    target: &MenuTarget,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for entry in entries {
        menu = match entry {
            Entry::Separator => menu.separator(),
            Entry::Submenu(label, entries) => {
                let target = target.clone();
                menu.submenu(label, window, cx, move |menu, window, cx| {
                    populate(menu, entries.clone(), &target, window, cx)
                })
            }
            Entry::Label(label) => menu.label(label),
            Entry::Command(command) => {
                let target = target.clone();
                menu.item(
                    PopupMenuItem::new(command.label)
                        .action(command.action.boxed_clone())
                        .disabled(!command.enabled)
                        .checked(command.checked)
                        .on_click(move |_, window, cx| {
                            let current = target
                                .owner
                                .update(cx, |ws, cx| {
                                    ws.main_menu.open = None;
                                    cx.notify();
                                    let current = match command.scope {
                                        Scope::Global => true,
                                        Scope::Recording => {
                                            ws.app.doc.generation() == target.generation
                                        }
                                        Scope::Panel => {
                                            ws.app.doc.generation() == target.generation
                                                && ws.app.panels.focused_id() == target.panel
                                        }
                                    };
                                    let focus = if matches!(command.scope, Scope::Panel) && current
                                    {
                                        target.focus.clone()
                                    } else {
                                        ws.window_focus(window)
                                    };
                                    (current, focus)
                                })
                                .ok();
                            if let Some((current, focus)) = current {
                                window.focus(&focus, cx);
                                if current {
                                    focus.dispatch_action(command.action.as_ref(), window, cx);
                                }
                            }
                        }),
                )
            }
        };
    }
    let available = (window.viewport_size().width - theme(cx).px(16.0)).max(px(0.0));
    menu.min_w(theme(cx).px(240.0).min(available))
        .max_w(theme(cx).px(500.0).min(available))
        .scrollable(true)
        .max_h((window.viewport_size().height - theme(cx).px(16.0)).max(px(0.0)))
        .action_context(target.focus.clone())
}
