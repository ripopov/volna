//! Shared attribute ownership with iterative release of the final graph owner.

use super::AttributeValue;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, ops::Deref, sync::Arc};

#[derive(Clone)]
pub struct SharedValue(Option<Arc<AttributeValue>>);

impl SharedValue {
    #[must_use]
    pub fn new(value: AttributeValue) -> Self {
        Self(Some(Arc::new(value)))
    }

    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(self.owner(), other.owner())
    }

    #[must_use]
    pub(crate) fn as_ptr(&self) -> *const AttributeValue {
        Arc::as_ptr(self.owner())
    }

    fn owner(&self) -> &Arc<AttributeValue> {
        // The slot is empty only after Drop transfers ownership into the release loop.
        self.0.as_ref().expect("shared value owner")
    }
}

impl Deref for SharedValue {
    type Target = AttributeValue;

    fn deref(&self) -> &Self::Target {
        self.owner()
    }
}

impl fmt::Debug for SharedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl PartialEq for SharedValue {
    fn eq(&self, other: &Self) -> bool {
        (**self).eq(&**other)
    }
}

impl Serialize for SharedValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (**self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SharedValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        AttributeValue::deserialize(deserializer).map(Self::new)
    }
}

enum Remaining {
    List(std::vec::IntoIter<AttributeValue>),
    Map(std::vec::IntoIter<(String, AttributeValue)>),
}

impl Remaining {
    fn next(&mut self) -> Option<AttributeValue> {
        match self {
            Self::List(values) => values.next_back(),
            Self::Map(values) => values.next_back().map(|(_, value)| value),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::List(values) => values.len() == 0,
            Self::Map(values) => values.len() == 0,
        }
    }
}

impl Drop for SharedValue {
    fn drop(&mut self) {
        // into_inner elects one releasing owner even when clones are dropped concurrently.
        let mut next = self.0.take().and_then(Arc::into_inner);
        let mut remaining = Vec::<Remaining>::new();
        loop {
            match next.take() {
                Some(AttributeValue::Shared(mut value)) => {
                    next = value.0.take().and_then(Arc::into_inner);
                }
                Some(AttributeValue::List(values)) => {
                    let mut tail = Remaining::List(values.into_iter());
                    next = tail.next();
                    if !tail.is_empty() {
                        remaining.push(tail);
                    }
                }
                Some(AttributeValue::Map(values)) => {
                    let mut tail = Remaining::Map(values.into_iter());
                    next = tail.next();
                    if !tail.is_empty() {
                        remaining.push(tail);
                    }
                }
                Some(_) => (),
                None => {
                    let Some(tail) = remaining.last_mut() else {
                        break;
                    };
                    next = tail.next();
                    if tail.is_empty() {
                        remaining.pop();
                    }
                }
            }
        }
    }
}
