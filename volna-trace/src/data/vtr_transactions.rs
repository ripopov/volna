//! Project typed records and interned metadata into viewer values.

use super::loaded_tracks::{LoadedGenerator, LoadedRelation, LoadedTrack, TransactionLocation};
use super::transactions::{
    AttributeValue, Attributes, Relation, Track, TrackRef, Transaction, TransactionAttribute,
    TransactionEvent, TransactionRef, TransactionStage,
};
use super::vtr_source::LocalSession;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use vtr::{NodeData, Reader, Value, schema::TypeKind};

pub(super) fn load_track(session: &LocalSession, track: TrackRef) -> anyhow::Result<LoadedTrack> {
    anyhow::ensure!(
        session.tracks.iter().any(|t| t.id == track),
        "unknown transaction track {}",
        track.0
    );
    let mut records = Vec::new();
    let mut failure = None;
    let mut accept = |record: vtr::Record| {
        if failure.is_some() {
            return false;
        }
        match transaction(&session.reader, &record) {
            Ok(record) => {
                records.push(record);
                true
            }
            Err(error) => {
                failure = Some(error);
                false
            }
        }
    };
    session.reader.transactions(
        vtr::TransactionQuery {
            generator: Some(track.0),
            ..Default::default()
        },
        &mut accept,
    )?;
    session.reader.records(
        vtr::RecordQuery {
            generator: Some(track.0),
            ..Default::default()
        },
        &mut accept,
    )?;
    if let Some(error) = failure {
        return Err(error);
    }
    let mut owners: HashMap<_, _> = records
        .iter()
        .flat_map(|record| {
            std::iter::once(record.id)
                .chain(record.events.iter().map(|event| event.id))
                .chain(record.stages.iter().map(|stage| stage.id))
                .map(|id| (id, Some(track)))
        })
        .collect();
    let mut external = HashSet::new();
    for record in &records {
        if let Some(parent) = record.parent
            && !owners.contains_key(&parent)
        {
            external.insert(parent);
        }
    }
    let mut incident = Vec::new();
    let mut ordinal = 0;
    session.reader.scan_relations(|edge| {
        let id = ordinal;
        ordinal += 1;
        if owners.contains_key(&TransactionRef(edge.source))
            || owners.contains_key(&TransactionRef(edge.destination))
        {
            for endpoint in [edge.source, edge.destination] {
                if !owners.contains_key(&TransactionRef(endpoint)) {
                    external.insert(TransactionRef(endpoint));
                }
            }
            incident.push((id, relation(&session.reader, edge)));
        }
        true
    })?;
    for id in external {
        let mut record = session.reader.lookup(id.0)?;
        // Embedded endpoints belong to their runtime owner's track.
        if record
            .as_ref()
            .is_some_and(|record| record.generator().is_none())
        {
            record = match record.unwrap().parent() {
                Some(parent) => session.reader.lookup(parent)?,
                None => None,
            };
        }
        owners.insert(
            id,
            record.and_then(|record| record.generator()).map(TrackRef),
        );
    }
    let edges = incident
        .into_iter()
        .map(|(id, relation)| LoadedRelation {
            id,
            from_generator: owners[&relation.from],
            to_generator: owners[&relation.to],
            relation,
        })
        .collect();
    let parents = records
        .iter()
        .filter_map(|record| {
            record.parent.map(|parent| {
                (
                    record.id,
                    TransactionLocation {
                        transaction: parent,
                        generator: owners[&parent],
                    },
                )
            })
        })
        .collect();
    Ok(LoadedTrack {
        track,
        generators: vec![Arc::new(LoadedGenerator::new(
            track, records, parents, edges,
        )?)],
    })
}

