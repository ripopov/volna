use volna_trace::{
    OpenSpec,
    data::transactions::{AttributeValue, TrackRef, TransactionRef},
};
use vtr::{ExecutionMode, Recorder, SourceOptions, Status, Writer, WriterOptions};

struct Opcode(u64);
vtr::impl_payload_enum!(Opcode { "all" => u64::MAX });
struct Packet {
    pc: u64,
    operation: Opcode,
    signed: i16,
    fixed: vtr::schema::Fixed,
    names: Vec<String>,
}
vtr::impl_payload!(Packet {
    pc,
    operation,
    signed,
    fixed,
    names
});

#[test]
fn embedded_detail_extends_trace_limits_without_rewriting_owner_lifetimes() {
    for mode in [ExecutionMode::Inline, ExecutionMode::Threaded] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("outside-owner.vtr");
        let mut writer = Writer::create_with(
            &path,
            WriterOptions {
                mode,
                ..Default::default()
            },
        )
        .unwrap();
        let source = writer
            .transactions::<()>(SourceOptions::new("operations", None), "PIPELINE")
            .unwrap();
        let name = writer.intern("execute").unwrap();
        let owner = writer.begin_at(source, 100, &(), None).unwrap();
        writer
            .embedded_stage(owner.owner(), 50, name, name, Some(160), vec![])
            .unwrap();
        writer
            .embedded_event(owner.owner(), 180, name, vec![])
            .unwrap();
        writer.end_at(owner, 110, Status::Ok).unwrap();
        writer.close().unwrap();
        let session = OpenSpec::Path(path).open().unwrap();
        assert_eq!(session.info().time_range, (50, 180));
        let loaded = session.load_track(TrackRef(source.id())).unwrap();
        let row = &loaded.generators[0].transactions()[0];
        assert_eq!((row.begin, row.end), (100, 110));
        assert_eq!((row.stages[0].begin, row.stages[0].end), (50, Some(160)));
        assert_eq!(row.events[0].time, 180);
    }
}

