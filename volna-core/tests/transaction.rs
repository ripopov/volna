//! Headless transaction panel tests for selection, loading, history, pinning, and
//! persistence.

use std::path::PathBuf;
use std::sync::Arc;
use volna_core::testing::{a, complete_pending};

use volna_core::app::{Action, App, Command};
use volna_core::data::text::Radix;
use volna_core::geometry::{Modifiers, MouseButton, Rect, point};
use volna_core::panels::PanelId;
use volna_core::pipeline::{FollowActivity, Rows};
use volna_core::scene::MonoMeasure;
use volna_core::transaction::{RefRole, SectionKey, TransactionCommand, TxPanelState, TxView};
use volna_core::wave::PointerEvent;
use volna_core::wave::model::{MenuAction, WaveMenuKind};
use volna_core::wave::viewport::Viewport;
use volna_core::workspace::Workspace;
use volna_core::{Instant, Theme};
use volna_trace::data::transactions::{Status, TrackRef, TransactionRef};
use volna_trace::session::{OpenSpec, Session};

fn example(name: &str) -> Arc<dyn Session> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../volna/examples")
        .join(name);
    OpenSpec::Path(path).open().unwrap()
}

fn frame(app: &mut App, id: PanelId) {
    let theme = Theme::one_dark();
    app.layout_panel(
        id,
        Rect::from_xywh(0.0, 0.0, 1200.0, 600.0),
        &theme,
        1.0,
        &mut MonoMeasure,
    )
    .unwrap();
    app.render_panel(id, &theme, &mut MonoMeasure);
}

fn track(session: &dyn Session, path: &str) -> TrackRef {
    session
        .tracks()
        .iter()
        .find(|t| t.path.join(".") == path || t.path[..t.path.len() - 1].join(".") == path)
        .unwrap_or_else(|| panic!("no track {path}"))
        .id
}

fn labelled(session: &dyn Session, track: TrackRef, label: &str) -> TransactionRef {
    use volna_trace::data::transactions::AttributeValue;
    let loaded = session.load_track(track).unwrap();
    loaded
        .generators
        .iter()
        .flat_map(|g| g.transactions())
        .find(|t| {
            t.attributes.iter().any(|a| {
                a.key == "vtr.label"
                    && matches!(&a.value, AttributeValue::Text(text) if text == label)
            })
        })
        .unwrap_or_else(|| panic!("no record labelled {label:?}"))
        .id
}

fn child(session: &dyn Session, track: TrackRef, parent: TransactionRef) -> TransactionRef {
    session.load_track(track).unwrap().generators[0]
        .transactions()
        .iter()
        .find(|tx| tx.parent == Some(parent))
        .expect("child record")
        .id
}

fn with_pipeline(name: &str, stream: &str) -> (App, Arc<dyn Session>, PanelId) {
    let session = example(name);
    let mut app = App::new();
    app.set_session(session.clone());
    app.handle(Command::OpenPipeline {
        track: a(track(session.as_ref(), stream)),
    });
    let pipeline = app.panels.focused_id();
    complete_pending(&mut app);
    app.handle(Command::PipelineActivity(
        pipeline,
        volna_core::pipeline::ActivityCommand::Toggle,
    ));
    app.panels
        .pipeline_mut(pipeline)
        .unwrap()
        .rows
        .set(volna_core::pipeline::RowView::default());
    frame(&mut app, pipeline);
    (app, session, pipeline)
}

fn view(app: &App, panel: PanelId) -> TxView {
    match app.panels.transaction(panel).unwrap().state(&app.doc) {
        TxPanelState::Ready(view) => *view,
        TxPanelState::Empty => panic!("empty"),
        TxPanelState::Loading => panic!("loading"),
        TxPanelState::Missing => panic!("missing"),
        TxPanelState::Failed(e) | TxPanelState::Refused(e) => panic!("{e}"),
    }
}

fn select(app: &mut App, panel: PanelId, track: TrackRef, id: u64) {
    app.handle(Command::SelectTransaction {
        panel,
        track: a(track),
        id: TransactionRef(id),
        cursor: None,
    });
}

fn show(app: &mut App, pipeline: PanelId, track: TrackRef, id: u64) -> PanelId {
    select(app, pipeline, track, id);
    app.handle(Command::ShowTransaction { from: pipeline });
    let panel = app.panels.focused_id();
    assert!(app.panels.transaction(panel).is_some());
    panel
}

#[test]
fn located_and_embedded_activity_share_the_instruction_view_and_relation_navigation() {
    use vtr::{Recorder, SourceOptions};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mixed-activity.vtr");
    let mut writer = vtr::Writer::create(&path).unwrap();
    let instructions = writer
        .transactions::<()>(SourceOptions::new("instructions", None), "INSTRUCTION")
        .unwrap();
    let fetch = writer
        .stages::<()>(SourceOptions::new("fetch", None))
        .unwrap();
    let stages = writer
        .stages::<u32>(SourceOptions::new("execute", None))
        .unwrap();
    let events = writer
        .events::<()>(SourceOptions::new("retired", None))
        .unwrap();
    let name = writer.intern("decode").unwrap();
    let edge = writer.intern("causes").unwrap();
    let owner = writer.begin_at(instructions, 10, &(), None).unwrap();
    let fetched = writer
        .begin_stage_at(fetch, owner.owner(), 10, &())
        .unwrap();
    writer.end_stage_at(fetched, 11).unwrap();
    writer
        .embedded_stage(owner.owner(), 11, name, 0, Some(12), vec![])
        .unwrap();
    writer
        .embedded_event(owner.owner(), 12, name, vec![])
        .unwrap();
    let visit = writer
        .begin_stage_at(stages, owner.owner(), 12, &42)
        .unwrap();
    writer.end_stage_at(visit, 14).unwrap();
    writer.event_at(events, owner.owner(), 14, &()).unwrap();
    let child = writer.begin_at(instructions, 20, &(), None).unwrap();
    writer.end_at(child, 25, Status::Ok).unwrap();
    writer.relate(edge, visit.id(), child.id(), vec![]).unwrap();
    writer.end_at(owner, 15, Status::Ok).unwrap();
    writer.close().unwrap();

    let mut app = App::new();
    app.set_session(OpenSpec::Path(path).open().unwrap());
    let track = TrackRef(instructions.id());
    app.handle(Command::OpenPipeline { track: a(track) });
    let pipeline = app.panels.focused_id();
    complete_pending(&mut app);
    frame(&mut app, pipeline);
    let panel = show(&mut app, pipeline, track, owner.id());
    complete_pending(&mut app);
    let projection = view(&app, panel);
    assert_eq!(projection.stages.total, 3);
    assert_eq!(projection.events.total, 2);
    assert_eq!(
        projection
            .stages
            .rows
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["fetch", "decode", "execute"]
    );
    assert_eq!(
        projection
            .events
            .rows
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["decode", "retired"]
    );
    assert!(projection.stage_details.is_none());
    assert!(projection.stages.rows.iter().all(|row| !row.selected));
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::SelectStage(TransactionRef(visit.id())),
    ));
    let selected = view(&app, panel);
    assert_eq!(app.doc.shared.cursor, Some(12));
    assert!(selected.stages.rows[2].selected);
    let details = selected.stage_details.unwrap();
    assert_eq!(details.name, "execute");
    assert_eq!(details.attributes.total, 1);
    assert_eq!(details.attributes.rows[0].key, "payload");
    assert_eq!(details.attributes.rows[0].value, "42");
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::SelectStage(TransactionRef(fetched.id())),
    ));
    let selected = view(&app, panel);
    assert_eq!(app.doc.shared.cursor, Some(10));
    assert!(selected.stages.rows[0].selected);
    assert!(!selected.stages.rows[2].selected);
    let details = selected.stage_details.unwrap();
    assert_eq!(details.name, "fetch");
    assert_eq!(details.attributes.total, 0);
    assert!(details.attributes.rows.is_empty());
    assert!(
        projection
            .events
            .rows
            .iter()
            .all(|event| event.attributes.rows.is_empty() && event.attributes.total == 0)
    );
    assert_eq!(projection.lifeline.len(), 1);
    assert!(projection.lifeline[0].primary);
    assert_eq!(projection.lifeline[0].lane, "0");
    assert_eq!(projection.lifeline[0].cells.len(), 3);
    for stage in &projection.stages.rows {
        assert!(stage.primary);
        assert!(matches!(
            stage.swatch,
            volna_core::pipeline::palette::StageSwatch::Step { of: 3, .. }
        ));
    }
    assert!(
        projection
            .stages
            .rows
            .windows(2)
            .all(|stages| stages[0].swatch != stages[1].swatch)
    );
    assert_eq!(
        projection
            .event_ticks
            .iter()
            .map(|tick| tick.time)
            .collect::<Vec<_>>(),
        [12, 14]
    );
    let outgoing = projection
        .related
        .rows
        .iter()
        .find(|row| row.role == RefRole::Relation { outgoing: true })
        .unwrap();
    assert_eq!(outgoing.target.transaction, TransactionRef(child.id()));
    frame(&mut app, pipeline);

    let child_panel = show(&mut app, pipeline, track, child.id());
    let child_view = view(&app, child_panel);
    assert!(child_view.stage_details.is_none());
    let incoming = child_view
        .related
        .rows
        .iter()
        .find(|row| row.role == RefRole::Relation { outgoing: false })
        .unwrap();
    assert_eq!(incoming.target.transaction, TransactionRef(visit.id()));
    assert_eq!(incoming.target.generator, Some(TrackRef(stages.id())));
    app.handle(Command::Transaction(child_panel, TransactionCommand::Back));
    let previous = view(&app, child_panel);
    assert_eq!(previous.identity.id, TransactionRef(owner.id()));
    assert!(previous.stage_details.is_none());
    assert!(previous.stages.rows.iter().all(|row| !row.selected));
}

