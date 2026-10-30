//! The prepared view of one recorded transaction: a pure function of the resident record,
//! the document's time base and the reader's preferences.

use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use crate::clock::{Clock, ClockView, format_position};
use crate::data::loaded_tracks::LoadedGenerator;
use crate::data::text::{
    PREVIEW_BYTES, Radix, attribute_value_min_bytes, format_attribute, format_attribute_radix,
    is_integer, join_path_limited, truncate_ref,
};
use crate::document::Document;
use crate::pipeline::model::LABEL_ATTRIBUTE;
use crate::pipeline::palette::{StagePalette, StageSwatch};
use crate::trace::Traced;
use crate::wave::timeline::format_time;
use volna_trace::data::loaded_tracks::TransactionLocation;
use volna_trace::data::transactions::{
    AttributeValue, Attributes, SpanKind, Status, TrackRef, Transaction, TransactionRef,
};

pub const VIEW_BYTES: usize = 128 * 1024;
pub const FILTER_THRESHOLD: usize = 8;

fn clock_time(clock: Option<&Clock>, time: u64) -> Option<String> {
    let clock = clock?;
    let timeline = clock.timeline()?;
    let at = timeline.cycle_at(time)?;
    Some(format!(
        "{} {}",
        clock.name,
        format_position(&ClockView::default(), timeline, &at)
    ))
}

fn clock_duration(clock: Option<&Clock>, begin: u64, end: u64) -> Option<String> {
    let timeline = clock?.timeline()?;
    let first = timeline.cycle_at(begin)?;
    let last = timeline.cycle_at(end)?;
    if first.stopped || last.stopped {
        return None;
    }
    let cycles = (last.cycle as f64 + last.fraction) - (first.cycle as f64 + first.fraction);
    if cycles.fract().abs() < 1e-9 {
        Some(format!("{cycles:.0} clk"))
    } else {
        Some(format!("{cycles:.2} clk"))
    }
}

#[cfg(test)]
mod clock_readout_tests {
    use std::sync::Arc;

    use super::*;
    use crate::clock::{ClockState, ClockTimeline};
    use crate::trace::TraceId;

    #[test]
    fn linked_clock_formats_positions_and_elapsed_cycles() {
        let clock = Clock {
            trace: TraceId::A,
            track: TrackRef(1),
            path: "cpu.clock".into(),
            name: "clock".into(),
            state: ClockState::Ready(Arc::new(
                ClockTimeline::new(vec![(10, 50, 10)], false).unwrap(),
            )),
        };
        assert_eq!(
            clock_time(Some(&clock), 25).as_deref(),
            Some("clock 1 + 0.50")
        );
        assert_eq!(
            clock_duration(Some(&clock), 20, 45).as_deref(),
            Some("2.50 clk")
        );
        assert_eq!(
            clock_duration(Some(&clock), 20, 30).as_deref(),
            Some("1 clk")
        );
        assert_eq!(clock_time(Some(&clock), 5), None);
    }
}

const PHASES: [&str; 2] = ["record", "end"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionKey {
    Attributes,
    Stages,
    Events,
    Related,
}

impl SectionKey {
    pub const ALL: [SectionKey; 4] = [
        SectionKey::Attributes,
        SectionKey::Stages,
        SectionKey::Events,
        SectionKey::Related,
    ];

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            SectionKey::Attributes => "attributes",
            SectionKey::Stages => "stages",
            SectionKey::Events => "events",
            SectionKey::Related => "related",
        }
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            SectionKey::Attributes => "Attributes",
            SectionKey::Stages => "Stages",
            SectionKey::Events => "Events",
            SectionKey::Related => "Related",
        }
    }

    #[must_use]
    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.key() == key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewPrefs {
    pub detail_items: usize,
    pub radix: BTreeMap<String, Radix>,
    pub collapsed: BTreeSet<SectionKey>,
    pub filter: String,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self {
            detail_items: 20,
            radix: BTreeMap::new(),
            collapsed: BTreeSet::new(),
            filter: String::new(),
        }
    }
}

