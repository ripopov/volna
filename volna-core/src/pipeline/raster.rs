//! Exact mapping from source ticks and rows to clipped physical pixels.

use super::rows::RowView;
use crate::{geometry::Rect, scene::PresenceMask, wave::viewport::Viewport};
use num_rational::BigRational;
use num_traits::{FromPrimitive, ToPrimitive};
use std::{cmp::Ordering, ops::Range};
#[cfg(test)]
use volna_trace::data::transactions::Transaction;

/// Subtract the integer origin before conversion, retaining tick differences
/// above 2^53. Only endpoints close to a pixel or clip edge take the exact
/// dyadic-rational path; ordinary interval queries use floating arithmetic.
#[derive(Debug)]
pub(super) struct Axis {
    anchor: i128,
    offset: f64,
    span: f64,
    extent: f64,
    clip: f64,
    phase: f64,
    pixels: usize,
    linear: Option<(i128, i128, i128)>,
}
impl Axis {
    fn new(start: f64, span: f64, extent: f64, phase: f64) -> Self {
        let anchor = start.floor() as i128;
        let slope =
            BigRational::from_float(extent).unwrap() / BigRational::from_float(span).unwrap();
        let intercept = BigRational::from_float(phase).unwrap()
            - BigRational::from_float(start - anchor as f64).unwrap() * &slope;
        let linear = (|| {
            let a = slope.denom().to_i128()?;
            let b = intercept.denom().to_i128()?;
            let mut x = a;
            let mut y = b;
            while y != 0 {
                (x, y) = (y, x % y);
            }
            let denominator = (a / x).checked_mul(b)?;
            Some((
                slope.numer().to_i128()?.checked_mul(denominator / a)?,
                intercept.numer().to_i128()?.checked_mul(denominator / b)?,
                denominator,
            ))
        })();
        Self {
            anchor,
            offset: start - anchor as f64,
            span,
            extent,
            clip: extent,
            phase,
            pixels: pixel_extent(phase, extent),
            linear,
        }
    }
    fn at(&self, value: u64) -> f64 {
        self.phase
            + ((i128::from(value) - self.anchor) as f64 - self.offset) * self.extent / self.span
    }
    fn exact(&self, value: u64) -> BigRational {
        let rational = |v| BigRational::from_float(v).expect("finite pixel transform");
        rational(self.phase)
            + (BigRational::from_i128(i128::from(value) - self.anchor).unwrap()
                - rational(self.offset))
                * rational(self.extent)
                / rational(self.span)
    }
    fn uncertainty(&self, value: u64) -> f64 {
        16.0 * f64::EPSILON * (self.at(value).abs() + self.phase.abs() + self.extent.abs() + 1.0)
    }
    fn compare(&self, value: u64, right: bool) -> Ordering {
        let pixel = self.phase + if right { self.clip } else { 0.0 };
        let at = self.at(value);
        if (at - pixel).abs() <= self.uncertainty(value) {
            let edge = BigRational::from_float(self.phase).unwrap()
                + BigRational::from_float(if right { self.clip } else { 0.0 }).unwrap();
            self.exact(value).cmp(&edge)
        } else {
            at.total_cmp(&pixel)
        }
    }
    fn round(&self, value: u64, ceil: bool) -> i64 {
        let at = self.at(value);
        if at <= -1.0 {
            return -1;
        }
        if at >= self.pixels as f64 + 1.0 {
            return self.pixels as i64 + 1;
        }
        if (at - at.round()).abs() <= self.uncertainty(value) {
            if let Some((slope, intercept, denominator)) = self.linear
                && let Some(numerator) = (i128::from(value) - self.anchor)
                    .checked_mul(slope)
                    .and_then(|n| n.checked_add(intercept))
            {
                let floor = numerator.div_euclid(denominator);
                let rounded = floor + i128::from(ceil && numerator.rem_euclid(denominator) != 0);
                if let Ok(pixel) = i64::try_from(rounded) {
                    return pixel;
                }
            }
            let exact = self.exact(value);
            (if ceil { exact.ceil() } else { exact.floor() })
                .to_integer()
                .to_i64()
                .unwrap_or(if at < 0.0 { i64::MIN } else { i64::MAX })
        } else {
            (if ceil { at.ceil() } else { at.floor() }) as i64
        }
    }
    pub(super) fn floor(&self, value: u64) -> i64 {
        self.round(value, false)
    }
    pub(super) fn contains(&self, value: u64) -> bool {
        self.compare(value, false) != Ordering::Less && self.compare(value, true) == Ordering::Less
    }
    pub(super) fn inside(&self, begin: u64, end: u64, closed: bool) -> bool {
        self.compare(begin, false) != Ordering::Less
            && match self.compare(end, true) {
                Ordering::Less => true,
                Ordering::Equal => !closed,
                Ordering::Greater => false,
            }
    }
    pub(super) fn interval(&self, begin: u64, end: u64) -> Option<Range<usize>> {
        if begin == end {
            if !self.contains(begin) {
                return None;
            }
            let x = self.floor(begin).max(0) as usize;
            return Some(x..(x + 1).min(self.pixels));
        }
        if self.compare(end, false) != Ordering::Greater
            || self.compare(begin, true) != Ordering::Less
        {
            return None;
        }
        let a = self.floor(begin).max(self.phase.floor() as i64).max(0) as usize;
        let b = self.round(end, true).min(self.pixels as i64).max(0) as usize;
        (a < b).then_some(a..b)
    }
}

