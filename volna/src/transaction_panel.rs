//! The Transaction panel: everything recorded about one selected record.

#[cfg(not(target_arch = "wasm32"))]
use gpui_kit::ClipboardItem;
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
use volna_core::transaction::view::FILTER_THRESHOLD;
use volna_core::transaction::{
    AttrRow, Chip, EventRow, ParentTraceState, RefRole, RefRow, Section, StageRow,
    TransactionCommand, TxPanelState, TxView,
};

use crate::app::Workspace;
use crate::theme::{ThemePx, hsla, theme};
use crate::ui::{Icon, IconName, action_button};

const LANE_PX: f32 = 18.0;
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
    _subscriptions: Vec<Subscription>,
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
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter attributes"));
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
        match result {
            Some(Ok(text)) => {
                #[cfg(target_arch = "wasm32")]
                crate::table_clipboard::copy(&text);
                #[cfg(not(target_arch = "wasm32"))]
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            Some(Err(error)) => {
                owner.update(cx, |ws, cx| {
                    ws.dispatch_if_current(
                        self.generation,
                        Command::Notice(error.to_string()),
                        None,
                        cx,
                    );
                });
            }
            None => {}
        }
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

    fn toolbar_buttons(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Option<Vec<Button>> {
        use gpui_kit::assets::IconName as KitIcon;
        let owner = self.ws.upgrade()?;
        let model = owner.read(cx).app.panels.transaction(self.id)?;
        let pinned = model.pinned();
        Some(vec![
            action_button("tx-pin", KitIcon::Pin, cx)
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
        ])
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
        let (state, follows, revealable, back, forward) = {
            let ws = owner.read(cx);
            let Some(model) = ws.app.panels.transaction(self.id) else {
                return div().into_any_element();
            };
            let follows = if model.pinned() {
                Some("pinned".to_owned())
            } else {
                ws.app
                    .doc
                    .selection()
                    .and_then(|selection| ws.app.panels.get(selection.origin))
                    .map(|panel| format!("follows {}", panel.title()))
            };
            let revealable = model
                .shown()
                .and_then(|shown| Some((shown.track.track()?, shown.id)))
                .map(|(track, id)| {
                    ws.app
                        .panels
                        .pipeline_showing(&ws.app.doc, track, id)
                        .is_some()
                });
            (
                model.state(&ws.app.doc),
                follows,
                revealable,
                model.can_go_back(),
                model.can_go_forward(),
            )
        };
        let body = match state {
            TxPanelState::Ready(view) => self.render_view(*view, revealable, window, cx),
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
        let follows = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .h(t.px(28.0))
            .flex_none()
            .text_size(px(t.ui_size_small))
            .text_color(t.panel.text_muted)
            .when_some(follows, |element, text| {
                element.child(
                    div()
                        .debug_selector(|| "tx-follows".into())
                        .min_w_0()
                        .text_ellipsis()
                        .px_1p5()
                        .py_0p5()
                        .rounded_sm()
                        .bg(t.badge.bg)
                        .text_color(t.badge.text)
                        .child(SharedString::from(text)),
                )
            });
        let navigation = div()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .pb_1()
            .child(
                action_button("tx-back", gpui_kit::assets::IconName::ChevronLeft, cx)
                    .debug_selector(|| "tx-back".into())
                    .label("Back")
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
                    .label("Forward")
                    .ghost()
                    .tooltip(FORWARD_TOOLTIP)
                    .disabled(!forward)
                    .on_click(cx.listener(|view, _, window, cx| {
                        window.focus(&view.focus, cx);
                        view.dispatch(TransactionCommand::Forward, window, cx);
                    })),
            );
        let header = div()
            .debug_selector(|| "tx-header".into())
            .flex()
            .flex_col()
            .flex_none()
            .border_b_1()
            .border_color(t.border_variant)
            .child(follows)
            .child(navigation);
        div()
            .id(("transaction-panel", self.id.0))
            .size_full()
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
                        if let Some(origin) = view
                            .ws
                            .upgrade()
                            .and_then(|ws| ws.read(cx).app.doc.selection())
                            .map(|selection| selection.origin)
                        {
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
        &self,
        view: TxView,
        revealable: Option<bool>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *theme(cx);
        let muted = t.panel.text_muted;
        let identity = &view.identity;
        let mut breadcrumb = div()
            .debug_selector(|| "tx-record-path".into())
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(t.ui_size_small))
            .text_color(muted)
            .child(Icon::new(IconName::Workflow).size(t.px(12.0)).color(muted));
        if !identity.scope.is_empty() {
            breadcrumb = breadcrumb.child(SharedString::from(identity.scope.join(".")));
            breadcrumb = breadcrumb.child("›");
        }
        breadcrumb = breadcrumb
            .child(SharedString::from(identity.generator.clone()))
            .child(SharedString::from(format!("· row {}", identity.ordinal)));

        let mut title = div().flex().items_center().gap_2().child(
            div()
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
            div()
                .flex_1()
                .min_w_0()
                .font_family(t.mono_font)
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(SharedString::from(
                    identity
                        .label
                        .clone()
                        .unwrap_or_else(|| "(no label)".into()),
                )),
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
            title = title.child(status_badge(&t, identity.status_text, identity.status));
        }
        if let Some(kind) = identity.kind.filter(|_| view.point_kind.is_none()) {
            title = title.child(
                div()
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
            if view.point_kind.is_some() {
                point_metadata = point_metadata.child(parent);
            } else {
                title = title.child(parent);
            }
        }

        let timing = &view.timing;
        let mut tiles = div().flex().gap_2();
        for (label, text, time) in [
            ("Begin", timing.begin_text.clone(), Some(timing.begin)),
            ("End", timing.end_text.clone(), timing.end),
            ("Duration", timing.duration_text.clone(), None),
        ] {
            if view.point_kind.is_none() || label == "Begin" {
                tiles = tiles.child(Self::tile(
                    &t,
                    if view.point_kind.is_some() {
                        "Timestamp"
                    } else {
                        label
                    },
                    text,
                    time,
                    cx,
                ));
            }
        }

        let mut actions = div().flex().items_center().gap_1().flex_wrap();
        let reveal_label = match revealable {
            Some(true) => "Reveal in Pipeline",
            _ => "Open in Pipeline",
        };
        actions = actions
            .when(view.point_kind.is_none(), |actions| {
                actions.child(
                    action_button(
                        "tx-reveal",
                        gpui_kit::component::Icon::empty().path(IconName::Locate.path()),
                        cx,
                    )
                    .label(reveal_label)
                    .ghost()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.send(Command::RevealTransaction { panel: view.id }, window, cx);
                    })),
                )
            })
            .child(
                Button::new("tx-fit")
                    .label(if view.point_kind.is_some() {
                        "Zoom here"
                    } else {
                        "Fit lifetime"
                    })
                    .ghost()
                    .small()
                    .tooltip("Fit the shared viewport to this record")
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.dispatch(TransactionCommand::FitLifetime, window, cx);
                    })),
            )
            .child(
                Button::new("tx-copy")
                    .label("Copy")
                    .ghost()
                    .small()
                    .tooltip("Copy the complete record as TSV")
                    .on_click(cx.listener(|view, _, _, cx| view.copy(cx))),
            );

        let mut body = div()
            .debug_selector(|| "tx-body".into())
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
                    .when(view.point_kind.is_some(), |body| body.child(point_metadata))
                    .child(tiles)
                    .when(view.point_kind.is_none(), |body| {
                        body.child(Self::lifeline(&t, &view, cx))
                    })
                    .child(actions),
            );

        body = body.child(self.attributes(&t, &view, cx));
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
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(t.mono_font)
                                .child(SharedString::from(value.clone())),
                        ),
                );
            }
            body = body.child(source);
        }
        if view.point_kind.is_none() {
            body = body.child(Self::section(&t, &view.stages, cx, stage_row));
            body = body.child(Self::section(&t, &view.events, cx, event_row));
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
                    .child("Values were cut at the admitted byte limit."),
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

    fn tile(
        t: &crate::theme::Theme,
        label: &'static str,
        text: String,
        time: Option<u64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tile = div()
            .flex()
            .flex_col()
            .gap_0p5()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(t.panel.bg)
            .border_1()
            .border_color(t.border_variant)
            .child(
                div()
                    .text_size(px(t.ui_size_small))
                    .text_color(t.panel.text_muted)
                    .child(label),
            )
            .child(
                div()
                    .font_family(t.mono_font)
                    .child(SharedString::from(text)),
            );
        match time {
            Some(time) => tile
                .id(SharedString::from(format!("tx-time-{label}")))
                .cursor_pointer()
                .hover(|style| style.bg(t.hover.bg))
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.dispatch(TransactionCommand::Cursor(time), window, cx);
                }))
                .into_any_element(),
            None => tile.into_any_element(),
        }
    }

    fn lifeline(t: &crate::theme::Theme, view: &TxView, cx: &mut Context<Self>) -> AnyElement {
        let mut lanes = div().flex().flex_col().gap_0p5().relative();
        for (index, lane) in view.lifeline.iter().enumerate() {
            let mut row = div()
                .relative()
                .w_full()
                .h(t.px(LANE_PX))
                .bg(t.panel.bg)
                .rounded_sm();
            for (cell_ix, cell) in lane.cells.iter().enumerate() {
                let begin = cell.begin;
                let style = crate::theme::core_theme(cx).swatch(cell.swatch);
                row = row.child(
                    div()
                        .id(SharedString::from(format!("tx-cell-{index}-{cell_ix}")))
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(cell.start))
                        .w(relative(cell.width.max(0.004)))
                        .bg(hsla(style.fill))
                        .border_1()
                        .border_color(hsla(style.edge))
                        .cursor_pointer()
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(t.ui_size_small))
                        .text_color(hsla(style.text))
                        .font_family(t.mono_font)
                        .child(SharedString::from(cell.name.clone()))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.dispatch(TransactionCommand::Cursor(begin), window, cx);
                        })),
                );
            }
            lanes = lanes.child(row);
        }
        for (index, tick) in view.event_ticks.iter().enumerate() {
            let time = tick.time;
            lanes = lanes.child(
                div()
                    .id(("tx-event-tick", index))
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(tick.at))
                    .w(t.px(2.0))
                    .bg(t.wave_event_coalesced)
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.dispatch(TransactionCommand::Cursor(time), window, cx);
                    })),
            );
        }
        if let Some(at) = view.cursor {
            lanes = lanes.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(at))
                    .w(px(1.0))
                    .bg(t.wave_cursor),
            );
        }
        lanes.into_any_element()
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
            .py_1()
            .cursor_pointer()
            .hover(|style| style.bg(t.hover.bg))
            .text_size(px(t.ui_size_small))
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .text_color(t.panel.text_muted)
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(t.px(12.0))
                .color(t.panel.icon_muted),
            )
            .child(SharedString::from(format!("{} · {}", title, section.total)))
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

    fn section<T>(
        t: &crate::theme::Theme,
        section: &Section<T>,
        cx: &mut Context<Self>,
        row: impl Fn(&crate::theme::Theme, usize, &T, &mut Context<Self>) -> AnyElement,
    ) -> AnyElement {
        let mut element = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.border_variant)
            .child(Self::heading(t, section, cx));
        if !section.collapsed {
            for (index, entry) in section.rows.iter().enumerate() {
                element = element.child(row(t, index, entry, cx));
            }
            if let Some(note) = Self::cut_note(t, section) {
                element = element.child(note);
            }
        }
        element.into_any_element()
    }

    fn attributes(
        &self,
        t: &crate::theme::Theme,
        view: &TxView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let section = &view.attributes;
        let mut element = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.border_variant)
            .child(Self::named_heading(
                t,
                section,
                if view.point_kind.is_some() {
                    "Arguments"
                } else {
                    "Attributes"
                },
                cx,
            ));
        if section.collapsed {
            return element.into_any_element();
        }
        if section.total > FILTER_THRESHOLD {
            element = element.child(div().px_3().pb_1().child(Input::new(&self.filter).small()));
        }
        for row in &section.rows {
            element = element.child(Self::attribute_row(t, row, view.identity.id, cx));
        }
        if let Some(note) = Self::cut_note(t, section) {
            element = element.child(note);
        }
        element.into_any_element()
    }

    fn attribute_row(
        t: &crate::theme::Theme,
        row: &AttrRow,
        record: volna_trace::data::transactions::TransactionRef,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let attribute = row.attribute;
        let value = div()
            .flex_1()
            .min_w_0()
            .font_family(t.mono_font)
            .child(SharedString::from(row.value.clone()));
        let value = match row.radix {
            Some(radix) => value
                .id(SharedString::from(format!("tx-attr-{attribute}")))
                .cursor_pointer()
                .hover(|style| style.bg(t.hover.bg))
                .tooltip({
                    let hint = format!("{} · click to cycle the radix", radix.name());
                    move |w, cx| Tooltip::new(hint.clone()).build(w, cx)
                })
                .on_click(cx.listener(move |view, _, window, cx| {
                    let key = view.ws.upgrade().and_then(|owner| {
                        let ws = owner.read(cx);
                        ws.app.panels.transaction(view.id)?.attribute_key(
                            &ws.app.doc,
                            record,
                            attribute,
                        )
                    });
                    if let Some(key) = key {
                        view.dispatch(TransactionCommand::Radix(key), window, cx);
                    }
                }))
                .into_any_element(),
            None => value.into_any_element(),
        };
        let owner = cx.entity().downgrade();
        let key = row.key.clone();
        div()
            .flex()
            .items_start()
            .gap_2()
            .px_3()
            .py_0p5()
            .child(
                div()
                    .w(t.px(150.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(t.panel.text_muted)
                    .child(SharedString::from(row.key.clone()))
                    .when_some(row.phase, |element, phase| element.child(tag(t, phase))),
            )
            .child(value)
            .context_menu(move |menu, _, _| {
                let owner = owner.clone();
                let key = key.clone();
                menu.item(PopupMenuItem::new("Show as table column").on_click(
                    move |_, window, cx| {
                        drop(owner.update(cx, |view, cx| {
                            view.send(
                                Command::ShowAttributeColumn {
                                    from: view.id,
                                    key: key.clone(),
                                },
                                window,
                                cx,
                            );
                        }));
                    },
                ))
            })
            .into_any_element()
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
            (None, false) => "not loaded · loads on demand".into(),
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
                    .w(t.px(52.0))
                    .flex_none()
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
                    .child(SharedString::from(subtitle))
                    .when_some(row.track.clone(), |element, path| {
                        element.child(
                            div()
                                .text_size(px(t.ui_size_small))
                                .text_color(t.panel.text_muted)
                                .child(SharedString::from(path.join("."))),
                        )
                    })
                    .when_some(row.relation_label.clone(), |element, label| {
                        element.child(
                            div()
                                .text_size(px(t.ui_size_small))
                                .text_color(t.panel.text_muted)
                                .child(SharedString::from(label)),
                        )
                    }),
            )
            .child(div().text_color(t.panel.text_muted).child("›"))
            .into_any_element()
    }
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
        .px_1()
        .rounded_sm()
        .bg(t.badge.bg)
        .text_size(px(t.ui_size_small))
        .text_color(t.badge.text)
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

fn status_badge(
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
        .px_1p5()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(color)
        .text_size(px(t.ui_size_small))
        .text_color(color)
        .when(status == Status::Open, gpui_kit::Styled::border_dashed)
        .tooltip(move |w, cx| Tooltip::new(raw).build(w, cx))
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

fn chips(t: &crate::theme::Theme, items: &[Chip], total: usize) -> AnyElement {
    let mut element = div().flex().flex_wrap().gap_1();
    for chip in items {
        element = element.child(
            div()
                .px_1()
                .rounded_sm()
                .bg(t.badge.bg)
                .font_family(t.mono_font)
                .text_size(px(t.ui_size_small))
                .text_color(t.badge.text)
                .child(SharedString::from(format!("{} {}", chip.key, chip.value))),
        );
    }
    if items.len() < total {
        element = element.child(
            div()
                .text_size(px(t.ui_size_small))
                .text_color(t.panel.text_muted)
                .child(SharedString::from(format!("+{}", total - items.len()))),
        );
    }
    element.into_any_element()
}

fn stage_row(
    t: &crate::theme::Theme,
    index: usize,
    row: &StageRow,
    cx: &mut Context<TransactionPanelView>,
) -> AnyElement {
    let begin = row.begin;
    let style = crate::theme::core_theme(cx).swatch(row.swatch);
    div()
        .id(("tx-stage", index))
        .flex()
        .items_start()
        .gap_2()
        .px_3()
        .py_1()
        .cursor_pointer()
        .hover(|style| style.bg(t.hover.bg))
        .on_click(cx.listener(move |view, _, window, cx| {
            view.dispatch(TransactionCommand::Cursor(begin), window, cx);
        }))
        .child(
            div()
                .mt(t.px(4.0))
                .w(t.px(10.0))
                .h(t.px(10.0))
                .flex_none()
                .rounded_sm()
                .bg(hsla(style.fill)),
        )
        .child(
            div()
                .w(t.px(140.0))
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .font_family(t.mono_font)
                .child(SharedString::from(row.name.clone()))
                .when(!row.primary, |element| element.child(tag(t, &row.lane))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .w_full()
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
                .child(chips(t, &row.attributes, row.attributes_total)),
        )
        .child(
            div()
                .w(t.px(72.0))
                .flex_none()
                .font_family(t.mono_font)
                .text_size(px(t.ui_size_small))
                .text_color(t.panel.text_muted)
                .child(SharedString::from(row.duration_text.clone())),
        )
        .into_any_element()
}

fn event_row(
    t: &crate::theme::Theme,
    index: usize,
    row: &EventRow,
    cx: &mut Context<TransactionPanelView>,
) -> AnyElement {
    let time = row.time;
    div()
        .id(("tx-event", index))
        .flex()
        .items_start()
        .gap_2()
        .px_3()
        .py_1()
        .cursor_pointer()
        .hover(|style| style.bg(t.hover.bg))
        .on_click(cx.listener(move |view, _, window, cx| {
            view.dispatch(TransactionCommand::Cursor(time), window, cx);
        }))
        .child(
            div()
                .w(t.px(150.0))
                .flex_none()
                .font_family(t.mono_font)
                .child(SharedString::from(row.name.clone())),
        )
        .child(
            div()
                .w(t.px(100.0))
                .flex_none()
                .font_family(t.mono_font)
                .text_size(px(t.ui_size_small))
                .text_color(t.panel.text_muted)
                .child(SharedString::from(row.time_text.clone())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(chips(t, &row.attributes, row.attributes_total)),
        )
        .into_any_element()
}
