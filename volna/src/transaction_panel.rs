//! The Transaction panel: everything recorded about one selected record.

mod attributes;
mod lifeline;

use std::collections::{HashMap, HashSet};

use gpui_kit::base::dock as base;
use gpui_kit::component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    dock::Panel,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    scroll::ScrollableElement,
    tooltip::Tooltip,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement,
    Render, SharedString, Subscription, WeakEntity, Window, actions, div, px, relative,
    uniform_list,
};
use volna_core::Command;
use volna_core::data::points::PointKind;
use volna_core::panels::{PanelId, PanelsCommand};
use volna_core::trace::Traced;
use volna_core::transaction::attributes::AttributeId;
use volna_core::transaction::{
    EventRow, LifelineScale, ParentTraceState, RefRole, RefRow, Section, StageRow,
    TransactionCommand, TxPanelState, TxView,
};
use volna_trace::data::transactions::{TrackRef, TransactionRef};

use crate::app::Workspace;
use crate::theme::{ThemePx, hsla, theme};
use crate::ui::{Icon, IconName, action_button};

const LANE_PX: f32 = 20.0;
const PARENT_ROW_PX: f32 = 88.0;

actions!(transaction, [Back, Forward]);

#[cfg(target_os = "macos")]
const BACK_TOOLTIP: &str = "Back (⌘[)";
#[cfg(target_os = "macos")]
const FORWARD_TOOLTIP: &str = "Forward (⌘])";
#[cfg(all(not(target_os = "macos"), not(target_family = "wasm")))]
const BACK_TOOLTIP: &str = "Back (Alt+←)";
#[cfg(all(not(target_os = "macos"), not(target_family = "wasm")))]
const FORWARD_TOOLTIP: &str = "Forward (Alt+→)";
#[cfg(target_family = "wasm")]
const BACK_TOOLTIP: &str = "Back (Alt+← / ⌘[)";
#[cfg(target_family = "wasm")]
const FORWARD_TOOLTIP: &str = "Forward (Alt+→ / ⌘])";

pub(crate) struct TransactionPanelView {
    ws: WeakEntity<Workspace>,
    id: PanelId,
    generation: u64,
    pub(crate) focus: FocusHandle,
    filter: Entity<InputState>,
    synced_filter: String,
    group_filter: Entity<InputState>,
    synced_group_filter: String,
    hovered_stage: Option<usize>,
    shown_record: Option<(Traced<TrackRef>, TransactionRef)>,
    expanded_stage_groups: HashSet<(String, String)>,
    stage_focus: HashMap<TransactionRef, FocusHandle>,
    attribute_focus: HashMap<AttributeId, FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

fn external_origin_panel(
    app: &volna_core::App,
    self_id: PanelId,
) -> Option<&volna_core::panels::Panel> {
    let live_external = |id: PanelId| {
        (id != self_id)
            .then(|| app.panels.get(id))
            .flatten()
            .filter(|panel| panel.kind.transaction().is_none())
    };
    app.panels
        .transaction(self_id)
        .and_then(volna_core::transaction::TransactionModel::external_origin)
        .and_then(live_external)
        .or_else(|| app.doc.selection().and_then(|s| live_external(s.origin)))
}

impl TransactionPanelView {
    pub(crate) fn new(
        ws: WeakEntity<Workspace>,
        id: PanelId,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        cx.on_focus_in(&focus, window, |view, window, cx| {
            view.panels(PanelsCommand::Focus(view.id), window, cx);
        })
        .detach();
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Find a field or value…"));
        let subscription = cx.subscribe_in(&filter, window, |view, input, event, window, cx| {
            if let InputEvent::Change = event {
                let text = input.read(cx).value().to_string();
                view.synced_filter.clone_from(&text);
                view.dispatch(TransactionCommand::Filter(text), window, cx);
            }
        });
        let group_filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Find records, IDs, timestamps, arguments")
        });
        let group_subscription =
            cx.subscribe_in(&group_filter, window, |view, input, event, window, cx| {
                if let InputEvent::Change = event {
                    let text = input.read(cx).value().to_string();
                    view.synced_group_filter.clone_from(&text);
                    view.dispatch(TransactionCommand::GroupSearch(text), window, cx);
                }
            });
        Self {
            ws,
            id,
            generation,
            focus,
            filter,
            synced_filter: String::new(),
            group_filter,
            synced_group_filter: String::new(),
            hovered_stage: None,
            shown_record: None,
            expanded_stage_groups: HashSet::new(),
            stage_focus: HashMap::new(),
            attribute_focus: HashMap::new(),
            _subscriptions: vec![subscription, group_subscription],
        }
    }

    fn dispatch(&self, command: TransactionCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.send(Command::Transaction(self.id, command), window, cx);
    }

    fn panels(&self, command: PanelsCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.send(Command::Panels(command), window, cx);
    }

    fn send(&self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        drop(self.ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(self.generation, command, Some(window), cx);
        }));
    }

    fn copy(&self, cx: &mut Context<Self>) {
        let Some(owner) = self.ws.upgrade() else {
            return;
        };
        let result = {
            let ws = owner.read(cx);
            ws.app
                .panels
                .transaction(self.id)
                .map(|model| model.copy_tsv(&ws.app.doc))
        };
        Workspace::copy_result(&owner, self.generation, result, cx);
    }

    fn sync_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.ws.upgrade() else { return };
        let Some(text) = ws
            .read(cx)
            .app
            .panels
            .transaction(self.id)
            .map(|model| model.prefs().filter.clone())
        else {
            return;
        };
        if text != self.synced_filter {
            self.synced_filter.clone_from(&text);
            self.filter
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }
}

