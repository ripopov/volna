use std::sync::Arc;

use volna_core::geometry::{Modifiers, MouseButton, Rect, point};
use volna_core::scene::MonoMeasure;
use volna_core::testing::{ProceduralTrace, a_all, paths};
use volna_core::wave::{PointerEvent, ScopeColumn};
use volna_core::workspace::Workspace;
use volna_core::{Action, App, Command, Theme};

fn app() -> App {
    let mut app = App::new();
    app.set_session(Arc::new(ProceduralTrace::new(10)));
    app.handle(Command::AddVars(a_all([0])));
    app.handle(Command::Action(Action::SplitDown));
    app
}

fn frame(app: &mut App, width: f32, theme: &Theme) {
    let ids: Vec<_> = app
        .panels
        .iter()
        .filter(|panel| panel.kind.waves().is_some())
        .map(|panel| panel.id)
        .collect();
    for id in ids {
        app.layout_panel(
            id,
            Rect::from_xywh(25.0, 0.0, width, 400.0),
            theme,
            1.0,
            &mut MonoMeasure,
        )
        .unwrap();
    }
}

fn metadata(app: &App, theme: &Theme) -> f32 {
    let layout = app.panels.focused_waves().unwrap().last_layout();
    (layout.waves.left() - layout.scopes.left()) / theme.zoom
}

fn resize_divider(app: &mut App, width: f32, theme: &Theme) {
    let id = app.panels.focused_id();
    let layout = app.panels.waves(id).unwrap().last_layout().clone();
    let y = layout.names.top() + 10.0;
    app.handle(Command::Pointer(
        id,
        PointerEvent::Down {
            position: point(layout.values.right(), y),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    app.handle(Command::Pointer(
        id,
        PointerEvent::Move {
            position: point(layout.scopes.left() + width * theme.zoom, y),
        },
    ));
    app.handle(Command::Pointer(id, PointerEvent::Up));
    frame(app, 1200.0, theme);
}

fn round_trip(app: &mut App) {
    let saved = Workspace::capture(app, paths("trace.vtr"), None).unwrap();
    Workspace::parse(&saved.to_bytes().unwrap())
        .unwrap()
        .prepare(app, "file:///tmp/trace.vtr", "file:///tmp/trace.volna.json")
        .unwrap()
        .commit(app)
        .unwrap();
}

#[test]
fn constrained_shared_dividers_survive_scope_cycles_and_collapsed_workspace_reload() {
    for theme in [Theme::one_dark(), Theme::volna(false).zoomed(1.5)] {
        for width in [216.0, 300.0, 500.0] {
            let mut app = app();
            frame(&mut app, 1200.0, &theme);
            resize_divider(&mut app, width, &theme);
            let before = metadata(&app, &theme);
            for _ in 0..3 {
                app.handle(Command::Action(Action::ToggleScopes));
                round_trip(&mut app);
                frame(&mut app, 100.0, &theme);
                app.handle(Command::Action(Action::ToggleScopes));
                frame(&mut app, 1200.0, &theme);
                assert_eq!(metadata(&app, &theme), before);
                assert_eq!(
                    Workspace::capture(&app, paths("trace.vtr"), None)
                        .unwrap()
                        .timeline_dividers[0]
                        .width,
                    width
                );
            }
        }
    }
}

#[test]
fn multiple_scope_columns_can_expand_in_any_order_without_divider_drift() {
    use volna_core::panels::PanelsCommand;
    for reverse in [false, true] {
        let mut app = app();
        let second = app.panels.focused_id();
        let first = app
            .panels
            .iter()
            .find(|panel| panel.id != second && panel.kind.waves().is_some())
            .unwrap()
            .id;
        app.panels.waves_mut(second).unwrap().scope.resize(235.0);
        let theme = Theme::one_dark();
        frame(&mut app, 1200.0, &theme);
        resize_divider(&mut app, 216.0, &theme);
        let before = metadata(&app, &theme);
        for id in [first, second] {
            app.handle(Command::Panels(PanelsCommand::Focus(id)));
            app.handle(Command::Action(Action::ToggleScopes));
        }
        round_trip(&mut app);
        for id in if reverse {
            [second, first]
        } else {
            [first, second]
        } {
            app.handle(Command::Panels(PanelsCommand::Focus(id)));
            app.handle(Command::Action(Action::ToggleScopes));
        }
        frame(&mut app, 1200.0, &theme);
        for panel in app.panels.iter().filter_map(|panel| panel.kind.waves()) {
            assert_eq!(
                panel.last_layout().waves.left() - panel.last_layout().scopes.left(),
                before
            );
        }
    }
}

#[test]
fn resizing_a_shared_divider_while_collapsed_sets_a_new_reversible_reference() {
    let mut app = app();
    let theme = Theme::one_dark();
    frame(&mut app, 1200.0, &theme);
    app.handle(Command::Action(Action::ToggleScopes));
    frame(&mut app, 1200.0, &theme);
    resize_divider(&mut app, 400.0, &theme);
    round_trip(&mut app);
    app.handle(Command::Action(Action::ToggleScopes));
    frame(&mut app, 1200.0, &theme);
    assert_eq!(metadata(&app, &theme), 540.0);
    app.handle(Command::Action(Action::ToggleScopes));
    frame(&mut app, 1200.0, &theme);
    assert_eq!(metadata(&app, &theme), 400.0);
}

#[test]
fn scope_column_validates_saved_state_and_keeps_its_expanded_width_when_snapping() {
    let mut column = ScopeColumn::default();
    column.resize(235.0);
    column.resize(30.0);
    assert_eq!(column.width(), 20.0);
    let mut restored: ScopeColumn =
        serde_json::from_str(&serde_json::to_string(&column).unwrap()).unwrap();
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        restored.resize(invalid);
    }
    restored.toggle();
    assert_eq!(restored.width(), 235.0);
    restored.resize(50.0);
    assert_eq!(restored.width(), 72.0);
    for invalid in [
        serde_json::json!({"collapsed":true,"expanded_width":-1}),
        serde_json::json!({"collapsed":true,"expanded_width":20}),
        serde_json::json!({"collapsed":false,"expanded_width":null}),
    ] {
        assert!(serde_json::from_value::<ScopeColumn>(invalid).is_err());
    }
}