#[test]
fn pipeline_label_goes_to_transaction_start_without_changing_scale() {
    let (mut app, session, panel) = with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
    let instruction = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
    let add = labelled(session.as_ref(), instruction, "80000008: add  a3, a1, a0");
    let (row, begin, label) = {
        let pipeline = app.panels.pipeline(panel).unwrap();
        let Rows::Ready(set) = pipeline.rows(&app.doc) else {
            panic!("not loaded")
        };
        let row = set.row_of(instruction, add).unwrap();
        let (_, tx) = set.get(row).unwrap();
        let layout = pipeline.last_layout();
        assert!(layout.row_range.contains(&row));
        (
            row,
            tx.begin,
            point(layout.labels.left() + 20.0, layout.row_y(row) + 4.0),
        )
    };
    let initial = Viewport {
        start: 20.0,
        end: 60.0,
    };
    app.doc.navigation.animation = volna_core::settings::Animation::Off;
    app.panels
        .pipeline_mut(panel)
        .unwrap()
        .nav
        .viewport_state_mut(&mut app.doc)
        .set(initial);
    let row_position = app.panels.pipeline(panel).unwrap().rows.target();
    app.handle(Command::Pointer(
        panel,
        PointerEvent::Down {
            position: label,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    assert_eq!(
        app.panels.pipeline(panel).unwrap().selected_row(&app.doc),
        Some(row)
    );
    app.panels.pipeline_mut(panel).unwrap().follow = FollowActivity::Following;
    app.handle(Command::PanToTransactionStart { panel, row });
    let pipeline = app.panels.pipeline(panel).unwrap();
    let viewport = pipeline.nav.viewport(&app.doc);
    assert!((viewport.width() - initial.width()).abs() < 1e-9);
    assert!(
        (viewport.x_of_tick(begin, pipeline.last_layout().cells_width_f64()) - 32.0).abs() < 1e-6
    );
    assert_eq!(pipeline.rows.target(), row_position);
    assert_eq!(pipeline.follow, FollowActivity::Suspended);
    assert_eq!(pipeline.selected_row(&app.doc), Some(row));
    assert_eq!(app.panels.iter().count(), 2);

    app.panels
        .pipeline_mut(panel)
        .unwrap()
        .nav
        .viewport_state_mut(&mut app.doc)
        .set(initial);
    app.handle(Command::Pointer(
        panel,
        PointerEvent::Down {
            position: label,
            button: MouseButton::Right,
            modifiers: Modifiers::default(),
        },
    ));
    let menu = app.menu().unwrap();
    assert_eq!(menu.kind, WaveMenuKind::PipelineLabel);
    assert_eq!(
        menu.items().next().unwrap().action,
        MenuAction::PipelineStart(row)
    );
    app.handle(Command::MenuSelect(panel, MenuAction::PipelineStart(row)));
    let viewport = app.panels.pipeline(panel).unwrap().nav.viewport(&app.doc);
    assert!(
        (viewport.x_of_tick(
            begin,
            app.panels
                .pipeline(panel)
                .unwrap()
                .last_layout()
                .cells_width_f64()
        ) - 32.0)
            .abs()
            < 1e-6
    );
    assert!(app.menu().is_none());
}

#[test]
fn a_click_selects_the_row_enter_shows_it_and_every_reader_follows() {
    let (mut app, session, pipeline) = with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
    let insn = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
    let add = labelled(session.as_ref(), insn, "80000008: add  a3, a1, a0");
    let (row, position, cycle) = {
        let p = app.panels.pipeline(pipeline).unwrap();
        let Rows::Ready(set) = p.rows(&app.doc) else {
            panic!("not loaded")
        };
        let row = set.row_of(insn, add).unwrap();
        let layout = p.last_layout();
        assert!(layout.row_range.contains(&row));
        let vp = p.nav.viewport(&app.doc);
        let x = layout.cells.left() + vp.x_of(6.5, layout.cells_width_f64()) as f32;
        (
            row,
            point(x, layout.row_y(row) + layout.rows.row_px / 2.0),
            6,
        )
    };
    app.handle(Command::Pointer(
        pipeline,
        PointerEvent::Down {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    app.handle(Command::Pointer(pipeline, PointerEvent::Up));
    let selection = app.doc.selection().unwrap();
    assert_eq!((selection.track, selection.id), (a(insn), add));
    assert_eq!(selection.origin, pipeline);
    assert_eq!(app.doc.shared.cursor, Some(cycle));
    let p = app.panels.pipeline(pipeline).unwrap();
    assert_eq!(p.selected_row(&app.doc), Some(row));

    frame(&mut app, pipeline);
    let theme = Theme::one_dark();
    let arrows = app
        .scene()
        .prims
        .iter()
        .filter(|p| {
            matches!(p, volna_core::scene::Prim::Lines { color, .. }
                if *color == theme.tx_relation_in || *color == theme.tx_relation_out)
        })
        .count();
    assert_eq!(arrows, 2, "one batch into the row and one out of it");

    app.handle(Command::ShowTransaction { from: pipeline });
    let panel = app.panels.focused_id();
    let v = view(&app, panel);
    assert_eq!(v.identity.id, add);
    assert_eq!(
        v.identity.label.as_deref(),
        Some("80000008: add  a3, a1, a0")
    );
    app.handle(Command::ShowTransaction { from: pipeline });
    assert_eq!(app.panels.focused_id(), panel);
    let count = app
        .panels
        .iter()
        .filter(|p| p.kind.transaction().is_some())
        .count();
    assert_eq!(count, 1);

    app.handle(Command::Panels(volna_core::panels::PanelsCommand::Focus(
        pipeline,
    )));
    app.handle(Command::Action(Action::MoveSelectionDown));
    let p = app.panels.pipeline(pipeline).unwrap();
    assert_eq!(p.selected_row(&app.doc), Some(row + 1));
    assert_ne!(view(&app, panel).identity.id, add);

    app.handle(Command::Action(Action::ClearSelection));
    assert_eq!(app.doc.selection(), None);
    assert!(app.doc.shared.cursor.is_some());
    assert!(matches!(
        app.panels.transaction(panel).unwrap().state(&app.doc),
        TxPanelState::Ready(_)
    ));
    app.handle(Command::Action(Action::ClearSelection));
    assert_eq!(app.doc.shared.cursor, None);
}

#[test]
fn the_view_reads_identity_timing_lifeline_stages_and_relations() {
    let (mut app, session, pipeline) = with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
    let insn = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
    let load = labelled(session.as_ref(), insn, "80000060: lw   a1, 0(a2)");
    let request = child(
        session.as_ref(),
        track(session.as_ref(), "soc.l2.bus.read"),
        load,
    );
    let producer = labelled(session.as_ref(), insn, "80000050: addi a2, a2, 16");
    let dependent = labelled(session.as_ref(), insn, "80000068: add  a3, a1, a0");
    let panel = show(&mut app, pipeline, insn, load.0);
    let v = view(&app, panel);
    assert_eq!(v.identity.scope, ["soc", "cpu0", "pipeline"]);
    assert_eq!(v.identity.generator, "instruction");
    assert_eq!(
        v.identity.label.as_deref(),
        Some("80000060: lw   a1, 0(a2)")
    );
    assert_eq!(v.identity.status, Status::Unset);
    assert_eq!(v.identity.status_text, "ended");
    assert_eq!((v.timing.begin, v.timing.end), (33, Some(47)));
    assert_eq!(v.timing.duration, 14);
    assert!(
        v.timing.begin_text.contains("cycle"),
        "{}",
        v.timing.begin_text
    );

    assert_eq!(v.lifeline.len(), 1);
    let lane = &v.lifeline[0];
    assert!(lane.primary);
    let names: Vec<_> = lane.cells.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "fetch_buffer",
            "decode_register",
            "execute_register",
            "memory_register",
            "writeback_register"
        ]
    );
    let palette = app.panels.pipeline(pipeline).unwrap().palette();
    assert_eq!(lane.cells[3].swatch, palette.swatch("memory_register"));
    assert!((lane.cells[3].width - 10.0 / 14.0).abs() < 1e-4);

    let keys: Vec<_> = v.attributes.rows.iter().map(|a| a.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "record.kind",
            "payload.insn_id",
            "payload.pc",
            "payload.iteration"
        ]
    );
    assert_eq!(v.attributes.total, 4);
    let pc = v
        .attributes
        .rows
        .iter()
        .find(|a| a.key == "payload.pc")
        .unwrap();
    assert_eq!(
        (pc.value.as_str(), pc.radix),
        ("2147483744", Some(Radix::Dec))
    );
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Radix("payload.pc".into()),
    ));
    assert_eq!(
        view(&app, panel)
            .attributes
            .rows
            .iter()
            .find(|a| a.key == "payload.pc")
            .unwrap()
            .value,
        "0x80000060"
    );

    let m = v
        .stages
        .rows
        .iter()
        .find(|s| s.name == "memory_register")
        .unwrap();
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::SelectStage(m.id),
    ));
    assert!(
        view(&app, panel)
            .stage_details
            .unwrap()
            .attributes
            .rows
            .iter()
            .any(|c| c.key == "payload.miss" && c.value == "true")
    );
    assert!((m.share - 10.0 / 14.0).abs() < 1e-4);

    let related: Vec<_> = v
        .related
        .rows
        .iter()
        .map(|r| (r.group.as_str(), r.target.transaction.0, r.loaded))
        .collect();
    assert_eq!(
        related,
        [
            ("causes · to", request.0, false),
            ("wakeup · to", dependent.0, true),
            ("wakeup · from", producer.0, true)
        ]
    );
    let caused = &v.related.rows[0];
    assert_eq!(caused.role, RefRole::Relation { outgoing: true });
    assert_eq!(
        caused.track.as_deref(),
        Some(&["soc".to_owned(), "l2".into(), "bus".into(), "read".into()][..])
    );
    assert_eq!(
        caused.relation_label.as_deref(),
        Some("instruction 24 causes request")
    );

    app.handle(Command::Transaction(panel, TransactionCommand::Cursor(36)));
    assert_eq!(app.doc.shared.cursor, Some(36));
    assert!(
        view(&app, panel)
            .cursor
            .is_some_and(|c| (c - 3.0 / 14.0).abs() < 1e-4)
    );
    app.handle_at(
        Command::Transaction(panel, TransactionCommand::FitLifetime),
        Instant::now(),
    );
    let target = app.doc.shared.viewport.target();
    assert!(target.start <= 33.0 && target.end >= 47.0 && target.width() < 20.0);

    let tsv = app
        .panels
        .transaction(panel)
        .unwrap()
        .copy_tsv(&app.doc)
        .unwrap();
    assert!(tsv.starts_with("Generator\tID\tBegin\tEnd\tDuration\tStatus\tKind\n"));
    assert!(
        tsv.lines().any(|line| {
            line.starts_with("memory_register\t0\t36\t46\t") && line.contains("payload.miss=true")
        }),
        "{tsv}"
    );
}

