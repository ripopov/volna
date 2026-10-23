//! The landing fixture restores its checked-in workspace over its checked-in trace without
//! a notice, loads its data panels, reuses its transaction panel for selections, and saves
//! back to the same bytes.

use std::path::PathBuf;

use volna_core::geometry::Rect;
use volna_core::panels::PanelId;
use volna_core::pipeline::Rows;
use volna_core::scene::MonoMeasure;
use volna_core::transaction::TxPanelState;
use volna_core::workspace::Workspace;
use volna_core::{App, Command, Instant, Theme};
use volna_trace::session::OpenSpec;

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../volna/examples")
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

fn landing() -> App {
    let trace = examples().join("landing.vtr").canonicalize().unwrap();
    let workspace = examples().join("landing-page.volna.json");
    let saved = std::fs::read(&workspace).unwrap();
    let trace_uri = url::Url::from_file_path(&trace).unwrap().to_string();
    let location = url::Url::from_file_path(workspace).unwrap().to_string();
    let session = OpenSpec::Path(trace.clone()).open().unwrap();
    let mut app = App::new();
    app.open_resource(OpenSpec::Path(trace), trace_uri.clone());
    volna_core::testing::complete_open(&mut app, session);
    Workspace::parse(&saved)
        .unwrap()
        .prepare(&app, &trace_uri, &location)
        .unwrap()
        .commit(&mut app)
        .unwrap();
    pump(&mut app);
    app
}

#[test]
fn landing_waveforms_render_at_full_range_and_saved_zoom() {
    let mut app = landing();
    let theme = Theme::volna(true);
    let bounds = Rect::from_xywh(0.0, 0.0, 1200.0, 640.0);
    let (first, last) = app.doc.limits();
    let saved_viewport = app.doc.shared.viewport.value;
    let render = |app: &mut App| {
        app.layout_panel(PanelId(39), bounds, &theme, 1.0).unwrap();
        app.render_panel(PanelId(39), &theme, &mut MonoMeasure);
        pump(app);
        app.layout_panel(PanelId(39), bounds, &theme, 1.0).unwrap();
        let scene = app.render_panel(PanelId(39), &theme, &mut MonoMeasure);
        let texts: Vec<String> = scene.texts().map(str::to_owned).collect();
        let quads: Vec<_> = scene.quads().collect();
        (texts, quads)
    };
    app.doc.shared.viewport.value.start = first as f64;
    app.doc.shared.viewport.value.end = last as f64;
    let (texts, quads) = render(&mut app);
    assert!(
        quads.iter().any(|(_, c)| (c.a - 0.22).abs() < 1e-3),
        "state rows are tinted"
    );
    assert!(
        texts.iter().any(|t| t.starts_with('…') && t.len() > 3),
        "a number keeps its last digits: {texts:?}"
    );
    for text in &texts {
        assert_ne!(text, "…", "a clipped value must retain a digit");
    }
    app.doc.shared.viewport.value = saved_viewport;
    let (texts, _) = render(&mut app);
    assert!(
        texts.iter().any(|t| t == "\"dot product loop\""),
        "{texts:?}"
    );
}

#[test]
fn the_landing_workspace_restores_every_panel_and_saves_the_same_bytes() {
    let trace = examples().join("landing.vtr").canonicalize().unwrap();
    let workspace = examples().join("landing-page.volna.json");
    let saved = std::fs::read(&workspace).unwrap();
    let trace_uri = url::Url::from_file_path(&trace).unwrap().to_string();
    let location = url::Url::from_file_path(workspace).unwrap().to_string();
    let session = OpenSpec::Path(trace.clone()).open().unwrap();
    let mut app = App::new();
    app.open_resource(OpenSpec::Path(trace), trace_uri.clone());
    volna_core::testing::complete_open(&mut app, session);

    let plan = Workspace::parse(&saved)
        .unwrap()
        .prepare(&app, &trace_uri, &location)
        .unwrap();
    assert_eq!(plan.report().notices, Vec::<String>::new());
    plan.commit(&mut app).unwrap();
    pump(&mut app);
    let theme = Theme::one_dark();
    let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 420.0);
    for id in [39, 38, 40] {
        app.layout_panel(PanelId(id), bounds, &theme, 1.0).unwrap();
        app.render_panel(PanelId(id), &theme, &mut MonoMeasure);
    }
    pump(&mut app);
    app.tick(Instant::now());

    let waves = app.panels.waves(PanelId(39)).unwrap();
    let signals = waves
        .items()
        .iter()
        .filter(|i| i.signal().is_some())
        .count();
    assert_eq!(signals, 13);
    assert_eq!(waves.loaded_count(), signals, "every signal row loaded");
    app.layout_panel(PanelId(39), bounds, &theme, 1.0).unwrap();
    let texts: Vec<String> = app
        .render_panel(PanelId(39), &theme, &mut MonoMeasure)
        .texts()
        .map(str::to_owned)
        .collect();
    for text in ["ipc", "req_valid", "req_addr", "pc", "phase", "irq"] {
        assert!(
            texts.iter().any(|t| t == text),
            "the waves panel shows {text:?}: {texts:?}"
        );
    }
    match app.panels.pipeline(PanelId(38)).unwrap().rows(&app.doc) {
        Rows::Ready(rows) => assert_eq!(rows.len(), 420),
        _ => panic!("the pipeline rows are not ready"),
    }
    let initial_record = match app.panels.transaction(PanelId(37)).unwrap().state(&app.doc) {
        TxPanelState::Ready(view) => {
            assert_eq!(
                view.identity.label.as_deref(),
                Some("80000054: addi a4, a4, 4")
            );
            view.identity.id
        }
        _ => panic!("the initial record is not ready"),
    };
    assert_eq!(
        app.panels
            .get(PanelId(40))
            .unwrap()
            .kind
            .table()
            .unwrap()
            .len(),
        420
    );
    let markers: Vec<_> = app
        .doc
        .markers()
        .iter()
        .map(|m| (m.time, m.label.as_deref()))
        .collect();
    assert_eq!(markers, [(51, None), (63, None)]);

    let again = Workspace::capture(&app, volna_core::testing::paths("landing.vtr"), None).unwrap();
    assert_eq!(
        String::from_utf8(again.to_bytes().unwrap()).unwrap(),
        String::from_utf8(saved).unwrap(),
        "the landing workspace must round-trip without changes"
    );

    let (track, id) = match app.panels.pipeline(PanelId(38)).unwrap().rows(&app.doc) {
        Rows::Ready(rows) => {
            let record = rows.get(191).unwrap().1;
            (volna_core::testing::a(record.generator), record.id)
        }
        _ => panic!("the pipeline rows are not ready"),
    };
    assert_ne!(id, initial_record);
    app.handle(Command::SelectTransaction {
        panel: PanelId(38),
        track,
        id,
        cursor: None,
    });
    app.handle(Command::ShowTransaction { from: PanelId(38) });
    pump(&mut app);
    assert_eq!(app.panels.focused_id(), PanelId(37));
    assert_eq!(app.panels.len(), 4, "reuse the existing transaction panel");
    match app.panels.transaction(PanelId(37)).unwrap().state(&app.doc) {
        TxPanelState::Ready(view) => assert_eq!(view.identity.id, id),
        _ => panic!("the selected record is not ready"),
    }
}
