//! Recorded-input computation controls with debounced previews on the shared waveform timeline.

mod inputs;
mod reference;
mod search;
#[cfg(all(test, not(target_family = "wasm")))]
mod tests;

use crate::{
    app::Workspace,
    theme::{ThemePx, theme},
    ui::{Splitter, SplitterAxis},
};
use gpui_kit::base::Disableable;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, ClipboardItem, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, Subscription,
    WeakEntity, Window, canvas, div, px,
};
use volna_core::{
    Command,
    computed::{Command as DerivedCommand, ComputationStatus, Draft},
    panels::PanelId,
};

use inputs::{BindingFields, InputPicker};
use search::InputSearch;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SnapshotSelection {
    Saved,
    Edited,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GuideVisibility {
    Open,
    Closed,
}

#[derive(Clone, Copy)]
struct WidthDrag {
    start: Pixels,
    initial: f32,
}

pub(crate) struct DerivedEditor {
    pub id: u64,
    owner: WeakEntity<Workspace>,
    panel: PanelId,
    generation: u64,
    name: Entity<InputState>,
    window: Entity<InputState>,
    interval: Entity<InputState>,
    expression: Entity<InputState>,
    reference_search: Entity<InputState>,
    scope_start: Entity<InputState>,
    scope_end: Entity<InputState>,
    formula: bool,
    search: Entity<InputState>,
    picker: Option<InputPicker>,
    input_search: InputSearch,
    bindings: Vec<BindingFields>,
    partial: bool,
    skip_unknown: bool,
    scope: volna_core::computed::MeasurementScope,
    selected_markers: Vec<volna_core::marker::MarkerId>,
    snapshot_selection: SnapshotSelection,
    guide: GuideVisibility,
    resize_focus: FocusHandle,
    width_drag: Option<WidthDrag>,
    preview_task: Option<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl DerivedEditor {
    pub fn new(
        owner: WeakEntity<Workspace>,
        panel: PanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let entity = owner.upgrade().expect("editor has workspace");
        let ws = entity.read(cx);
        let editor = ws.app.derived_editor.as_ref().expect("open editor");
        let (id, draft, generation, domain, markers) = (
            editor.id,
            editor.draft.clone(),
            ws.app.doc.generation(),
            editor.domain,
            ws.app.doc.markers().to_vec(),
        );
        let name = cx.new(|cx| InputState::new(window, cx).default_value(draft.name));
        let trailing = cx.new(|cx| InputState::new(window, cx).default_value(draft.window));
        let interval = cx.new(|cx| InputState::new(window, cx).default_value(draft.interval));
        let expression = cx.new(|cx| {
            InputState::new(window, cx).default_value(draft.formula.clone().unwrap_or_default())
        });
        let reference_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a function or example…"));
        let explicit_bounds = match &draft.scope {
            volna_core::computed::MeasurementScope::Explicit { start, end } => (*start, *end),
            _ => domain,
        };
        let scope_start =
            cx.new(|cx| InputState::new(window, cx).default_value(explicit_bounds.0.to_string()));
        let scope_end =
            cx.new(|cx| InputState::new(window, cx).default_value(explicit_bounds.1.to_string()));
        let selected_markers = match &draft.scope {
            volna_core::computed::MeasurementScope::LinkedMarkers { start, end } => {
                vec![*start, *end]
            }
            volna_core::computed::MeasurementScope::MarkerSet { markers } => markers.clone(),
            volna_core::computed::MeasurementScope::MarkerSnapshot { start, end } => markers
                .iter()
                .filter(|marker| marker.time == *start || marker.time == *end)
                .take(2)
                .map(|marker| marker.id)
                .collect(),
            _ => Vec::new(),
        };
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search recorded signals…"));
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe(&reference_search, |_, _, _: &InputEvent, cx| cx.notify()));
        for input in [
            &name,
            &trailing,
            &interval,
            &expression,
            &scope_start,
            &scope_end,
        ] {
            subscriptions.push(
                cx.subscribe_in(input, window, |view, _, event, window, cx| {
                    if matches!(event, InputEvent::Change) && view.update_draft(window, cx) {
                        view.schedule_preview(window, cx);
                    }
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        view.preview_task = None;
                        view.send(DerivedCommand::Preview, window, cx);
                    }
                }),
            );
        }
        subscriptions.push(cx.subscribe(&search, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.sync_search(cx);
            }
        }));
        cx.observe(&entity, |view, _, cx| {
            view.sync_search(cx);
            cx.notify();
        })
        .detach();
        Self {
            id,
            owner,
            panel,
            generation,
            name,
            window: trailing,
            interval,
            expression,
            reference_search,
            scope_start,
            scope_end,
            formula: draft.formula.is_some(),
            search,
            picker: None,
            input_search: InputSearch::default(),
            bindings: Vec::new(),
            partial: draft.partial,
            skip_unknown: draft.skip_unknown,
            scope: draft.scope,
            selected_markers,
            snapshot_selection: SnapshotSelection::Saved,
            guide: GuideVisibility::Open,
            resize_focus: cx.focus_handle().tab_stop(true),
            width_drag: None,
            preview_task: None,
            _subscriptions: subscriptions,
        }
    }

    fn send(&mut self, command: DerivedCommand, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            command,
            DerivedCommand::AddInput(_)
                | DerivedCommand::RemoveInput(_)
                | DerivedCommand::Rebind(..)
                | DerivedCommand::Close
        ) {
            self.close_picker();
        }
        drop(self.owner.update(cx, |ws, cx| {
            if ws
                .app
                .derived_editor
                .as_ref()
                .is_none_or(|e| e.id != self.id)
            {
                return;
            }
            ws.dispatch_if_current(
                self.generation,
                Command::Derived(self.panel, command),
                Some(window),
                cx,
            );
            if ws.app.doc.generation() == self.generation
                && ws.app.derived_editor.is_none()
                && let Some(focus) = ws.panel_focus(self.panel, cx)
            {
                window.focus(&focus, cx);
            }
        }));
    }

    fn schedule_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(300));
        self.preview_task = Some(cx.spawn_in(window, async move |this, cx| {
            timer.await;
            this.update_in(cx, |view, window, cx| {
                view.preview_task = None;
                view.send(DerivedCommand::Preview, window, cx);
            })
            .ok();
        }));
    }

    fn update_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(owner) = self.owner.upgrade() else {
            return false;
        };
        let revision = owner
            .read(cx)
            .app
            .derived_editor
            .as_ref()
            .map(|editor| editor.calculation_revision);
        let scope = if self.formula {
            let Ok(scope) = self.resolved_scope(cx) else {
                cx.notify();
                return false;
            };
            scope
        } else {
            self.scope.clone()
        };
        self.send(
            DerivedCommand::Update(Draft {
                name: self.name.read(cx).value().to_string(),
                window: self.window.read(cx).value().to_string(),
                interval: self.interval.read(cx).value().to_string(),
                partial: self.partial,
                skip_unknown: self.skip_unknown,
                formula: self
                    .formula
                    .then(|| self.expression.read(cx).value().to_string()),
                scope,
            }),
            window,
            cx,
        );
        owner
            .read(cx)
            .app
            .derived_editor
            .as_ref()
            .is_some_and(|editor| Some(editor.calculation_revision) != revision)
    }

    fn resolved_scope(
        &self,
        cx: &Context<Self>,
    ) -> Result<volna_core::computed::MeasurementScope, String> {
        use volna_core::computed::MeasurementScope;
        match &self.scope {
            MeasurementScope::IntervalSnapshot { .. } => Ok(self.scope.clone()),
            MeasurementScope::MarkerIntervals => Ok(MeasurementScope::MarkerIntervals),
            MeasurementScope::WholeRange => Ok(MeasurementScope::WholeRange),
            MeasurementScope::Explicit { .. } => {
                let read = |field: &Entity<InputState>| {
                    field.read(cx).value().trim().parse::<u64>().map_err(|_| {
                        "Enter each explicit bound as a whole timeline tick.".to_owned()
                    })
                };
                Ok(MeasurementScope::Explicit {
                    start: read(&self.scope_start)?,
                    end: read(&self.scope_end)?,
                })
            }
            MeasurementScope::MarkerSnapshot { .. }
                if self.snapshot_selection == SnapshotSelection::Saved =>
            {
                Ok(self.scope.clone())
            }
            MeasurementScope::LinkedMarkers { .. } | MeasurementScope::MarkerSnapshot { .. } => {
                if self.selected_markers.len() != 2 {
                    return Err("Select two markers for this scope.".into());
                }
                let markers = self
                    .owner
                    .upgrade()
                    .ok_or("Workspace is unavailable.")?
                    .read(cx)
                    .app
                    .doc
                    .markers()
                    .to_vec();
                let positions = self
                    .selected_markers
                    .iter()
                    .map(|id| {
                        markers
                            .iter()
                            .find(|m| m.id == *id)
                            .map(|m| m.time)
                            .ok_or_else(|| format!("Marker {id} is unavailable."))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(
                    if matches!(self.scope, MeasurementScope::LinkedMarkers { .. }) {
                        MeasurementScope::LinkedMarkers {
                            start: self.selected_markers[0],
                            end: self.selected_markers[1],
                        }
                    } else {
                        MeasurementScope::MarkerSnapshot {
                            start: positions[0].min(positions[1]),
                            end: positions[0].max(positions[1]),
                        }
                    },
                )
            }
            MeasurementScope::MarkerSet { .. } => {
                if self.selected_markers.len() < 2 {
                    return Err("Select at least two markers for a table.".into());
                }
                Ok(MeasurementScope::MarkerSet {
                    markers: self.selected_markers.clone(),
                })
            }
        }
    }

    fn select_marker(
        &mut self,
        id: volna_core::marker::MarkerId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(owner) = self.owner.upgrade() {
            let available = owner
                .read(cx)
                .app
                .doc
                .markers()
                .iter()
                .map(|marker| marker.id)
                .collect::<std::collections::HashSet<_>>();
            self.selected_markers.retain(|id| available.contains(id));
        }
        if let Some(index) = self
            .selected_markers
            .iter()
            .position(|selected| *selected == id)
        {
            self.selected_markers.remove(index);
        } else {
            if !matches!(
                self.scope,
                volna_core::computed::MeasurementScope::MarkerSet { .. }
            ) && self.selected_markers.len() == 2
            {
                self.selected_markers.remove(0);
            }
            self.selected_markers.push(id);
        }
        self.snapshot_selection = SnapshotSelection::Edited;
        self.preview_task = None;
        self.update_draft(window, cx);
        if self.resolved_scope(cx).is_ok() {
            self.send(DerivedCommand::Preview, window, cx);
        }
        cx.notify();
    }

    fn cycle_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use volna_core::computed::MeasurementScope;
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let ws = owner.read(cx);
        let Some(editor) = ws.app.derived_editor.as_ref() else {
            return;
        };
        let markers = ws.app.doc.markers();
        self.scope = match &self.scope {
            MeasurementScope::MarkerIntervals if markers.len() >= 2 => {
                self.selected_markers = markers[..2].iter().map(|marker| marker.id).collect();
                MeasurementScope::LinkedMarkers {
                    start: markers[0].id,
                    end: markers[1].id,
                }
            }
            MeasurementScope::WholeRange | MeasurementScope::IntervalSnapshot { .. } => {
                MeasurementScope::MarkerIntervals
            }
            MeasurementScope::MarkerIntervals | MeasurementScope::MarkerSet { .. } => {
                MeasurementScope::Explicit {
                    start: editor.domain.0,
                    end: editor.domain.1,
                }
            }
            MeasurementScope::LinkedMarkers { .. } => {
                let selected = self
                    .selected_markers
                    .iter()
                    .filter_map(|id| markers.iter().find(|marker| marker.id == *id))
                    .collect::<Vec<_>>();
                if selected.len() == 2 {
                    MeasurementScope::MarkerSnapshot {
                        start: selected[0].time.min(selected[1].time),
                        end: selected[0].time.max(selected[1].time),
                    }
                } else {
                    MeasurementScope::Explicit {
                        start: editor.domain.0,
                        end: editor.domain.1,
                    }
                }
            }
            MeasurementScope::MarkerSnapshot { .. } => MeasurementScope::MarkerSet {
                markers: self.selected_markers.clone(),
            },
            MeasurementScope::Explicit { .. } => MeasurementScope::WholeRange,
        };
        self.snapshot_selection = SnapshotSelection::Saved;
        self.preview_task = None;
        self.update_draft(window, cx);
        if self.resolved_scope(cx).is_ok() {
            self.send(DerivedCommand::Preview, window, cx);
        }
        cx.notify();
    }

    fn width_limits(window: &Window, cx: &Context<Self>) -> (f32, f32) {
        let available = f32::from(window.viewport_size().width) / theme(cx).zoom;
        if available < 500.0 {
            return (available, available);
        }
        let min = 320.0;
        let max = (available - 180.0).max(min).min(1000.0);
        (min, max)
    }

    fn set_width(&mut self, width: f32, window: &mut Window, cx: &mut Context<Self>) {
        let (min, max) = Self::width_limits(window, cx);
        let width = width.clamp(min, max);
        drop(self.owner.update(cx, |ws, cx| {
            ws.derived_width = width;
            cx.notify();
        }));
        cx.notify();
    }

    fn start_resize(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.resize_focus, cx);
        if event.click_count == 2 {
            self.width_drag = None;
            self.set_width(440.0, window, cx);
        } else {
            let initial = self
                .owner
                .upgrade()
                .map_or(440.0, |owner| owner.read(cx).derived_width);
            self.width_drag = Some(WidthDrag {
                start: event.position.x,
                initial,
            });
            cx.notify();
        }
    }

    fn resize_to(&mut self, x: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.width_drag else { return };
        self.set_width(
            drag.initial + f32::from(drag.start - x) / theme(cx).zoom,
            window,
            cx,
        );
    }

    fn resize_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (min, max) = Self::width_limits(window, cx);
        let current = self
            .owner
            .upgrade()
            .map_or(440.0, |owner| owner.read(cx).derived_width);
        let step = if event.keystroke.modifiers.shift {
            64.0
        } else {
            16.0
        };
        let next = match event.keystroke.key.as_str() {
            "left" => current + step,
            "right" => current - step,
            "home" => min,
            "end" => max,
            "enter" => 440.0,
            "escape" => {
                if let Some(drag) = self.width_drag.take() {
                    self.set_width(drag.initial, window, cx);
                }
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        self.set_width(next, window, cx);
        cx.stop_propagation();
    }

    fn resize_tracker(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.width_drag?;
        let owner = cx.weak_entity();
        Some(
            canvas(
                |_, _, _| (),
                move |_, (), window, _| {
                    window.on_mouse_event({
                        let owner = owner.clone();
                        move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase.bubble() {
                                drop(owner.update(cx, |view, cx| {
                                    if event.pressed_button == Some(MouseButton::Left) {
                                        view.resize_to(event.position.x, window, cx);
                                    } else if view.width_drag.take().is_some() {
                                        cx.notify();
                                    }
                                }));
                            }
                        }
                    });
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase.bubble() && event.button == MouseButton::Left {
                            drop(owner.update(cx, |view, cx| {
                                view.resize_to(event.position.x, window, cx);
                                view.width_drag = None;
                                cx.notify();
                            }));
                        }
                    });
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }

    fn use_example(&mut self, formula: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.expression
            .update(cx, |input, cx| input.set_value(formula, window, cx));
        self.expression.read(cx).focus_handle(cx).focus(window, cx);
        self.preview_task = None;
        self.update_draft(window, cx);
        self.send(DerivedCommand::Preview, window, cx);
    }

    fn operation(&mut self, formula: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.formula = formula;
        if formula && self.expression.read(cx).value().is_empty() {
            let alias = self
                .owner
                .upgrade()
                .and_then(|owner| {
                    owner
                        .read(cx)
                        .app
                        .derived_editor
                        .as_ref()?
                        .bindings
                        .first()
                        .map(|i| i.alias.clone())
                })
                .unwrap_or_default();
            self.expression
                .update(cx, |input, cx| input.set_value(alias, window, cx));
        }
        self.preview_task = None;
        self.update_draft(window, cx);
        self.send(DerivedCommand::Preview, window, cx);
    }
}