#[test]
fn a_jump_loads_the_target_track_back_returns_and_reveal_finds_the_row() {
    let (mut app, session, pipeline) = with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
    let insn = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
    let load = labelled(session.as_ref(), insn, "80000060: lw   a1, 0(a2)");
    let request = child(
        session.as_ref(),
        track(session.as_ref(), "soc.l2.bus.read"),
        load,
    );
    let read = track(session.as_ref(), "soc.l2.bus.read");
    let panel = show(&mut app, pipeline, insn, load.0);
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Jump {
            track: a(read),
            id: request,
        },
    ));
    assert!(matches!(
        app.panels.transaction(panel).unwrap().state(&app.doc),
        TxPanelState::Loading
    ));
    complete_pending(&mut app);
    let v = view(&app, panel);
    assert_eq!(v.identity.label.as_deref(), Some("read 0x10000020"));
    assert_eq!(v.identity.parent, Some(load));
    assert_eq!(v.related.rows[0].role, RefRole::Parent);
    assert_eq!(v.related.rows[0].target.transaction, load);
    assert!(v.related.rows[0].loaded);
    assert_eq!(app.doc.selection().unwrap().id, request);
    assert_eq!(
        app.panels
            .pipeline(pipeline)
            .unwrap()
            .selected_row(&app.doc),
        None
    );
    let model = app.panels.transaction(panel).unwrap();
    assert!(model.can_go_back() && !model.can_go_forward());

    app.handle(Command::Transaction(panel, TransactionCommand::Back));
    let v = view(&app, panel);
    assert_eq!(v.identity.id, load);
    assert!(!v.related.rows.iter().any(|r| r.role == RefRole::Child));
    let focused = app.panels.focused_id();
    app.handle(Command::OpenPipeline {
        track: a(track(session.as_ref(), "soc.l2.bus")),
    });
    complete_pending(&mut app);
    app.handle(Command::Panels(volna_core::panels::PanelsCommand::Focus(
        focused,
    )));
    let v = view(&app, panel);
    assert!(
        v.related
            .rows
            .iter()
            .any(|r| r.role == RefRole::Child && r.target.transaction == request && r.loaded)
    );
    assert_eq!(app.doc.selection().unwrap().id, load);
    assert!(app.panels.transaction(panel).unwrap().can_go_forward());

    app.handle_at(Command::RevealTransaction { panel }, Instant::now());
    assert_eq!(app.panels.focused_id(), pipeline);
    frame(&mut app, pipeline);
    let p = app.panels.pipeline(pipeline).unwrap();
    let row = p.selected_row(&app.doc).unwrap();
    assert!(p.last_layout().row_range.contains(&row));

    app.handle(Command::Panels(volna_core::panels::PanelsCommand::Close(
        pipeline,
    )));
    assert_eq!(view(&app, panel).identity.id, load);
    app.handle(Command::RevealTransaction { panel });
    assert!(app.panels.pipeline(app.panels.focused_id()).is_some());
}

#[test]
fn a_pinned_panel_keeps_its_record_and_the_next_one_opens_in_the_right_dock() {
    let (mut app, session, pipeline) = with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
    let insn = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
    let add = labelled(session.as_ref(), insn, "80000008: add  a3, a1, a0");
    let load = labelled(session.as_ref(), insn, "80000060: lw   a1, 0(a2)");
    let first = show(&mut app, pipeline, insn, add.0);
    app.handle(Command::Transaction(first, TransactionCommand::Pin(true)));
    let second = show(&mut app, pipeline, insn, load.0);
    assert_ne!(first, second);
    assert_eq!(
        app.panels.layout().right.as_ref().unwrap().layout.panels(),
        vec![first, second]
    );
    assert_eq!(
        app.panels.layout().center.as_ref().unwrap().panels(),
        vec![pipeline]
    );
    assert_eq!(view(&app, first).identity.id, add);
    assert_eq!(view(&app, second).identity.id, load);
    app.handle(Command::Transaction(first, TransactionCommand::Pin(false)));
    assert_eq!(view(&app, first).identity.id, load);
    assert!(app.debug_state().contains("transaction"));
}

