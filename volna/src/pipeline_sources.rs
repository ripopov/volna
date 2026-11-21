//! In-panel source focus strip and collapsible Sources composer drawer for merged pipeline panels.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use gpui_kit::base::Disableable;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, CursorStyle, Entity, IntoElement, Render, SharedString, Subscription,
    WeakEntity, Window, div, px,
};
use volna_core::app::Command;
use volna_core::panels::PanelId;
use volna_core::pipeline::{FocusStyle, PipelineSourceCommand, SourceLoadStatus};
use volna_core::trace::Traced;
use volna_trace::data::transactions::TrackRef;

use crate::app::Workspace;
use crate::theme::{ThemePx, theme};

const COMPACT_CHIP_THRESHOLD: usize = 8;
const MAX_AVAILABLE_ITEMS: usize = 64;

struct AvailableCache {
    key: u64,
    query: String,
    items: Vec<(String, Traced<TrackRef>)>,
}

struct ChipSpec {
    index: usize,
    color_slot: usize,
    dist: String,
    badge: String,
    full_path: String,
    status: SourceLoadStatus,
    eff_visible: bool,
    is_focused: bool,
    is_dimmed: bool,
}

struct DrawerRowSpec {
    index: usize,
    color_slot: usize,
    path: String,
    auto_badge: String,
    eff_visible: bool,
    status: SourceLoadStatus,
}

pub(crate) struct PipelineSourcesView {
    ws: WeakEntity<Workspace>,
    panel: PanelId,
    generation: u64,
    drawer_open: bool,
    search: Entity<InputState>,
    alias_inputs: HashMap<usize, (Entity<InputState>, String, Subscription)>,
    last_fingerprint: u64,
    alias_sync_key: u64,
    available_cache: Option<AvailableCache>,
    _search_sub: Subscription,
}

impl PipelineSourcesView {
    pub(crate) fn new(
        ws: WeakEntity<Workspace>,
        panel: PanelId,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if let Some(owner) = ws.upgrade() {
            cx.observe(&owner, |view, owner, cx| {
                let fp = view.compute_fingerprint(&owner, cx);
                if fp != view.last_fingerprint {
                    view.last_fingerprint = fp;
                    cx.notify();
                }
            })
            .detach();
        }
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter transaction generators…"));
        let search_sub = cx.subscribe_in(&search, window, |_, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            ws,
            panel,
            generation,
            drawer_open: false,
            search,
            alias_inputs: HashMap::new(),
            last_fingerprint: 0,
            alias_sync_key: 0,
            available_cache: None,
            _search_sub: search_sub,
        }
    }

    pub(crate) fn is_drawer_open(&self) -> bool {
        self.drawer_open
    }

    pub(crate) fn toggle_drawer(&mut self, cx: &mut Context<Self>) {
        self.drawer_open = !self.drawer_open;
        self.last_fingerprint = 0;
        cx.notify();
    }

    fn compute_fingerprint(&self, owner: &Entity<Workspace>, cx: &Context<Self>) -> u64 {
        let app = &owner.read(cx).app;
        let mut h = DefaultHasher::new();
        self.drawer_open.hash(&mut h);
        let Some(pipeline) = app.panels.pipeline(self.panel) else {
            0u8.hash(&mut h);
            return h.finish();
        };
        1u8.hash(&mut h);
        pipeline.sources.len().hash(&mut h);
        pipeline.focused_source.hash(&mut h);
        std::mem::discriminant(&pipeline.focus_style).hash(&mut h);
        pipeline.disjoint_notice().is_some().hash(&mut h);
        for (i, s) in pipeline.sources.iter().enumerate() {
            s.color_slot.hash(&mut h);
            s.visible.hash(&mut h);
            s.alias.hash(&mut h);
            s.track.trace().hash(&mut h);
            s.track.track().hash(&mut h);
            match pipeline.source_status(&app.doc, i) {
                SourceLoadStatus::Ready { count } => {
                    0u8.hash(&mut h);
                    count.hash(&mut h);
                }
                SourceLoadStatus::Loading => 1u8.hash(&mut h),
                SourceLoadStatus::Failed(err) => {
                    2u8.hash(&mut h);
                    err.hash(&mut h);
                }
                SourceLoadStatus::Unresolved => 3u8.hash(&mut h),
                SourceLoadStatus::Unavailable => 4u8.hash(&mut h),
            }
        }
        if self.drawer_open {
            pipeline.row_count(&app.doc).hash(&mut h);
            for (trace, session) in app.doc.traces().loaded() {
                trace.hash(&mut h);
                session.tracks().len().hash(&mut h);
            }
        }
        h.finish()
    }