impl EventEmitter<base::PanelEvent> for TransactionPanelView {}
impl Focusable for TransactionPanelView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl base::Panel for TransactionPanelView {
    fn panel_name(&self) -> &'static str {
        "volna.transaction"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn dump(&self, _: &App) -> base::PanelState {
        let mut state = base::PanelState::new("volna.transaction");
        state.info = base::PanelInfo::panel(serde_json::json!({"id": self.id.0}));
        state
    }
}
impl Panel for TransactionPanelView {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::dock::tab_title(&self.ws, self.id, Some(self.generation), cx)
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn dropdown_menu(
        &mut self,
        menu: PopupMenu,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> PopupMenu {
        crate::dock::panel_menu(menu, &self.ws, self.id, self.generation, cx)
    }

    fn toolbar_buttons(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<Vec<Button>> {
        None
    }
}

impl Render for TransactionPanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_filter(window, cx);
        let group_search = self
            .ws
            .upgrade()
            .and_then(|owner| {
                owner
                    .read(cx)
                    .app
                    .panels
                    .transaction(self.id)
                    .map(|m| m.group_query().search.clone())
            })
            .unwrap_or_default();
        if self.synced_group_filter != group_search {
            self.synced_group_filter.clone_from(&group_search);
            self.group_filter
                .update(cx, |input, cx| input.set_value(group_search, window, cx));
        }
        let t = *theme(cx);
        let Some(owner) = self.ws.upgrade() else {
            return div().into_any_element();
        };
        let (state, follows, pinned, back, forward, prefs) = {
            let ws = owner.read(cx);
            let Some(model) = ws.app.panels.transaction(self.id) else {
                return div().into_any_element();
            };
            let pinned = model.pinned();
            let follows = if pinned {
                "Pinned".to_owned()
            } else {
                match external_origin_panel(&ws.app, self.id) {
                    Some(panel) => format!("follows {}", panel.title()),
                    None => "Follows selection".to_owned(),
                }
            };
            (
                model.state(&ws.app.doc),
                follows,
                pinned,
                model.can_go_back(),
                model.can_go_forward(),
                model.prefs().clone(),
            )
        };
        let body = match state {
            TxPanelState::Ready(view) => self.render_view(*view, &prefs, window, cx),
            TxPanelState::Empty => notice(
                &t,
                "No record selected",
                "Select a record in waveforms, a pipeline, or a table.",
            ),
            TxPanelState::Loading => notice(&t, "Loading the record's track…", ""),
            TxPanelState::Missing => notice(
                &t,
                "Record not in this trace",
                "The saved track or record is not part of the open recording.",
            ),
            TxPanelState::Failed(error) => notice(&t, "Cannot load the record", &error),
            TxPanelState::Refused(error) => notice(&t, "Not admitted", &error),
        };
        let header = div()
            .debug_selector(|| "tx-header".into())
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(t.px(34.0))
            .px_2()
            .border_b_1()
            .border_color(t.border_variant)
            .child(
                action_button("tx-back", gpui_kit::assets::IconName::ChevronLeft, cx)
                    .debug_selector(|| "tx-back".into())
                    .accessibility_label("Back")
                    .ghost()
                    .tooltip(BACK_TOOLTIP)
                    .disabled(!back)
                    .on_click(cx.listener(|view, _, window, cx| {
                        window.focus(&view.focus, cx);
                        view.dispatch(TransactionCommand::Back, window, cx);
                    })),
            )
            .child(
                action_button("tx-forward", gpui_kit::assets::IconName::ChevronRight, cx)
                    .debug_selector(|| "tx-forward".into())
                    .accessibility_label("Forward")
                    .ghost()
                    .tooltip(FORWARD_TOOLTIP)
                    .disabled(!forward)
                    .on_click(cx.listener(|view, _, window, cx| {
                        window.focus(&view.focus, cx);
                        view.dispatch(TransactionCommand::Forward, window, cx);
                    })),
            )
            .child(
                div()
                    .debug_selector(|| "tx-follows".into())
                    .flex_1()
                    .min_w_0()
                    .px_1()
                    .truncate()
                    .text_size(px(t.ui_size_small))
                    .text_color(t.panel.text_muted)
                    .child(SharedString::from(follows)),
            )
            .child(
                action_button("tx-pin", gpui_kit::assets::IconName::Pin, cx)
                    .debug_selector(|| "tx-pin".into())
                    .ghost()
                    .selected(pinned)
                    .tooltip(if pinned {
                        "Pinned: this panel keeps its record"
                    } else {
                        "Pin this record; the next one opens another panel"
                    })
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.dispatch(TransactionCommand::Pin(!pinned), window, cx);
                    })),
            );
        div()
            .id(("transaction-panel", self.id.0))
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(t.panel.bg)
            .font_family(t.ui_font)
            .text_size(px(t.ui_size))
            .text_color(t.editor.text)
            .track_focus(&self.focus)
            .key_context("Transaction")
            .on_action(cx.listener(|view, _: &Back, window, cx| {
                view.dispatch(TransactionCommand::Back, window, cx);
            }))
            .on_action(cx.listener(|view, _: &Forward, window, cx| {
                view.dispatch(TransactionCommand::Forward, window, cx);
            }))
            .on_key_down(
                cx.listener(|view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    let key = event.keystroke.key.as_str();
                    let modifiers = crate::app::to_modifiers(event.keystroke.modifiers);
                    if key.eq_ignore_ascii_case("c") && modifiers.secondary() {
                        view.copy(cx);
                        cx.stop_propagation();
                    } else if key == "escape" {
                        if let Some(origin) = view.ws.upgrade().and_then(|ws| {
                            external_origin_panel(&ws.read(cx).app, view.id).map(|panel| panel.id)
                        }) {
                            view.panels(PanelsCommand::Focus(origin), window, cx);
                        }
                        cx.stop_propagation();
                    }
                }),
            )
            .child(header)
            .child(body)
            .into_any_element()
    }
}

