//! Job ownership cancels abandoned work; request identities reject late completions.

use super::{Definition, Input, Interpretation, MeasurementScope, ResultKind};
use crate::{data::admission::Buffer, session::LoadRequest, trace::Traced};
use anyhow::Context as _;
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use volna_trace::{
    data::{SignalHistory, SignalRef},
    remote::memory::MemoryBudget,
};

#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementRow {
    pub start: Option<crate::marker::MarkerId>,
    pub end: Option<crate::marker::MarkerId>,
    pub bounds: (u64, u64),
    pub result: Result<super::Measurement, &'static str>,
}

#[derive(Debug)]
pub struct MeasurementTable {
    rows: Buffer<MeasurementRow>,
    timeline: Buffer<(u64, Option<f64>)>,
}

impl MeasurementTable {
    pub(crate) fn timeline(&self) -> &[(u64, Option<f64>)] {
        &self.timeline
    }
}

impl std::ops::Deref for MeasurementTable {
    type Target = [MeasurementRow];

    fn deref(&self) -> &[MeasurementRow] {
        &self.rows
    }
}

impl PartialEq for MeasurementTable {
    fn eq(&self, other: &Self) -> bool {
        *self.rows == *other.rows
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MeasurementResult {
    Scalar(super::Measurement),
    Table(Arc<MeasurementTable>),
}

#[derive(Clone)]
pub enum DerivedOutput {
    Waveform(Arc<dyn SignalHistory>),
    Measurement(super::Measurement),
    IntervalTable(Arc<MeasurementTable>),
}

impl DerivedOutput {
    #[must_use]
    pub fn measurement(&self) -> Option<MeasurementResult> {
        match self {
            Self::Waveform(_) => None,
            Self::Measurement(measurement) => Some(MeasurementResult::Scalar(*measurement)),
            Self::IntervalTable(rows) => Some(MeasurementResult::Table(rows.clone())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TableInterval {
    start: Option<crate::marker::MarkerId>,
    end: Option<crate::marker::MarkerId>,
    bounds: (u64, u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ResolvedScope {
    Waveform,
    Scalar(Option<(u64, u64)>),
    Table(Vec<TableInterval>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub generation: u64,
    domain: (u64, u64),
    pub(crate) inputs: Vec<KeyInput>,
    pub(crate) calculation: Arc<super::Program>,
    formula: super::FormulaSettings,
    scope: ResolvedScope,
    timescale: i8,
    time_unit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct KeyInput {
    pub signal: Option<Traced<SignalRef>>,
    pub formula: Option<Box<Key>>,
    pub used: bool,
    pub interpretation: Interpretation,
    pub shape: volna_trace::data::SignalShape,
    pub alias: String,
    pub domain: (u64, u64),
}

impl Key {
    #[must_use]
    pub(crate) fn domain(&self) -> (u64, u64) {
        self.domain
    }

    pub fn new(
        doc: &crate::Document,
        inputs: &[Input],
        definition: &Definition,
    ) -> anyhow::Result<Self> {
        Self::prepare(doc, inputs, definition, &mut Vec::new())
    }

    fn prepare(
        doc: &crate::Document,
        inputs: &[Input],
        definition: &Definition,
        stack: &mut Vec<crate::computed::FormulaId>,
    ) -> anyhow::Result<Self> {
        if definition.id.0 == 0 {
            return Self::new_inner(doc, inputs, definition, stack);
        }
        if let Some(start) = stack.iter().position(|id| *id == definition.id) {
            let path = stack[start..]
                .iter()
                .chain(std::iter::once(&definition.id))
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" -> ");
            anyhow::bail!("Formula dependency cycle: {path}.");
        }
        stack.push(definition.id);
        let result = Self::new_inner(doc, inputs, definition, stack);
        stack.pop();
        result
    }

    fn new_inner(
        doc: &crate::Document,
        inputs: &[Input],
        definition: &Definition,
        stack: &mut Vec<crate::computed::FormulaId>,
    ) -> anyhow::Result<Self> {
        use anyhow::Context;
        let inputs = live_inputs(doc, inputs)?;
        let super::Operation::Formula {
            expression,
            settings,
        } = &definition.operation;
        let calculation = doc.formula_programs.parse(
            expression,
            super::input_types(&inputs)?,
            super::input_captures(&inputs)?,
        )?;
        let formula = settings.clone();
        let mut key_inputs = Vec::new();
        for (index, input) in inputs.iter().enumerate() {
            let used = calculation.uses_input(index);
            if !used {
                key_inputs.push(KeyInput {
                    signal: None,
                    formula: None,
                    used,
                    interpretation: input.interpretation,
                    shape: input.shape,
                    alias: input.alias.clone(),
                    domain: doc.limits(),
                });
                continue;
            }
            if let crate::wave::model::RowSource::Computed { inputs, definition } = &input.source {
                let formula = Self::prepare(doc, inputs, definition, stack)?;
                key_inputs.push(KeyInput {
                    signal: None,
                    domain: formula.domain,
                    formula: Some(Box::new(formula)),
                    used,
                    interpretation: input.interpretation,
                    shape: input.shape,
                    alias: input.alias.clone(),
                });
            } else {
                key_inputs.push(KeyInput {
                    signal: Some(input.source.signal().with_context(|| {
                        format!(
                            "Input '{}' is missing or ambiguous; choose a replacement.",
                            input.alias
                        )
                    })?),
                    formula: None,
                    used,
                    interpretation: input.interpretation,
                    shape: input.shape,
                    alias: input.alias.clone(),
                    domain: super::input_domain(std::slice::from_ref(input), doc)?,
                });
            }
        }
        let mut domain = (0, u64::MAX);
        let mut used = false;
        for input in key_inputs.iter().filter(|input| input.used) {
            domain.0 = domain.0.max(input.domain.0);
            domain.1 = domain.1.min(input.domain.1);
            used = true;
        }
        if !used {
            domain = doc.limits();
        }
        anyhow::ensure!(
            domain.0 <= domain.1,
            "Formula inputs have no common time range."
        );
        let scope = resolve_scope(doc, &formula, calculation.kind, domain)?;
        Ok(Self {
            generation: doc.generation(),
            domain,
            inputs: key_inputs,
            calculation,
            formula,
            scope,
            timescale: doc.timescale(),
            time_unit: doc.time_base().unit.map(str::to_owned),
        })
    }

    pub fn signals(&self) -> Box<dyn Iterator<Item = Traced<SignalRef>> + '_> {
        Box::new(
            self.inputs
                .iter()
                .filter(|input| input.used)
                .flat_map(|input| {
                    if let Some(formula) = &input.formula {
                        formula.signals()
                    } else {
                        Box::new(input.signal.into_iter())
                    }
                }),
        )
    }

    pub(crate) fn build(
        &self,
        sources: &[Arc<dyn SignalHistory>],
        budget: &MemoryBudget,
        cancelled: &AtomicBool,
    ) -> anyhow::Result<DerivedOutput> {
        let mut cursor = 0;
        let result = self.build_inner(sources, &mut cursor, budget, cancelled);
        if result.is_ok() {
            anyhow::ensure!(cursor == sources.len(), "Formula source count changed.");
        }
        result
    }

    fn build_inner(
        &self,
        sources: &[Arc<dyn SignalHistory>],
        cursor: &mut usize,
        budget: &MemoryBudget,
        cancelled: &AtomicBool,
    ) -> anyhow::Result<DerivedOutput> {
        let mut resolved = Vec::new();
        for input in &self.inputs {
            if !input.used {
                resolved.push(empty_history(input.shape));
            } else if let Some(formula) = &input.formula {
                match formula.build_inner(sources, cursor, budget, cancelled)? {
                    DerivedOutput::Waveform(history) => resolved.push(history),
                    _ => anyhow::bail!(
                        "Scalar measurement '{}' cannot be a waveform input.",
                        input.alias
                    ),
                }
            } else {
                resolved.push(
                    sources
                        .get(*cursor)
                        .context("Formula source count changed.")?
                        .clone(),
                );
                *cursor += 1;
            }
        }
        let program = &self.calculation;
        let settings = &self.formula;
        let inputs = self
            .inputs
            .iter()
            .zip(&resolved)
            .map(|(binding, source)| super::Source {
                history: source.as_ref(),
                interpretation: binding.interpretation,
                domain: binding.domain,
            })
            .collect::<Vec<_>>();
        let options = super::EvaluationOptions {
            saved_range: settings.saved_range,
            default_step: settings.default_step,
            time_base: crate::wave::timeline::TimeBase {
                timescale: self.timescale,
                unit: self.time_unit.as_deref(),
            },
            interval: None,
            unknown: settings.unknown,
        };
        match &self.scope {
            ResolvedScope::Waveform => {
                match super::evaluate_program(program, &inputs, options, budget, cancelled)? {
                    super::Evaluation::Waveform(history) => {
                        Ok(DerivedOutput::Waveform(Arc::new(history)))
                    }
                    super::Evaluation::Measurement(_) => {
                        anyhow::bail!("Expected a waveform formula.")
                    }
                }
            }
            ResolvedScope::Scalar(interval) => {
                let options = super::EvaluationOptions {
                    interval: *interval,
                    ..options
                };
                match super::evaluate_program(program, &inputs, options, budget, cancelled)? {
                    super::Evaluation::Measurement(measurement) => {
                        Ok(DerivedOutput::Measurement(measurement))
                    }
                    super::Evaluation::Waveform(_) => {
                        anyhow::bail!("Expected a scalar formula.")
                    }
                }
            }
            ResolvedScope::Table(intervals) => {
                let mut evaluator = super::formula::engine::Evaluator::new(
                    program, &inputs, options, budget, cancelled,
                )?;
                let mut rows = Buffer::new(budget)?;
                let mut timeline: Buffer<(u64, Option<f64>)> = Buffer::new(budget)?;
                for interval in intervals {
                    let result = if interval.bounds.0 >= interval.bounds.1 {
                        Err("Coincident markers do not define a positive interval.")
                    } else {
                        Ok(evaluator.measure(interval.bounds)?)
                    };
                    if interval.bounds.0 < interval.bounds.1 {
                        let sample = (
                            interval.bounds.0,
                            result.as_ref().ok().and_then(|value| value.value),
                        );
                        if let Some(last) = timeline.last_mut().filter(|last| last.0 == sample.0) {
                            *last = sample;
                        } else {
                            timeline.push(sample)?;
                        }
                        timeline.push((interval.bounds.1, None))?;
                    }
                    rows.push(MeasurementRow {
                        start: interval.start,
                        end: interval.end,
                        bounds: interval.bounds,
                        result,
                    })?;
                }
                Ok(DerivedOutput::IntervalTable(Arc::new(MeasurementTable {
                    rows,
                    timeline,
                })))
            }
        }
    }
}

fn live_inputs(doc: &crate::Document, inputs: &[Input]) -> anyhow::Result<Vec<Input>> {
    inputs
        .iter()
        .map(|input| {
            let mut input = input.clone();
            let crate::wave::model::RowSource::Computed { definition, .. } = &input.source else {
                return Ok(input);
            };
            if definition.id.0 == 0 {
                return Ok(input);
            }
            let Some(current) = doc.formula_definitions.get(&definition.id) else {
                return Ok(input);
            };
            let Some((inputs, definition)) = current else {
                anyhow::bail!(
                    "Formula {} is ambiguous; choose a replacement.",
                    input.alias
                );
            };
            let shape = definition.validate(inputs)?;
            input.source = crate::wave::model::RowSource::Computed {
                inputs: inputs.clone(),
                definition: definition.clone(),
            };
            input.shape = shape;
            input.interpretation.projection = None;
            input.interpretation.kind = if shape == volna_trace::data::SignalShape::Real {
                crate::data::NumericKind::Real
            } else {
                crate::data::NumericKind::Unsigned
            };
            Ok(input)
        })
        .collect()
}

fn empty_history(shape: volna_trace::data::SignalShape) -> Arc<dyn SignalHistory> {
    Arc::new(volna_trace::data::history::VecHistory {
        shape,
        times: Vec::new(),
        values: Vec::new(),
        initial: volna_trace::data::WaveValue::Unavailable,
    })
}

fn resolve_scope(
    doc: &crate::Document,
    settings: &super::FormulaSettings,
    kind: ResultKind,
    domain: (u64, u64),
) -> anyhow::Result<ResolvedScope> {
    if kind != ResultKind::ScalarMeasurement {
        anyhow::ensure!(
            matches!(
                settings.scope,
                MeasurementScope::WholeRange | MeasurementScope::MarkerIntervals
            ),
            "Waveform formulas use Whole range scope."
        );
        return Ok(ResolvedScope::Waveform);
    }
    let single = |bounds: (u64, u64)| {
        anyhow::ensure!(
            bounds.0 < bounds.1,
            "Measurement scope must have positive duration."
        );
        anyhow::ensure!(
            bounds.0 >= domain.0 && bounds.1 <= domain.1,
            "Measurement bounds must be inside the resolved source domain."
        );
        Ok(ResolvedScope::Scalar(Some(bounds)))
    };
    match &settings.scope {
        MeasurementScope::IntervalSnapshot { intervals } => {
            let mut rows = Vec::with_capacity(intervals.len());
            for &bounds in intervals {
                single(bounds)?;
                anyhow::ensure!(
                    rows.last()
                        .is_none_or(|previous: &TableInterval| previous.bounds.1 <= bounds.0),
                    "Saved intervals must be ordered and non-overlapping."
                );
                rows.push(TableInterval {
                    start: None,
                    end: None,
                    bounds,
                });
            }
            Ok(ResolvedScope::Table(rows))
        }
        MeasurementScope::MarkerIntervals => {
            let mut boundaries = vec![(domain.0, None)];
            boundaries.extend(
                doc.markers()
                    .iter()
                    .filter(|marker| marker.time > domain.0 && marker.time < domain.1)
                    .map(|marker| (marker.time, Some(marker.id))),
            );
            boundaries.sort_by_key(|(time, _)| *time);
            boundaries.dedup_by_key(|(time, _)| *time);
            boundaries.push((domain.1, None));
            if boundaries.len() == 2 {
                return Ok(ResolvedScope::Scalar(None));
            }
            Ok(ResolvedScope::Table(
                boundaries
                    .windows(2)
                    .map(|pair| TableInterval {
                        start: pair[0].1,
                        end: pair[1].1,
                        bounds: (pair[0].0, pair[1].0),
                    })
                    .collect(),
            ))
        }
        MeasurementScope::WholeRange => Ok(ResolvedScope::Scalar(None)),
        MeasurementScope::Explicit { start, end }
        | MeasurementScope::MarkerSnapshot { start, end } => single((*start, *end)),
        MeasurementScope::LinkedMarkers { start, end } => {
            let marker = |id| {
                doc.markers()
                    .iter()
                    .find(|marker| marker.id == id)
                    .map(|marker| marker.time)
                    .with_context(|| {
                        format!("Marker {id} is missing; repair the measurement scope.")
                    })
            };
            let (start, end) = (marker(*start)?, marker(*end)?);
            single((start.min(end), start.max(end)))
        }
        MeasurementScope::MarkerSet { markers } => {
            let mut markers = markers
                .iter()
                .map(|id| {
                    doc.markers()
                        .iter()
                        .find(|marker| &marker.id == id)
                        .cloned()
                        .with_context(|| format!("Marker {id} is missing; repair the marker set."))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            markers.sort_by_key(|marker| marker.time);
            Ok(ResolvedScope::Table(
                markers
                    .windows(2)
                    .map(|pair| TableInterval {
                        start: Some(pair[0].id),
                        end: Some(pair[1].id),
                        bounds: (pair[0].time, pair[1].time),
                    })
                    .collect(),
            ))
        }
    }
}

pub enum Status {
    Waiting,
    Building,
    Ready(Arc<dyn SignalHistory>),
    ReadyMeasurement(super::Measurement),
    ReadyTable(Arc<MeasurementTable>),
    Failed(String),
}

struct Job {
    request: u64,
    cancelled: Arc<AtomicBool>,
    status: Status,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub(crate) struct Computations {
    jobs: HashMap<Key, Job>,
    sources: HashMap<Traced<SignalRef>, Result<Arc<dyn SignalHistory>, String>>,
    next_request: u64,
    generation: Option<u64>,
    wanted_sources: HashSet<Traced<SignalRef>>,
}

impl Computations {
    pub fn clear(&mut self) {
        self.jobs.clear();
        self.sources.clear();
        self.wanted_sources.clear();
    }

    pub fn retain(&mut self, wanted: &HashSet<Key>, sources: &HashSet<Traced<SignalRef>>) {
        self.wanted_sources.clone_from(sources);
        self.jobs.retain(|key, _| wanted.contains(key));
        self.sources
            .retain(|signal, _| self.wanted_sources.contains(signal));
    }

    pub fn needs_source(&self, signal: Traced<SignalRef>) -> bool {
        !self.sources.contains_key(&signal)
    }

    pub fn status(&self, key: &Key) -> Option<&Status> {
        self.jobs.get(key).map(|j| &j.status)
    }

    pub fn source(&self, signal: Traced<SignalRef>) -> Option<&Arc<dyn SignalHistory>> {
        self.sources.get(&signal)?.as_ref().ok()
    }

    pub fn deliver_source(
        &mut self,
        signal: Traced<SignalRef>,
        result: Result<Arc<dyn SignalHistory>, String>,
    ) {
        if self.wanted_sources.contains(&signal) {
            self.sources.insert(signal, result);
        }
    }

    pub fn retry(&mut self) {
        self.sources.retain(|_, s| s.is_ok());
        self.jobs
            .retain(|_, j| !matches!(j.status, Status::Failed(_)));
    }

    pub fn sync(
        &mut self,
        wanted: HashSet<Key>,
        source_demand: &HashSet<Traced<SignalRef>>,
        resident: &crate::wave::model::Resident,
        generation: u64,
        budget: &MemoryBudget,
    ) -> Vec<LoadRequest> {
        if self.generation != Some(generation) {
            self.clear();
            self.generation = Some(generation);
        }
        self.retain(&wanted, source_demand);
        let mut requests = Vec::new();
        for signal in source_demand {
            if let Some(source) = resident.get(signal) {
                self.sources.insert(*signal, Ok(source.clone()));
            }
        }
        for key in wanted {
            let job = self.jobs.entry(key.clone()).or_insert_with(|| Job {
                request: 0,
                cancelled: Arc::new(AtomicBool::new(false)),
                status: Status::Waiting,
            });
            if !matches!(job.status, Status::Waiting) {
                continue;
            }
            let sources = key
                .signals()
                .map(|signal| self.sources.get(&signal))
                .collect::<Vec<_>>();
            if let Some(error) = sources.iter().find_map(|s| match s {
                Some(Err(e)) => Some(e.clone()),
                _ => None,
            }) {
                job.status = Status::Failed(error);
            } else if let Some(sources) = sources
                .into_iter()
                .map(|s| s?.as_ref().ok().cloned())
                .collect::<Option<Vec<_>>>()
            {
                self.next_request = self
                    .next_request
                    .checked_add(1)
                    .expect("computed-signal request identities exhausted");
                job.request = self.next_request;
                job.status = Status::Building;
                requests.push(LoadRequest::Derived {
                    key,
                    request: job.request,
                    sources,
                    budget: budget.clone(),
                    cancelled: job.cancelled.clone(),
                });
            }
        }
        requests
    }

    pub fn deliver(
        &mut self,
        key: Key,
        request: u64,
        result: anyhow::Result<DerivedOutput>,
    ) -> bool {
        let Some(job) = self.jobs.get_mut(&key) else {
            return false;
        };
        if job.request != request || !matches!(job.status, Status::Building) {
            return false;
        }
        job.status = match result {
            Ok(DerivedOutput::Waveform(history)) => Status::Ready(history),
            Ok(DerivedOutput::Measurement(measurement)) => Status::ReadyMeasurement(measurement),
            Ok(DerivedOutput::IntervalTable(rows)) => Status::ReadyTable(rows),
            Err(error) => Status::Failed(format!("{error:#}")),
        };
        true
    }
}

pub use Status as ComputationStatus;