#[test]
fn every_value_tag_an_open_record_and_a_cross_stream_parent_read_as_recorded() {
    let (mut app, session, pipeline) = with_pipeline("feature_showcase.vtr", "soc.dma.memory_bus");
    let read = track(session.as_ref(), "soc.dma.memory_bus.read");
    let write = track(session.as_ref(), "soc.dma.memory_bus.write");
    let transfer = labelled(session.as_ref(), read, "DMA transfer 0");
    let submission = labelled(
        session.as_ref(),
        track(session.as_ref(), "firmware.driver.submit_transfer"),
        "submission 0",
    );
    let instruction = labelled(
        session.as_ref(),
        track(session.as_ref(), "soc.cpu.thread0.instructions"),
        "0x80000004 sw zero,8(s1)",
    );
    let unfinished = labelled(session.as_ref(), write, "unfinished write 0");
    let panel = show(&mut app, pipeline, read, transfer.0);
    app.panels
        .transaction_mut(panel)
        .unwrap()
        .set_detail_items(26);
    let v = view(&app, panel);
    assert_eq!(v.identity.kind, Some("consumer"));
    assert_eq!(v.identity.status_text, "ok");
    assert_eq!(v.attributes.total, 26);
    let kinds: Vec<_> = v.attributes.rows.iter().map(|a| a.kind).collect();
    for kind in [
        "null", "bool", "int", "real", "text", "logic", "time", "enum", "pointer", "fixed", "list",
        "map",
    ] {
        assert!(kinds.contains(&kind), "{kind} missing from {kinds:?}");
    }
    for (key, value) in [
        ("attributes.signed_i8", "-8"),
        ("attributes.signed_i16", "-16"),
        ("attributes.signed_i32", "-32"),
        ("attributes.unsigned_u8", "8"),
        ("attributes.unsigned_u16", "16"),
        ("attributes.unsigned_u32", "32"),
        ("attributes.float32", "0.25"),
    ] {
        assert_eq!(
            v.attributes
                .rows
                .iter()
                .find(|a| a.key == key)
                .unwrap_or_else(|| panic!("missing {key}: {:?}", v.attributes.rows))
                .value,
            value,
            "{key}"
        );
    }
    let message = v
        .attributes
        .rows
        .iter()
        .find(|a| a.key == "attributes.message")
        .unwrap();
    assert_eq!(message.value, "DMA → memory: café ✓");
    assert_eq!(v.events.rows.len(), 1);
    assert_eq!(
        (v.events.rows[0].name.as_str(), v.events.rows[0].time),
        ("request", 104)
    );
    assert_eq!(v.event_ticks.len(), 1);
    assert_eq!(v.stages.rows[0].lane, "memory");
    let parent = &v.related.rows[0];
    assert_eq!(parent.role, RefRole::Parent);
    assert_eq!(parent.target.transaction, submission);
    assert!(!parent.loaded);
    assert!(
        v.related
            .rows
            .iter()
            .any(|r| r.group == "request · from" && r.target.transaction == instruction)
    );

    app.panels
        .transaction_mut(panel)
        .unwrap()
        .set_detail_items(5);
    let v = view(&app, panel);
    assert_eq!(v.attributes.total, 26);
    assert!(!v.attributes.cut());
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Filter("sign".into()),
    ));
    let v = view(&app, panel);
    let keys: Vec<_> = v.attributes.rows.iter().map(|a| a.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "attributes.signed_i8",
            "attributes.signed_i16",
            "attributes.signed_i32",
            "attributes.unsigned_u8",
            "attributes.unsigned_u16",
            "attributes.unsigned_u32",
            "attributes.signed",
            "attributes.unsigned"
        ]
    );
    assert_eq!((v.attributes.rows.len(), v.attributes.total), (8, 26));
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Collapse(SectionKey::Attributes, true),
    ));
    assert!(view(&app, panel).attributes.collapsed);

    select(&mut app, pipeline, write, unfinished.0);
    complete_pending(&mut app);
    let v = view(&app, panel);
    assert_eq!(v.identity.status, Status::Open);
    assert_eq!((v.timing.end, v.timing.end_text.as_str()), (None, "open"));
    assert!(v.timing.duration_text.starts_with('≥'));
}

#[test]
fn a_restored_panel_loads_without_another_track_consumer() {
    for pinned in [false, true] {
        let (mut app, session, pipeline) =
            with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
        let insn = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
        let load = labelled(session.as_ref(), insn, "80000060: lw   a1, 0(a2)");
        let panel = show(&mut app, pipeline, insn, load.0);
        app.handle(Command::Transaction(panel, TransactionCommand::Pin(pinned)));
        app.handle(Command::Panels(
            volna_core::panels::PanelsCommand::CloseOthers(panel),
        ));
        assert_eq!(app.panels.len(), 1);
        let saved =
            Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();

        let mut restored = App::new();
        restored.set_session(session);
        let plan = saved
            .prepare(
                &restored,
                "file:///tmp/trace.vtr",
                "file:///tmp/trace.vtr.volna.json",
            )
            .unwrap();
        assert!(plan.commit(&mut restored).unwrap().notices.is_empty());
        complete_pending(&mut restored);
        assert_eq!(view(&restored, panel).identity.id, load);
        assert_eq!(restored.panels.transaction(panel).unwrap().pinned(), pinned);

        restored.handle(Command::Panels(volna_core::panels::PanelsCommand::Close(
            panel,
        )));
        assert!(restored.doc.resident_generator(a(insn)).is_none());
        restored.handle(Command::Undo);
        complete_pending(&mut restored);
        assert_eq!(view(&restored, panel).identity.id, load);
        restored.handle(Command::Redo);
        assert!(restored.doc.resident_generator(a(insn)).is_none());
    }
}

#[test]
fn the_workspace_restores_a_pinned_record_and_the_reader_choices() {
    let (mut app, session, pipeline) = with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
    let insn = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
    let add = labelled(session.as_ref(), insn, "80000008: add  a3, a1, a0");
    let load = labelled(session.as_ref(), insn, "80000060: lw   a1, 0(a2)");
    let pinned = show(&mut app, pipeline, insn, load.0);
    for command in [
        TransactionCommand::Pin(true),
        TransactionCommand::Radix("payload.pc".into()),
        TransactionCommand::Collapse(SectionKey::Events, true),
        TransactionCommand::CursorHeight(286.0),
    ] {
        app.handle(Command::Transaction(pinned, command));
    }
    let free = show(&mut app, pipeline, insn, add.0);
    let saved = Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    let json = serde_json::to_value(&saved).unwrap();
    let entry = json["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == "transaction" && p["pinned"] == true)
        .unwrap()
        .clone();
    assert_eq!(entry["record"]["id"], load.0);
    assert_eq!(entry["radix"]["payload.pc"], "hex");
    assert_eq!(entry["cursor_height"], 286.0);

    let mut restored = App::new();
    restored.set_session(session.clone());
    let plan = Workspace::parse(&saved.to_bytes().unwrap())
        .unwrap()
        .prepare(
            &restored,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap();
    assert!(plan.report().notices.is_empty(), "{:?}", plan.report());
    plan.commit(&mut restored).unwrap();
    complete_pending(&mut restored);
    let v = view(&restored, pinned);
    assert_eq!(v.identity.id, load);
    assert_eq!(
        v.attributes
            .rows
            .iter()
            .find(|a| a.key == "payload.pc")
            .unwrap()
            .value,
        "0x80000060"
    );
    assert!(v.events.collapsed);
    assert_eq!(
        restored
            .panels
            .transaction(pinned)
            .unwrap()
            .prefs()
            .cursor_height,
        286.0
    );
    assert_eq!(
        restored
            .panels
            .transaction(free)
            .unwrap()
            .prefs()
            .cursor_height,
        volna_core::transaction::view::DEFAULT_CURSOR_HEIGHT
    );
    assert_eq!(view(&restored, free).identity.id, add);
    for id in [load, add] {
        select(&mut restored, pipeline, insn, id.0);
        assert_eq!(view(&restored, free).identity.id, id);
        assert_eq!(view(&restored, pinned).identity.id, load);
    }
}

#[test]
fn cursor_split_validates_sizes_and_only_marks_changed_preferences_for_autosave() {
    use volna_core::transaction::view::{
        DEFAULT_CURSOR_HEIGHT, MAX_CURSOR_HEIGHT, MIN_CURSOR_HEIGHT,
    };
    use volna_core::workspace::persistence::Persistence;
    let (mut app, session, pipeline) = with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
    let insn = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
    let load = labelled(session.as_ref(), insn, "80000060: lw   a1, 0(a2)");
    let panel = show(&mut app, pipeline, insn, load.0);
    app.configure_persistence(Persistence::Auto);
    let initial = app.workspace.scheduler.revision();
    for height in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        DEFAULT_CURSOR_HEIGHT,
    ] {
        app.handle(Command::Transaction(
            panel,
            TransactionCommand::CursorHeight(height),
        ));
        assert_eq!(
            app.panels.transaction(panel).unwrap().prefs().cursor_height,
            DEFAULT_CURSOR_HEIGHT
        );
        assert_eq!(app.workspace.scheduler.revision(), initial);
    }
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::CursorHeight(-100.0),
    ));
    assert_eq!(
        app.panels.transaction(panel).unwrap().prefs().cursor_height,
        MIN_CURSOR_HEIGHT
    );
    assert_eq!(app.workspace.scheduler.revision(), initial + 1);
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::CursorHeight(10_000.0),
    ));
    assert_eq!(
        app.panels.transaction(panel).unwrap().prefs().cursor_height,
        MAX_CURSOR_HEIGHT
    );
    assert_eq!(app.workspace.scheduler.revision(), initial + 2);
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::CursorHeight(10_000.0),
    ));
    assert_eq!(app.workspace.scheduler.revision(), initial + 2);
    assert_eq!(view(&app, panel).identity.id, load);
    assert_eq!(app.doc.selection().unwrap().id, load);
}

