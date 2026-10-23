//! Value translators turn a [`WaveValue`] into display text.

use std::fmt::Write as _;
use std::sync::Arc;

use volna_trace::data::value::{SignalShape, ValueKind, WaveValue};
use volna_trace::data::value_view::{LogicView, ValueView};

use super::text::{truncate, truncate_ref};

#[derive(Clone, PartialEq, Debug)]
pub struct Translated {
    pub text: String,
    pub kind: ValueKind,
    pub truncated: bool,
}

impl Translated {
    fn bounded(text: &str, kind: ValueKind, limit: usize) -> Self {
        Self {
            text: truncate_ref(text, limit),
            kind,
            truncated: text.len() > limit,
        }
    }

    fn chars(chars: impl IntoIterator<Item = char>, kind: ValueKind, limit: usize) -> Self {
        let mut text = String::new();
        let mut truncated = false;
        for c in chars {
            if text.len().saturating_add(c.len_utf8()) > limit {
                text.push(c);
                truncated = true;
                break;
            }
            text.push(c);
        }
        if truncated {
            text = truncate(text, limit);
        }
        Self {
            text,
            kind,
            truncated,
        }
    }
}

pub trait Translator: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn badge(&self) -> &'static str;
    fn applies(&self, shape: SignalShape) -> bool;
    /// Format within a UTF-8 byte budget without materializing an unbounded input.
    fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated;
    fn translate(&self, value: &WaveValue) -> Translated {
        self.translate_view(&ValueView::borrowed(value), usize::MAX)
    }
    fn prefix(&self) -> &'static str {
        ""
    }
    fn numeric_kind(&self) -> Option<NumericKind> {
        None
    }
    fn numeric(&self, value: &ValueView<'_>) -> Option<f64> {
        self.numeric_kind()?.read(value)
    }
    fn limits(&self, shape: SignalShape) -> Option<(f64, f64)> {
        self.numeric_kind()?.limits(shape)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericKind {
    Unsigned,
    Signed,
    Float,
    Real,
}

impl NumericKind {
    #[must_use]
    pub fn read(self, value: &ValueView<'_>) -> Option<f64> {
        let bits = match (self, value) {
            (Self::Real, ValueView::Real(v)) => return v.is_finite().then_some(*v),
            (Self::Real, _) => return None,
            (_, ValueView::Logic(bits)) if bits.width > 0 => bits,
            _ => return None,
        };
        let w = bits.width;
        let bit = |i: usize| match bits.bit(i) {
            b'0' => Some(0u64),
            b'1' => Some(1),
            _ => None,
        };
        if w > 64 {
            let mut acc = 0.0f64;
            for i in 0..w {
                acc = acc * 2.0 + bit(i)? as f64;
            }
            let v = match self {
                Self::Signed if bits.bit(0) == b'1' => acc - 2f64.powi(w as i32),
                Self::Unsigned | Self::Signed => acc,
                _ => return None,
            };
            return v.is_finite().then_some(v);
        }
        let raw = bits.to_u64()?;
        let v = match self {
            Self::Signed if w == 64 => raw as i64 as f64,
            Self::Signed if (raw >> (w - 1)) & 1 == 1 => raw as f64 - 2f64.powi(w as i32),
            Self::Unsigned | Self::Signed => raw as f64,
            Self::Float => match w {
                16 => half_to_f64(raw as u16),
                32 => f64::from(f32::from_bits(raw as u32)),
                64 => f64::from_bits(raw),
                _ => return None,
            },
            Self::Real => unreachable!(),
        };
        v.is_finite().then_some(v)
    }

    #[must_use]
    pub fn limits(self, shape: SignalShape) -> Option<(f64, f64)> {
        let width = match shape {
            SignalShape::Bit => 1,
            SignalShape::Vector { width } => width,
            _ => return None,
        };
        let (lo, hi) = match self {
            Self::Unsigned => (0.0, 2f64.powi(width as i32) - 1.0),
            Self::Signed => {
                let half = 2f64.powi(width as i32 - 1);
                (-half, half - 1.0)
            }
            Self::Float | Self::Real => return None,
        };
        (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
    }

    #[must_use]
    pub fn value_of(self, v: f64, shape: SignalShape) -> Option<WaveValue> {
        match (self, shape) {
            (Self::Real, SignalShape::Real) => Some(WaveValue::Real(v)),
            (Self::Unsigned | Self::Signed, SignalShape::Vector { width }) if width <= 127 => {
                let n = v.round().clamp(i128::MIN as f64, i128::MAX as f64) as i128;
                Some(WaveValue::Bits(
                    (0..width)
                        .rev()
                        .map(|i| if (n >> i) & 1 == 1 { '1' } else { '0' })
                        .collect(),
                ))
            }
            (Self::Float, SignalShape::Vector { width: 32 }) => {
                Some(WaveValue::Bits(format!("{:032b}", (v as f32).to_bits())))
            }
            (Self::Float, SignalShape::Vector { width: 64 }) => {
                Some(WaveValue::Bits(format!("{:064b}", v.to_bits())))
            }
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Translators {
    all: Vec<Arc<dyn Translator>>,
}

impl Translators {
    #[must_use]
    pub fn builtin() -> Self {
        let mut t = Translators { all: Vec::new() };
        t.register(Arc::new(BitTranslator));
        t.register(Arc::new(HexTranslator));
        t.register(Arc::new(BinaryTranslator));
        t.register(Arc::new(UnsignedTranslator));
        t.register(Arc::new(SignedTranslator));
        t.register(Arc::new(FloatTranslator));
        t.register(Arc::new(RealTranslator));
        t.register(Arc::new(TextTranslator));
        t
    }

    pub fn register(&mut self, translator: Arc<dyn Translator>) {
        self.all.retain(|t| t.id() != translator.id());
        self.all.push(translator);
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<dyn Translator>> {
        self.all.iter().find(|t| t.id() == id).cloned()
    }

    #[must_use]
    pub fn applicable(&self, shape: SignalShape) -> Vec<Arc<dyn Translator>> {
        self.all
            .iter()
            .filter(|t| t.applies(shape))
            .cloned()
            .collect()
    }

    #[must_use]
    pub fn default_for(&self, shape: SignalShape) -> Arc<dyn Translator> {
        let id = match shape {
            SignalShape::Bit | SignalShape::Event => "bit",
            SignalShape::Vector { .. } => "hex",
            SignalShape::Real => "real",
            SignalShape::Text => "text",
        };
        self.get(id).expect("built-in translator")
    }
}

fn non_binary_word(bits: &LogicView<'_>, limit: usize) -> Option<Translated> {
    let kind = bits.kind();
    let text = match kind {
        ValueKind::Normal => return None,
        ValueKind::Undef => "UNDEF",
        ValueKind::HighImp => "HIGHIMP",
        ValueKind::DontCare => "DON'T CARE",
        ValueKind::Weak => "WEAK",
    };
    Some(Translated::bounded(text, kind, limit))
}

fn not_applicable(value: &ValueView<'_>, limit: usize) -> Translated {
    Translated::bounded(
        if matches!(value, ValueView::Unavailable) {
            "—"
        } else {
            "?"
        },
        ValueKind::Undef,
        limit,
    )
}

struct BitTranslator;
impl Translator for BitTranslator {
    fn id(&self) -> &'static str {
        "bit"
    }
    fn name(&self) -> &'static str {
        "Bit"
    }
    fn badge(&self) -> &'static str {
        "bit"
    }
    fn applies(&self, shape: SignalShape) -> bool {
        matches!(shape, SignalShape::Bit | SignalShape::Event)
    }
    fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated {
        raw_bits(value, limit)
    }
}

fn raw_bits(value: &ValueView<'_>, limit: usize) -> Translated {
    let ValueView::Logic(bits) = value else {
        return not_applicable(value, limit);
    };
    Translated::chars(
        (0..bits.width).map(|i| char::from(bits.bit(i))),
        bits.kind(),
        limit,
    )
}

struct BinaryTranslator;
impl Translator for BinaryTranslator {
    fn prefix(&self) -> &'static str {
        "0b"
    }
    fn id(&self) -> &'static str {
        "bin"
    }
    fn numeric_kind(&self) -> Option<NumericKind> {
        Some(NumericKind::Unsigned)
    }
    fn name(&self) -> &'static str {
        "Binary"
    }
    fn badge(&self) -> &'static str {
        "bin"
    }
    fn applies(&self, shape: SignalShape) -> bool {
        shape.is_digital()
    }
    fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated {
        raw_bits(value, limit)
    }
}

