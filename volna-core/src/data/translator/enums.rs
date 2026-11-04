//! Trace-owned enum translators with allocation-free indexed value lookup.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use volna_trace::data::source::EnumTable;
use volna_trace::data::value_view::ValueView;
use volna_trace::data::{Hierarchy, SignalShape, VarId};

use super::{HexTranslator, Translated, Translator, not_applicable, raw_bits};

pub(crate) struct EnumTranslators {
    hierarchy: Hierarchy,
    tables: HashMap<u32, OnceLock<Arc<dyn Translator>>>,
}

impl EnumTranslators {
    pub(crate) fn new(hierarchy: Hierarchy) -> Self {
        let tables = hierarchy
            .enum_tables()
            .iter()
            .map(|table| (table.id, OnceLock::new()))
            .collect();
        Self { hierarchy, tables }
    }

    pub(crate) fn get(&self, var: VarId) -> Option<Arc<dyn Translator>> {
        let var = self.hierarchy.get_var(var)?;
        if !matches!(var.shape, SignalShape::Bit | SignalShape::Vector { .. }) {
            return None;
        }
        let id = var.enum_table?;
        let cached = self.tables.get(&id)?;
        Some(
            cached
                .get_or_init(|| {
                    Arc::new(EnumTranslator::new(
                        self.hierarchy.enum_table(id).expect("declared enum table"),
                    ))
                })
                .clone(),
        )
    }
}

struct EnumLiteral {
    bits: String,
    name: String,
}

struct EnumTranslator {
    literals: Vec<EnumLiteral>,
}

impl EnumTranslator {
    fn new(table: &EnumTable) -> Self {
        let mut literals: Vec<_> = table
            .entries
            .iter()
            .map(|(name, bits)| EnumLiteral {
                bits: bits.trim_start_matches('0').to_ascii_lowercase(),
                name: name.clone(),
            })
            .collect();
        // Stable ordering keeps the first declared name for duplicate encodings.
        literals.sort_by(|a, b| a.bits.cmp(&b.bits));
        literals.dedup_by(|a, b| a.bits == b.bits);
        Self { literals }
    }
}

impl Translator for EnumTranslator {
    fn id(&self) -> &'static str {
        "enum"
    }
    fn name(&self) -> &'static str {
        "Enum"
    }
    fn badge(&self) -> &'static str {
        "enum"
    }
    fn applies(&self, shape: SignalShape) -> bool {
        matches!(shape, SignalShape::Bit | SignalShape::Vector { .. })
    }
    fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated {
        let ValueView::Logic(bits) = value else {
            return not_applicable(value, limit);
        };
        let start = (0..bits.width)
            .find(|&i| bits.bit(i) != b'0')
            .unwrap_or(bits.width);
        if let Ok(index) = self.literals.binary_search_by(|literal| {
            literal
                .bits
                .bytes()
                .cmp((start..bits.width).map(|i| bits.bit(i)))
        }) {
            return Translated::bounded(&self.literals[index].name, bits.kind(), limit);
        }
        if bits.width == 1 {
            raw_bits(value, limit)
        } else {
            HexTranslator.translate_view(value, limit)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_trace::data::{ValueKind, WaveValue};

    #[test]
    fn enum_labels_match_padded_wide_and_four_state_values_with_bounded_fallback() {
        let wide = format!("1{}", "0".repeat(127));
        let table = volna_trace::data::source::EnumTable {
            id: 7,
            entries: vec![
                ("空闲 idle".into(), "000".into()),
                ("RUN".into(), "01".into()),
                ("UNKNOWN".into(), "XX".into()),
                ("WIDE".into(), wide.clone()),
            ],
        };
        let t = EnumTranslator::new(&table);
        for (bits, text, kind) in [
            ("0", "空闲 idle", ValueKind::Normal),
            ("0001", "RUN", ValueKind::Normal),
            ("0010", "2", ValueKind::Normal),
            ("xx", "UNKNOWN", ValueKind::Undef),
            ("zz", "z", ValueKind::HighImp),
            (wide.as_str(), "WIDE", ValueKind::Normal),
        ] {
            let value = WaveValue::Bits(bits.into());
            let translated = t.translate(&value);
            assert_eq!((translated.text.as_str(), translated.kind), (text, kind));
            let bounded = t.translate_view(&ValueView::borrowed(&value), 6);
            assert!(bounded.text.len() <= 6);
            assert_eq!(bounded.truncated, text.len() > 6);
        }
        assert_eq!(t.translate(&WaveValue::Unavailable).text, "—");
        let bit = EnumTranslator::new(&table);
        assert_eq!(bit.translate(&WaveValue::Bits("1".into())).text, "RUN");
        assert_eq!(bit.translate(&WaveValue::Bits("z".into())).text, "z");
    }

    #[test]
    fn indexed_enum_lookup_preserves_alias_precedence_for_packed_and_padded_values() {
        let mut entries: Vec<_> = (0..8192u16)
            .step_by(2)
            .rev()
            .map(|value| (format!("state_{value}"), format!("{value:b}")))
            .collect();
        entries.push(("later_zero_alias".into(), "0000".into()));
        entries.push(("later_two_alias".into(), "0010".into()));
        let translator = EnumTranslator::new(&EnumTable { id: 0, entries });
        for value in 0..8192u16 {
            let expected = if value.is_multiple_of(2) {
                format!("state_{value}")
            } else {
                format!("{value:04x}")
            };
            let packed = value.to_le_bytes();
            let view = ValueView::Logic(volna_trace::data::value_view::LogicView::packed_lsb(
                16, 2, &packed,
            ));
            assert_eq!(translator.translate_view(&view, 64).text, expected);
            let padded = WaveValue::Bits(format!("{value:016b}"));
            assert_eq!(translator.translate(&padded).text, expected);
        }
    }
}