#[test]
fn a_table_row_is_the_document_selection() {
    let session = example("pipeline_showcase.vtr");
    let mut app = App::new();
    app.set_session(session.clone());
    let read = track(session.as_ref(), "soc.l2.bus.read");
    let generator = session
        .hierarchy()
        .generators()
        .iter()
        .position(|g| g.track == read)
        .unwrap();
    app.handle(Command::OpenTable {
        selected: vec![a(volna_trace::data::Member::Generator(generator))],
        clicked: None,
    });
    let table = app.panels.focused_id();
    complete_pending(&mut app);
    frame(&mut app, table);
    app.handle(Command::Table(
        table,
        volna_core::table::TableCommand::GoTo(3),
    ));
    let selection = app.doc.selection().unwrap();
    assert_eq!((selection.track, selection.origin), (a(read), table));
    app.handle(Command::ShowTransaction { from: table });
    let panel = app.panels.focused_id();
    assert_eq!(view(&app, panel).identity.id, selection.id);
    let other = session_first_other(&app, read, selection.id);
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Jump {
            track: a(read),
            id: other,
        },
    ));
    assert_eq!(
        app.panels.get(table).unwrap().kind.table().unwrap().active,
        Some(volna_core::table::RowIdentity::Transaction(other))
    );
}

fn session_first_other(app: &App, track: TrackRef, not: TransactionRef) -> TransactionRef {
    app.doc
        .resident_generator(a(track))
        .unwrap()
        .transactions()
        .iter()
        .map(|tx| tx.id)
        .find(|id| *id != not)
        .unwrap()
}

#[test]
fn inspector_placement_preserves_record_history_and_restores_across_regions() {
    use volna_core::panels::{PanelsCommand, Placement};
    let (mut app, session, pipeline) = with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
    let insn = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
    let add = labelled(session.as_ref(), insn, "80000008: add  a3, a1, a0");
    let load = labelled(session.as_ref(), insn, "80000060: lw   a1, 0(a2)");
    let panel = show(&mut app, pipeline, insn, load.0);
    assert_eq!(app.panels.layout().placement(panel), Some(Placement::Right));
    assert_eq!(
        app.panels.layout().center.as_ref().unwrap().panels(),
        vec![pipeline]
    );
    app.handle(Command::Panels(PanelsCommand::Move {
        panel,
        placement: Placement::Center,
    }));
    assert!(app.panels.layout().right.is_none());
    assert_eq!(view(&app, panel).identity.id, load);
    app.handle(Command::Undo);
    assert_eq!(app.panels.layout().placement(panel), Some(Placement::Right));
    app.handle(Command::Redo);
    assert_eq!(
        app.panels.layout().placement(panel),
        Some(Placement::Center)
    );
    // Reusing the inspector respects the user's placement.
    assert_eq!(show(&mut app, pipeline, insn, add.0), panel);
    assert_eq!(
        app.panels.layout().placement(panel),
        Some(Placement::Center)
    );
    app.handle(Command::Panels(PanelsCommand::Move {
        panel,
        placement: Placement::Right,
    }));
    let mut layout = app.panels.layout().clone();
    layout.right.as_mut().unwrap().width = 510.0;
    app.handle(Command::Panels(PanelsCommand::SetLayout {
        layout,
        from_revision: app.panels.revision(),
    }));
    app.handle(Command::Panels(PanelsCommand::ToggleRightDock));
    assert!(!app.panels.layout().visible().contains(&panel));
    assert_eq!(app.panels.focused_id(), pipeline);
    let saved = Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    let saved = Workspace::parse(&saved.to_bytes().unwrap()).unwrap();
    let mut restored = App::new();
    restored.set_session(session);
    saved
        .prepare(
            &restored,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut restored)
        .unwrap();
    complete_pending(&mut restored);
    assert_eq!(
        restored.panels.layout().right.as_ref().unwrap().width,
        510.0
    );
    assert!(!restored.panels.layout().right.as_ref().unwrap().open);
    restored.handle(Command::Panels(PanelsCommand::Focus(panel)));
    assert!(restored.panels.layout().right.as_ref().unwrap().open);
    assert_eq!(view(&restored, panel).identity.id, add);
    assert_eq!(show(&mut restored, pipeline, insn, load.0), panel);
    restored.panels.validate().unwrap();
}

#[test]
fn new_viewers_open_centrally_when_the_inspector_is_focused_or_the_center_is_empty() {
    use volna_core::app::SettingsCommand;
    use volna_core::panels::{PanelsCommand, Placement};
    use volna_trace::data::Member;

    for empty_center in [false, true] {
        for viewer in ["waves", "table", "pipeline", "settings"] {
            let (mut app, session, pipeline) =
                with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
            let insn = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
            let add = labelled(session.as_ref(), insn, "80000008: add  a3, a1, a0");
            let detail = show(&mut app, pipeline, insn, add.0);
            if empty_center {
                app.handle(Command::Panels(PanelsCommand::Close(pipeline)));
                assert!(app.panels.layout().center.is_none());
            }
            let before = app.panels.layout().clone();
            let command = match viewer {
                "waves" => Command::AddToWaves(vec![a(Member::Generator(0))]),
                "table" => Command::OpenTable {
                    selected: vec![a(Member::Generator(0))],
                    clicked: None,
                },
                "pipeline" => Command::OpenPipeline {
                    track: a(track(session.as_ref(), "soc.l2.bus")),
                },
                "settings" => Command::Settings(SettingsCommand::Open),
                _ => unreachable!(),
            };
            app.handle(command);
            let opened = app.panels.focused_id();
            assert_eq!(
                app.panels.layout().placement(opened),
                Some(Placement::Center),
                "{viewer}"
            );
            let kind = &app.panels.get(opened).unwrap().kind;
            match viewer {
                "waves" => assert!(
                    kind.waves()
                        .unwrap()
                        .items()
                        .iter()
                        .any(|item| item.lane_track().is_some())
                ),
                "table" => assert!(kind.table().is_some()),
                "pipeline" => assert!(kind.pipeline().is_some()),
                "settings" => assert!(kind.is_settings()),
                _ => unreachable!(),
            }
            assert_eq!(app.panels.layout().right, before.right);
            assert_eq!(view(&app, detail).identity.id, add);
            app.panels.validate().unwrap();

            // Settings is transient chrome; content creation is one undo step,
            // including the first generator lane inserted into a new viewer.
            if viewer != "settings" {
                app.handle(Command::Undo);
                assert_eq!(app.panels.layout(), &before, "{viewer}");
                app.handle(Command::Redo);
                assert_eq!(app.panels.focused_id(), opened);
                assert_eq!(
                    app.panels.layout().placement(opened),
                    Some(Placement::Center)
                );
                app.panels.validate().unwrap();
            }
        }
    }
}

#[test]
fn table_from_a_sidebar_pipeline_opens_centrally_and_moved_waveforms_are_reused() {
    use volna_core::panels::{PanelsCommand, Placement};
    use volna_trace::data::Member;

    let (mut app, _, pipeline) =
        with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline.instruction");
    app.handle(Command::Panels(PanelsCommand::Move {
        panel: pipeline,
        placement: Placement::Right,
    }));
    assert!(app.panels.layout().center.is_none());
    app.handle(Command::OpenTableFromPanel {
        panel: pipeline,
        row: None,
    });
    let table = app.panels.focused_id();
    assert!(app.panels.get(table).unwrap().kind.table().is_some());
    assert_eq!(
        app.panels.layout().placement(table),
        Some(Placement::Center)
    );

    app.handle(Command::AddToWaves(vec![a(Member::Generator(0))]));
    let waves = app.panels.focused_id();
    app.handle(Command::Panels(PanelsCommand::Move {
        panel: waves,
        placement: Placement::Right,
    }));
    for empty_center in [false, true] {
        if empty_center {
            app.handle(Command::Panels(PanelsCommand::Close(table)));
        }
        app.handle(Command::Panels(PanelsCommand::ToggleRightDock));
        let count = app.panels.len();
        app.handle(Command::AddToWaves(vec![a(Member::Generator(0))]));
        assert_eq!(app.panels.focused_id(), waves);
        assert_eq!(app.panels.len(), count);
        assert_eq!(app.panels.layout().placement(waves), Some(Placement::Right));
        assert!(app.panels.layout().right.as_ref().unwrap().open);
        app.panels.validate().unwrap();
    }
}

