//! Duration-weighted trailing means on an explicit, viewport-independent output grid.

use crate::data::{
    NumericKind,
    numeric_input::{InputHistory, Interpretation, Sample, sample},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use volna_trace::data::{Bit, SignalHistory, SignalShape, WaveValue};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Settings {
    pub window: u64,
    pub interval: u64,
    pub origin: u64,
    pub end: u64,
    pub partial: bool,
    pub skip_unknown: bool,
}

impl Settings {
    pub fn sample_count(self) -> Result<usize> {
        ensure!(
            self.window > 0,
            "Window must be at least one timeline tick."
        );
        ensure!(
            self.interval > 0,
            "Output interval must be at least one timeline tick."
        );
        ensure!(self.end >= self.origin, "Output time range is reversed.");
        let count = (self.end - self.origin) / self.interval;
        let count = count.checked_add(1).context("Too many output samples.")?;
        ensure!(
            count <= 1_000_000,
            "More than 1,000,000 output samples; increase the output interval."
        );
        Ok(usize::try_from(count)?)
    }
}

impl crate::trace::Retime for Settings {
    fn retime(&mut self, by: crate::trace::Rescale) {
        self.window = by.time(self.window).max(1);
        self.interval = by.time(self.interval).max(1);
        self.origin = by.time(self.origin);
        self.end = by.time(self.end);
    }
}

pub fn duration(text: &str, base: crate::wave::timeline::TimeBase<'_>) -> Result<u64> {
    let text = text.trim();
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(end);
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    ensure!(
        !whole.is_empty() && fraction.len() <= 15 && !fraction.contains('.'),
        "Enter a positive duration, for example 4 us or 250 ns."
    );
    let scale = 10_u128.pow(fraction.len() as u32);
    let numerator = whole
        .parse::<u128>()?
        .checked_mul(scale)
        .and_then(|n| {
            n.checked_add(if fraction.is_empty() {
                0
            } else {
                fraction.parse().ok()?
            })
        })
        .context("Duration is too large.")?;
    let unit = unit.trim();
    let exponent = if unit.is_empty() || unit == "ticks" || base.unit == Some(unit) {
        0
    } else {
        ensure!(
            base.unit.is_none(),
            "Use timeline ticks or the recording's time unit."
        );
        let exp = match unit {
            "fs" => -15,
            "ps" => -12,
            "ns" => -9,
            "us" | "µs" | "μs" => -6,
            "ms" => -3,
            "s" => 0,
            _ => anyhow::bail!("Unknown time unit; use fs, ps, ns, us, ms, s, or ticks."),
        };
        exp - i32::from(base.timescale)
    };
    let factor = 10_u128
        .checked_pow(exponent.unsigned_abs())
        .context("Duration is too large.")?;
    let (numerator, denominator) = if exponent >= 0 {
        (
            numerator
                .checked_mul(factor)
                .context("Duration is too large.")?,
            scale,
        )
    } else {
        (
            numerator,
            scale
                .checked_mul(factor)
                .context("Duration is too large.")?,
        )
    };
    ensure!(
        numerator > 0 && numerator % denominator == 0,
        "Duration must be a positive whole number of timeline ticks."
    );
    u64::try_from(numerator / denominator).context("Duration is too large.")
}

pub struct MeanHistory {
    settings: Settings,
    values: Vec<Option<f64>>,
    reservation: Reservation,
}

impl MeanHistory {
    pub fn build(
        source: &dyn SignalHistory,
        interpretation: Interpretation,
        settings: Settings,
        budget: &MemoryBudget,
        cancelled: &AtomicBool,
    ) -> Result<Self> {
        interpretation.validate(source.shape())?;
        let count = settings.sample_count()?;
        let resident = crate::data::admission::sum([
            std::mem::size_of::<Self>() as u64,
            crate::data::admission::bytes::<Option<f64>>(count)?,
        ])?;
        let bytes = crate::data::admission::sum([
            resident,
            interpretation
                .projection
                .map_or(0, |p| u64::from(p.width())),
        ])?;
        let reservation = budget.reserve_object(
            "moving-average output; increase the interval or memory budget",
            bytes,
        )?;
        ensure!(!cancelled.load(Ordering::Relaxed), "Computation cancelled.");
        let mut values = Vec::new();
        values.try_reserve_exact(count)?;
        let input = InputHistory {
            source,
            projection: interpretation.projection,
        };
        let source: &dyn SignalHistory = &input;
        let mut right = Cursor::new(source, settings.origin, interpretation.kind);
        let mut left = Cursor::new(source, settings.origin, interpretation.kind);
        let mut area = 0.0;
        let mut compensation = 0.0;
        let mut unknown = 0_i128;
        for i in 0..count {
            ensure!(!cancelled.load(Ordering::Relaxed), "Computation cancelled.");
            let time = settings.origin + i as u64 * settings.interval;
            let start = time.saturating_sub(settings.window).max(settings.origin);
            if start >= right.time {
                right = Cursor::new(source, start, interpretation.kind);
                left = Cursor::new(source, start, interpretation.kind);
                area = 0.0;
                compensation = 0.0;
                unknown = 0;
            }
            for (cursor, target, sign) in [(&mut left, start, -1.0), (&mut right, time, 1.0)] {
                cursor.advance(target, cancelled, |duration, value| {
                    if let Some(value) = value {
                        let term = sign * value * (duration as f64 / settings.window as f64);
                        let delta = term - compensation;
                        let next = area + delta;
                        compensation = (next - area) - delta;
                        area = next;
                    } else {
                        unknown += if sign > 0.0 {
                            i128::from(duration)
                        } else {
                            -i128::from(duration)
                        };
                    }
                })?;
            }
            ensure!(
                area.is_finite() && compensation.is_finite(),
                "Moving-average accumulation exceeded the real-value range."
            );
            let width = time - start;
            let known = i128::from(width) - unknown;
            let value = if width == 0
                || (!settings.partial && time - settings.origin < settings.window)
                || (!settings.skip_unknown && unknown > 0)
                || known == 0
            {
                None
            } else {
                let mean = area / (known as f64 / settings.window as f64);
                mean.is_finite().then_some(mean)
            };
            values.push(value);
        }
        Ok(Self {
            settings,
            values,
            reservation: crate::data::admission::finish(reservation, resident)?,
        })
    }
}

struct Cursor<'a> {
    source: &'a dyn SignalHistory,
    kind: NumericKind,
    index: Option<usize>,
    time: u64,
}

