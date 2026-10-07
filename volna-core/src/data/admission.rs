//! Checked storage estimates for derived analysis, including construction scratch.

use anyhow::{Context, Result};
use volna_trace::remote::memory::Reservation;

pub(crate) fn bytes<T>(count: usize) -> Result<u64> {
    (count as u64)
        .checked_mul(std::mem::size_of::<T>() as u64)
        .context("analysis storage size overflow")
}

/// Every level is allocated at its final length; the outer vector is given
/// exactly `levels` slots so its storage is covered by the same estimate.
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
