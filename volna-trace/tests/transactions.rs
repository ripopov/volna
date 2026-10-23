use volna_trace::OpenSpec;
use volna_trace::data::transactions::*;
use vtr::{Attribute, Recorder, SourceOptions};
#[test]
fn empty_transaction_catalog_still_supports_transactions() {
    let file = tempfile::NamedTempFile::new().unwrap();
    vtr::Writer::create(file.path()).unwrap().close().unwrap();
    let vtr = OpenSpec::Path(file.path().into()).open().unwrap();
    assert!(vtr.capabilities().transactions);
    assert!(vtr.capabilities().relations);
    assert!(vtr.tracks().is_empty());
    assert!(vtr.load_track(TrackRef(0)).is_err());
}

#[test]
fn vtr_transaction_semantics_survive_the_common_contract() {
    use vtr::Value;
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut w = vtr::Writer::create(file.path()).unwrap();
    let scope = w
        .scope_with(
            SourceOptions::new("top", None),
            vtr::ScopeType::Module,
            "top_type",
        )
        .unwrap();
    let stream = w
        .scope_with(
            SourceOptions::new("cpu", Some(scope)),
            vtr::ScopeType::Generic,
            "",
        )
        .unwrap();
    let gen_a = w
        .transactions::<()>(SourceOptions::new("instructions", Some(stream)), "pipeline")
        .unwrap();
    let stream_b = w
        .scope_with(SourceOptions::new("bus", None), vtr::ScopeType::Generic, "")
        .unwrap();
    let gen_b = w
        .transactions::<()>(SourceOptions::new("reads", Some(stream_b)), "tlm")
        .unwrap();
    let key = w.intern("payload").unwrap();
    let text = w.intern("decoded").unwrap();
    let event = w.intern("issue").unwrap();
    let stage = w.intern("execute").unwrap();
    let lane = w.intern("alu").unwrap();
    let edge = w.intern("causes").unwrap();
    let parent = w.begin_at(gen_a, 10, &(), None).unwrap();
    w.span_kind(parent.owner(), SpanKind::Producer).unwrap();
    w.attribute(
        parent.owner(),
        key,
        Value::Map(vec![Attribute {
            key,
            value: Value::List(vec![Value::Str(text), Value::Bytes(vec![0, 255])]),
        }]),
    )
    .unwrap();
    w.embedded_event(
        parent.owner(),
        12,
        event,
        vec![Attribute {
            key,
            value: Value::U64(42),
        }],
    )
    .unwrap();
    w.embedded_stage(
        parent.owner(),
        12,
        stage,
        lane,
        Some(18),
        vec![Attribute {
            key,
            value: Value::Bool(true),
        }],
    )
    .unwrap();
    let child = w.begin_at(gen_b, 20, &(), Some(parent.owner())).unwrap();
    w.end_at(child, 20, Status::Ok).unwrap();
    w.end_at(parent, 20, Status::Aborted).unwrap();
    w.relate(
        edge,
        parent.id(),
        child.id(),
        vec![Attribute {
            key,
            value: Value::Enum {
                value: 3,
                name: text,
            },
        }],
    )
    .unwrap();
    w.close().unwrap();

    let session = OpenSpec::Path(file.path().into()).open().unwrap();
    assert_eq!(session.tracks().len(), 2);
    let generator = session
        .tracks()
        .iter()
        .find(|t| t.path == ["top", "cpu", "instructions"])
        .unwrap();
    assert_eq!(
        generator.kind,
        TrackKind::Generator {
            kind: "pipeline".into()
        }
    );
    let loaded_a = session.load_track(generator.id).unwrap();
    let loaded_b = session.load_track(TrackRef(gen_b.id())).unwrap();
    let a = &loaded_a.generators[0];
    let b = &loaded_b.generators[0];
    let p = a.transaction(TransactionRef(parent.id())).unwrap();
    assert_eq!(
        (p.begin, p.end, p.status, p.kind),
        (10, 20, Status::Aborted, SpanKind::Producer)
    );
    assert_eq!(p.attributes.len(), 2);
    assert_eq!(p.attributes[0].key, "attributes.payload");
    assert_eq!(
        p.attributes[0].value,
        AttributeValue::Map(vec![(
            "payload".into(),
            AttributeValue::List(vec![
                AttributeValue::Text("decoded".into()),
                AttributeValue::Bytes(vec![0, 255])
            ])
        )])
    );
    assert_eq!(p.events[0].name, "issue");
    assert_eq!(p.events[0].time, 12);
    assert_eq!(p.stages[0].lane, "alu");
    assert_eq!(p.stages[0].end, Some(18));
    assert_eq!(
        b.transaction(TransactionRef(child.id())).unwrap().parent,
        Some(TransactionRef(parent.id()))
    );
    assert!(a.transaction(TransactionRef(child.id())).is_none());
    let mut ids = Vec::new();
    for records in [a, b] {
        records
            .visit_window(20, 20, |tx| {
                ids.push(tx.id);
                true
            })
            .unwrap();
    }
    assert_eq!(
        ids.len(),
        2,
        "inclusive overlap includes ending and point transactions"
    );
    assert_eq!(
        a.transactions().iter().map(|tx| tx.id).collect::<Vec<_>>(),
        [TransactionRef(parent.id())]
    );
    let mut visits = 0;
    assert!(
        !a.visit_window(0, u64::MAX, |_| {
            visits += 1;
            false
        })
        .unwrap()
    );
    assert_eq!(visits, 1);
    assert!(a.visit_window(2, 1, |_| true).is_err());
    assert!(session.load_track(TrackRef(u32::MAX)).is_err());
    assert_eq!(a.relations(), b.relations());
    let from: Vec<_> = a.relations().iter().map(|edge| &edge.relation).collect();
    assert_eq!(from[0].kind, "causes");
    assert_eq!(
        from[0].attributes[0].1,
        AttributeValue::Enum {
            value: 3,
            name: "decoded".into()
        }
    );
    drop(session);
    assert_eq!(
        p.events[0].attributes[0].1,
        AttributeValue::U64(42),
        "results outlive their session"
    );
}

