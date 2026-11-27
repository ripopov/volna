//! Table format preferences and adaptation of recorded attributes to shared translators.

use super::columns::{CellValue, attributes};
use crate::data::text::{format_attribute, push_limited, truncate};
use crate::data::translator::{NumericKind, TableCellStyle, Translated, Translator, Translators};
use std::sync::Arc;
use volna_trace::data::transactions::AttributeValue;
use volna_trace::data::value_view::{LogicView, ValueView};
use volna_trace::data::{SignalHistory, SignalShape};

/// Maximum bits materialized per table cell to stay within the panel byte budget.
pub const MAX_BITMASK_BITS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitState {
    Zero,
    One,
    Undef,
    HighImp,
    DontCare,
    Weak,
}

impl BitState {
    #[must_use]
    pub fn from_ascii(byte: u8) -> Self {
        match byte {
            b'0' => Self::Zero,
            b'1' => Self::One,
            b'z' | b'Z' => Self::HighImp,
            b'-' => Self::DontCare,
            b'l' | b'L' | b'h' | b'H' => Self::Weak,
            _ => Self::Undef,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellBitmask {
    /// Bits in display order (MSB first), capped at [`MAX_BITMASK_BITS`].
    pub bits: Vec<BitState>,
    /// True width of the underlying value in bits.
    pub width: usize,
}

impl CellBitmask {
    #[must_use]
    pub fn from_logic(bits: &LogicView<'_>) -> Self {
        let count = bits.width.min(MAX_BITMASK_BITS);
        Self {
            bits: (0..count)
                .map(|i| BitState::from_ascii(bits.bit(i)))
                .collect(),
            width: bits.width,
        }
    }

    #[must_use]
    pub fn from_u64(value: u64) -> Self {
        Self {
            bits: (0..64)
                .map(|i| {
                    if (value >> (63 - i)) & 1 != 0 {
                        BitState::One
                    } else {
                        BitState::Zero
                    }
                })
                .collect(),
            width: 64,
        }
    }

    #[must_use]
    pub fn bytes(&self) -> usize {
        self.bits.len() * std::mem::size_of::<BitState>()
    }

    #[must_use]
    pub fn summary(&self) -> String {
        if self.width > self.bits.len() {
            return format!("{} bits", self.width);
        }
        let ones = self.bits.iter().filter(|&&b| b == BitState::One).count();
        let non_binary = self
            .bits
            .iter()
            .any(|b| !matches!(b, BitState::Zero | BitState::One));
        if non_binary {
            format!("{ones}/{} · x/z", self.width)
        } else {
            format!("{ones}/{}", self.width)
        }
    }
}

pub(super) struct FormattedCell {
    pub text: String,
    pub bitmask: Option<CellBitmask>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TableFormats {
    pub numbers: String,
    pub columns: Vec<Option<String>>,
}

impl TableFormats {
    #[must_use]
    pub fn new(columns: usize) -> Self {
        Self {
            numbers: "hex".into(),
            columns: vec![None; columns],
        }
    }

    pub fn bytes(&self) -> usize {
        self.numbers.len()
            + self.columns.len() * std::mem::size_of::<Option<String>>()
            + self
                .columns
                .iter()
                .flatten()
                .map(String::len)
                .sum::<usize>()
    }

    pub fn translator(
        &self,
        translators: &Translators,
        column: usize,
        shape: SignalShape,
    ) -> Arc<dyn Translator> {
        self.columns
            .get(column)
            .and_then(Option::as_deref)
            .and_then(|id| translators.get(id))
            .filter(|t| t.applies(shape))
            .unwrap_or_else(|| self.default_translator(translators, shape))
    }

    pub(super) fn default_translator(
        &self,
        translators: &Translators,
        shape: SignalShape,
    ) -> Arc<dyn Translator> {
        matches!(shape, SignalShape::Vector { .. })
            .then(|| translators.get(&self.numbers))
            .flatten()
            .filter(|t| t.applies(shape))
            .unwrap_or_else(|| translators.default_for(shape))
    }

    pub(super) fn format_cell(
        &self,
        translators: &Translators,
        value: CellValue<'_>,
        source_index: usize,
        limit: usize,
        requested: Option<&dyn Translator>,
    ) -> Translated {
        match value {
            CellValue::Missing => {
                Translated::bounded("—", volna_trace::data::ValueKind::Normal, limit)
            }
            CellValue::Time(value) => Translated::bounded(
                &value.to_string(),
                volna_trace::data::ValueKind::Normal,
                limit,
            ),
            CellValue::Number(value) => self.format_attribute(
                translators,
                &AttributeValue::U64(value),
                source_index,
                requested,
                limit,
            ),
            CellValue::Text(value) => {
                Translated::bounded(value, volna_trace::data::ValueKind::Normal, limit)
            }
            CellValue::Attribute(value) => {
                self.format_attribute(translators, value, source_index, requested, limit)
            }
            CellValue::Attributes(items) => {
                let mut text = String::new();
                let mut attributes = attributes(items).peekable();
                if attributes.peek().is_none() {
                    return Translated::bounded("—", volna_trace::data::ValueKind::Normal, limit);
                }
                let budget = limit.saturating_sub("…".len());
                let mut cut = false;
                'attributes: while let Some(attribute) = attributes.next() {
                    let separator = if text.is_empty() { "" } else { " · " };
                    for part in [separator, attribute.key.as_str(), "="] {
                        let room = budget.saturating_sub(text.len());
                        push_limited(&mut text, part, budget);
                        if part.len() > room {
                            cut = true;
                            break 'attributes;
                        }
                    }
                    let remaining = budget.saturating_sub(text.len());
                    let value = self.format_attribute(
                        translators,
                        &attribute.value,
                        source_index,
                        requested,
                        remaining,
                    );
                    cut |= value.truncated;
                    text.push_str(&value.text);
                    if budget.saturating_sub(text.len()) < " · ".len() + 1 {
                        cut |= attributes.peek().is_some() || remaining < "…".len();
                        break;
                    }
                }
                if cut && limit >= "…".len() && !text.ends_with('…') {
                    text.push('…');
                }
                Translated {
                    text,
                    kind: volna_trace::data::ValueKind::Normal,
                    truncated: cut,
                }
            }
        }
    }

    fn format_attribute(
        &self,
        translators: &Translators,
        value: &AttributeValue,
        column: usize,
        requested: Option<&dyn Translator>,
        limit: usize,
    ) -> Translated {
        let translator =
            attribute_shape(value).map(|shape| self.translator(translators, column, shape));
        attribute(value, requested.or(translator.as_deref()), limit)
    }

    pub(super) fn format_signal(
        &self,
        translators: &Translators,
        column: usize,
        history: &dyn SignalHistory,
        sample: Option<usize>,
        limit: usize,
    ) -> Translated {
        let value = if sample.is_none() {
            ValueView::Unavailable
        } else {
            history.value_view(sample)
        };
        translate(
            self.translator(translators, column, history.shape())
                .as_ref(),
            &value,
            limit,
        )
    }

    pub(super) fn prepare_cell(
        &self,
        translators: &Translators,
        value: CellValue<'_>,
        source_index: usize,
        limit: usize,
    ) -> FormattedCell {
        let text = self
            .format_cell(translators, value, source_index, limit, None)
            .text;
        let bitmask = match value {
            CellValue::Number(v) => self
                .is_bitmask(translators, source_index, SignalShape::Vector { width: 64 })
                .then(|| CellBitmask::from_u64(v)),
            CellValue::Attribute(a) => attribute_shape(a)
                .filter(|&shape| self.is_bitmask(translators, source_index, shape))
                .and_then(|_| attribute_bitmask(a)),
            _ => None,
        };
        FormattedCell { text, bitmask }
    }

    pub(super) fn prepare_signal(
        &self,
        translators: &Translators,
        column: usize,
        history: &dyn SignalHistory,
        sample: Option<usize>,
        limit: usize,
    ) -> FormattedCell {
        let translator = self.translator(translators, column, history.shape());
        let value = if sample.is_none() {
            ValueView::Unavailable
        } else {
            history.value_view(sample)
        };
        let text = translate(translator.as_ref(), &value, limit).text;
        let bitmask = (translator.table_cell_style() == TableCellStyle::Bitmask)
            .then(|| match &value {
                ValueView::Logic(bits) => Some(CellBitmask::from_logic(bits)),
                _ => None,
            })
            .flatten();
        FormattedCell { text, bitmask }
    }

    pub(super) fn preview_cell(
        &self,
        translators: &Translators,
        value: CellValue<'_>,
        source_index: usize,
        translator: &dyn Translator,
        limit: usize,
    ) -> String {
        if translator.table_cell_style() == TableCellStyle::Bitmask
            && let Some(mask) = cell_bitmask(value)
        {
            return mask.summary();
        }
        self.format_cell(translators, value, source_index, limit, Some(translator))
            .text
    }

    pub(super) fn preview_signal(
        translator: &dyn Translator,
        value: &ValueView<'_>,
        limit: usize,
    ) -> String {
        if translator.table_cell_style() == TableCellStyle::Bitmask
            && let ValueView::Logic(bits) = value
        {
            return CellBitmask::from_logic(bits).summary();
        }
        translate(translator, value, limit).text
    }

    fn is_bitmask(&self, translators: &Translators, column: usize, shape: SignalShape) -> bool {
        self.translator(translators, column, shape)
            .table_cell_style()
            == TableCellStyle::Bitmask
    }
}

fn cell_bitmask(value: CellValue<'_>) -> Option<CellBitmask> {
    match value {
        CellValue::Number(v) => Some(CellBitmask::from_u64(v)),
        CellValue::Attribute(a) => attribute_bitmask(a),
        _ => None,
    }
}

fn attribute_bitmask(value: &AttributeValue) -> Option<CellBitmask> {
    match value {
        AttributeValue::I64(v) => Some(CellBitmask::from_u64(*v as u64)),
        AttributeValue::U64(v) | AttributeValue::Pointer(v) => Some(CellBitmask::from_u64(*v)),
        AttributeValue::Logic {
            width,
            states,
            data,
        } if *width > 0 => Some(CellBitmask::from_logic(&LogicView::packed_lsb(
            *width, *states, data,
        ))),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct FormatChoice {
    pub translator: Option<String>,
    pub label: String,
    pub preview: String,
    pub checked: bool,
}

pub(super) fn number_translators(translators: &Translators) -> Vec<Arc<dyn Translator>> {
    translators
        .applicable(SignalShape::Vector { width: 64 })
        .into_iter()
        .filter(|t| {
            matches!(
                t.numeric_kind(),
                Some(NumericKind::Unsigned | NumericKind::Signed)
            )
        })
        .collect()
}

pub(super) fn attribute_shape(value: &AttributeValue) -> Option<SignalShape> {
    match value {
        AttributeValue::I64(_) | AttributeValue::U64(_) | AttributeValue::Pointer(_) => {
            Some(SignalShape::Vector { width: 64 })
        }
        AttributeValue::Logic { width, .. } => u32::try_from(*width)
            .ok()
            .filter(|width| *width > 0)
            .map(|width| {
                if width == 1 {
                    SignalShape::Bit
                } else {
                    SignalShape::Vector { width }
                }
            }),
        AttributeValue::F64(_) => Some(SignalShape::Real),
        _ => None,
    }
}

pub(super) fn translate(
    translator: &dyn Translator,
    value: &ValueView<'_>,
    limit: usize,
) -> Translated {
    let prefix = if matches!(value, ValueView::Logic(_)) {
        translator.prefix()
    } else {
        ""
    };
    let mut result = translator.translate_view(value, limit.saturating_sub(prefix.len()));
    if !prefix.is_empty() {
        result.text.insert_str(0, prefix);
        result.truncated |= result.text.len() > limit;
        result.text = truncate(result.text, limit);
    }
    result
}

pub(super) fn attribute(
    value: &AttributeValue,
    translator: Option<&dyn Translator>,
    limit: usize,
) -> Translated {
    let Some(translator) =
        translator.filter(|t| attribute_shape(value).is_some_and(|s| t.applies(s)))
    else {
        let full_len = match value {
            AttributeValue::Text(text) => text.len(),
            AttributeValue::Enum { value, name } => {
                name.len().saturating_add(format!(" ({value})").len())
            }
            _ => format_attribute(value, usize::MAX).len(),
        };
        return Translated {
            text: format_attribute(value, limit),
            kind: volna_trace::data::ValueKind::Normal,
            truncated: full_len > limit,
        };
    };
    let value = match value {
        AttributeValue::I64(v) => *v as u64,
        AttributeValue::U64(v) | AttributeValue::Pointer(v) => *v,
        AttributeValue::F64(v) => return translate(translator, &ValueView::Real(*v), limit),
        AttributeValue::Logic {
            width,
            states,
            data,
        } => {
            return translate(
                translator,
                &ValueView::Logic(LogicView::packed_lsb(*width, *states, data)),
                limit,
            );
        }
        _ => unreachable!("translatable attribute"),
    };
    // Preserve the recorded width when a translator interprets the sign bit.
    let view = ValueView::Logic(LogicView::ascii(format!("{value:064b}").into_bytes()));
    let mut result = translator.translate_view(&view, 128);
    if !translator.prefix().is_empty() {
        let digits = result.text.trim_start_matches('0');
        result.text = format!(
            "{}{}",
            translator.prefix(),
            if digits.is_empty() { "0" } else { digits }
        );
    }
    result.truncated |= result.text.len() > limit;
    result.text = truncate(result.text, limit);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_integers_keep_exact_bits_and_nonnumeric_attributes_keep_their_meaning() {
        let translators = Translators::builtin();
        let hex = translators.get("hex").unwrap();
        let unsigned = translators.get("udec").unwrap();
        let signed = translators.get("sdec").unwrap();
        for (value, hex_text, unsigned_text, signed_text) in [
            (
                AttributeValue::U64(u64::MAX),
                "0xffffffffffffffff",
                "18446744073709551615",
                "-1",
            ),
            (
                AttributeValue::I64(i64::MIN),
                "0x8000000000000000",
                "9223372036854775808",
                "-9223372036854775808",
            ),
            (AttributeValue::Pointer(128), "0x80", "128", "128"),
            (AttributeValue::U64(0), "0x0", "0", "0"),
        ] {
            assert_eq!(attribute(&value, Some(hex.as_ref()), 64).text, hex_text);
            assert_eq!(
                attribute(&value, Some(unsigned.as_ref()), 64).text,
                unsigned_text
            );
            assert_eq!(
                attribute(&value, Some(signed.as_ref()), 64).text,
                signed_text
            );
        }
        for value in [
            AttributeValue::Bool(true),
            AttributeValue::Time(128),
            AttributeValue::Text("αβ".into()),
            AttributeValue::Enum {
                value: 7,
                name: "Ready".into(),
            },
        ] {
            assert_eq!(
                attribute(&value, Some(hex.as_ref()), 64).text,
                format_attribute(&value, 64)
            );
        }
    }
}
