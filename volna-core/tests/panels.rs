use std::collections::BTreeSet;
use std::sync::Arc;
use volna_core::testing::a_all;

use volna_core::Document;
use volna_core::app::SettingsCommand;
use volna_core::panels::PanelsCommand;
use volna_core::panels::{Axis, Layout, PanelId, PanelKind, Panels};
use volna_core::testing::ProceduralTrace;
use volna_core::wave::model::{LinkDim, PointerEvent};
use volna_core::{Action, App, Command, Instant};
use volna_trace::Session;
use volna_trace::data::source::Lookup;

fn linked_app() -> App {
    let mut app = App::new();
    app.set_session(Arc::new(ProceduralTrace::new(100)));
    for request in app.take_requests() {
        app.deliver(request.perform());
    }
    app.handle(Command::AddVars(a_all(vec![0, 1])));
    app
}

#[test]
fn closing_last_content_resets_shared_viewport_even_on_redo() {
    use volna_core::wave::viewport::Viewport;

    let mut app = linked_app();
    let first = app.panels.focused_id();
    panel(&mut app, PanelsCommand::NewTab { group_of: first });
    let second = app.panels.focused_id();
    let zoomed = Viewport {
        start: 10.0,
        end: 20.0,
    };
    app.doc.shared.viewport.set(zoomed);
    panel(&mut app, PanelsCommand::Close(second));
    assert_eq!(app.doc.shared.viewport.value, zoomed);
    panel(&mut app, PanelsCommand::Close(first));
    let fit = Viewport::fit(app.doc.limits());
    assert_eq!(app.doc.shared.viewport.value, fit);
    app.handle(Command::Undo);
    app.doc.shared.viewport.set(zoomed);
    app.handle(Command::Redo);
    assert_eq!(app.doc.shared.viewport.value, fit);
    assert_eq!(app.doc.shared.viewport.target(), fit);
}

fn panel(app: &mut App, command: PanelsCommand) {
    app.handle(Command::Panels(command));
}

fn wave_panels() -> Panels {
    let mut p = Panels::new();
    let start = p.focused_id();
    p.replace(start, PanelKind::Waves(Box::default())).unwrap();
    p
}

#[test]
fn focused_actions_update_one_shared_value_and_unlink_snapshots_the_displayed_frame() {
    let mut app = linked_app();
    let a = app.panels.focused_id();
    panel(
        &mut app,
        PanelsCommand::Split {
            panel: a,
            axis: Axis::Horizontal,
        },
    );
    let b = app.panels.focused_id();
    let now = Instant::now();
    app.handle_at(Command::Action(Action::ZoomIn), now);
    assert!(app.tick(now + std::time::Duration::from_millis(50)));
    let shared = app.doc.shared.viewport.value;
    assert_eq!(app.panels.waves(a).unwrap().viewport(&app.doc), shared);
    assert_eq!(app.panels.waves(b).unwrap().viewport(&app.doc), shared);
    panel(
        &mut app,
        PanelsCommand::ToggleLink {
            panel: b,
            dim: LinkDim::Viewport,
        },
    );
    app.tick(now + std::time::Duration::from_secs(1));
    assert_eq!(app.panels.waves(b).unwrap().viewport(&app.doc), shared);
    let retained = app.doc.shared.viewport.value;
    assert_ne!(retained, shared);
    app.handle_at(Command::Action(Action::PanRight), now);
    app.tick(now + std::time::Duration::from_secs(1));
    assert_eq!(app.panels.waves(a).unwrap().viewport(&app.doc), retained);
    assert_ne!(app.panels.waves(b).unwrap().viewport(&app.doc), shared);
    panel(
        &mut app,
        PanelsCommand::ToggleLink {
            panel: a,
            dim: LinkDim::Viewport,
        },
    );
    app.handle_at(Command::Action(Action::ZoomFit), now);
    app.tick(now + std::time::Duration::from_secs(1));
    panel(
        &mut app,
        PanelsCommand::ToggleLink {
            panel: b,
            dim: LinkDim::Viewport,
        },
    );
    assert_eq!(app.panels.waves(b).unwrap().viewport(&app.doc), retained);
    assert!(!app.panels.waves(b).unwrap().is_animating());
}