#[test]
fn typed_payloads_located_records_embedded_edges_and_outside_endpoints_survive_loading() {
    for (mode, workers) in [(ExecutionMode::Inline, 0), (ExecutionMode::Threaded, 2)] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("typed.vtr");
        let mut options = WriterOptions {
            mode,
            ..Default::default()
        };
        options.encoding.workers = workers;
        options.encoding.block_records = 2;
        let mut writer = Writer::create_with(&path, options).unwrap();
        let logger = writer
            .logger(SourceOptions::new("diagnostics", None))
            .unwrap();
        let instructions = writer
            .transactions::<Packet>(SourceOptions::new("instructions", None), "PIPELINE")
            .unwrap();
        let memory = writer
            .transactions::<()>(SourceOptions::new("memory", None), "BUS")
            .unwrap();
        let stages = writer
            .stages::<u32>(SourceOptions::new("execute", None))
            .unwrap();
        let events = writer
            .events::<String>(SourceOptions::new("issued", None))
            .unwrap();
        let logs = vtr::logging::declare::<u64>(
            &mut writer,
            vtr::logging::CallsiteOptions::new("retired {}"),
        )
        .unwrap();
        let retry_site = vtr::logging::declare::<String>(
            &mut writer,
            vtr::logging::CallsiteOptions {
                severity: 3,
                file: Some("decoder.cpp".into()),
                line: Some(17),
                ..vtr::logging::CallsiteOptions::new("retry {}")
            },
        )
        .unwrap();
        let key = writer.intern("payload.pc").unwrap();
        let name = writer.intern("fetch").unwrap();
        let lane = writer.intern("").unwrap();
        let edge = writer.intern("depends").unwrap();
        let scale = i64::from(i32::MAX) + 1;
        let packet = Packet {
            pc: u64::MAX,
            operation: Opcode(u64::MAX),
            signed: -17,
            fixed: vtr::schema::Fixed { raw: -3, scale },
            names: vec!["λ".into(), "é".into()],
        };
        let owner = writer.begin_at(instructions, 10, &packet, None).unwrap();
        writer
            .attribute(owner.owner(), key, vtr::Value::U64(99))
            .unwrap();
        let embedded = writer
            .embedded_event(owner.owner(), 12, name, vec![])
            .unwrap();
        let embedded_stage = writer
            .embedded_stage(owner.owner(), 11, name, lane, Some(15), vec![])
            .unwrap();
        let stage = writer
            .begin_stage_at(stages, owner.owner(), 11, &42)
            .unwrap();
        writer.end_stage_at(stage, 18).unwrap();
        let event = writer
            .event_at(events, owner.owner(), 22, &"issue".into())
            .unwrap();
        let log = writer
            .log_at(logger, logs, 25, &123, Some(owner.owner()))
            .unwrap();
        let retry = writer
            .log_at(logger, retry_site, 26, &"bank 3".into(), None)
            .unwrap();
        let child = writer
            .begin_at(memory, 30, &(), Some(owner.owner()))
            .unwrap();
        for (from, to) in [
            (owner.id(), child.id()),
            (owner.id(), child.id()),
            (embedded, child.id()),
            (stage.id(), child.id()),
            (event, child.id()),
            (0, owner.id()),
            (owner.id(), 0),
        ] {
            writer.relate(edge, from, to, vec![]).unwrap();
        }
        writer.end_at(child, 31, Status::Ok).unwrap();
        writer.end_at(owner, 20, Status::Ok).unwrap();
        let unknown = writer
            .begin_at(
                instructions,
                50,
                &Packet {
                    pc: 1,
                    operation: Opcode(u64::MAX - 1),
                    signed: 0,
                    fixed: vtr::schema::Fixed { raw: 0, scale: 0 },
                    names: vec![],
                },
                None,
            )
            .unwrap();
        writer.end_at(unknown, 50, Status::Ok).unwrap();
        writer.close().unwrap();
        let session = OpenSpec::Path(path).open().unwrap();
        let messages = session.load_track(TrackRef(logger.id())).unwrap();
        let retry = messages.generators[0]
            .transaction(TransactionRef(retry))
            .unwrap();
        for (key, expected) in [
            (
                "log.datatype",
                AttributeValue::U64(u64::from(retry_site.id())),
            ),
            ("log.format", AttributeValue::Text("retry {}".into())),
            (
                "log.severity",
                AttributeValue::Enum {
                    value: 3,
                    name: "warn".into(),
                },
            ),
            ("log.file", AttributeValue::Text("decoder.cpp".into())),
            ("log.message", AttributeValue::Text("retry bank 3".into())),
        ] {
            assert!(
                retry
                    .attributes
                    .iter()
                    .any(|attribute| attribute.key == key && attribute.value == expected)
            );
        }
        assert_eq!(session.info().time_range, (10, 50));
        let owners = session.load_track(TrackRef(instructions.id())).unwrap();
        let owner_data = &owners.generators[0];
        let row = owner_data.transaction(TransactionRef(owner.id())).unwrap();
        let attr = |key| {
            &row.attributes
                .iter()
                .find(|attr| attr.key == key)
                .unwrap()
                .value
        };
        assert_eq!(attr("payload.pc"), &AttributeValue::U64(u64::MAX));
        assert_eq!(attr("attributes.payload.pc"), &AttributeValue::U64(99));
        assert_eq!(
            attr("payload.operation"),
            &AttributeValue::Enum {
                value: i128::from(u64::MAX),
                name: "all".into()
            }
        );
        assert_eq!(attr("payload.signed"), &AttributeValue::I64(-17));
        assert_eq!(
            attr("payload.fixed"),
            &AttributeValue::Fixed { raw: -3, scale }
        );
        assert_eq!(
            attr("payload.names"),
            &AttributeValue::List(vec![
                AttributeValue::Text("λ".into()),
                AttributeValue::Text("é".into())
            ])
        );
        assert_eq!(row.events[0].id, TransactionRef(embedded));
        assert_eq!(row.stages[0].id, TransactionRef(embedded_stage));
        assert_eq!(row.stages[0].lane, "0");
        assert_eq!((row.events.len(), row.stages.len()), (2, 2));
        assert_eq!(row.events[1].id, TransactionRef(event));
        assert_eq!(row.events[1].generator, Some(TrackRef(events.id())));
        assert_eq!(row.events[1].time, 22);
        assert_eq!(row.stages[1].id, TransactionRef(stage.id()));
        assert_eq!(row.stages[1].generator, Some(TrackRef(stages.id())));
        assert_eq!(row.stages[1].lane, "0");
        assert_eq!((row.stages[1].begin, row.stages[1].end), (11, Some(18)));
        assert_eq!(
            row.stages[1].attributes,
            vec![("payload".into(), AttributeValue::U64(42))]
        );
        assert_eq!(
            row.events[1].attributes,
            vec![("payload".into(), AttributeValue::Text("issue".into()))]
        );
        for id in [stage.id(), event] {
            assert_eq!(
                owner_data.transaction(TransactionRef(id)).unwrap().id,
                row.id
            );
        }
        assert_eq!(
            owner_data.transaction(TransactionRef(embedded)).unwrap().id,
            row.id
        );
        assert_eq!(owner_data.relations_of(row.id).count(), 7);
        let unknown = owner_data
            .transaction(TransactionRef(unknown.id()))
            .unwrap();
        assert!(
            unknown
                .attributes
                .iter()
                .any(|attr| attr.key == "payload.operation"
                    && attr.value
                        == AttributeValue::Enum {
                            value: i128::from(u64::MAX - 1),
                            name: String::new()
                        })
        );
        assert!(
            owner_data
                .relations()
                .iter()
                .any(|edge| edge.from_generator.is_none())
        );
        assert!(
            owner_data
                .relations()
                .iter()
                .any(|edge| edge.to_generator.is_none())
        );
        let memory_data = session.load_track(TrackRef(memory.id())).unwrap();
        assert_eq!(
            memory_data.generators[0]
                .parent(TransactionRef(child.id()))
                .unwrap()
                .generator,
            Some(TrackRef(instructions.id()))
        );
        let owner_ids: Vec<_> = owner_data
            .relations()
            .iter()
            .filter(|edge| edge.relation.to == TransactionRef(child.id()))
            .map(|edge| edge.id)
            .collect();
        let child_ids: Vec<_> = memory_data.generators[0]
            .relations()
            .iter()
            .map(|edge| edge.id)
            .collect();
        assert_eq!(owner_ids, child_ids);
        for (source, id, kind, field, expected) in [
            (
                stages.id(),
                stage.id(),
                "stage",
                "payload",
                AttributeValue::U64(42),
            ),
            (
                events.id(),
                event,
                "event",
                "payload",
                AttributeValue::Text("issue".into()),
            ),
            (
                logger.id(),
                log,
                "log",
                "log.message",
                AttributeValue::Text("retired 123".into()),
            ),
        ] {
            let loaded = session.load_track(TrackRef(source)).unwrap();
            let row = loaded.generators[0]
                .transaction(TransactionRef(id))
                .unwrap();
            assert!(
                row.attributes.iter().any(|attr| attr.key == "record.kind"
                    && attr.value == AttributeValue::Text(kind.into()))
            );
            assert!(
                row.attributes
                    .iter()
                    .any(|attr| attr.key == field && attr.value == expected)
            );
        }
    }
}

