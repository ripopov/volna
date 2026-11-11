use volna_trace::data::transactions::AttributeValue;
use volna_trace::session::{LocalSession, Session};
use vtr::{Attribute, SourceOptions, Value, Writer};

fn description(directory: &std::path::Path, name: &str, leaf: Value) -> AttributeValue {
    let path = directory.join(name);
    let mut writer = Writer::create(&path).unwrap();
    let key = writer.intern("description").unwrap();
    let mut value = writer.intern_value(leaf).unwrap();
    for _ in 0..96 {
        value = writer
            .intern_value(Value::List(vec![Value::Ref(value); 2]))
            .unwrap();
    }
    let mut options = SourceOptions::new("instructions", None);
    options.attributes = vec![Attribute {
        key,
        value: Value::Ref(value),
    }];
    writer.transactions::<()>(options, "INSTRUCTION").unwrap();
    writer.close().unwrap();
    let session = LocalSession::open(&path).unwrap();
    session.tracks()[0].attributes[0].1.clone()
}

#[test]
fn independently_loaded_shared_descriptions_compare_by_content_and_float_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let a = description(dir.path(), "a.vtr", Value::U64(u64::MAX));
    let b = description(dir.path(), "b.vtr", Value::U64(u64::MAX));
    let changed = description(dir.path(), "changed.vtr", Value::U64(u64::MAX - 1));
    assert!(a.eq(&b), "independently loaded descriptions differ");
    assert!(a.eq(&a.clone()), "cloned description differs");
    assert!(!a.eq(&changed), "changed leaf compares equal");
    let zero = description(dir.path(), "zero.vtr", Value::F64(0.0));
    let negative_zero = description(dir.path(), "negative-zero.vtr", Value::F64(-0.0));
    assert!(
        zero.eq(&negative_zero),
        "ordinary equality distinguishes signed zero"
    );
    let nan = description(
        dir.path(),
        "nan.vtr",
        Value::F64(f64::from_bits(0x7ff8_0000_0000_0001)),
    );
    assert!(!nan.eq(&nan.clone()), "ordinary equality accepts NaN");
    let inline = AttributeValue::U64(7);
    let shared = AttributeValue::shared(inline.clone());
    assert_ne!(inline, shared);
    assert_eq!(shared, shared.clone());
}

#[test]
fn deeply_nested_shared_aliases_compare_without_recursive_calls() {
    fn chain(leaf: u64) -> AttributeValue {
        let mut value = AttributeValue::U64(leaf);
        for _ in 0..1024 {
            value = AttributeValue::shared(AttributeValue::List(vec![value]));
        }
        value
    }
    assert!(chain(17).eq(&chain(17)));
    assert!(!chain(17).eq(&chain(18)));
}