fn pixel_extent(phase: f64, extent: f64) -> usize {
    (BigRational::from_float(phase).unwrap() + BigRational::from_float(extent).unwrap())
        .ceil()
        .to_integer()
        .to_usize()
        .expect("representable physical extent")
}

pub(crate) struct PixelMap {
    pub(super) x: Axis,
    pub(super) y: Axis,
    pub rect: Rect,
}
impl PixelMap {
    pub(crate) fn new(cells: Rect, viewport: Viewport, rows: RowView, scale: f64) -> Self {
        let left = f64::from(cells.left()) * scale;
        let top = f64::from(cells.top()) * scale;
        let width = f64::from(cells.width()) * scale;
        let height = f64::from(cells.height()) * scale;
        let x = Axis::new(viewport.start, viewport.width(), width, left - left.floor());
        let y = Axis::new(
            rows.top,
            1.0,
            f64::from(rows.row_px) * scale,
            top - top.floor(),
        );
        let y = Axis {
            pixels: pixel_extent(top - top.floor(), height),
            clip: height,
            ..y
        };
        let rect = Rect::from_xywh(
            (left.floor() / scale) as f32,
            (top.floor() / scale) as f32,
            (x.pixels as f64 / scale) as f32,
            (y.pixels as f64 / scale) as f32,
        );
        Self { x, y, rect }
    }
    pub(crate) fn mask(&self) -> PresenceMask {
        PresenceMask::new(self.x.pixels, self.y.pixels)
    }
    pub(crate) fn aborted_mask(&self) -> PresenceMask {
        PresenceMask::new(1, self.y.pixels)
    }
    pub(crate) fn aborted(&self, row: usize, mask: &mut PresenceMask) {
        if let Some(y) = self.y.interval(row as u64, (row + 1) as u64) {
            mask.fill(0..1, y);
        }
    }
}

#[cfg(test)]
fn rat(v: f64) -> BigRational {
    BigRational::from_float(v).unwrap()
}
#[cfg(test)]
fn int(v: u64) -> BigRational {
    BigRational::from_integer(v.into())
}

/// Inverse pixel footprints in source coordinates. This deliberately
/// avoids the forward mapper, union normalization and hierarchy traversal.
#[cfg(test)]
pub(super) fn oracle(records: &[Transaction], offset: usize, map: &PixelMap) -> PresenceMask {
    let footprints = |axis: &Axis| {
        (0..axis.pixels)
            .map(|p| {
                let left = rat(p as f64).max(rat(axis.phase));
                let right = rat((p + 1) as f64).min(rat(axis.phase) + rat(axis.clip));
                if left >= right {
                    return None;
                }
                let inverse = |p| {
                    (p - rat(axis.phase)) * rat(axis.span) / rat(axis.extent)
                        + BigRational::from_integer(axis.anchor.into())
                        + rat(axis.offset)
                };
                Some((inverse(left), inverse(right)))
            })
            .collect::<Vec<_>>()
    };
    let xs = footprints(&map.x);
    let ys = footprints(&map.y);
    let mut out = map.mask();
    for (y, footprint) in ys.iter().enumerate() {
        let Some((top, bottom)) = footprint else {
            continue;
        };
        for (row, tx) in records.iter().enumerate() {
            if int((offset + row) as u64) >= *bottom || int((offset + row + 1) as u64) <= *top {
                continue;
            }
            let spans = if tx.stages.is_empty() {
                vec![(tx.begin, tx.end)]
            } else {
                tx.stages
                    .iter()
                    .map(|s| (s.begin, s.end.unwrap_or(tx.end).max(s.begin)))
                    .collect()
            };
            for (x, footprint) in xs.iter().enumerate() {
                let Some((left, right)) = footprint else {
                    continue;
                };
                if spans.iter().any(|&(a, b)| {
                    if a == b {
                        int(a) >= *left && int(a) < *right
                    } else {
                        int(a) < *right && int(b) > *left
                    }
                }) {
                    out.fill(x..x + 1, y..y + 1);
                }
            }
        }
    }
    out
}
