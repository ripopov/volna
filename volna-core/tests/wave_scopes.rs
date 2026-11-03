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
    assert_eq!(app.panels.waves(id).unwrap().scope.width(), 235.0);
}

#[test]
fn narrow_panels_preserve_name_space_and_keep_columns_inside_the_panel() {
    for theme in [Theme::one_dark(), Theme::volna(false).zoomed(2.0)] {
        let mut app = app();
        for width in [0.0, 100.0, 250.0, 360.0, 1200.0] {
            frame(&mut app, &theme, width);
            let layout = app.panels.focused_waves().unwrap().last_layout();
            assert!(
                layout.names.width()
                    >= (72.0 * theme.zoom).min((layout.waves.left() - layout.scopes.right()) / 2.0)
            );
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
    app.panels.focused_waves_mut().unwrap().scope.resize(72.0);
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

fn press(app: &mut App, position: volna_core::geometry::Point) {
    let id = app.panels.focused_id();
    app.handle(Command::Pointer(
        id,
        PointerEvent::Down {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
}

#[test]
fn scope_header_and_keyboard_restore_width_after_workspace_reload() {
    use volna_core::Action;
    for theme in [Theme::one_dark(), Theme::volna(false).zoomed(1.5)] {
        let mut app = app();
        app.panels.focused_waves_mut().unwrap().scope.resize(235.0);
        let selected = app.panels.focused_waves().unwrap().selected.clone();
        frame(&mut app, &theme, 1200.0);
        let layout = app.panels.focused_waves().unwrap().last_layout().clone();
        press(
            &mut app,
            point(
                layout.scopes_toggle.left() + 8.0 * theme.zoom,
                layout.header.top() + 5.0,
            ),
        );
        app.handle(Command::Pointer(app.panels.focused_id(), PointerEvent::Up));
        frame(&mut app, &theme, 1200.0);
        let compact = app.panels.focused_waves().unwrap().last_layout().clone();
        assert_eq!(compact.scopes.width(), 20.0 * theme.zoom);
        assert_eq!(compact.names.width(), layout.names.width());
        assert!((compact.waves.width() - layout.waves.width() - 215.0 * theme.zoom).abs() < 0.001);
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
        assert_eq!(app.panels.focused_waves().unwrap().scope.width(), 20.0);
        app.handle(Command::Action(Action::ToggleScopes));
        assert_eq!(app.panels.focused_waves().unwrap().scope.width(), 235.0);
        assert_eq!(app.panels.focused_waves().unwrap().selected, selected);
    }
}

#[test]
fn dragging_scope_divider_snaps_to_rail_and_can_expand_again() {
    for theme in [Theme::one_dark(), Theme::volna(false).zoomed(2.0)] {
        let mut app = app();
        frame(&mut app, &theme, 1200.0);
        let id = app.panels.focused_id();
        let layout = app.panels.focused_waves().unwrap().last_layout().clone();
        press(&mut app, point(layout.scopes.right(), 100.0));
        app.handle(Command::Pointer(
            id,
            PointerEvent::Move {
                position: point(layout.scopes.left() - 100.0, 100.0),
            },
        ));
        app.handle(Command::Pointer(id, PointerEvent::Up));
        frame(&mut app, &theme, 1200.0);
        assert_eq!(app.panels.focused_waves().unwrap().scope.width(), 20.0);
        let compact = app.panels.focused_waves().unwrap().last_layout().clone();
        press(&mut app, point(compact.scopes.right(), 100.0));
        app.handle(Command::Pointer(
            id,
            PointerEvent::Move {
                position: point(compact.scopes.left() + 190.0 * theme.zoom, 100.0),
            },
        ));
        app.handle(Command::Pointer(id, PointerEvent::Up));
        assert_eq!(app.panels.focused_waves().unwrap().scope.width(), 190.0);
    }
}

#[test]
fn compact_scopes_keep_full_path_tooltips_and_accessible_names() {
    let mut app = app();
    app.handle(Command::Action(volna_core::Action::ToggleScopes));
    let theme = Theme::one_dark();
    frame(&mut app, &theme, 1200.0);
    let id = app.panels.focused_id();
    let layout = app.panels.focused_waves().unwrap().last_layout().clone();
    let scene = app.render_panel(id, &theme, &mut MonoMeasure);
    assert!(!scene.texts().any(|text| text == "top.cpu"));
    let swatches: Vec<_> = scene
        .prims
        .iter()
        .filter_map(|prim| match prim {
            Prim::Quad { rect, fill, .. }
                if rect.left() >= layout.scopes.left()
                    && rect.right() < layout.scopes.right()
                    && rect.width() == 2.0 * theme.zoom =>
            {
                Some(*fill)
            }
            _ => None,
        })
        .collect();
    assert_eq!(swatches.len(), 2);
    assert_ne!(swatches[0], swatches[1]);
    let labels: Vec<_> = app
        .panels
        .waves(id)
        .unwrap()
        .accessible_rows(&app.doc)
        .map(|row| row.label)
        .collect();
    assert!(labels.iter().any(|label| label == "top.cpu.valid"));
    assert!(labels.iter().any(|label| label == "top.dma_io.valid"));
    app.handle(Command::Pointer(
        id,
        PointerEvent::Move {
            position: point(layout.scopes.left() + 10.0, layout.row_y(0) + 10.0),
        },
    ));
    assert!(
        app.render_panel(id, &theme, &mut MonoMeasure)
            .texts()
            .any(|text| text == "top.cpu")
    );
    app.handle(Command::Action(volna_core::Action::ToggleScopes));
    frame(&mut app, &theme, 1200.0);
    let scene = app.render_panel(id, &theme, &mut MonoMeasure);
    for fill in swatches {
        assert!(
            scene
                .prims
                .iter()
                .any(|prim| matches!(prim, Prim::Quad { fill: other, .. } if other.with_alpha(1.0) == fill.with_alpha(1.0)))
        );
    }
}

#[test]
fn collapsing_scopes_reclaims_plot_space_and_keeps_shared_timelines_aligned() {
    let mut app = app();
    app.handle(Command::Action(volna_core::Action::SplitDown));
    let id = app.panels.focused_id();
    let theme = Theme::one_dark();
    frame(&mut app, &theme, 1200.0);
    let before = app.panels.waves(id).unwrap().last_layout().waves.left();
    app.handle(Command::Action(volna_core::Action::ToggleScopes));
    frame(&mut app, &theme, 1200.0);
    let after = app.panels.waves(id).unwrap().last_layout().waves.left();
    assert_eq!(before - after, 140.0);
    let first = app
        .panels
        .iter()
        .find(|panel| panel.id != id && panel.kind.waves().is_some())
        .unwrap()
        .id;
    app.layout_panel(
        first,
        Rect::from_xywh(25.0, 0.0, 1200.0, 400.0),
        &theme,
        1.0,
        &mut MonoMeasure,
    )
    .unwrap();
    assert_eq!(
        app.panels.waves(first).unwrap().last_layout().waves.left(),
        after
    );
    app.handle(Command::Action(volna_core::Action::ToggleScopes));
    frame(&mut app, &theme, 1200.0);
    assert_eq!(
        app.panels.waves(id).unwrap().last_layout().waves.left(),
        before
    );
}

#[test]
fn compact_rail_stays_available_when_shared_timeline_space_is_tight() {
    let mut app = app();
    app.handle(Command::Action(volna_core::Action::SplitDown));
    app.handle(Command::Action(volna_core::Action::ToggleScopes));
    let theme = Theme::one_dark();
    frame(&mut app, &theme, 180.0);
    let layout = app.panels.focused_waves().unwrap().last_layout();
    assert_eq!(layout.scopes.width(), 20.0);
    assert!(layout.scopes_toggle.width() > 0.0);
    assert!(layout.waves.right() <= layout.bounds.right());
}

fn band_app() -> App {
    let source = ProceduralTrace::new(10);
    let mut hierarchy = source.hierarchy().to_builder();
    hierarchy.scopes[0].name = "Stats".into();
    hierarchy.scopes[1].name = "Memory".into();
    let compute = hierarchy.push_scope("Compute".into(), "module".into(), Some(0));
    hierarchy.vars.truncate(6);
    for scope in &mut hierarchy.scopes {
        scope.vars.clear();
    }
    for (id, variable) in hierarchy.vars.iter_mut().enumerate() {
        variable.name = format!("metric_{id}");
        variable.scope = if id < 4 { 1 } else { compute };
        hierarchy.scopes[variable.scope].vars.push(id);
    }
    let mut app = App::new();
    app.set_session(hierarchy_session(hierarchy.finish()));
    app.handle(Command::AddVars(a_all(0..6)));
    app.handle(Command::Action(volna_core::Action::IncreaseRowHeight));
    app.handle(Command::Action(volna_core::Action::IncreaseRowHeight));
    app.panels.focused_waves_mut().unwrap().selected = [0].into();
    app.handle(Command::Action(volna_core::Action::IncreaseRowHeight));
    app
}

#[test]
fn scope_bands_merge_variable_height_rows_and_keep_one_label_visible_while_scrolling() {
    for theme in [Theme::one_dark(), Theme::volna(false).zoomed(1.5)] {
        let mut app = band_app();
        app.layout_panel(
            app.panels.focused_id(),
            Rect::from_xywh(25.0, 0.0, 1200.0, 900.0),
            &theme,
            1.0,
            &mut MonoMeasure,
        )
        .unwrap();
        let id = app.panels.focused_id();
        let layout = app.panels.waves(id).unwrap().last_layout().clone();
        let scene = app.render_panel(id, &theme, &mut MonoMeasure);
        assert_eq!(scene.texts().filter(|text| *text == "Memory").count(), 1);
        let memory_bottom = layout.row_y(4);
        assert!(scene.prims.iter().any(|prim| matches!(prim,
            Prim::Quad { rect, .. } if rect.width() == 2.0 * theme.zoom
                && rect.top() == layout.row_y(0) + 2.0 * theme.zoom
                && (rect.bottom() - (memory_bottom - 2.0 * theme.zoom)).abs() < 0.001
        )));
        let selected = app.panels.waves(id).unwrap().selected.clone();
        app.panels.waves_mut(id).unwrap().scroll_y = 160.0 * theme.zoom;
        app.layout_panel(
            id,
            Rect::from_xywh(25.0, 0.0, 1200.0, 200.0 * theme.zoom),
            &theme,
            1.0,
            &mut MonoMeasure,
        )
        .unwrap();
        let layout = app.panels.waves(id).unwrap().last_layout().clone();
        let scene = app.render_panel(id, &theme, &mut MonoMeasure);
        assert_eq!(scene.texts().filter(|text| *text == "Memory").count(), 1);
        assert!(scene.prims.iter().any(|prim| matches!(prim,
            Prim::Text { text, origin, .. } if text == "Memory" && origin.y == layout.scopes.top()
        )));
        assert_eq!(app.panels.waves(id).unwrap().selected, selected);
        let names: Vec<_> = app
            .panels
            .waves(id)
            .unwrap()
            .items()
            .iter()
            .map(|row| row.name().to_owned())
            .collect();
        assert_eq!(
            names,
            (0..6).map(|id| format!("metric_{id}")).collect::<Vec<_>>()
        );
    }
}

#[test]
fn compact_bands_preserve_selection_and_scope_boundaries_on_hover() {
    let mut app = band_app();
    app.handle(Command::Action(volna_core::Action::ToggleScopes));
    let theme = Theme::one_dark();
    frame(&mut app, &theme, 1200.0);
    let id = app.panels.focused_id();
    let layout = app.panels.waves(id).unwrap().last_layout().clone();
    let scene = app.render_panel(id, &theme, &mut MonoMeasure);
    let edges: Vec<_> = scene
        .prims
        .iter()
        .filter_map(|prim| match prim {
            Prim::Quad { rect, .. }
                if rect.width() == 2.0
                    && rect.left() > layout.scopes.left()
                    && rect.right() < layout.scopes.right() =>
            {
                Some(*rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(edges.len(), 2);
    assert!(edges[0].bottom() < edges[1].top());
    assert!(
        !scene
            .texts()
            .any(|text| text == "Memory" || text == "Stats.Memory")
    );
    let selected = app.panels.waves(id).unwrap().selected.clone();
    app.handle(Command::Pointer(
        id,
        PointerEvent::Move {
            position: point(layout.scopes.left() + 10.0, layout.row_y(1) + 10.0),
        },
    ));
    let scene = app.render_panel(id, &theme, &mut MonoMeasure);
    assert!(scene.texts().any(|text| text == "Stats.Memory"));
    assert!(scene.prims.iter().any(|prim| matches!(prim,
        Prim::Quad { rect, fill, .. } if rect.width() == 2.0 && rect.left() > layout.scopes.left() && rect.right() < layout.scopes.right() && fill.a == 1.0
    )));
    assert_eq!(app.panels.waves(id).unwrap().selected, selected);
}

#[test]
fn distinct_scope_ids_with_the_same_display_path_remain_separate() {
    use volna_core::scene::Prim;
    use volna_core::testing::hierarchy_session;
    use volna_trace::session::Session;
    let source = ProceduralTrace::new(10);
    let mut hierarchy = source.hierarchy().to_builder();
    hierarchy.scopes[0].name = "top".into();
    hierarchy.scopes[1].name = "cpu".into();
    for scope in &mut hierarchy.scopes {
        scope.vars.clear();
    }
    let other = hierarchy.push_scope("cpu".into(), "module".into(), Some(0));
    hierarchy.vars.truncate(2);
    for (id, scope) in [1, other].into_iter().enumerate() {
        hierarchy.vars[id].scope = scope;
        hierarchy.scopes[scope].vars.push(id);
    }
    let mut app = App::new();
    app.set_session(hierarchy_session(hierarchy.finish()));
    app.handle(Command::AddVars(a_all([0, 1])));
    frame(&mut app, &Theme::one_dark(), 1200.0);
    let id = app.panels.focused_id();
    let layout = app.panels.waves(id).unwrap().last_layout().clone();
    let scene = app.render_panel(id, &Theme::one_dark(), &mut MonoMeasure);
    let bands = scene.prims.iter().filter(|prim| matches!(prim,
        Prim::Quad { rect, .. } if rect.width() == 2.0 && rect.left() > layout.scopes.left() && rect.right() < layout.scopes.right()
    )).count();
    assert_eq!(bands, 2);
}
