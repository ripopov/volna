//! Typed row ordering over in-memory source ordinals, never formatted cells.

#[cfg(not(target_arch = "wasm32"))]
use super::columns::TransactionColumn;
#[cfg(not(target_arch = "wasm32"))]
use crate::data::loaded_tracks::LoadedGenerator;
#[cfg(not(target_arch = "wasm32"))]
use std::cmp::Ordering;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use volna_trace::data::transactions::{AttributeValue, TransactionAttribute};
#[cfg(not(target_arch = "wasm32"))]
use volna_trace::data::{SignalHistory, value_view::ValueView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub column: usize,
    pub descending: bool,
}

#[derive(Clone)]
#[cfg(not(target_arch = "wasm32"))]
pub(super) enum Source {
    Generator(Arc<LoadedGenerator>),
    Signals {
        histories: Vec<Arc<dyn SignalHistory>>,
        axis: Option<Arc<[u64]>>,
        _reservation: Option<Arc<volna_trace::remote::memory::Reservation>>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
impl Source {
    fn time(&self, row: u64) -> u64 {
        match self {
            Self::Signals {
                histories, axis, ..
            } => axis
                .as_ref()
                .map_or_else(|| histories[0].time(row as usize), |a| a[row as usize]),
            _ => unreachable!(),
        }
    }

    pub(super) fn compare(&self, sort: Sort, a: u64, b: u64) -> Ordering {
        let (order, missing) = match self {
            Self::Generator(g) => {
                let (a, b) = (&g.transactions()[a as usize], &g.transactions()[b as usize]);
                match TransactionColumn::ALL[sort.column] {
                    TransactionColumn::Id => (a.id.0.cmp(&b.id.0), false),
                    TransactionColumn::Begin => (a.begin.cmp(&b.begin), false),
                    TransactionColumn::End | TransactionColumn::Duration => {
                        let missing = (
                            a.status == vtr::TxStatus::Open,
                            b.status == vtr::TxStatus::Open,
                        );
                        let order = if sort.column == 2 {
                            a.end
                                .saturating_sub(a.begin)
                                .cmp(&b.end.saturating_sub(b.begin))
                        } else {
                            a.end.cmp(&b.end)
                        };
                        nullable(missing, order)
                    }
                    TransactionColumn::Status => (a.status.name().cmp(b.status.name()), false),
                    TransactionColumn::Label => {
                        let label = |attributes: &Vec<TransactionAttribute>| {
                            attributes.iter().position(|a| a.key == "vtr.label")
                        };
                        match (label(&a.attributes), label(&b.attributes)) {
                            (Some(i), Some(j)) => {
                                attributes(&a.attributes[i].value, &b.attributes[j].value)
                            }
                            (x, y) => nullable((x.is_none(), y.is_none()), Ordering::Equal),
                        }
                    }
                    TransactionColumn::Attributes => (
                        sequence(
                            a.attributes.iter().filter(|a| a.key != "vtr.label"),
                            b.attributes.iter().filter(|a| a.key != "vtr.label"),
                            |a, b| {
                                a.key
                                    .cmp(&b.key)
                                    .then_with(|| attributes(&a.value, &b.value).0)
                            },
                        ),
                        false,
                    ),
                }
            }
            Self::Signals { histories, .. } => {
                let (a, b) = (self.time(a), self.time(b));
                if sort.column == 0 {
                    (a.cmp(&b), false)
                } else {
                    let history = &histories[sort.column - 1];
                    let (a, b) = (history.index_at(a), history.index_at(b));
                    match (a, b) {
                        (Some(a), Some(b)) => {
                            values(history.value_view(Some(a)), history.value_view(Some(b)))
                        }
                        _ => nullable((a.is_none(), b.is_none()), Ordering::Equal),
                    }
                }
            }
        };
        // The source ordinal is always ascending, including descending sorts: ties are stable.
        (if sort.descending && !missing {
            order.reverse()
        } else {
            order
        })
        .then(a.cmp(&b))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn nullable((a, b): (bool, bool), order: Ordering) -> (Ordering, bool) {
    if a || b {
        (a.cmp(&b), true)
    } else {
        (order, false)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sequence<A, B, T>(mut a: A, mut b: B, compare: impl Fn(T, T) -> Ordering) -> Ordering
where
    A: Iterator<Item = T>,
    B: Iterator<Item = T>,
{
    loop {
        match (a.next(), b.next()) {
            (Some(a), Some(b)) => {
                let order = compare(a, b);
                if order != Ordering::Equal {
                    return order;
                }
            }
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn values(a: ValueView<'_>, b: ValueView<'_>) -> (Ordering, bool) {
    let tag = |v: &ValueView<'_>| match v {
        ValueView::Unavailable => 5,
        ValueView::Logic(_) => 0,
        ValueView::Real(_) => 1,
        ValueView::Text(_) => 2,
        ValueView::Bytes(_) => 3,
    };
    let missing = (
        matches!(a, ValueView::Unavailable),
        matches!(b, ValueView::Unavailable),
    );
    let order = tag(&a).cmp(&tag(&b)).then_with(|| match (a, b) {
        (ValueView::Logic(a), ValueView::Logic(b)) => a.width.cmp(&b.width).then_with(|| {
            sequence(
                (0..a.width).map(|i| a.bit(i)),
                (0..b.width).map(|i| b.bit(i)),
                |a, b| a.cmp(&b),
            )
        }),
        (ValueView::Real(a), ValueView::Real(b)) => a.total_cmp(&b),
        (ValueView::Text(a), ValueView::Text(b)) => a.cmp(&b),
        (ValueView::Bytes(a), ValueView::Bytes(b)) => a.cmp(&b),
        _ => Ordering::Equal,
    });
    nullable(missing, order)
}

#[cfg(not(target_arch = "wasm32"))]
fn attributes(a: &AttributeValue, b: &AttributeValue) -> (Ordering, bool) {
    use AttributeValue::*;
    let tag = |a: &AttributeValue| match a {
        Null => 0,
        Bool(_) => 1,
        I64(_) => 2,
        U64(_) => 3,
        F64(_) => 4,
        Text(_) => 5,
        Bytes(_) => 6,
        Logic { .. } => 7,
        Time(_) => 8,
        Enum { .. } => 9,
        Pointer(_) => 10,
        Fixed { .. } => 11,
        UFixed { .. } => 12,
        List(_) => 13,
        Map(_) => 14,
    };
    let order = tag(a).cmp(&tag(b)).then_with(|| match (a, b) {
        (Bool(a), Bool(b)) => a.cmp(b),
        (I64(a), I64(b)) => a.cmp(b),
        (U64(a), U64(b)) | (Time(a), Time(b)) | (Pointer(a), Pointer(b)) => a.cmp(b),
        (F64(a), F64(b)) => a.total_cmp(b),
        (Text(a), Text(b)) => a.cmp(b),
        (Bytes(a), Bytes(b)) => a.cmp(b),
        (
            Logic {
                width: a,
                states: sa,
                data: da,
            },
            Logic {
                width: b,
                states: sb,
                data: db,
            },
        ) => {
            values(
                ValueView::Logic(volna_trace::data::value_view::LogicView::packed_lsb(
                    *a, *sa, da,
                )),
                ValueView::Logic(volna_trace::data::value_view::LogicView::packed_lsb(
                    *b, *sb, db,
                )),
            )
            .0
        }
        (Enum { value: a, name: na }, Enum { value: b, name: nb }) => a.cmp(b).then(na.cmp(nb)),
        (Fixed { raw: a, scale: sa }, Fixed { raw: b, scale: sb }) => {
            decimal(*a as i128, *sa, *b as i128, *sb)
        }
        (UFixed { raw: a, scale: sa }, UFixed { raw: b, scale: sb }) => {
            decimal(*a as i128, *sa, *b as i128, *sb)
        }
        (List(a), List(b)) => sequence(a.iter(), b.iter(), |a, b| attributes(a, b).0),
        (Map(a), Map(b)) => sequence(a.iter(), b.iter(), |a, b| {
            a.0.cmp(&b.0).then_with(|| attributes(&a.1, &b.1).0)
        }),
        _ => Ordering::Equal,
    });
    nullable((matches!(a, Null), matches!(b, Null)), order)
}

#[cfg(not(target_arch = "wasm32"))]
fn decimal(a: i128, scale_a: i32, b: i128, scale_b: i32) -> Ordering {
    let sign = a.signum().cmp(&b.signum());
    if sign != Ordering::Equal || a == 0 {
        return sign;
    }
    let (a_abs, b_abs) = (a.unsigned_abs(), b.unsigned_abs());
    let (digits_a, digits_b) = (a_abs.ilog10() + 1, b_abs.ilog10() + 1);
    let order = (i64::from(scale_a) + i64::from(digits_a))
        .cmp(&(i64::from(scale_b) + i64::from(digits_b)))
        .then_with(|| {
            (a_abs * 10u128.pow(20 - digits_a)).cmp(&(b_abs * 10u128.pow(20 - digits_b)))
        });
    if a < 0 { order.reverse() } else { order }
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub(super) use native::{Job, Order, required_bytes, retire};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use volna_trace::data::value_view::LogicView;

    #[test]
    fn keys_use_exact_values_and_total_orders() {
        assert_eq!(
            attributes(
                &AttributeValue::U64(u64::MAX - 1),
                &AttributeValue::U64(u64::MAX)
            )
            .0,
            Ordering::Less
        );
        assert_eq!(
            attributes(&AttributeValue::I64(-10), &AttributeValue::I64(2)).0,
            Ordering::Less
        );
        assert_eq!(
            values(ValueView::Real(-0.0), ValueView::Real(0.0)).0,
            Ordering::Less
        );
        assert_eq!(
            values(ValueView::Real(f64::INFINITY), ValueView::Real(f64::NAN)).0,
            Ordering::Less
        );
        assert_eq!(
            values(ValueView::Unavailable, ValueView::Text("a".into())),
            (Ordering::Greater, true)
        );
        assert_eq!(
            values(
                ValueView::Logic(LogicView::ascii(b"011".as_slice())),
                ValueView::Logic(LogicView::ascii(b"100".as_slice()))
            )
            .0,
            Ordering::Less
        );
        assert_eq!(decimal(3, 1, 30, 0), Ordering::Equal);
        assert_eq!(decimal(-3, 1, -29, 0), Ordering::Less);
        assert_eq!(
            decimal(u64::MAX as i128, i32::MAX, 1, i32::MIN),
            Ordering::Greater
        );
        assert_eq!(decimal(0, 100, -1, 0), Ordering::Greater);
        let prefix = "a".repeat(4096);
        assert_eq!(
            attributes(
                &AttributeValue::Text(format!("{prefix}b")),
                &AttributeValue::Text(format!("{prefix}c"))
            )
            .0,
            Ordering::Less
        );
    }
}
