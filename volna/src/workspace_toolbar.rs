//! Shared waveform controls with native title-bar and embedded editor placements.

use super::*;

#[derive(Clone, Copy)]
struct ToolbarLayout {
    wide: bool,
    navigation: bool,
}

impl ToolbarLayout {
    fn new(embedded: bool, width: f32) -> Self {
        Self {
            wide: !embedded || width >= 340.0,
            navigation: !embedded || width >= 240.0,
        }
    }

    fn fits(self, tool: WaveTool) -> bool {
        if matches!(tool.action, Action::ZoomFit) {
            return true;
        }
        match tool.group {
            ToolGroup::Zoom | ToolGroup::Pan => self.wide,
            ToolGroup::Transitions => self.navigation,
        }
    }
}

#[derive(Clone, Copy)]
struct WaveTool {
    id: &'static str,
    icon: IconName,
    label: &'static str,
    action: Action,
    group: ToolGroup,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolGroup {
    Zoom,
    Pan,
    Transitions,
}

impl WaveTool {
    fn available(self, enabled: bool, transitions: bool) -> bool {
        enabled && (!matches!(self.action, Action::PrevEdge | Action::NextEdge) || transitions)
    }
}

const WAVE_TOOLS: [WaveTool; 7] = [
    WaveTool {
        id: "wave-zoom-in",
        icon: IconName::ZoomIn,
        label: "Zoom in (=)",
        action: Action::ZoomIn,
        group: ToolGroup::Zoom,
    },
    WaveTool {
        id: "wave-zoom-out",
        icon: IconName::ZoomOut,
        label: "Zoom out (-)",
        action: Action::ZoomOut,
        group: ToolGroup::Zoom,
    },
    WaveTool {
        id: "wave-zoom-fit",
        icon: IconName::ZoomFit,
        label: "Zoom to fit (F)",
        action: Action::ZoomFit,
        group: ToolGroup::Zoom,
    },
    WaveTool {
        id: "wave-pan-left",
        icon: IconName::ArrowLeft,
        label: "Pan left",
        action: Action::PanPageLeft,
        group: ToolGroup::Pan,
    },
    WaveTool {
        id: "wave-pan-right",
        icon: IconName::ArrowRight,
        label: "Pan right",
        action: Action::PanPageRight,
        group: ToolGroup::Pan,
    },
    WaveTool {
        id: "wave-prev-transition",
        icon: IconName::StepBack,
        label: "Previous transition / record",
        action: Action::PrevEdge,
        group: ToolGroup::Transitions,
    },
    WaveTool {
        id: "wave-next-transition",
        icon: IconName::StepForward,
        label: "Next transition / record",
        action: Action::NextEdge,
        group: ToolGroup::Transitions,
    },
];

fn hidden_tools(layout: ToolbarLayout) -> impl Iterator<Item = WaveTool> {
    WAVE_TOOLS
        .into_iter()
        .filter(move |&tool| !layout.fits(tool))
}

impl Workspace {
    pub(super) fn workspace_header_height(&self, cx: &App) -> f32 {
        if self.embedded {
            theme(cx).scale(32.0)
        } else {
            crate::theme::titlebar_height(cx)
        }
    }

