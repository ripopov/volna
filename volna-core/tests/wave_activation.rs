use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use volna_core::Theme;
use volna_core::app::{Action, App, Command};
use volna_core::geometry::{Modifiers, Rect};
use volna_core::panels::{PanelId, PanelKind, PanelsCommand, Placement};
use volna_core::scene::MonoMeasure;
use volna_core::sidebar::Key;
use volna_core::testing::{a, a_all};
use volna_core::trace::{TraceId, Traced};
use volna_core::wave::model::WaveRow;
use volna_core::workspace::Workspace;
use volna_trace::data::Member;
use volna_trace::session::{OpenSpec, Session};

const TRACE: &str = "file:///tmp/landing.vtr";
const LOCATION: &str = "file:///tmp/landing.vtr.volna.json";

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../volna/examples/landing.vtr")
}

fn opened() -> (App, Arc<dyn Session>) {
    let session = OpenSpec::Path(path()).open().unwrap();
    let mut app = App::new();
    app.set_session(session.clone());
    pump(&mut app);
    (app, session)
}

fn pump(app: &mut App) {
    loop {
        let requests = app.take_requests();
        if requests.is_empty() {
            return;
        }
        for request in requests {
            app.deliver(request.perform());
        }
    }
}

fn generator(session: &dyn Session, kind: &str) -> Member {
    Member::Generator(
        session
            .hierarchy()
            .generators()
            .iter()
            .position(|g| g.kind == kind)
            .unwrap(),
    )
}

fn frame(app: &mut App, panel: PanelId) {
    let theme = Theme::one_dark();
    app.layout_panel(
        panel,
        Rect::from_xywh(0.0, 0.0, 1000.0, 180.0),
        &theme,
        1.0,
        &mut MonoMeasure,
    )
    .unwrap();
    app.render_panel(panel, &theme, &mut MonoMeasure);
}

fn capture(app: &App) -> Vec<u8> {
    Workspace::capture(app, volna_core::testing::paths(TRACE), None)
        .unwrap()
        .to_bytes()
        .unwrap()
}

#[test]
fn mixed_activation_and_add_to_waves_share_order_deduplication_and_one_undo_step() {
    let (mut app, session) = opened();
    let members = a_all([
        generator(session.as_ref(), "EVENT"),
        Member::Var(0),
        generator(session.as_ref(), "CLOCK"),
        generator(session.as_ref(), "PIPELINE"),
        generator(session.as_ref(), "LOG"),
    ]);
    let start = app.panels.focused_id();
    let mut repeated = members.clone();
    repeated.extend(members.clone());
    app.handle(Command::ActivateMembers(repeated));
    let waves = app.panels.focused_id();
    pump(&mut app);
    frame(&mut app, waves);
    assert_eq!(app.panels.len(), 1);
    let w = app.panels.waves(waves).unwrap();
    assert_eq!(w.items().len(), members.len());
    assert_eq!(w.selected, (0..members.len()).collect());
    assert!(matches!(w.items()[0].row, WaveRow::Lane(_)));
    assert!(matches!(w.items()[1].row, WaveRow::Signal(_)));
    assert!(matches!(w.items()[2].row, WaveRow::Clock(_)));
    assert!(matches!(w.items()[3].row, WaveRow::Lane(_)));
    assert!(matches!(w.items()[4].row, WaveRow::Lane(_)));
    for &index in &[0, 3, 4] {
        assert!(
            w.items()[index]
                .lane()
                .unwrap()
                .generator(&app.doc)
                .is_some()
        );
    }
    let revision = app.history.revision();
    let scroll = w.scroll_y;
    app.handle(Command::AddToWaves(members.clone()));
    frame(&mut app, waves);
    assert_eq!(app.panels.waves(waves).unwrap().scroll_y, scroll);
    assert_eq!(app.history.revision(), revision);
    assert!(app.take_requests().is_empty());
    assert_eq!(
        app.panels.waves(waves).unwrap().items().len(),
        members.len()
    );
    app.handle(Command::Undo);
    assert_eq!(app.panels.focused_id(), start);
    assert!(app.panels.focused().kind.is_start());
    assert!(!app.can_undo());
    app.handle(Command::Redo);
    assert_eq!(app.panels.focused_id(), waves);
    assert_eq!(
        app.panels.waves(waves).unwrap().items().len(),
        members.len()
    );

    let mut shared = opened().0;
    shared.handle(Command::AddToWaves(members));
    pump(&mut shared);
    frame(&mut shared, waves);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&capture(&app)).unwrap(),
        serde_json::from_slice::<serde_json::Value>(&capture(&shared)).unwrap(),
    );
}

