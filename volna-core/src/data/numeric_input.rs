//! Captured numeric interpretation for recorded inputs and projected reads.

use super::{NumericKind, projection::BitSlice};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use volna_trace::data::{Bit, SignalHistory, SignalShape, WaveValue, value_view::ValueView};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Interpretation {
    pub kind: NumericKind,
    pub projection: Option<BitSlice>,
}

impl Interpretation {
    pub fn validate(self, source_shape: SignalShape) -> Result<()> {
        let shape = self
            .projection
            .map(|slice| slice.validate(source_shape).map(BitSlice::shape))
            .transpose()?
            .unwrap_or(source_shape);
        ensure!(
            match self.kind {
                NumericKind::Real => shape == SignalShape::Real,
                NumericKind::Unsigned | NumericKind::Signed =>
                    matches!(shape, SignalShape::Bit | SignalShape::Vector { width: 2.. }),
                NumericKind::Float => matches!(
                    shape,
                    SignalShape::Vector {
                        width: 16 | 32 | 64
                    }
                ),
            },
            "Input shape does not support the saved numeric interpretation."
        );
        Ok(())
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self.kind {
            NumericKind::Unsigned => "Unsigned",
            NumericKind::Signed => "Signed",
            NumericKind::Float => "IEEE floating point",
            NumericKind::Real => "Real",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sample {
    Missing,
    Undefined,
    Value(f64),
}

pub fn sample(h: &dyn SignalHistory, kind: NumericKind, i: Option<usize>) -> Sample {
    if h.shape() == SignalShape::Bit && kind == NumericKind::Unsigned {
        return match h.bit(i) {
            Bit::Zero => Sample::Value(0.0),
            Bit::One => Sample::Value(1.0),
            Bit::Unavailable => Sample::Missing,
            Bit::Other => match h.value_view(i) {
                ValueView::Logic(l) if l.width == 1 => match l.bit(0) {
                    b'h' | b'H' => Sample::Value(1.0),
                    b'l' | b'L' => Sample::Value(0.0),
                    _ => Sample::Undefined,
                },
                _ => Sample::Undefined,
            },
            Bit::X | Bit::Z | Bit::DontCare => Sample::Undefined,
        };
    }
    let view = h.value_view(i);
    if matches!(view, ValueView::Unavailable) {
        return Sample::Missing;
    }
    kind.read(&view).map_or(Sample::Undefined, Sample::Value)
}

pub(crate) struct InputHistory<'a> {
    pub source: &'a dyn SignalHistory,
    pub projection: Option<BitSlice>,
}

impl InputHistory<'_> {
    pub fn number(&self, kind: NumericKind, index: Option<usize>) -> Option<f64> {
        let Some(slice) = self.projection else {
            return match sample(self.source, kind, index) {
                Sample::Value(v) => Some(v),
                _ => None,
            };
        };
        let ValueView::Logic(bits) = self.source.value_view(index) else {
            return None;
        };
        if slice.width() == 1 && kind == NumericKind::Unsigned {
            return match bits.bit(slice.first as usize) {
                b'0' | b'l' => Some(0.0),
                b'1' | b'h' => Some(1.0),
                _ => None,
            };
        }
        kind.read_bits(slice.width() as usize, |i| bits.bit(slice.offset(i as u32)))
    }
}

impl SignalHistory for InputHistory<'_> {
    fn shape(&self) -> SignalShape {
        self.projection
            .map_or_else(|| self.source.shape(), BitSlice::shape)
    }
    fn len(&self) -> usize {
        self.source.len()
    }
    fn time(&self, i: usize) -> u64 {
        self.source.time(i)
    }
    fn value(&self, i: Option<usize>) -> WaveValue {
        self.projection.map_or_else(
            || self.source.value(i),
            |p| p.value(self.source.value_view(i)),
        )
    }
    fn value_view(&self, i: Option<usize>) -> ValueView<'_> {
        self.projection.map_or_else(
            || self.source.value_view(i),
            |p| ValueView::owned(p.value(self.source.value_view(i))),
        )
    }
    fn bit(&self, i: Option<usize>) -> Bit {
        match self.projection {
            Some(slice) => match self.source.value_view(i) {
                ValueView::Logic(bits) => Bit::from_ascii(bits.bit(slice.first as usize)),
                _ => Bit::Unavailable,
            },
            None => self.source.bit(i),
        }
    }
}