impl ViewPrefs {
    #[must_use]
    pub fn radix_of(&self, key: &str) -> Radix {
        self.radix.get(key).copied().unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section<T> {
    pub key: SectionKey,
    pub total: usize,
    pub matched: usize,
    pub collapsed: bool,
    pub rows: Vec<T>,
}

impl<T> Section<T> {
    #[must_use]
    pub fn cut(&self) -> bool {
        self.rows.len() < self.matched
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    pub track: Traced<TrackRef>,
    pub scope: Vec<String>,
    pub generator: String,
    pub ordinal: usize,
    pub id: TransactionRef,
    pub label: Option<String>,
    pub status: Status,
    pub status_text: &'static str,
    pub kind: Option<&'static str>,
    pub parent: Option<TransactionRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    pub begin: u64,
    pub end: Option<u64>,
    pub duration: u64,
    pub begin_text: String,
    pub end_text: String,
    pub duration_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LaneRow {
    pub lane: String,
    pub primary: bool,
    pub cells: Vec<LaneCell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LaneCell {
    pub name: String,
    pub begin: u64,
    pub end: u64,
    pub start: f32,
    pub width: f32,
    pub swatch: StageSwatch,
    pub stage: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventTick {
    pub name: String,
    pub time: u64,
    pub at: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttrRow {
    pub key: String,
    pub attribute: usize,
    pub phase: Option<&'static str>,
    pub value: String,
    pub radix: Option<Radix>,
    pub kind: &'static str,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageRow {
    pub name: String,
    pub lane: String,
    pub primary: bool,
    pub begin: u64,
    pub end: Option<u64>,
    pub duration: u64,
    pub share: f32,
    pub swatch: StageSwatch,
    pub begin_text: String,
    pub end_text: String,
    pub duration_text: String,
    pub attributes: Vec<Chip>,
    pub attributes_total: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventRow {
    pub name: String,
    pub time: u64,
    pub time_text: String,
    pub attributes: Vec<Chip>,
    pub attributes_total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefRole {
    Parent,
    Child,
    Relation { outgoing: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RefRow {
    pub role: RefRole,
    pub group: String,
    pub target: TransactionLocation,
    pub trace: crate::trace::TraceId,
    pub label: Option<String>,
    pub track: Option<Vec<String>>,
    pub relation_label: Option<String>,
    pub loaded: bool,
}

impl RefRow {
    #[must_use]
    pub fn target_track(&self) -> Option<Traced<TrackRef>> {
        self.target
            .generator
            .map(|generator| Traced::new(self.trace, generator))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TxView {
    pub identity: Identity,
    pub timing: Timing,
    pub lifeline: Vec<LaneRow>,
    pub event_ticks: Vec<EventTick>,
    pub cursor: Option<f32>,
    pub attributes: Section<AttrRow>,
    pub stages: Section<StageRow>,
    pub events: Section<EventRow>,
    pub related: Section<RefRow>,
    pub truncated_bytes: bool,
}

impl TxView {
    #[must_use]
    pub fn filterable(&self) -> bool {
        self.attributes.total > FILTER_THRESHOLD
    }
}

#[must_use]
pub fn view(
    doc: &Document,
    track: Traced<TrackRef>,
    id: TransactionRef,
    prefs: &ViewPrefs,
) -> Option<TxView> {
    let generator = doc.resident_generator(track)?;
    let tx = generator.transaction(id)?;
    let session = doc.session(track.trace)?;
    let catalog = session.tracks();
    let declaration = catalog.iter().find(|t| t.id == track.item)?;
    let mut budget = Budget::new(VIEW_BYTES - 4096);
    let scope = budget.path(&declaration.path[..declaration.path.len().saturating_sub(1)]);
    let base = doc.time_base();
    let clock = doc
        .clocks
        .linked(track)
        .filter(|clock| clock.timeline().is_some());
    let time = |t: u64| clock_time(clock, t).unwrap_or_else(|| format_time(t as f64, base));
    let elapsed = |begin: u64, end: u64| {
        clock_duration(clock, begin, end)
            .unwrap_or_else(|| format_time(end.saturating_sub(begin) as f64, base))
    };
    let palette = generator.palette();
    let limit = prefs.detail_items.clamp(1, 1000);

    let label = tx
        .attributes
        .iter()
        .find(|a| a.key == LABEL_ATTRIBUTE)
        .map(|a| budget.text(&format_attribute(&a.value, PREVIEW_BYTES)));
    let identity = Identity {
        track,
        scope,
        generator: declaration
            .path
            .last()
            .map(|s| budget.text(s))
            .unwrap_or_default(),
        ordinal: generator.transaction_ordinal(id).unwrap_or(0),
        id,
        label,
        status: tx.status,
        status_text: status_text(tx.status),
        kind: kind_name(tx.kind),
        parent: tx.parent,
    };

    let open = tx.status == Status::Open;
    let duration = tx.end.saturating_sub(tx.begin);
    let timing = Timing {
        begin: tx.begin,
        end: (!open).then_some(tx.end),
        duration,
        begin_text: budget.text(&time(tx.begin)),
        end_text: budget.text(&if open { "open".into() } else { time(tx.end) }),
        duration_text: budget.text(&if open {
            format!("≥ {}", elapsed(tx.begin, tx.end))
        } else {
            elapsed(tx.begin, tx.end)
        }),
    };

    let span = duration.max(1) as f64;
    let fraction = |t: u64| ((t.saturating_sub(tx.begin) as f64) / span).clamp(0.0, 1.0) as f32;
    let (lifeline, event_ticks) = lifeline(tx, palette, &fraction, limit, &mut budget);
    let cursor = doc
        .shared
        .cursor
        .filter(|c| (tx.begin..=tx.end.max(tx.begin)).contains(c))
        .map(fraction);

    let attributes = attributes(tx, prefs, limit, &mut budget);
    let mut stages = stages(
        tx,
        palette,
        generator.stage_order(identity.ordinal),
        limit,
        duration,
        (&time, &elapsed),
        &mut budget,
    );
    let mut events = events(tx, limit, &time, &mut budget);
    let mut related = related(doc, &generator, tx, track, limit, &mut budget);
    stages.collapsed = prefs.collapsed.contains(&SectionKey::Stages);
    events.collapsed = prefs.collapsed.contains(&SectionKey::Events);
    related.collapsed = prefs.collapsed.contains(&SectionKey::Related);

    Some(TxView {
        identity,
        timing,
        lifeline,
        event_ticks,
        cursor,
        attributes,
        stages,
        events,
        related,
        truncated_bytes: budget.truncated,
    })
}

struct Budget {
    remaining: usize,
    truncated: bool,
}

impl Budget {
    fn new(bytes: usize) -> Self {
        Self {
            remaining: bytes,
            truncated: false,
        }
    }

    fn text(&mut self, value: &str) -> String {
        let room = self.remaining.min(PREVIEW_BYTES);
        let out = truncate_ref(value, room);
        self.truncated |= out.len() < value.len();
        self.remaining = self.remaining.saturating_sub(out.len());
        out
    }

    fn value(&mut self, value: &AttributeValue, radix: Radix) -> (String, bool) {
        let room = self.remaining.min(PREVIEW_BYTES);
        let mut out = format_attribute_radix(value, radix, room);
        out.shrink_to_fit();
        let cut = attribute_value_min_bytes(value) > out.len();
        self.truncated |= cut;
        self.remaining = self.remaining.saturating_sub(out.len());
        (out, cut)
    }

    fn slot<T>(&mut self, rows: &mut Vec<T>) -> bool {
        if rows.len() < rows.capacity() {
            return true;
        }
        let size = std::mem::size_of::<T>().max(1);
        let growth = rows.capacity().max(1).min(self.remaining / size);
        if growth == 0 {
            self.truncated = true;
            return false;
        }
        self.remaining -= growth * size;
        rows.reserve_exact(rows.capacity() + growth - rows.len());
        true
    }

    fn path(&mut self, path: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for part in path {
            if !self.slot(&mut out) {
                break;
            }
            out.push(self.text(part));
        }
        out
    }

    fn ordered<K: Ord, V>(
        &mut self,
        items: impl Iterator<Item = (K, V)>,
        limit: usize,
    ) -> Vec<Ranked<K, V>> {
        let count = limit.min(self.remaining / std::mem::size_of::<Ranked<K, V>>().max(1));
        self.remaining -= count * std::mem::size_of::<Ranked<K, V>>();
        let mut heap = BinaryHeap::with_capacity(count);
        for (key, value) in items {
            if count == 0 {
                self.truncated = true;
                break;
            }
            let item = Ranked { key, value };
            if heap.len() < count {
                heap.push(item);
            } else if item.key < heap.peek().unwrap().key {
                *heap.peek_mut().unwrap() = item;
            }
        }
        heap.into_sorted_vec()
    }
}

struct Ranked<K, V> {
    key: K,
    value: V,
}
impl<K: Ord, V> PartialEq for Ranked<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl<K: Ord, V> Eq for Ranked<K, V> {}
impl<K: Ord, V> PartialOrd for Ranked<K, V> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<K: Ord, V> Ord for Ranked<K, V> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

fn contains_folded(text: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    text.char_indices().any(|(i, _)| {
        let mut hay = text[i..].chars().flat_map(char::to_lowercase);
        needle
            .chars()
            .flat_map(char::to_lowercase)
            .all(|c| hay.next() == Some(c))
    })
}

fn status_text(status: Status) -> &'static str {
    match status {
        Status::Unset => "ended",
        Status::Ok => "ok",
        Status::Error => "error",
        Status::Aborted => "aborted",
        Status::Open => "open",
    }
}

fn kind_name(kind: SpanKind) -> Option<&'static str> {
    match kind {
        SpanKind::Unspecified => None,
        SpanKind::Internal => Some("internal"),
        SpanKind::Server => Some("server"),
        SpanKind::Client => Some("client"),
        SpanKind::Producer => Some("producer"),
        SpanKind::Consumer => Some("consumer"),
    }
}

fn value_kind(value: &AttributeValue) -> &'static str {
    match value {
        AttributeValue::Null => "null",
        AttributeValue::Bool(_) => "bool",
        AttributeValue::I64(_) | AttributeValue::U64(_) => "int",
        AttributeValue::F64(_) => "real",
        AttributeValue::Text(_) => "text",
        AttributeValue::Bytes(_) => "bytes",
        AttributeValue::Logic { .. } => "logic",
        AttributeValue::Time(_) => "time",
        AttributeValue::Enum { .. } => "enum",
        AttributeValue::Pointer(_) => "pointer",
        AttributeValue::Fixed { .. } | AttributeValue::UFixed { .. } => "fixed",
        AttributeValue::List(_) => "list",
        AttributeValue::Map(_) => "map",
    }
}

fn split_phase(key: &str) -> (&str, Option<&'static str>) {
    for phase in PHASES {
        if let Some(base) = key.strip_suffix(phase)
            && let Some(base) = base.strip_suffix('.')
            && !base.is_empty()
        {
            return (base, Some(phase));
        }
    }
    (key, None)
}

fn lifeline(
    tx: &Transaction,
    palette: &StagePalette,
    fraction: &impl Fn(u64) -> f32,
    limit: usize,
    budget: &mut Budget,
) -> (Vec<LaneRow>, Vec<EventTick>) {
    let primary = palette.primary_lane();
    let mut lanes: Vec<LaneRow> = Vec::new();
    for (index, stage) in tx.stages.iter().enumerate().take(limit) {
        let row = if let Some(row) = lanes.iter().position(|l| {
            l.cells
                .first()
                .and_then(|c| c.stage)
                .is_some_and(|i| tx.stages[i].lane == stage.lane)
        }) {
            row
        } else {
            if !budget.slot(&mut lanes) {
                break;
            }
            let row = lanes.len();
            lanes.push(LaneRow {
                lane: budget.text(&stage.lane),
                primary: stage.lane == primary,
                cells: Vec::new(),
            });
            row
        };
        if !budget.slot(&mut lanes[row].cells) {
            break;
        }
        let end = crate::pipeline::PipelineModel::stage_end(tx, stage);
        let start = fraction(stage.begin);
        lanes[row].cells.push(LaneCell {
            name: budget.text(&stage.name),
            begin: stage.begin,
            end,
            start,
            width: (fraction(end) - start).max(0.0),
            swatch: if stage.lane == primary {
                palette.swatch(&stage.name)
            } else {
                StageSwatch::Fallback
            },
            stage: Some(index),
        });
    }
    lanes.sort_unstable_by_key(|lane| !lane.primary);
    if tx.stages.is_empty() && budget.slot(&mut lanes) {
        let mut cells = Vec::new();
        if budget.slot(&mut cells) {
            cells.push(LaneCell {
                name: String::new(),
                begin: tx.begin,
                end: tx.end,
                start: 0.0,
                width: 1.0,
                swatch: StageSwatch::Fallback,
                stage: None,
            });
        }
        lanes.push(LaneRow {
            lane: budget.text(primary),
            primary: true,
            cells,
        });
    }
    let mut ticks = Vec::new();
    for event in tx.events.iter().take(limit) {
        if !budget.slot(&mut ticks) {
            break;
        }
        ticks.push(EventTick {
            name: budget.text(&event.name),
            time: event.time,
            at: fraction(event.time),
        });
    }
    (lanes, ticks)
}

fn attributes(
    tx: &Transaction,
    prefs: &ViewPrefs,
    limit: usize,
    budget: &mut Budget,
) -> Section<AttrRow> {
    let filter = prefs.filter.trim();
    let listed = tx
        .attributes
        .iter()
        .enumerate()
        .filter(|(_, a)| a.key != LABEL_ATTRIBUTE);
    let total = listed.clone().count();
    let matching = listed.filter(|(_, a)| {
        filter.is_empty()
            || contains_folded(&a.key, filter)
            || contains_folded(&format_attribute(&a.value, PREVIEW_BYTES), filter)
    });
    let matched = matching.clone().count();
    let mut rows = Vec::new();
    for (index, attribute) in matching.take(limit) {
        if !budget.slot(&mut rows) {
            break;
        }
        let radix = is_integer(&attribute.value).then(|| prefs.radix_of(&attribute.key));
        let (value, truncated) = budget.value(&attribute.value, radix.unwrap_or_default());
        let (key, phase) = split_phase(&attribute.key);
        rows.push(AttrRow {
            key: budget.text(key),
            attribute: index,
            phase,
            value,
            radix,
            kind: value_kind(&attribute.value),
            truncated,
        });
    }
    Section {
        key: SectionKey::Attributes,
        total,
        matched,
        collapsed: prefs.collapsed.contains(&SectionKey::Attributes),
        rows,
    }
}

fn chips(attributes: &Attributes, limit: usize, budget: &mut Budget) -> Vec<Chip> {
    let mut chips = Vec::new();
    for (key, value) in attributes.iter().take(limit) {
        if !budget.slot(&mut chips) {
            break;
        }
        let key = budget.text(key);
        let (value, _) = budget.value(value, Radix::Dec);
        chips.push(Chip { key, value });
    }
    chips
}

fn stages(
    tx: &Transaction,
    palette: &StagePalette,
    order: &[usize],
    limit: usize,
    lifetime: u64,
    formatters: (&impl Fn(u64) -> String, &impl Fn(u64, u64) -> String),
    budget: &mut Budget,
) -> Section<StageRow> {
    let (time, elapsed) = formatters;
    let primary = palette.primary_lane();
    let mut rows = Vec::new();
    for &index in order.iter().take(limit) {
        if !budget.slot(&mut rows) {
            break;
        }
        let stage = &tx.stages[index];
        let end = crate::pipeline::PipelineModel::stage_end(tx, stage);
        let duration = end.saturating_sub(stage.begin);
        let attributes_total = stage.attributes.len();
        rows.push(StageRow {
            name: budget.text(&stage.name),
            lane: budget.text(&stage.lane),
            primary: stage.lane == primary,
            begin: stage.begin,
            end: stage.end,
            duration,
            share: if lifetime == 0 {
                1.0
            } else {
                (duration as f64 / lifetime as f64).clamp(0.0, 1.0) as f32
            },
            swatch: if stage.lane == primary {
                palette.swatch(&stage.name)
            } else {
                StageSwatch::Fallback
            },
            begin_text: budget.text(&time(stage.begin)),
            end_text: budget.text(&match stage.end {
                Some(end) => time(end),
                None => "open".into(),
            }),
            duration_text: budget.text(&elapsed(stage.begin, end)),
            attributes: chips(&stage.attributes, limit, budget),
            attributes_total,
        });
    }
    Section {
        key: SectionKey::Stages,
        total: tx.stages.len(),
        matched: tx.stages.len(),
        collapsed: false,
        rows,
    }
}

fn events(
    tx: &Transaction,
    limit: usize,
    time: &impl Fn(u64) -> String,
    budget: &mut Budget,
) -> Section<EventRow> {
    let order = budget.ordered(
        tx.events.iter().enumerate().map(|(i, e)| ((e.time, i), i)),
        limit,
    );
    let mut rows = Vec::new();
    for item in order {
        let index = item.value;
        if !budget.slot(&mut rows) {
            break;
        }
        let event = &tx.events[index];
        rows.push(EventRow {
            name: budget.text(&event.name),
            time: event.time,
            time_text: budget.text(&time(event.time)),
            attributes_total: event.attributes.len(),
            attributes: chips(&event.attributes, limit, budget),
        });
    }
    Section {
        key: SectionKey::Events,
        total: tx.events.len(),
        matched: tx.events.len(),
        collapsed: false,
        rows,
    }
}

fn related(
    doc: &Document,
    generator: &LoadedGenerator,
    tx: &Transaction,
    track: Traced<TrackRef>,
    limit: usize,
    budget: &mut Budget,
) -> Section<RefRow> {
    let here = TransactionLocation {
        transaction: tx.id,
        generator: Some(track.item),
    };
    let parent = generator.parent(tx.id);
    let children = || {
        doc.resident_generators(track.trace).flat_map(move |other| {
            other
                .children(here)
                .iter()
                .map(move |&child| TransactionLocation {
                    transaction: child,
                    generator: Some(other.generator()),
                })
        })
    };
    let outgoing = |e: &volna_trace::data::loaded_tracks::LoadedRelation| {
        generator.transaction_ordinal(e.relation.from) == generator.transaction_ordinal(tx.id)
    };
    let edge_count = generator.relations_of(tx.id).count();
    let edges = budget.ordered(
        generator
            .relations_of(tx.id)
            .enumerate()
            .map(|(i, e)| ((e.relation.kind.as_str(), !outgoing(e), i), e)),
        limit,
    );
    let total = usize::from(parent.is_some())
        + doc
            .resident_generators(track.trace)
            .map(|other| other.children(here).len())
            .sum::<usize>()
        + edge_count;
    let entries = parent
        .map(|parent| ("Parent", parent, RefRole::Parent, None))
        .into_iter()
        .chain(children().map(|child| ("Children", child, RefRole::Child, None)))
        .chain(edges.iter().map(|item| {
            let edge = item.value;
            let outgoing = outgoing(edge);
            let (target, target_generator) = if outgoing {
                (edge.relation.to, edge.to_generator)
            } else {
                (edge.relation.from, edge.from_generator)
            };
            let label = edge
                .relation
                .attributes
                .iter()
                .find(|(key, _)| key == LABEL_ATTRIBUTE)
                .map(|(_, value)| value);
            (
                edge.relation.kind.as_str(),
                TransactionLocation {
                    transaction: target,
                    generator: target_generator,
                },
                RefRole::Relation { outgoing },
                label,
            )
        }));
    let catalog = doc.session(track.trace).map_or(&[][..], |s| s.tracks());
    let mut rows = Vec::new();
    for (group, target, role, relation_label) in entries.take(limit) {
        if !budget.slot(&mut rows) {
            break;
        }
        let resident = target
            .generator
            .and_then(|generator| doc.resident_generator(track.with(generator)));
        let label = resident
            .as_ref()
            .and_then(|g| g.transaction(target.transaction))
            .and_then(|tx| tx.attributes.iter().find(|a| a.key == LABEL_ATTRIBUTE))
            .map(|a| budget.value(&a.value, Radix::Dec).0)
            .filter(|label| !label.is_empty())
            .or_else(|| {
                target.generator.is_none().then(|| {
                    budget.text(if target.transaction.0 == 0 {
                        "outside recording"
                    } else {
                        "record unavailable"
                    })
                })
            });
        let path = (target.generator != Some(track.item))
            .then(|| {
                catalog
                    .iter()
                    .find(|t| Some(t.id) == target.generator)
                    .map(|t| budget.path(&t.path))
            })
            .flatten();
        let group = match role {
            RefRole::Relation { outgoing } => {
                let mut text = budget.text(group);
                let suffix = if outgoing { " · to" } else { " · from" };
                if budget.remaining >= suffix.len() {
                    budget.remaining -= suffix.len();
                    text.reserve_exact(suffix.len());
                    text.push_str(suffix);
                }
                text
            }
            _ => budget.text(group),
        };
        rows.push(RefRow {
            role,
            group,
            target,
            trace: track.trace,
            label,
            track: path,
            relation_label: relation_label.map(|value| budget.value(value, Radix::Dec).0),
            loaded: resident.is_some(),
        });
    }
    Section {
        key: SectionKey::Related,
        total,
        matched: total,
        collapsed: false,
        rows,
    }
}

pub fn copy_tsv(
    doc: &Document,
    track: Traced<TrackRef>,
    id: TransactionRef,
    limit: usize,
) -> anyhow::Result<String> {
    let generator = doc
        .resident_generator(track)
        .ok_or_else(|| anyhow::anyhow!("Record is not loaded."))?;
    let tx = generator
        .transaction(id)
        .ok_or_else(|| anyhow::anyhow!("Record is not loaded."))?;
    let catalog = doc.session(track.trace).map_or(&[][..], |s| s.tracks());
    let mut truncated = false;
    let path = catalog
        .iter()
        .find(|t| t.id == track.item)
        .map(|t| join_path_limited(&t.path, limit, &mut truncated))
        .unwrap_or_default();
    let mut text = String::new();
    let row = |text: &mut String, cells: &[&str]| -> anyhow::Result<()> {
        crate::data::text::append_exact(text, &cells.join("\t"), limit)?;
        crate::data::text::append_exact(text, "\n", limit)
    };
    row(
        &mut text,
        &[
            "Generator",
            "ID",
            "Begin",
            "End",
            "Duration",
            "Status",
            "Kind",
        ],
    )?;
    row(
        &mut text,
        &[
            &path,
            &tx.id.0.to_string(),
            &tx.begin.to_string(),
            &tx.end.to_string(),
            &tx.end.saturating_sub(tx.begin).to_string(),
            crate::data::text::status_name(tx.status),
            kind_name(tx.kind).unwrap_or("—"),
        ],
    )?;
    if !tx.attributes.is_empty() {
        row(&mut text, &["Attribute", "Value"])?;
        for attribute in &tx.attributes {
            let value = format_attribute(&attribute.value, limit);
            row(&mut text, &[&attribute.key, &value])?;
        }
    }
    if !tx.stages.is_empty() {
        row(&mut text, &["Stage", "Lane", "Begin", "End", "Attributes"])?;
        for stage in &tx.stages {
            let (attributes, _) = crate::data::text::format_attributes(&stage.attributes, limit);
            row(
                &mut text,
                &[
                    &stage.name,
                    &stage.lane,
                    &stage.begin.to_string(),
                    &stage
                        .end
                        .map_or_else(|| "open".into(), |end| end.to_string()),
                    &attributes,
                ],
            )?;
        }
    }
    if !tx.events.is_empty() {
        row(&mut text, &["Event", "Time", "Attributes"])?;
        for event in &tx.events {
            let (attributes, _) = crate::data::text::format_attributes(&event.attributes, limit);
            row(
                &mut text,
                &[&event.name, &event.time.to_string(), &attributes],
            )?;
        }
    }
    anyhow::ensure!(!truncated, "Record exceeds the 64 KiB clipboard limit.");
    Ok(text)
}
