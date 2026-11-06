use std::sync::Arc;
use volna_core::app::{App, Command, Event};
use volna_core::geometry::Modifiers;
use volna_core::icons::IconName;
use volna_core::sidebar::TreeNode;
use volna_core::sidebar::icons::{Tint, member_icon, scope_icon, scope_kind_icon};
use volna_core::sidebar::{Key, MemberListModel, ScopeTreeModel};
use volna_core::testing::{a, hierarchy_document};
use volna_trace::data::source::Lookup;
use volna_trace::data::{Hierarchy, Member};
use volna_trace::remote::objects::Metadata;
use volna_trace::session::LoadRequest as TraceRequest;
use volna_trace::session::{OpenSpec, Session};

fn fixture() -> Arc<dyn Session> {
    use vtr::Recorder;
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut w = vtr::Writer::create(file.path()).unwrap();
    let root = w
        .scope_with(
            vtr::SourceOptions::new("soc", None),
            vtr::ScopeType::Module,
            "soc_top",
        )
        .unwrap();
    let entries = [("IDLE", "0"), ("BUSY", "1")]
        .map(|(name, bits)| (w.intern(name).unwrap(), w.intern(bits).unwrap()));
    let table = w
        .enum_table(
            vtr::SourceOptions::new("state_t", Some(root)),
            entries.to_vec(),
        )
        .unwrap();
    let mut signal = vtr::SourceOptions::new("read_valid", Some(root));
    signal.attributes.push(vtr::Attribute {
        key: w.intern("enum_table").unwrap(),
        value: vtr::Value::U64(u64::from(table.0)),
    });
    let state = w
        .signal(
            signal,
            vtr::SignalKind::bits(1, vtr::LogicStates::Four),
            vtr::VarType::Wire,
            vtr::Direction::Input,
        )
        .unwrap();
    let cpu = w
        .scope_with(
            vtr::SourceOptions::new("cpu", Some(root)),
            vtr::ScopeType::Core,
            "",
        )
        .unwrap();
    let stream = w
        .scope_with(
            vtr::SourceOptions::new("thread0", Some(cpu)),
            vtr::ScopeType::Generic,
            "",
        )
        .unwrap();
    w.transactions::<()>(
        vtr::SourceOptions::new("instructions", Some(stream)),
        "PIPELINE",
    )
    .unwrap();
    let bus = w
        .scope_with(
            vtr::SourceOptions::new("read_bus", Some(root)),
            vtr::ScopeType::Generic,
            "",
        )
        .unwrap();
    w.transactions::<()>(
        vtr::SourceOptions::new("read_request", Some(bus)),
        "TRANSACTOR",
    )
    .unwrap();
    w.transactions::<()>(
        vtr::SourceOptions::new("write_request", Some(bus)),
        "TRANSACTOR",
    )
    .unwrap();
    let log = w
        .scope_with(
            vtr::SourceOptions::new("log", Some(root)),
            vtr::ScopeType::Generic,
            "",
        )
        .unwrap();
    w.logger(vtr::SourceOptions::new("read_failures", Some(log)))
        .unwrap();
    vtr::logging::declare::<u64>(
        &mut w,
        vtr::logging::CallsiteOptions {
            severity: 4,
            file: Some("soc.sv".into()),
            line: Some(42),
            ..vtr::logging::CallsiteOptions::new("read_failed at pc={:x}")
        },
    )
    .unwrap();
    w.scope_with(
        vtr::SourceOptions::new("empty", Some(root)),
        vtr::ScopeType::Generic,
        "",
    )
    .unwrap();
    w.emit_u64(state, 0).unwrap();
    w.advance_time(10).unwrap();
    w.emit_u64(state, 1).unwrap();
    w.advance_time(20).unwrap();
    w.close().unwrap();
    if let Some(path) = std::env::var_os("VOLNA_HIERARCHY_FIXTURE") {
        std::fs::copy(file.path(), path).unwrap();
    }
    OpenSpec::Path(file.path().into()).open().unwrap()
}