#[test]
fn mixed_activity_keeps_occurrences_and_filters_shared_blocks_by_owner() {
    for mode in [ExecutionMode::Inline, ExecutionMode::Threaded] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mixed.vtr");
        let mut writer = Writer::create_with(
            &path,
            WriterOptions {
                mode,
                ..Default::default()
            },
        )
        .unwrap();
        let core = writer.scope("core0", None).unwrap();
        let owners = writer
            .transactions::<()>(
                SourceOptions::new("instructions", Some(core)),
                "INSTRUCTION",
            )
            .unwrap();
        let stages = writer
            .stages::<()>(SourceOptions::new("execute", Some(core)))
            .unwrap();
        let events = writer
            .events::<()>(SourceOptions::new("retired", Some(core)))
            .unwrap();
        let name = writer.intern("decode").unwrap();
        let lane = writer.intern("pipeline").unwrap();
        let owner = writer.begin_at(owners, 10, &(), None).unwrap();
        let other = writer.begin_at(owners, 10, &(), None).unwrap();
        let embedded_stage = writer
            .embedded_stage(owner.owner(), 12, name, lane, None, vec![])
            .unwrap();
        let embedded_event = writer
            .embedded_event(owner.owner(), 14, name, vec![])
            .unwrap();
        let visit = writer
            .begin_stage_at(stages, owner.owner(), 5, &())
            .unwrap();
        writer.end_stage_at(visit, 25).unwrap();
        let unrelated = writer.event_at(events, other.owner(), 14, &()).unwrap();
        let first = writer.event_at(events, owner.owner(), 14, &()).unwrap();
        let repeated = writer.event_at(events, owner.owner(), 14, &()).unwrap();
        writer.end_at(owner, 15, Status::Ok).unwrap();
        writer.end_at(other, 15, Status::Ok).unwrap();
        writer.seal().unwrap();

        let reader = vtr::Reader::open(&path).unwrap();
        let raw = reader.lookup(owner.id()).unwrap().unwrap();
        let other_raw = reader.lookup(other.id()).unwrap().unwrap();
        assert_eq!(raw.child_blocks(), other_raw.child_blocks());
        assert_eq!(raw.child_blocks().len(), 1);
        let session = OpenSpec::Path(path).open().unwrap();
        let loaded = session.load_track(TrackRef(owners.id())).unwrap();
        let row = loaded.generators[0]
            .transaction(TransactionRef(owner.id()))
            .unwrap();
        assert_eq!((row.begin, row.end), (10, 15));
        assert_eq!(
            row.stages
                .iter()
                .map(|stage| stage.id.0)
                .collect::<Vec<_>>(),
            [visit.id(), embedded_stage]
        );
        assert_eq!(row.stages[0].lane, "0");
        assert_eq!((row.stages[0].begin, row.stages[0].end), (5, Some(25)));
        assert_eq!(row.stages[1].lane, "pipeline");
        assert_eq!(row.stages[1].end, None);
        assert_eq!(
            row.events
                .iter()
                .map(|event| event.id.0)
                .collect::<Vec<_>>(),
            [embedded_event, first, repeated]
        );
        assert_eq!(
            loaded.generators[0]
                .transaction(TransactionRef(other.id()))
                .unwrap()
                .events[0]
                .id
                .0,
            unrelated
        );
        assert!(
            loaded.generators[0]
                .transaction(TransactionRef(unrelated))
                .is_some_and(|tx| tx.id.0 == other.id())
        );
    }
}

