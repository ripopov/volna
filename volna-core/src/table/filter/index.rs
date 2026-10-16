//! Rank/select bitmap: one bit per source row and one prefix count per 512 rows.

use anyhow::{Context, Result, ensure};
use std::sync::atomic::{AtomicBool, Ordering};

pub(in crate::table) fn required_bytes(len: u64) -> Option<u64> {
    len.div_ceil(64)
        .checked_mul(8)?
        .checked_add(len.div_ceil(512).checked_mul(8)?)
}

pub(in crate::table) struct Matches {
    words: Vec<u64>,
    prefixes: Vec<u64>,
    count: u64,
}
impl Matches {
    pub fn build(
        len: u64,
        mut matches: impl FnMut(u64) -> bool,
        stop: &AtomicBool,
    ) -> Result<Self> {
        let mut words = Vec::new();
        let mut prefixes = Vec::new();
        words
            .try_reserve_exact(usize::try_from(len.div_ceil(64))?)
            .context("Cannot allocate filter bitmap")?;
        prefixes
            .try_reserve_exact(usize::try_from(len.div_ceil(512))?)
            .context("Cannot allocate filter ranks")?;
        let mut count = 0;
        for first in (0..len).step_by(64) {
            ensure!(!stop.load(Ordering::Relaxed), "Filter cancelled");
            if first.is_multiple_of(512) {
                prefixes.push(count);
            }
            let mut word = 0u64;
            for row in first..first.saturating_add(64).min(len) {
                if matches(row) {
                    word |= 1 << (row - first);
                }
            }
            count += u64::from(word.count_ones());
            words.push(word);
        }
        Ok(Self {
            words,
            prefixes,
            count,
        })
    }
    pub fn len(&self) -> u64 {
        self.count
    }
    pub fn source_row(&self, rank: u64) -> Option<u64> {
        if rank >= self.count {
            return None;
        }
        let block = self
            .prefixes
            .partition_point(|&n| n <= rank)
            .checked_sub(1)?;
        let mut local = rank - self.prefixes[block];
        for (i, mut word) in self
            .words
            .iter()
            .copied()
            .enumerate()
            .skip(block * 8)
            .take(8)
        {
            let count = u64::from(word.count_ones());
            if local < count {
                for _ in 0..local {
                    word &= word - 1;
                }
                return Some(i as u64 * 64 + u64::from(word.trailing_zeros()));
            }
            local -= count;
        }
        None
    }
    pub fn rank(&self, row: u64) -> Option<u64> {
        let index = usize::try_from(row / 64).ok()?;
        let word = *self.words.get(index)?;
        let bit = row % 64;
        if word & (1 << bit) == 0 {
            return None;
        }
        let block = index / 8;
        Some(
            self.prefixes[block]
                + self.words[block * 8..index]
                    .iter()
                    .map(|w| u64::from(w.count_ones()))
                    .sum::<u64>()
                + u64::from((word & ((1 << bit) - 1)).count_ones()),
        )
    }
    pub fn iter(&self) -> impl Iterator<Item = u64> + '_ {
        self.words.iter().enumerate().flat_map(|(i, &word)| {
            let mut word = word;
            std::iter::from_fn(move || {
                if word == 0 {
                    return None;
                }
                let row = i as u64 * 64 + u64::from(word.trailing_zeros());
                word &= word - 1;
                Some(row)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranks_and_selection_skip_empty_blocks_and_partial_words() {
        let stop = AtomicBool::new(false);
        for len in [0, 1, 63, 64, 65, 511, 512, 513, 12347] {
            let keep = |r| r % 997 < 13;
            let index = Matches::build(len, keep, &stop).unwrap();
            let expected: Vec<_> = (0..len).filter(|&r| keep(r)).collect();
            assert_eq!(index.iter().collect::<Vec<_>>(), expected);
            assert_eq!(index.len(), expected.len() as u64);
            for (rank, &row) in expected.iter().enumerate() {
                assert_eq!(index.source_row(rank as u64), Some(row));
                assert_eq!(index.rank(row), Some(rank as u64));
            }
            assert_eq!(index.source_row(index.len()), None);
            assert_eq!(index.rank(len + 64), None);
        }
        stop.store(true, Ordering::Relaxed);
        assert!(Matches::build(10, |_| true, &stop).is_err());
    }

    #[test]
    #[ignore = "10M-row scale measurement"]
    fn ten_million_rows() {
        let len = 10_000_000;
        let start = std::time::Instant::now();
        let index =
            Matches::build(len, |r| r % 3 == 0 && r > 100, &AtomicBool::new(false)).unwrap();
        assert_eq!(index.len(), 3_333_300);
        assert_eq!(index.source_row(0), Some(102));
        assert_eq!(index.source_row(index.len() - 1), Some(9_999_999));
        for row in (102..len).step_by(997 * 3) {
            assert_eq!(index.source_row(index.rank(row).unwrap()), Some(row));
        }
        eprintln!(
            "10M filter: {:?}, {} index bytes",
            start.elapsed(),
            required_bytes(len).unwrap()
        );
    }
}
