//! Object-local shared attribute tables and their borrowed wire views.

use super::decode::Reader;
use super::objects::{GeneratorPayload, GeneratorView};
use crate::data::loaded_tracks::LoadedRelation;
use crate::data::source::Generator;
use crate::data::transactions::{
    AttributeValue, Attributes, Relation, SharedValue, Track, Transaction, TransactionAttribute,
    TransactionEvent, TransactionStage,
};

use serde::{Serialize, Serializer, ser::SerializeSeq};
use std::{collections::HashMap, future::Future, pin::Pin};

#[derive(Default)]
pub(super) struct Table<'a> {
    ids: HashMap<usize, u32>,
    entries: Vec<&'a AttributeValue>,
    pending: Vec<(&'a AttributeValue, bool)>,
}

impl<'a> Table<'a> {
    pub fn collect(&mut self, roots: impl IntoIterator<Item = &'a AttributeValue>) {
        for root in roots {
            if !matches!(
                root,
                AttributeValue::List(_) | AttributeValue::Map(_) | AttributeValue::Shared(_)
            ) {
                continue;
            }
            self.pending.push((root, false));
            while let Some((value, finish)) = self.pending.pop() {
                match value {
                    AttributeValue::Shared(shared) => {
                        let pointer = shared.as_ptr() as usize;
                        if self.ids.contains_key(&pointer) {
                            continue;
                        }
                        if finish {
                            // Dependencies precede their users so forward references are never needed.
                            let id = u32::try_from(self.entries.len()).expect("shared value count");
                            self.ids.insert(pointer, id);
                            self.entries.push(shared);
                        } else {
                            self.pending.push((value, true));
                            self.pending.push((shared, false));
                        }
                    }
                    AttributeValue::List(values) => {
                        self.pending.extend(values.iter().rev().map(|v| (v, false)));
                    }
                    AttributeValue::Map(values) => {
                        self.pending
                            .extend(values.iter().rev().map(|(_, v)| (v, false)));
                    }
                    _ => (),
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn wrap<T: ?Sized>(&self, value: &'a T) -> View<'_, 'a, T> {
        View { value, table: self }
    }
}

pub(super) struct View<'t, 'a, T: ?Sized> {
    pub value: &'a T,
    pub table: &'t Table<'a>,
}

impl<T> Serialize for View<'_, '_, [T]>
where
    for<'t, 'a> View<'t, 'a, T>: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.value.len()))?;
        for value in self.value {
            seq.serialize_element(&self.table.wrap(value))?;
        }
        seq.end()
    }
}

impl Serialize for Table<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.entries.len()))?;
        for value in &self.entries {
            seq.serialize_element(&self.wrap(*value))?;
        }
        seq.end()
    }
}

impl Serialize for View<'_, '_, (String, AttributeValue)> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (&self.value.0, self.table.wrap(&self.value.1)).serialize(serializer)
    }
}

impl Serialize for View<'_, '_, AttributeValue> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.value {
            AttributeValue::List(values) => serializer.serialize_newtype_variant(
                "AttributeValue",
                13,
                "List",
                &self.table.wrap(values.as_slice()),
            ),
            AttributeValue::Map(values) => serializer.serialize_newtype_variant(
                "AttributeValue",
                14,
                "Map",
                &self.table.wrap(values.as_slice()),
            ),
            AttributeValue::Shared(value) => serializer.serialize_newtype_variant(
                "AttributeValue",
                15,
                "Ref",
                self.table
                    .ids
                    .get(&(value.as_ptr() as usize))
                    .expect("collected shared value"),
            ),
            value => value.serialize(serializer),
        }
    }
}

pub(super) struct Values(Vec<SharedValue>);

impl Values {
    pub async fn read(r: &Reader) -> anyhow::Result<Self> {
        let count = r.length(4).await?;
        anyhow::ensure!(
            u32::try_from(count).is_ok(),
            "shared attribute table is too large"
        );
        r.charge(
            count
                .checked_mul(std::mem::size_of::<SharedValue>())
                .ok_or_else(|| anyhow::anyhow!("shared attribute table size overflow"))?,
        )?;
        let mut values = Self(Vec::new());
        values.0.try_reserve_exact(count)?;
        for _ in 0..count {
            r.checkpoint().await;
            r.charge(std::mem::size_of::<AttributeValue>() + 2 * std::mem::size_of::<usize>())?;
            let value = attribute(r, &values, 0).await?;
            values.0.push(SharedValue::new(value));
        }
        Ok(values)
    }
}

pub(super) async fn attributes<'a>(
    r: &'a Reader,
    values: &'a Values,
    depth: usize,
) -> anyhow::Result<Attributes> {
    r.vector(12, || async {
        Ok((r.string().await?, attribute(r, values, depth).await?))
    })
    .await
}

