//! Headless vertical-slice checks for the reduced table panel.

use std::path::PathBuf;
use volna_core::testing::a_all;
use volna_core::trace::TraceId;
use volna_trace::session::LoadRequest as TraceRequest;

use volna_core::app::{App, Command, PanelLayout};
use volna_core::geometry::Rect;
use volna_core::scene::MonoMeasure;
use volna_core::session::LoadRequest;
use volna_core::table::columns::TransactionColumn;
use volna_core::table::{RowIdentity, TableCommand, TableSource, TableState};
use volna_core::workspace::Workspace;
use volna_core::{Instant, Theme};
use volna_trace::data::Member;
use volna_trace::session::{OpenSpec, Session};

fn fixture() -> std::sync::Arc<dyn Session> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../volna/examples/pipeline_showcase.vtr");
    OpenSpec::Path(path).open().unwrap()
}

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

fn frame(app: &mut App, panel: volna_core::panels::PanelId) {
    let bounds = Rect::from_xywh(0.0, 0.0, 1000.0, 500.0);
    assert!(matches!(
        app.layout_panel(panel, bounds, &Theme::one_dark(), 1.0, &mut MonoMeasure),
        Some(PanelLayout::Table(_))
    ));
    app.render_panel(panel, &Theme::one_dark(), &mut MonoMeasure);
}

fn log_fixture() -> std::sync::Arc<dyn Session> {
    use vtr::Recorder;
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let logger = writer
        .logger(vtr::SourceOptions::new("logs", None))
        .unwrap();
    for (time, severity, value) in [(10, 3, "zebra"), (20, 2, "alpha"), (30, 4, "omega")] {
        let callsite = vtr::logging::declare::<String>(
            &mut writer,
            vtr::logging::CallsiteOptions {
                severity,
                ..vtr::logging::CallsiteOptions::new("message: {}")
            },
        )
        .unwrap();
        writer.advance_time(time).unwrap();
        writer
            .log(logger, callsite, &value.to_owned(), None)
            .unwrap();
    }
    writer.close().unwrap();
    OpenSpec::Path(file.path().into()).open().unwrap()
}

fn finish_query(app: &mut App, panel: volna_core::panels::PanelId) {
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        app.tick(Instant::now());
        let table = app.panels.get(panel).unwrap().kind.table().unwrap();
        assert!(table.query_error.is_none(), "{:?}", table.query_error);
        if !table.querying() {
            break;
        }
        assert!(Instant::now() < deadline, "table query did not finish");
        std::thread::yield_now();
    }
    frame(app, panel);
}

#[test]
fn logs_can_explicitly_open_a_table_with_formatted_filterable_sortable_columns() {
    let mut app = App::new();
    app.set_session(log_fixture());
    app.handle(Command::OpenTable {
        selected: a_all(vec![Member::Generator(0)]),
        clicked: None,
    });
    let panel = app.panels.focused_id();
    pump(&mut app);
    frame(&mut app, panel);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert_eq!(table.len(), 3);
    assert_eq!(
        table
            .layout
            .columns
            .iter()
            .map(|c| table.column_title(c.source_index))
            .collect::<Vec<_>>(),
        ["Time", "Severity", "Message"]
    );
    assert_eq!(
        table.window.rows[0]
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        ["10", "warn (3)", "message: zebra"]
    );
    let message = table.layout.columns[2].source_index;
    let severity = table.layout.columns[1].source_index;
    assert!(table.layout.columns[2].rect.width() > table.layout.body.width() / 2.0);
    assert!((table.layout.content_width - table.layout.body.width()).abs() < 0.01);

    app.handle(Command::Table(panel, TableCommand::SortColumn(severity)));
    finish_query(&mut app, panel);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert_eq!(
        (0..3).map(|row| table.row_time(row)).collect::<Vec<_>>(),
        [Some(20), Some(10), Some(30)]
    );
    app.handle(Command::Table(panel, TableCommand::SortColumn(message)));
    app.handle(Command::Table(
        panel,
        TableCommand::SetFilter {
            source_index: message,
            filter: Some(volna_core::table::filter::ColumnFilter {
                query: "message:".into(),
                ..Default::default()
            }),
        },
    ));
    finish_query(&mut app, panel);
    assert_eq!(
        app.panels.get(panel).unwrap().kind.table().unwrap().len(),
        3
    );
    app.handle(Command::Table(
        panel,
        TableCommand::SetFilter {
            source_index: severity,
            filter: Some(volna_core::table::filter::ColumnFilter {
                query: ">= 3".into(),
                mode: volna_core::table::filter::FilterMode::Expression,
                ..Default::default()
            }),
        },
    ));
    finish_query(&mut app, panel);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert_eq!(table.len(), 2);
    assert_eq!(
        table
            .window
            .rows
            .iter()
            .map(|r| r.cells[2].text.as_str())
            .collect::<Vec<_>>(),
        ["message: omega", "message: zebra"]
    );
    app.handle(Command::Table(panel, TableCommand::ClearFilters));
    finish_query(&mut app, panel);
    assert_eq!(
        app.panels.get(panel).unwrap().kind.table().unwrap().len(),
        3
    );
}

