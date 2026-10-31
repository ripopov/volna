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
    w.signal(
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
