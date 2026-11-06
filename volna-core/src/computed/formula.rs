//! Typed waveform expressions, event captures, generators, and interval measurements.

use anyhow::{Result, ensure};

mod cache;
pub mod engine;
pub(super) mod evaluate;
pub mod language;
mod parser;
pub(crate) use cache::Programs;

pub const MAX_INPUTS: usize = 16;
const MAX_NODES: usize = 256;
const MAX_DEPTH: usize = 32;
pub const MAX_EXPRESSION: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Real,
    Boolean,
}

impl Type {
    #[must_use]
    pub fn shape(self) -> volna_trace::data::SignalShape {
        match self {
            Self::Real => volna_trace::data::SignalShape::Real,
            Self::Boolean => volna_trace::data::SignalShape::Bit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResultKind {
    Waveform,
    CapturedWaveform,
    ScalarMeasurement,
}

#[derive(Clone, Copy, Debug)]
pub enum Value {
    Real(Option<f64>),
    Boolean(Option<bool>),
}

impl Value {
    pub fn same(self, other: Self) -> bool {
        match (self, other) {
            (Self::Real(a), Self::Real(b)) => a.map(f64::to_bits) == b.map(f64::to_bits),
            (Self::Boolean(a), Self::Boolean(b)) => a == b,
            _ => false,
        }
    }

    pub(crate) fn real(self) -> Option<f64> {
        match self {
            Self::Real(v) => v,
            Self::Boolean(v) => v.map(|b| f64::from(u8::from(b))),
        }
    }

    pub(crate) fn boolean(self) -> Option<bool> {
        match self {
            Self::Boolean(v) => v,
            Self::Real(v) => v.map(|v| v != 0.0),
        }
    }

    pub(crate) fn finite(v: Option<f64>) -> Self {
        Self::Real(v.filter(|v| v.is_finite()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Unary {
    Negate,
    Positive,
    Not,
    Abs,
    Real,
    Boolean,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
    And,
    Or,
    Min,
    Max,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum DurationUnit {
    Si(i32),
    Ticks,
    Logical(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Duration {
    digits: u128,
    scale: i32,
    unit: DurationUnit,
}

impl Duration {
    fn ticks(&self, base: crate::wave::timeline::TimeBase<'_>) -> Result<u64> {
        let exponent = match (&self.unit, base.unit) {
            (DurationUnit::Ticks, _) => 0,
            (DurationUnit::Logical(unit), Some(expected)) if unit == expected => 0,
            (DurationUnit::Logical(_), Some(_)) => {
                anyhow::bail!("Use timeline ticks or the recording's logical time unit.")
            }
            (DurationUnit::Si(unit), None) => unit - i32::from(base.timescale),
            (DurationUnit::Si(_), Some(_)) => {
                anyhow::bail!("SI durations require a physical timeline; use logical ticks.")
            }
            (DurationUnit::Logical(unit), None) => {
                anyhow::bail!("Unknown time unit '{unit}'; use SI units or timeline ticks.")
            }
        };
        let scale = self
            .scale
            .checked_add(exponent)
            .context("Duration is too large.")?;
        let value = if scale >= 0 {
            self.digits
                .checked_mul(
                    10_u128
                        .checked_pow(scale as u32)
                        .context("Duration is too large.")?,
                )
                .context("Duration is too large.")?
        } else {
            let divisor = 10_u128
                .checked_pow(scale.unsigned_abs())
                .context("Duration is too large.")?;
            ensure!(
                self.digits.is_multiple_of(divisor),
                "Duration must be a whole number of timeline ticks."
            );
            self.digits / divisor
        };
        u64::try_from(value).context("Duration is too large.")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Warmup {
    Full,
    Available,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownPolicy {
    Gap,
    Skip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Reduction {
    Mean,
    Integral,
    Min,
    Max,
    Sum,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Trigger {
    Periodic(Duration),
    Posedge(usize),
    Changes(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Node {
    Number(u64),
    Boolean(bool),
    Input(usize),
    Unary(Unary, usize),
    Binary(Binary, usize, usize),
    MovingAverage {
        x: usize,
        window: Duration,
        step: Option<Duration>,
        warmup: Warmup,
        unknown: UnknownPolicy,
    },
    Lowpass {
        x: usize,
        tau: Duration,
        step: Option<Duration>,
    },
    Integrate {
        x: usize,
        initial: usize,
        step: Option<Duration>,
    },
    Sine {
        period: Duration,
        amplitude: usize,
        offset: usize,
        phase: usize,
        step: Option<Duration>,
    },
    Clock {
        period: Duration,
        high: Duration,
        phase: Duration,
    },
    Capture {
        x: usize,
        trigger: Trigger,
    },
    Reduce {
        reduction: Reduction,
        x: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum NodeKind {
    Literal,
    Waveform { captured: bool },
    Scalar,
}

impl NodeKind {
    fn pointwise(self) -> Self {
        match self {
            Self::Literal => Self::Literal,
            Self::Waveform { .. } => Self::Waveform { captured: false },
            Self::Scalar => Self::Scalar,
        }
    }

    fn is_waveform_or_literal(self) -> bool {
        !matches!(self, Self::Scalar)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TypedNode {
    node: Node,
    ty: Type,
    kind: NodeKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Program {
    nodes: Vec<TypedNode>,
    pub output: Type,
    pub kind: ResultKind,
    used_inputs: Vec<bool>,
}

impl Program {
    pub fn with_bindings(expression: &str, inputs: &[super::Input]) -> Result<Self> {
        Self::parse_captured(
            expression,
            &super::input_types(inputs)?,
            &super::input_captures(inputs)?,
        )
    }

    pub fn parse(expression: &str, inputs: &[(String, Type)]) -> Result<Self> {
        Self::parse_captured(expression, inputs, &vec![false; inputs.len()])
    }

    pub fn parse_captured(
        expression: &str,
        inputs: &[(String, Type)],
        captured: &[bool],
    ) -> Result<Self> {
        ensure!(
            captured.len() == inputs.len(),
            "Formula input metadata changed."
        );
        ensure!(
            expression.len() <= MAX_EXPRESSION,
            "Formula is limited to {MAX_EXPRESSION} bytes."
        );
        ensure!(!expression.trim().is_empty(), "Enter a formula.");
        ensure!(
            inputs.len() <= MAX_INPUTS,
            "A formula supports at most {MAX_INPUTS} inputs."
        );
        for (i, (alias, _)) in inputs.iter().enumerate() {
            ensure!(
                valid_alias(alias),
                "Use an input alias starting with a letter or underscore, followed by letters, digits, or underscores (maximum 64 bytes)."
            );
            ensure!(
                !inputs[..i].iter().any(|(other, _)| other == alias),
                "Input alias '{alias}' is used more than once."
            );
        }
        parser::parse(expression, inputs, captured)
    }

    #[must_use]
    pub fn uses_input(&self, index: usize) -> bool {
        self.used_inputs.get(index).copied().unwrap_or(false)
    }

    #[must_use]
    pub fn uses_default_step(&self) -> bool {
        self.nodes.iter().any(|node| {
            matches!(
                node.node,
                Node::MovingAverage { step: None, .. }
                    | Node::Lowpass { step: None, .. }
                    | Node::Integrate { step: None, .. }
                    | Node::Sine { step: None, .. }
            )
        })
    }

    pub fn used_inputs(&self) -> impl Iterator<Item = usize> + '_ {
        self.used_inputs
            .iter()
            .enumerate()
            .filter_map(|(i, used)| used.then_some(i))
    }
}

#[must_use]
pub fn valid_alias(alias: &str) -> bool {
    alias.len() <= 64
        && !parser::reserved(alias)
        && alias
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && alias
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

use anyhow::Context;
