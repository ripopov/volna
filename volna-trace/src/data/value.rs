//! Signal value representation shared by histories, translators and the renderer.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Bit {
    Unavailable,
    Zero,
    One,
    X,
    Z,
    DontCare,
    Other,
}

impl Bit {
    pub fn from_ascii(c: u8) -> Bit {
        match c {
            b'0' => Bit::Zero,
            b'1' => Bit::One,
            b'x' | b'X' | b'u' | b'U' | b'w' | b'W' => Bit::X,
            b'z' | b'Z' => Bit::Z,
            b'-' => Bit::DontCare,
            _ => Bit::Other,
        }
    }

    pub fn kind(self) -> ValueKind {
        match self {
            Bit::Zero | Bit::One => ValueKind::Normal,
            Bit::X | Bit::Unavailable => ValueKind::Undef,
            Bit::Z => ValueKind::HighImp,
            Bit::DontCare => ValueKind::DontCare,
            Bit::Other => ValueKind::Weak,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ValueKind {
    Normal,
    Undef,
    HighImp,
    DontCare,
    Weak,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, serde::Serialize, serde::Deserialize)]
pub enum SignalShape {
    Event,
    Bit,
    Vector { width: u32 },
    Real,
    Text,
}

impl SignalShape {
    pub fn dims(self) -> String {
        match self {
            Self::Vector { width } => format!("[{}:0]", width - 1),
            Self::Real => "real".into(),
            Self::Text => "str".into(),
            Self::Bit => String::new(),
            Self::Event => "event".into(),
        }
    }

    pub fn width(self) -> u32 {
        match self {
            SignalShape::Bit => 1,
            SignalShape::Vector { width } => width,
            SignalShape::Real => 64,
            SignalShape::Text | SignalShape::Event => 0,
        }
    }

    pub fn is_digital(self) -> bool {
        matches!(self, SignalShape::Bit | SignalShape::Vector { .. })
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum WaveValue {
    Unavailable,
    Bits(String),
    Real(f64),
    Text(String),
    Bytes(Vec<u8>),
}

impl WaveValue {
    pub fn kind(&self) -> ValueKind {
        match self {
            WaveValue::Unavailable => ValueKind::Undef,
            WaveValue::Bits(s) => kind_of_bits(s.as_bytes()),
            WaveValue::Real(_) | WaveValue::Text(_) | WaveValue::Bytes(_) => ValueKind::Normal,
        }
    }
}

pub fn kind_of_bits(bits: &[u8]) -> ValueKind {
    kind_of_codes(bits.iter().copied())
}

pub fn kind_of_codes(bits: impl IntoIterator<Item = u8>) -> ValueKind {
    let mut kind = ValueKind::Normal;
    for c in bits {
        let k = match c {
            b'0' | b'1' => continue,
            b'x' | b'X' | b'u' | b'U' | b'w' | b'W' => return ValueKind::Undef,
            b'z' | b'Z' => ValueKind::HighImp,
            b'-' => ValueKind::DontCare,
            _ => ValueKind::Weak,
        };
        kind = match (kind, k) {
            (ValueKind::HighImp, _) => ValueKind::HighImp,
            (_, ValueKind::HighImp) => ValueKind::HighImp,
            (ValueKind::DontCare, _) => ValueKind::DontCare,
            (_, k) => k,
        };
    }
    kind
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_kinds() {
        assert_eq!(Bit::from_ascii(b'1'), Bit::One);
        assert_eq!(Bit::from_ascii(b'u'), Bit::X);
        assert_eq!(Bit::from_ascii(b'z').kind(), ValueKind::HighImp);
        assert_eq!(Bit::from_ascii(b'-').kind(), ValueKind::DontCare);
        assert_eq!(Bit::from_ascii(b'l').kind(), ValueKind::Weak);
    }

    #[test]
    fn kind_priority() {
        assert_eq!(kind_of_bits(b"0101"), ValueKind::Normal);
        assert_eq!(kind_of_bits(b"0z01"), ValueKind::HighImp);
        assert_eq!(kind_of_bits(b"xz01"), ValueKind::Undef);
        assert_eq!(kind_of_bits(b"zx01"), ValueKind::Undef);
        assert_eq!(kind_of_bits(b"-l01"), ValueKind::DontCare);
        assert_eq!(kind_of_bits(b"hl"), ValueKind::Weak);
    }
}