#[test]
fn virtual_top_retains_root_members_aliases_and_borrowed_names_after_session_drop() {
    use vtr::{Direction, ScopeType, SignalKind, VarType};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("hierarchy.vtr");
    let mut writer = Writer::create(&path).unwrap();
    let before = writer.scope("before", None).unwrap();
    let signal = writer
        .signal(
            SourceOptions::new("root", None),
            SignalKind::bits(8, vtr::signal::LogicStates::Two),
            VarType::Wire,
            Direction::Input,
        )
        .unwrap();
    let after = writer
        .scope_with(
            SourceOptions::new("after", None),
            ScopeType::Module,
            "module_type",
        )
        .unwrap();
    writer
        .alias(
            signal,
            SourceOptions::new("alias", Some(after)),
            VarType::Wire,
            Direction::Output,
        )
        .unwrap();
    writer
        .signal(
            SourceOptions::new("nested", Some(before)),
            SignalKind::Real,
            VarType::Real,
            Direction::Implicit,
        )
        .unwrap();
    writer.close().unwrap();
    let session = OpenSpec::Path(path).open().unwrap();
    let hierarchy = session.hierarchy().clone();
    drop(session);
    let roots: Vec<_> = hierarchy
        .roots()
        .iter()
        .map(|id| hierarchy.scope(id).name.to_owned())
        .collect();
    assert_eq!(roots, ["before", "(top)", "after"]);
    assert_eq!(hierarchy.scope(2).component, "module_type");
    assert_eq!(hierarchy.scope(1).vars.len(), 1);
    let root = hierarchy.scope(1).vars.first().unwrap();
    let alias = hierarchy.scope(2).vars.first().unwrap();
    assert_eq!(hierarchy.var(root).signal, hierarchy.var(alias).signal);
    assert_eq!(hierarchy.var(root).name, "root");
    assert_eq!(hierarchy.var(alias).name, "alias");
}