#[test]
fn log_table_customization_survives_waveform_activation_restore_and_reset_undo() {
    let mut app = App::new();
    app.set_session(log_fixture());
    app.handle(Command::OpenTable {
        selected: a_all(vec![Member::Generator(0)]),
        clicked: None,
    });
    let panel = app.panels.focused_id();
    pump(&mut app);
    frame(&mut app, panel);
    let message = app
        .panels
        .get(panel)
        .unwrap()
        .kind
        .table()
        .unwrap()
        .layout
        .columns[2]
        .source_index;
    for command in [
        TableCommand::ResizeColumn {
            source_index: message,
            width: 250.0,
        },
        TableCommand::MoveColumn {
            source_index: message,
            before: Some(1),
        },
        TableCommand::ToggleTransactionColumn(TransactionColumn::Id),
    ] {
        app.handle(Command::Table(panel, command));
    }
    let columns = app
        .panels
        .get(panel)
        .unwrap()
        .kind
        .table()
        .unwrap()
        .columns
        .get()
        .clone();
    let count = app.panels.len();
    app.handle(Command::ActivateMembers(a_all(vec![
        Member::Generator(0),
        Member::Generator(0),
    ])));
    assert_eq!(app.panels.len(), count + 1);
    assert!(app.panels.focused().kind.waves().is_some());
    let saved = Workspace::capture(
        &app,
        volna_core::testing::paths("file:///tmp/logs.vtr"),
        None,
    )
    .unwrap();
    Workspace::parse(&saved.to_bytes().unwrap())
        .unwrap()
        .prepare(
            &app,
            "file:///tmp/logs.vtr",
            "file:///tmp/logs.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    pump(&mut app);
    frame(&mut app, panel);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert_eq!(table.columns.get(), &columns);
    assert_eq!(table.column_title(message), "Message");
    assert_eq!(table.layout.columns[0].rect.width(), 250.0);
    app.handle(Command::Table(panel, TableCommand::ResetColumns));
    frame(&mut app, panel);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert_eq!(
        table
            .layout
            .columns
            .iter()
            .map(|c| table.column_title(c.source_index))
            .collect::<Vec<_>>(),
        ["Time", "Severity", "Message"]
    );
    assert!(table.layout.columns[2].rect.width() > table.layout.body.width() / 2.0);
    app.handle(Command::Undo);
    frame(&mut app, panel);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert_eq!(table.columns.get(), &columns);
    assert_eq!(table.widths[message], Some(250.0));
}

#[test]
fn log_table_keeps_generic_attribute_sizing_when_message_is_hidden_or_resized() {
    for hide_message in [false, true] {
        let mut app = App::new();
        app.set_session(log_fixture());
        app.handle(Command::OpenTable {
            selected: a_all(vec![Member::Generator(0)]),
            clicked: None,
        });
        let panel = app.panels.focused_id();
        pump(&mut app);
        frame(&mut app, panel);
        let message = app
            .panels
            .get(panel)
            .unwrap()
            .kind
            .table()
            .unwrap()
            .layout
            .columns[2]
            .source_index;
        let command = if hide_message {
            TableCommand::ToggleTransactionColumn(TransactionColumn::Attribute(
                "log.message".into(),
            ))
        } else {
            TableCommand::ResizeColumn {
                source_index: message,
                width: 200.0,
            }
        };
        app.handle(Command::Table(panel, command));
        app.handle(Command::Table(
            panel,
            TableCommand::ShowTransactionColumn(TransactionColumn::Attributes),
        ));
        frame(&mut app, panel);
        let table = app.panels.get(panel).unwrap().kind.table().unwrap();
        assert!((table.layout.content_width - table.layout.body.width()).abs() < 0.01);
        if !hide_message {
            assert_eq!(
                table
                    .layout
                    .columns
                    .iter()
                    .find(|c| c.source_index == message)
                    .unwrap()
                    .rect
                    .width(),
                200.0
            );
        }
        let attributes = table
            .layout
            .columns
            .iter()
            .find(|c| table.column_title(c.source_index) == "Attributes · preview")
            .unwrap()
            .source_index;
        app.handle(Command::Table(
            panel,
            TableCommand::ResizeColumn {
                source_index: attributes,
                width: 200.0,
            },
        ));
        frame(&mut app, panel);
        let table = app.panels.get(panel).unwrap().kind.table().unwrap();
        assert!(table.layout.content_width < table.layout.body.width());
    }
}

#[test]
fn generator_table_loads_selects_follows_copies_and_keeps_bounded_rows() {
    let session = fixture();
    let mut app = App::new();
    app.set_session(session.clone());
    pump(&mut app);
    let budget = app
        .doc
        .session(TraceId::A)
        .unwrap()
        .memory_budget()
        .unwrap();
    let session_bytes = budget.used();
    assert!(session_bytes > 0, "the native VTR owner must be admitted");
    app.handle(Command::OpenTable {
        selected: a_all(vec![Member::Generator(0)]),
        clicked: None,
    });
    let panel = app.panels.focused_id();
    assert!(matches!(
        app.take_requests().as_slice(),
        [LoadRequest::Trace(TraceRequest::Track { .. })]
    ));
    app.handle(Command::Table(panel, TableCommand::Cancel));
    app.handle(Command::Table(panel, TableCommand::Retry));
    pump(&mut app);
    assert!(
        budget.used() > session_bytes + volna_core::table::model::PANEL_BYTES,
        "the native track and table must share the session ledger"
    );
    frame(&mut app, panel);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert!(matches!(table.state, TableState::Ready));
    assert!(table.len() > 20);
    assert!(table.window.rows.len() <= 256);
    assert!(matches!(table.source, TableSource::Generator(_)));

    let now = Instant::now();
    app.handle_at(Command::Table(panel, TableCommand::GoTo(10)), now);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert!(matches!(table.selected, Some(RowIdentity::Transaction(_))));
    assert_eq!(app.doc.shared.cursor, table.row_time(10));
    assert!(
        table
            .copy_tsv()
            .unwrap()
            .starts_with("Generator\tID\tBegin\tEnd\tDuration\tStatus\n")
    );

    let selection = app.doc.selection().expect("the row is the selection");
    assert_eq!(selection.origin, panel);
    assert_eq!(
        Some(volna_core::table::RowIdentity::Transaction(selection.id)),
        app.panels
            .get(panel)
            .unwrap()
            .kind
            .table()
            .unwrap()
            .selected
    );
    app.handle(Command::Table(
        panel,
        TableCommand::ToggleTransactionColumn(TransactionColumn::Id),
    ));
    frame(&mut app, panel);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert!(
        table.accessible_rows().count() <= volna_core::table::layout::MAX_PREPARED_ROWS as usize
    );
    assert!(app.scene().texts().any(|text| text == "ID"));
}

#[test]
fn fixed_signal_set_builds_shared_change_axis_and_workspace_reopens_at_first_row() {
    let session = fixture();
    let mut app = App::new();
    app.set_session(session.clone());
    assert!(session.hierarchy().var_count() >= 2);
    app.handle(Command::OpenTable {
        selected: a_all(vec![Member::Var(0), Member::Var(1)]),
        clicked: None,
    });
    let panel = app.panels.focused_id();
    pump(&mut app);
    frame(&mut app, panel);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert!(matches!(table.source, TableSource::Signals(_)));
    assert!(!table.is_empty());
    let time = table.row_time(0).unwrap();
    app.handle(Command::Table(panel, TableCommand::Select(0)));
    assert_eq!(app.doc.shared.cursor, Some(time));
    let tsv = app
        .panels
        .get(panel)
        .unwrap()
        .kind
        .table()
        .unwrap()
        .copy_tsv()
        .unwrap();
    assert!(tsv.starts_with("Time\t"));
    app.handle(Command::Table(
        panel,
        TableCommand::NumberFormat("bin".into()),
    ));
    app.handle(Command::Table(
        panel,
        TableCommand::ColumnFormat {
            source_index: 1,
            translator: Some("udec".into()),
        },
    ));
    let formats = app
        .panels
        .get(panel)
        .unwrap()
        .kind
        .table()
        .unwrap()
        .formats
        .get()
        .clone();
    assert_eq!(formats.numbers, "bin");
    assert_eq!(formats.columns[1].as_deref(), Some("udec"));
    app.handle(Command::Undo);
    assert!(
        app.panels
            .get(panel)
            .unwrap()
            .kind
            .table()
            .unwrap()
            .formats
            .columns[1]
            .is_none()
    );
    app.handle(Command::Redo);
    assert_eq!(
        app.panels
            .get(panel)
            .unwrap()
            .kind
            .table()
            .unwrap()
            .formats
            .get(),
        &formats
    );
    app.handle(Command::Table(
        panel,
        TableCommand::ResizeColumn {
            source_index: 1,
            width: 750.0,
        },
    ));
    app.handle(Command::Table(panel, TableCommand::ToggleSignalColumn(1)));

    let filter = volna_core::table::filter::ColumnFilter {
        query: ">= 0".into(),
        mode: volna_core::table::filter::FilterMode::Expression,
        ..Default::default()
    };
    app.handle(Command::Table(
        panel,
        TableCommand::SetFilter {
            source_index: 0,
            filter: Some(filter.clone()),
        },
    ));
    app.handle(Command::Table(panel, TableCommand::ToggleFilters));

    let saved = Workspace::capture(
        &app,
        volna_core::testing::paths("file:///tmp/trace.vtr"),
        None,
    )
    .unwrap();
    let bytes = saved.to_bytes().unwrap();
    fn contains_table_v6(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(object) => {
                (object.get("kind").and_then(|value| value.as_str()) == Some("table")
                    && object.get("version").and_then(serde_json::Value::as_u64) == Some(6))
                    || object.values().any(contains_table_v6)
            }
            serde_json::Value::Array(values) => values.iter().any(contains_table_v6),
            _ => false,
        }
    }
    let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(contains_table_v6(&raw));
    let table = raw["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == "table")
        .unwrap();
    assert_eq!(table["filters_visible"], true);
    assert_eq!(table["filters"][0], serde_json::to_value(&filter).unwrap());
    let parsed = Workspace::parse(&bytes).unwrap();
    let plan = parsed
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/session.volna.json",
        )
        .unwrap();
    plan.commit(&mut app).unwrap();
    pump(&mut app);
    let restored = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert!(matches!(restored.source, TableSource::Signals(_)));
    assert_eq!(restored.viewport.top, 0);
    assert_eq!(restored.selected, None);
    assert_eq!(restored.widths[1], Some(750.0));
    assert_eq!(restored.formats.get(), &formats);
    app.handle(Command::Table(panel, TableCommand::ToggleSignalColumn(1)));
    frame(&mut app, panel);
    assert_eq!(
        app.panels
            .get(panel)
            .unwrap()
            .kind
            .table()
            .unwrap()
            .layout
            .columns[1]
            .rect
            .width(),
        750.0
    );
}