struct HexTranslator;
impl Translator for HexTranslator {
    fn prefix(&self) -> &'static str {
        "0x"
    }
    fn id(&self) -> &'static str {
        "hex"
    }
    fn numeric_kind(&self) -> Option<NumericKind> {
        Some(NumericKind::Unsigned)
    }
    fn name(&self) -> &'static str {
        "Hexadecimal"
    }
    fn badge(&self) -> &'static str {
        "hex"
    }
    fn applies(&self, shape: SignalShape) -> bool {
        shape.is_digital()
    }
    fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated {
        let ValueView::Logic(bits) = value else {
            return not_applicable(value, limit);
        };
        let ndigits = bits.width.div_ceil(4);
        let chars = (0..ndigits).map(|d| {
            let end = bits.width - (ndigits - 1 - d) * 4;
            let start = end.saturating_sub(4);
            let mut v = 0u8;
            let mut special = None;
            for i in start..end {
                match bits.bit(i) {
                    b'0' => v <<= 1,
                    b'1' => v = (v << 1) | 1,
                    c => {
                        special = Some(match (special, c) {
                            (Some(b'x'), _) | (_, b'x' | b'u' | b'w') => b'x',
                            (Some(b'z'), _) | (_, b'z') => b'z',
                            (_, c) => c,
                        });
                    }
                }
            }
            special.map_or_else(|| char::from_digit(u32::from(v), 16).unwrap(), char::from)
        });
        Translated::chars(chars, bits.kind(), limit)
    }
}

