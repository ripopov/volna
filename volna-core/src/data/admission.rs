//! Checked storage estimates for derived analysis, including construction scratch.

use anyhow::{Context, Result};
use volna_trace::remote::memory::Reservation;

pub(crate) fn bytes<T>(count: usize) -> Result<u64> {
    (count as u64)
        .checked_mul(std::mem::size_of::<T>() as u64)
        .context("analysis storage size overflow")
}

pub(crate) fn pyramid(
    mut blocks: usize,
    fanout: usize,
    stop: usize,
    header: usize,
    level_bytes: impl Fn(usize) -> Result<u64>,
) -> Result<(u64, usize)> {
    let (mut total, mut levels) = (0u64, 0usize);
    loop {
        total = total
            .checked_add(level_bytes(blocks)?)
            .and_then(|n| n.checked_add(header as u64))
            .context("analysis storage size overflow")?;
        levels += 1;
        if blocks <= stop {
            break;
        }
        blocks = blocks.div_ceil(fanout);
    }
    Ok((total, levels))
}

pub(crate) fn sum<const N: usize>(parts: [u64; N]) -> Result<u64> {
    parts.into_iter().try_fold(0u64, |total, bytes| {
        total
            .checked_add(bytes)
            .context("analysis storage size overflow")
    })
}

pub(crate) fn finish(mut held: Reservation, resident: u64) -> Result<Reservation> {
    held.shrink(
        held.bytes()
            .checked_sub(resident)
            .context("analysis exceeded admitted storage")?,
    )?;
    Ok(held)
}

pub(crate) struct Storage {
    what: &'static str,
    held: Option<Reservation>,
}

impl Storage {
    pub(crate) fn new(
        what: &'static str,
        header: usize,
        budget: Option<&volna_trace::remote::memory::MemoryBudget>,
    ) -> Result<Self> {
        Ok(Self {
            what,
            held: budget
                .map(|b| b.reserve_object(what, header as u64))
                .transpose()?,
        })
    }

    fn grow(&mut self, bytes: u64) -> Result<()> {
        if let Some(held) = &mut self.held {
            held.grow_object(self.what, bytes)?;
        }
        Ok(())
    }

    pub(crate) fn vec<T>(&mut self, count: usize) -> Result<Vec<T>> {
        self.grow(bytes::<T>(count)?)?;
        let mut out = Vec::new();
        out.try_reserve_exact(count)?;
        Ok(out)
    }

    pub(crate) fn push<T>(&mut self, out: &mut Vec<T>, value: T) -> Result<()> {
        if out.len() == out.capacity() {
            let capacity = out
                .capacity()
                .max(1)
                .checked_mul(2)
                .context("analysis storage size overflow")?;
            self.grow(bytes::<T>(capacity - out.capacity())?)?;
            out.try_reserve_exact(capacity - out.len())?;
        }
        out.push(value);
        Ok(())
    }

    pub(crate) fn release<T>(&mut self, scratch: Vec<T>) -> Result<()> {
        let bytes = bytes::<T>(scratch.capacity())?;
        drop(scratch);
        if let Some(held) = &mut self.held {
            held.shrink(bytes)?;
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Option<Reservation> {
        self.held
    }
}
