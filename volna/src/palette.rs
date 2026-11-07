//! The ⌘K command palette: every registered action with its key hint, and the settings the
//! core matcher ranks for the query.

use gpui_kit::component::{
    WindowExt,
    command::{Command as Palette, CommandGroup, CommandItem, CommandState},
    kbd::Kbd,
};
use gpui_kit::prelude::*;
use gpui_kit::{Action, Context, Focusable, SharedString, WeakEntity, Window, div, px};
use volna_core::app::SettingsCommand;
use volna_core::marker::{LaneVerb, NavigatorRow};
use volna_core::settings::{self, Kind, Value};
use volna_core::{App as CoreApp, Command};

use crate::app::Workspace;
use crate::theme::ThemePx;

pub(crate) struct PaletteModel {
    commands: Vec<(String, Box<dyn Action>)>,
    settings: Vec<(&'static settings::Spec, bool)>,
    markers: Option<Vec<NavigatorRow>>,
    find_marker: bool,
    preview: Vec<NavigatorRow>,
}

impl PaletteModel {
    fn markers_section(&self) -> usize {
        usize::from(self.preview.is_empty())
    }
}

const PREVIEW: usize = 8;
const FIND_MARKER: &str = "Find Marker… (@)";

gpui_kit::actions!(palette, [MeasureFromMarker, RenameMarker, RemoveMarker,]);

pub(crate) fn key_bindings() -> Vec<gpui_kit::KeyBinding> {
    use gpui_kit::KeyBinding;
    vec![
        KeyBinding::new("shift-enter", MeasureFromMarker, Some("Command")),
        KeyBinding::new("f2", RenameMarker, Some("Command")),
        KeyBinding::new("delete", RemoveMarker, Some("Input")),
    ]
}

const MAX_SETTINGS: usize = 8;

impl PaletteModel {
    fn compute(query: &str, app: &CoreApp) -> Self {
        if let Some(query) = volna_core::marker::navigator_query(query) {
            return Self {
                commands: Vec::new(),
                settings: Vec::new(),
                markers: Some(app.navigator_rows(query)),
                find_marker: false,
                preview: Vec::new(),
            };
        }
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let commands = crate::commands::palette(app, query)
            .into_iter()
            .filter(|(label, _)| {
                let lower = label.to_lowercase();
                words.iter().all(|w| lower.contains(w))
            })
            .collect();
        let store = &app.settings;
        let settings = if query.trim().is_empty() {
            Vec::new()
        } else {
            settings::search(query, store.host(), &|id| store.is_modified(id))
                .into_iter()
                .take(MAX_SETTINGS)
                .map(|hit| {
                    let on = matches!(hit.spec.kind, Kind::Bool)
                        && store
                            .value(hit.spec.id)
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                    (hit.spec, on)
                })
                .collect()
        };
        let asked = volna_core::marker::palette_preview(query);
        let loaded = app.doc.is_loaded();
        let find = FIND_MARKER.to_lowercase();
        Self {
            commands,
            settings,
            markers: None,
            find_marker: loaded
                && (asked.is_some() || words.iter().all(|w| find.contains(w.as_str()))),
            preview: asked
                .filter(|_| loaded)
                .map(|rest| {
                    let mut rows = app.navigator_rows(&rest);
                    rows.truncate(PREVIEW);
                    rows
                })
                .unwrap_or_default(),
        }
    }
}

fn marker_row(row: &NavigatorRow, cx: &gpui_kit::App) -> gpui_kit::Div {
    let t = *crate::theme::theme(cx);
    let colors = t.editor;
    let mono = |text: String, color: gpui_kit::Hsla, w: f32| {
        div()
            .flex_none()
            .w(t.px(w))
            .overflow_hidden()
            .whitespace_nowrap()
            .font_family(t.mono_font)
            .text_size(px(t.ui_size_small))
            .text_color(color)
            .child(SharedString::from(text))
    };
    let marker = t.marker(row.id.palette_index());
    let number = if row.is_reference {
        format!("{} R", row.id)
    } else {
        row.id.to_string()
    };
    div()
        .debug_selector(move || format!("marker-row-{}", row.id))
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .child(mono(number, marker.stroke, 36.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(if row.name.is_some() {
                    colors.text
                } else {
                    colors.text_placeholder
                })
                .child(SharedString::from(
                    row.name.clone().unwrap_or_else(|| "No name".into()),
                )),
        )
        .child(mono(row.time.clone(), colors.text_muted, 84.0))
        .child(mono(
            row.step.clone().unwrap_or_default(),
            colors.text_muted,
            84.0,
        ))
        .child(mono(
            row.from_reference
                .clone()
                .map(|d| format!("R {d}"))
                .unwrap_or_default(),
            colors.text_muted,
            190.0,
        ))
}

impl Workspace {
    pub(crate) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette_with("", window, cx);
    }

    pub(crate) fn open_palette_with(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = cx.new(|cx| CommandState::new(window, cx));
        let model = cx.new(|_| PaletteModel::compute(query, &self.app));
        let ws: WeakEntity<Workspace> = cx.weak_entity();
        let query_ws = ws.clone();
        let query_model = model.clone();
        let confirm_ws = ws;
        let confirm_model = model.clone();
        let command_target = self.window_focus(window);
        let binding_target = window.focused(cx).unwrap_or_else(|| command_target.clone());
        let focus_state = state.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let state = state.clone();
            let model = model.clone();
            let query_ws = query_ws.clone();
            let query_model = query_model.clone();
            let confirm_ws = confirm_ws.clone();
            let confirm_model = confirm_model.clone();
            let command_target = command_target.clone();
            let binding_target = binding_target.clone();
            dialog
                .title("Command palette")
                .overlay_closable(true)
                .content(move |content, _, cx| {
                    let read = model.read(cx);
                    let navigator = read.markers.is_some();
                    let mut palette = Palette::new(&state)
                        .filterable(false)
                        .bordered(false)
                        .placeholder(if navigator {
                            "Find a marker by name or number…"
                        } else {
                            "Type a command, search settings, or @ for markers…"
                        })
                        .on_query({
                            let ws = query_ws.clone();
                            let model = query_model.clone();
                            move |query, _, cx| {
                                let Some(ws) = ws.upgrade() else { return };
                                let next = PaletteModel::compute(query, &ws.read(cx).app);
                                model.update(cx, |model, cx| {
                                    *model = next;
                                    cx.notify();
                                });
                            }
                        })
                        .on_confirm({
                            let ws = confirm_ws.clone();
                            let model = confirm_model.clone();
                            let state = state.clone();
                            let command_target = command_target.clone();
                            move |path, window, cx| {
                                let (find, marker) = {
                                    let read = model.read(cx);
                                    match &read.markers {
                                        Some(rows) => (false, rows.get(path.row).map(|r| r.id)),
                                        None if path.section == read.markers_section() => {
                                            let row = path.row;
                                            match (read.find_marker, row) {
                                                (true, 0) => (true, None),
                                                (true, row) => {
                                                    (false, read.preview.get(row - 1).map(|r| r.id))
                                                }
                                                (false, row) => {
                                                    (false, read.preview.get(row).map(|r| r.id))
                                                }
                                            }
                                        }
                                        None => (false, None),
                                    }
                                };
                                if find {
                                    state.update(cx, |state, cx| state.set_query("@", window, cx));
                                    return;
                                }
                                if let Some(id) = marker {
                                    window.close_dialog(cx);
                                    drop(ws.update(cx, |ws, cx| {
                                        ws.dispatch(
                                            Command::Lane(LaneVerb::GoTo(id)),
                                            Some(window),
                                            cx,
                                        );
                                    }));
                                    return;
                                }
                                let (action, chosen) = {
                                    let read = model.read(cx);
                                    if path.section == 2 {
                                        (None, read.settings.get(path.row).copied())
                                    } else if path.section != read.markers_section() {
                                        (
                                            read.commands
                                                .get(path.row)
                                                .map(|(_, action)| action.boxed_clone()),
                                            None,
                                        )
                                    } else {
                                        (None, None)
                                    }
                                };
                                window.close_dialog(cx);
                                if let Some(action) = action {
                                    command_target.dispatch_action(action.as_ref(), window, cx);
                                }
                                drop(ws.update(cx, |ws, cx| ws.focus_rename(window, cx)));
                                let Some((spec, on)) = chosen else { return };
                                drop(ws.update(cx, |ws, cx| {
                                    let command = if matches!(spec.kind, Kind::Bool) {
                                        SettingsCommand::Set {
                                            id: spec.id.into(),
                                            value: Value::Bool(!on),
                                        }
                                    } else {
                                        SettingsCommand::Reveal { id: spec.id.into() }
                                    };
                                    ws.dispatch(Command::Settings(command), Some(window), cx);
                                }));
                            }
                        })
                        .on_cancel(gpui_kit::component::WindowExt::close_dialog);
                    if let Some(rows) = &read.markers {
                        let mut group = CommandGroup::new().label("Markers");
                        for row in rows {
                            let label = match &row.name {
                                Some(name) => format!("{} {name}", row.id),
                                None => row.id.to_string(),
                            };
                            let row = row.clone();
                            group = group.item(
                                CommandItem::new()
                                    .label(label)
                                    .child(move |_, cx| marker_row(&row, cx)),
                            );
                        }
                        let empty = if read.markers.as_ref().is_some_and(|_| {
                            query_ws
                                .upgrade()
                                .is_some_and(|ws| ws.read(cx).app.doc.markers().is_empty())
                        }) {
                            "No markers yet · M marks the cursor"
                        } else {
                            "No marker matches"
                        };
                        let palette = palette
                            .group(group)
                            .empty(move |_, _, _| div().p_3().child(empty))
                            .footer(|_, _, cx| {
                                let t = *crate::theme::theme(cx);
                                div()
                                    .px_3()
                                    .py_1()
                                    .text_size(px(t.ui_size_small))
                                    .text_color(t.editor.text_muted)
                                    .child("↵ go to · ⇧↵ measure from · F2 rename · Del remove")
                            });
                        return content.child(navigator_keys(palette, &state, &model, &query_ws));
                    }
                    let mut commands = CommandGroup::new().label("Commands");
                    for (label, action) in &read.commands {
                        let label = label.clone();
                        let action = action.boxed_clone();
                        let binding_target = binding_target.clone();
                        commands = commands.item(CommandItem::new().label(label.clone()).child(
                            move |window, _| {
                                let binding = Kbd::binding_for_action_in(
                                    action.as_ref(),
                                    &binding_target,
                                    window,
                                )
                                .or_else(|| Kbd::binding_for_action(action.as_ref(), None, window));
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .w_full()
                                    .child(label.clone())
                                    .children(binding)
                            },
                        ));
                    }
                    let mut markers = CommandGroup::new().label("Markers");
                    if read.find_marker {
                        markers = markers.item(CommandItem::new().label(FIND_MARKER));
                    }
                    for row in &read.preview {
                        let label = match &row.name {
                            Some(name) => format!("{} {name}", row.id),
                            None => row.id.to_string(),
                        };
                        let row = row.clone();
                        markers = markers.item(
                            CommandItem::new()
                                .label(label)
                                .child(move |_, cx| marker_row(&row, cx)),
                        );
                    }
                    palette = if read.markers_section() == 0 {
                        palette.group(markers).group(commands)
                    } else {
                        palette.group(commands).group(markers)
                    };
                    let mut group = CommandGroup::new().label("Settings");
                    for (spec, on) in &read.settings {
                        group = group.item(
                            CommandItem::new()
                                .label(format!("{}: {}", spec.page.title(), spec.title))
                                .keywords(spec.keywords.iter().copied())
                                .checked(matches!(spec.kind, Kind::Bool) && *on),
                        );
                    }
                    palette = palette.group(group);
                    content.child(palette)
                })
        });
        if !query.is_empty() {
            focus_state.update(cx, |state, cx| state.set_query(query, window, cx));
        }
        let focus = focus_state.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }
}