#[test]
fn opening_a_pipeline_reveals_a_reusable_inspector_without_stealing_focus() {
    use volna_core::panels::{PanelsCommand, Placement};
    let (mut app, session, pipeline) = with_pipeline("pipeline_showcase.vtr", "soc.cpu0.pipeline");
    let pipeline_track = a(track(session.as_ref(), "soc.cpu0.pipeline"));
    let insn = track(session.as_ref(), "soc.cpu0.pipeline.instruction");
    let add = labelled(session.as_ref(), insn, "80000008: add  a3, a1, a0");
    let load = labelled(session.as_ref(), insn, "80000060: lw   a1, 0(a2)");
    let detail = app.panels.first_free_transaction().unwrap();
    assert_eq!(app.panels.focused_id(), pipeline);
    assert_eq!(
        app.panels.layout().placement(detail),
        Some(Placement::Right)
    );
    assert!(matches!(
        app.panels.transaction(detail).unwrap().state(&app.doc),
        TxPanelState::Empty
    ));
    assert!(app.doc.selection().is_none());

    // The pipeline and its automatically created inspector are one layout edit.
    app.handle(Command::Undo);
    assert_eq!(app.panels.len(), 1);
    assert!(app.panels.focused().kind.is_start());
    assert!(app.panels.layout().right.is_none());
    app.handle(Command::Redo);
    complete_pending(&mut app);
    assert_eq!(app.panels.focused_id(), pipeline);
    assert_eq!(app.panels.first_free_transaction(), Some(detail));

    select(&mut app, pipeline, insn, add.0);
    assert_eq!(view(&app, detail).identity.id, add);
    assert_eq!(app.panels.focused_id(), pipeline);
    app.handle(Command::Panels(PanelsCommand::ToggleRightDock));
    app.handle(Command::OpenPipeline {
        track: pipeline_track,
    });
    assert_eq!(app.panels.len(), 2);
    assert!(app.panels.layout().right.as_ref().unwrap().open);
    assert_eq!(app.panels.focused_id(), pipeline);

    app.handle(Command::Panels(PanelsCommand::Move {
        panel: detail,
        placement: Placement::Center,
    }));
    app.handle(Command::OpenPipeline {
        track: pipeline_track,
    });
    assert_eq!(app.panels.first_free_transaction(), Some(detail));
    assert_eq!(
        app.panels.layout().placement(detail),
        Some(Placement::Center)
    );
    assert!(app.panels.layout().right.is_none());

    app.handle(Command::Transaction(detail, TransactionCommand::Pin(true)));
    app.handle(Command::OpenPipeline {
        track: pipeline_track,
    });
    let following = app.panels.first_free_transaction().unwrap();
    assert_ne!(following, detail);
    select(&mut app, pipeline, insn, load.0);
    assert_eq!(view(&app, detail).identity.id, add);
    assert_eq!(view(&app, following).identity.id, load);
    assert_eq!(app.panels.focused_id(), pipeline);

    // Opening another track from a focused inspector still puts the pipeline
    // in the center and reuses the same inspector.
    app.handle(Command::Panels(PanelsCommand::Focus(following)));
    app.handle(Command::OpenPipeline {
        track: a(track(session.as_ref(), "soc.l2.bus")),
    });
    let second = app.panels.focused_id();
    assert!(app.panels.pipeline(second).is_some());
    assert_eq!(
        app.panels.layout().placement(second),
        Some(Placement::Center)
    );
    assert_eq!(app.panels.first_free_transaction(), Some(following));
    app.panels.validate().unwrap();
}

struct AttributePacket {
    flops: [u64; 4],
    samples: Vec<u64>,
}
vtr::impl_payload!(AttributePacket { flops, samples });

fn nested_attribute_record(
    samples: usize,
    long_values: usize,
) -> (App, PanelId, tempfile::TempDir) {
    use vtr::{Attribute, Recorder, SourceOptions, Value};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("attributes.vtr");
    let mut writer = vtr::Writer::create(&path).unwrap();
    let source = writer
        .transactions::<AttributePacket>(SourceOptions::new("instructions", None), "INSTRUCTION")
        .unwrap();
    let instruction = writer.intern("instruction").unwrap();
    let mnemonic = writer.intern("mnemonic").unwrap();
    let pc = writer.intern("pc").unwrap();
    let empty = writer.intern("empty").unwrap();
    let bytes = writer.intern("bytes").unwrap();
    let bits = writer.intern("bits").unwrap();
    let note = writer.intern("note").unwrap();
    let value = writer
        .intern_value(Value::Map(vec![
            Attribute {
                key: mnemonic,
                value: Value::Text("ds_read_b32".into()),
            },
            Attribute {
                key: pc,
                value: Value::U64(u64::MAX),
            },
            Attribute {
                key: empty,
                value: Value::List(vec![]),
            },
            Attribute {
                key: bytes,
                value: Value::Bytes(vec![0, 0xab, 0xff]),
            },
            Attribute {
                key: bits,
                value: Value::Bits {
                    width: 4,
                    data: vec![0b1010],
                },
            },
            Attribute {
                key: note,
                value: Value::Text(format!("{}café needle", "x".repeat(300))),
            },
        ]))
        .unwrap();
    let owner = writer
        .begin_at(
            source,
            10,
            &AttributePacket {
                flops: [0, 1, 2, 3],
                samples: (0..samples as u64).collect(),
            },
            None,
        )
        .unwrap();
    writer
        .attribute(owner.owner(), instruction, Value::Ref(value))
        .unwrap();
    if long_values > 0 {
        let notes = writer.intern("notes").unwrap();
        writer
            .attribute(
                owner.owner(),
                notes,
                Value::List(
                    (0..long_values)
                        .map(|i| Value::Text(format!("entry-{i:03} {}", "λ".repeat(2000))))
                        .collect(),
                ),
            )
            .unwrap();
    }
    let attrs = vec![Attribute {
        key: instruction,
        value: Value::Ref(value),
    }];
    writer
        .embedded_stage(owner.owner(), 11, mnemonic, 0, Some(12), attrs.clone())
        .unwrap();
    writer
        .embedded_event(owner.owner(), 12, mnemonic, attrs)
        .unwrap();
    writer.end_at(owner, 15, Status::Ok).unwrap();
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(path).open().unwrap());
    let track = TrackRef(source.id());
    app.handle(Command::OpenPipeline { track: a(track) });
    let pipeline = app.panels.focused_id();
    complete_pending(&mut app);
    let panel = show(&mut app, pipeline, track, owner.id());
    (app, panel, directory)
}

#[test]
fn nested_record_stage_and_event_values_are_expanded_searchable_and_copyable() {
    let (mut app, panel, _directory) = nested_attribute_record(0, 0);
    let projection = view(&app, panel);
    for (path, value) in [
        ("attributes.instruction.mnemonic", "ds_read_b32"),
        ("attributes.instruction.pc", "18446744073709551615"),
        ("attributes.instruction.empty", "0 items"),
        ("attributes.instruction.bytes", "00 ab ff"),
        ("attributes.instruction.bits", "1010"),
        ("payload.flops[3]", "3"),
    ] {
        assert!(
            projection
                .attributes
                .rows
                .iter()
                .any(|row| row.path == path && row.value == value),
            "{path}"
        );
    }
    let branch = projection
        .attributes
        .rows
        .iter()
        .find(|row| row.key == "attributes.instruction")
        .unwrap();
    let branch_id = branch.id.clone();
    assert!(branch.expanded);
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::ToggleAttribute(branch_id.clone()),
    ));
    assert!(
        !view(&app, panel)
            .attributes
            .rows
            .iter()
            .any(|row| row.key == "mnemonic")
    );
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Filter("CAFÉ NEEDLE".into()),
    ));
    let projection = view(&app, panel);
    assert_eq!(projection.attributes.rows.len(), 2);
    assert_eq!(projection.attributes.rows[0].key, "attributes.instruction");
    assert!(projection.attributes.rows[1].value.ends_with("café needle"));
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Filter(String::new()),
    ));
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::ExpandAttributes(true),
    ));
    let projection = view(&app, panel);
    let pc = projection
        .attributes
        .rows
        .iter()
        .find(|row| row.key == "pc")
        .unwrap();
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::AttributeRadix(pc.id.clone()),
    ));
    assert_eq!(
        view(&app, panel)
            .attributes
            .rows
            .iter()
            .find(|row| row.key == "pc")
            .unwrap()
            .value,
        "0xffffffffffffffff"
    );
    let copied = app
        .panels
        .transaction(panel)
        .unwrap()
        .copy_attribute(&app.doc, &branch_id)
        .unwrap();
    assert!(copied.contains("\"mnemonic\": \"ds_read_b32\""));
    assert!(copied.contains("18446744073709551615"));
    assert!(copied.contains("\"empty\": []"));
    let stage = projection.stages.rows[0].id;
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::SelectStage(stage),
    ));
    let projection = view(&app, panel);
    assert!(
        projection
            .stage_details
            .unwrap()
            .attributes
            .rows
            .iter()
            .any(|row| row.key == "mnemonic" && row.value == "ds_read_b32")
    );
    assert!(
        projection.events.rows[0]
            .attributes
            .rows
            .iter()
            .any(|row| row.key == "pc" && row.value == u64::MAX.to_string())
    );
    let tsv = app
        .panels
        .transaction(panel)
        .unwrap()
        .copy_tsv(&app.doc)
        .unwrap();
    assert!(tsv.contains("payload.flops\t[0, 1, 2, 3]"));
    assert_eq!(tsv.matches("\"mnemonic\": \"ds_read_b32\"").count(), 3);
}

