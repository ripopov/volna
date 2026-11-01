use volna_core::geometry::{Modifiers, MouseButton, Rect, point};
use volna_core::scene::{FontRole, MonoMeasure, Prim, TextMeasure};
use volna_core::testing::{ProceduralTrace, a_all, hierarchy_session};
use volna_core::wave::{Drag, PointerEvent};
use volna_core::{App, Command, Theme};
use volna_trace::session::Session;

fn app() -> App {
    let source = ProceduralTrace::new(10);
    let mut h = source.hierarchy().to_builder();
    h.scopes[0].name = "top".into();
    h.scopes[1].name = "cpu".into();
    let dma = h.push_scope("dma_io".into(), "module".into(), Some(0));
    h.vars.truncate(2);
    for scope in &mut h.scopes {
        scope.vars.clear();
    }
    for (id, scope) in [1, dma].into_iter().enumerate() {
        h.vars[id].name = "valid".into();
        h.vars[id].scope = scope;
        h.scopes[scope].vars.push(id);
    }
    let mut app = App::new();
    app.set_session(hierarchy_session(h.finish()));
    app.handle(Command::AddVars(a_all([0, 1])));
    app
}

fn frame(app: &mut App, theme: &Theme, width: f32) {
    app.layout_panel(
        app.panels.focused_id(),
        Rect::from_xywh(25.0, 0.0, width, 400.0),
        theme,
        1.0,
        &mut MonoMeasure,
    )
    .unwrap();
}

#[test]
fn duplicate_names_stay_aligned_with_distinct_scope_cells() {
    for theme in [Theme::one_dark(), Theme::volna(false).zoomed(1.5)] {
        let mut app = app();
        frame(&mut app, &theme, 1200.0);
        let id = app.panels.focused_id();
        let layout = app.panels.waves(id).unwrap().last_layout().clone();
        let scene = app.render_panel(id, &theme, &mut MonoMeasure);
        let names: Vec<_> = scene
            .prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::Text { origin, text, .. } if text == "valid" => Some(*origin),
                _ => None,
            })
            .collect();
        assert_eq!(names.len(), 2);
        assert_eq!(names[0].x, names[1].x);
        for (row, scope) in ["top.cpu", "top.dma_io"].into_iter().enumerate() {
            assert!(scene.prims.iter().any(|prim| matches!(prim,
                Prim::Text { origin, text, .. }
                if text == scope && (origin.x + MonoMeasure.text_width(scope, FontRole::Ui, theme.ui_size)
                    - (layout.scopes.right() - 8.0 * theme.zoom)).abs() < 0.001
                    && origin.y == layout.row_y(row)
            )));
        }
        let position = point(
            layout.scopes.left() + 20.0,
            layout.row_y(1) + layout.row_h / 2.0,
        );
        app.handle(Command::Pointer(
            id,
            PointerEvent::Down {
                position,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        ));
        app.handle(Command::Pointer(id, PointerEvent::Up));
        assert_eq!(
            app.panels
                .waves(id)
                .unwrap()
                .selected
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [1]
        );
    }
}

#[test]
fn scope_resize_preserves_name_width_and_round_trips() {
    let mut app = app();
    let theme = Theme::one_dark();
    frame(&mut app, &theme, 1200.0);
    let id = app.panels.focused_id();
    let layout = app.panels.waves(id).unwrap().last_layout().clone();
    app.handle(Command::Pointer(
        id,
        PointerEvent::Down {
            position: point(layout.scopes.right(), 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    assert_eq!(app.panels.waves(id).unwrap().drag, Some(Drag::ScopesSplit));
    app.handle(Command::Pointer(
        id,
        PointerEvent::Move {
            position: point(layout.scopes.left() + 235.0, 10.0),
        },
    ));
    app.handle(Command::Pointer(id, PointerEvent::Up));
    frame(&mut app, &theme, 1200.0);
    let resized = app.panels.waves(id).unwrap().last_layout();
    assert_eq!(resized.names.width(), layout.names.width());
    assert_eq!(resized.names.left(), resized.scopes.right());
    assert_eq!(resized.scopes.width(), 235.0);
    assert_eq!(resized.values.left(), resized.names.right());
    let saved = volna_core::workspace::Workspace::capture(
        &app,
        volna_core::testing::paths("trace.vtr"),
        None,
    )
    .unwrap();
    saved
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    assert_eq!(app.panels.waves(id).unwrap().scopes_width, 235.0);
}

#[test]
fn narrow_panels_preserve_name_space_and_keep_columns_inside_the_panel() {
    for theme in [Theme::one_dark(), Theme::volna(false).zoomed(2.0)] {
        let mut app = app();
        for width in [0.0, 100.0, 250.0, 360.0, 1200.0] {
            frame(&mut app, &theme, width);
            let layout = app.panels.focused_waves().unwrap().last_layout();
            assert!(layout.names.width() >= (72.0 * theme.zoom).min(width / 2.0));
            for cell in [layout.names, layout.scopes, layout.values, layout.waves] {
                assert!(cell.width() >= 0.0);
                assert!(cell.left() >= layout.bounds.left());
                assert!(cell.right() <= layout.bounds.right());
            }
        }
    }
}

#[test]
fn narrow_scope_cells_keep_unicode_suffix_and_hover_shows_full_scope() {
    let mut app = app();
    let source = ProceduralTrace::new(10);
    let mut h = source.hierarchy().to_builder();
    h.scopes[0].name = "a_very_long_top_level_scope".into();
    h.scopes[1].name = "处理器".into();
    let var = h.vars.iter().position(|v| v.scope == 1).unwrap();
    app.set_session(hierarchy_session(h.finish()));
    app.handle(Command::AddVars(a_all([var])));
    app.panels.focused_waves_mut().unwrap().scopes_width = 72.0;
    let theme = Theme::one_dark();
    frame(&mut app, &theme, 1200.0);
    let id = app.panels.focused_id();
    let layout = app.panels.waves(id).unwrap().last_layout().clone();
    let row = app
        .panels
        .waves(id)
        .unwrap()
        .items()
        .iter()
        .position(|row| row.scope().ends_with("处理器"))
        .unwrap();
    let y = layout.entry_span(row).unwrap().0;
    let scene = app.render_panel(id, &theme, &mut MonoMeasure);
    assert!(
        scene
            .texts()
            .any(|text| text.starts_with('…') && text.ends_with("处理器"))
    );
    app.handle(Command::Pointer(
        id,
        PointerEvent::Move {
            position: point(layout.scopes.left() + 10.0, y + 10.0),
        },
    ));
    let scene = app.render_panel(id, &theme, &mut MonoMeasure);
    assert!(
        scene
            .texts()
            .any(|text| text == "a_very_long_top_level_scope.处理器")
    );
}