#[test]
fn cursor_links_are_independent_of_view_links_and_markers_use_the_focused_cursor() {
    let mut app = linked_app();
    let a = app.panels.focused_id();
    app.doc.shared.cursor = Some(1_u64 << 54);
    panel(
        &mut app,
        PanelsCommand::Split {
            panel: a,
            axis: Axis::Vertical,
        },
    );
    let b = app.panels.focused_id();
    panel(
        &mut app,
        PanelsCommand::ToggleLink {
            panel: b,
            dim: LinkDim::Cursor,
        },
    );
    assert_eq!(app.panels.waves(b).unwrap().cursor(&app.doc), Some(1 << 54));
    app.panels
        .waves_mut(b)
        .unwrap()
        .set_cursor(&mut app.doc, Some((1 << 54) + 1));
    app.handle(Command::Action(Action::AddOrRenameMarker));
    assert_eq!(app.doc.markers()[0].time, (1 << 54) + 1);
    assert_eq!(app.panels.waves(a).unwrap().cursor(&app.doc), Some(1 << 54));
    panel(
        &mut app,
        PanelsCommand::ToggleLink {
            panel: b,
            dim: LinkDim::Cursor,
        },
    );
    assert_eq!(app.panels.waves(b).unwrap().cursor(&app.doc), Some(1 << 54));
    app.handle(Command::Action(Action::AddOrRenameMarker));
    let ids: BTreeSet<_> = app.doc.markers().iter().map(|m| m.id).collect();
    assert_eq!(ids.len(), 2);
    app.handle(Command::Action(Action::RemoveAllMarkers));
    app.handle(Command::Action(Action::AddOrRenameMarker));
    assert_eq!(
        app.doc.markers()[0].id.get(),
        1,
        "a new marker takes the lowest free number"
    );
}

#[test]
fn pending_and_loaded_histories_are_shared_across_all_panels() {
    let mut app = linked_app();
    let a = app.panels.focused_id();
    let requests = app.take_requests();
    assert_eq!(requests.len(), 1);
    panel(&mut app, PanelsCommand::NewTab { group_of: a });
    let b = app.panels.focused_id();
    app.handle(Command::AddVars(a_all(vec![0])));
    assert!(app.take_requests().is_empty());
    for r in requests {
        app.deliver(r.perform());
    }
    let first = app.panels.waves(a).unwrap().items()[0]
        .signal()
        .unwrap()
        .history
        .clone()
        .unwrap();
    let second = app.panels.waves(b).unwrap().items()[0]
        .signal()
        .unwrap()
        .history
        .clone()
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    panel(&mut app, PanelsCommand::NewTab { group_of: b });
    app.handle(Command::AddVars(a_all(vec![0])));
    assert!(
        app.take_requests().is_empty(),
        "adding an already loaded signal must not decode again"
    );
    assert!(Arc::ptr_eq(
        &first,
        app.panels.focused_waves().unwrap().items()[0]
            .signal()
            .unwrap()
            .history
            .as_ref()
            .unwrap()
    ));
    let weak = Arc::downgrade(&first);
    drop(first);
    drop(second);
    for id in app.panels.layout().panels() {
        panel(&mut app, PanelsCommand::Close(id));
    }
    assert_eq!(
        weak.strong_count(),
        1,
        "only the procedural source retains its history"
    );
    app.close_trace();
    assert!(weak.upgrade().is_none());
}

#[test]
fn pointer_targets_do_not_drift_when_focus_changes_or_a_trace_is_replaced() {
    let mut app = linked_app();
    let theme = volna_core::Theme::one_dark();
    let a = app.panels.focused_id();
    panel(
        &mut app,
        PanelsCommand::Split {
            panel: a,
            axis: Axis::Horizontal,
        },
    );
    let b = app.panels.focused_id();
    panel(
        &mut app,
        PanelsCommand::ToggleLink {
            panel: a,
            dim: LinkDim::Cursor,
        },
    );
    let bounds = volna_core::geometry::Rect::from_xywh(0.0, 0.0, 1200.0, 600.0);
    app.layout_panel(a, bounds, &theme, 1.0, &mut volna_core::scene::MonoMeasure)
        .unwrap();
    let layout = app.panels.waves(a).unwrap().last_layout().clone();
    let click = PointerEvent::Down {
        position: volna_core::geometry::point(layout.waves.left() + 100.0, layout.row_y(0) + 10.0),
        button: volna_core::geometry::MouseButton::Left,
        modifiers: Default::default(),
    };
    app.handle(Command::Pointer(a, click));
    assert_eq!(app.panels.focused_id(), a);
    assert!(app.panels.waves(a).unwrap().cursor(&app.doc).is_some());
    assert_eq!(app.panels.waves(b).unwrap().cursor(&app.doc), None);
    app.close_trace();
    let before = app.debug_state();
    app.handle(Command::Pointer(a, click));
    assert_eq!(app.debug_state(), before);
}