impl TransactionPanelView {
    fn render_view(
        &mut self,
        view: TxView,
        prefs: &volna_core::transaction::ViewPrefs,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_record = Some((view.identity.track, view.identity.id));
        if self.shown_record != current_record {
            self.shown_record = current_record;
            self.expanded_stage_groups.clear();
            self.hovered_stage = None;
        }
        let visible: HashSet<_> = view
            .attributes
            .rows
            .iter()
            .chain(
                view.stage_details
                    .iter()
                    .flat_map(|stage| stage.attributes.rows.iter()),
            )
            .chain(
                view.events
                    .rows
                    .iter()
                    .flat_map(|event| event.attributes.rows.iter()),
            )
            .map(|row| &row.id)
            .collect();
        self.attribute_focus.retain(|id, _| visible.contains(id));
        let t = *theme(cx);
        let muted = t.panel.text_muted;
        let identity = &view.identity;
        let path = format!(
            "{}{}{} · row {}",
            identity.scope.join("."),
            if identity.scope.is_empty() {
                ""
            } else {
                " › "
            },
            identity.generator,
            identity.ordinal,
        );
        let breadcrumb = div()
            .debug_selector(|| "tx-record-path".into())
            .flex()
            .min_w_0()
            .items_center()
            .gap_1()
            .text_size(px(t.ui_size_small))
            .text_color(muted)
            .child(Icon::new(IconName::Workflow).size(t.px(12.0)).color(muted))
            .child(detail_text(cx, "tx-path", path).flex_1());

        let mut title = div().flex().min_w_0().items_center().gap_2().child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .px_1p5()
                .py_0p5()
                .rounded_sm()
                .bg(t.badge.bg)
                .font_family(t.mono_font)
                .text_size(px(t.ui_size_small))
                .text_color(t.badge.text)
                .child(SharedString::from(format!("#{}", identity.id.0))),
        );
        title = title.child(
            detail_text(
                cx,
                "tx-label",
                identity
                    .label
                    .clone()
                    .unwrap_or_else(|| "(no label)".into()),
            )
            .flex_1()
            .font_family(t.mono_font)
            .font_weight(gpui_kit::FontWeight::SEMIBOLD),
        );
        let mut point_metadata = div().flex().flex_wrap().items_center().gap_2();
        if let Some(kind) = view.point_kind {
            point_metadata = point_metadata.child(
                div()
                    .text_size(px(t.ui_size_small))
                    .text_color(muted)
                    .child(match kind {
                        PointKind::Log => "Log message",
                        PointKind::Event => "Event",
                    }),
            );
            if let Some(severity) = &view.severity {
                point_metadata = point_metadata.child(
                    div()
                        .text_size(px(t.ui_size_small))
                        .text_color(muted)
                        .child(SharedString::from(severity.clone())),
                );
            }
        } else {
            title = title.child(status_badge(cx, &t, identity.status_text, identity.status));
        }
        if let Some(kind) = identity.kind.filter(|_| view.point_kind.is_none()) {
            point_metadata = point_metadata.child(
                div()
                    .whitespace_nowrap()
                    .text_size(px(t.ui_size_small))
                    .text_color(muted)
                    .child(kind),
            );
        }
        if let Some(parent) = identity.parent {
            let parent = div()
                .text_size(px(t.ui_size_small))
                .text_color(muted)
                .child(SharedString::from(format!("Owner #{}", parent.0)));
            point_metadata = point_metadata.child(parent.whitespace_nowrap());
        }