struct ManyUnits;
impl vtr::Payload for ManyUnits {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        types.sequence("ManyUnits", vtr::schema::Primitive::Unit.id())
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        <u32 as vtr::Payload>::encode(&2_000_000, out)
    }
    fn decode(reader: &mut vtr::PayloadReader<'_>) -> vtr::Result<Self> {
        let _count = <u32 as vtr::Payload>::decode(reader)?;
        Ok(Self)
    }
}

#[test]
fn compact_zero_size_payloads_cannot_force_unbounded_viewer_materialization() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("zero-size.vtr");
    let mut writer = Writer::create(&path).unwrap();
    let source = writer
        .transactions::<ManyUnits>(SourceOptions::new("many", None), "OWNER")
        .unwrap();
    let owner = writer.begin(source, &ManyUnits, None).unwrap();
    writer.end(owner, Status::Ok).unwrap();
    writer.close().unwrap();
    vtr::Reader::open(&path).unwrap().validate().unwrap();
    let session = OpenSpec::Path(path).open().unwrap();
    let error = session.load_track(TrackRef(source.id())).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("typed payload exceeds the viewer allocation limit")
    );
}

#[test]
fn incomplete_recordings_keep_ready_children_with_unavailable_parent_locations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("incomplete.vtr");
    let mut writer = Writer::create(&path).unwrap();
    let source = writer
        .transactions::<u64>(SourceOptions::new("owners", None), "OWNER")
        .unwrap();
    let missing = writer.begin_deferred_at(source, 10, None).unwrap();
    let child = writer
        .begin_at(source, 11, &7, Some(missing.owner()))
        .unwrap();
    writer.end_at(child, 12, Status::Ok).unwrap();
    let edge = writer.intern("parent").unwrap();
    writer
        .relate(edge, child.id(), missing.id(), vec![])
        .unwrap();
    writer.seal().unwrap();
    let reader = vtr::Reader::open(&path).unwrap();
    assert!(reader.lookup(missing.id()).unwrap().is_none());
    assert!(!reader.validate().unwrap().missing.is_empty());
    let session = OpenSpec::Path(path).open().unwrap();
    let loaded = session.load_track(TrackRef(source.id())).unwrap();
    let generator = &loaded.generators[0];
    assert_eq!(generator.transactions().len(), 1);
    let parent = generator.parent(TransactionRef(child.id())).unwrap();
    assert_eq!(parent.transaction, TransactionRef(missing.id()));
    assert!(parent.generator.is_none());
    assert_eq!(generator.relations().len(), 1);
    assert!(generator.relations()[0].to_generator.is_none());
}

struct Labelled(String);
impl vtr::Payload for Labelled {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let text = <String as vtr::Payload>::bind(types)?;
        types.structure("Labelled", vec![vtr::schema::Field::new("vtr.label", text)])
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.0.encode(out)
    }
    fn decode(reader: &mut vtr::PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self(String::decode(reader)?))
    }
}

#[test]
fn typed_payload_and_tracker_labels_reach_the_viewer() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("labels.vtr");
    let mut writer = Writer::create(&path).unwrap();
    let typed = writer
        .transactions::<Labelled>(SourceOptions::new("typed", None), "OWNER")
        .unwrap();
    let tracked = writer
        .transactions::<()>(SourceOptions::new("tracked", None), "OWNER")
        .unwrap();
    let tx = writer
        .begin(typed, &Labelled("payload λ".into()), None)
        .unwrap();
    writer.end(tx, Status::Ok).unwrap();
    let mut tracker = vtr::Tracker::new(&mut writer, tracked, "tracked").unwrap();
    let keys = tracker.keyspace("items").unwrap();
    let id = tracker
        .open(&mut writer, keys, 7, "fetch", 10, None)
        .unwrap()
        .unwrap();
    tracker.label(&mut writer, keys, 7, "tracker é").unwrap();
    tracker.close(&mut writer, keys, 7, Status::Ok, 20).unwrap();
    writer.close().unwrap();

    let session = OpenSpec::Path(path).open().unwrap();
    for (source, id, expected) in [
        (typed.id(), tx.id(), "payload λ"),
        (tracked.id(), id, "tracker é"),
    ] {
        let loaded = session.load_track(TrackRef(source)).unwrap();
        let row = loaded.generators[0]
            .transaction(TransactionRef(id))
            .unwrap();
        let labels: Vec<_> = row
            .attributes
            .iter()
            .filter(|a| a.key == "vtr.label")
            .collect();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].value, AttributeValue::Text(expected.into()));
    }
}