#[test]
fn hierarchy_locators_round_trip_declarations_and_literal_dots() {
    let source = ProceduralTrace::new(10);
    let mut h = source.hierarchy().to_builder();
    h.scopes[0].name = "test.bench".into();
    h.vars[0].name = "\\escaped.name".into();
    let h = h.finish();
    for id in 0..h.var_count() {
        let (path, nth) = h.var_path(id);
        assert_eq!(h.find_var(&path, nth), Lookup::Found(id));
    }
    for id in 0..h.scope_count() {
        assert_eq!(h.find_scope(&h.scope_path(id)), Lookup::Found(id));
    }
    assert_eq!(h.find_scope(&["test", "bench"]), Lookup::Missing);
    assert_eq!(h.find_var(&["absent"], None), Lookup::Missing);
    assert_eq!(h.find_scope(&[] as &[String]), Lookup::Missing);
}

#[test]
fn duplicate_variables_require_occurrence_but_duplicate_scopes_cannot_be_guessed() {
    let source = ProceduralTrace::new(10);
    let mut h = source.hierarchy().to_builder();
    let duplicate = h.vars.len();
    h.vars.push(h.vars[0].clone());
    let scope = h.vars[0].scope;
    h.scopes[scope].vars.push(duplicate);
    let h = h.finish();
    let (path, nth) = h.var_path(duplicate);
    assert_eq!(nth, Some(1));
    assert_eq!(h.find_var(&path, None), Lookup::Ambiguous);
    assert_eq!(h.find_var(&path, Some(0)), Lookup::Found(0));
    assert_eq!(h.find_var(&path, Some(1)), Lookup::Found(duplicate));
    assert_eq!(h.find_var(&path, Some(2)), Lookup::Missing);
    let mut builder = h.to_builder();
    let root = builder.roots[0];
    builder.roots.push(builder.scopes.len());
    builder.scopes.push(builder.scopes[root].clone());
    let h = builder.finish();
    assert_eq!(h.find_var(&path, Some(0)), Lookup::Ambiguous);
    assert_eq!(h.find_scope(&h.scope_path(scope)), Lookup::Ambiguous);
}

#[test]
fn split_shares_survive_normalization_and_close() {
    let mut p = wave_panels();
    let a = p.focused_id();
    let b = p.create(a, Some(Axis::Horizontal)).unwrap();
    p.set_layout(
        Layout::Split {
            split: Axis::Horizontal,
            sizes: vec![0.8, 0.2],
            children: vec![Layout::single(a), Layout::single(b)],
        }
        .into(),
        p.revision(),
    )
    .unwrap();
    let c = p.create(a, Some(Axis::Horizontal)).unwrap();
    let Layout::Split {
        sizes, children, ..
    } = p.layout().center.as_ref().unwrap()
    else {
        panic!()
    };
    assert_eq!(sizes, &[0.4, 0.4, 0.2]);
    assert_eq!(children.len(), 3);
    assert_eq!(p.layout().panels(), vec![a, c, b]);
    p.close(c).unwrap();
    let Layout::Split { sizes, .. } = p.layout().center.as_ref().unwrap() else {
        panic!()
    };
    assert!((sizes[0] - 2.0 / 3.0).abs() < 1e-6);
    assert!((sizes[1] - 1.0 / 3.0).abs() < 1e-6);
    p.close(a).unwrap();
    assert_eq!(p.layout(), &Layout::single(b).into());
    p.close(b).unwrap();
    assert_eq!(p.len(), 1);
    p.validate().unwrap();
}