fn scope(h: &Hierarchy, path: &[&str]) -> usize {
    match h.find_scope(path) {
        Lookup::Found(id) => id,
        other => panic!("{other:?}"),
    }
}

#[test]
fn mixed_vtr_metadata_icons_and_logger_identity() {
    let session = fixture();
    let h = session.hierarchy();
    let pipeline = scope(h, &["soc", "cpu", "thread0"]);
    assert_eq!(
        scope_icon(&h.scope(pipeline)),
        (IconName::Folder, Tint::None)
    );
    assert_eq!(h.scope(0).component, "soc_top");
    let mut doc = volna_core::Document::new();
    doc.set_session(session.clone());
    let traces = doc.traces();
    let mut list = MemberListModel::default();
    list.set_scope(traces, Some(a(pipeline)));
    assert_eq!(list.title(traces), "Members");
    assert_eq!(list.rows, [a(Member::Generator(0))]);
    assert_eq!(member_icon(h, list.rows[0].item), IconName::Workflow);
    assert_eq!(member_icon(h, Member::Var(0)), IconName::Tags);
    assert_eq!(
        h.find_generator(&["soc", "cpu", "thread0", "instructions"]),
        Lookup::Found(0)
    );
    for member in h
        .generators()
        .iter()
        .enumerate()
        .map(|(id, _)| Member::Generator(id))
    {
        let track = session
            .tracks()
            .iter()
            .find(|t| Some(t.id) == h.member_track(member))
            .unwrap();
        assert_eq!(track.path.join("."), h.member_path(member));
    }
    let bus = scope(h, &["soc", "read_bus"]);
    assert_eq!(
        h.generators()[h.scope(bus).generators.iter().next().unwrap()].kind,
        "TRANSACTOR"
    );
    let log = scope(h, &["soc", "log"]);
    list.set_scope(traces, Some(a(log)));
    assert_eq!(list.title(traces), "Members");
    assert!(h.is_log(list.rows[0].item));
    assert_eq!(h.member_name(list.rows[0].item), "read_failures");
    list.set_scope(traces, Some(a(scope(h, &["soc", "empty"]))));
    assert_eq!(list.placeholder(traces), Some("No members in this scope"));
    Metadata::from_session(session.as_ref()).validate().unwrap();
}

#[test]
fn search_order_activation_and_keyboard() {
    let session = fixture();
    let mut app = App::new();
    app.set_session(session.clone());
    app.handle(Command::SetSearchEverywhere(true));
    app.handle(Command::SetFilter("read_".into()));
    let rows = app.variables.rows.clone();
    let members: Vec<Member> = rows.iter().map(|m| m.item).collect();
    assert!(matches!(
        members.as_slice(),
        [Member::Var(_), Member::Generator(_), Member::Generator(_)]
    ));
    assert!(app.variables.show_scope());
    app.handle(Command::SelectVar {
        ix: 0,
        modifiers: Modifiers::default(),
    });
    app.handle(Command::AddSelectedOrAllVars);
    assert_eq!(app.panels.focused_waves().unwrap().items().len(), 1);
    app.take_requests();
    app.take_events();
    app.handle(Command::ActivateMembers(vec![rows[1], rows[2]]));
    let requests = app.take_requests();
    assert!(matches!(
        requests.as_slice(),
        [
            volna_core::session::LoadRequest::Trace(TraceRequest::Track { .. }),
            volna_core::session::LoadRequest::Trace(TraceRequest::Track { .. })
        ]
    ));
    assert_eq!(app.panels.len(), 1);
    assert_eq!(app.panels.focused_waves().unwrap().items().len(), 3);
    assert!(app.panels.iter().all(|panel| panel.kind.waves().is_some()));
    assert!(app.status().sidebar_notice.is_none());
    assert!(
        !app.take_events()
            .iter()
            .any(|e| matches!(e, Event::Notice(_)))
    );
    let pipeline = scope(session.hierarchy(), &["soc", "cpu", "thread0"]);
    app.handle(Command::SelectScope(TreeNode::scope(a(pipeline))));
    assert!(!app.variables.search_everywhere);
    app.handle(Command::SetFilter(String::new()));
    app.handle(Command::ScopesKey(Key::Enter));
    assert_eq!(app.panels.len(), 1);
    app.handle(Command::ActivateMembers(vec![a(Member::Generator(0))]));
    assert!(app.status().sidebar_notice.is_none());
    assert_eq!(app.panels.len(), 1);
    assert_eq!(app.panels.focused_waves().unwrap().items().len(), 4);
    assert_eq!(
        app.panels.focused_waves().unwrap().items()[3].name(),
        "instructions"
    );
    app.handle(Command::ScopesKey(Key::Left));
    assert_eq!(
        app.scopes.selected,
        Some(TreeNode::scope(a(scope(
            session.hierarchy(),
            &["soc", "cpu"]
        ))))
    );
    app.handle(Command::SetFilter("read_".into()));
    app.handle(Command::VariablesKey(Key::Escape, Modifiers::default()));
    assert!(app.variables.filter.is_empty());
}