        let timing = &view.timing;
        let timing_module = if view.point_kind.is_some() {
            div().flex().min_w_0().gap_2().child(Self::tile(
                &t,
                "Timestamp",
                timing.begin_text.clone(),
                "Move cursor to timestamp",
                Some(TransactionCommand::Cursor(timing.begin)),
                cx,
            ))
        } else {
            let mut module = div()
                .debug_selector(|| "tx-timing-module".into())
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded(t.px(6.0))
                .border_1()
                .border_color(t.border_variant)
                .bg(t.islands.map_or(t.panel.bg, |i| i.heading));
            let range_row = div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .min_w_0()
                        .child(Self::inline_time(
                            &t,
                            "Begin",
                            timing.begin_text.clone(),
                            "Move cursor to begin",
                            Some(TransactionCommand::Cursor(timing.begin)),
                            cx,
                        ))
                        .child(
                            div()
                                .text_size(px(t.ui_size_small))
                                .text_color(muted)
                                .child("→"),
                        )
                        .child(Self::inline_time(
                            &t,
                            "End",
                            timing.end_text.clone(),
                            "Move cursor to end",
                            timing.end.map(TransactionCommand::Cursor),
                            cx,
                        )),
                )
                .child(Self::inline_time(
                    &t,
                    "Duration",
                    timing.duration_text.clone(),
                    "Fit lifetime in viewport",
                    Some(TransactionCommand::FitLifetime),
                    cx,
                ));
            module =
                module
                    .child(range_row)
                    .child(self.lifeline(&t, &view, prefs.lifeline_scale, cx));
            module
        };

        let mut actions = div()
            .debug_selector(|| "tx-actions".into())
            .flex()
            .items_center()
            .justify_between()
            .gap_1()
            .flex_wrap();
        actions = actions
            .when(view.point_kind.is_none(), |actions| {
                actions.child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(t.ui_size_small))
                                .text_color(muted)
                                .pr_0p5()
                                .child("Show in:"),
                        )
                        .child(
                            Button::new("tx-reveal-selected")
                                .debug_selector(|| "tx-reveal-selected".into())
                                .label("Waves")
                                .ghost()
                                .small()
                                .tooltip("Focus or open this record in Waveforms")
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.send(
                                        Command::RevealWaveTransaction { panel: view.id },
                                        window,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("tx-reveal")
                                .debug_selector(|| "tx-reveal".into())
                                .label("Pipeline")
                                .ghost()
                                .small()
                                .tooltip("Focus or open this record in Pipeline")
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.send(
                                        Command::RevealTransaction { panel: view.id },
                                        window,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("tx-reveal-table")
                                .debug_selector(|| "tx-reveal-table".into())
                                .label("Table")
                                .ghost()
                                .small()
                                .tooltip("Focus or open this record in Table")
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.send(
                                        Command::RevealTableTransaction { panel: view.id },
                                        window,
                                        cx,
                                    );
                                })),
                        ),
                )
            })
            .when(view.point_kind.is_some(), |actions| {
                actions.child(
                    Button::new("tx-fit")
                        .label("Zoom here")
                        .ghost()
                        .small()
                        .tooltip("Fit the shared viewport to this record")
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.dispatch(TransactionCommand::FitLifetime, window, cx);
                        })),
                )
            })
            .child(
                Button::new("tx-copy")
                    .debug_selector(|| "tx-copy".into())
                    .label("Copy TSV")
                    .ghost()
                    .small()
                    .tooltip("Copy the complete record as TSV")
                    .on_click(cx.listener(|view, _, _, cx| view.copy(cx))),
            );

        let mut body = div()
            .debug_selector(|| "tx-body".into())
            .min_w_0()
            .overflow_x_hidden()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .child(breadcrumb)
                    .child(title)
                    .when(
                        view.point_kind.is_some()
                            || identity.kind.is_some()
                            || identity.parent.is_some(),
                        |body| body.child(point_metadata),
                    )
                    .child(timing_module)
                    .child(actions),
            );

        body = body.child(self.attributes(&t, &view, prefs.attributes_grouped, cx));
        if !view.source.is_empty() {
            let mut source = div()
                .flex()
                .flex_col()
                .gap_1()
                .p_3()
                .border_t_1()
                .border_color(t.border_variant)
                .child(
                    div()
                        .text_size(px(t.ui_size_small))
                        .text_color(muted)
                        .child("Runtime source"),
                );
            for (name, value) in &view.source {
                source = source.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .w(t.px(70.0))
                                .text_color(muted)
                                .child(SharedString::from(name.clone())),
                        )
                        .child(
                            detail_text(cx, format!("tx-source-{name}"), value.clone())
                                .flex_1()
                                .font_family(t.mono_font),
                        ),
                );
            }
            body = body.child(source);
        }
        if view.point_kind.is_none() {
            body = body.child(self.stages(&t, &view.stages, prefs, cx));
            if let Some(stage) = &view.stage_details {
                body = body.child(self.stage_attributes(&t, stage, cx));
            }
            if let Some(events) = self.events(&t, &view.events, cx) {
                body = body.child(events);
            }
        }
        body = body.child(Self::related(&t, &view.related, cx));
        body = body.child(self.parents(&t, cx));
        if view.truncated_bytes {
            body = body.child(
                div()
                    .px_3()
                    .pb_3()
                    .text_size(px(t.ui_size_small))
                    .text_color(t.panel.text_muted)
                    .child(
                        "Some values are shown as previews. Use Copy value for the complete value.",
                    ),
            );
        }
        if let Some(group) = self.record_group(view.point_kind, cx) {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(group)
                .child(body)
                .into_any_element();
        }
        body.into_any_element()
    }

    fn record_group(&self, kind: Option<PointKind>, cx: &mut Context<Self>) -> Option<AnyElement> {
        use gpui_kit::component::input::Input;
        use volna_core::transaction::group::PAGE_SIZE;
        let owner = self.ws.upgrade()?;
        let (group, query, selected, time_base) = {
            let ws = owner.read(cx);
            let model = ws.app.panels.transaction(self.id)?;
            let group = model.group(&ws.app.doc)?.clone();
            (
                group,
                model.group_query().clone(),
                model.shown()?.id,
                ws.app.doc.time_base(),
            )
        };
        let t = *theme(cx);
        let time = |tick| volna_core::wave::timeline::format_time(tick as f64, time_base);
        let bounds = if group.range.begin == group.range.end {
            format!("at {}", time(group.range.begin))
        } else {
            format!("{}–{}", time(group.range.begin), time(group.range.end))
        };
        let mut header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(SharedString::from(format!(
                "{} records {bounds}",
                group.total
            )));
        if group.range.begin != group.range.end {
            header = header.child(
                Button::new("record-group-fit")
                    .label("Zoom to group")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.dispatch(TransactionCommand::FitGroup, window, cx);
                    })),
            );
        }
        let mut filters = div().flex().flex_wrap().gap_1().child(
            Button::new("record-group-all")
                .label("All")
                .small()
                .ghost()
                .selected(query.severity.is_none())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.dispatch(TransactionCommand::GroupSeverity(None), window, cx);
                })),
        );
        if kind == Some(PointKind::Log) {
            for (level, label) in [
                "Trace", "Debug", "Info", "Warning", "Error", "Fatal", "Unknown",
            ]
            .into_iter()
            .enumerate()
            {
                if group.counts[level] == 0 {
                    continue;
                }
                filters = filters.child(
                    Button::new(("record-group-level", level))
                        .label(format!("{label} {}", group.counts[level]))
                        .small()
                        .ghost()
                        .selected(query.severity == Some(level))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.dispatch(
                                TransactionCommand::GroupSeverity(Some(level)),
                                window,
                                cx,
                            );
                        })),
                );
            }
        }
        let mut records = div()
            .flex()
            .flex_col()
            .max_h(t.px(180.0))
            .overflow_y_scrollbar();
        for record in &group.records {
            let id = record.id;
            let severity = if kind == Some(PointKind::Log) {
                format!(
                    "{} · ",
                    [
                        "TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL", "UNKNOWN"
                    ][record.severity]
                )
            } else {
                String::new()
            };
            records = records.child(
                Button::new(("record-group-row", id.0))
                    .label(format!(
                        "{} · #{} · {severity}{}",
                        time(record.time),
                        id.0,
                        record.label
                    ))
                    .small()
                    .ghost()
                    .selected(selected == id)
                    .w_full()
                    .justify_start()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dispatch(TransactionCommand::GroupSelect(id), window, cx);
                    })),
            );
        }
        if group.matching == 0 {
            records = records.child("No matching records");
        }
        if !group.records.iter().any(|record| record.id == selected) {
            records = records.child(div().text_color(t.panel.text_muted).child(
                SharedString::from(format!("Details show #{} outside this page.", selected.0)),
            ));
        }
        let page = group.page;
        let footer = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(SharedString::from(format!(
                "{}–{} of {} matching / {} total",
                if group.matching == 0 {
                    0
                } else {
                    page * PAGE_SIZE + 1
                },
                ((page + 1) * PAGE_SIZE).min(group.matching),
                group.matching,
                group.total
            )))
            .child(
                Button::new("record-group-prev")
                    .label("Previous")
                    .small()
                    .ghost()
                    .disabled(page == 0)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dispatch(
                            TransactionCommand::GroupPage(page.saturating_sub(1)),
                            window,
                            cx,
                        );
                    })),
            )
            .child(
                Button::new("record-group-next")
                    .label("Next")
                    .small()
                    .ghost()
                    .disabled((page + 1) * PAGE_SIZE >= group.matching)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dispatch(TransactionCommand::GroupPage(page + 1), window, cx);
                    })),
            );
        Some(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap_2()
                .p_3()
                .border_b_1()
                .border_color(t.border)
                .text_size(px(t.ui_size_small))
                .child(header)
                .child(
                    div()
                        .debug_selector(|| "record-group-search".into())
                        .child(Input::new(&self.group_filter).small()),
                )
                .child(filters)
                .child(records)
                .child(footer)
                .into_any_element(),
        )
    }

    fn parents(&self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(owner) = self.ws.upgrade() else {
            return div().into_any_element();
        };
        let (state, count) = {
            let ws = owner.read(cx);
            let Some(model) = ws.app.panels.transaction(self.id) else {
                return div().into_any_element();
            };
            (
                model.parents().state().clone(),
                model.parents().nodes().len(),
            )
        };
        let mut actions = div().flex().items_center().gap_1().flex_wrap().child(
            action_button(
                "tx-trace-parents",
                gpui_kit::component::Icon::empty().path(IconName::Workflow.path()),
                cx,
            )
            .debug_selector(|| "tx-trace-parents".into())
            .label(if matches!(state, ParentTraceState::Failed(_)) {
                "Retry tracing parents"
            } else {
                "Trace parents"
            })
            .disabled(matches!(
                state,
                ParentTraceState::Loading | ParentTraceState::Ready
            ))
            .tooltip("Load the full parent chain and show its graph")
            .on_click(cx.listener(|view, _, window, cx| {
                view.dispatch(TransactionCommand::TraceParents, window, cx);
            })),
        );
        if state != ParentTraceState::Idle {
            actions = actions.child(
                Button::new("tx-clear-parents")
                    .label(if state == ParentTraceState::Loading {
                        "Cancel"
                    } else {
                        "Hide graph"
                    })
                    .ghost()
                    .small()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.dispatch(TransactionCommand::ClearParents, window, cx);
                    })),
            );
        }
        let mut element = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(t.border_variant)
            .child(actions);
        match state {
            ParentTraceState::Idle => {}
            ParentTraceState::Loading => {
                element = element.child(
                    div()
                        .text_size(px(t.ui_size_small))
                        .text_color(t.panel.text_muted)
                        .child(SharedString::from(format!(
                            "Tracing parents… {count} transactions found"
                        ))),
                );
            }
            ParentTraceState::Failed(error) => {
                element = element.child(
                    div()
                        .text_size(px(t.ui_size_small))
                        .text_color(t.editor.error)
                        .child(SharedString::from(error)),
                );
            }
            ParentTraceState::Ready => {
                element = element
                    .child(
                        div()
                            .text_size(px(t.ui_size_small))
                            .text_color(t.panel.text_muted)
                            .child(SharedString::from(if count == 1 {
                                "This transaction has no parents.".into()
                            } else {
                                format!(
                                    "{} parents · oldest first · arrows point to children",
                                    count - 1
                                )
                            })),
                    )
                    .child(
                        uniform_list(
                            "tx-parent-graph",
                            count,
                            cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                                view.parent_rows(range, cx)
                            }),
                        )
                        .debug_selector(|| "tx-parent-graph".into())
                        .h(t.px(PARENT_ROW_PX * count.min(4) as f32))
                        .w_full()
                        .min_w_0()
                        .flex_none(),
                    );
            }
        }
        element.into_any_element()
    }

    fn parent_rows(
        &self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = *theme(cx);
        let Some(owner) = self.ws.upgrade() else {
            return Vec::new();
        };
        let ws = owner.read(cx);
        let Some(model) = ws.app.panels.transaction(self.id) else {
            return Vec::new();
        };
        let nodes = model.parents().nodes();
        range
            .filter_map(|index| {
                let node = nodes.get(nodes.len().checked_sub(index + 1)?)?;
                let (track, id) = (node.track, node.id);
                let selected = index + 1 == nodes.len();
                let mut row = div()
                    .id(("tx-parent-row", index))
                    .h(t.px(PARENT_ROW_PX))
                    .w_full()
                    .flex()
                    .flex_col();
                let mut edge = div()
                    .h(t.px(24.0))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center();
                if index > 0 {
                    edge = edge
                        .child(div().w(px(1.0)).flex_1().bg(t.panel.icon_muted))
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .size(t.px(12.0))
                                .color(t.panel.icon_muted),
                        );
                }
                row = row.child(edge).child(
                    Button::new(("tx-parent-node", index))
                        .debug_selector(move || format!("tx-parent-node-{index}"))
                        .accessibility_label(format!(
                            "Transaction #{}: {}{}",
                            id.0,
                            node.label,
                            if selected {
                                ", selected transaction"
                            } else {
                                ""
                            }
                        ))
                        .outline()
                        .selected(selected)
                        .w_full()
                        .h(t.px(64.0))
                        .min_w_0()
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .font_family(t.mono_font)
                                                .flex_none()
                                                .child(SharedString::from(format!("#{}", id.0))),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_ellipsis()
                                                .child(SharedString::from(node.label.clone())),
                                        )
                                        .when(selected, |element| {
                                            element.child(tag(&t, "current"))
                                        }),
                                )
                                .child(
                                    div()
                                        .text_size(px(t.ui_size_small))
                                        .text_color(t.panel.text_muted)
                                        .text_ellipsis()
                                        .child(SharedString::from(node.path.clone())),
                                ),
                        )
                        .tooltip(format!("{} · #{} · {}", node.path, id.0, node.label))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            window.focus(&view.focus, cx);
                            view.dispatch(TransactionCommand::Jump { track, id }, window, cx);
                        })),
                );
                Some(row.into_any_element())
            })
            .collect()
    }

    fn heading<T>(
        t: &crate::theme::Theme,
        section: &Section<T>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Self::named_heading(t, section, section.key.title(), cx)
    }

    fn named_heading<T>(
        t: &crate::theme::Theme,
        section: &Section<T>,
        title: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = section.key;
        let collapsed = section.collapsed;
        div()
            .id(SharedString::from(format!("tx-section-{}", key.key())))
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .min_h(t.px(30.))
            .tab_stop(true)
            .role(gpui_kit::accesskit::Role::Button)
            .aria_label(format!("{title}, {} items", section.total))
            .aria_expanded(!collapsed)
            .cursor_pointer()
            .hover(|style| style.bg(t.hover.bg))
            .focus_visible(|style| style.bg(t.selection.bg))
            .text_size(px(t.ui_size))
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .text_color(t.panel.text)
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(t.px(12.0))
                .color(t.panel.icon_muted),
            )
            .child(SharedString::from(title.to_owned()))
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .text_size(px(t.ui_size_small))
                    .child(section.total.to_string()),
            )
            .on_key_down(
                cx.listener(move |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        view.dispatch(TransactionCommand::Collapse(key, !collapsed), window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .on_click(cx.listener(move |view, _, window, cx| {
                view.dispatch(TransactionCommand::Collapse(key, !collapsed), window, cx);
            }))
            .into_any_element()
    }

    fn cut_note<T>(t: &crate::theme::Theme, section: &Section<T>) -> Option<AnyElement> {
        section.cut().then(|| {
            div()
                .px_3()
                .py_1()
                .text_size(px(t.ui_size_small))
                .text_color(t.panel.text_muted)
                .child(SharedString::from(format!(
                    "Showing the first {} of {} · transaction.detailItems",
                    section.rows.len(),
                    section.matched
                )))
                .into_any_element()
        })
    }

    fn stages(
        &mut self,
        t: &crate::theme::Theme,
        section: &Section<StageRow>,
        prefs: &volna_core::transaction::ViewPrefs,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let visible_ids: HashSet<_> = section.rows.iter().map(|row| row.id).collect();
        self.stage_focus.retain(|id, _| visible_ids.contains(id));
        for row in &section.rows {
            self.stage_focus
                .entry(row.id)
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
        let mut header_row = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_1()
            .pr_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Self::heading(t, section, cx)),
            );
        if !section.collapsed && section.total > 1 {
            header_row = header_row.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_0p5()
                    .child(
                        Button::new("tx-stages-all")
                            .debug_selector(|| "tx-stages-all".into())
                            .label("All")
                            .ghost()
                            .small()
                            .selected(!prefs.stages_grouped)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.dispatch(TransactionCommand::StagesGrouped(false), window, cx);
                            })),
                    )
                    .child(
                        Button::new("tx-stages-grouped")
                            .debug_selector(|| "tx-stages-grouped".into())
                            .label("Grouped")
                            .ghost()
                            .small()
                            .selected(prefs.stages_grouped)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.dispatch(TransactionCommand::StagesGrouped(true), window, cx);
                            })),
                    ),
            );
        }
        let mut element = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.border_variant)
            .child(header_row);
        if section.collapsed {
            return element.into_any_element();
        }
        if prefs.stages_grouped {
            struct StageGroup<'a> {
                lane: String,
                name: String,
                instances: Vec<(usize, &'a StageRow)>,
            }
            let mut groups: Vec<StageGroup<'_>> = Vec::new();
            for (index, row) in section.rows.iter().enumerate() {
                if let Some(group) = groups
                    .iter_mut()
                    .find(|g| g.lane == row.lane && g.name == row.name)
                {
                    group.instances.push((index, row));
                } else {
                    groups.push(StageGroup {
                        lane: row.lane.clone(),
                        name: row.name.clone(),
                        instances: vec![(index, row)],
                    });
                }
            }
            for (group_ix, group) in groups.into_iter().enumerate() {
                let StageGroup {
                    lane,
                    name,
                    instances,
                } = group;
                let key = (lane.clone(), name.clone());
                let expanded = self.expanded_stage_groups.contains(&key);
                let first = instances[0].1;
                let style = crate::theme::core_theme(cx).swatch(first.swatch);
                let count = instances.len();
                let total_share: f32 = instances.iter().map(|(_, r)| r.share).sum::<f32>().min(1.0);
                let total_pct = (total_share * 100.0).clamp(0.0, 100.0);
                let total_label = if count == 1 {
                    first.duration_text.clone()
                } else if total_pct < 0.1 {
                    "<0.1%".to_owned()
                } else {
                    format!("{total_pct:.1}%")
                };
                let toggle_key = key;
                let single_id = (count == 1).then_some(first.id);
                element = element.child(
                    div()
                        .id(("tx-stage-group", group_ix))
                        .debug_selector(move || format!("tx-stage-group-{group_ix}"))
                        .w_full()
                        .h(t.px(28.0))
                        .flex_none()
                        .px_3()
                        .py_1()
                        .cursor_pointer()
                        .hover(|style| style.bg(t.hover.bg))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            if let Some(id) = single_id {
                                view.dispatch(TransactionCommand::SelectStage(id), window, cx);
                            } else {
                                if !view.expanded_stage_groups.remove(&toggle_key) {
                                    view.expanded_stage_groups.insert(toggle_key.clone());
                                }
                                cx.notify();
                            }
                        }))
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().w(t.px(12.0)).flex_none().when(count > 1, |el| {
                                    el.child(
                                        Icon::new(if expanded {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        })
                                        .size(t.px(12.0))
                                        .color(t.panel.icon_muted),
                                    )
                                }))
                                .child(
                                    div()
                                        .w(t.px(10.0))
                                        .h(t.px(10.0))
                                        .flex_none()
                                        .rounded_sm()
                                        .bg(hsla(style.fill)),
                                )
                                .child(
                                    div()
                                        .w(relative(0.38))
                                        .min_w_0()
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .font_family(t.mono_font)
                                        .child(
                                            detail_text(
                                                cx,
                                                format!("tx-stage-group-name-{group_ix}"),
                                                name,
                                            )
                                            .flex_1(),
                                        )
                                        .when(count > 1, |el| {
                                            el.child(tag(t, &format!("×{count}")))
                                        })
                                        .when(!first.primary, |el| el.child(tag(t, &lane))),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .h(t.px(4.0))
                                        .rounded_sm()
                                        .bg(t.panel.bg)
                                        .child(
                                            div()
                                                .h_full()
                                                .w(relative(total_share.max(0.01)))
                                                .rounded_sm()
                                                .bg(hsla(style.fill)),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(t.px(72.0))
                                        .flex_none()
                                        .font_family(t.mono_font)
                                        .text_size(px(t.ui_size_small))
                                        .text_color(t.panel.text_muted)
                                        .text_ellipsis()
                                        .child(SharedString::from(total_label)),
                                ),
                        ),
                );
                if expanded && count > 1 {
                    for (index, row) in instances {
                        let id = row.id;
                        element = element.child(
                            div()
                                .id(("tx-stage-inst", index))
                                .debug_selector(move || format!("tx-stage-inst-{}", id.0))
                                .w_full()
                                .h(t.px(24.0))
                                .flex_none()
                                .pl(t.px(32.0))
                                .pr_3()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .cursor_pointer()
                                .when(row.selected, |el| el.bg(t.selection.bg))
                                .when(!row.selected, |el| el.hover(|s| s.bg(t.hover.bg)))
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.dispatch(TransactionCommand::SelectStage(id), window, cx);
                                }))
                                .child(
                                    div()
                                        .font_family(t.mono_font)
                                        .text_size(px(t.ui_size_small))
                                        .text_color(t.panel.text_muted)
                                        .child(SharedString::from(row.offset_text.clone())),
                                )
                                .child(
                                    div()
                                        .font_family(t.mono_font)
                                        .text_size(px(t.ui_size_small))
                                        .text_color(t.editor.text)
                                        .child(SharedString::from(row.duration_text.clone())),
                                ),
                        );
                    }
                }
            }
        } else {
            for (index, row) in section.rows.iter().enumerate() {
                element = element.child(stage_row(t, index, row, &self.stage_focus[&row.id], cx));
            }
            if section.cut() {
                element = element.child(
                    div().px_3().py_1().child(
                        Button::new("tx-stages-show-all")
                            .debug_selector(|| "tx-stages-show-all".into())
                            .label(format!("Show all {} stages", section.matched))
                            .ghost()
                            .small()
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.dispatch(TransactionCommand::StagesShowAll(true), window, cx);
                            })),
                    ),
                );
            } else if prefs.stages_show_all && section.total > prefs.detail_items {
                element = element.child(
                    div().px_3().py_1().child(
                        Button::new("tx-stages-show-fewer")
                            .debug_selector(|| "tx-stages-show-fewer".into())
                            .label("Show fewer")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.dispatch(TransactionCommand::StagesShowAll(false), window, cx);
                            })),
                    ),
                );
            }
        }
        element.into_any_element()
    }

    fn related(
        t: &crate::theme::Theme,
        section: &Section<RefRow>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut element = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.border_variant)
            .child(Self::heading(t, section, cx));
        if section.collapsed {
            return element.into_any_element();
        }
        let mut group = String::new();
        for (index, row) in section.rows.iter().enumerate() {
            if row.group != group {
                group.clone_from(&row.group);
                element = element.child(
                    div()
                        .px_3()
                        .pt_1()
                        .text_size(px(t.ui_size_small))
                        .text_color(t.panel.text_muted)
                        .child(SharedString::from(group.clone())),
                );
            }
            element = element.child(Self::reference(t, index, row, cx));
        }
        if let Some(note) = Self::cut_note(t, section) {
            element = element.child(note);
        }
        element.into_any_element()
    }

    fn reference(
        t: &crate::theme::Theme,
        index: usize,
        row: &RefRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (track, id) = (row.target_track(), row.target.transaction);
        let color = match row.role {
            RefRole::Relation { outgoing: false } => t.tx_relation_in,
            RefRole::Relation { outgoing: true } => t.tx_relation_out,
            _ => t.panel.icon_muted,
        };
        let subtitle = match (&row.label, row.loaded) {
            (Some(label), _) => label.clone(),
            (None, true) => "(no label)".into(),
            (None, false) => "Unloaded track".into(),
        };
        let action_label = if row.loaded {
            "Inspect ›"
        } else {
            "Load ›"
        };
        div()
            .id(("tx-ref", index))
            .debug_selector(move || format!("tx-ref-{index}"))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .when_some(track, |element, track| {
                element
                    .cursor_pointer()
                    .hover(|style| style.bg(t.hover.bg))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        window.focus(&view.focus, cx);
                        view.dispatch(TransactionCommand::Jump { track, id }, window, cx);
                    }))
            })
            .child(
                div()
                    .w(t.px(82.0))
                    .flex_none()
                    .text_ellipsis()
                    .font_family(t.mono_font)
                    .text_color(color)
                    .child(SharedString::from(format!("#{}", id.0))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(detail_text(
                        cx,
                        format!("tx-related-label-{index}"),
                        subtitle,
                    ))
                    .when_some(row.track.clone(), |element, path| {
                        element.child(
                            div()
                                .text_size(px(t.ui_size_small))
                                .text_color(t.panel.text_muted)
                                .child(detail_text(
                                    cx,
                                    format!("tx-related-path-{index}"),
                                    path.join("."),
                                )),
                        )
                    })
                    .when_some(row.relation_label.clone(), |element, label| {
                        element.child(
                            div()
                                .text_size(px(t.ui_size_small))
                                .text_color(t.panel.text_muted)
                                .child(detail_text(
                                    cx,
                                    format!("tx-related-relation-{index}"),
                                    label,
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .px_1p5()
                    .py_0p5()
                    .rounded_sm()
                    .text_size(px(t.ui_size_small))
                    .text_color(if row.loaded {
                        t.panel.text_muted
                    } else {
                        t.badge.text
                    })
                    .when(!row.loaded, |el| el.bg(t.badge.bg))
                    .child(action_label),
            )
            .into_any_element()
    }
}

fn detail_text(
    cx: &App,
    id: impl Into<gpui_kit::ElementId>,
    text: impl Into<SharedString>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let text = text.into();
    div()
        .id(id)
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .tooltip_show_delay(crate::ui::tooltip::delay(cx))
        .tooltip({
            let text = text.clone();
            move |window, cx| Tooltip::new(text.clone()).build(window, cx)
        })
        .child(text)
}

fn notice(t: &crate::theme::Theme, title: &str, detail: &str) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .p_4()
        .child(
            div()
                .text_color(t.editor.text_muted)
                .child(SharedString::from(title.to_owned())),
        )
        .when(!detail.is_empty(), |element| {
            element.child(
                div()
                    .text_size(px(t.ui_size_small))
                    .text_color(t.editor.text_placeholder)
                    .child(SharedString::from(detail.to_owned())),
            )
        })
        .into_any_element()
}

fn tag(t: &crate::theme::Theme, text: &str) -> AnyElement {
    div()
        .flex_none()
        .whitespace_nowrap()
        .px_1()
        .rounded_sm()
        .bg(t.badge.bg)
        .text_size(px(t.ui_size_small))
        .text_color(t.badge.text)
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

fn status_badge(
    cx: &App,
    t: &crate::theme::Theme,
    text: &str,
    status: volna_trace::data::transactions::Status,
) -> AnyElement {
    use volna_trace::data::transactions::Status;
    let color: Hsla = match status {
        Status::Error | Status::Aborted => t.editor.error,
        Status::Open => t.wave_event_coalesced,
        _ => t.panel.text_muted,
    };
    let raw = volna_core::data::text::status_name(status);
    div()
        .id("tx-status")
        .flex_none()
        .whitespace_nowrap()
        .px_1p5()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(t.panel.bg.blend(color.opacity(0.32)))
        .bg(t.panel.bg.blend(color.opacity(0.08)))
        .text_size(px(t.ui_size_small))
        .text_color(color)
        .when(status == Status::Open, gpui_kit::Styled::border_dashed)
        .tooltip_show_delay(crate::ui::tooltip::delay(cx))
        .tooltip(move |w, cx| Tooltip::new(raw).build(w, cx))
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

fn stage_row(
    t: &crate::theme::Theme,
    index: usize,
    row: &StageRow,
    focus: &FocusHandle,
    cx: &mut Context<TransactionPanelView>,
) -> AnyElement {
    let id = row.id;
    let click_focus = focus.clone();
    let key_focus = focus.clone();
    let style = crate::theme::core_theme(cx).swatch(row.swatch);
    div()
        .id(("tx-stage", index))
        .track_focus(focus)
        .tab_stop(true)
        .debug_selector(move || format!("tx-stage-{}", id.0))
        .role(gpui_kit::accesskit::Role::Button)
        .aria_label(format!(
            "Stage {}{} · {}",
            row.name,
            if row.selected { ", selected" } else { "" },
            row.duration_text,
        ))
        .w_full()
        .h(t.px(28.0))
        .flex_none()
        .px_3()
        .py_1()
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .cursor_pointer()
        .when(row.selected, |element| element.bg(t.selection.bg))
        .when(!row.selected, |element| {
            element.hover(|style| style.bg(t.hover.bg))
        })
        .focus_visible(|style| style.border_color(t.border_focused))
        .tooltip_show_delay(crate::ui::tooltip::delay(cx))
        .tooltip(|window, cx| Tooltip::new("Select stage to inspect attributes").build(window, cx))
        .on_key_down(
            cx.listener(move |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                if event.keystroke.key == "tab" {
                    if event.keystroke.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                    cx.stop_propagation();
                } else if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    view.dispatch(TransactionCommand::SelectStage(id), window, cx);
                    window.focus(&key_focus, cx);
                    cx.stop_propagation();
                }
            }),
        )
        .on_click(cx.listener(move |view, _, window, cx| {
            view.dispatch(TransactionCommand::SelectStage(id), window, cx);
            window.focus(&click_focus, cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(t.px(10.0))
                        .h(t.px(10.0))
                        .flex_none()
                        .rounded_sm()
                        .bg(hsla(style.fill)),
                )
                .child(
                    div()
                        .w(relative(0.38))
                        .min_w_0()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_1()
                        .font_family(t.mono_font)
                        .child(
                            detail_text(cx, format!("tx-stage-name-{index}"), row.name.clone())
                                .flex_1(),
                        )
                        .when(!row.primary, |element| element.child(tag(t, &row.lane))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h(t.px(4.0))
                        .rounded_sm()
                        .bg(t.panel.bg)
                        .child(
                            div()
                                .h_full()
                                .w(relative(row.share.max(0.01)))
                                .rounded_sm()
                                .bg(hsla(style.fill)),
                        ),
                )
                .child(
                    div()
                        .w(t.px(72.0))
                        .flex_none()
                        .font_family(t.mono_font)
                        .text_size(px(t.ui_size_small))
                        .text_color(t.panel.text_muted)
                        .text_ellipsis()
                        .child(SharedString::from(row.duration_text.clone())),
                ),
        )
        .into_any_element()
}