#[test]
fn large_nested_text_collections_project_every_value_in_order() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("text-collection.vtr");
    let mut writer = Writer::create(&path).unwrap();
    let source = writer
        .transactions::<Vec<Vec<String>>>(SourceOptions::new("rows", None), "OWNER")
        .unwrap();
    let rows: Vec<Vec<_>> = (0..128)
        .map(|row| (0..128).map(|column| format!("λ-{row}-{column}")).collect())
        .collect();
    let tx = writer.begin(source, &rows, None).unwrap();
    writer.end(tx, Status::Ok).unwrap();
    writer.close().unwrap();
    let session = OpenSpec::Path(path).open().unwrap();
    let loaded = session.load_track(TrackRef(source.id())).unwrap();
    let record = loaded.generators[0]
        .transaction(TransactionRef(tx.id()))
        .unwrap();
    let expected = AttributeValue::List(
        rows.into_iter()
            .map(|row| AttributeValue::List(row.into_iter().map(AttributeValue::Text).collect()))
            .collect(),
    );
    assert_eq!(
        record
            .attributes
            .iter()
            .find(|attr| attr.key == "payload")
            .unwrap()
            .value,
        expected
    );
}

#[test]
fn metadata_text_and_interned_strings_preserve_design_identity_and_cycle_units() {
    for interned in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("metadata.vtr");
        let mut writer = Writer::create(&path).unwrap();
        let mut metadata = writer.metadata().unwrap();
        for (key, text) in [("design.vdb_id", "设计 λ"), ("time.unit", "cycle")] {
            let value = if interned {
                vtr::Value::Str(writer.intern(text).unwrap())
            } else {
                vtr::Value::Text(text.into())
            };
            metadata.attributes.push(vtr::Attribute {
                key: writer.intern(key).unwrap(),
                value,
            });
        }
        writer.set_metadata(metadata).unwrap();
        writer.close().unwrap();
        let session = OpenSpec::Path(path).open().unwrap();
        assert_eq!(session.info().design_id.as_deref(), Some("设计 λ"));
        assert_eq!(session.info().time_unit.as_deref(), Some("cycle"));
    }
}

#[test]
fn checked_in_log_recordings_load_with_named_severity_and_formatted_messages() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "tests/fixtures/landing.vtr",
        "tests/fixtures/feature_showcase.vtr",
        "../volna/examples/landing.vtr",
        "../volna/examples/feature_showcase.vtr",
        "../volna/examples/volna-theme.vtr",
        "../volna/examples/pipeline_demo.vtr",
    ] {
        let session = OpenSpec::Path(root.join(relative)).open().unwrap();
        let mut logs = 0;
        for track in session.tracks().iter().filter(|track| track.kind == "LOG") {
            let loaded = session.load_track(track.id).unwrap();
            for generator in &loaded.generators {
                for record in generator.transactions() {
                    let severity = record
                        .attributes
                        .iter()
                        .find(|attribute| attribute.key == "log.severity")
                        .unwrap();
                    let AttributeValue::Enum { value, name } = &severity.value else {
                        panic!("{relative}: severity is not an enum");
                    };
                    let names = ["trace", "debug", "info", "warn", "error", "fatal"];
                    let expected = names
                        .get(usize::try_from(*value).unwrap())
                        .copied()
                        .unwrap_or("other");
                    assert_eq!(name, expected, "{relative}");
                    assert!(
                        record
                            .attributes
                            .iter()
                            .any(|attribute| attribute.key == "log.message"
                                && matches!(
                                    attribute.value,
                                    AttributeValue::Text(_) | AttributeValue::Bytes(_)
                                )),
                        "{relative}: formatted log message is missing"
                    );
                    logs += 1;
                }
            }
        }
        assert!(logs > 0, "{relative}: expected recorded log messages");
    }
}

