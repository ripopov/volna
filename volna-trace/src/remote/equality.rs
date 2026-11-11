//! Compare repeated raw relations without serializing their attributes again.

use crate::data::loaded_tracks::LoadedRelation;
use crate::data::transactions::{
    AttributeValue,
    comparison::{Comparison, References, Step},
};

pub(super) async fn relation<F: std::future::Future<Output = ()>>(
    a: &LoadedRelation,
    b: &LoadedRelation,
    checkpoint: &mut impl FnMut() -> F,
) -> bool {
    if a.id != b.id
        || a.from_generator != b.from_generator
        || a.to_generator != b.to_generator
        || a.relation.from != b.relation.from
        || a.relation.to != b.relation.to
    {
        return false;
    }
    if !bytes(
        a.relation.kind.as_bytes(),
        b.relation.kind.as_bytes(),
        checkpoint,
    )
    .await
    {
        return false;
    }
    for step in Comparison::attributes(
        &a.relation.attributes,
        &b.relation.attributes,
        References::Resolve,
    ) {
        checkpoint().await;
        let equal = match step {
            Step::Progress => true,
            Step::Length(a, b) => a == b,
            Step::Key(a, b) => bytes(a.as_bytes(), b.as_bytes(), checkpoint).await,
            Step::Value(a, b) => {
                use AttributeValue::{Bytes, Enum, F64, Logic, Text};
                match (a, b) {
                    (F64(a), F64(b)) => a.to_bits() == b.to_bits(),
                    (Text(a), Text(b)) => bytes(a.as_bytes(), b.as_bytes(), checkpoint).await,
                    (Bytes(a), Bytes(b)) => bytes(a, b, checkpoint).await,
                    (
                        Logic {
                            width: a_width,
                            states: a_states,
                            data: a,
                        },
                        Logic {
                            width: b_width,
                            states: b_states,
                            data: b,
                        },
                    ) => {
                        a_width == b_width && a_states == b_states && bytes(a, b, checkpoint).await
                    }
                    (
                        Enum {
                            value: a_value,
                            name: a,
                        },
                        Enum {
                            value: b_value,
                            name: b,
                        },
                    ) => a_value == b_value && bytes(a.as_bytes(), b.as_bytes(), checkpoint).await,
                    _ => a == b,
                }
            }
        };
        if !equal {
            return false;
        }
    }
    true
}

async fn bytes<F: std::future::Future<Output = ()>>(
    a: &[u8],
    b: &[u8],
    checkpoint: &mut impl FnMut() -> F,
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (a, b) in a.chunks(4096).zip(b.chunks(4096)) {
        checkpoint().await;
        if a != b {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::transactions::{Relation, TrackRef, TransactionRef};
    use std::cell::Cell;
    use std::future::{Future, poll_fn};
    use std::task::{Context, Poll, Waker};

    fn compare(a: &LoadedRelation, b: &LoadedRelation) -> (bool, usize) {
        let credits = Cell::new(0);
        let mut checkpoint = || {
            poll_fn(|_| {
                if credits.get() == 0 {
                    Poll::Pending
                } else {
                    credits.set(credits.get() - 1);
                    Poll::Ready(())
                }
            })
        };
        let mut future = std::pin::pin!(relation(a, b, &mut checkpoint));
        let mut polls = 0;
        loop {
            polls += 1;
            credits.set(8);
            match future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
            {
                Poll::Pending => assert_eq!(credits.get(), 0),
                Poll::Ready(equal) => return (equal, polls),
            }
        }
    }

    #[test]
    fn nested_attributes_compare_bitwise_and_large_values_yield() {
        let a = LoadedRelation {
            id: 1,
            from_generator: Some(TrackRef(1)),
            to_generator: Some(TrackRef(2)),
            relation: Relation {
                kind: "raw".into(),
                from: TransactionRef(1),
                to: TransactionRef(2),
                attributes: vec![(
                    "values".into(),
                    AttributeValue::List(vec![
                        AttributeValue::Map(vec![("nan".into(), AttributeValue::F64(f64::NAN))]),
                        AttributeValue::Text("λ".repeat(256 * 1024)),
                    ]),
                )],
            },
        };
        let mut b = a.clone();
        let (equal, polls) = compare(&a, &b);
        assert!(equal && polls > 8);
        let AttributeValue::List(values) = &mut b.relation.attributes[0].1 else {
            unreachable!()
        };
        let AttributeValue::Text(text) = &mut values[1] else {
            unreachable!()
        };
        text.push('x');
        assert!(!compare(&a, &b).0);
        let mut a = a;
        a.relation.attributes = vec![("zero".into(), AttributeValue::F64(-0.0))];
        b.relation.attributes = vec![("zero".into(), AttributeValue::F64(0.0))];
        assert!(!compare(&a, &b).0);
    }
    #[test]
    fn repeated_shared_relation_details_compare_without_expansion_and_keep_float_bits() {
        fn graph(value: f64) -> AttributeValue {
            let mut value = AttributeValue::shared(AttributeValue::F64(value));
            for _ in 0..96 {
                value = AttributeValue::shared(AttributeValue::List(vec![value.clone(), value]));
            }
            value
        }
        let mut a = LoadedRelation {
            id: 1,
            from_generator: Some(TrackRef(1)),
            to_generator: Some(TrackRef(2)),
            relation: Relation {
                kind: "shared".into(),
                from: TransactionRef(1),
                to: TransactionRef(2),
                attributes: vec![(
                    "description".into(),
                    graph(f64::from_bits(0x7ff8_0000_0000_0001)),
                )],
            },
        };
        let mut b = a.clone();
        b.relation.attributes[0].1 = graph(f64::from_bits(0x7ff8_0000_0000_0001));
        assert!(compare(&a, &b).0);
        b.relation.attributes[0].1 = graph(f64::from_bits(0x7ff8_0000_0000_0002));
        assert!(!compare(&a, &b).0);
        a.relation.attributes[0].1 = graph(-0.0);
        b.relation.attributes[0].1 = graph(0.0);
        assert!(!compare(&a, &b).0);
    }
}