#[test]
fn activation_targets_focused_waves_then_reuses_existing_waves_from_other_panels() {
    let (mut app, session) = opened();
    app.handle(Command::ActivateMembers(vec![a(Member::Var(0))]));
    let first = app.panels.focused_id();
    app.handle(Command::Action(Action::NewPanel));
    let second = app.panels.focused_id();
    app.handle(Command::ActivateMembers(vec![a(generator(
        session.as_ref(),
        "LOG",
    ))]));
    assert_eq!(app.panels.focused_id(), second);
    assert_eq!(app.panels.waves(first).unwrap().items().len(), 1);
    assert_eq!(app.panels.waves(second).unwrap().items().len(), 1);

    let start = app.panels.open(PanelKind::Start, first, None).unwrap();
    app.handle(Command::Panels(PanelsCommand::Focus(start)));
    app.handle(Command::ActivateMembers(vec![a(Member::Var(0))]));
    assert_eq!(app.panels.focused_id(), first);
    assert!(app.panels.get(start).unwrap().kind.is_start());
    assert_eq!(app.panels.len(), 3);

    app.handle(Command::OpenTable {
        selected: vec![a(Member::Var(0))],
        clicked: None,
    });
    let table = app.panels.focused_id();
    app.handle(Command::Panels(PanelsCommand::Close(second)));
    app.handle(Command::Panels(PanelsCommand::Move {
        panel: first,
        placement: Placement::Right,
    }));
    app.handle(Command::Panels(PanelsCommand::ToggleRightDock));
    app.handle(Command::Panels(PanelsCommand::Focus(table)));
    let count = app.panels.len();
    app.handle(Command::ActivateMembers(vec![a(Member::Var(0))]));
    assert_eq!(app.panels.focused_id(), first);
    assert_eq!(app.panels.layout().placement(first), Some(Placement::Right));
    assert!(app.panels.layout().right.as_ref().unwrap().open);
    assert_eq!(app.panels.len(), count);
    app.panels.validate().unwrap();
}

#[test]
fn enter_reveals_an_existing_signal_inside_collapsed_groups_without_an_edit() {
    let (mut app, session) = opened();
    app.handle(Command::AddScopeAsGroup {
        scope: a(0),
        recursive: true,
    });
    let waves = app.panels.focused_id();
    let w = app.panels.waves(waves).unwrap();
    let (index, var) = w
        .items()
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, row)| {
            (row.depth > 1)
                .then(|| row.signal()?.source.var().map(|var| (i, var)))
                .flatten()
        })
        .unwrap();
    app.panels
        .waves_mut(waves)
        .unwrap()
        .fold_all(&app.doc, true);
    frame(&mut app, waves);
    let count = app.panels.waves(waves).unwrap().items().len();
    let revision = app.history.revision();
    let name = session.hierarchy().var_name(var.item);
    app.handle(Command::SetSearchEverywhere(true));
    app.handle(Command::SetFilter(name.into()));
    let ix = app
        .variables
        .rows
        .iter()
        .position(|m| *m == var.with(Member::Var(var.item)))
        .unwrap();
    app.handle(Command::SelectVar {
        ix,
        modifiers: Modifiers::default(),
    });
    app.handle(Command::VariablesKey(Key::Enter, Modifiers::default()));
    frame(&mut app, waves);
    let w = app.panels.waves(waves).unwrap();
    assert_eq!(w.items().len(), count);
    assert_eq!(w.selected, BTreeSet::from([index]));
    assert_eq!(app.history.revision(), revision);
    assert!(w.visible().contains(&(index as u32)));
    let visible_index = w
        .visible()
        .iter()
        .position(|&i| i as usize == index)
        .unwrap();
    let layout = w.last_layout();
    assert!(layout.row_y(visible_index) >= layout.names.top());
    assert!(
        layout.row_y(visible_index) + layout.row_height(visible_index) <= layout.names.bottom()
    );
}