pub(super) fn attribute<'a>(
    r: &'a Reader,
    values: &'a Values,
    depth: usize,
) -> Pin<Box<dyn Future<Output = anyhow::Result<AttributeValue>> + Send + 'a>> {
    Box::pin(async move {
        anyhow::ensure!(depth < 128, "attribute nesting exceeds client limit");
        use AttributeValue::{
            Bool, Bytes, Enum, F64, Fixed, I64, List, Logic, Map, Null, Pointer, Text, Time, U64,
            UFixed,
        };
        Ok(match r.u32().await? {
            0 => Null,
            1 => Bool(r.boolean().await?),
            2 => I64(r.u64().await? as i64),
            3 => U64(r.u64().await?),
            4 => F64(f64::from_bits(r.u64().await?)),
            5 => Text(r.string().await?),
            6 => Bytes(r.bytes().await?),
            7 => {
                let width = r.u64().await?;
                let states = r.u8().await?;
                let data = r.bytes().await?;
                anyhow::ensure!(
                    usize::try_from(width).is_ok(),
                    "logic width exceeds this platform's address space"
                );
                let stride = match states {
                    2 => 1u64,
                    4 => 2,
                    9 => 4,
                    _ => anyhow::bail!("invalid logic alphabet"),
                };
                let per_byte = 8 / stride;
                anyhow::ensure!(
                    data.len() as u64 == width.div_ceil(per_byte),
                    "logic length does not match width"
                );
                let padding = (width % per_byte) * stride;
                anyhow::ensure!(
                    padding == 0 || data.last().is_none_or(|byte| byte >> padding == 0),
                    "nonzero logic padding"
                );
                anyhow::ensure!(
                    states != 9
                        || data.iter().enumerate().all(|(index, byte)| byte & 15 <= 8
                            && (index as u64 * 2 + 1 >= width || byte >> 4 <= 8)),
                    "invalid logic state"
                );
                Logic {
                    width,
                    states,
                    data,
                }
            }
            8 => Time(r.u64().await?),
            9 => Enum {
                value: r.i128().await?,
                name: r.string().await?,
            },
            10 => Pointer(r.u64().await?),
            11 => Fixed {
                raw: r.u64().await? as i64,
                scale: r.u64().await? as i64,
            },
            12 => UFixed {
                raw: r.u64().await?,
                scale: r.u64().await? as i64,
            },
            13 => List(r.vector(4, || attribute(r, values, depth + 1)).await?),
            14 => Map(attributes(r, values, depth + 1).await?),
            15 => {
                let id = r.u32().await? as usize;
                AttributeValue::Shared(
                    values
                        .0
                        .get(id)
                        .ok_or_else(|| {
                            anyhow::anyhow!("unknown or forward shared attribute reference")
                        })?
                        .clone(),
                )
            }
            _ => anyhow::bail!("invalid attribute kind"),
        })
    })
}

impl<'a> Table<'a> {
    pub fn collect_transactions(
        &mut self,
        transactions: &'a [Transaction],
        relations: &'a [LoadedRelation],
    ) {
        for tx in transactions {
            self.collect(tx.attributes.iter().map(|attr| &attr.value));
            for attrs in tx
                .events
                .iter()
                .map(|event| &event.attributes)
                .chain(tx.stages.iter().map(|stage| &stage.attributes))
            {
                self.collect(attrs.iter().map(|(_, value)| value));
            }
        }
        for edge in relations {
            self.collect(edge.relation.attributes.iter().map(|(_, value)| value));
        }
    }
}

impl Serialize for View<'_, '_, Track> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (v, t) = (self.value, self.table);
        (&v.id, &v.path, &v.kind, t.wrap(v.attributes.as_slice())).serialize(serializer)
    }
}

impl Serialize for View<'_, '_, Generator> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (v, t) = (self.value, self.table);
        (
            &v.name,
            &v.scope,
            &v.root,
            &v.kind,
            &v.track,
            t.wrap(v.attributes.as_slice()),
        )
            .serialize(serializer)
    }
}

impl Serialize for View<'_, '_, TransactionAttribute> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (v, t) = (self.value, self.table);
        (&v.key, t.wrap(&v.value)).serialize(serializer)
    }
}

impl Serialize for View<'_, '_, Transaction> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (v, t) = (self.value, self.table);
        (
            &v.id,
            &v.generator,
            &v.begin,
            &v.end,
            v.status as u8,
            v.kind as u8,
            &v.parent,
            t.wrap(v.attributes.as_slice()),
            t.wrap(v.events.as_slice()),
            t.wrap(v.stages.as_slice()),
        )
            .serialize(serializer)
    }
}

impl Serialize for View<'_, '_, TransactionEvent> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (v, t) = (self.value, self.table);
        (
            &v.id,
            &v.generator,
            &v.time,
            &v.name,
            t.wrap(v.attributes.as_slice()),
        )
            .serialize(serializer)
    }
}

impl Serialize for View<'_, '_, TransactionStage> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (v, t) = (self.value, self.table);
        (
            &v.id,
            &v.generator,
            &v.name,
            &v.lane,
            &v.begin,
            &v.end,
            t.wrap(v.attributes.as_slice()),
        )
            .serialize(serializer)
    }
}

impl Serialize for View<'_, '_, Relation> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (v, t) = (self.value, self.table);
        (&v.kind, &v.from, &v.to, t.wrap(v.attributes.as_slice())).serialize(serializer)
    }
}

impl Serialize for View<'_, '_, LoadedRelation> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (v, t) = (self.value, self.table);
        (
            &v.id,
            &v.from_generator,
            &v.to_generator,
            t.wrap(&v.relation),
        )
            .serialize(serializer)
    }
}

impl Serialize for View<'_, '_, GeneratorPayload> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (v, t) = (self.value, self.table);
        (
            &v.generator,
            t.wrap(v.transactions.as_slice()),
            &v.parents,
            &v.parent_labels,
            t.wrap(v.relations.as_slice()),
        )
            .serialize(serializer)
    }
}

impl Serialize for View<'_, '_, GeneratorView<'_>> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (v, t) = (self.value.source, self.table);
        let (parents, labels) = (&self.value.parents, &self.value.labels);
        if t.is_empty() {
            return (
                v.generator(),
                v.transactions(),
                parents,
                labels,
                v.relations(),
            )
                .serialize(serializer);
        }
        (
            v.generator(),
            t.wrap(v.transactions()),
            parents,
            labels,
            t.wrap(v.relations()),
        )
            .serialize(serializer)
    }
}