#[test]
fn printf_logs_retain_arguments_metadata_and_rendered_bytes() {
    for mode in [ExecutionMode::Inline, ExecutionMode::Threaded] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("printf.vtr");
        let mut writer = Writer::create_with(
            &path,
            WriterOptions {
                mode,
                ..Default::default()
            },
        )
        .unwrap();
        let logger = writer.logger(SourceOptions::new("printf", None)).unwrap();
        let cases = [
            ("pc=%#018x", "pc=0xffffffffffffffff"),
            ("%1$llu/%1$X", "18446744073709551615/FFFFFFFFFFFFFFFF"),
            ("missing=%2$d", "missing={?}"),
        ];
        let mut ids = Vec::new();
        for (format, _) in cases {
            let site = vtr::logging::declare::<u64>(
                &mut writer,
                vtr::logging::CallsiteOptions {
                    style: vtr::logging::FormatStyle::Printf,
                    ..vtr::logging::CallsiteOptions::new(format)
                },
            )
            .unwrap();
            ids.push(writer.log_at(logger, site, 1, &u64::MAX, None).unwrap());
        }
        let binary = vtr::logging::declare::<u32>(
            &mut writer,
            vtr::logging::CallsiteOptions {
                style: vtr::logging::FormatStyle::Printf,
                ..vtr::logging::CallsiteOptions::new("%c")
            },
        )
        .unwrap();
        let binary_id = writer.log_at(logger, binary, 2, &255, None).unwrap();
        writer.close().unwrap();
        let session = OpenSpec::Path(path).open().unwrap();
        let messages = session.load_track(TrackRef(logger.id())).unwrap();
        for (id, (format, expected)) in ids.into_iter().zip(cases) {
            let row = messages.generators[0]
                .transaction(TransactionRef(id))
                .unwrap();
            for (key, value) in [
                ("log.format", AttributeValue::Text(format.into())),
                ("log.format_style", AttributeValue::Text("printf".into())),
                ("log.message", AttributeValue::Text(expected.into())),
            ] {
                assert!(
                    row.attributes
                        .iter()
                        .any(|a| a.key == key && a.value == value),
                    "missing {key}"
                );
            }
            assert!(
                row.attributes
                    .iter()
                    .any(|a| a.value == AttributeValue::U64(u64::MAX))
            );
        }
        let row = messages.generators[0]
            .transaction(TransactionRef(binary_id))
            .unwrap();
        assert!(
            row.attributes
                .iter()
                .any(|a| a.key == "log.message" && a.value == AttributeValue::Bytes(vec![255]))
        );
    }
}

#[test]
fn cpp_printf_fixtures_render_through_volna() {
    // vtr_printf_test generates these compact sample traces in both execution modes.

    for mode in ["inline", "threaded"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/printf/{mode}.vtr"));
        let reader = vtr::Reader::open(&path).unwrap();
        let logger = reader.find_generator("printf").unwrap();
        let session = OpenSpec::Path(path).open().unwrap();
        let track = session.load_track(TrackRef(logger)).unwrap();
        let rows = track.generators[0].transactions();
        assert_eq!(rows.len(), 3);
        for (row, expected) in rows.iter().zip([
            AttributeValue::Text("pc=0xffffffffffffffff value=-1.25 label=abc".into()),
            AttributeValue::Text("[    1.250]".into()),
            AttributeValue::Bytes(vec![255]),
        ]) {
            assert!(
                row.attributes
                    .iter()
                    .any(|a| a.key == "log.message" && a.value == expected)
            );
            assert!(
                row.attributes.iter().any(|a| a.key == "log.format_style"
                    && a.value == AttributeValue::Text("printf".into()))
            );
        }
    }
}