#[test]
fn closing_inactive_tabs_preserves_active_identity_and_focus_cycles_in_layout_order() {
    let mut p = wave_panels();
    let a = p.focused_id();
    let b = p.create(a, None).unwrap();
    let c = p.create(a, None).unwrap();
    let d = p.create(c, Some(Axis::Vertical)).unwrap();
    p.focus(c).unwrap();
    p.close(a).unwrap();
    assert_eq!(p.layout().visible(), vec![c, d]);
    assert_eq!(p.focused_id(), c);
    p.focus_next(false).unwrap();
    assert_eq!(p.focused_id(), d);
    p.focus_next(false).unwrap();
    assert_eq!(p.focused_id(), b);
    p.focus_next(true).unwrap();
    assert_eq!(p.focused_id(), d);
    p.validate().unwrap();
}

#[test]
fn invalid_and_stale_dock_proposals_are_atomic() {
    let mut p = wave_panels();
    let a = p.focused_id();
    let old = p.revision();
    let b = p.create(a, Some(Axis::Horizontal)).unwrap();
    let before = p.layout().clone();
    for tree in [
        Layout::single(a),
        Layout::Tabs {
            tabs: vec![a, a, b],
            active: a,
        },
        Layout::Tabs {
            tabs: vec![a, b],
            active: PanelId(999),
        },
        Layout::Split {
            split: Axis::Vertical,
            sizes: vec![f32::NAN, 1.0],
            children: vec![Layout::single(a), Layout::single(b)],
        },
        Layout::Split {
            split: Axis::Vertical,
            sizes: vec![1.0],
            children: vec![Layout::single(a), Layout::single(b)],
        },
        Layout::Split {
            split: Axis::Vertical,
            sizes: vec![0.0, 1.0],
            children: vec![Layout::single(a), Layout::single(b)],
        },
    ] {
        assert!(p.set_layout(tree.into(), p.revision()).is_err());
        assert_eq!(p.layout(), &before);
        assert_eq!(p.focused_id(), b);
    }
    assert!(p.set_layout(before.clone(), old).is_err());
    assert!(!p.set_layout(before, p.revision()).unwrap());
}

#[test]
fn split_copies_rows_but_shares_history_and_clears_transient_input() {
    let mut p = wave_panels();
    let mut doc = Document::new();
    let source = Arc::new(ProceduralTrace::new(10));
    doc.set_session(source.clone());
    let a = p.focused_id();
    let w = p.focused_mut().kind.waves_mut().unwrap();
    w.add_members(
        &mut doc,
        &volna_core::testing::a_all([0, 1].map(volna_trace::data::Member::Var)),
        &Default::default(),
    );
    w.finish_signal(
        volna_core::testing::a(source.hierarchy().var(0).signal),
        source.load_signal(source.hierarchy().var(0).signal),
    );
    w.drag = Some(volna_core::wave::model::Drag::Cursor);
    let b = p.create(a, Some(Axis::Vertical)).unwrap();
    let wa = p.get(a).unwrap().kind.waves().unwrap();
    let wb = p.get(b).unwrap().kind.waves().unwrap();
    assert_eq!(wa.items().len(), wb.items().len());
    assert!(Arc::ptr_eq(
        wa.items()[0].signal().unwrap().history.as_ref().unwrap(),
        wb.items()[0].signal().unwrap().history.as_ref().unwrap()
    ));
    assert_eq!(wb.drag, None);
    let c = p.create(b, None).unwrap();
    assert!(p.get(c).unwrap().kind.waves().unwrap().items().is_empty());
    p.get_mut(b).unwrap().kind.waves_mut().unwrap().reset(None);
    assert_eq!(p.get(a).unwrap().kind.waves().unwrap().items().len(), 2);
}