fn navigator_keys(
    palette: Palette,
    state: &gpui_kit::Entity<CommandState>,
    model: &gpui_kit::Entity<PaletteModel>,
    ws: &WeakEntity<Workspace>,
) -> gpui_kit::Div {
    let selected = {
        let (state, model) = (state.clone(), model.clone());
        move |cx: &gpui_kit::App| {
            let row = state.read(cx).selected_index()?.row;
            model.read(cx).markers.as_ref()?.get(row).map(|r| r.id)
        }
    };
    let act = {
        let (ws, model, state) = (ws.clone(), model.clone(), state.clone());
        move |verb: LaneVerb, close: bool, window: &mut Window, cx: &mut gpui_kit::App| {
            if close {
                window.close_dialog(cx);
            }
            let Some(owner) = ws.upgrade() else { return };
            owner.update(cx, |ws, cx| {
                ws.dispatch(Command::Lane(verb), Some(window), cx);
            });
            if !close {
                let query = state.read(cx).query(cx).to_string();
                let next = PaletteModel::compute(&query, &owner.read(cx).app);
                model.update(cx, |model, cx| {
                    *model = next;
                    cx.notify();
                });
            }
        }
    };
    let (s1, s2, s3) = (selected.clone(), selected.clone(), selected);
    let (a1, a2, a3) = (act.clone(), act.clone(), act);
    div()
        .on_action(move |_: &MeasureFromMarker, window, cx| match s1(cx) {
            Some(id) => a1(LaneVerb::MeasureFrom(id), true, window, cx),
            None => cx.propagate(),
        })
        .on_action(move |_: &RenameMarker, window, cx| match s2(cx) {
            Some(id) => a2(LaneVerb::Rename(id), true, window, cx),
            None => cx.propagate(),
        })
        .on_action(move |_: &RemoveMarker, window, cx| match s3(cx) {
            Some(id) => a3(LaneVerb::Remove(id), false, window, cx),
            None => cx.propagate(),
        })
        .child(palette)
}