    fn dispatch(
        &self,
        command: PipelineSourceCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        drop(self.ws.update(cx, |ws, cx| {
            ws.dispatch_if_current(
                self.generation,
                Command::PipelineSources(self.panel, command),
                Some(window),
                cx,
            );
        }));
    }

    fn sync_alias_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(owner) = self.ws.upgrade() else {
            self.alias_inputs.clear();
            self.alias_sync_key = 0;
            return;
        };
        let key = {
            let app = &owner.read(cx).app;
            let Some(pipeline) = app.panels.pipeline(self.panel) else {
                self.alias_inputs.clear();
                self.alias_sync_key = 0;
                return;
            };
            let badges = pipeline.badges();
            let mut h = DefaultHasher::new();
            pipeline.sources.len().hash(&mut h);
            for (i, s) in pipeline.sources.iter().enumerate() {
                s.color_slot.hash(&mut h);
                s.alias.hash(&mut h);
                if let Some(lbl) = badges.labels.get(i) {
                    lbl.auto_badge.hash(&mut h);
                }
            }
            h.finish()
        };
        if key == self.alias_sync_key && !self.alias_inputs.is_empty() {
            return;
        }
        self.alias_sync_key = key;

        let specs: Vec<(usize, String, String)> = {
            let app = &owner.read(cx).app;
            let Some(pipeline) = app.panels.pipeline(self.panel) else {
                self.alias_inputs.clear();
                return;
            };
            let badges = pipeline.badges();
            pipeline
                .sources
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let placeholder = badges
                        .labels
                        .get(i)
                        .map_or_else(String::new, |l| l.auto_badge.clone());
                    (
                        s.color_slot,
                        s.alias.clone().unwrap_or_default(),
                        placeholder,
                    )
                })
                .collect()
        };
        let active_slots: HashSet<usize> = specs.iter().map(|(slot, _, _)| *slot).collect();
        self.alias_inputs
            .retain(|slot, _| active_slots.contains(slot));

        for (slot, alias_val, placeholder) in specs {
            if let Some((input, last_placeholder, _)) = self.alias_inputs.get_mut(&slot) {
                let current = input.read(cx).value().to_string();
                let value_changed = current.trim() != alias_val;
                let placeholder_changed = *last_placeholder != placeholder;
                if value_changed || placeholder_changed {
                    last_placeholder.clone_from(&placeholder);
                    input.update(cx, |state, cx| {
                        if value_changed {
                            state.set_value(alias_val.clone(), window, cx);
                        }
                        if placeholder_changed {
                            state.set_placeholder(placeholder.clone(), window, cx);
                        }
                    });
                }
            } else {
                let placeholder_stored = placeholder.clone();
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(alias_val)
                        .placeholder(placeholder)
                });
                let sub =
                    cx.subscribe_in(&input, window, move |view, entity, event, window, cx| {
                        if !matches!(event, InputEvent::Change) {
                            return;
                        }
                        let val = entity.read(cx).value().to_string();
                        let index = view.ws.upgrade().and_then(|owner| {
                            owner
                                .read(cx)
                                .app
                                .panels
                                .pipeline(view.panel)?
                                .sources
                                .iter()
                                .position(|s| s.color_slot == slot)
                        });
                        if let Some(index) = index {
                            let next = (!val.trim().is_empty()).then_some(val);
                            view.dispatch(PipelineSourceCommand::SetAlias(index, next), window, cx);
                        }
                    });
                self.alias_inputs
                    .insert(slot, (input, placeholder_stored, sub));
            }
        }
    }

    fn render_chip(spec: ChipSpec, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = *theme(cx);
        let i = spec.index;
        let accent = t.source_accent(spec.color_slot);
        let status_mark = match &spec.status {
            SourceLoadStatus::Loading => Some(("…", None)),
            SourceLoadStatus::Failed(err) => Some(("!", Some(err.clone()))),
            SourceLoadStatus::Unresolved => Some(("?", None)),
            _ => None,
        };
        let tooltip_text = match &status_mark {
            Some((_, Some(err))) => {
                format!("{} [{}] — Load failed: {err}", spec.full_path, spec.badge)
            }
            _ => format!(
                "{} ({}) [{}] — Click to Solo, double-click to Isolate, click ● to hide",
                spec.full_path, spec.dist, spec.badge
            ),
        };
        let chip_bg = if spec.is_focused {
            t.selection.bg
        } else {
            t.bar.bg
        };
        let chip_border = if spec.is_focused { accent } else { t.border };
        let chip_opacity = if !spec.eff_visible {
            0.38
        } else if spec.is_dimmed {
            0.48
        } else {
            1.0
        };
        let eye_label = if spec.eff_visible { "●" } else { "○" };
        let eye_tooltip = if spec.eff_visible {
            format!("Hide {}", spec.dist)
        } else {
            format!("Show {}", spec.dist)
        };

        let mut chip = div()
            .id(("source-chip", i))
            .debug_selector(move || format!("source-chip-{i}"))
            .flex()
            .items_center()
            .gap(t.px(if compact { 4.0 } else { 5.0 }))
            .px(t.px(if compact { 5.0 } else { 6.0 }))
            .py(t.px(2.0))
            .rounded(t.px(4.0))
            .border_1()
            .border_color(chip_border)
            .bg(chip_bg)
            .opacity(chip_opacity)
            .cursor(CursorStyle::PointingHand)
            .font_family(t.mono_font)
            .text_size(px(t.ui_size_small * 0.92))
            .text_color(t.panel.text)
            .hover(move |s| s.bg(t.hover.bg))
            .tooltip_show_delay(crate::ui::tooltip::delay(cx))
            .tooltip(move |w, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tooltip_text.clone()).build(w, cx)
            })
            .on_hover(cx.listener(move |view, hovered: &bool, window, cx| {
                if *hovered {
                    view.dispatch(PipelineSourceCommand::Hover(Some(i)), window, cx);
                } else {
                    // Only clear hover if this chip still owns it, preventing a leave event
                    // from clobbering an adjacent chip's enter event.
                    let is_current = view.ws.upgrade().is_some_and(|owner| {
                        owner
                            .read(cx)
                            .app
                            .panels
                            .pipeline(view.panel)
                            .is_some_and(|p| p.hovered_source == Some(i))
                    });
                    if is_current {
                        view.dispatch(PipelineSourceCommand::Hover(None), window, cx);
                    }
                }
            }))
            .on_click(
                cx.listener(move |view, ev: &gpui_kit::ClickEvent, window, cx| {
                    let cmd = if ev.click_count() >= 2 {
                        PipelineSourceCommand::Isolate(i)
                    } else {
                        PipelineSourceCommand::ToggleFocus(i)
                    };
                    view.dispatch(cmd, window, cx);
                }),
            )
            .child(
                div()
                    .size(t.px(8.0))
                    .flex_none()
                    .rounded(t.px(2.0))
                    .bg(accent),
            );

        if compact {
            let mut badge_el = div()
                .text_color(accent)
                .child(SharedString::from(spec.badge));
            if !spec.eff_visible {
                badge_el = badge_el.line_through();
            }
            chip = chip.child(badge_el);
        } else {
            let mut dist_el = div()
                .max_w(t.px(150.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(spec.dist));
            if !spec.eff_visible {
                dist_el = dist_el.line_through();
            }
            chip = chip.child(dist_el).child(
                div()
                    .text_color(accent)
                    .child(SharedString::from(format!("[{}]", spec.badge))),
            );
        }

        if let Some((mark, _)) = status_mark {
            chip = chip.child(div().text_color(t.panel.warning).child(mark));
        }

        chip.child(
            div()
                .id(("source-eye", i))
                .debug_selector(move || format!("source-eye-{i}"))
                .px(t.px(3.0))
                .rounded(t.px(3.0))
                .text_color(t.panel.text_muted)
                .hover(move |s| s.bg(t.elevated.bg).text_color(t.panel.text))
                .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                .tooltip(move |w, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(eye_tooltip.clone()).build(w, cx)
                })
                .on_click(cx.listener(move |view, _, window, cx| {
                    cx.stop_propagation();
                    view.dispatch(PipelineSourceCommand::ToggleVisible(i), window, cx);
                }))
                .child(eye_label),
        )
        .into_any_element()
    }

    fn render_focus_controls(
        focused: Option<usize>,
        focus_style: FocusStyle,
        any_hidden: bool,
        compact_chips: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *theme(cx);
        let mut right = div().flex_none().flex().items_center().gap(t.px(6.0));

        if focused.is_some() {
            let dim_active = focus_style == FocusStyle::Dim;
            let iso_active = focus_style == FocusStyle::Isolate;
            let seg_variant = |active: bool| {
                if active {
                    ButtonCustomVariant::new(cx)
                        .color(t.selection.bg)
                        .foreground(t.panel.text)
                        .hover(t.selection.bg)
                        .active(t.selection.bg)
                } else {
                    ButtonCustomVariant::new(cx)
                        .color(gpui_kit::transparent_black())
                        .foreground(t.panel.text_muted)
                        .hover(t.hover.bg)
                        .active(t.selection.bg)
                }
            };
            right = right.child(
                div()
                    .flex()
                    .items_center()
                    .gap(t.px(2.0))
                    .p(t.px(2.0))
                    .rounded(t.px(4.0))
                    .bg(t.bar.bg)
                    .border_1()
                    .border_color(t.border)
                    .child(
                        Button::new("focus-mode-dim")
                            .debug_selector(|| "focus-mode-dim".into())
                            .label("Solo (Dim)")
                            .xsmall()
                            .custom(seg_variant(dim_active))
                            .border_1()
                            .border_color(if dim_active {
                                t.border_focused
                            } else {
                                gpui_kit::transparent_black()
                            })
                            .tooltip(
                                "Keep peer rows visible at reduced contrast for cross-slot timing context",
                            )
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.dispatch(
                                    PipelineSourceCommand::SetFocusStyle(FocusStyle::Dim),
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("focus-mode-isolate")
                            .debug_selector(|| "focus-mode-isolate".into())
                            .label("Isolate (Hide)")
                            .xsmall()
                            .custom(seg_variant(iso_active))
                            .border_1()
                            .border_color(if iso_active {
                                t.border_focused
                            } else {
                                gpui_kit::transparent_black()
                            })
                            .tooltip(
                                "Hide peer rows and collapse to this source's contiguous rows",
                            )
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.dispatch(
                                    PipelineSourceCommand::SetFocusStyle(FocusStyle::Isolate),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            );
        }

        if focused.is_some() || any_hidden {
            right = right.child(
                Button::new("sources-show-all")
                    .debug_selector(|| "sources-show-all".into())
                    .label("Show All")
                    .xsmall()
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .color(t.selection.bg)
                            .foreground(t.panel.text)
                            .hover(t.hover.bg)
                            .active(t.selection.bg),
                    )
                    .border_1()
                    .border_color(t.border_focused)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.dispatch(PipelineSourceCommand::ClearFocusAndShowAll, window, cx);
                    })),
            );
        } else if !compact_chips {
            right = right.child(
                div()
                    .font_family(t.mono_font)
                    .text_size(px(t.ui_size_small * 0.88))
                    .text_color(t.panel.text_placeholder)
                    .child("Click to Solo · Double-click to Isolate"),
            );
        }

        right.into_any_element()
    }

    fn render_chip_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(owner) = self.ws.upgrade() else {
            return div().into_any_element();
        };
        let t = *theme(cx);
        let (chips_data, focused, focus_style, any_hidden, source_count) = {
            let app = &owner.read(cx).app;
            let Some(pipeline) = app.panels.pipeline(self.panel) else {
                return div().into_any_element();
            };
            let badges = pipeline.badges();
            let chips_data: Vec<ChipSpec> = pipeline
                .sources
                .iter()
                .enumerate()
                .map(|(index, s)| {
                    let lbl = badges.labels.get(index);
                    let dist =
                        lbl.map_or_else(|| format!("#{index}"), |l| l.distinguishing.clone());
                    let badge = lbl.map_or_else(|| format!("#{index}"), |l| l.badge.clone());
                    let eff_visible = pipeline.is_source_effectively_visible(index);
                    let is_focused = pipeline.focused_source == Some(index);
                    let is_dimmed = pipeline.focused_source.is_some()
                        && pipeline.focus_style == FocusStyle::Dim
                        && !is_focused
                        && eff_visible;
                    ChipSpec {
                        index,
                        color_slot: s.color_slot,
                        dist,
                        badge,
                        full_path: s.track.path().join("."),
                        status: pipeline.source_status(&app.doc, index),
                        eff_visible,
                        is_focused,
                        is_dimmed,
                    }
                })
                .collect();
            let any_hidden = pipeline.sources.iter().any(|s| !s.visible);
            (
                chips_data,
                pipeline.focused_source,
                pipeline.focus_style,
                any_hidden,
                pipeline.sources.len(),
            )
        };

        let compact_chips = source_count > COMPACT_CHIP_THRESHOLD;
        let mut chips_wrap = div()
            .id(("pipeline-source-chips", self.panel.0))
            .flex_1()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(t.px(if compact_chips { 4.0 } else { 5.0 }))
            .max_h(t.px(96.0))
            .overflow_y_scroll()
            .child(
                div()
                    .font_family(t.mono_font)
                    .text_size(px(t.ui_size_small * 0.92))
                    .text_color(t.panel.text_placeholder)
                    .mr(t.px(2.0))
                    .child("Sources:"),
            );

        for spec in chips_data {
            chips_wrap = chips_wrap.child(Self::render_chip(spec, compact_chips, cx));
        }

        div()
            .id(("pipeline-source-strip", self.panel.0))
            .debug_selector(|| "pipeline-source-strip".into())
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap(t.px(8.0))
            .px(t.px(8.0))
            .py(t.px(4.0))
            .bg(t.panel.bg)
            .border_b_1()
            .border_color(t.border)
            .child(chips_wrap)
            .child(Self::render_focus_controls(
                focused,
                focus_style,
                any_hidden,
                compact_chips,
                cx,
            ))
            .into_any_element()
    }

    fn available_generators(
        &mut self,
        owner: &Entity<Workspace>,
        query: &str,
        cx: &Context<Self>,
    ) -> Vec<(String, Traced<TrackRef>)> {
        let app = &owner.read(cx).app;
        let Some(pipeline) = app.panels.pipeline(self.panel) else {
            return Vec::new();
        };
        let mut h = DefaultHasher::new();
        for (trace, session) in app.doc.traces().loaded() {
            trace.hash(&mut h);
            session.tracks().len().hash(&mut h);
        }
        for s in &pipeline.sources {
            s.track.track().hash(&mut h);
        }
        let cache_key = h.finish();
        if let Some(cached) = &self.available_cache
            && cached.key == cache_key
            && cached.query == query
        {
            return cached.items.clone();
        }
        let existing: HashSet<_> = pipeline
            .sources
            .iter()
            .filter_map(|s| s.track.track())
            .collect();
        let items: Vec<_> = app
            .transaction_generators()
            .into_iter()
            .filter(|(path, track)| {
                !existing.contains(track)
                    && (query.is_empty() || path.to_lowercase().contains(query))
            })
            .collect();
        self.available_cache = Some(AvailableCache {
            key: cache_key,
            query: query.to_owned(),
            items: items.clone(),
        });
        items
    }

    fn render_drawer_row(
        &self,
        row: DrawerRowSpec,
        source_count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *theme(cx);
        let i = row.index;
        let accent = t.source_accent(row.color_slot);
        let failed_err = match &row.status {
            SourceLoadStatus::Failed(err) => Some(err.clone()),
            _ => None,
        };
        let status_text = match &row.status {
            SourceLoadStatus::Ready { count } => format!("{count} tx"),
            SourceLoadStatus::Loading => "loading…".into(),
            SourceLoadStatus::Failed(_) => "failed".into(),
            SourceLoadStatus::Unresolved => "missing".into(),
            SourceLoadStatus::Unavailable => "—".into(),
        };
        let alias_tooltip = format!(
            "Row badge (auto: \"{}\"). Type custom alias to override (max 6 chars).",
            row.auto_badge
        );
        let alias_el = self.alias_inputs.get(&row.color_slot).map(|(input, _, _)| {
            div()
                .id(("drawer-alias", row.color_slot))
                .w(t.px(64.0))
                .flex_none()
                .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                .tooltip(move |w, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(alias_tooltip.clone()).build(w, cx)
                })
                .child(Input::new(input).xsmall())
        });

        let mut status_el = div()
            .id(("drawer-status", i))
            .flex_none()
            .text_color(if failed_err.is_some() {
                t.panel.warning
            } else {
                t.panel.text_placeholder
            })
            .child(SharedString::from(status_text));
        if let Some(err) = failed_err {
            status_el = status_el
                .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                .tooltip(move |w, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(err.clone()).build(w, cx)
                });
        }

        div()
            .id(("drawer-source-row", i))
            .flex()
            .items_center()
            .gap(t.px(6.0))
            .px(t.px(6.0))
            .py(t.px(3.0))
            .rounded(t.px(4.0))
            .bg(t.bar.bg)
            .border_1()
            .border_color(t.border_variant)
            .font_family(t.mono_font)
            .text_size(px(t.ui_size_small * 0.92))
            .child(
                div()
                    .size(t.px(8.0))
                    .flex_none()
                    .rounded(t.px(2.0))
                    .bg(accent),
            )
            .when_some(alias_el, gpui_kit::ParentElement::child)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(if row.eff_visible {
                        t.panel.text
                    } else {
                        t.panel.text_muted
                    })
                    .child(SharedString::from(row.path)),
            )
            .child(status_el)
            .child(
                Button::new(("drawer-vis", i))
                    .label(if row.eff_visible { "●" } else { "○" })
                    .xsmall()
                    .ghost()
                    .tooltip(if row.eff_visible {
                        "Hide source"
                    } else {
                        "Show source"
                    })
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.dispatch(PipelineSourceCommand::ToggleVisible(i), window, cx);
                    })),
            )
            .child(
                Button::new(("drawer-up", i))
                    .label("↑")
                    .xsmall()
                    .ghost()
                    .disabled(i == 0)
                    .tooltip("Move up (tie-break priority)")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        if i > 0 {
                            view.dispatch(
                                PipelineSourceCommand::Reorder { from: i, to: i - 1 },
                                window,
                                cx,
                            );
                        }
                    })),
            )
            .child(
                Button::new(("drawer-down", i))
                    .label("↓")
                    .xsmall()
                    .ghost()
                    .disabled(i + 1 >= source_count)
                    .tooltip("Move down")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        if i + 1 < source_count {
                            view.dispatch(
                                PipelineSourceCommand::Reorder { from: i, to: i + 1 },
                                window,
                                cx,
                            );
                        }
                    })),
            )
            .child(
                Button::new(("drawer-rm", i))
                    .label("×")
                    .xsmall()
                    .ghost()
                    .disabled(source_count <= 1)
                    .tooltip("Remove source from panel")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.dispatch(PipelineSourceCommand::Remove(i), window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.sync_alias_inputs(window, cx);
        let Some(owner) = self.ws.upgrade() else {
            return div().into_any_element();
        };
        let t = *theme(cx);
        let query = self.search.read(cx).value().to_lowercase();
        let available = self.available_generators(&owner, &query, cx);
        let (rows_data, total_rows, disjoint_notice, any_failed) = {
            let app = &owner.read(cx).app;
            let Some(pipeline) = app.panels.pipeline(self.panel) else {
                return div().into_any_element();
            };
            let badges = pipeline.badges();
            let mut any_failed = false;
            let rows_data: Vec<DrawerRowSpec> = pipeline
                .sources
                .iter()
                .enumerate()
                .map(|(index, s)| {
                    let status = pipeline.source_status(&app.doc, index);
                    if matches!(status, SourceLoadStatus::Failed(_)) {
                        any_failed = true;
                    }
                    let auto_badge = badges
                        .labels
                        .get(index)
                        .map_or_else(String::new, |l| l.auto_badge.clone());
                    DrawerRowSpec {
                        index,
                        color_slot: s.color_slot,
                        path: s.track.path().join("."),
                        auto_badge,
                        eff_visible: pipeline.is_source_effectively_visible(index),
                        status,
                    }
                })
                .collect();
            let total_rows = pipeline.row_count(&app.doc);
            let notice = pipeline.disjoint_notice().map(str::to_owned);
            (rows_data, total_rows, notice, any_failed)
        };
        let source_count = rows_data.len();

        let mut current_list = div()
            .id(("sources-current-list", self.panel.0))
            .flex()
            .flex_col()
            .gap(t.px(4.0))
            .max_h(t.px(190.0))
            .overflow_y_scroll();
        for row in rows_data {
            current_list = current_list.child(self.render_drawer_row(row, source_count, cx));
        }

        let total_available = available.len();
        let mut add_list = div()
            .id(("sources-available-list", self.panel.0))
            .flex()
            .flex_col()
            .gap(t.px(3.0))
            .max_h(t.px(150.0))
            .overflow_y_scroll();

        if available.is_empty() {
            add_list = add_list.child(
                div()
                    .px(t.px(6.0))
                    .py(t.px(4.0))
                    .text_size(px(t.ui_size_small))
                    .text_color(t.panel.text_placeholder)
                    .child(if query.is_empty() {
                        "All transaction generators are already in this panel."
                    } else {
                        "No matching transaction generators."
                    }),
            );
        } else {
            for (ix, (path, track)) in available.into_iter().take(MAX_AVAILABLE_ITEMS).enumerate() {
                add_list = add_list.child(
                    Button::new(("drawer-add-gen", ix))
                        .label(format!("+ {path}"))
                        .justify_start()
                        .xsmall()
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .color(t.bar.bg)
                                .foreground(t.panel.text)
                                .hover(t.hover.bg)
                                .active(t.selection.bg),
                        )
                        .font_family(t.mono_font)
                        .w_full()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.dispatch(PipelineSourceCommand::Add(track), window, cx);
                        })),
                );
            }
            if total_available > MAX_AVAILABLE_ITEMS {
                add_list = add_list.child(
                    div()
                        .px(t.px(6.0))
                        .py(t.px(2.0))
                        .text_size(px(t.ui_size_small * 0.9))
                        .text_color(t.panel.text_placeholder)
                        .child(SharedString::from(format!(
                            "Showing {MAX_AVAILABLE_ITEMS} of {total_available} — refine filter to see more"
                        ))),
                );
            }
        }

        let row_summary = if source_count > 1 {
            format!("{total_rows} merged rows")
        } else {
            format!("{total_rows} rows")
        };

        div()
            .id(("pipeline-sources-drawer", self.panel.0))
            .debug_selector(|| "pipeline-sources-drawer".into())
            .flex_none()
            .w_full()
            .bg(t.elevated.bg)
            .border_b_1()
            .border_color(t.border)
            .px(t.px(10.0))
            .py(t.px(8.0))
            .flex()
            .flex_col()
            .gap(t.px(8.0))
            .when_some(disjoint_notice, |el, notice| {
                el.child(
                    div()
                        .debug_selector(|| "pipeline-disjoint-notice".into())
                        .px(t.px(8.0))
                        .py(t.px(6.0))
                        .rounded(t.px(4.0))
                        .border_1()
                        .border_color(t.panel.warning.opacity(0.45))
                        .bg(t.panel.warning.opacity(0.12))
                        .text_size(px(t.ui_size_small))
                        .text_color(t.panel.warning)
                        .child(SharedString::from(notice)),
                )
            })
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .gap(t.px(12.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(t.px(260.0))
                            .flex()
                            .flex_col()
                            .gap(t.px(5.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(t.px(6.0))
                                    .font_family(t.mono_font)
                                    .text_size(px(t.ui_size_small * 0.9))
                                    .text_color(t.panel.text_muted)
                                    .child("PANEL SOURCES (ALIAS · PATH)")
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(t.px(6.0))
                                            .when(any_failed, |el| {
                                                el.child(
                                                    Button::new("drawer-retry-failed")
                                                        .label("Retry failed")
                                                        .xsmall()
                                                        .ghost()
                                                        .on_click(cx.listener(
                                                            |view, _, window, cx| {
                                                                view.dispatch(
                                                                    PipelineSourceCommand::RetryFailed,
                                                                    window,
                                                                    cx,
                                                                );
                                                            },
                                                        )),
                                                )
                                            })
                                            .child(SharedString::from(row_summary)),
                                    ),
                            )
                            .child(current_list),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(t.px(220.0))
                            .flex()
                            .flex_col()
                            .gap(t.px(5.0))
                            .child(
                                div()
                                    .font_family(t.mono_font)
                                    .text_size(px(t.ui_size_small * 0.9))
                                    .text_color(t.panel.text_muted)
                                    .child("ADD GENERATOR TO PANEL"),
                            )
                            .child(Input::new(&self.search).xsmall())
                            .child(add_list),
                    ),
            )
            .into_any_element()
    }
}

impl Render for PipelineSourcesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.ws.upgrade() else {
            return div().into_any_element();
        };
        let is_merged = owner
            .read(cx)
            .app
            .panels
            .pipeline(self.panel)
            .is_some_and(volna_core::pipeline::PipelineModel::is_merged);
        if !is_merged && !self.drawer_open {
            return div().into_any_element();
        }
        div()
            .flex_none()
            .w_full()
            .flex()
            .flex_col()
            .when(is_merged, |el| el.child(self.render_chip_strip(cx)))
            .when(self.drawer_open, |el| {
                el.child(self.render_drawer(window, cx))
            })
            .into_any_element()
    }
}