#[test]
fn deterministic_random_commands_preserve_invariants_and_never_reuse_ids() {
    let mut p = Panels::new();
    let mut seen = BTreeSet::from([p.focused_id()]);
    let mut rng = 0x913c_729a_835e_19a3_u64;
    for _ in 0..5000 {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let ids = p.layout().panels();
        let id = ids[(rng as usize / 12) % ids.len()];
        match rng % 12 {
            0..=2 if p.len() < 40 => {
                let axis = match rng % 3 {
                    0 => None,
                    1 => Some(Axis::Horizontal),
                    _ => Some(Axis::Vertical),
                };
                if let Ok(new) = p.create(id, axis) {
                    assert!(seen.insert(new));
                }
            }
            3 => {
                p.close(id).unwrap();
            }
            4 => {
                p.focus(id).unwrap();
            }
            5 => {
                p.focus_next(true).unwrap();
            }
            6 => {
                p.focus_next(false).unwrap();
            }
            7 => {
                p.reset().unwrap();
                assert!(seen.insert(p.focused_id()));
            }
            8 if !p.get(id).unwrap().kind.is_start() => {
                p.move_to_new_window(id).unwrap();
            }
            9 => {
                if let Some(window) = p.layout().windows.keys().next().copied() {
                    p.return_window(window).unwrap();
                }
            }
            10 => {
                p.close_others(id).unwrap();
            }
            11 if !p.get(id).unwrap().kind.is_start() => {
                p.move_panel(id, volna_core::panels::Placement::Right)
                    .unwrap();
            }
            _ => {}
        }
        p.validate().unwrap();
        let json = serde_json::to_string(p.layout()).unwrap();
        let tree: volna_core::panels::DockLayout = serde_json::from_str(&json).unwrap();
        assert_eq!(&tree, p.layout());
    }
}

#[test]
fn close_others_keeps_any_kind_and_a_lone_settings_tab_gets_a_start_panel() {
    let mut app = linked_app();
    app.handle(Command::Action(Action::SplitRight));
    app.handle(Command::Action(Action::SplitDown));
    let id = app.panels.focused_id();
    app.handle(Command::Panels(PanelsCommand::CloseOthers(id)));
    assert_eq!(app.panels.len(), 1);
    assert_eq!(app.panels.focused_id(), id);
    app.panels.validate().unwrap();
    app.handle(Command::Settings(SettingsCommand::Open));
    let settings = app.panels.settings_id().unwrap();
    app.handle(Command::Panels(PanelsCommand::CloseOthers(settings)));
    assert_eq!(app.panels.len(), 2);
    assert_eq!(app.panels.focused_id(), settings);
    let start = app.panels.iter().find(|p| p.kind.is_start()).unwrap().id;
    assert!(app.panels.can_close(settings));
    assert!(!app.panels.can_close(start));
    app.panels.validate().unwrap();
    app.handle(Command::AddVars(a_all(vec![0])));
    assert_eq!(app.panels.len(), 2);
    let waves = app.panels.focused_id();
    assert_eq!(app.panels.waves(waves).unwrap().items().len(), 1);
    assert!(app.panels.iter().all(|p| !p.kind.is_start()));
}

#[test]
fn the_start_panel_gives_way_to_content_and_returns_when_the_last_panel_closes() {
    let mut app = App::new();
    app.set_session(Arc::new(ProceduralTrace::new(100)));
    let start = app.panels.focused_id();
    assert!(app.panels.focused().kind.is_start());
    assert_eq!(app.panels.len(), 1);
    app.handle(Command::Action(Action::SplitRight));
    let waves = app.panels.focused_id();
    assert_ne!(waves, start);
    assert_eq!(app.panels.len(), 1);
    assert!(app.panels.get(start).is_none());
    assert!(app.panels.waves(waves).unwrap().nav.link.viewport);
    app.handle(Command::Action(Action::SplitRight));
    let second = app.panels.focused_id();
    assert!(app.panels.can_close(waves) && app.panels.can_close(second));
    app.handle(Command::Panels(PanelsCommand::Close(waves)));
    assert_eq!(app.panels.layout(), &Layout::single(second).into());
    app.handle(Command::Panels(PanelsCommand::Close(second)));
    assert_eq!(app.panels.len(), 1);
    let again = app.panels.focused_id();
    assert!(app.panels.focused().kind.is_start());
    assert!(again > second);
    app.panels.validate().unwrap();
    assert!(!app.panels.can_close(again));
    let revision = app.panels.revision();
    assert!(app.panels.close(again).unwrap().is_empty());
    assert_eq!(app.panels.revision(), revision);
    assert!(app.doc.is_loaded());
    app.handle(Command::Action(Action::ClosePanel));
    assert!(!app.doc.is_loaded());
    assert!(app.panels.focused().kind.is_start());
}