#[test]
fn single_signal_table_ignores_out_of_range_rows_and_next_starts_at_the_first() {
    let session = fixture();
    let mut app = App::new();
    app.set_session(session.clone());
    app.handle(Command::OpenTable {
        selected: a_all(vec![Member::Var(0)]),
        clicked: None,
    });
    let panel = app.panels.focused_id();
    pump(&mut app);
    frame(&mut app, panel);
    let len = app.panels.get(panel).unwrap().kind.table().unwrap().len();
    assert!(len > 1);
    let now = Instant::now();
    app.handle_at(Command::Table(panel, TableCommand::GoTo(len + 3)), now);
    app.handle_at(Command::Table(panel, TableCommand::Select(len)), now);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert_eq!(table.selected, None);
    assert_eq!(table.row_time(len), None);

    app.handle_at(Command::Table(panel, TableCommand::Next), now);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert_eq!(table.selected_ordinal(), Some(0));
}

#[test]
fn attribute_layout_roundtrips_hidden_keys_widths_order_and_undo() {
    let mut app = App::new();
    app.set_session(fixture());
    app.handle(Command::OpenTable {
        selected: a_all(vec![Member::Generator(0)]),
        clicked: None,
    });
    let panel = app.panels.focused_id();
    pump(&mut app);
    let key = app
        .panels
        .get(panel)
        .unwrap()
        .kind
        .table()
        .unwrap()
        .loaded_generator()
        .unwrap()
        .attribute_catalogue()
        .iter()
        .find(|a| a.key != "vtr.label")
        .unwrap()
        .key
        .clone();
    let attribute = TransactionColumn::Attribute(key);
    app.handle(Command::Table(
        panel,
        TableCommand::ToggleTransactionColumn(attribute.clone()),
    ));
    app.handle(Command::Table(
        panel,
        TableCommand::ResizeColumn {
            source_index: 7,
            width: 257.0,
        },
    ));
    app.handle(Command::Table(
        panel,
        TableCommand::MoveColumn {
            source_index: 7,
            before: Some(1),
        },
    ));
    app.handle(Command::Table(
        panel,
        TableCommand::ToggleTransactionColumn(attribute.clone()),
    ));
    let expected = app
        .panels
        .get(panel)
        .unwrap()
        .kind
        .table()
        .unwrap()
        .columns
        .get()
        .clone();
    let bytes = Workspace::capture(
        &app,
        volna_core::testing::paths("file:///tmp/trace.vtr"),
        None,
    )
    .unwrap()
    .to_bytes()
    .unwrap();
    let saved = Workspace::parse(&bytes).unwrap();
    saved
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/session.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    pump(&mut app);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert_eq!(table.columns.get(), &expected);
    assert_eq!(table.widths[7], Some(257.0));
    app.handle(Command::Table(
        panel,
        TableCommand::ToggleTransactionColumn(attribute.clone()),
    ));
    app.handle(Command::Undo);
    assert!(
        !app.panels
            .get(panel)
            .unwrap()
            .kind
            .table()
            .unwrap()
            .columns
            .transaction_visible(attribute.clone())
    );
    app.handle(Command::Redo);
    frame(&mut app, panel);
    let table = app.panels.get(panel).unwrap().kind.table().unwrap();
    assert!(table.columns.transaction_visible(attribute));
    assert_eq!(table.layout.columns[1].source_index, 7);
    assert_eq!(table.layout.columns[1].rect.width(), 257.0);
    app.handle(Command::Table(panel, TableCommand::ResetColumns));
    app.handle(Command::Undo);
    frame(&mut app, panel);
    assert_eq!(
        app.panels.get(panel).unwrap().kind.table().unwrap().widths[7],
        Some(257.0)
    );
}