impl Render for DerivedEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_bindings(window, cx);
        let t = *theme(cx);
        let Some(owner) = self.owner.upgrade() else {
            return div().into_any_element();
        };
        let ws = owner.read(cx);
        let Some(editor) = ws.app.derived_editor.as_ref().filter(|e| e.id == self.id) else {
            return div().into_any_element();
        };
        let inputs = editor.bindings.clone();
        let bounds = format!(
            "{} — {}",
            volna_core::wave::timeline::format_time(editor.domain.0 as f64, ws.app.doc.time_base()),
            volna_core::wave::timeline::format_time(editor.domain.1 as f64, ws.app.doc.time_base())
        );
        let mut ready = false;
        let mut building = false;
        let mut failed = false;
        let status = match ws.app.derived_preview() {
            Some(ComputationStatus::Ready(history)) => {
                ready = editor.error.is_none();
                let warnings = history.warnings();
                format!(
                    "{} output {} · live preview{}",
                    history.len(),
                    if self.formula { "events" } else { "samples" },
                    if warnings.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", warnings.join(" · "))
                    }
                )
            }
            Some(ComputationStatus::ReadyMeasurement(measurement)) => {
                ready = editor.error.is_none();
                format!(
                    "{} · {:.1}% known · live preview",
                    measurement
                        .value
                        .map_or_else(|| "unavailable".into(), |value| value.to_string()),
                    measurement.coverage() * 100.0
                )
            }
            Some(ComputationStatus::ReadyTable(rows)) => {
                ready = editor.error.is_none();
                format!("{} intervals · live preview", rows.len())
            }
            Some(ComputationStatus::Waiting) => {
                building = true;
                "Loading input…".into()
            }
            Some(ComputationStatus::Building) => {
                building = true;
                "Updating waveform…".into()
            }
            Some(ComputationStatus::Failed(_)) => {
                failed = true;
                "Preview failed".into()
            }
            None if editor.stopped => "Preview stopped.".into(),
            None if editor.error.is_some() => "Fix the settings to update the waveform.".into(),
            None => "Updating waveform…".into(),
        };
        let stopped = editor.stopped;
        let error = editor.error.clone();
        let editing = editor.editing;
        let title = if editing {
            "Edit derived signal"
        } else {
            "Derive signal"
        };
        let field = |label: &'static str, input: &Entity<InputState>, selector: &'static str| {
            div()
                .id(selector)
                .debug_selector(move || selector.to_owned())
                .flex()
                .flex_col()
                .gap_1()
                .child(label)
                .child(Input::new(input).small().w_full())
        };
        let scope_error = self
            .formula
            .then(|| self.resolved_scope(cx).err())
            .flatten();
        ready &= scope_error.is_none();
        let marker_scope = matches!(
            self.scope,
            volna_core::computed::MeasurementScope::LinkedMarkers { .. }
                | volna_core::computed::MeasurementScope::MarkerSnapshot { .. }
                | volna_core::computed::MeasurementScope::MarkerSet { .. }
        );
        let mut marker_controls = div().flex().flex_wrap().gap_1();
        for marker in ws.app.doc.markers() {
            let id = marker.id;
            let label = marker.label.as_deref().map_or_else(
                || format!("M{id} · {}", marker.time),
                |label| format!("M{id} {label} · {}", marker.time),
            );
            marker_controls = marker_controls.child(
                Button::new(format!("scope-marker-{id}"))
                    .label(label)
                    .small()
                    .selected(self.selected_markers.contains(&id))
                    .debug_selector(move || format!("scope-marker-{id}"))
                    .on_click(
                        cx.listener(move |view, _, window, cx| view.select_marker(id, window, cx)),
                    ),
            );
        }
        let scope_details = div()
            .flex()
            .flex_col()
            .gap_1()
            .when(
                matches!(
                    self.scope,
                    volna_core::computed::MeasurementScope::Explicit { .. }
                ),
                |el| {
                    el.child(field(
                        "Start tick",
                        &self.scope_start,
                        "formula-scope-start",
                    ))
                    .child(field(
                        "End tick",
                        &self.scope_end,
                        "formula-scope-end",
                    ))
                },
            )
            .when(marker_scope, |el| {
                el.child(
                    if matches!(
                        self.scope,
                        volna_core::computed::MeasurementScope::MarkerSet { .. }
                    ) {
                        "Select at least two markers for the table."
                    } else {
                        "Select two markers for the interval."
                    },
                )
                .child(marker_controls)
            });
        let close = Button::new("derived-close")
            .label("Cancel")
            .ghost()
            .xsmall()
            .on_click(
                cx.listener(|view, _, window, cx| view.send(DerivedCommand::Close, window, cx)),
            );
        let warmup = Button::new("average-warmup")
            .label(if self.partial {
                "Warm-up: available history"
            } else {
                "Warm-up: wait for full window"
            })
            .small()
            .selected(self.partial)
            .on_click(cx.listener(|view, _, window, cx| {
                view.partial = !view.partial;
                view.preview_task = None;
                view.update_draft(window, cx);
                view.send(DerivedCommand::Preview, window, cx);
            }));
        let missing = Button::new("average-unknown")
            .label(if self.skip_unknown {
                "Missing values: skip unknown time"
            } else {
                "Missing values: propagate gaps"
            })
            .small()
            .selected(self.skip_unknown)
            .on_click(cx.listener(|view, _, window, cx| {
                view.skip_unknown = !view.skip_unknown;
                view.preview_task = None;
                view.update_draft(window, cx);
                view.send(DerivedCommand::Preview, window, cx);
            }));
        let actions =
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .when(failed || stopped, |el| {
                    el.child(
                        Button::new("derived-retry")
                            .label(if stopped { "Resume" } else { "Retry" })
                            .small()
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.preview_task = None;
                                view.send(DerivedCommand::Preview, window, cx);
                            })),
                    )
                })
                .when(building, |el| {
                    el.child(Button::new("derived-stop").label("Stop").small().on_click(
                        cx.listener(|view, _, window, cx| {
                            view.preview_task = None;
                            view.send(DerivedCommand::Stop, window, cx);
                        }),
                    ))
                })
                .child(
                    Button::new("derived-apply")
                        .label(if editing {
                            "Apply changes"
                        } else {
                            "Add to waveform"
                        })
                        .primary()
                        .small()
                        .disabled(!ready)
                        .debug_selector(|| "derived-apply".into())
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.send(DerivedCommand::Apply, window, cx);
                        })),
                )
                .child(
                    Button::new("derived-table")
                        .debug_selector(|| "derived-table".into())
                        .label("Add to table")
                        .small()
                        .disabled(!ready)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.send(DerivedCommand::ApplyTable, window, cx);
                        })),
                );
        let description = if self.formula {
            "Held inputs are evaluated at their combined transition times, within the common recording domain. Zoom does not change the result."
        } else {
            "Time-weighted mean over [t − window, t). Output values are held until the next output sample. Zoom does not change the result."
        };
        let diagnostic = scope_error
            .or(error)
            .or_else(|| match ws.app.derived_preview() {
                Some(ComputationStatus::Failed(error)) => Some(error.clone()),
                _ => None,
            });
        let (min_width, max_width) = Self::width_limits(window, cx);
        let width = ws.derived_width.clamp(min_width, max_width);
        let wide = width >= max_width - 1.0;
        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .p(t.px(12.0))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .min_w_0()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new("derived-width")
                            .debug_selector(|| "derived-width".into())
                            .label(if wide { "Reset width" } else { "Widen" })
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.set_width(if wide { 440.0 } else { max_width }, window, cx);
                            })),
                    )
                    .child(close),
            );
        let composer = div()
            .id("formula-composer")
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .p(t.px(12.0))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child("Formula"),
            )
            .child(
                div()
                    .id("formula-expression")
                    .debug_selector(|| "formula-expression".into())
                    .font_family(t.mono_font)
                    .child(Input::new(&self.expression).w_full()),
            )
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .whitespace_normal()
                    .child("Use your input aliases below. Changes preview automatically."),
            );
        let form = div()
            .id("derived-form-scroll")
            .debug_selector(|| "derived-form-scroll".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scrollbar()
            .p(t.px(12.0))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div().flex().flex_col().gap_1().child("Operation").child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(
                            Button::new("operation-average")
                                .label("Moving average")
                                .small()
                                .selected(!self.formula)
                                .disabled(inputs.len() != 1)
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.operation(false, window, cx);
                                })),
                        )
                        .child(
                            Button::new("operation-formula")
                                .label("Formula")
                                .small()
                                .selected(self.formula)
                                .debug_selector(|| "operation-formula".into())
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.operation(true, window, cx);
                                })),
                        ),
                ),
            )
            .child(field("Name", &self.name, "derived-name"))
            .child(self.input_controls(&inputs, window, cx))
            .when(self.formula, |el| {
                el.child(field(
                    "Default sampling step",
                    &self.interval,
                    "formula-default-step",
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child("Measurement scope")
                        .child(
                            Button::new("formula-scope")
                                .debug_selector(|| "formula-scope".into())
                                .label(match &self.scope {
                                    volna_core::computed::MeasurementScope::IntervalSnapshot {
                                        ..
                                    } => "Saved intervals",
                                    volna_core::computed::MeasurementScope::MarkerIntervals => {
                                        "Each marker interval"
                                    }
                                    volna_core::computed::MeasurementScope::WholeRange => {
                                        "Whole range"
                                    }
                                    volna_core::computed::MeasurementScope::Explicit { .. } => {
                                        "Explicit bounds"
                                    }
                                    volna_core::computed::MeasurementScope::MarkerSnapshot {
                                        ..
                                    } => "Marker snapshot",
                                    volna_core::computed::MeasurementScope::LinkedMarkers {
                                        ..
                                    } => "Linked markers",
                                    volna_core::computed::MeasurementScope::MarkerSet {
                                        ..
                                    } => "Marker set table",
                                })
                                .small()
                                .on_click(
                                    cx.listener(|view, _, window, cx| view.cycle_scope(window, cx)),
                                ),
                        ),
                )
                .child(scope_details)
                .child(
                    Button::new("formula-unknown")
                        .label(if self.skip_unknown {
                            "Missing values: skip unknown"
                        } else {
                            "Missing values: propagate gaps"
                        })
                        .small()
                        .selected(self.skip_unknown)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.skip_unknown = !view.skip_unknown;
                            view.preview_task = None;
                            view.update_draft(window, cx);
                            view.send(DerivedCommand::Preview, window, cx);
                        })),
                )
            })
            .when(!self.formula, |el| {
                el.child(field("Trailing window", &self.window, "average-window"))
                    .child(field("Output interval", &self.interval, "average-interval"))
                    .child(
                        div()
                            .text_color(t.panel.text_muted)
                            .child("Use time units (4 us, 250 ns) or timeline ticks."),
                    )
                    .child(warmup)
                    .child(missing)
            })
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .child(format!("Output domain · {bounds}")),
            )
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .whitespace_normal()
                    .child(description),
            );
        let has_diagnostic = diagnostic.is_some();
        let error_card = diagnostic.map(|message| {
            let for_copy = message.clone();
            let for_details = message.clone();
            div()
                .id("derived-error")
                .debug_selector(|| "derived-error".into())
                .flex()
                .flex_col()
                .gap_1()
                .p(t.px(8.0))
                .border_1()
                .border_color(t.panel.error)
                .child(
                    div()
                        .max_h(t.px(90.0))
                        .overflow_y_scrollbar()
                        .whitespace_normal()
                        .text_color(t.panel.error)
                        .child(gpui_kit::base::SelectableText::new(
                            "derived-error-text",
                            message,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            Button::new("derived-copy-error")
                                .debug_selector(|| "derived-copy-error".into())
                                .label("Copy error")
                                .xsmall()
                                .on_click(move |_, _, cx| {
                                    #[cfg(target_family = "wasm")]
                                    {
                                        crate::clipboard::copy(&for_copy);
                                        let _ = cx;
                                    }
                                    #[cfg(not(target_family = "wasm"))]
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        for_copy.clone(),
                                    ));
                                }),
                        )
                        .child(
                            Button::new("derived-error-details")
                                .label("Details…")
                                .xsmall()
                                .on_click(move |_, window, cx| {
                                    crate::ui::message_dialog::open_message_dialog(
                                        "Formula error",
                                        for_details.clone(),
                                        window,
                                        cx,
                                    );
                                }),
                        ),
                )
        });
        let footer = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .p(t.px(12.0))
            .border_t_1()
            .border_color(t.border)
            .bg(t.panel.bg)
            .child(
                div()
                    .id("derived-status")
                    .debug_selector(|| "derived-status".into())
                    .text_color(if failed {
                        t.panel.error
                    } else {
                        t.panel.text_muted
                    })
                    .child(status),
            )
            .when_some(error_card, ParentElement::child)
            .child(actions)
            .when(editing, |el| {
                el.child(
                    Button::new("derived-remove")
                        .label("Remove definition")
                        .ghost()
                        .small()
                        .debug_selector(|| "derived-remove".into())
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.send(DerivedCommand::Remove, window, cx);
                        })),
                )
            });
        let guide_height = (window.viewport_size().height
            * if has_diagnostic { 0.12 } else { 0.22 })
        .min(t.px(260.0))
        .max(t.px(80.0));
        let guide = div()
            .flex_none()
            .border_t_1()
            .border_color(t.border)
            .child(
                Button::new("formula-guide-toggle")
                    .debug_selector(|| "formula-guide-toggle".into())
                    .label(if self.guide == GuideVisibility::Open {
                        "Formula guide · Hide"
                    } else {
                        "Formula guide · Show"
                    })
                    .ghost()
                    .small()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.guide = if view.guide == GuideVisibility::Open {
                            GuideVisibility::Closed
                        } else {
                            GuideVisibility::Open
                        };
                        cx.notify();
                    })),
            )
            .when(self.guide == GuideVisibility::Open, |el| {
                el.child(div().px(t.px(12.0)).pb(t.px(8.0)).flex().flex_col().gap_2()
                    .child(div().id("formula-guide-search").debug_selector(|| "formula-guide-search".into())
                        .child(Input::new(&self.reference_search).small().w_full()))
                    .child(div().text_color(t.panel.text_muted).whitespace_normal()
                        .child("Use example replaces the formula. Replace a, b and clock aliases with your inputs.")))
                    .child(reference::render(&t, guide_height, &cx.entity(), &self.reference_search.read(cx).value()))
            });
        let splitter = div()
            .id("derived-resize-wrapper")
            .debug_selector(|| "derived-resize".into())
            .track_focus(&self.resize_focus)
            .tab_stop(true)
            .role(gpui_kit::accesskit::Role::Splitter)
            .aria_label("Resize formula panel")
            .aria_description(
                "Left and Right resize. Shift resizes faster. Enter or double-click resets.",
            )
            .aria_orientation(gpui_kit::accesskit::Orientation::Vertical)
            .aria_numeric_value(f64::from(width))
            .aria_min_numeric_value(f64::from(min_width))
            .aria_max_numeric_value(f64::from(max_width))
            .aria_numeric_value_step(16.0)
            .focus_visible(|s| s.bg(t.border_focused))
            .on_key_down(cx.listener(Self::resize_key))
            .child(
                Splitter::new("derived-resize", SplitterAxis::Vertical)
                    .dragging(self.width_drag.is_some() || self.resize_focus.is_focused(window))
                    .on_drag_start(cx.listener(Self::start_resize)),
            );
        let resize_tracker = self.resize_tracker(cx);
        div()
            .id("derived-signal-editor")
            .debug_selector(|| "derived-signal-editor".into())
            .relative()
            .h_full()
            .w(t.px(width))
            .max_w_full()
            .flex_shrink_0()
            .flex()
            .text_color(t.panel.text)
            .text_size(px(t.ui_size_small))
            .child(splitter)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .bg(t.panel.bg)
                    .child(header)
                    .when(self.formula, |el| el.child(composer))
                    .child(form)
                    .when(self.formula, |el| el.child(guide))
                    .child(footer),
            )
            .when_some(resize_tracker, ParentElement::child)
            .into_any_element()
    }
}
