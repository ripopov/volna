//! Opaque physical-pixel coverage, independent of image formats and GPU resources.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresenceMask {
    width: usize,
    height: usize,
    words: Vec<u64>,
}

impl PresenceMask {
    pub(crate) fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            words: vec![0; width.div_ceil(64) * height],
        }
    }

    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }
    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    #[must_use]
    pub fn contains(&self, x: usize, y: usize) -> bool {
        x < self.width
            && y < self.height
            && self.words[y * self.width.div_ceil(64) + x / 64] & (1 << (x % 64)) != 0
    }

    pub(crate) fn fill(&mut self, x: Range<usize>, y: Range<usize>) {
        if x.is_empty() || y.is_empty() {
            return;
        }
        let stride = self.width.div_ceil(64);
        let first = x.start / 64;
        let last = (x.end - 1) / 64;
        let left = u64::MAX << (x.start % 64);
        let right = u64::MAX >> (63 - (x.end - 1) % 64);
        for y in y {
            let row = &mut self.words[y * stride..(y + 1) * stride];
            if first == last {
                row[first] |= left & right;
            } else {
                row[first] |= left;
                row[first + 1..last].fill(u64::MAX);
                row[last] |= right;
            }
        }
    }

    pub fn runs(&self) -> impl Iterator<Item = (usize, Range<usize>)> + '_ {
        (0..self.height).flat_map(move |y| {
            let mut x = 0;
            std::iter::from_fn(move || {
                let find = |mut x: usize, occupied: bool| {
                    while x < self.width {
                        let word = self.words[y * self.width.div_ceil(64) + x / 64];
                        let bits = if occupied { word } else { !word };
                        let bits = bits >> (x % 64);
                        if bits != 0 {
                            return (x + bits.trailing_zeros() as usize).min(self.width);
                        }
                        x = (x / 64 + 1) * 64;
                    }
                    self.width
                };
                let start = find(x, true);
                if start == self.width {
                    return None;
                }
                x = find(start, false);
                Some((y, start..x))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_and_occupancy_agree_across_word_and_row_edges() {
        for width in [1, 63, 64, 65, 130] {
            let mut mask = PresenceMask::new(width, 4);
            mask.fill(0..width, 0..1);
            mask.fill(width / 2..width, 1..3);
            for x in (0..width).step_by(2) {
                mask.fill(x..x + 1, 3..4);
            }
            let mut from_runs = vec![false; width * 4];
            for (y, x) in mask.runs() {
                for x in x {
                    from_runs[y * width + x] = true;
                }
            }
            for y in 0..4 {
                for x in 0..width {
                    let expected = y == 0 || (y < 3 && x >= width / 2) || (y == 3 && x % 2 == 0);
                    assert_eq!(mask.contains(x, y), expected);
                    assert_eq!(from_runs[y * width + x], expected);
                }
            }
        }
    }
}