fn decimal(bits: &LogicView<'_>, signed: bool, limit: usize) -> Translated {
    if let Some(value) = non_binary_word(bits, limit) {
        return value;
    }
    let negative = signed && bits.width > 0 && bits.bit(0) == b'1';
    let last_one = negative.then(|| (0..bits.width).rfind(|&i| bits.bit(i) == b'1').unwrap());
    let bit = |i| {
        let value = u64::from(bits.bit(i) == b'1');
        if last_one.is_some_and(|last| i < last) {
            1 - value
        } else {
            value
        }
    };
    let first = (0..bits.width).find(|&i| bit(i) != 0).unwrap_or(bits.width);
    // Decimal conversion is quadratic; never build an integer larger than the output budget.
    if bits.width - first > limit.saturating_mul(4) {
        return Translated {
            text: truncate_ref("…", limit),
            kind: ValueKind::Normal,
            truncated: true,
        };
    }
    let mut limbs: Vec<u32> = vec![0];
    for i in first..bits.width {
        let mut carry = bit(i);
        for limb in &mut limbs {
            let v = u64::from(*limb) * 2 + carry;
            *limb = (v % 1_000_000_000) as u32;
            carry = v / 1_000_000_000;
        }
        if carry > 0 {
            limbs.push(carry as u32);
        }
    }
    let mut text = if negative { "-".into() } else { String::new() };
    for (i, limb) in limbs.iter().rev().enumerate() {
        if i == 0 {
            write!(text, "{limb}").unwrap();
        } else {
            write!(text, "{limb:09}").unwrap();
        }
    }
    Translated::bounded(&text, ValueKind::Normal, limit)
}