#[test]
fn closing_the_last_sidebar_content_returns_to_a_central_start_panel() {
    use volna_core::panels::Placement;
    for settings_placement in [None, Some(Placement::Center), Some(Placement::Right)] {
        let mut app = linked_app();
        let content = app.panels.focused_id();
        panel(
            &mut app,
            PanelsCommand::Move {
                panel: content,
                placement: Placement::Right,
            },
        );
        if let Some(placement) = settings_placement {
            app.handle(Command::Settings(SettingsCommand::Open));
            let settings = app.panels.settings_id().unwrap();
            panel(
                &mut app,
                PanelsCommand::Move {
                    panel: settings,
                    placement,
                },
            );
        }
        let before = app.panels.saved_view().0;
        panel(&mut app, PanelsCommand::Close(content));
        let start = app.panels.first_start().unwrap();
        assert!(app.panels.get(content).is_none());
        assert_eq!(
            app.panels.layout().placement(start),
            Some(Placement::Center)
        );
        assert_eq!(
            app.panels.layout().right.is_some(),
            settings_placement == Some(Placement::Right)
        );
        app.panels.validate().unwrap();
        let closed = app.panels.saved_view().0;
        app.handle(Command::Undo);
        assert_eq!(app.panels.saved_view().0, before);
        app.panels.validate().unwrap();
        app.handle(Command::Redo);
        assert_eq!(app.panels.saved_view().0, closed);
        if let Some(placement) = settings_placement {
            let settings = app.panels.settings_id().unwrap();
            assert_eq!(app.panels.layout().placement(settings), Some(placement));
        }
        app.panels.validate().unwrap();
    }
}

#[test]
fn keeping_only_sidebar_settings_puts_the_start_panel_in_the_center() {
    use volna_core::panels::Placement;
    let mut app = linked_app();
    app.handle(Command::Settings(SettingsCommand::Open));
    let settings = app.panels.settings_id().unwrap();
    panel(
        &mut app,
        PanelsCommand::Move {
            panel: settings,
            placement: Placement::Right,
        },
    );
    panel(&mut app, PanelsCommand::CloseOthers(settings));
    let start = app.panels.first_start().unwrap();
    assert_eq!(app.panels.len(), 2);
    assert_eq!(app.panels.focused_id(), settings);
    assert_eq!(app.panels.layout().center, Some(Layout::single(start)));
    assert_eq!(
        app.panels.layout().right.as_ref().unwrap().layout,
        Layout::single(settings)
    );
    app.panels.validate().unwrap();
}

#[test]
fn moving_the_start_panel_to_the_sidebar_is_rejected_without_changing_the_layout() {
    use volna_core::panels::{DockLayout, Placement, SideDock};
    let mut panels = Panels::new();
    let start = panels.focused_id();
    let before = panels.layout().clone();
    let revision = panels.revision();
    assert!(panels.move_panel(start, Placement::Right).is_err());
    let proposal = DockLayout {
        windows: Default::default(),
        center: None,
        right: Some(SideDock {
            layout: Layout::single(start),
            width: 420.0,
            open: true,
        }),
    };
    assert!(panels.set_layout(proposal, revision).is_err());
    assert_eq!(panels.layout(), &before);
    assert_eq!(panels.revision(), revision);
    panels.validate().unwrap();
}

#[test]
fn dock_moves_allow_an_empty_center_and_reject_duplicate_membership_atomically() {
    use volna_core::panels::{Placement, SideDock};
    let mut panels = wave_panels();
    let a = panels.focused_id();
    let b = panels.create(a, None).unwrap();
    panels.move_panel(b, Placement::Right).unwrap();
    let before = panels.layout().clone();
    let revision = panels.revision();
    let mut invalid = before.clone();
    invalid.right = Some(SideDock {
        layout: Layout::single(a),
        width: 400.0,
        open: true,
    });
    assert!(panels.set_layout(invalid, revision).is_err());
    assert_eq!(panels.layout(), &before);
    assert_eq!(panels.revision(), revision);
    let mut invalid = before;
    invalid.right.as_mut().unwrap().width = f32::NAN;
    assert!(panels.set_layout(invalid, revision).is_err());
    panels.move_panel(a, Placement::Right).unwrap();
    assert!(panels.layout().center.is_none());
    panels.toggle_right_dock().unwrap();
    assert!(panels.layout().visible().is_empty());
    panels.validate().unwrap();
    panels.focus(a).unwrap();
    assert!(panels.layout().right.as_ref().unwrap().open);
    panels.close(a).unwrap();
    assert_eq!(panels.focused_id(), b);
    panels.move_panel(b, Placement::Center).unwrap();
    assert!(panels.layout().right.is_none());
    panels.validate().unwrap();
}

