//! The traces a document shows together.

mod names;
mod placement;

use std::sync::Arc;

pub use names::derive_names;
pub use placement::Placement;

use volna_trace::data::TraceInfo;
use volna_trace::session::Session;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TraceId(u8);

impl TraceId {
    pub const A: TraceId = TraceId(0);
    pub const MAX: usize = 26;

    #[must_use]
    pub fn new(index: usize) -> Option<Self> {
        (index < Self::MAX).then_some(TraceId(index as u8))
    }

    #[must_use]
    pub fn index(self) -> usize {
        usize::from(self.0)
    }

    #[must_use]
    pub fn letter(self) -> char {
        char::from(b'A' + self.0)
    }

    #[must_use]
    pub fn from_letter(letter: char) -> Option<Self> {
        letter
            .is_ascii_uppercase()
            .then(|| TraceId(letter as u8 - b'A'))
    }

    #[must_use]
    pub fn is_a(&self) -> bool {
        *self == TraceId::A
    }
}

impl Default for TraceId {
    fn default() -> Self {
        TraceId::A
    }
}

impl std::fmt::Display for TraceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.letter())
    }
}

impl serde::Serialize for TraceId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for TraceId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        let mut chars = text.chars();
        match (chars.next().and_then(TraceId::from_letter), chars.next()) {
            (Some(id), None) => Ok(id),
            _ => Err(serde::de::Error::custom(format!(
                "trace letter expected, found {text:?}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Traced<T> {
    pub trace: TraceId,
    pub item: T,
}

impl<T> Traced<T> {
    pub fn new(trace: TraceId, item: T) -> Self {
        Self { trace, item }
    }

    pub fn with<U>(&self, item: U) -> Traced<U> {
        Traced::new(self.trace, item)
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Traced<U> {
        Traced::new(self.trace, f(self.item))
    }
}

impl<T: serde::Serialize> serde::Serialize for Traced<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (self.trace, &self.item).serialize(s)
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Traced<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (trace, item) = <(TraceId, T)>::deserialize(d)?;
        Ok(Self { trace, item })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub timescale: i8,
    pub name: Option<String>,
}

impl Unit {
    #[must_use]
    pub fn of(info: &TraceInfo) -> Self {
        Self {
            timescale: info.timescale,
            name: info.time_unit.clone(),
        }
    }

    fn factor_to(&self, coarser: &Unit) -> Option<u64> {
        if self.name.is_some() || coarser.name.is_some() {
            return (self == coarser).then_some(1);
        }
        let steps = u32::try_from(i32::from(coarser.timescale) - i32::from(self.timescale)).ok()?;
        10u64.checked_pow(steps)
    }

    #[must_use]
    pub fn label(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => crate::wave::timeline::unit_label(self.timescale),
        }
    }
}

#[derive(Clone)]
pub enum SlotState {
    Loading,
    Loaded(Arc<dyn Session>),
    Error(String),
}

#[derive(Clone)]
pub struct TraceSlot {
    pub id: TraceId,
    pub source: String,
    pub uri: Option<String>,
    pub rename: Option<String>,
    pub(crate) state: SlotState,
    pub(crate) generation: u64,
    pub(crate) placement: Placement,
    pub(crate) sizes: Option<Arc<volna_trace::data::ScopeSizes>>,
}

impl TraceSlot {
    pub(crate) fn new(id: TraceId, source: String, uri: Option<String>, generation: u64) -> Self {
        Self {
            id,
            source,
            uri,
            rename: None,
            state: SlotState::Loading,
            generation,
            placement: Placement::IDENTITY,
            sizes: None,
        }
    }

    #[must_use]
    pub fn sizes(&self) -> Option<&volna_trace::data::ScopeSizes> {
        self.sizes.as_deref()
    }

    #[must_use]
    pub fn state(&self) -> &SlotState {
        &self.state
    }

    #[must_use]
    pub fn session(&self) -> Option<&Arc<dyn Session>> {
        match &self.state {
            SlotState::Loaded(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn placement(&self) -> Placement {
        self.placement
    }

    #[must_use]
    pub fn file_name(&self) -> &str {
        names::file_name(&self.source)
    }

    #[must_use]
    pub fn format(&self) -> Option<&'static str> {
        let session = self.session()?;
        Some(
            session
                .format()
                .unwrap_or(if session.remote_id().is_some() {
                    "remote"
                } else {
                    "trace"
                }),
        )
    }

    #[must_use]
    pub fn unit(&self) -> Option<Unit> {
        self.session().map(|s| Unit::of(s.info()))
    }

    #[must_use]
    pub fn range(&self) -> Option<(u64, u64)> {
        let (start, end) = self.session()?.info().time_range;
        Some((
            self.placement.to_session(start),
            self.placement.to_session(end),
        ))
    }
}

#[derive(Clone, Default)]
pub struct TraceSet {
    slots: Vec<TraceSlot>,
    unit: Option<Unit>,
    revision: u64,
}

impl TraceSet {
    pub fn iter(&self) -> impl Iterator<Item = &TraceSlot> {
        self.slots.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    #[must_use]
    pub fn is_combined(&self) -> bool {
        self.slots.len() > 1
    }

    pub fn ids(&self) -> impl Iterator<Item = TraceId> + '_ {
        self.slots.iter().map(|s| s.id)
    }

    #[must_use]
    pub fn get(&self, id: TraceId) -> Option<&TraceSlot> {
        self.slots.iter().find(|s| s.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: TraceId) -> Option<&mut TraceSlot> {
        self.revision += 1;
        self.slots.iter_mut().find(|s| s.id == id)
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn session(&self, id: TraceId) -> Option<&Arc<dyn Session>> {
        self.get(id)?.session()
    }

    pub fn loaded(&self) -> impl Iterator<Item = (TraceId, &Arc<dyn Session>)> {
        self.slots
            .iter()
            .filter_map(|s| s.session().map(|session| (s.id, session)))
    }

    #[must_use]
    pub fn first_loaded(&self) -> Option<(TraceId, &Arc<dyn Session>)> {
        self.loaded().next()
    }

    #[must_use]
    pub fn unit(&self) -> Option<&Unit> {
        self.unit.as_ref()
    }

    pub fn limits(&self) -> Option<(u64, u64)> {
        self.slots
            .iter()
            .filter_map(TraceSlot::range)
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }

    pub fn free_id(&self) -> Option<TraceId> {
        (0..TraceId::MAX)
            .filter_map(TraceId::new)
            .find(|id| self.get(*id).is_none())
    }

    pub(crate) fn insert(&mut self, slot: TraceSlot) {
        debug_assert!(self.get(slot.id).is_none());
        self.revision += 1;
        let at = self.slots.partition_point(|s| s.id < slot.id);
        self.slots.insert(at, slot);
    }

    pub(crate) fn remove(&mut self, id: TraceId) -> Option<TraceSlot> {
        let at = self.slots.iter().position(|s| s.id == id)?;
        self.revision += 1;
        Some(self.slots.remove(at))
    }

    pub(crate) fn admit(&self, info: &TraceInfo) -> anyhow::Result<(Unit, Placement, u64)> {
        let own = Unit::of(info);
        let Some(unit) = &self.unit else {
            return Ok((own, Placement::IDENTITY, 1));
        };
        let refusal = || {
            anyhow::anyhow!(
                "its times are in {}, the others' in {}: traces share a timeline only in \
                 seconds-based units or in the same named unit",
                own.label(),
                unit.label()
            )
        };
        let (session, factor, scale) = match (own.factor_to(unit), unit.factor_to(&own)) {
            (Some(1), _) | (_, Some(1)) => (unit.clone(), 1, 1),
            (Some(factor), _) => (own.clone(), factor, 1),
            (_, Some(scale)) => (unit.clone(), 1, scale),
            _ => return Err(refusal()),
        };
        let placement = Placement::scaled(scale);
        let end = info.time_range.1;
        anyhow::ensure!(
            placement.fits(end)
                && self
                    .limits()
                    .is_none_or(|(_, e)| e.checked_mul(factor).is_some()),
            "its time range does not fit on a timeline in {}",
            session.label()
        );
        Ok((session, placement, factor))
    }

    pub(crate) fn forget_unit(&mut self) {
        self.unit = None;
    }

    pub(crate) fn set_unit(&mut self, unit: Unit) {
        self.unit = Some(unit);
    }

    #[must_use]
    pub fn name(&self, id: TraceId) -> Option<String> {
        let slot = self.get(id)?;
        if let Some(name) = &slot.rename {
            return Some(name.clone());
        }
        let sources: Vec<&str> = self.slots.iter().map(|s| s.source.as_str()).collect();
        let at = self.slots.iter().position(|s| s.id == id)?;
        derive_names(&sources).into_iter().nth(at)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rescale {
    Finer(u64),
    Coarser(u64),
}

impl Rescale {
    pub(crate) fn between(from: i8, to: i8) -> Option<Self> {
        let steps = i32::from(from) - i32::from(to);
        let factor = 10u64.checked_pow(steps.unsigned_abs())?;
        match steps.signum() {
            1 => Some(Self::Finer(factor)),
            -1 => Some(Self::Coarser(factor)),
            _ => None,
        }
    }

    pub(crate) fn time(self, t: u64) -> u64 {
        match self {
            Self::Finer(k) => t.saturating_mul(k),
            Self::Coarser(k) => t / k + u64::from(t % k >= k.div_ceil(2)),
        }
    }

    pub(crate) fn real(self, t: f64) -> f64 {
        match self {
            Self::Finer(k) => t * k as f64,
            Self::Coarser(k) => t / k as f64,
        }
    }
}

pub(crate) trait Retime {
    fn retime(&mut self, by: Rescale);
}

impl Retime for u64 {
    fn retime(&mut self, by: Rescale) {
        *self = by.time(*self);
    }
}

impl<T: Retime> Retime for Option<T> {
    fn retime(&mut self, by: Rescale) {
        if let Some(t) = self {
            t.retime(by);
        }
    }
}

impl<T: Retime> Retime for Vec<T> {
    fn retime(&mut self, by: Rescale) {
        for t in self.iter_mut() {
            t.retime(by);
        }
    }
}

impl Retime for crate::wave::viewport::Viewport {
    fn retime(&mut self, by: Rescale) {
        self.start = by.real(self.start);
        self.end = by.real(self.end);
    }
}

impl<T: crate::nav::Lerp + Retime> Retime for crate::nav::Tween<T> {
    fn retime(&mut self, by: Rescale) {
        self.map(|v| {
            let mut v = v.clone();
            v.retime(by);
            v
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_round_trip_and_serialize() {
        let b = TraceId::new(1).unwrap();
        assert_eq!(b.letter(), 'B');
        assert_eq!(TraceId::from_letter('B'), Some(b));
        assert_eq!(TraceId::from_letter('b'), None);
        assert_eq!(TraceId::new(26), None);
        assert_eq!(serde_json::to_string(&b).unwrap(), "\"B\"");
        let traced = Traced::new(b, "soc.clk".to_owned());
        let json = serde_json::to_string(&traced).unwrap();
        assert_eq!(json, r#"["B","soc.clk"]"#);
        assert_eq!(
            serde_json::from_str::<Traced<String>>(&json).unwrap(),
            traced
        );
        assert!(serde_json::from_str::<TraceId>("\"AB\"").is_err());
    }

    fn info(timescale: i8, end: u64, unit: Option<&str>) -> TraceInfo {
        TraceInfo {
            name: "t".into(),
            design_id: None,
            timescale,
            time_range: (0, end),
            signal_count: 0,
            change_count: None,
            time_unit: unit.map(str::to_owned),
        }
    }

    #[test]
    fn a_finer_trace_refines_the_unit_and_a_coarser_one_is_scaled() {
        let mut set = TraceSet::default();
        let (unit, placement, factor) = set.admit(&info(-9, 1000, None)).unwrap();
        assert_eq!((placement, factor), (Placement::IDENTITY, 1));
        set.set_unit(unit);
        let (unit, placement, factor) = set.admit(&info(-12, 5, None)).unwrap();
        assert_eq!(
            (unit.timescale, placement, factor),
            (-12, Placement::IDENTITY, 1000)
        );
        set.set_unit(unit);
        let (_, placement, factor) = set.admit(&info(-6, 5, None)).unwrap();
        assert_eq!((placement.to_session(5), factor), (5_000_000, 1));
    }

    #[test]
    fn named_units_combine_only_with_themselves() {
        let mut set = TraceSet::default();
        set.set_unit(Unit::of(&info(0, 10, Some("cycle"))));
        assert!(set.admit(&info(0, 10, Some("cycle"))).is_ok());
        let error = set.admit(&info(-9, 10, None)).unwrap_err().to_string();
        assert!(error.contains("in ns, the others' in cycle"), "{error}");
    }

    #[test]
    fn rescaling_is_exact_finer_and_rounds_coarser() {
        assert_eq!(Rescale::between(-9, -9), None);
        let finer = Rescale::between(-9, -12).unwrap();
        assert_eq!((finer, finer.time(783)), (Rescale::Finer(1000), 783_000));
        let coarser = Rescale::between(-12, -9).unwrap();
        assert_eq!(coarser, Rescale::Coarser(1000));
        assert_eq!(
            [
                coarser.time(783_000),
                coarser.time(1_499),
                coarser.time(1_500)
            ],
            [783, 1, 2]
        );
        assert_eq!(coarser.real(2_500.0), 2.5);
    }

    #[test]
    fn a_range_that_would_overflow_is_refused() {
        let mut set = TraceSet::default();
        set.set_unit(Unit::of(&info(-15, 10, None)));
        assert!(set.admit(&info(0, u64::MAX / 10, None)).is_err());
    }
}
