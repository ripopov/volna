//! Located events and logs share waveform grouping and the record detail panel.

use std::path::PathBuf;
use volna_core::data::points::PointKind;
use volna_core::geometry::{Modifiers, MouseButton, Rect, point};
use volna_core::panels::PanelId;
use volna_core::scene::MonoMeasure;
use volna_core::testing::a;
use volna_core::transaction::{TransactionCommand, TxPanelState};
use volna_core::wave::{PointerEvent, points};
use volna_core::{App, Command, Theme};
use volna_trace::data::Member;
use volna_trace::session::OpenSpec;

fn pump(app: &mut App) {
    loop {
        let requests = app.take_requests();
        if requests.is_empty() {
            break;
        }
        for request in requests {
            app.deliver(request.perform());
        }
    }
}

fn frame(app: &mut App, waves: PanelId) {
    let theme = Theme::one_dark();
    app.layout_panel(
        waves,
        Rect::from_xywh(0.0, 0.0, 1200.0, 700.0),
        &theme,
        1.0,
        &mut MonoMeasure,
    )
    .unwrap();
    app.render_panel(waves, &theme, &mut MonoMeasure);
}

#[test]
fn point_generators_open_on_waves_and_group_details_preserve_record_identity() {
    for (name, kind, min_burst) in [
        ("log", PointKind::Log, 300),
        ("events", PointKind::Event, 128),
    ] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../volna/examples/landing.vtr");
        let session = OpenSpec::Path(path).open().unwrap();
        let member = Member::Generator(
            session
                .hierarchy()
                .generators()
                .iter()
                .position(|g| g.name == name)
                .unwrap(),
        );
        let mut app = App::new();
        app.set_session(session.clone());
        app.handle(Command::ActivateMembers(vec![a(member)]));
        let waves = app.panels.focused_id();
        assert!(app.panels.waves(waves).is_some());
        pump(&mut app);
        let lane = app.panels.waves(waves).unwrap().items()[0].lane().unwrap();
        let generator = lane.generator(&app.doc).unwrap();
        assert_eq!(generator.points().unwrap().kind, kind);
        let burst = generator
            .transactions()
            .chunk_by(|a, b| a.begin == b.begin)
            .max_by_key(|g| g.len())
            .unwrap();
        assert!(burst.len() >= min_burst);
        let time = burst[0].begin;
        let selected_label = generator.points().unwrap().kind.label(&burst[0]).unwrap();
        let expected_label = volna_core::data::text::format_attribute(selected_label, 1024);
        let (value, _) = volna_core::wave::lane::value_text(generator, time);
        assert_eq!(
            value,
            format!(
                "{} {}",
                burst.len(),
                if kind == PointKind::Log {
                    "messages"
                } else {
                    "events"
                }
            )
        );
        app.doc.shared.viewport.value = volna_core::wave::Viewport {
            start: time as f64 - 1.0,
            end: time as f64 + 1.0,
        };
        frame(&mut app, waves);
        let group = {
            let w = app.panels.waves(waves).unwrap();
            let l = w.last_layout();
            let g = w.items()[0].lane().unwrap().generator(&app.doc).unwrap();
            points::layout(
                g.transactions(),
                g.points().unwrap(),
                w.viewport(&app.doc),
                Rect::from_xywh(l.waves.left(), l.row_y(0), l.waves.width(), l.row_height(0)),
                l.zoom,
            )
            .into_iter()
            .find(|g| g.records.len() >= min_burst)
            .unwrap()
        };
        let click = point(group.rect.left() + 10.0, group.rect.top() + 10.0);
        app.handle(Command::Pointer(
            waves,
            PointerEvent::Down {
                position: click,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        ));
        app.handle(Command::Pointer(waves, PointerEvent::Up));
        assert_eq!(app.doc.selection_range().unwrap().begin, time);
        let opened = app
            .panels
            .first_free_transaction()
            .expect("a point click opens record details");
        assert_eq!(app.panels.focused_id(), waves);
        app.handle(Command::ShowTransaction { from: waves });
        let details = app.panels.focused_id();
        assert_eq!(details, opened);
        let model = app.panels.transaction(details).unwrap();
        let view = model.group(&app.doc).unwrap();
        assert!(view.total >= min_burst);
        assert_eq!(view.records.len(), 12);
        let total = view.total;
        drop(view);
        match model.state(&app.doc) {
            TxPanelState::Ready(view) => {
                assert_eq!(view.point_kind, Some(kind));
                assert_eq!(
                    view.identity.label.as_deref(),
                    Some(expected_label.as_str())
                );
                assert_eq!(view.timing.begin, time);
                if kind == PointKind::Log {
                    assert!(!view.source.is_empty());
                    assert!(view.attributes.rows.iter().any(|a| a.key == "vtr.label"));
                    assert!(
                        view.attributes
                            .rows
                            .iter()
                            .all(|a| !a.key.starts_with("log."))
                    );
                }
            }
            _ => panic!("point details unavailable"),
        }
        app.handle(Command::Transaction(
            details,
            TransactionCommand::GroupPage(1),
        ));
        let page = app
            .panels
            .transaction(details)
            .unwrap()
            .group(&app.doc)
            .unwrap()
            .clone();
        assert_eq!(page.page, 1);
        assert_eq!(page.total, total);
        let picked = page.records[3].id;
        app.handle(Command::Transaction(
            details,
            TransactionCommand::GroupSelect(picked),
        ));
        assert_eq!(app.doc.selection().unwrap().id, picked);
        assert_eq!(app.doc.shared.cursor, Some(time));
        assert!(
            app.panels
                .transaction(details)
                .unwrap()
                .shown()
                .unwrap()
                .range
                .is_some()
        );
        if kind == PointKind::Log {
            app.handle(Command::Transaction(
                details,
                TransactionCommand::GroupSeverity(Some(4)),
            ));
            let view = app
                .panels
                .transaction(details)
                .unwrap()
                .group(&app.doc)
                .unwrap();
            assert!(view.matching > 0 && view.matching < total);
            assert!(view.records.iter().all(|r| r.severity == 4));
        }
        app.handle(Command::Transaction(
            details,
            TransactionCommand::GroupSearch("no such diagnostic".into()),
        ));
        let empty = app
            .panels
            .transaction(details)
            .unwrap()
            .group(&app.doc)
            .unwrap();
        assert_eq!(empty.matching, 0);
        assert!(empty.records.is_empty());
        drop(empty);

        for command in [
            TransactionCommand::GroupSearch(String::new()),
            TransactionCommand::GroupSeverity(None),
            TransactionCommand::Pin(true),
        ] {
            app.handle(Command::Transaction(details, command));
        }
        let shown = app
            .panels
            .transaction(details)
            .unwrap()
            .shown()
            .unwrap()
            .clone();
        let track = shown.track.track().unwrap();
        let different = app
            .doc
            .resident_generator(track)
            .unwrap()
            .transactions()
            .iter()
            .find(|r| r.begin != time)
            .unwrap()
            .id;
        app.handle(Command::SelectTransaction {
            panel: waves,
            track,
            id: different,
            cursor: None,
        });
        assert_eq!(
            app.panels.transaction(details).unwrap().shown(),
            Some(&shown)
        );

        let saved = volna_core::workspace::Workspace::capture(
            &app,
            volna_core::testing::paths("landing.vtr"),
            None,
        )
        .unwrap();
        let mut restored = App::new();
        restored.set_session(session);
        volna_core::workspace::Workspace::parse(&saved.to_bytes().unwrap())
            .unwrap()
            .prepare(
                &restored,
                "file:///tmp/landing.vtr",
                "file:///tmp/landing.volna.json",
            )
            .unwrap()
            .commit(&mut restored)
            .unwrap();
        pump(&mut restored);
        let model = restored.panels.transaction(details).unwrap();
        assert!(model.pinned());
        assert_eq!(model.shown(), Some(&shown));
        assert_eq!(model.group(&restored.doc).unwrap().total, total);

        app.handle(Command::Transaction(
            details,
            TransactionCommand::Pin(false),
        ));
        assert!(
            app.panels
                .transaction(details)
                .unwrap()
                .group(&app.doc)
                .is_none()
        );
        app.handle(Command::Transaction(details, TransactionCommand::Back));
        assert_eq!(app.doc.selection_range(), shown.range);
        assert_eq!(
            app.panels
                .transaction(details)
                .unwrap()
                .group(&app.doc)
                .unwrap()
                .total,
            total
        );

        app.handle(Command::Transaction(details, TransactionCommand::Pin(true)));
        let finer = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../volna-trace/tests/fixtures/values.vtr");
        app.handle(Command::AddTrace(OpenSpec::Path(finer)));
        pump(&mut app);
        assert_eq!(app.doc.time_base().timescale, -12);
        assert_eq!(app.doc.selection_range().unwrap().begin, time * 1000);
        let model = app.panels.transaction(details).unwrap();
        let group = model.group(&app.doc).unwrap();
        assert_eq!(group.range.begin, time * 1000);
        assert_eq!(group.total, total);
        assert!(group.records.iter().all(|r| r.time == time * 1000));
        drop(group);
        app.handle(Command::CloseTrace);
        assert!(app.doc.selection().is_none());
        assert!(app.doc.selection_range().is_none());
    }
}