impl<'a> Cursor<'a> {
    fn new(source: &'a dyn SignalHistory, time: u64, kind: NumericKind) -> Self {
        Self {
            source,
            kind,
            index: source.index_at(time),
            time,
        }
    }

    fn advance(
        &mut self,
        target: u64,
        cancelled: &AtomicBool,
        mut add: impl FnMut(u64, Option<f64>),
    ) -> Result<()> {
        while self.time < target {
            ensure!(!cancelled.load(Ordering::Relaxed), "Computation cancelled.");
            let next = self.index.map_or(0, |i| i + 1);
            let end = if next < self.source.len() {
                self.source.time(next).min(target)
            } else {
                target
            };
            ensure!(end >= self.time, "Source history is not ordered.");
            let value = match sample(self.source, self.kind, self.index) {
                Sample::Value(v) => Some(v),
                _ => None,
            };
            add(end - self.time, value);
            self.time = end;
            if next < self.source.len() && self.source.time(next) <= target {
                self.index = Some(next);
            }
        }
        Ok(())
    }
}

impl SignalHistory for MeanHistory {
    fn resident_bytes(&self) -> u64 {
        self.reservation.bytes()
    }
    fn shape(&self) -> SignalShape {
        SignalShape::Real
    }
    fn len(&self) -> usize {
        self.values.len()
    }
    fn time(&self, i: usize) -> u64 {
        self.settings.origin + i as u64 * self.settings.interval
    }
    fn value(&self, i: Option<usize>) -> WaveValue {
        i.and_then(|i| self.values.get(i).copied().flatten())
            .map_or(WaveValue::Unavailable, WaveValue::Real)
    }
    fn bit(&self, _: Option<usize>) -> Bit {
        Bit::Other
    }
}