#[test]
fn explicit_views_and_restored_layout_survive_default_activation() {
    let (mut app, session) = opened();
    let member = a(generator(session.as_ref(), "PIPELINE"));
    let track = member.with(session.hierarchy().member_track(member.item).unwrap());
    app.handle(Command::OpenPipeline { track });
    let pipeline = app.panels.focused_id();
    assert!(app.panels.pipeline(pipeline).is_some());
    app.handle(Command::OpenTable {
        selected: vec![member],
        clicked: None,
    });
    let table = app.panels.focused_id();
    assert!(app.panels.get(table).unwrap().kind.table().is_some());
    app.handle(Command::ActivateMembers(vec![member]));
    let waves = app.panels.focused_id();
    let count = app.panels.len();
    pump(&mut app);
    frame(&mut app, waves);
    let saved = capture(&app);
    let plan = Workspace::parse(&saved)
        .unwrap()
        .prepare(&app, TRACE, LOCATION)
        .unwrap();
    assert!(plan.report().notices.is_empty());
    plan.commit(&mut app).unwrap();
    assert_eq!(capture(&app), saved);
    app.handle(Command::Panels(PanelsCommand::Focus(pipeline)));
    app.handle(Command::ActivateMembers(vec![member]));
    assert_eq!(app.panels.focused_id(), waves);
    assert_eq!(app.panels.len(), count);
    assert_eq!(app.panels.waves(waves).unwrap().items().len(), 1);
    assert!(app.panels.get(table).unwrap().kind.table().is_some());
    app.handle(Command::OpenPipeline { track });
    assert_eq!(app.panels.focused_id(), pipeline);
    assert_eq!(app.panels.len(), count);
}

#[test]
fn identical_members_from_different_traces_remain_distinct() {
    let (mut app, session) = opened();
    app.handle(Command::AddTrace(OpenSpec::Path(path())));
    pump(&mut app);
    let second = TraceId::new(1).unwrap();
    let generator = generator(session.as_ref(), "LOG");
    let members = vec![
        a(Member::Var(0)),
        Traced::new(second, Member::Var(0)),
        a(generator),
        Traced::new(second, generator),
    ];
    app.handle(Command::ActivateMembers(members.clone()));
    let waves = app.panels.focused_id();
    pump(&mut app);
    app.handle(Command::AddToWaves(members));
    assert_eq!(app.panels.waves(waves).unwrap().items().len(), 4);
    assert_eq!(app.panels.len(), 1);
}

#[test]
fn mixed_existing_and_new_rows_restore_selection_through_undo_and_redo() {
    let (mut app, session) = opened();
    app.handle(Command::AddVars(a_all([0, 1])));
    let waves = app.panels.focused_id();
    {
        let w = app.panels.waves_mut(waves).unwrap();
        w.selected = BTreeSet::from([1]);
        w.anchor = Some(1);
    }
    let members = a_all([
        generator(session.as_ref(), "PIPELINE"),
        Member::Var(0),
        generator(session.as_ref(), "CLOCK"),
    ]);
    app.handle(Command::ActivateMembers(members.clone()));
    let w = app.panels.waves(waves).unwrap();
    assert_eq!(w.items().len(), 4);
    assert_eq!(w.selected, BTreeSet::from([0, 2, 3]));
    assert_eq!(w.anchor, Some(2));
    app.handle(Command::Undo);
    let w = app.panels.waves(waves).unwrap();
    assert_eq!(w.items().len(), 2);
    assert_eq!(w.selected, BTreeSet::from([1]));
    assert_eq!(w.anchor, Some(1));
    app.handle(Command::Redo);
    let w = app.panels.waves(waves).unwrap();
    assert_eq!(w.items().len(), 4);
    assert_eq!(w.selected, BTreeSet::from([0, 2, 3]));
    assert_eq!(w.anchor, Some(2));
    let revision = app.history.revision();
    app.handle(Command::AddToWaves(members));
    assert_eq!(app.history.revision(), revision);
    assert_eq!(app.panels.len(), 1);
}