    fn run_toolbar_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Command::Action(action), Some(window), cx);
        if let Some(focus) = self.panel_focus(self.app.panels.focused_id(), cx) {
            window.focus(&focus, cx);
        }
    }

    pub(super) fn render_sidebar_toggles(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme(cx);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(t.px(4.0))
            .child(
                icon_button(
                    "toggle-sidebar",
                    IconName::PanelLeft,
                    t.bar,
                    t.bar_hover,
                    cx,
                )
                .selected(self.app.sidebar_visible)
                .debug_selector(|| "toggle-sidebar".into())
                .tooltip(if self.embedded {
                    "Toggle sidebar"
                } else {
                    "Toggle sidebar (⌘B)"
                })
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
    }

    pub(super) fn titlebar_divider(cx: &App) -> impl IntoElement {
        let t = theme(cx);
        div()
            .flex_none()
            .w(px(1.0))
            .h(t.px(18.0))
            .mx(t.px(4.0))
            .bg(crate::theme::chrome_border(cx))
    }

    pub(super) fn render_wave_toolbar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *theme(cx);
        let panel = self.app.panels.focused_id();
        let waves = self.app.panels.focused().kind.waves();
        let enabled = self.app.doc.is_loaded()
            && waves.is_some()
            && (self.embedded
                || self
                    .panel_focus(panel, cx)
                    .is_some_and(|focus| focus.contains_focused(window, cx)));
        let transitions = waves.is_some_and(|waves| !waves.selected.is_empty());
        let layout = ToolbarLayout::new(
            self.embedded,
            f32::from(window.viewport_size().width) / t.zoom,
        );
        let mut toolbar = div()
            .id("wave-toolbar")
            .flex()
            .flex_none()
            .items_center()
            .gap(t.px(4.0));
        let button_size = t.px(if self.embedded { 24.0 } else { 32.0 });
        let mut previous_group = None;
        for tool in WAVE_TOOLS.into_iter().filter(|&tool| layout.fits(tool)) {
            if previous_group.is_some_and(|previous| previous != tool.group) {
                toolbar = toolbar.child(Self::titlebar_divider(cx));
            }
            previous_group = Some(tool.group);
            toolbar = toolbar.child(
                icon_button(tool.id, tool.icon, t.bar, t.bar_hover, cx)
                    .w(button_size)
                    .h(button_size)
                    .debug_selector(move || tool.id.into())
                    .disabled(!tool.available(enabled, transitions))
                    .tooltip(tool.label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run_toolbar_action(tool.action, window, cx);
                    })),
            );
        }
        toolbar
    }

    pub(super) fn render_embedded_toolbar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *theme(cx);
        let layout = ToolbarLayout::new(true, f32::from(window.viewport_size().width) / t.zoom);
        let overflow = !layout.navigation || hidden_tools(layout).next().is_some();
        div()
            .id("viewer-toolbar")
            .debug_selector(|| "viewer-toolbar".into())
            .aria_label("Waveform controls")
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(self.workspace_header_height(cx)))
            .w_full()
            .px(t.px(4.0))
            .bg(t.bar.bg)
            .border_b_1()
            .border_color(t.border)
            .child(self.render_wave_toolbar(window, cx))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(t.px(4.0))
                    .when(layout.navigation, |el| {
                        el.child(self.render_sidebar_toggles(cx))
                    })
                    .when(overflow, |el| {
                        el.child(
                            icon_button(
                                "wave-toolbar-more",
                                IconName::ChevronDown,
                                t.bar,
                                t.bar_hover,
                                cx,
                            )
                            .debug_selector(|| "wave-toolbar-more".into())
                            .tooltip("More waveform actions")
                            .on_click(cx.listener(
                                move |this, event: &gpui_kit::ClickEvent, window, cx| {
                                    this.open_toolbar_menu(event.position(), layout, window, cx);
                                },
                            )),
                        )
                    }),
            )
    }

    fn open_toolbar_menu(
        &mut self,
        position: gpui_kit::Point<Pixels>,
        layout: ToolbarLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.app.doc.generation();
        let panel = self.app.panels.focused_id();
        let waves = self.app.panels.focused_waves();
        let enabled = self.app.doc.is_loaded() && waves.is_some();
        let transitions = enabled && waves.is_some_and(|waves| !waves.selected.is_empty());
        let focus = self
            .panel_focus(panel, cx)
            .unwrap_or_else(|| self.focus_handle.clone());
        let owner = cx.weak_entity();
        let run = move |command: Command| {
            let owner = owner.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                owner
                    .update(cx, |this, cx| {
                        this.status_menu = None;
                        if this.app.doc.generation() != generation
                            || this.app.panels.focused_id() != panel
                        {
                            return;
                        }
                        this.dispatch_if_current(generation, command.clone(), Some(window), cx);
                        if let Some(focus) = this.panel_focus(panel, cx) {
                            window.focus(&focus, cx);
                        }
                    })
                    .ok();
            }
        };
        let sidebar = self.app.sidebar_visible;
        let right = self
            .app
            .panels
            .layout()
            .right
            .as_ref()
            .map(|dock| dock.open);
        let menu = PopupMenu::build(window, cx, move |mut menu, _, _| {
            for tool in hidden_tools(layout) {
                menu = menu.item(
                    PopupMenuItem::new(tool.label)
                        .disabled(!tool.available(enabled, transitions))
                        .on_click(run(Command::Action(tool.action))),
                );
            }
            if !layout.navigation {
                menu = menu.separator().item(
                    PopupMenuItem::new("Show sidebar")
                        .checked(sidebar)
                        .on_click(run(Command::ToggleSidebar)),
                );
                if let Some(open) = right {
                    menu = menu.item(
                        PopupMenuItem::new("Show right sidebar")
                            .checked(open)
                            .on_click(run(Command::Panels(
                                volna_core::panels::PanelsCommand::ToggleRightDock,
                            ))),
                    );
                }
            }
            menu.action_context(focus)
        });
        self.show_status_menu(position, menu, window, cx);
    }
}
