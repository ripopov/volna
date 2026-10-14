//! Time formatting and tick placement for the timeline header.

use super::viewport::Viewport;
use volna_trace::data::TraceInfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeBase<'a> {
    pub timescale: i8,
    pub unit: Option<&'a str>,
}

impl<'a> TimeBase<'a> {
    #[must_use]
    pub fn of(info: &'a TraceInfo) -> Self {
        Self {
            timescale: info.timescale,
            unit: info.time_unit.as_deref(),
        }
    }

    #[must_use]
    pub fn si(timescale: i8) -> Self {
        Self {
            timescale,
            unit: None,
        }
    }
}

const TICK_STEPS: [f64; 4] = [1.0, 2.0, 2.5, 5.0];

const UNITS: [(i32, &str); 6] = [
    (-15, "fs"),
    (-12, "ps"),
    (-9, "ns"),
    (-6, "µs"),
    (-3, "ms"),
    (0, "s"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    pub time: f64,
    pub label: String,
}

fn choose_unit(units: f64, timescale: i8) -> (i32, &'static str) {
    if units <= 0.0 || !units.is_finite() {
        return UNITS
            .iter()
            .copied()
            .find(|(e, _)| *e == i32::from(timescale))
            .unwrap_or((-9, "ns"));
    }
    let seconds_exp = units.log10() + f64::from(timescale);
    let mut best = UNITS[0];
    for u in UNITS {
        if seconds_exp >= f64::from(u.0) {
            best = u;
        }
    }
    if best.0 < i32::from(timescale) {
        best = UNITS
            .iter()
            .copied()
            .find(|(e, _)| *e >= i32::from(timescale))
            .unwrap_or(best);
    }
    best
}

#[must_use]
pub fn unit_label(timescale: i8) -> String {
    let exp = i32::from(timescale);
    match UNITS.iter().rev().find(|(e, _)| *e <= exp) {
        Some(&(e, suffix)) if e == exp => suffix.to_owned(),
        Some(&(e, suffix)) => format!("{} {suffix}", 10u64.pow((exp - e) as u32)),
        None => format!("1e{exp} s"),
    }
}

fn trim_float(v: f64, max_decimals: usize) -> String {
    let s = format!("{v:.max_decimals$}");
    if s.contains('.') {
        let s = s.trim_end_matches('0');
        s.trim_end_matches('.').to_string()
    } else {
        s
    }
}

#[must_use]
pub fn format_time(t: f64, base: TimeBase<'_>) -> String {
    if let Some(unit) = base.unit {
        return format!("{} {unit}", trim_float(t, 3));
    }
    let timescale = base.timescale;
    let (exp, suffix) = choose_unit(t.abs().max(1.0), timescale);
    let v = t * 10f64.powi(i32::from(timescale) - exp);
    let decimals = if v.abs() >= 100.0 {
        1
    } else if v.abs() >= 10.0 {
        2
    } else {
        3
    };
    format!("{} {suffix}", trim_float(v, decimals))
}

fn format_in_unit(t: f64, timescale: i8, exp: i32, decimals: usize) -> String {
    let v = t * 10f64.powi(i32::from(timescale) - exp);
    trim_float(v, decimals)
}

fn tick_step(viewport: &Viewport, width_px: f64, min_spacing_px: f64) -> f64 {
    let max_ticks = (width_px / min_spacing_px).max(1.0);
    let raw = viewport.width() / max_ticks;
    let mag = 10f64.powf(raw.log10().floor());
    for s in TICK_STEPS {
        let step = s * mag;
        if step >= raw {
            return step;
        }
    }
    10.0 * mag
}

#[must_use]
pub fn ticks<'a>(
    viewport: &Viewport,
    width_px: f64,
    base: TimeBase<'a>,
    min_spacing_px: f64,
) -> (Vec<Tick>, &'a str) {
    if width_px <= 0.0 {
        return (Vec::new(), "");
    }
    let step = tick_step(viewport, width_px, min_spacing_px);
    let timescale = base.timescale;
    let (exp, suffix) = match base.unit {
        Some(unit) => (i32::from(timescale), unit),
        None => choose_unit(step, timescale),
    };
    let step_in_unit = step * 10f64.powi(i32::from(timescale) - exp);
    let decimals = if step_in_unit >= 1.0 {
        0
    } else {
        (-step_in_unit.log10().floor()) as usize
    }
    .min(6);
    let (first, last) = (
        (viewport.start / step).ceil(),
        (viewport.end / step).floor(),
    );
    if !(step > 0.0 && first.is_finite() && last.is_finite()) {
        return (Vec::new(), suffix);
    }
    let (first, last) = (first as i64, last as i64);
    let mut out = Vec::new();
    for k in first..=last.min(first.saturating_add(999)) {
        let t = k as f64 * step;
        out.push(Tick {
            time: t,
            label: format_in_unit(t, timescale, exp, decimals),
        });
    }
    (out, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units() {
        assert_eq!(choose_unit(1.0, -9), (-9, "ns"));
        assert_eq!(choose_unit(1500.0, -9), (-6, "µs"));
        assert_eq!(choose_unit(2_000_000.0, -12), (-6, "µs"));
        assert_eq!(choose_unit(0.5, -9), (-9, "ns"));
    }

    #[test]
    fn formats() {
        assert_eq!(format_time(1250.0, TimeBase::si(-9)), "1.25 µs");
        assert_eq!(format_time(958_189.0, TimeBase::si(-9)), "958.2 µs");
        assert_eq!(format_time(0.0, TimeBase::si(-9)), "0 ns");
        assert_eq!(format_time(5.0, TimeBase::si(-12)), "5 ps");
        assert_eq!(format_time(3_000_000.0, TimeBase::si(-9)), "3 ms");
        let cycles = TimeBase {
            timescale: 0,
            unit: Some("cycle"),
        };
        assert_eq!(format_time(1250.0, cycles), "1250 cycle");
        assert_eq!(format_time(0.5, cycles), "0.5 cycle");
    }

    #[test]
    fn nice_steps() {
        let v = Viewport {
            start: 0.0,
            end: 1000.0,
        };
        let step = tick_step(&v, 1000.0, 100.0);
        assert_eq!(step, 100.0);
        let (t, unit) = ticks(&v, 1000.0, TimeBase::si(-9), 100.0);
        assert_eq!(unit, "ns");
        assert_eq!(t.len(), 11);
        assert_eq!(t[1].label, "100");
        let cycles = TimeBase {
            timescale: 0,
            unit: Some("cycle"),
        };
        let (t, unit) = ticks(&v, 1000.0, cycles, 100.0);
        assert_eq!(unit, "cycle");
        assert_eq!(t[1].label, "100");
        let v = Viewport {
            start: 1000.0,
            end: 1250.0,
        };
        let (t, unit) = ticks(&v, 1000.0, TimeBase::si(-9), 100.0);
        assert_eq!(unit, "ns");
        assert_eq!(t[0].label, "1000");
        assert!(t.windows(2).all(|w| w[1].time > w[0].time));
    }

    #[test]
    fn sub_unit_decimals() {
        let v = Viewport {
            start: 0.0,
            end: 2500.0,
        };
        let (t, unit) = ticks(&v, 1000.0, TimeBase::si(-12), 100.0);
        assert_eq!(unit, "ps");
        assert_eq!(t[1].label, "250");
    }

    #[test]
    fn degenerate_windows_have_no_ticks() {
        let t = (1u64 << 60) as f64;
        let v = Viewport { start: t, end: t };
        assert!(ticks(&v, 1000.0, TimeBase::si(-15), 100.0).0.is_empty());
        let v = Viewport {
            start: 1e21,
            end: 1e21 + 1e7,
        };
        assert!(ticks(&v, 1000.0, TimeBase::si(-15), 100.0).0.len() <= 1000);
    }
}