#[test]
fn every_large_list_element_is_reachable_and_search_reveals_collapsed_matches() {
    let (mut app, panel, _directory) = nested_attribute_record(150, 0);
    app.panels
        .transaction_mut(panel)
        .unwrap()
        .set_detail_items(1);
    let mut values = Vec::new();
    let mut page = 0;
    loop {
        let projection = view(&app, panel);
        for row in &projection.attributes.rows {
            if row.path.starts_with("payload.samples[") {
                values.push(row.value.parse::<u64>().unwrap());
            }
        }
        if !projection.attributes.has_next() {
            break;
        }
        page += 1;
        app.handle(Command::Transaction(
            panel,
            TransactionCommand::AttributePage(projection.attributes.owner, page),
        ));
    }
    assert_eq!(values, (0..150).collect::<Vec<_>>());
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::ExpandAttributes(false),
    ));
    let projection = view(&app, panel);
    assert_eq!(projection.attributes.page, 0);
    assert!(projection.attributes.rows.iter().all(|row| row.depth == 0));
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Filter("149".into()),
    ));
    let projection = view(&app, panel);
    assert_eq!(
        projection
            .attributes
            .rows
            .iter()
            .map(|row| row.key.as_str())
            .collect::<Vec<_>>(),
        ["payload.samples", "[149]"]
    );
    assert_eq!(projection.attributes.rows[1].value, "149");
    assert!(projection.attributes.rows[0].expanded);
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Filter("absent".into()),
    ));
    assert!(view(&app, panel).attributes.rows.is_empty());
}

#[test]
fn paging_long_values_never_skips_an_attribute_and_individual_copies_remain_complete() {
    let (mut app, panel, _directory) = nested_attribute_record(0, 70);
    let mut entries = Vec::new();
    let mut page = 0;
    loop {
        let projection = view(&app, panel);
        for row in &projection.attributes.rows {
            if row.path.starts_with("attributes.notes[") {
                let index = entries.len();
                assert!(
                    row.value.starts_with(&format!("entry-{index:03}")),
                    "{}: {}",
                    row.path,
                    row.value
                );
                let copy = app
                    .panels
                    .transaction(panel)
                    .unwrap()
                    .copy_attribute(&app.doc, &row.id)
                    .unwrap();
                assert_eq!(copy, format!("entry-{index:03} {}", "λ".repeat(2000)));
                entries.push(row.id.clone());
            }
        }
        if !projection.attributes.has_next() {
            break;
        }
        page += 1;
        app.handle(Command::Transaction(
            panel,
            TransactionCommand::AttributePage(projection.attributes.owner, page),
        ));
    }
    assert_eq!(entries.len(), 70);
}

fn attribute_record(
    value: impl FnOnce(&mut vtr::Writer) -> vtr::Value,
    name: &str,
) -> (App, PanelId, tempfile::TempDir) {
    use vtr::{Recorder, SourceOptions};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("attributes.vtr");
    let mut writer = vtr::Writer::create(&path).unwrap();
    let source = writer
        .transactions::<()>(SourceOptions::new("instructions", None), "INSTRUCTION")
        .unwrap();
    let key = writer.intern(name).unwrap();
    let value = value(&mut writer);
    let owner = writer.begin_at(source, 10, &(), None).unwrap();
    writer.attribute(owner.owner(), key, value).unwrap();
    writer.end_at(owner, 15, Status::Ok).unwrap();
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(path).open().unwrap());
    let track = TrackRef(source.id());
    app.handle(Command::OpenPipeline { track: a(track) });
    let pipeline = app.panels.focused_id();
    complete_pending(&mut app);
    let panel = show(&mut app, pipeline, track, owner.id());
    (app, panel, directory)
}

#[test]
fn shared_value_graphs_open_page_and_search_without_expanding_all_occurrences() {
    let (done, result) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use vtr::Value;
        let (mut app, panel, _directory) = attribute_record(
            |writer| {
                let mut value = writer.intern_value(Value::U64(255)).unwrap();
                for _ in 0..40 {
                    value = writer
                        .intern_value(Value::List(vec![Value::Ref(value); 2]))
                        .unwrap();
                }
                Value::Ref(value)
            },
            "shared",
        );
        let first = view(&app, panel);
        assert!(first.attributes.has_next());
        assert!(first.attributes.rows.iter().any(|row| row.value == "255"));
        app.handle(Command::Transaction(
            panel,
            TransactionCommand::AttributePage(first.attributes.owner, 1),
        ));
        let second = view(&app, panel);
        assert_eq!(second.attributes.page, 1);
        assert!(second.attributes.has_previous());
        let leaf = second
            .attributes
            .rows
            .iter()
            .find(|row| row.value == "255")
            .unwrap();
        assert_eq!(
            app.panels
                .transaction(panel)
                .unwrap()
                .copy_attribute(&app.doc, &leaf.id)
                .unwrap(),
            "255"
        );
        app.handle(Command::Transaction(
            panel,
            TransactionCommand::AttributePage(first.attributes.owner, 0),
        ));
        assert_eq!(view(&app, panel).attributes.rows, first.attributes.rows);
        app.handle(Command::Transaction(
            panel,
            TransactionCommand::Filter("absent".into()),
        ));
        let searched = view(&app, panel);
        assert!(searched.attributes.rows.is_empty());
        assert!(!searched.attributes.has_next());
        done.send(()).unwrap();
    });
    result
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("A compact shared graph must remain interactive.");
}

#[test]
fn sparse_search_continues_from_its_work_window_and_keeps_the_matching_ancestor() {
    let (mut app, panel, _directory) = nested_attribute_record(12_000, 0);
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Filter("11999".into()),
    ));
    let first = view(&app, panel);
    assert!(first.attributes.rows.is_empty());
    assert!(first.attributes.has_next());
    let mut page = 0;
    loop {
        let projection = view(&app, panel);
        assert!(!projection.attributes.limited);
        if projection
            .attributes
            .rows
            .iter()
            .any(|row| row.key == "[11999]")
        {
            assert!(
                projection
                    .attributes
                    .rows
                    .iter()
                    .any(|row| row.key == "payload.samples")
            );
            assert_eq!(
                projection
                    .attributes
                    .rows
                    .iter()
                    .find(|row| row.key == "[11999]")
                    .unwrap()
                    .value,
                "11999"
            );
            break;
        }
        assert!(
            projection.attributes.has_next(),
            "search must reach the last sample"
        );
        page += 1;
        app.handle(Command::Transaction(
            panel,
            TransactionCommand::AttributePage(projection.attributes.owner, page),
        ));
    }
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::AttributePage(first.attributes.owner, 0),
    ));
    assert_eq!(view(&app, panel).attributes.page, 0);
}