#[test]
fn saving_with_settings_focused_and_a_hidden_inspector_keeps_a_valid_content_focus() {
    use volna_core::panels::Placement;
    let mut panels = wave_panels();
    let content = panels.focused_id();
    panels.open_settings().unwrap();
    let settings = panels.focused_id();
    panels.move_panel(content, Placement::Right).unwrap();
    panels.toggle_right_dock().unwrap();
    assert_eq!(panels.focused_id(), settings);
    let (layout, focused, saved) = panels.saved_view();
    assert_eq!(saved.len(), 1);
    assert_eq!(focused, content);
    assert!(layout.center.is_none());
    assert!(!layout.right.unwrap().open);
}

#[test]
fn settings_keep_their_region_across_panel_move_history_reset_and_restore() {
    use volna_core::panels::Placement;
    for placement in [Placement::Center, Placement::Right] {
        let mut app = linked_app();
        let content = app.panels.focused_id();
        app.handle(Command::Settings(SettingsCommand::Open));
        let settings = app.panels.settings_id().unwrap();
        panel(
            &mut app,
            PanelsCommand::Move {
                panel: settings,
                placement,
            },
        );
        panel(
            &mut app,
            PanelsCommand::Move {
                panel: content,
                placement: Placement::Right,
            },
        );
        for command in [Command::Undo, Command::Redo, Command::Undo] {
            app.handle(command);
            assert_eq!(app.panels.settings_id(), Some(settings));
            assert_eq!(app.panels.layout().placement(settings), Some(placement));
            app.panels.validate().unwrap();
        }
        let saved = volna_core::workspace::Workspace::capture(
            &app,
            volna_core::testing::paths("trace.vtr"),
            None,
        )
        .unwrap();
        app.panels.reset().unwrap();
        assert_eq!(app.panels.layout().placement(settings), Some(placement));
        saved
            .prepare(
                &app,
                "file:///tmp/trace.vtr",
                "file:///tmp/trace.vtr.volna.json",
            )
            .unwrap()
            .commit(&mut app)
            .unwrap();
        assert_eq!(app.panels.layout().placement(settings), Some(placement));
        app.panels.validate().unwrap();
    }
}

#[test]
fn auxiliary_moves_keep_navigation_state_and_share_undo_history() {
    use volna_core::panels::Placement;
    let mut app = linked_app();
    let a = app.panels.focused_id();
    panel(&mut app, PanelsCommand::NewTab { group_of: a });
    let b = app.panels.focused_id();
    let before = app.panels.layout().clone();
    panel(&mut app, PanelsCommand::MoveToNewWindow(b));
    let detached = app.panels.layout().clone();
    let Placement::Window(window) = detached.placement(b).unwrap() else {
        panic!("detached panel")
    };
    assert_eq!(detached.for_window(None).panels(), vec![a]);
    assert_eq!(detached.for_window(Some(window)).panels(), vec![b]);
    assert_eq!(app.panels.focused_id(), b);
    app.doc.shared.cursor = Some(37);
    for focused in [a, b] {
        panel(&mut app, PanelsCommand::Focus(focused));
        let now = Instant::now();
        app.handle_at(Command::Action(Action::ZoomIn), now);
        app.tick(now + std::time::Duration::from_secs(1));
        assert_eq!(
            app.panels.waves(a).unwrap().viewport(&app.doc),
            app.panels.waves(b).unwrap().viewport(&app.doc)
        );
        assert_eq!(app.panels.waves(a).unwrap().cursor(&app.doc), Some(37));
        assert_eq!(app.panels.waves(b).unwrap().cursor(&app.doc), Some(37));
    }
    panel(
        &mut app,
        PanelsCommand::ToggleLink {
            panel: b,
            dim: LinkDim::Cursor,
        },
    );
    app.doc.shared.cursor = Some(51);
    assert_eq!(app.panels.waves(a).unwrap().cursor(&app.doc), Some(51));
    assert_eq!(app.panels.waves(b).unwrap().cursor(&app.doc), Some(37));
    panel(&mut app, PanelsCommand::ReturnWindow(window));
    assert!(app.panels.layout().windows.is_empty());
    assert_eq!(app.panels.waves(b).unwrap().cursor(&app.doc), Some(37));
    assert!(!app.panels.waves(b).unwrap().nav.link.cursor);
    app.handle(Command::Undo);
    assert_eq!(app.panels.layout(), &detached);
    app.handle(Command::Redo);
    assert!(app.panels.layout().windows.is_empty());
    app.panels.validate().unwrap();
    assert_eq!(before.panels(), app.panels.layout().panels());
}