struct UnsignedTranslator;
impl Translator for UnsignedTranslator {
    fn id(&self) -> &'static str {
        "udec"
    }
    fn numeric_kind(&self) -> Option<NumericKind> {
        Some(NumericKind::Unsigned)
    }
    fn name(&self) -> &'static str {
        "Unsigned decimal"
    }
    fn badge(&self) -> &'static str {
        "dec"
    }
    fn applies(&self, shape: SignalShape) -> bool {
        shape.is_digital()
    }
    fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated {
        match value {
            ValueView::Logic(bits) => decimal(bits, false, limit),
            _ => not_applicable(value, limit),
        }
    }
}

struct SignedTranslator;
impl Translator for SignedTranslator {
    fn id(&self) -> &'static str {
        "sdec"
    }
    fn numeric_kind(&self) -> Option<NumericKind> {
        Some(NumericKind::Signed)
    }
    fn name(&self) -> &'static str {
        "Signed decimal"
    }
    fn badge(&self) -> &'static str {
        "sdec"
    }
    fn applies(&self, shape: SignalShape) -> bool {
        matches!(shape, SignalShape::Vector { .. })
    }
    fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated {
        match value {
            ValueView::Logic(bits) => decimal(bits, true, limit),
            _ => not_applicable(value, limit),
        }
    }
}

fn half_to_f64(h: u16) -> f64 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1f);
    let frac = f64::from(h & 0x3ff);
    match exp {
        0 => sign * frac * 2f64.powi(-24),
        0x1f => {
            if frac == 0.0 {
                sign * f64::INFINITY
            } else {
                f64::NAN
            }
        }
        _ => sign * (1.0 + frac / 1024.0) * 2f64.powi(exp - 15),
    }
}

fn format_float(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0.0 {
            "+Inf".into()
        } else {
            "-Inf".into()
        }
    } else if v == 0.0 {
        "0.0".into()
    } else if v.abs() >= 1e-4 && v.abs() < 1e9 {
        let s = format!("{v:.6}");
        let s = s.trim_end_matches('0');
        if s.ends_with('.') {
            format!("{s}0")
        } else {
            s.to_string()
        }
    } else {
        format!("{v:.5e}")
    }
}

struct FloatTranslator;
impl Translator for FloatTranslator {
    fn id(&self) -> &'static str {
        "float"
    }
    fn numeric_kind(&self) -> Option<NumericKind> {
        Some(NumericKind::Float)
    }
    fn name(&self) -> &'static str {
        "IEEE 754 float"
    }
    fn badge(&self) -> &'static str {
        "flt"
    }
    fn applies(&self, shape: SignalShape) -> bool {
        matches!(
            shape,
            SignalShape::Vector {
                width: 16 | 32 | 64
            }
        )
    }
    fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated {
        let ValueView::Logic(bits) = value else {
            return not_applicable(value, limit);
        };
        if let Some(value) = non_binary_word(bits, limit) {
            return value;
        }
        let Some(raw) = bits.to_u64() else {
            return not_applicable(value, limit);
        };
        let number = match bits.width {
            16 => half_to_f64(raw as u16),
            32 => f64::from(f32::from_bits(raw as u32)),
            64 => f64::from_bits(raw),
            _ => return not_applicable(value, limit),
        };
        Translated::bounded(&format_float(number), ValueKind::Normal, limit)
    }
}

struct RealTranslator;
impl Translator for RealTranslator {
    fn id(&self) -> &'static str {
        "real"
    }
    fn numeric_kind(&self) -> Option<NumericKind> {
        Some(NumericKind::Real)
    }
    fn name(&self) -> &'static str {
        "Real"
    }
    fn badge(&self) -> &'static str {
        "real"
    }
    fn applies(&self, shape: SignalShape) -> bool {
        shape == SignalShape::Real
    }
    fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated {
        match value {
            ValueView::Real(number) => {
                Translated::bounded(&format_float(*number), ValueKind::Normal, limit)
            }
            _ => not_applicable(value, limit),
        }
    }
}