#[test]
fn bulk_add_includes_generators_in_scopes_and_filtered_results() {
    let session = fixture();
    for search in [false, true] {
        for command in [
            Command::AddAllVars,
            Command::AddSelectedOrAllVars,
            Command::VariablesKey(Key::Enter, Modifiers::default()),
        ] {
            let mut app = App::new();
            app.set_session(session.clone());
            if search {
                app.handle(Command::SetSearchEverywhere(true));
                app.handle(Command::SetFilter("read_".into()));
            } else {
                app.handle(Command::SelectScope(TreeNode::scope(a(scope(
                    session.hierarchy(),
                    &["soc", "read_bus"],
                )))));
            }
            let expected: Vec<_> = app
                .variables
                .rows
                .iter()
                .map(|m| session.hierarchy().member_name(m.item).to_owned())
                .collect();
            assert_eq!(expected.len(), if search { 3 } else { 2 });
            if matches!(command, Command::AddAllVars) {
                app.handle(Command::SelectVar {
                    ix: 0,
                    modifiers: Modifiers::default(),
                });
            }
            app.handle(command);
            let waves = app.panels.focused_waves().unwrap();
            let names: Vec<_> = waves.items().iter().map(|row| row.name()).collect();
            assert_eq!(names, expected);
            assert_eq!(
                waves
                    .items()
                    .iter()
                    .filter(|row| row.lane().is_some())
                    .count(),
                2
            );
            assert_eq!(app.panels.len(), 1);
            let revision = app.history.revision();
            app.handle(Command::AddAllVars);
            assert_eq!(app.history.revision(), revision);
            app.handle(Command::Undo);
            assert!(app.panels.focused().kind.is_start());
        }
    }
}

#[test]
fn search_cap_counts_all_kinds_and_deep_trees_are_iterative() {
    let session = fixture();
    let mut h = session.hierarchy().to_builder();
    let template = h.vars[0].clone();
    h.vars = vec![template; 5000];
    let mut list = MemberListModel::default();
    list.set_filter(hierarchy_document(h.clone()).traces(), "read_");
    assert_eq!(list.rows.len(), 5000);
    assert!(list.truncated);
    h.vars.truncate(4996);
    list.rebuild(hierarchy_document(h).traces());
    assert_eq!(list.rows.len(), 4998);
    assert!(!list.truncated);
    let mut h = volna_trace::data::HierarchyBuilder::default();
    let mut parent = None;
    for _ in 0..20000 {
        parent = Some(h.push_scope("nested".into(), "module".into(), parent));
    }
    let mut tree = ScopeTreeModel::default();
    tree.set_all(hierarchy_document(h).traces(), true);
    assert_eq!(
        tree.visible.last(),
        Some(&(TreeNode::scope(a(19999)), 19999))
    );
}

