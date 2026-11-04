//! Bounded signal labels preserve numeric scale and radix.

use super::{Translated, Translator};
use crate::data::text::{PREVIEW_BYTES, Radix};
use volna_trace::data::ValueKind;
use volna_trace::data::value_view::{LogicView, ValueView};

pub(super) fn radix<T: Translator + ?Sized>(
    translator: &T,
    value: &ValueView<'_>,
    width: usize,
    radix: Radix,
) -> Translated {
    let (group, prefix) = match radix {
        Radix::Hex => (4, "0x"),
        Radix::Bin => (1, "0b"),
        Radix::Dec => return decimal(value, width, false),
    };
    let ValueView::Logic(bits) = value else {
        return translator
            .translate_view(value, width.saturating_mul(4))
            .fit_text(width);
    };
    if bits.width.div_ceil(group) <= width {
        return translator.translate_view(value, width);
    }
    let kind = bits.kind();
    let Some(room) = width.checked_sub(prefix.len() + 1).filter(|&n| n >= 2) else {
        return overflow(None, kind, width);
    };
    let head = room.div_ceil(2) * group - (group - bits.width % group) % group;
    let tail = room / 2 * group;
    let part = |start, end| {
        let data: Vec<_> = (start..end).map(|i| bits.bit(i)).collect();
        translator
            .translate_view(&ValueView::Logic(LogicView::ascii(data)), width)
            .text
    };
    overflow(
        Some(format!(
            "{prefix}{}…{}",
            part(0, head),
            part(bits.width - tail, bits.width)
        )),
        kind,
        width,
    )
}

pub(super) fn decimal(value: &ValueView<'_>, width: usize, signed: bool) -> Translated {
    let ValueView::Logic(bits) = value else {
        return super::not_applicable(value, width.saturating_mul(4)).fit_text(width);
    };
    let translated = super::decimal_digits(bits, signed, PREVIEW_BYTES.max(width));
    if translated.kind != ValueKind::Normal {
        return translated.fit_text(width);
    }
    if !translated.truncated && translated.text.chars().count() <= width {
        return translated;
    }
    let compact = scientific(&translated.text, width);
    overflow(compact, translated.kind, width)
}

pub(super) fn real<T: Translator + ?Sized>(
    translator: &T,
    value: &ValueView<'_>,
    width: usize,
) -> Translated {
    let translated = translator.translate_view(value, PREVIEW_BYTES);
    if translated.kind != ValueKind::Normal {
        return translated.fit_text(width);
    }
    if translated.text.chars().count() <= width {
        return translated;
    }
    let compact = translator
        .numeric(value)
        .and_then(|v| scientific(&format!("{v:.16e}"), width));
    overflow(compact, translated.kind, width)
}

fn overflow(label: Option<String>, kind: ValueKind, width: usize) -> Translated {
    Translated {
        text: if width == 0 {
            String::new()
        } else {
            label.unwrap_or_else(|| "…".into())
        },
        kind,
        truncated: true,
    }
}

fn scientific(text: &str, width: usize) -> Option<String> {
    let (sign, magnitude) = text.strip_prefix('-').map_or(("", text), |s| ("-", s));
    let (mantissa, exponent) = match magnitude.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
        None => (magnitude, 0),
    };
    if mantissa.is_empty()
        || !mantissa.bytes().all(|c| c.is_ascii_digit() || c == b'.')
        || mantissa.bytes().filter(|&c| c == b'.').count() > 1
    {
        return None;
    }
    let decimal = mantissa.find('.').unwrap_or(mantissa.len());
    let digits: Vec<_> = mantissa.bytes().filter(|&c| c != b'.').collect();
    let first = digits.iter().position(|&c| c != b'0');
    let Some(first) = first else {
        let zero = format!("{sign}0");
        return (zero.len() <= width).then_some(zero);
    };
    let exponent = exponent + decimal as i32 - first as i32 - 1;
    let digits = &digits[first..];
    let significant = digits.iter().rposition(|&c| c != b'0').unwrap() + 1;

    // Preserve exact integer digits whenever bounded translation provides them.
    for precision in (1..=significant.min(width)).rev() {
        let rounded = precision < significant;
        let mut kept = digits[..precision].to_vec();
        let mut exponent = exponent;
        if precision < significant && digits[precision] >= b'5' {
            let mut carry = true;
            for digit in kept.iter_mut().rev() {
                if *digit == b'9' {
                    *digit = b'0';
                } else {
                    *digit += 1;
                    carry = false;
                    break;
                }
            }
            if carry {
                kept[0] = b'1';
                exponent += 1;
            }
        }
        while kept.len() > 1 && kept.last() == Some(&b'0') {
            kept.pop();
        }
        let mut label = String::new();
        if rounded {
            label.push('≈');
        }
        label.push_str(sign);
        label.push(char::from(kept[0]));
        if kept.len() > 1 {
            label.push('.');
            label.extend(kept[1..].iter().map(|&c| char::from(c)));
        }
        label.push('e');
        label.push_str(&exponent.to_string());
        if label.chars().count() <= width {
            return Some(label);
        }
    }
    None
}