#[test]
fn a_slice_does_not_hide_the_full_signal_from_activation() {
    let (mut app, session) = opened();
    let var = session
        .hierarchy()
        .vars()
        .position(|v| v.shape.width() > 2)
        .unwrap();
    app.handle(Command::AddVars(a_all([var])));
    let waves = app.panels.focused_id();
    app.panels
        .waves_mut(waves)
        .unwrap()
        .add_slice(&app.doc, 0, "1:0")
        .unwrap();
    {
        let w = app.panels.waves_mut(waves).unwrap();
        w.selected = BTreeSet::from([0]);
        w.anchor = Some(0);
    }
    app.handle(Command::Action(Action::RemoveSelected));
    app.handle(Command::ActivateMembers(vec![a(Member::Var(var))]));
    let w = app.panels.waves(waves).unwrap();
    assert_eq!(w.items().len(), 2);
    assert!(w.items()[0].signal().unwrap().projection.is_some());
    assert!(w.items()[1].signal().unwrap().projection.is_none());
    app.handle(Command::ActivateMembers(vec![a(Member::Var(var))]));
    let w = app.panels.waves(waves).unwrap();
    assert_eq!(w.items().len(), 2);
    assert_eq!(w.selected, BTreeSet::from([1]));
}

#[test]
fn scope_addition_includes_every_member_kind_and_preserves_group_depths() {
    fn member(app: &App, row: &WaveRow) -> Traced<Member> {
        let track = match row {
            WaveRow::Signal(signal) => {
                let var = signal.source.var().unwrap();
                return var.with(Member::Var(var.item));
            }
            WaveRow::Lane(lane) => lane.track().unwrap(),
            WaveRow::Clock(clock) => app.doc.clocks.find(&clock.key).unwrap().generator(),
            WaveRow::Group(_) => panic!("expected a track row"),
        };
        let generator = app
            .doc
            .hierarchy(track.trace)
            .unwrap()
            .generators()
            .iter()
            .position(|g| g.track == track.item)
            .unwrap();
        track.with(Member::Generator(generator))
    }

    let (_, session) = opened();
    let h = session.hierarchy();
    let mut kinds = BTreeSet::new();
    let mut generator_only = false;
    for id in 0..h.scopes().len() {
        let scope = h.scope(id);
        if !scope.generators.is_empty() && scope.vars.is_empty() {
            assert!(h.has_members(id));
            generator_only = true;
        }
        for recursive in [false, true] {
            let mut pending = vec![(id, 0)];
            let mut expected = Vec::new();
            while let Some((id, depth)) = pending.pop() {
                let scope = h.scope(id);
                expected.extend(scope.vars.iter().map(|v| (Member::Var(v), depth + 1)));
                expected.extend(scope.generators.iter().map(|g| {
                    kinds.insert(h.generators()[g].kind.clone());
                    (Member::Generator(g), depth + 1)
                }));
                if recursive {
                    pending.extend(scope.children.iter().rev().map(|c| (c, depth + 1)));
                }
            }
            let (mut reference, _) = opened();
            reference.handle(Command::AddToWaves(a_all(expected.iter().map(|(m, _)| *m))));
            let rows: Vec<_> = reference
                .panels
                .focused_waves()
                .unwrap()
                .items()
                .iter()
                .map(|entry| {
                    (
                        member(&reference, &entry.row),
                        std::mem::discriminant(&entry.row),
                    )
                })
                .collect();
            assert_eq!(
                rows.iter().map(|(member, _)| *member).collect::<Vec<_>>(),
                a_all(expected.iter().map(|(member, _)| *member))
            );
            for grouped in [false, true] {
                if !grouped && !recursive {
                    continue;
                }
                let (mut app, _) = opened();
                app.handle(if grouped {
                    Command::AddScopeAsGroup {
                        scope: a(id),
                        recursive,
                    }
                } else {
                    Command::AddScopeFlat(a(id))
                });
                pump(&mut app);
                let items = app.panels.focused_waves().unwrap().items();
                let actual: Vec<_> = items
                    .iter()
                    .filter(|e| !e.is_group())
                    .map(|e| (member(&app, &e.row), std::mem::discriminant(&e.row)))
                    .collect();
                assert_eq!(actual, rows);
                let depths: Vec<_> = items
                    .iter()
                    .filter(|e| !e.is_group())
                    .map(|e| e.depth)
                    .collect();
                assert_eq!(
                    depths,
                    expected
                        .iter()
                        .map(|(_, depth)| if grouped { *depth as u8 } else { 0 })
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    assert!(generator_only);
    for kind in ["EVENT", "CLOCK", "PIPELINE", "LOG"] {
        assert!(kinds.contains(kind));
    }
}
