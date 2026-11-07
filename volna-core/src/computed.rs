//! Row-owned waveform and measurement computations, including applied-formula composition.

pub(crate) mod formula;
mod sampling;
pub(crate) use sampling::sampling_only_aliases;
pub use sampling::{capture_expression, input_change_expression};
mod search;
pub use search::{RecordedMatch, RecordedSearch};
mod jobs;
pub use crate::data::numeric_input::Interpretation;
pub use formula::engine::{
    CaptureSupport, Evaluation, EvaluationOptions, FormulaHistory, Measurement, MeasurementStatus,
    Source, evaluate_program,
};
pub use formula::language;
pub use formula::{Program, ResultKind, Type as FormulaType, UnknownPolicy};
pub(crate) use jobs::Computations;
pub use jobs::{
    ComputationStatus, DerivedOutput, Key, MeasurementResult, MeasurementRow, MeasurementTable,
};

use crate::{
    moving_average::duration,
    trace::{Retime, Traced},
    wave::model::RowSource,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt::Write, sync::Arc};
use volna_trace::data::{SignalShape, VarId};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Input {
    pub alias: String,
    pub source: RowSource,
    pub shape: SignalShape,
    pub interpretation: Interpretation,
}

impl Input {
    pub(crate) fn ty(&self) -> formula::Type {
        if self
            .interpretation
            .projection
            .map_or(self.shape, crate::data::projection::BitSlice::shape)
            == SignalShape::Bit
        {
            formula::Type::Boolean
        } else {
            formula::Type::Real
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let RowSource::Computed { inputs, definition } = &self.source {
            ensure!(
                definition.calculation(inputs)?.kind != ResultKind::ScalarMeasurement,
                "Scalar measurements cannot be formula inputs."
            );
        }
        ensure!(
            formula::valid_alias(&self.alias),
            "Use a unique input alias with letters, digits, and underscores (starting with a letter or underscore; maximum 64 bytes)."
        );
        self.interpretation.validate(self.shape)
    }
    #[must_use]
    pub fn label(&self, doc: &crate::Document) -> String {
        let (mut path, nth) = self.source.locator(doc);
        if let Some(slice) = self.interpretation.projection
            && let Some(name) = path.last_mut()
        {
            *name = crate::wave::bus::BusIndices::new(name, self.shape.width()).label(slice);
        }
        let mut label = format!("{}: {}", self.source.trace(), path.join("."));
        if let Some(nth) = nth {
            write!(label, " (occurrence {})", nth + 1).expect("write to String");
        }
        label
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeasurementScope {
    #[default]
    MarkerIntervals,
    IntervalSnapshot {
        intervals: Vec<(u64, u64)>,
    },
    WholeRange,
    Explicit {
        start: u64,
        end: u64,
    },
    MarkerSnapshot {
        start: u64,
        end: u64,
    },
    LinkedMarkers {
        start: crate::marker::MarkerId,
        end: crate::marker::MarkerId,
    },
    MarkerSet {
        markers: Vec<crate::marker::MarkerId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FormulaSettings {
    pub default_step: u64,
    pub saved_range: (u64, u64),
    pub scope: MeasurementScope,
    pub unknown: UnknownPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    Formula {
        expression: String,
        settings: FormulaSettings,
    },
}
impl Retime for Operation {
    fn retime(&mut self, by: crate::trace::Rescale) {
        match self {
            Self::Formula { settings, .. } => {
                settings.default_step = by.time(settings.default_step).max(1);
                settings.saved_range.0 = by.time(settings.saved_range.0);
                settings.saved_range.1 = by.time(settings.saved_range.1);
                match &mut settings.scope {
                    MeasurementScope::IntervalSnapshot { intervals } => {
                        for (start, end) in intervals {
                            *start = by.time(*start);
                            *end = by.time(*end);
                        }
                    }
                    MeasurementScope::Explicit { start, end }
                    | MeasurementScope::MarkerSnapshot { start, end } => {
                        *start = by.time(*start);
                        *end = by.time(*end);
                    }
                    _ => {}
                }
            }
        }
    }
}
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct FormulaId(pub u64);

impl std::fmt::Display for FormulaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Definition {
    pub id: FormulaId,
    pub revision: u64,
    pub name: String,
    pub operation: Operation,
}
impl Definition {
    pub fn validate(&self, inputs: &[Input]) -> Result<SignalShape> {
        self.validate_program(inputs)
            .map(|program| program.output.shape())
    }
    fn validate_program(&self, inputs: &[Input]) -> Result<Arc<formula::Program>> {
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 256
                && !self.name.chars().any(char::is_control),
            "Use a name of 1–256 bytes without control characters."
        );
        self.calculation(inputs)
    }
    pub(crate) fn calculation(&self, inputs: &[Input]) -> Result<Arc<formula::Program>> {
        let Operation::Formula { expression, .. } = &self.operation;
        Program::with_bindings(expression, inputs).map(Arc::new)
    }
}

fn input_types(inputs: &[Input]) -> Result<Vec<(String, formula::Type)>> {
    ensure!(
        inputs.len() <= formula::MAX_INPUTS,
        "Use at most 16 recorded or applied waveform inputs."
    );
    inputs
        .iter()
        .map(|input| {
            input.validate()?;
            Ok((input.alias.clone(), input.ty()))
        })
        .collect()
}

fn input_captures(inputs: &[Input]) -> Result<Vec<bool>> {
    inputs
        .iter()
        .map(|input| {
            Ok(match &input.source {
                RowSource::Computed { inputs, definition } => {
                    definition.calculation(inputs)?.kind == ResultKind::CapturedWaveform
                }
                _ => false,
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub name: String,
    pub interval: String,
    pub skip_unknown: bool,
    pub formula: String,
    pub scope: MeasurementScope,
}
impl Draft {
    pub fn definition(
        &self,
        saved_range: (u64, u64),
        base: crate::wave::timeline::TimeBase<'_>,
        inputs: &[Input],
    ) -> Result<Definition> {
        let operation = Operation::Formula {
            expression: self.formula.clone(),
            settings: FormulaSettings {
                default_step: duration(&self.interval, base).context("Default step")?,
                saved_range,
                scope: self.scope.clone(),
                unknown: if self.skip_unknown {
                    UnknownPolicy::Skip
                } else {
                    UnknownPolicy::Gap
                },
            },
        };
        let mut definition = Definition {
            id: FormulaId::default(),
            revision: 0,
            name: self.name.trim().into(),
            operation,
        };
        let program = definition.validate_program(inputs)?;
        if program.kind != ResultKind::ScalarMeasurement {
            let Operation::Formula { settings, .. } = &mut definition.operation;
            // Retain the measurement scope in the draft for switching back to a measurement.
            settings.scope = MeasurementScope::WholeRange;
        }
        Ok(definition)
    }
    #[must_use]
    pub fn calculation_changed(&self, other: &Self) -> bool {
        self.formula != other.formula
            || self.interval != other.interval
            || self.skip_unknown != other.skip_unknown
            || self.scope != other.scope
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Open(Option<usize>),
    SampleOnInputChanges,
    Sample {
        var: Traced<VarId>,
        mode: crate::sampling::SamplingMode,
        make_default: bool,
    },
    Update(Draft),
    AddInput(Traced<VarId>),
    Rebind(usize, Traced<VarId>),
    RemoveInput(usize),
    SetInput {
        index: usize,
        alias: String,
        kind: crate::data::NumericKind,
        slice: String,
    },
    Preview,
    Stop,
    Apply,
    ApplyTable,
    Remove,
    Close,
}
pub struct Editor {
    pub id: u64,
    pub panel: crate::panels::PanelId,
    pub row: usize,
    pub revision: u64,
    pub definition_id: FormulaId,
    pub definition_revision: u64,
    pub bindings: Vec<InputDraft>,
    pub editing: bool,
    pub draft: Draft,
    pub domain: (u64, u64),
    pub preview: Option<Key>,
    pub calculation_revision: u64,
    pub error: Option<String>,
    pub stopped: bool,
    pub(crate) display_definition: Definition,
    pub(crate) display_inputs: Vec<Input>,
    pub(crate) last_result: Option<DerivedOutput>,
    pub(crate) reveal_preview: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDraft {
    pub alias: String,
    pub source: RowSource,
    pub shape: SignalShape,
    pub kind: crate::data::NumericKind,
    pub slice: String,
}
impl InputDraft {
    #[must_use]
    pub fn new(input: Input, doc: &crate::Document) -> Self {
        let name = input
            .source
            .locator(doc)
            .0
            .last()
            .cloned()
            .unwrap_or_default();
        let indices = crate::wave::bus::BusIndices::new(&name, input.shape.width());
        let slice = input
            .interpretation
            .projection
            .map(|p| format!("{}:{}", indices.index(p.first), indices.index(p.last)))
            .unwrap_or_default();
        Self {
            alias: input.alias,
            source: input.source,
            shape: input.shape,
            kind: input.interpretation.kind,
            slice,
        }
    }
    fn projection(
        &self,
        doc: &crate::Document,
    ) -> Result<Option<crate::data::projection::BitSlice>> {
        if self.slice.trim().is_empty() {
            return Ok(None);
        }
        let name = self
            .source
            .locator(doc)
            .0
            .last()
            .cloned()
            .unwrap_or_default();
        crate::wave::bus::BusIndices::new(&name, self.shape.width())
            .parse(&self.slice)
            .map(Some)
            .with_context(|| format!("Input '{}' slice", self.alias))
    }
    pub fn resolve(&self, doc: &crate::Document) -> Result<Input> {
        let projection = self.projection(doc)?;
        let kind = if projection.is_some_and(|p| p.shape() == SignalShape::Bit) {
            crate::data::NumericKind::Unsigned
        } else {
            self.kind
        };
        let input = Input {
            alias: self.alias.clone(),
            source: self.source.clone(),
            shape: self.shape,
            interpretation: Interpretation { kind, projection },
        };
        input.validate()?;
        Ok(input)
    }
    #[must_use]
    pub fn effective_shape(&self, doc: &crate::Document) -> SignalShape {
        self.projection(doc)
            .ok()
            .flatten()
            .map_or(self.shape, crate::data::projection::BitSlice::shape)
    }
    #[must_use]
    pub fn label(&self, doc: &crate::Document) -> String {
        Input {
            alias: self.alias.clone(),
            source: self.source.clone(),
            shape: self.shape,
            interpretation: Interpretation {
                kind: self.kind,
                projection: self.projection(doc).ok().flatten(),
            },
        }
        .label(doc)
    }
}
impl Editor {
    pub fn inputs(&self, doc: &crate::Document) -> Result<Vec<Input>> {
        self.bindings.iter().map(|b| b.resolve(doc)).collect()
    }

    pub fn definition(
        &self,
        saved_range: (u64, u64),
        base: crate::wave::timeline::TimeBase<'_>,
        inputs: &[Input],
    ) -> Result<Definition> {
        let mut definition = self.draft.definition(saved_range, base, inputs)?;
        definition.id = self.definition_id;
        definition.revision = self.definition_revision;
        Ok(definition)
    }
}
pub(crate) fn input_domain(inputs: &[Input], doc: &crate::Document) -> Result<(u64, u64)> {
    if inputs.is_empty() {
        return Ok(doc.limits());
    }
    let mut domain = (0, u64::MAX);
    for input in inputs {
        let range = doc
            .traces()
            .get(input.source.trace())
            .and_then(crate::trace::TraceSlot::range)
            .context("Input recording is unavailable.")?;
        domain.0 = domain.0.max(range.0);
        domain.1 = domain.1.min(range.1);
    }
    ensure!(
        domain.0 <= domain.1,
        "The input recordings have no common time range."
    );
    Ok(domain)
}