#[test]
fn shortened_labels_preserve_full_paths_radix_actions_and_table_column_keys() {
    let name = format!("{}tail", "field_".repeat(80));
    let (mut app, panel, _directory) = attribute_record(|_| vtr::Value::U64(255), &name);
    let projection = view(&app, panel);
    let row = projection
        .attributes
        .rows
        .iter()
        .find(|row| row.key.starts_with("attributes.field_"))
        .unwrap();
    assert!(row.key.len() < name.len());
    let id = row.id.clone();
    let full_key = format!("attributes.{name}");
    let model = app.panels.transaction(panel).unwrap();
    assert_eq!(model.copy_attribute_path(&app.doc, &id).unwrap(), full_key);
    assert_eq!(model.attribute_column(&app.doc, &id).unwrap(), full_key);
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::AttributeRadix(id.clone()),
    ));
    let projection = view(&app, panel);
    assert_eq!(
        projection
            .attributes
            .rows
            .iter()
            .find(|row| row.id == id)
            .unwrap()
            .value,
        "0xff"
    );
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Filter("tail".into()),
    ));
    assert!(
        view(&app, panel)
            .attributes
            .rows
            .iter()
            .any(|row| row.id == id)
    );
    let key = app
        .panels
        .transaction(panel)
        .unwrap()
        .attribute_column(&app.doc, &id)
        .unwrap();
    app.handle(Command::ShowAttributeColumn { from: panel, key });
    let table = app.panels.focused_id();
    complete_pending(&mut app);
    frame(&mut app, table);
    let model = app.panels.get(table).unwrap().kind.table().unwrap();
    assert!((0..model.columns.get().len()).any(|column| model.column_title(column) == full_key));
    let copied = model.copy_tsv().unwrap();
    let mut lines = copied.lines();
    let column = lines
        .next()
        .unwrap()
        .split('\t')
        .position(|title| title == full_key)
        .unwrap();
    assert_eq!(lines.next().unwrap().split('\t').nth(column), Some("0xff"));
}

#[test]
fn deep_attribute_ids_keep_copy_and_radix_actions_after_the_path_preview_is_shortened() {
    let (mut app, panel, _directory) = attribute_record(
        |writer| {
            let mut value = writer.intern_value(vtr::Value::U64(255)).unwrap();
            for _ in 0..150 {
                value = writer
                    .intern_value(vtr::Value::List(vec![vtr::Value::Ref(value)]))
                    .unwrap();
            }
            vtr::Value::Ref(value)
        },
        "deep",
    );
    let first = view(&app, panel);
    let root = &first.attributes.rows[0].id;
    assert_eq!(
        app.panels
            .transaction(panel)
            .unwrap()
            .copy_attribute(&app.doc, root)
            .unwrap(),
        format!("{}255{}", "[".repeat(150), "]".repeat(150))
    );
    let mut page = 0;
    let leaf = loop {
        let projection = view(&app, panel);
        if let Some(leaf) = projection
            .attributes
            .rows
            .iter()
            .find(|row| row.value == "255")
        {
            break leaf.id.clone();
        }
        assert!(projection.attributes.has_next());
        page += 1;
        app.handle(Command::Transaction(
            panel,
            TransactionCommand::AttributePage(projection.attributes.owner, page),
        ));
    };
    assert_eq!(
        app.panels
            .transaction(panel)
            .unwrap()
            .copy_attribute_path(&app.doc, &leaf)
            .unwrap(),
        format!("attributes.deep{}", "[0]".repeat(150))
    );
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::AttributeRadix(leaf.clone()),
    ));
    let projection = view(&app, panel);
    assert_eq!(
        projection
            .attributes
            .rows
            .iter()
            .find(|row| row.id == leaf)
            .unwrap()
            .value,
        "0xff"
    );
    assert_eq!(
        app.panels
            .transaction(panel)
            .unwrap()
            .copy_attribute(&app.doc, &leaf)
            .unwrap(),
        "255"
    );
}

fn overlapping_attribute_record(pc: u64) -> (App, PanelId, tempfile::TempDir) {
    attribute_record(
        |writer| {
            let pc_key = writer.intern("pc").unwrap();
            let samples = writer.intern("samples").unwrap();
            vtr::Value::Map(vec![
                vtr::Attribute {
                    key: pc_key,
                    value: vtr::Value::U64(pc),
                },
                vtr::Attribute {
                    key: samples,
                    value: vtr::Value::List((0..150).map(vtr::Value::U64).collect()),
                },
            ])
        },
        "instruction",
    )
}

fn show_added_attribute_record(
    app: &mut App,
    directory: &std::path::Path,
) -> (volna_core::trace::TraceId, PanelId) {
    let session = OpenSpec::Path(directory.join("attributes.vtr"))
        .open()
        .unwrap();
    let track = session.tracks()[0].id;
    let owner = session.load_track(track).unwrap().generators[0].transactions()[0].id;
    let trace = app.add_session(session).unwrap();
    let track = volna_core::trace::Traced::new(trace, track);
    app.handle(Command::OpenPipeline { track });
    let pipeline = app.panels.focused_id();
    complete_pending(app);
    app.handle(Command::SelectTransaction {
        panel: pipeline,
        track,
        id: owner,
        cursor: None,
    });
    app.handle(Command::ShowTransaction { from: pipeline });
    complete_pending(app);
    (trace, app.panels.focused_id())
}

#[test]
fn attribute_actions_stay_with_their_recording_when_the_inspector_switches_traces() {
    let (mut app, panel, _first_directory) = overlapping_attribute_record(255);
    let first = view(&app, panel);
    let first_branch = first
        .attributes
        .rows
        .iter()
        .find(|row| row.key == "attributes.instruction")
        .unwrap()
        .id
        .clone();
    let first_pc = first
        .attributes
        .rows
        .iter()
        .find(|row| row.key == "pc")
        .unwrap()
        .id
        .clone();
    let (_second, _, second_directory) = overlapping_attribute_record(100);
    let (_, current_panel) = show_added_attribute_record(&mut app, second_directory.path());
    assert_eq!(current_panel, panel);
    let second = view(&app, panel);
    assert_eq!(first.identity.id, second.identity.id);
    let second_pc = second
        .attributes
        .rows
        .iter()
        .find(|row| row.key == "pc")
        .unwrap()
        .id
        .clone();
    let model = app.panels.transaction(panel).unwrap();
    assert!(model.copy_attribute(&app.doc, &first_pc).is_err());
    assert!(model.copy_attribute_path(&app.doc, &first_pc).is_err());
    assert!(model.attribute_column(&app.doc, &first_branch).is_none());
    assert_eq!(model.copy_attribute(&app.doc, &second_pc).unwrap(), "100");
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::AttributePage(second.attributes.owner, 1),
    ));
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::AttributeRadix(first_pc.clone()),
    ));
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::ToggleAttribute(first_branch),
    ));
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::AttributePage(first.attributes.owner, 0),
    ));
    assert_eq!(view(&app, panel).attributes.page, 1);
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::AttributePage(second.attributes.owner, 0),
    ));
    let projection = view(&app, panel);
    assert!(
        projection
            .attributes
            .rows
            .iter()
            .find(|row| row.key == "attributes.instruction")
            .unwrap()
            .expanded
    );
    assert_eq!(
        projection
            .attributes
            .rows
            .iter()
            .find(|row| row.key == "pc")
            .unwrap()
            .value,
        "100"
    );
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::AttributeRadix(second_pc.clone()),
    ));
    assert_eq!(
        view(&app, panel)
            .attributes
            .rows
            .iter()
            .find(|row| row.id == second_pc)
            .unwrap()
            .value,
        "0x64"
    );
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Jump {
            track: first.identity.track,
            id: first.identity.id,
        },
    ));
    complete_pending(&mut app);
    assert_eq!(
        app.panels
            .transaction(panel)
            .unwrap()
            .copy_attribute(&app.doc, &first_pc)
            .unwrap(),
        "255"
    );
}

#[test]
fn attribute_actions_reject_a_removed_recording_when_its_trace_letter_is_reused() {
    let (mut app, panel, _first_directory) = overlapping_attribute_record(1);
    let (_second, _, second_directory) = overlapping_attribute_record(2);
    let (trace, _) = show_added_attribute_record(&mut app, second_directory.path());
    let previous = view(&app, panel);
    let old_id = previous
        .attributes
        .rows
        .iter()
        .find(|row| row.key == "pc")
        .unwrap()
        .id
        .clone();
    app.handle(Command::RemoveTrace(trace));
    let (_replacement, _, replacement_directory) = overlapping_attribute_record(9);
    let (replacement, current_panel) =
        show_added_attribute_record(&mut app, replacement_directory.path());
    assert_eq!(trace, replacement);
    let model = app.panels.transaction(current_panel).unwrap();
    assert!(model.copy_attribute(&app.doc, &old_id).is_err());
    assert!(model.copy_attribute_path(&app.doc, &old_id).is_err());
    app.handle(Command::Transaction(
        current_panel,
        TransactionCommand::AttributeRadix(old_id),
    ));
    app.handle(Command::Transaction(
        current_panel,
        TransactionCommand::AttributePage(previous.attributes.owner, 1),
    ));
    let current = view(&app, current_panel);
    assert_eq!(current.attributes.page, 0);
    let pc = current
        .attributes
        .rows
        .iter()
        .find(|row| row.key == "pc")
        .unwrap();
    assert_eq!(pc.value, "9");
    assert_eq!(
        app.panels
            .transaction(current_panel)
            .unwrap()
            .copy_attribute(&app.doc, &pc.id)
            .unwrap(),
        "9"
    );
}