#[test]
fn transaction_attribute_shortcut_reuses_a_table_and_never_toggles_it_off() {
    let mut app = App::new();
    app.set_session(fixture());
    app.handle(Command::OpenTable {
        selected: a_all(vec![Member::Generator(0)]),
        clicked: None,
    });
    let table_panel = app.panels.focused_id();
    pump(&mut app);
    app.handle(Command::Table(table_panel, TableCommand::Select(0)));
    app.handle(Command::ShowTransaction { from: table_panel });
    let inspector = app
        .panels
        .iter()
        .find(|p| p.kind.transaction().is_some())
        .unwrap()
        .id;
    for _ in 0..2 {
        app.handle(Command::ShowAttributeColumn {
            from: inspector,
            key: "status".into(),
        });
        assert_eq!(app.panels.focused_id(), table_panel);
        assert_eq!(
            app.panels
                .iter()
                .filter(|p| p.kind.table().is_some())
                .count(),
            1
        );
        let table = app.panels.get(table_panel).unwrap().kind.table().unwrap();
        assert!(
            table
                .columns
                .transaction_visible(TransactionColumn::Attribute("status".into()))
        );
        assert!(table.columns.transaction_visible(TransactionColumn::Status));
    }
    app.handle(Command::Panels(volna_core::panels::PanelsCommand::Close(
        table_panel,
    )));
    app.handle(Command::ShowAttributeColumn {
        from: inspector,
        key: "vtr.label".into(),
    });
    pump(&mut app);
    let table = app
        .panels
        .focused()
        .kind
        .table()
        .expect("opens a table from details");
    assert!(table.columns.transaction_visible(TransactionColumn::Label));
    assert_eq!(table.columns.len(), 7);
}