struct TextTranslator;
impl Translator for TextTranslator {
    fn id(&self) -> &'static str {
        "text"
    }
    fn name(&self) -> &'static str {
        "Text"
    }
    fn badge(&self) -> &'static str {
        "str"
    }
    fn applies(&self, shape: SignalShape) -> bool {
        shape == SignalShape::Text
    }
    fn translate_view(&self, value: &ValueView<'_>, limit: usize) -> Translated {
        match value {
            ValueView::Text(text) => Translated::bounded(text, ValueKind::Normal, limit),
            ValueView::Bytes(bytes) => Translated::chars(
                std::iter::once('"')
                    .chain(
                        bytes
                            .iter()
                            .flat_map(|b| std::ascii::escape_default(*b).map(char::from)),
                    )
                    .chain(std::iter::once('"')),
                ValueKind::Normal,
                limit,
            ),
            _ => not_applicable(value, limit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_packed_values_share_waveform_translation_and_respect_byte_limits() {
        let translators = Translators::builtin();
        for bits in [
            "00111111110000000000000000000000",
            "11111111111111111111111111111111",
            "0101x1z00110",
        ] {
            let mut packed = vec![0; bits.len().div_ceil(4)];
            for (i, c) in bits.bytes().rev().enumerate() {
                vtr::signal::set_code(&mut packed, 4, i, vtr::signal::code_from_ascii(c));
            }
            let view = ValueView::Logic(LogicView::packed_lsb(bits.len() as u64, 4, &packed));
            for translator in translators.applicable(SignalShape::Vector {
                width: bits.len() as u32,
            }) {
                let expected = translator.translate(&WaveValue::Bits(bits.into()));
                assert_eq!(
                    translator.translate_view(&view, usize::MAX),
                    expected,
                    "{}",
                    translator.id()
                );
                for limit in 0..=expected.text.len() + 1 {
                    let bounded = translator.translate_view(&view, limit);
                    assert!(bounded.text.len() <= limit);
                    assert_eq!(bounded.kind, expected.kind);
                    assert_eq!(bounded.truncated, expected.text.len() > limit);
                    if !bounded.truncated {
                        assert_eq!(bounded.text, expected.text);
                    }
                }
            }
        }
        for value in [
            WaveValue::Text("αβγδ".into()),
            WaveValue::Bytes(vec![0, 255, b'"', b'\\']),
        ] {
            let text = translators.get("text").unwrap();
            for limit in 0..=32 {
                let bounded = text.translate_view(&ValueView::borrowed(&value), limit);
                assert!(bounded.text.len() <= limit);
                assert_eq!(bounded.truncated, text.translate(&value).text.len() > limit);
            }
        }
    }

    #[test]
    fn huge_decimal_previews_and_wide_twos_complement_stay_bounded() {
        let translators = Translators::builtin();
        let huge = ValueView::Logic(LogicView::ascii(vec![b'1'; 1_000_000]));
        for id in ["hex", "bin", "udec"] {
            let result = translators.get(id).unwrap().translate_view(&huge, 32);
            assert!(result.truncated);
            assert!(result.text.len() <= 32);
        }
        assert_eq!(
            translators
                .get("sdec")
                .unwrap()
                .translate_view(&huge, 32)
                .text,
            "-1"
        );
        assert_eq!(
            tr("sdec", &format!("1{}", "0".repeat(79))),
            "-604462909807314587353088"
        );
    }

    #[test]
    fn packed_logic_reads_as_numbers() {
        use volna_trace::data::value_view::LogicView;
        let two = LogicView::packed_lsb(12, 2, &[0x34, 0xfa]);
        assert_eq!(two.to_u64(), Some(0xa34));
        assert_eq!(
            NumericKind::Signed.read(&ValueView::Logic(two)),
            Some(f64::from(0xa34) - 4096.0)
        );
        let four = LogicView::packed_lsb(4, 4, &[0b01_00_01_01]);
        assert_eq!(four.to_u64(), Some(0b1011));
        assert_eq!(LogicView::packed_lsb(4, 4, &[0b10_00_01_01]).to_u64(), None);
    }

    #[test]
    fn vectors_past_f64_range_read_as_nothing() {
        let ones = WaveValue::Bits("1".repeat(1100));
        let view = ValueView::borrowed(&ones);
        assert_eq!(NumericKind::Unsigned.read(&view), None);
        assert_eq!(NumericKind::Signed.read(&view), None);
        let wide = SignalShape::Vector { width: 1100 };
        assert_eq!(NumericKind::Unsigned.limits(wide), None);
        let ok = WaveValue::Bits(format!("1{}", "0".repeat(99)));
        assert_eq!(
            NumericKind::Unsigned.read(&ValueView::borrowed(&ok)),
            Some(2f64.powi(99))
        );
    }

    #[test]
    fn raw_text_bytes_are_unambiguously_escaped() {
        let translated = Translators::builtin()
            .get("text")
            .unwrap()
            .translate(&WaveValue::Bytes(vec![b'a', 0, 255, b'\\', b'\n', b'"']));
        assert_eq!(translated.text, "\"a\\x00\\xff\\\\\\n\\\"\"");
    }

    fn tr(id: &str, bits: &str) -> String {
        Translators::builtin()
            .get(id)
            .unwrap()
            .translate(&WaveValue::Bits(bits.into()))
            .text
    }

    #[test]
    fn hex_digitwise() {
        assert_eq!(tr("hex", "00111111"), "3f");
        assert_eq!(tr("hex", "1111"), "f");
        assert_eq!(tr("hex", "101"), "5");
        assert_eq!(tr("hex", "0011x1111111"), "3xf");
        assert_eq!(tr("hex", "zzzz0000"), "z0");
        assert_eq!(tr("hex", "zxzz0000"), "x0");
    }

    #[test]
    fn decimal() {
        assert_eq!(tr("udec", "0"), "0");
        assert_eq!(tr("udec", "11111111"), "255");
        assert_eq!(tr("sdec", "11111111"), "-1");
        assert_eq!(tr("sdec", "10000000"), "-128");
        assert_eq!(tr("sdec", "01111111"), "127");
        let big = "1".repeat(80);
        assert_eq!(tr("udec", &big), "1208925819614629174706175");
        assert_eq!(tr("udec", "xx01"), "UNDEF");
        assert_eq!(tr("sdec", "zz01"), "HIGHIMP");
    }

    #[test]
    fn floats() {
        assert_eq!(tr("float", &format!("{:032b}", 1.5f32.to_bits())), "1.5");
        assert_eq!(
            tr("float", &format!("{:064b}", (-0.25f64).to_bits())),
            "-0.25"
        );
        assert_eq!(tr("float", &format!("{:016b}", 0x3c00u16)), "1.0");
        assert_eq!(
            tr("float", &format!("{:032b}", f32::INFINITY.to_bits())),
            "+Inf"
        );
        let t = Translators::builtin();
        assert!(
            t.get("float")
                .unwrap()
                .applies(SignalShape::Vector { width: 32 })
        );
        assert!(
            !t.get("float")
                .unwrap()
                .applies(SignalShape::Vector { width: 8 })
        );
    }

    #[test]
    fn defaults() {
        let t = Translators::builtin();
        assert_eq!(t.default_for(SignalShape::Bit).id(), "bit");
        assert_eq!(t.default_for(SignalShape::Vector { width: 8 }).id(), "hex");
        assert_eq!(t.applicable(SignalShape::Bit).len(), 4);
    }
}