#[test]
fn expand_all_keeps_gate_leaves_out_of_state_and_saved_paths() {
    use volna_core::workspace::Workspace;

    let mut h = volna_trace::data::HierarchyBuilder::default();
    let root = h.push_scope("top".into(), "module".into(), None);
    let module = h.push_scope("module".into(), "module".into(), Some(root));
    for id in 0..100_000 {
        h.push_scope(format!("gate{id}"), "module".into(), Some(module));
    }
    let leaf = h.scopes.len() - 1;
    let doc = hierarchy_document(h);
    let session = doc.traces().first_loaded().unwrap().1.clone();
    let mut app = App::new();
    app.set_session(session);
    assert_eq!(app.scopes.expanded().count(), 2);

    app.handle(Command::ExpandAllScopes(true));
    assert_eq!(app.scopes.visible.len(), 100_002);
    assert_eq!(app.scopes.expanded().count(), 2);
    assert!(!app.scopes.is_expanded(TreeNode::scope(a(leaf))));
    let save =
        |app: &App| Workspace::capture(app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    let mut saved = save(&app);
    assert_eq!(saved.sidebar.expanded.len(), 2);
    assert!(saved.to_bytes().unwrap().len() < 10_000);

    app.handle(Command::ToggleScope(TreeNode::scope(a(leaf))));
    assert_eq!(app.scopes.expanded().count(), 2);
    app.handle(Command::ExpandAllScopes(false));
    assert_eq!(app.scopes.visible, [(TreeNode::scope(a(root)), 0)]);
    saved
        .sidebar
        .expanded
        .push(a(vec!["top".into(), "module".into(), "gate99999".into()]));
    saved
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    assert_eq!(app.scopes.visible.len(), 100_002);
    assert_eq!(app.scopes.expanded().count(), 2);
    assert_eq!(save(&app).sidebar.expanded.len(), 2);
}

#[test]
fn stream_workspace_paths_roundtrip_and_unresolved_paths_survive() {
    use volna_core::workspace::Workspace;
    let session = fixture();
    let mut app = App::new();
    app.set_session(session.clone());
    app.handle(Command::SelectScope(TreeNode::scope(a(scope(
        session.hierarchy(),
        &["soc", "cpu", "thread0"],
    )))));
    app.handle(Command::ExpandAllScopes(true));
    let capture =
        |app: &App| Workspace::capture(app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    let saved = capture(&app);
    let before = serde_json::to_value(&saved).unwrap();
    app.handle(Command::SetSearchEverywhere(true));
    saved
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    assert_eq!(serde_json::to_value(capture(&app)).unwrap(), before);
    assert!(!app.variables.search_everywhere);
    let mut saved = before;
    saved["sidebar"]["selected_scope"] = serde_json::json!(["A", ["missing", "stream"]]);
    saved["sidebar"]["expanded"] = serde_json::json!([["A", ["missing", "stream"]]]);
    Workspace::parse(&serde_json::to_vec(&saved).unwrap())
        .unwrap()
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    assert_eq!(serde_json::to_value(capture(&app)).unwrap(), saved);
}

#[test]
fn icons_have_assets_and_malformed_track_references_are_rejected() {
    for code in 0..=255 {
        let icon = scope_kind_icon(vtr::ScopeType::from_code(code).name());
        assert!(!icon.svg().is_empty());
        assert_eq!(IconName::from_path(icon.path()), Some(icon));
    }
    for icon in IconName::ALL {
        assert!(!icon.svg().is_empty());
    }
    let session = fixture();
    let mut metadata = Metadata::from_session(session.as_ref());
    let mut builder = metadata.hierarchy.to_builder();
    builder.generators[0].track = volna_trace::data::transactions::TrackRef(u32::MAX);
    metadata.hierarchy = builder.finish();
    assert!(metadata.validate().is_err());
    let mut metadata = Metadata::from_session(session.as_ref());
    let mut builder = metadata.hierarchy.to_builder();
    builder.generators[0].kind = "wrong-kind".into();
    metadata.hierarchy = builder.finish();
    assert!(metadata.validate().is_err());
}

#[test]
fn enum_format_is_default_selectable_and_restored_with_its_table() {
    use volna_core::wave::model::MenuAction;
    use volna_core::workspace::Workspace;
    use volna_trace::data::WaveValue;

    let mut app = App::new();
    app.set_session(fixture());
    app.handle(Command::AddVars(vec![a(0)]));
    for request in app.take_requests() {
        app.deliver(request.perform());
    }
    let panel = app.panels.focused_id();
    for theme in [
        volna_core::theme::Theme::volna(true),
        volna_core::theme::Theme::volna(false),
    ] {
        app.layout_panel(
            panel,
            volna_core::geometry::Rect::from_xywh(0.0, 0.0, 1200.0, 600.0),
            &theme,
            1.0,
            &mut volna_core::scene::MonoMeasure,
        );
        let scene = app.render_panel(panel, &theme, &mut volna_core::scene::MonoMeasure);
        assert!(scene.texts().any(|text| text == "IDLE"));
        assert!(scene.texts().any(|text| text == "BUSY"));
    }
    let translator = |app: &App| {
        app.panels
            .focused_waves()
            .unwrap()
            .signal(0)
            .unwrap()
            .translator
            .clone()
    };
    let original = translator(&app);
    assert_eq!(original.id(), "enum");
    assert_eq!(
        translator(&app)
            .translate(&WaveValue::Bits("0".into()))
            .text,
        "IDLE"
    );
    let capture =
        |app: &App| Workspace::capture(app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    let saved = capture(&app);
    let panel = app.panels.focused_id();
    let choose = |app: &mut App, id: &str| {
        app.panels.focused_waves_mut().unwrap().open_signal_menu(
            &app.doc,
            0,
            volna_core::geometry::point(0.0, 0.0),
        );
        assert!(
            app.panels
                .focused_waves()
                .unwrap()
                .menu
                .as_ref()
                .unwrap()
                .items()
                .any(|i| i.action == MenuAction::Format(id.into()))
        );
        app.handle(Command::MenuSelect(panel, MenuAction::Format(id.into())));
    };
    choose(&mut app, "bit");
    assert_eq!(
        translator(&app)
            .translate(&WaveValue::Bits("0".into()))
            .text,
        "0"
    );
    choose(&mut app, "enum");
    assert!(Arc::ptr_eq(&original, &translator(&app)));
    assert_eq!(
        translator(&app)
            .translate(&WaveValue::Bits("1".into()))
            .text,
        "BUSY"
    );
    saved
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    assert_eq!(translator(&app).id(), "enum");
    assert!(Arc::ptr_eq(&original, &translator(&app)));
    assert_eq!(
        translator(&app)
            .translate(&WaveValue::Bits("1".into()))
            .text,
        "BUSY"
    );
}

#[test]
fn showcase_enum_aliases_and_unmapped_values_use_the_recorded_table() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../volna/examples/feature_showcase.vtr");
    let session = OpenSpec::Path(path).open().unwrap();
    let mut app = App::new();
    app.set_session(session.clone());
    let var = |name: &str| {
        session
            .hierarchy()
            .vars()
            .position(|v| v.name == name)
            .unwrap()
    };
    app.handle(Command::AddVars(vec![
        a(var("state")),
        a(var("state_alias")),
        a(var("state_probe")),
    ]));
    for request in app.take_requests() {
        app.deliver(request.perform());
    }
    let waves = app.panels.focused_waves().unwrap();
    for row in [1, 2] {
        assert!(Arc::ptr_eq(
            &waves.signal(0).unwrap().translator,
            &waves.signal(row).unwrap().translator
        ));
    }
    for row in [0, 1] {
        let signal = waves.signal(row).unwrap();
        let history = signal.history.as_ref().unwrap();
        for (time, label) in [(0, "BOOT"), (64, "RUN"), (768, "FAULT"), (896, "RECOVER")] {
            let value = history.value_view(history.index_at(time));
            assert_eq!(signal.translator.translate_view(&value, 64).text, label);
        }
    }
    assert!(Arc::ptr_eq(
        waves.signal(0).unwrap().history.as_ref().unwrap(),
        waves.signal(1).unwrap().history.as_ref().unwrap()
    ));
    let probe = waves.signal(2).unwrap();
    let history = probe.history.as_ref().unwrap();
    for (time, label) in [
        (64, "RUN"),
        (768, "7"),
        (800, "x"),
        (832, "z"),
        (896, "RECOVER"),
    ] {
        let value = history.value_view(history.index_at(time));
        assert_eq!(probe.translator.translate_view(&value, 64).text, label);
    }
}

#[test]
fn enum_translators_are_isolated_by_trace_and_release_on_replacement() {
    use volna_core::trace::Traced;
    use volna_trace::data::WaveValue;

    let mut app = App::new();
    let first = fixture();
    let mut other_hierarchy = first.hierarchy().to_builder();
    other_hierarchy.enum_tables[0].entries[0].0 = "OTHER_IDLE".into();
    let other = volna_core::testing::hierarchy_session(other_hierarchy);
    app.set_session(first);
    let (other_id, _) = app.doc.add_session(other).unwrap();
    app.handle(Command::AddVars(vec![a(0), Traced::new(other_id, 0)]));
    let (old_translator, other_translator) = {
        let waves = app.panels.focused_waves().unwrap();
        let first = &waves.signal(0).unwrap().translator;
        let other = &waves.signal(1).unwrap().translator;
        assert!(!Arc::ptr_eq(first, other));
        assert_eq!(first.translate(&WaveValue::Bits("0".into())).text, "IDLE");
        assert_eq!(
            other.translate(&WaveValue::Bits("0".into())).text,
            "OTHER_IDLE"
        );
        (Arc::downgrade(first), Arc::downgrade(other))
    };
    app.handle(Command::RemoveTrace(other_id));
    app.handle(Command::Undo);
    assert!(Arc::ptr_eq(
        &other_translator.upgrade().unwrap(),
        &app.panels
            .focused_waves()
            .unwrap()
            .signal(1)
            .unwrap()
            .translator,
    ));
    app.set_session(fixture());
    assert!(old_translator.upgrade().is_none());
    assert!(other_translator.upgrade().is_none());
    app.handle(Command::AddVars(vec![a(0)]));
    assert_eq!(
        app.panels
            .focused_waves()
            .unwrap()
            .signal(0)
            .unwrap()
            .translator
            .translate(&WaveValue::Bits("0".into()))
            .text,
        "IDLE"
    );
}

#[test]
fn bit_waveforms_follow_the_translators_digital_style() {
    use volna_core::data::{Translated, Translator};
    use volna_core::wave::model::MenuAction;
    use volna_trace::data::value_view::ValueView;
    use volna_trace::data::{SignalShape, ValueKind};

    struct WordTranslator;
    impl Translator for WordTranslator {
        fn id(&self) -> &'static str {
            "words"
        }
        fn name(&self) -> &'static str {
            "Words"
        }
        fn badge(&self) -> &'static str {
            "words"
        }
        fn applies(&self, shape: SignalShape) -> bool {
            shape == SignalShape::Bit
        }
        fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated {
            let text = match value {
                ValueView::Logic(bits) if bits.to_u64() == Some(0) => "OFF",
                _ => "ON",
            };
            Translated {
                text: text[..limit.min(text.len())].into(),
                kind: ValueKind::Normal,
                truncated: limit < text.len(),
            }
        }
    }
    let mut app = App::new();
    app.set_session(fixture());
    app.doc.translators.register(Arc::new(WordTranslator));
    app.handle(Command::AddVars(vec![a(0)]));
    for request in app.take_requests() {
        app.deliver(request.perform());
    }
    let panel = app.panels.focused_id();
    app.panels.focused_waves_mut().unwrap().open_signal_menu(
        &app.doc,
        0,
        volna_core::geometry::point(0.0, 0.0),
    );
    app.handle(Command::MenuSelect(
        panel,
        MenuAction::Format("words".into()),
    ));
    for theme in [
        volna_core::theme::Theme::volna(true),
        volna_core::theme::Theme::volna(false),
    ] {
        app.layout_panel(
            panel,
            volna_core::geometry::Rect::from_xywh(0.0, 0.0, 1200.0, 600.0),
            &theme,
            1.0,
            &mut volna_core::scene::MonoMeasure,
        );
        let scene = app.render_panel(panel, &theme, &mut volna_core::scene::MonoMeasure);
        assert!(scene.texts().any(|text| text == "OFF"));
        assert!(scene.texts().any(|text| text == "ON"));
    }
}