#[test]
fn auxiliary_groups_return_all_tabs_and_moves_never_reuse_window_ids() {
    use volna_core::panels::Placement;
    let mut panels = wave_panels();
    let a = panels.focused_id();
    panels.move_to_new_window(a).unwrap();
    let Placement::Window(first) = panels.layout().placement(a).unwrap() else {
        panic!()
    };
    assert!(panels.layout().center.is_none());
    let b = panels.create(a, Some(Axis::Vertical)).unwrap();
    assert_eq!(panels.layout().placement(b), Some(Placement::Window(first)));
    panels.move_panel(a, Placement::Center).unwrap();
    panels.move_to_new_window(a).unwrap();
    let Placement::Window(second) = panels.layout().placement(a).unwrap() else {
        panic!()
    };
    assert_ne!(first, second);
    panels.return_window(first).unwrap();
    panels.return_window(second).unwrap();
    assert_eq!(panels.layout().panels(), vec![b, a]);
    assert!(panels.layout().windows.is_empty());
    panels.validate().unwrap();
}

#[test]
fn window_allocation_reserves_ids_introduced_by_layout_edits() {
    use volna_core::panels::{Placement, WindowId};

    let mut panels = wave_panels();
    let a = panels.focused_id();
    let b = panels.create(a, None).unwrap();
    let first = WindowId(1);
    panels.move_panel(a, Placement::Window(first)).unwrap();
    panels.move_to_new_window(b).unwrap();
    assert_ne!(panels.layout().placement(b), Some(Placement::Window(first)));
    assert_eq!(panels.layout().windows.len(), 2);

    let reserved = WindowId(100);
    let mut layout = panels.layout().clone();
    let tree = layout.windows.remove(&first).unwrap();
    layout.windows.insert(reserved, tree);
    panels.set_layout(layout, panels.revision()).unwrap();
    panels.return_window(reserved).unwrap();
    panels.move_to_new_window(a).unwrap();
    let Some(Placement::Window(next)) = panels.layout().placement(a) else {
        panic!("detached panel")
    };
    assert!(next > reserved);
    panels.validate().unwrap();
}

#[test]
fn auxiliary_layout_proposals_cannot_steal_or_duplicate_another_windows_panels() {
    use volna_core::panels::{DockLayout, Placement};
    let mut panels = wave_panels();
    let a = panels.focused_id();
    let b = panels.create(a, None).unwrap();
    panels.move_to_new_window(b).unwrap();
    let layout = panels.layout().clone();
    let Placement::Window(window) = layout.placement(b).unwrap() else {
        panic!()
    };
    assert!(layout.with_window(None, DockLayout::single(b)).is_err());
    assert!(
        layout
            .with_window(Some(window), DockLayout::single(a))
            .is_err()
    );
    let mut invalid = layout.clone();
    invalid.center = Some(Layout::single(b));
    assert!(panels.set_layout(invalid, panels.revision()).is_err());
    assert_eq!(panels.layout(), &layout);
    let local = layout.for_window(Some(window));
    assert_eq!(layout.with_window(Some(window), local).unwrap(), layout);
}

#[test]
fn closing_last_auxiliary_content_restores_a_main_start_and_undo_reopens_the_window() {
    use volna_core::panels::Placement;
    let mut app = linked_app();
    let a = app.panels.focused_id();
    panel(&mut app, PanelsCommand::MoveToNewWindow(a));
    let detached = app.panels.layout().clone();
    panel(&mut app, PanelsCommand::Close(a));
    assert!(app.panels.layout().windows.is_empty());
    let start = app.panels.first_start().unwrap();
    assert_eq!(
        app.panels.layout().placement(start),
        Some(Placement::Center)
    );
    app.handle(Command::Undo);
    assert_eq!(app.panels.layout(), &detached);
    app.panels.validate().unwrap();
}
