//! The prepared view of one recorded transaction: a pure function of the resident record,
//! the document's time base and the reader's preferences.

use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use crate::clock::{Clock, ClockView, format_position};
use crate::data::loaded_tracks::LoadedGenerator;
use crate::data::text::{
    PREVIEW_BYTES, Radix, attribute_value_min_bytes, format_attribute, format_attribute_radix,
    join_path_limited, truncate_ref,
};
use crate::document::Document;
use crate::pipeline::model::LABEL_ATTRIBUTE;
use crate::pipeline::palette::{StagePalette, StageSwatch};
use crate::trace::Traced;
use crate::wave::timeline::format_time;
use volna_trace::data::loaded_tracks::TransactionLocation;
use volna_trace::data::transactions::{
    AttributeValue, SpanKind, Status, TrackRef, Transaction, TransactionRef,
};

pub const VIEW_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LifelineScale {
    #[default]
    Linear,
    Balanced,
}

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
            scope_id: None,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionKey {
    Attributes,
    Stages,
    StageAttributes,
    Events,
    Related,
}

impl SectionKey {
    pub const ALL: [SectionKey; 5] = [
        SectionKey::Attributes,
        SectionKey::Stages,
        SectionKey::StageAttributes,
        SectionKey::Events,
        SectionKey::Related,
    ];

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            SectionKey::Attributes => "attributes",
            SectionKey::Stages => "stages",
            SectionKey::StageAttributes => "stage-attributes",
            SectionKey::Events => "events",
            SectionKey::Related => "related",
        }
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            SectionKey::Attributes => "Attributes",
            SectionKey::Stages => "Stages",
            SectionKey::StageAttributes => "Stage attributes",
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
    pub lifeline_scale: LifelineScale,
    pub stages_grouped: bool,
    pub stages_show_all: bool,
    pub attributes_grouped: bool,
    pub radix: BTreeMap<String, Radix>,
    pub collapsed: BTreeSet<SectionKey>,
    pub filter: String,
    pub attributes: super::attributes::AttributePrefs,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self {
            detail_items: 20,
            lifeline_scale: LifelineScale::Linear,
            stages_grouped: false,
            stages_show_all: false,
            attributes_grouped: true,
            radix: BTreeMap::new(),
            collapsed: BTreeSet::new(),
            filter: String::new(),
            attributes: super::attributes::AttributePrefs::default(),
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
    pub duration: u64,
    pub offset_text: String,
    pub duration_text: String,
    pub share: f32,
    pub start: f32,
    pub width: f32,
    pub swatch: StageSwatch,
    pub stage: Option<usize>,
    pub stage_id: Option<TransactionRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventTick {
    pub name: String,
    pub time: u64,
    pub at: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageRow {
    pub id: TransactionRef,
    pub selected: bool,
    pub name: String,
    pub lane: String,
    pub primary: bool,
    pub begin: u64,
    pub end: Option<u64>,
    pub duration: u64,
    pub share: f32,
    pub swatch: StageSwatch,
    pub offset_text: String,
    pub begin_text: String,
    pub end_text: String,
    pub duration_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageDetails {
    pub name: String,
    pub attributes: super::attributes::AttributeTree,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventRow {
    pub name: String,
    pub time: u64,
    pub time_text: String,
    pub attributes: super::attributes::AttributeTree,
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
    pub point_kind: Option<crate::data::points::PointKind>,
    pub severity: Option<String>,
    pub source: Vec<(String, String)>,
    pub identity: Identity,
    pub timing: Timing,
    pub lifeline: Vec<LaneRow>,
    pub event_ticks: Vec<EventTick>,
    pub cursor: Option<f32>,
    pub attributes: super::attributes::AttributeTree,
    pub stages: Section<StageRow>,
    pub stage_details: Option<StageDetails>,
    pub events: Section<EventRow>,
    pub related: Section<RefRow>,
    pub truncated_bytes: bool,
}

#[must_use]
pub fn view(
    doc: &Document,
    track: Traced<TrackRef>,
    id: TransactionRef,
    selected_stage: Option<TransactionRef>,
    prefs: &ViewPrefs,
) -> Option<TxView> {
    let generator = doc.resident_generator(track)?;
    let tx = generator.transaction(id)?;
    let owner = super::attributes::AttributeOwner::new(
        track,
        doc.traces().get(track.trace)?.generation(),
        id,
    );
    let session = doc.session(track.trace)?;
    let catalog = session.tracks();
    let declaration = catalog.iter().find(|t| t.id == track.item)?;
    let mut budget = Budget::new(VIEW_BYTES - super::attributes::SCAN_SCRATCH_BYTES);
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

    let point_kind = crate::data::points::PointKind::from_generator(&declaration.kind);
    let label_key = if point_kind == Some(crate::data::points::PointKind::Log) {
        "log.message"
    } else {
        LABEL_ATTRIBUTE
    };
    let severity = (point_kind == Some(crate::data::points::PointKind::Log)).then(|| {
        tx.attributes
            .iter()
            .find(|a| a.key == "log.severity")
            .map_or_else(
                || "unknown".into(),
                |a| budget.text(&format_attribute(&a.value, PREVIEW_BYTES)),
            )
    });
    let label = tx
        .attributes
        .iter()
        .find(|a| a.key == label_key)
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
    let lifeline_cap = budget.remaining / 3;
    let mut lifeline_budget = Budget::new(lifeline_cap);
    let cursor_time = doc
        .shared
        .cursor
        .filter(|c| (tx.begin..=tx.end.max(tx.begin)).contains(c));
    let (lifeline, event_ticks, cursor) = lifeline(
        tx,
        palette,
        &fraction,
        &elapsed,
        LifelineParams {
            lifetime: duration,
            scale: prefs.lifeline_scale,
            cursor_time,
        },
        &mut lifeline_budget,
    );
    let lifeline_used = lifeline_cap.saturating_sub(lifeline_budget.remaining);
    budget.remaining = budget.remaining.saturating_sub(lifeline_used);
    budget.truncated |= lifeline_budget.truncated;

    let source = if point_kind == Some(crate::data::points::PointKind::Log) {
        [
            ("log.file", "File"),
            ("log.line", "Line"),
            ("log.func", "Function"),
            ("log.format", "Format"),
        ]
        .into_iter()
        .filter_map(|(key, name)| {
            tx.attributes.iter().find(|a| a.key == key).map(|a| {
                (
                    name.to_owned(),
                    budget.text(&format_attribute(&a.value, PREVIEW_BYTES)),
                )
            })
        })
        .collect()
    } else {
        Vec::new()
    };
    let attributes = attributes(tx, owner, prefs, point_kind, &mut budget);
    let stage_limit = if prefs.stages_show_all || prefs.stages_grouped {
        usize::MAX
    } else {
        limit
    };
    let mut stages = stages(
        tx,
        palette,
        generator.stage_order(identity.ordinal),
        stage_limit,
        duration,
        (&time, &elapsed),
        &mut budget,
    );
    for row in &mut stages.rows {
        row.selected = Some(row.id) == selected_stage;
    }
    let stage_details = selected_stage
        .and_then(|id| tx.stages.iter().find(|stage| stage.id == id))
        .map(|stage| StageDetails {
            name: budget.text(&stage.name),
            attributes: super::attributes::tree(
                owner.with_record(stage.id),
                SectionKey::StageAttributes,
                stage
                    .attributes
                    .iter()
                    .enumerate()
                    .map(|(i, (k, v))| (i, k.as_str(), v)),
                prefs,
                "",
                &mut budget,
            ),
        });
    let mut events = events(tx, owner, prefs, limit, &time, &mut budget);
    let mut related = related(doc, &generator, tx, track, limit, &mut budget);
    stages.collapsed = prefs.collapsed.contains(&SectionKey::Stages);
    events.collapsed = prefs.collapsed.contains(&SectionKey::Events);
    related.collapsed = prefs.collapsed.contains(&SectionKey::Related);

    Some(TxView {
        point_kind,
        severity,
        source,
        identity,
        timing,
        lifeline,
        event_ticks,
        cursor,
        attributes,
        stages,
        stage_details,
        events,
        related,
        truncated_bytes: budget.truncated,
    })
}

pub(super) struct Budget {
    pub(super) remaining: usize,
    pub(super) truncated: bool,
}

impl Budget {
    pub(super) fn new(bytes: usize) -> Self {
        Self {
            remaining: bytes,
            truncated: false,
        }
    }

    pub(super) fn admit(&mut self, bytes: usize) -> bool {
        if bytes > self.remaining {
            self.truncated = true;
            return false;
        }
        self.remaining -= bytes;
        true
    }
    pub(super) fn text(&mut self, value: &str) -> String {
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

    pub(super) fn slot<T>(&mut self, rows: &mut Vec<T>) -> bool {
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

const MAX_LIFELINE_CELLS: usize = 128;

fn map_balanced(cells: &[LaneCell], t: u64, fallback: &impl Fn(u64) -> f32) -> f32 {
    let Some(first) = cells.first() else {
        return fallback(t);
    };
    if t <= first.begin {
        return first.start;
    }
    for cell in cells {
        if t <= cell.end {
            if t <= cell.begin {
                return cell.start;
            }
            let local = ((t.saturating_sub(cell.begin) as f64) / cell.duration.max(1) as f64)
                .clamp(0.0, 1.0) as f32;
            return (cell.start + local * cell.width).clamp(0.0, 1.0);
        }
    }
    let last = cells.last().unwrap();
    (last.start + last.width).clamp(0.0, 1.0)
}

struct LifelineParams {
    lifetime: u64,
    scale: LifelineScale,
    cursor_time: Option<u64>,
}

fn lifeline(
    tx: &Transaction,
    palette: &StagePalette,
    fraction: &impl Fn(u64) -> f32,
    elapsed: &impl Fn(u64, u64) -> String,
    params: LifelineParams,
    budget: &mut Budget,
) -> (Vec<LaneRow>, Vec<EventTick>, Option<f32>) {
    let LifelineParams {
        lifetime,
        scale,
        cursor_time,
    } = params;
    let primary = palette.primary_lane();
    let mut lane_counts: Vec<(&str, bool, usize)> = Vec::new();
    for stage in &tx.stages {
        if let Some((_, _, count)) = lane_counts
            .iter_mut()
            .find(|(lane, _, _)| *lane == stage.lane)
        {
            *count += 1;
        } else {
            lane_counts.push((&stage.lane, stage.lane == primary, 1));
        }
    }
    lane_counts.sort_unstable_by_key(|(_, is_primary, _)| !*is_primary);

    let mut lanes: Vec<LaneRow> = Vec::new();
    for (lane_name, is_primary, total_stages) in lane_counts {
        if !budget.slot(&mut lanes) {
            break;
        }
        let mut cells = Vec::new();
        let lane_iter = tx
            .stages
            .iter()
            .enumerate()
            .filter(|(_, stage)| stage.lane == lane_name);
        if total_stages <= MAX_LIFELINE_CELLS {
            for (index, stage) in lane_iter {
                if !budget.slot(&mut cells) {
                    break;
                }
                let end = crate::pipeline::PipelineModel::stage_end(tx, stage);
                let stage_duration = end.saturating_sub(stage.begin);
                let share = if lifetime == 0 {
                    1.0
                } else {
                    (stage_duration as f64 / lifetime as f64).clamp(0.0, 1.0) as f32
                };
                let start = fraction(stage.begin);
                cells.push(LaneCell {
                    name: budget.text(&stage.name),
                    begin: stage.begin,
                    end,
                    duration: stage_duration,
                    offset_text: budget.text(&format!("+{}", elapsed(tx.begin, stage.begin))),
                    duration_text: budget.text(&elapsed(stage.begin, end)),
                    share,
                    start,
                    width: (fraction(end) - start).max(0.0),
                    swatch: if is_primary {
                        palette.swatch(&stage.name)
                    } else {
                        StageSwatch::Fallback
                    },
                    stage: Some(index),
                    stage_id: Some(stage.id),
                });
            }
        } else {
            // Stream large stage lists into MAX_LIFELINE_CELLS contiguous bins
            // with O(1) scratch memory so the full [begin, end] lifetime stays visible.
            let mut bin: usize = 0;
            let mut bin_hi = total_stages / MAX_LIFELINE_CELLS;
            let mut pos = 0usize;
            let mut bin_begin = 0u64;
            let mut bin_end = 0u64;
            let mut bin_count = 0usize;
            let mut dominant_idx = 0usize;
            let mut dominant_dur = 0u64;
            for (index, stage) in lane_iter {
                let end = crate::pipeline::PipelineModel::stage_end(tx, stage);
                let dur = end.saturating_sub(stage.begin);
                if bin_count == 0 {
                    bin_begin = stage.begin;
                    bin_end = end;
                    dominant_idx = index;
                    dominant_dur = dur;
                } else {
                    bin_begin = bin_begin.min(stage.begin);
                    bin_end = bin_end.max(end);
                    if dur > dominant_dur {
                        dominant_idx = index;
                        dominant_dur = dur;
                    }
                }
                bin_count += 1;
                pos += 1;
                if pos == bin_hi {
                    if budget.slot(&mut cells) {
                        let dominant = &tx.stages[dominant_idx];
                        let bin_duration = bin_end.saturating_sub(bin_begin);
                        let share = if lifetime == 0 {
                            1.0
                        } else {
                            (bin_duration as f64 / lifetime as f64).clamp(0.0, 1.0) as f32
                        };
                        let start = fraction(bin_begin);
                        let label = if bin_count > 1 {
                            format!("{} (×{bin_count})", dominant.name)
                        } else {
                            dominant.name.clone()
                        };
                        cells.push(LaneCell {
                            name: budget.text(&label),
                            begin: bin_begin,
                            end: bin_end,
                            duration: bin_duration,
                            offset_text: budget.text(&format!("+{}", elapsed(tx.begin, bin_begin))),
                            duration_text: budget.text(&elapsed(bin_begin, bin_end)),
                            share,
                            start,
                            width: (fraction(bin_end) - start).max(0.0),
                            swatch: if is_primary {
                                palette.swatch(&dominant.name)
                            } else {
                                StageSwatch::Fallback
                            },
                            stage: Some(dominant_idx),
                            stage_id: Some(dominant.id),
                        });
                    }
                    bin += 1;
                    let prev_hi = bin_hi;
                    bin_hi = (bin + 1) * total_stages / MAX_LIFELINE_CELLS;
                    while bin < MAX_LIFELINE_CELLS && bin_hi <= prev_hi {
                        bin += 1;
                        bin_hi = (bin + 1) * total_stages / MAX_LIFELINE_CELLS;
                    }
                    bin_count = 0;
                }
            }
        }
        lanes.push(LaneRow {
            lane: budget.text(lane_name),
            primary: is_primary,
            cells,
        });
    }

    if tx.stages.is_empty() && budget.slot(&mut lanes) {
        let mut cells = Vec::new();
        if budget.slot(&mut cells) {
            let duration_text = budget.text(&elapsed(tx.begin, tx.end));
            cells.push(LaneCell {
                name: String::new(),
                begin: tx.begin,
                end: tx.end,
                duration: lifetime,
                offset_text: budget.text("+0"),
                duration_text,
                share: 1.0,
                start: 0.0,
                width: 1.0,
                swatch: StageSwatch::Fallback,
                stage: None,
                stage_id: None,
            });
        }
        lanes.push(LaneRow {
            lane: budget.text(primary),
            primary: true,
            cells,
        });
    }

    if scale == LifelineScale::Balanced
        && let Some((primary_lane, secondary_lanes)) = lanes.split_first_mut()
        && !primary_lane.cells.is_empty()
    {
        let weights: Vec<f64> = primary_lane
            .cells
            .iter()
            .map(|cell| ((cell.duration as f64) + 1.0).ln().max(0.5))
            .collect();
        let total: f64 = weights.iter().sum::<f64>().max(f64::EPSILON);
        let mut pos = 0.0_f64;
        for (cell, weight) in primary_lane.cells.iter_mut().zip(weights) {
            let w = weight / total;
            cell.start = pos as f32;
            cell.width = w as f32;
            pos += w;
        }
        for lane in secondary_lanes {
            for cell in &mut lane.cells {
                let s = map_balanced(&primary_lane.cells, cell.begin, fraction);
                let e = map_balanced(&primary_lane.cells, cell.end, fraction);
                cell.start = s;
                cell.width = (e - s).max(0.0);
            }
        }
    }

    let map_time = |t: u64| match scale {
        LifelineScale::Linear => fraction(t),
        LifelineScale::Balanced => lanes.first().map_or_else(
            || fraction(t),
            |lane| map_balanced(&lane.cells, t, fraction),
        ),
    };

    let mut ticks = Vec::new();
    for event in &tx.events {
        if !budget.slot(&mut ticks) {
            break;
        }
        ticks.push(EventTick {
            name: budget.text(&event.name),
            time: event.time,
            at: map_time(event.time),
        });
    }
    let cursor = cursor_time.map(map_time);
    (lanes, ticks, cursor)
}

fn attributes(
    tx: &Transaction,
    owner: super::attributes::AttributeOwner,
    prefs: &ViewPrefs,
    point_kind: Option<crate::data::points::PointKind>,
    budget: &mut Budget,
) -> super::attributes::AttributeTree {
    let roots = tx
        .attributes
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            (a.key != LABEL_ATTRIBUTE || point_kind == Some(crate::data::points::PointKind::Log))
                && (point_kind.is_none()
                    || (a.key != "record.kind" && a.key != "kind" && !a.key.starts_with("log.")))
        })
        .map(|(i, a)| (i, a.key.as_str(), &a.value));
    super::attributes::tree(
        owner,
        SectionKey::Attributes,
        roots,
        prefs,
        prefs.filter.trim(),
        budget,
    )
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
        rows.push(StageRow {
            id: stage.id,
            selected: false,
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
            offset_text: budget.text(&format!("+{}", elapsed(tx.begin, stage.begin))),
            begin_text: budget.text(&time(stage.begin)),
            end_text: budget.text(&match stage.end {
                Some(end) => time(end),
                None => "open".into(),
            }),
            duration_text: budget.text(&elapsed(stage.begin, end)),
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
    owner: super::attributes::AttributeOwner,
    prefs: &ViewPrefs,
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
            attributes: super::attributes::tree(
                owner.with_record(event.id),
                SectionKey::Attributes,
                event
                    .attributes
                    .iter()
                    .enumerate()
                    .map(|(i, (k, v))| (i, k.as_str(), v)),
                prefs,
                "",
                budget,
            ),
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
            let value = super::attributes::copy_value(&attribute.value, limit)?;
            row(&mut text, &[&attribute.key, &value])?;
        }
    }
    if !tx.stages.is_empty() {
        row(&mut text, &["Stage", "Lane", "Begin", "End", "Attributes"])?;
        for stage in &tx.stages {
            let attributes = super::attributes::copy_attributes(&stage.attributes, limit)?;
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
            let attributes = super::attributes::copy_attributes(&event.attributes, limit)?;
            row(
                &mut text,
                &[&event.name, &event.time.to_string(), &attributes],
            )?;
        }
    }
    anyhow::ensure!(!truncated, "Record exceeds the 64 KiB clipboard limit.");
    Ok(text)
}