pub(super) fn value(reader: &Reader, value: &Value) -> AttributeValue {
    match value {
        Value::Null => AttributeValue::Null,
        Value::Bool(x) => AttributeValue::Bool(*x),
        Value::I8(x) => AttributeValue::I64(i64::from(*x)),
        Value::I16(x) => AttributeValue::I64(i64::from(*x)),
        Value::I32(x) => AttributeValue::I64(i64::from(*x)),
        Value::I64(x) => AttributeValue::I64(*x),
        Value::U8(x) => AttributeValue::U64(u64::from(*x)),
        Value::U16(x) => AttributeValue::U64(u64::from(*x)),
        Value::U32(x) => AttributeValue::U64(u64::from(*x)),
        Value::U64(x) => AttributeValue::U64(*x),
        Value::F32(x) => AttributeValue::F64(f64::from(*x)),
        Value::F64(x) => AttributeValue::F64(*x),
        Value::Str(x) => AttributeValue::Text(reader.string(*x).expect("validated string").into()),
        Value::Text(x) => AttributeValue::Text(x.clone()),
        Value::Bytes(x) => AttributeValue::Bytes(x.clone()),
        Value::Bits { width, data } => AttributeValue::Logic {
            width: *width,
            states: 2,
            data: data.clone(),
        },
        Value::Logic { width, data } => AttributeValue::Logic {
            width: *width,
            states: 4,
            data: data.clone(),
        },
        Value::Logic9 { width, data } => AttributeValue::Logic {
            width: *width,
            states: 9,
            data: data.clone(),
        },
        Value::Time(x) => AttributeValue::Time(*x),
        Value::Enum { value, name } => AttributeValue::Enum {
            value: i128::from(*value),
            name: reader.string(*name).expect("enum label").into(),
        },
        Value::Pointer(x) => AttributeValue::Pointer(*x),
        Value::Fixed { raw, scale } => AttributeValue::Fixed {
            raw: *raw,
            scale: *scale,
        },
        Value::UFixed { raw, scale } => AttributeValue::UFixed {
            raw: *raw,
            scale: *scale,
        },
        Value::List(values) => AttributeValue::List(
            values
                .iter()
                .map(|item| self::value(reader, item))
                .collect(),
        ),
        Value::Map(values) => AttributeValue::Map(attributes(reader, values)),
    }
}

pub(super) fn attributes(reader: &Reader, attrs: &[vtr::Attribute]) -> Attributes {
    attrs
        .iter()
        .map(|attr| {
            (
                reader.string(attr.key).expect("attribute name").into(),
                value(reader, &attr.value),
            )
        })
        .collect()
}

pub(super) fn tracks(reader: &Reader) -> Vec<Track> {
    reader
        .declarations()
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(id, node)| {
            let NodeData::Generator { kind, .. } = node.data else {
                return None;
            };
            let mut path = vec![reader.string(node.name).expect("generator name").into()];
            let mut parent = node.parent;
            while let Some(id) = parent {
                let ancestor = reader.node(id).expect("ancestor");
                path.push(reader.string(ancestor.name).expect("ancestor name").into());
                parent = ancestor.parent;
            }
            path.reverse();
            Some(Track {
                id: TrackRef(id as u32),
                path,
                kind: reader.string(kind).expect("generator kind").into(),
                attributes: attributes(reader, &node.attributes),
            })
        })
        .collect()
}

fn spend(budget: &mut usize, bytes: usize) -> anyhow::Result<()> {
    *budget = budget
        .checked_sub(bytes)
        .ok_or_else(|| anyhow::anyhow!("typed payload exceeds the viewer allocation limit"))?;
    Ok(())
}

fn payload_value(
    reader: &Reader,
    payload: &vtr::PayloadView,
    budget: &mut usize,
) -> anyhow::Result<AttributeValue> {
    let datatype = payload.schema().get(payload.datatype())?;
    match &datatype.kind {
        TypeKind::Struct(fields) => {
            spend(
                budget,
                fields
                    .len()
                    .checked_mul(std::mem::size_of::<(String, AttributeValue)>())
                    .context("payload size overflow")?,
            )?;
            let mut values = Vec::new();
            values.try_reserve_exact(fields.len())?;
            for (field, child) in fields.iter().zip(payload.children()?) {
                spend(budget, field.name.len())?;
                values.push((field.name.clone(), payload_value(reader, &child?, budget)?));
            }
            Ok(AttributeValue::Map(values))
        }
        TypeKind::Array { .. } | TypeKind::Sequence { .. } => {
            let count = payload.len()?;
            spend(
                budget,
                count
                    .checked_mul(std::mem::size_of::<AttributeValue>())
                    .context("payload size overflow")?,
            )?;
            let mut values = Vec::new();
            values.try_reserve_exact(count)?;
            for child in payload.children()? {
                values.push(payload_value(reader, &child?, budget)?);
            }
            Ok(AttributeValue::List(values))
        }
        TypeKind::Enum { values, .. } => {
            let scalar = payload.scalar()?;
            let (number, bits) = match scalar {
                Value::I8(x) => (i128::from(x), x as u64),
                Value::I16(x) => (i128::from(x), x as u64),
                Value::I32(x) => (i128::from(x), x as u64),
                Value::I64(x) => (i128::from(x), x as u64),
                Value::U8(x) => (i128::from(x), u64::from(x)),
                Value::U16(x) => (i128::from(x), u64::from(x)),
                Value::U32(x) => (i128::from(x), u64::from(x)),
                Value::U64(x) => (i128::from(x), x),
                _ => anyhow::bail!("enum has a noninteger base"),
            };
            let name = values
                .iter()
                .find(|entry| entry.value == bits)
                .map_or("", |entry| entry.name.as_str());
            spend(budget, name.len())?;
            Ok(AttributeValue::Enum {
                value: number,
                name: name.into(),
            })
        }
        TypeKind::Scalar(_) => {
            spend(budget, payload.bytes().len())?;
            Ok(value(reader, &payload.scalar()?))
        }
    }
}

