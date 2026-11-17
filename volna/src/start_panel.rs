//! The start panel: what every trace opens into.

use gpui_kit::base::dock as base;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dock::Panel;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Render, SharedString,
    WeakEntity, Window, div, px,
};
use volna_core::Command;
use volna_core::app::StartSummary;
use volna_core::panels::{PanelId, PanelsCommand};

use crate::app::Workspace;
use crate::theme::{ThemePx, theme};
use crate::ui::{IconName, branding};

const MAX_LISTED: usize = 8;

pub(crate) struct StartPanelView {
    ws: WeakEntity<Workspace>,
    id: PanelId,
    generation: u64,
    focus: FocusHandle,
}

impl StartPanelView {
    pub(crate) fn new(
        ws: WeakEntity<Workspace>,
        id: PanelId,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        cx.on_focus_in(&focus, window, |view, window, cx| {
            view.dispatch(Command::Panels(PanelsCommand::Focus(view.id)), window, cx);
        })
        .detach();
        Self {
            ws,
            id,
            generation,
            focus,
        }
    }

    fn dispatch(&self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.generation;
        drop(self.ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(generation, command, Some(window), cx);
        }));
    }
}

impl EventEmitter<base::PanelEvent> for StartPanelView {}
impl Focusable for StartPanelView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl base::Panel for StartPanelView {
    fn panel_name(&self) -> &'static str {
        "volna.start"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn dump(&self, _: &App) -> base::PanelState {
        let mut state = base::PanelState::new("volna.start");
        state.info = base::PanelInfo::panel(serde_json::json!({"id": self.id.0}));
        state
    }
}
impl Panel for StartPanelView {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::dock::tab_title(&self.ws, self.id, Some(self.generation), cx)
    }
    fn dropdown_menu(
        &mut self,
        menu: gpui_kit::component::menu::PopupMenu,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::component::menu::PopupMenu {
        crate::dock::panel_menu(menu, &self.ws, self.id, self.generation, cx)
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Render for StartPanelView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme(cx);
        let colors = t.editor;
        let summary: Option<StartSummary> = self
            .ws
            .upgrade()
            .and_then(|ws| ws.read(cx).app.start_summary());
        let hint = |text: &'static str| {
            div()
                .text_size(px(t.ui_size_small))
                .text_color(colors.text_muted)
                .child(text)
        };
        let mut body = div()
            .id(("start-panel", self.id.0))
            .key_context("Start")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .overflow_y_scroll()
            .px_4()
            .py_4()
            .gap_2()
            .bg(colors.bg)
            .font_family(t.ui_font)
            .text_size(px(t.ui_size))
            .child(
                div()
                    .mt_auto()
                    .child(branding::mark(t.px(48.), colors.icon_accent)),
            );
        let Some(summary) = summary else {
            return body.child(div().text_color(colors.text_muted).child("No trace open"));
        };
        let facts = format!(
            "{} · {} · {}",
            summary.time_range,
            plural(summary.variables, "variable", "variables"),
            plural(summary.tracks, "transaction track", "transaction tracks")
        );
        body = body
            .child(
                div()
                    .mt_2()
                    .text_size(t.px(20.0))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child(SharedString::from(summary.name.clone())),
            )
            .child(
                div()
                    .text_color(colors.text_muted)
                    .child(SharedString::from(facts)),
            );
        let mut hints = div()
            .mt_2()
            .max_w_full()
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .gap_1();
        if summary.variables > 0 {
            hints = hints.child(hint(
                "Double-click a variable to view its waveform. Press Enter to add a selection.",
            ));
        }
        if summary.tracks > 0 {
            hints = hints.child(hint(
                "Right-click a generator and choose Open in Pipeline to inspect its transactions.",
            ));
        }
        if summary.variables == 0 && summary.tracks == 0 {
            hints = hints.child(hint(
                "This trace declares no variables or transaction tracks",
            ));
        }
        body = body.child(hints);
        if !summary.pipelines.is_empty() {
            let mut list = div().mt_3().flex().flex_col().items_center().gap_1();
            for (ix, (path, track)) in summary.pipelines.iter().take(MAX_LISTED).enumerate() {
                let track = *track;
                list = list.child(
                    Button::new(("start-pipeline", ix))
                        .label(SharedString::from(path.clone()))
                        .icon(gpui_kit::component::Icon::empty().path(IconName::Workflow.path()))
                        .ghost()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.dispatch(Command::OpenPipeline { track }, window, cx);
                        })),
                );
            }
            if summary.pipelines.len() > MAX_LISTED {
                list = list.child(hint(
                    "More generators are listed in the sidebar and View ▸ Pipeline",
                ));
            }
            body = body.child(list);
        }
        body = body.child(
            div().mt_3().mb_auto().flex().gap_2().child(
                Button::new("start-new-waves")
                    .label("New waveform panel")
                    .icon(gpui_kit::component::Icon::empty().path(IconName::AudioWaveform.path()))
                    .primary()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.dispatch(
                            Command::Panels(PanelsCommand::NewTab { group_of: view.id }),
                            window,
                            cx,
                        );
                    })),
            ),
        );
        body
    }
}
