//! Raw transaction records and catalog.

use serde::{Deserialize, Serialize};
pub use vtr::{TxKind, TxStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransactionRef(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackRef(pub u32);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AttributeValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    Logic {
        width: u32,
        states: u8,
        data: Vec<u8>,
    },
    Time(u64),
    Enum {
        value: i64,
        name: String,
    },
    Pointer(u64),
    Fixed {
        raw: i64,
        scale: i32,
    },
    UFixed {
        raw: u64,
        scale: i32,
    },
    List(Vec<AttributeValue>),
    Map(Attributes),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeType {
    Null,
    Bool,
    I64,
    U64,
    F64,
    Text,
    Bytes,
    Logic,
    Time,
    Enum,
    Pointer,
    Fixed,
    UFixed,
    List,
    Map,
    Mixed,
}

impl AttributeType {
    #[must_use]
    pub fn of(value: &AttributeValue) -> Self {
        match value {
            AttributeValue::Null => Self::Null,
            AttributeValue::Bool(_) => Self::Bool,
            AttributeValue::I64(_) => Self::I64,
            AttributeValue::U64(_) => Self::U64,
            AttributeValue::F64(_) => Self::F64,
            AttributeValue::Text(_) => Self::Text,
            AttributeValue::Bytes(_) => Self::Bytes,
            AttributeValue::Logic { .. } => Self::Logic,
            AttributeValue::Time(_) => Self::Time,
            AttributeValue::Enum { .. } => Self::Enum,
            AttributeValue::Pointer(_) => Self::Pointer,
            AttributeValue::Fixed { .. } => Self::Fixed,
            AttributeValue::UFixed { .. } => Self::UFixed,
            AttributeValue::List(_) => Self::List,
            AttributeValue::Map(_) => Self::Map,
        }
    }
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool => "bool",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::F64 => "f64",
            Self::Text => "str",
            Self::Bytes => "bytes",
            Self::Logic => "logic",
            Self::Time => "time",
            Self::Enum => "enum",
            Self::Pointer => "pointer",
            Self::Fixed => "fixed",
            Self::UFixed => "ufixed",
            Self::List => "list",
            Self::Map => "map",
            Self::Mixed => "Mixed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeSummary {
    pub key: String,
    pub value_type: AttributeType,
    pub example_row: usize,
}

pub type Attributes = Vec<(String, AttributeValue)>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TrackKind {
    Generator { kind: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackRef,
    pub path: Vec<String>,
    pub kind: TrackKind,
    pub attributes: Attributes,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransactionAttribute {
    pub key: String,
    pub value: AttributeValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransactionEvent {
    pub time: u64,
    pub name: String,
    pub attributes: Attributes,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransactionStage {
    pub name: String,
    pub lane: String,
    pub begin: u64,
    pub end: Option<u64>,
    pub attributes: Attributes,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub id: TransactionRef,
    pub generator: TrackRef,
    pub begin: u64,
    pub end: u64,
    #[serde(with = "status_code")]
    pub status: TxStatus,
    #[serde(with = "kind_code")]
    pub kind: TxKind,
    pub parent: Option<TransactionRef>,
    pub attributes: Vec<TransactionAttribute>,
    pub events: Vec<TransactionEvent>,
    pub stages: Vec<TransactionStage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub kind: String,
    pub from: TransactionRef,
    pub to: TransactionRef,
    pub attributes: Attributes,
}

macro_rules! wire_code {
    ($module:ident, $ty:ident, $max:expr) => {
        mod $module {
            use super::*;
            pub fn serialize<S: serde::Serializer>(
                value: &$ty,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                serializer.serialize_u8(*value as u8)
            }
            pub fn deserialize<'de, D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<$ty, D::Error> {
                let value = u8::deserialize(deserializer)?;
                if value > $max {
                    return Err(serde::de::Error::custom("unknown transaction code"));
                }
                Ok($ty::from_u8(value))
            }
        }
    };
}
wire_code!(status_code, TxStatus, 4);
wire_code!(kind_code, TxKind, 5);