use anyhow::Context as _;

fn transaction(reader: &Reader, record: &vtr::Record) -> anyhow::Result<Transaction> {
    let mut attrs: Vec<_> = attributes(reader, record.attributes())
        .into_iter()
        .map(|(key, value)| TransactionAttribute {
            key: if key == "vtr.label" && matches!(value, AttributeValue::Text(_)) {
                key
            } else {
                format!("attributes.{key}")
            },
            value,
        })
        .collect();
    attrs.push(TransactionAttribute {
        key: "record.kind".into(),
        value: AttributeValue::Text(
            match record.kind() {
                vtr::RecordKind::Transaction => "transaction",
                vtr::RecordKind::Clock => "clock",
                vtr::RecordKind::Event => "event",
                vtr::RecordKind::Stage => "stage",
                vtr::RecordKind::Log => "log",
                vtr::RecordKind::EmbeddedEvent => "embedded event",
                vtr::RecordKind::EmbeddedStage => "embedded stage",
            }
            .into(),
        ),
    });
    let payload = record.payload();
    let mut budget = vtr::schema::DEFAULT_PAYLOAD_LIMIT;
    match payload_value(reader, &payload, &mut budget)? {
        AttributeValue::Map(fields) => {
            attrs.extend(fields.into_iter().map(|(key, value)| TransactionAttribute {
                key: if key == "vtr.label" && matches!(value, AttributeValue::Text(_)) {
                    key
                } else {
                    format!("payload.{key}")
                },
                value,
            }));
        }
        AttributeValue::Null => {}
        value => attrs.push(TransactionAttribute {
            key: "payload".into(),
            value,
        }),
    }
    if record.kind() == vtr::RecordKind::Log {
        let bytes = reader.format_log_bytes(record)?;
        let message = match String::from_utf8(bytes) {
            Ok(text) => AttributeValue::Text(text),
            Err(error) => AttributeValue::Bytes(error.into_bytes()),
        };
        attrs.push(TransactionAttribute {
            key: "log.message".into(),
            value: message,
        });
    }
    Ok(Transaction {
        id: TransactionRef(record.id()),
        generator: TrackRef(record.generator().context("record has no source")?),
        begin: record.begin(),
        end: record.end(),
        status: record.status(),
        kind: record.span_kind(),
        parent: record.parent().map(TransactionRef),
        attributes: attrs,
        events: record
            .events()
            .iter()
            .map(|event| TransactionEvent {
                id: TransactionRef(event.id),
                time: event.time,
                name: reader.string(event.name).expect("event name").into(),
                attributes: attributes(reader, &event.attributes),
            })
            .collect(),
        stages: record
            .stages()
            .iter()
            .map(|stage| TransactionStage {
                id: TransactionRef(stage.id),
                name: reader.string(stage.name).expect("stage name").into(),
                lane: reader.string(stage.lane).expect("stage lane").into(),
                begin: stage.begin,
                end: stage.end,
                attributes: attributes(reader, &stage.attributes),
            })
            .collect(),
    })
}

fn relation(reader: &Reader, edge: vtr::Relation) -> Relation {
    Relation {
        kind: reader.string(edge.kind).expect("relation kind").into(),
        from: TransactionRef(edge.source),
        to: TransactionRef(edge.destination),
        attributes: attributes(reader, &edge.attributes),
    }
}