#[test]
fn complete_tracks_include_empty_generators_parents_and_parallel_relations() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let stream = writer
        .scope_with(
            SourceOptions::new("stream", None),
            vtr::ScopeType::Generic,
            "",
        )
        .unwrap();
    let generator = writer
        .transactions::<()>(
            SourceOptions::new("generator", Some(stream)),
            "transactions",
        )
        .unwrap();
    let empty = writer
        .transactions::<()>(SourceOptions::new("empty", Some(stream)), "transactions")
        .unwrap();
    let other_stream = writer
        .scope_with(
            SourceOptions::new("other", None),
            vtr::ScopeType::Generic,
            "",
        )
        .unwrap();
    let other = writer
        .transactions::<()>(
            SourceOptions::new("other-generator", Some(other_stream)),
            "transactions",
        )
        .unwrap();
    let parent = writer.begin_at(other, 0, &(), None).unwrap();
    let child = writer
        .begin_at(generator, 50, &(), Some(parent.owner()))
        .unwrap();
    writer.end_at(child, 50, Status::Ok).unwrap();
    writer.end_at(parent, 100, Status::Ok).unwrap();
    let kind = writer.intern("parallel").unwrap();
    for _ in 0..2 {
        writer
            .relate(kind, parent.id(), child.id(), vec![])
            .unwrap();
    }
    writer.close().unwrap();

    let reader = vtr::Reader::open(file.path()).unwrap();
    assert_eq!(
        reader.lookup(parent.id()).unwrap().unwrap().generator(),
        Some(other.id())
    );
    assert_eq!(
        reader.lookup(child.id()).unwrap().unwrap().generator(),
        Some(generator.id())
    );
    assert!(reader.lookup(u64::MAX).unwrap().is_none());
    let session = OpenSpec::Path(file.path().into()).open().unwrap();
    let loaded = session.load_track(TrackRef(generator.id())).unwrap();
    assert_eq!(loaded.generators.len(), 1);
    let records = loaded
        .generators
        .iter()
        .find(|g| g.generator() == TrackRef(generator.id()))
        .unwrap();
    assert_eq!(records.transactions().len(), 1);
    assert_eq!(
        records
            .parent(TransactionRef(child.id()))
            .unwrap()
            .generator,
        Some(TrackRef(other.id()))
    );
    assert_eq!(records.relations().len(), 2);
    assert_ne!(records.relations()[0].id, records.relations()[1].id);
    assert_eq!(
        records.relations()[0].relation,
        records.relations()[1].relation
    );
    assert!(
        session
            .load_track(TrackRef(empty.id()))
            .unwrap()
            .generators
            .iter()
            .find(|g| g.generator() == TrackRef(empty.id()))
            .unwrap()
            .transactions()
            .is_empty()
    );
    let opposite = session.load_track(TrackRef(other.id())).unwrap();
    assert_eq!(records.relations(), opposite.generators[0].relations());
    assert!(session.load_track(TrackRef(u32::MAX)).is_err());
    drop(session);
    let mut hits = vec![];
    opposite.generators[0]
        .visit_window(50, 50, |tx| {
            hits.push(tx.id);
            true
        })
        .unwrap();
    assert_eq!(hits, [TransactionRef(parent.id())]);
}
