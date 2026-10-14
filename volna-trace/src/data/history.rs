//! The change history of one signal.

use super::value::{Bit, SignalShape, WaveValue};

pub trait SignalHistory: Send + Sync {
    fn resident_bytes(&self) -> u64 {
        0
    }

    fn shape(&self) -> SignalShape;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn time(&self, i: usize) -> u64;

    fn value(&self, i: Option<usize>) -> WaveValue;

    fn value_view(&self, i: Option<usize>) -> super::value_view::ValueView<'_> {
        super::value_view::ValueView::owned(self.value(i))
    }

    fn always_normal(&self) -> bool {
        false
    }

    fn bit(&self, i: Option<usize>) -> Bit;

    fn index_at(&self, t: u64) -> Option<usize> {
        let n = self.len();
        if n == 0 || self.time(0) > t {
            return None;
        }
        Some(bisect(self, 0, n, t))
    }

    fn index_at_hint(&self, t: u64, hint: usize) -> Option<usize> {
        let n = self.len();
        if n == 0 {
            return None;
        }
        let hint = hint.min(n - 1);
        if self.time(hint) <= t {
            let mut lo = hint;
            let mut step = 1usize;
            let mut hi = hint + 1;
            while hi < n && self.time(hi) <= t {
                lo = hi;
                step <<= 1;
                hi = hint.saturating_add(step).min(n);
                if hi == n {
                    break;
                }
            }
            Some(bisect(self, lo, hi.min(n), t))
        } else {
            if self.time(0) > t {
                return None;
            }
            let mut hi = hint;
            let mut step = 1usize;
            let mut lo = hint.saturating_sub(step);
            while lo > 0 && self.time(lo) > t {
                hi = lo;
                step <<= 1;
                lo = hint.saturating_sub(step);
            }
            Some(bisect(self, lo, hi, t))
        }
    }

    fn next_change_after(&self, t: u64) -> Option<u64> {
        let n = self.len();
        let start = match self.index_at(t) {
            None => 0,
            Some(i) => i + 1,
        };
        (start..n).map(|i| self.time(i)).find(|&ti| ti > t)
    }

    fn prev_change_before(&self, t: u64) -> Option<u64> {
        let i = self.index_at(t.checked_sub(1)?)?;
        Some(self.time(i))
    }
}

fn bisect<H: SignalHistory + ?Sized>(h: &H, mut lo: usize, mut hi: usize, t: u64) -> usize {
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if h.time(mid) <= t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

pub struct VecHistory {
    pub shape: SignalShape,
    pub times: Vec<u64>,
    pub values: Vec<WaveValue>,
    pub initial: WaveValue,
}

impl SignalHistory for VecHistory {
    fn resident_bytes(&self) -> u64 {
        let values = self.values.iter().fold(0u64, |bytes, value| {
            bytes.saturating_add(match value {
                WaveValue::Bits(text) | WaveValue::Text(text) => text.capacity() as u64,
                WaveValue::Bytes(value) => value.capacity() as u64,
                WaveValue::Unavailable | WaveValue::Real(_) => 0,
            })
        });
        (self.times.capacity() as u64)
            .saturating_mul(std::mem::size_of::<u64>() as u64)
            .saturating_add(
                (self.values.capacity() as u64)
                    .saturating_mul(std::mem::size_of::<WaveValue>() as u64),
            )
            .saturating_add(values)
    }

    fn shape(&self) -> SignalShape {
        self.shape
    }
    fn len(&self) -> usize {
        self.times.len()
    }
    fn time(&self, i: usize) -> u64 {
        self.times[i]
    }
    fn value(&self, i: Option<usize>) -> WaveValue {
        match i {
            None => self.initial.clone(),
            Some(i) => self.values[i].clone(),
        }
    }
    fn value_view(&self, i: Option<usize>) -> super::value_view::ValueView<'_> {
        super::value_view::ValueView::borrowed(match i {
            None => &self.initial,
            Some(i) => &self.values[i],
        })
    }
    fn bit(&self, i: Option<usize>) -> Bit {
        let value = match i {
            None => &self.initial,
            Some(i) => &self.values[i],
        };
        match value {
            WaveValue::Unavailable => Bit::Unavailable,
            WaveValue::Bits(s) => s.bytes().next().map_or(Bit::Other, Bit::from_ascii),
            _ => Bit::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist(times: &[u64]) -> VecHistory {
        VecHistory {
            shape: SignalShape::Bit,
            times: times.to_vec(),
            values: times.iter().map(|_| WaveValue::Bits("1".into())).collect(),
            initial: WaveValue::Bits("0".into()),
        }
    }

    #[test]
    fn index_at_matches_linear_scan() {
        let h = hist(&[5, 5, 10, 20, 20, 20, 31]);
        for t in 0..40 {
            let expect = (0..h.len()).rev().find(|&i| h.time(i) <= t);
            assert_eq!(h.index_at(t), expect, "t={t}");
            for hint in 0..h.len() {
                assert_eq!(h.index_at_hint(t, hint), expect, "t={t} hint={hint}");
            }
        }
        assert_eq!(hist(&[]).index_at(3), None);
        assert_eq!(hist(&[]).index_at_hint(3, 0), None);
    }

    #[test]
    fn neighbours() {
        let h = hist(&[5, 10, 20]);
        assert_eq!(h.next_change_after(5), Some(10));
        assert_eq!(h.next_change_after(20), None);
        assert_eq!(h.prev_change_before(10), Some(5));
        assert_eq!(h.prev_change_before(5), None);
        assert_eq!(h.prev_change_before(7), Some(5));
        let h = hist(&[5, 5, 10, 20, 20, 20, 31]);
        for t in 0..40 {
            let expect = (0..h.len()).rev().map(|i| h.time(i)).find(|&ti| ti < t);
            assert_eq!(h.prev_change_before(t), expect, "t = {t}");
        }
    }
}
