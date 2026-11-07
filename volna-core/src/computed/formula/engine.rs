//! Event-accurate formula evaluation with bounded work and admitted evaluation storage.

use super::{
    Binary, Node, NodeKind, Program, Reduction, ResultKind, Trigger, Type, Unary, UnknownPolicy,
    Value, Warmup, evaluate,
};
use crate::computed::Interpretation;
use crate::data::{admission::Buffer, numeric_input::InputHistory};
use anyhow::{Context, Result, bail, ensure};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use volna_trace::{
    data::{Bit, SignalHistory, SignalShape, WaveValue},
    remote::memory::MemoryBudget,
};

const MAX_EVENTS: usize = 1_000_000;
const MAX_WORK: usize = 10_000_000;

#[derive(Clone, Copy, Debug)]
struct Event {
    time: u64,
    value: Value,
}

#[derive(Debug)]
struct Waveform {
    domain: (u64, u64),
    ty: Type,
    events: Buffer<Event>,
    captures: Buffer<u64>,
}

impl Waveform {
    fn value_at(&self, time: u64) -> Value {
        let unknown = || match self.ty {
            Type::Real => Value::Real(None),
            Type::Boolean => Value::Boolean(None),
        };
        if time < self.domain.0 || time > self.domain.1 {
            return unknown();
        }
        let index = self.events.partition_point(|event| event.time <= time);
        index
            .checked_sub(1)
            .map_or_else(unknown, |index| self.events[index].value)
    }

    fn captured(&self) -> bool {
        !self.captures.is_empty() || self.events.len() == self.captures.len()
    }
}

fn push(events: &mut Buffer<Event>, time: u64, value: Value, captured: bool) -> Result<()> {
    if let Some(last) = events.last_mut()
        && last.time == time
    {
        last.value = value;
        return Ok(());
    }
    if !captured && events.last().is_some_and(|event| event.value.same(value)) {
        return Ok(());
    }
    ensure!(
        events.len() < MAX_EVENTS,
        "Formula exceeds 1,000,000 output events at one expression node."
    );
    events.push(Event { time, value })?;
    Ok(())
}

pub struct Source<'a> {
    pub history: &'a dyn SignalHistory,
    pub interpretation: Interpretation,
    pub domain: (u64, u64),
}

impl Source<'_> {
    fn ty(&self) -> Type {
        if self.interpretation.projection.map_or(
            self.history.shape(),
            crate::data::projection::BitSlice::shape,
        ) == SignalShape::Bit
        {
            Type::Boolean
        } else {
            Type::Real
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EvaluationOptions<'a> {
    pub saved_range: (u64, u64),
    pub default_step: u64,
    pub time_base: crate::wave::timeline::TimeBase<'a>,
    pub interval: Option<(u64, u64)>,
    pub unknown: UnknownPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasurementStatus {
    Complete,
    UnknownData,
    NoKnownData,
    NonFinite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureSupport {
    pub known: usize,
    pub total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    pub value: Option<f64>,
    pub bounds: (u64, u64),
    pub requested_ticks: u64,
    pub known_ticks: u64,
    pub captures: Option<CaptureSupport>,
    pub policy: UnknownPolicy,
    pub status: MeasurementStatus,
}

impl Measurement {
    #[must_use]
    pub fn coverage(self) -> f64 {
        match self.captures {
            Some(captures) if captures.total > 0 => captures.known as f64 / captures.total as f64,
            Some(_) => 1.0,
            None if self.requested_ticks > 0 => {
                self.known_ticks as f64 / self.requested_ticks as f64
            }
            None => 0.0,
        }
    }
}

#[derive(Debug)]
pub enum Evaluation {
    Waveform(FormulaHistory),
    Measurement(Measurement),
}

pub fn evaluate_program(
    program: &Program,
    sources: &[Source<'_>],
    options: EvaluationOptions<'_>,
    budget: &MemoryBudget,
    cancelled: &AtomicBool,
) -> Result<Evaluation> {
    let mut engine = Evaluator::new(program, sources, options, budget, cancelled)?;
    let root = program.nodes.len() - 1;
    if program.kind == ResultKind::ScalarMeasurement {
        let scalar = engine.scalar(root)?;
        return Ok(Evaluation::Measurement(scalar));
    }
    let waveform = engine.waveform(root)?;
    let coverage = engine.coverages[root].take();
    let Evaluator {
        warnings,
        waveforms,
        domains,
        domain_ready,
        coverages,
        ..
    } = engine;
    drop((waveforms, domains, domain_ready, coverages));
    let waveform = Arc::try_unwrap(waveform)
        .map_err(|_| anyhow::anyhow!("Formula waveform is still in use."))?;
    Ok(Evaluation::Waveform(FormulaHistory::from_waveform(
        waveform, warnings, coverage, cancelled,
    )?))
}

pub(crate) struct Evaluator<'a> {
    program: &'a Program,
    sources: &'a [Source<'a>],
    options: EvaluationOptions<'a>,
    budget: &'a MemoryBudget,
    work: Work<'a>,
    waveforms: Buffer<Option<Arc<Waveform>>>,
    domains: Buffer<Option<(u64, u64)>>,
    domain_ready: Buffer<bool>,
    coverages: Buffer<Option<Buffer<(u64, f64)>>>,
    warnings: Buffer<String>,
}

impl<'a> Evaluator<'a> {
    pub(crate) fn new(
        program: &'a Program,
        sources: &'a [Source<'a>],
        options: EvaluationOptions<'a>,
        budget: &'a MemoryBudget,
        cancelled: &'a AtomicBool,
    ) -> Result<Self> {
        ensure!(
            sources.len() == program.used_inputs.len(),
            "Formula input count changed."
        );
        ensure!(
            options.saved_range.0 <= options.saved_range.1,
            "Saved range is reversed."
        );
        ensure!(options.default_step > 0, "Default step must be positive.");
        Ok(Self {
            program,
            sources,
            options,
            budget,
            work: Work {
                cancelled,
                steps: 0,
            },
            waveforms: Buffer::filled(budget, program.nodes.len(), || None)?,
            domains: Buffer::filled(budget, program.nodes.len(), || None)?,
            domain_ready: Buffer::filled(budget, program.nodes.len(), || false)?,
            coverages: Buffer::filled(budget, program.nodes.len(), || None)?,
            warnings: Buffer::new(budget)?,
        })
    }

    pub(crate) fn measure(&mut self, bounds: (u64, u64)) -> Result<Measurement> {
        ensure!(
            self.program.kind == ResultKind::ScalarMeasurement,
            "Expected a scalar formula."
        );
        self.options.interval = Some(bounds);
        self.scalar(self.program.nodes.len() - 1)
    }

    fn domain(&mut self, index: usize) -> Result<Option<(u64, u64)>> {
        if self.domain_ready[index] {
            return Ok(self.domains[index]);
        }
        let node = self.program.nodes[index].node.clone();
        let domain = match node {
            Node::Input(input) => Some(self.sources[input].domain),
            Node::Number(_) | Node::Boolean(_) | Node::Sine { .. } | Node::Clock { .. } => None,
            Node::Unary(_, child)
            | Node::MovingAverage { x: child, .. }
            | Node::Lowpass { x: child, .. }
            | Node::Integrate { x: child, .. }
            | Node::Reduce { x: child, .. } => self.domain(child)?,
            Node::Binary(_, left, right) => self.intersect_domains(left, right)?,
            Node::Capture {
                x,
                trigger: Trigger::Periodic(_),
            } => self.domain(x)?,
            Node::Capture { x, trigger } => {
                let clock = match trigger {
                    Trigger::Posedge(clock) | Trigger::Changes(clock) => clock,
                    Trigger::Periodic(_) => unreachable!(),
                };
                self.intersect_domains(x, clock)?
            }
        };
        if let Some((start, end)) = domain {
            ensure!(
                start <= end,
                "The input recordings have no common time range."
            );
        }
        self.domains[index] = domain;
        self.domain_ready[index] = true;
        Ok(domain)
    }

    fn intersect_domains(&mut self, left: usize, right: usize) -> Result<Option<(u64, u64)>> {
        let left = self.domain(left)?;
        let right = self.domain(right)?;
        Ok(match (left, right) {
            (Some(left), Some(right)) => Some((left.0.max(right.0), left.1.min(right.1))),
            (Some(domain), None) | (None, Some(domain)) => Some(domain),
            (None, None) => None,
        })
    }

    fn resolved_domain(&mut self, index: usize) -> Result<(u64, u64)> {
        Ok(self.domain(index)?.unwrap_or(self.options.saved_range))
    }

    fn step(&mut self, step: Option<&super::Duration>) -> Result<u64> {
        let step = step.map_or(Ok(self.options.default_step), |step| {
            step.ticks(self.options.time_base)
        })?;
        ensure!(
            step > 0,
            "Sampling steps must be positive whole timeline ticks."
        );
        Ok(step)
    }

    fn constant(&self, index: usize) -> Result<f64> {
        evaluate::constant(&self.program.nodes, index)
    }

    fn constant_waveform(&self, index: usize, domain: (u64, u64)) -> Result<Arc<Waveform>> {
        let node = &self.program.nodes[index];
        let value = match node.node {
            Node::Number(value) => Value::Real(Some(f64::from_bits(value))),
            Node::Boolean(value) => Value::Boolean(Some(value)),
            Node::Unary(_, _) | Node::Binary(_, _, _) => {
                evaluate::constant_value(&self.program.nodes, index)?
            }
            _ => bail!("Internal error: non-literal used as a constant waveform."),
        };
        Ok(Arc::new(Waveform {
            domain,
            ty: node.ty,
            events: Buffer::filled(self.budget, 1, || Event {
                time: domain.0,
                value,
            })?,
            captures: Buffer::new(self.budget)?,
        }))
    }

    fn waveform(&mut self, index: usize) -> Result<Arc<Waveform>> {
        if let Some(waveform) = &self.waveforms[index] {
            return Ok(waveform.clone());
        }
        self.work.spend(1)?;
        let domain = self.resolved_domain(index)?;
        let typed = self.program.nodes[index].clone();
        let waveform = match typed.node {
            Node::Number(_) | Node::Boolean(_) => self.constant_waveform(index, domain)?,
            Node::Unary(_, _) | Node::Binary(_, _, _) if typed.kind == NodeKind::Literal => {
                self.constant_waveform(index, domain)?
            }
            Node::Input(input) => self.input(input, domain)?,
            Node::Unary(operator, child) => {
                let child = self.waveform(child)?;
                self.pointwise_unary(operator, child, domain)?
            }
            Node::Binary(operator, left, right) => {
                let left = self.operand(left, domain)?;
                let right = self.operand(right, domain)?;
                self.pointwise_binary(operator, left, right, domain)?
            }
            Node::MovingAverage {
                x,
                window,
                step,
                warmup,
                unknown,
            } => {
                let child = self.waveform(x)?;
                let window = positive(window.ticks(self.options.time_base)?, "moving_avg window")?;
                let step = self.step(step.as_ref())?;
                self.moving_average(index, child, window, step, warmup, unknown)?
            }
            Node::Lowpass { x, tau, step } => {
                let child = self.waveform(x)?;
                let tau = positive(tau.ticks(self.options.time_base)?, "lowpass tau")?;
                let step = self.step(step.as_ref())?;
                self.lowpass(child, tau, step)?
            }
            Node::Integrate { x, initial, step } => {
                let child = self.waveform(x)?;
                let initial = self.constant(initial)?;
                let step = self.step(step.as_ref())?;
                self.integrate(child, initial, step)?
            }
            Node::Sine {
                period,
                amplitude,
                offset,
                phase,
                step,
            } => {
                let period = positive(period.ticks(self.options.time_base)?, "sine period")?;
                let step = self.step(step.as_ref())?;
                ensure!(
                    u128::from(step) * 2 <= u128::from(period),
                    "sine requires at least two samples per period."
                );
                if u128::from(step) * 20 > u128::from(period) {
                    self.warnings.push_text(
                        "sine has fewer than twenty samples per period; reduce step for a smoother waveform.",
                    )?;
                }
                self.sine(
                    domain,
                    period,
                    self.constant(amplitude)?,
                    self.constant(offset)?,
                    self.constant(phase)?,
                    step,
                )?
            }
            Node::Clock {
                period,
                high,
                phase,
            } => {
                let period = positive(period.ticks(self.options.time_base)?, "clock period")?;
                let high = positive(high.ticks(self.options.time_base)?, "clock high")?;
                let phase = phase.ticks(self.options.time_base)?;
                ensure!(high < period, "clock high must be less than period.");
                ensure!(phase < period, "clock phase must be less than period.");
                self.clock(domain, period, high, phase)?
            }
            Node::Capture { x, trigger } => {
                let child = self.waveform(x)?;
                self.capture(child, trigger)?
            }
            Node::Reduce { .. } => bail!("A scalar measurement has no waveform history."),
        };
        self.waveforms[index] = Some(waveform.clone());
        Ok(waveform)
    }

    fn operand(&mut self, index: usize, domain: (u64, u64)) -> Result<Arc<Waveform>> {
        if self.program.nodes[index].kind == NodeKind::Literal {
            self.constant_waveform(index, domain)
        } else {
            self.waveform(index)
        }
    }

    fn input(&mut self, input: usize, domain: (u64, u64)) -> Result<Arc<Waveform>> {
        let source = &self.sources[input];
        ensure!(source.domain == domain, "Input domain resolution changed.");
        source.interpretation.validate(source.history.shape())?;
        let reader = InputHistory {
            source: source.history,
            projection: source.interpretation.projection,
        };
        let ty = source.ty();
        let decode = |index: Option<usize>| {
            let value = reader.number(source.interpretation.kind, index);
            if reader.shape() == SignalShape::Bit {
                Value::Boolean(value.map(|value| value != 0.0))
            } else {
                Value::Real(value)
            }
        };
        let capture_times = source.history.capture_times();
        let mut events = Buffer::new(self.budget)?;
        let mut captures = Buffer::new(self.budget)?;
        if let Some(times) = capture_times {
            for &time in times {
                if time < domain.0 || time > domain.1 {
                    continue;
                }
                self.work.spend(1)?;
                let value = decode(source.history.index_at(time));
                push(&mut events, time, value, true)?;
                if events.last().is_some_and(|event| event.time == time) {
                    captures.push(time)?;
                }
            }
        } else {
            let initial = source.history.index_at(domain.0);
            push(&mut events, domain.0, decode(initial), false)?;
            let mut next = initial.map_or(0, |index| index + 1);
            while next < source.history.len() {
                let time = source.history.time(next);
                if time <= domain.0 {
                    next += 1;
                    continue;
                }
                if time > domain.1 {
                    break;
                }
                self.work.spend(1)?;
                push(&mut events, time, decode(Some(next)), false)?;
                next += 1;
            }
        }
        Ok(Arc::new(Waveform {
            domain,
            ty,
            events,
            captures,
        }))
    }

    fn pointwise_unary(
        &mut self,
        operator: Unary,
        child: Arc<Waveform>,
        domain: (u64, u64),
    ) -> Result<Arc<Waveform>> {
        let ty = match operator {
            Unary::Not | Unary::Boolean => Type::Boolean,
            _ => Type::Real,
        };
        let mut events = Buffer::new(self.budget)?;
        push(
            &mut events,
            domain.0,
            evaluate::unary(operator, child.value_at(domain.0)),
            false,
        )?;
        for event in child.events.iter() {
            if event.time <= domain.0 || event.time > domain.1 {
                continue;
            }
            self.work.spend(1)?;
            push(
                &mut events,
                event.time,
                evaluate::unary(operator, event.value),
                false,
            )?;
        }
        Ok(Arc::new(Waveform {
            domain,
            ty,
            events,
            captures: Buffer::new(self.budget)?,
        }))
    }

    fn pointwise_binary(
        &mut self,
        operator: Binary,
        left: Arc<Waveform>,
        right: Arc<Waveform>,
        domain: (u64, u64),
    ) -> Result<Arc<Waveform>> {
        let ty = match operator {
            Binary::And
            | Binary::Or
            | Binary::Equal
            | Binary::NotEqual
            | Binary::Less
            | Binary::LessEqual
            | Binary::Greater
            | Binary::GreaterEqual => Type::Boolean,
            _ => Type::Real,
        };
        let mut events = Buffer::new(self.budget)?;
        push(
            &mut events,
            domain.0,
            evaluate::binary(operator, left.value_at(domain.0), right.value_at(domain.0)),
            false,
        )?;
        let (mut a, mut b) = (0, 0);
        loop {
            let left_time = left.events.get(a).map(|event| event.time);
            let right_time = right.events.get(b).map(|event| event.time);
            let time = match (left_time, right_time) {
                (Some(left), Some(right)) => left.min(right),
                (Some(time), None) | (None, Some(time)) => time,
                (None, None) => break,
            };
            while left.events.get(a).is_some_and(|event| event.time == time) {
                a += 1;
            }
            while right.events.get(b).is_some_and(|event| event.time == time) {
                b += 1;
            }
            if time <= domain.0 || time > domain.1 {
                continue;
            }
            self.work.spend(1)?;
            push(
                &mut events,
                time,
                evaluate::binary(operator, left.value_at(time), right.value_at(time)),
                false,
            )?;
        }
        Ok(Arc::new(Waveform {
            domain,
            ty,
            events,
            captures: Buffer::new(self.budget)?,
        }))
    }

    fn moving_average(
        &mut self,
        index: usize,
        child: Arc<Waveform>,
        window: u64,
        step: u64,
        warmup: Warmup,
        unknown: UnknownPolicy,
    ) -> Result<Arc<Waveform>> {
        let domain = child.domain;
        let mut events = Buffer::new(self.budget)?;
        let mut coverage = Buffer::new(self.budget)?;
        let mut left = HeldCursor::new(&child, domain.0);
        let mut right = HeldCursor::new(&child, domain.0);
        let mut area = 0.0;
        let mut compensation = 0.0;
        let mut unknown_ticks = 0_i128;
        for time in Grid::new(domain, step) {
            self.work.spend(1)?;
            let start = time.saturating_sub(window).max(domain.0);
            if start >= right.time {
                left = HeldCursor::new(&child, start);
                right = HeldCursor::new(&child, start);
                area = 0.0;
                compensation = 0.0;
                unknown_ticks = 0;
            }
            for (cursor, target, sign) in [(&mut left, start, -1.0), (&mut right, time, 1.0)] {
                cursor.advance(target, &mut self.work, |duration, value| {
                    if let Some(value) = value {
                        let term = sign * value * duration as f64;
                        let delta = term - compensation;
                        let next = area + delta;
                        compensation = (next - area) - delta;
                        area = next;
                    } else {
                        unknown_ticks += if sign > 0.0 {
                            i128::from(duration)
                        } else {
                            -i128::from(duration)
                        };
                    }
                })?;
            }
            let width = time - start;
            let known = i128::from(width) - unknown_ticks;
            let divisor = match unknown {
                UnknownPolicy::Gap => i128::from(width),
                UnknownPolicy::Skip => known,
            };
            let value = if width == 0
                || (warmup == Warmup::Full && time - domain.0 < window)
                || (unknown == UnknownPolicy::Gap && unknown_ticks > 0)
                || known == 0
            {
                None
            } else {
                let value = area / divisor as f64;
                value.is_finite().then_some(value)
            };
            if unknown == UnknownPolicy::Skip {
                coverage.push((
                    time,
                    if width == 0 {
                        1.0
                    } else {
                        known as f64 / width as f64
                    },
                ))?;
            }
            push(&mut events, time, Value::Real(value), false)?;
        }
        if unknown == UnknownPolicy::Skip {
            self.coverages[index] = Some(coverage);
        }
        Ok(Arc::new(Waveform {
            domain,
            ty: Type::Real,
            events,
            captures: Buffer::new(self.budget)?,
        }))
    }

    fn lowpass(&mut self, child: Arc<Waveform>, tau: u64, step: u64) -> Result<Arc<Waveform>> {
        let domain = child.domain;
        let mut events = Buffer::new(self.budget)?;
        let mut cursor = HeldCursor::new(&child, domain.0);
        let mut state = match child.value_at(domain.0) {
            Value::Real(value) => value,
            Value::Boolean(_) => None,
        };
        for time in Grid::new(domain, step) {
            self.work.spend(1)?;
            cursor.advance(time, &mut self.work, |duration, value| {
                if let Some(value) = value {
                    state = Some(match state {
                        Some(state) if duration > 0 => {
                            value + (state - value) * (-(duration as f64) / tau as f64).exp()
                        }
                        Some(state) => state,
                        None => value,
                    });
                } else if duration > 0 {
                    state = None;
                }
            })?;
            if state.is_none()
                && let Some(index) = cursor.index
                && child.events[index].time == time
                && let Value::Real(Some(value)) = child.events[index].value
            {
                state = Some(value);
            }
            push(
                &mut events,
                time,
                Value::Real(state.filter(|v| v.is_finite())),
                false,
            )?;
        }
        Ok(Arc::new(Waveform {
            domain,
            ty: Type::Real,
            events,
            captures: Buffer::new(self.budget)?,
        }))
    }

    fn integrate(
        &mut self,
        child: Arc<Waveform>,
        initial: f64,
        step: u64,
    ) -> Result<Arc<Waveform>> {
        let domain = child.domain;
        let weight = tick_weight(self.options.time_base);
        let mut events = Buffer::new(self.budget)?;
        let mut cursor = HeldCursor::new(&child, domain.0);
        let mut area = initial;
        let mut compensation = 0.0;
        let mut poisoned = false;
        for time in Grid::new(domain, step) {
            self.work.spend(1)?;
            cursor.advance(time, &mut self.work, |duration, value| {
                if let Some(value) = value {
                    let term = value * duration as f64 * weight;
                    let delta = term - compensation;
                    let next = area + delta;
                    compensation = (next - area) - delta;
                    area = next;
                } else if duration > 0 {
                    poisoned = true;
                }
            })?;
            let value = (!poisoned && area.is_finite()).then_some(area);
            push(&mut events, time, Value::Real(value), false)?;
        }
        Ok(Arc::new(Waveform {
            domain,
            ty: Type::Real,
            events,
            captures: Buffer::new(self.budget)?,
        }))
    }

    fn sine(
        &mut self,
        domain: (u64, u64),
        period: u64,
        amplitude: f64,
        offset: f64,
        phase: f64,
        step: u64,
    ) -> Result<Arc<Waveform>> {
        let mut events = Buffer::new(self.budget)?;
        for time in Grid::new(domain, step) {
            self.work.spend(1)?;
            let angle = std::f64::consts::TAU * (time - domain.0) as f64 / period as f64 + phase;
            let value = offset + amplitude * angle.sin();
            push(
                &mut events,
                time,
                Value::Real(value.is_finite().then_some(value)),
                false,
            )?;
        }
        Ok(Arc::new(Waveform {
            domain,
            ty: Type::Real,
            events,
            captures: Buffer::new(self.budget)?,
        }))
    }

    fn clock(
        &mut self,
        domain: (u64, u64),
        period: u64,
        high: u64,
        phase: u64,
    ) -> Result<Arc<Waveform>> {
        let mut events = Buffer::new(self.budget)?;
        let mut remainder = if phase == 0 { 0 } else { period - phase };
        let mut time = domain.0;
        loop {
            self.work.spend(1)?;
            let high_now = remainder < high;
            push(&mut events, time, Value::Boolean(Some(high_now)), false)?;
            let advance = if high_now {
                high - remainder
            } else {
                period - remainder
            };
            let Some(next) = time.checked_add(advance) else {
                break;
            };
            if next > domain.1 {
                break;
            }
            time = next;
            remainder = if high_now { high } else { 0 };
            ensure!(advance > 0, "clock generated a zero-duration transition.");
        }
        Ok(Arc::new(Waveform {
            domain,
            ty: Type::Boolean,
            events,
            captures: Buffer::new(self.budget)?,
        }))
    }

    // Adjacent counter values above 2^53 can become the same f64; compare logic values.
    fn capture_bus(&mut self, child: Arc<Waveform>, input: usize) -> Result<Arc<Waveform>> {
        use volna_trace::data::value_view::ValueView;
        let source = &self.sources[input];
        let domain = (
            child.domain.0.max(source.domain.0),
            child.domain.1.min(source.domain.1),
        );
        ensure!(
            domain.0 <= domain.1,
            "Capture input and sampling signal have no common range."
        );
        let reader = InputHistory {
            source: source.history,
            projection: source.interpretation.projection,
        };
        let mut previous = reader.index_at(if domain.0 > source.domain.0 {
            domain.0 - 1
        } else {
            domain.0
        });
        let mut next = previous.map_or(0, |i| i + 1);
        let mut events = Buffer::new(self.budget)?;
        let mut captures = Buffer::new(self.budget)?;
        while next < reader.len() {
            let time = reader.time(next);
            if time > domain.1 {
                break;
            }
            self.work.spend(1)?;
            let changed = match (reader.value_view(previous), reader.value_view(Some(next))) {
                (ValueView::Logic(a), ValueView::Logic(b)) => {
                    self.work.spend(a.width)?;
                    a.width != b.width || (0..a.width).any(|i| a.bit(i) != b.bit(i))
                }
                (ValueView::Unavailable, ValueView::Unavailable) => false,
                _ => true,
            };
            previous = Some(next);
            next += 1;
            if time < domain.0 || !changed {
                continue;
            }
            push(&mut events, time, child.value_at(time), true)?;
            if captures.last().copied() != Some(time) {
                captures.push(time)?;
            }
        }
        Ok(Arc::new(Waveform {
            domain,
            ty: child.ty,
            events,
            captures,
        }))
    }

    fn capture(&mut self, child: Arc<Waveform>, trigger: Trigger) -> Result<Arc<Waveform>> {
        let warning =
            "Captures use timestamp-coalesced histories; delta-cycle edges cannot be recovered.";
        if !self.warnings.iter().any(|existing| existing == warning) {
            self.warnings.push_text(warning)?;
        }
        match trigger {
            Trigger::Periodic(duration) => {
                let step = positive(duration.ticks(self.options.time_base)?, "capture duration")?;
                let mut events = Buffer::new(self.budget)?;
                let mut captures = Buffer::new(self.budget)?;
                for time in Grid::new(child.domain, step) {
                    self.work.spend(1)?;
                    push(&mut events, time, child.value_at(time), true)?;
                    captures.push(time)?;
                }
                Ok(Arc::new(Waveform {
                    domain: child.domain,
                    ty: child.ty,
                    events,
                    captures,
                }))
            }
            Trigger::Posedge(clock) | Trigger::Changes(clock) => {
                let rising = matches!(trigger, Trigger::Posedge(_));
                if !rising
                    && let Node::Input(input) = self.program.nodes[clock].node
                    && matches!(
                        self.sources[input].history.shape(),
                        SignalShape::Vector { .. }
                    )
                {
                    return self.capture_bus(child, input);
                }
                let clock = self.waveform(clock)?;
                let domain = (
                    child.domain.0.max(clock.domain.0),
                    child.domain.1.min(clock.domain.1),
                );
                ensure!(
                    domain.0 <= domain.1,
                    "Capture input and clock have no common range."
                );
                let mut events = Buffer::new(self.budget)?;
                let mut captures = Buffer::new(self.budget)?;
                let mut previous =
                    (domain.0 > clock.domain.0).then(|| clock.value_at(domain.0 - 1));
                for event in clock.events.iter() {
                    if event.time < domain.0 || event.time > domain.1 {
                        continue;
                    }
                    self.work.spend(1)?;
                    if event.time > domain.0 && previous.is_none() {
                        previous = Some(clock.value_at(domain.0));
                    }
                    let next = clock.value_at(event.time);
                    let publish = if rising {
                        matches!(previous, Some(Value::Boolean(Some(false))))
                            && matches!(next, Value::Boolean(Some(true)))
                    } else {
                        previous.is_some_and(|previous| !previous.same(next))
                    };
                    previous = Some(next);
                    if !publish {
                        continue;
                    }
                    push(&mut events, event.time, child.value_at(event.time), true)?;
                    if events
                        .last()
                        .is_some_and(|capture| capture.time == event.time)
                    {
                        captures.push(event.time)?;
                    }
                }
                Ok(Arc::new(Waveform {
                    domain,
                    ty: child.ty,
                    events,
                    captures,
                }))
            }
        }
    }

    fn scalar(&mut self, index: usize) -> Result<Measurement> {
        let value = self.scalar_value(index)?;
        let domain = self.resolved_domain(index)?;
        let bounds = self.options.interval.unwrap_or(domain);
        ensure!(
            bounds.0 < bounds.1,
            "Measurement scope must have positive duration."
        );
        ensure!(
            bounds.0 >= domain.0 && bounds.1 <= domain.1,
            "Measurement bounds must be inside the resolved source domain."
        );
        let support = value.support.unwrap_or(Support {
            requested: bounds.1 - bounds.0,
            known: bounds.1 - bounds.0,
            captures: None,
            status: MeasurementStatus::Complete,
        });
        Ok(Measurement {
            value: match value.value {
                Value::Real(value) => value,
                Value::Boolean(_) => bail!("Scalar formulas must produce Real values."),
            },
            bounds,
            requested_ticks: support.requested,
            known_ticks: support.known,
            captures: support.captures,
            policy: self.options.unknown,
            status: if value.value.real().is_some() {
                support.status
            } else if support.status == MeasurementStatus::Complete {
                MeasurementStatus::NonFinite
            } else {
                support.status
            },
        })
    }

    fn scalar_value(&mut self, index: usize) -> Result<Scalar> {
        self.work.spend(1)?;
        let typed = self.program.nodes[index].clone();
        if typed.kind == NodeKind::Literal {
            return Ok(Scalar {
                value: Value::Real(Some(self.constant(index)?)),
                support: None,
            });
        }
        match typed.node {
            Node::Unary(operator, child) => {
                let child = self.scalar_value(child)?;
                Ok(Scalar {
                    value: evaluate::unary(operator, child.value),
                    support: child.support,
                })
            }
            Node::Binary(operator, left, right) => {
                let left = self.scalar_value(left)?;
                let right = self.scalar_value(right)?;
                let support = match (left.support, right.support) {
                    (Some(support), None) | (None, Some(support)) => Some(support),
                    _ => None,
                };
                Ok(Scalar {
                    value: evaluate::binary(operator, left.value, right.value),
                    support,
                })
            }
            Node::Reduce { reduction, x } => {
                let waveform = self.waveform(x)?;
                let domain = self.resolved_domain(index)?;
                let bounds = self.options.interval.unwrap_or(domain);
                ensure!(
                    bounds.0 < bounds.1,
                    "Measurement scope must have positive duration."
                );
                ensure!(
                    bounds.0 >= domain.0 && bounds.1 <= domain.1,
                    "Measurement bounds must be inside the resolved source domain."
                );
                let (value, support) = self.reduce(reduction, &waveform, bounds)?;
                Ok(Scalar {
                    value,
                    support: Some(support),
                })
            }
            _ => bail!("Expected a scalar measurement expression."),
        }
    }

    fn reduce(
        &mut self,
        reduction: Reduction,
        waveform: &Waveform,
        bounds: (u64, u64),
    ) -> Result<(Value, Support)> {
        if reduction == Reduction::Sum {
            let first = waveform.captures.partition_point(|time| *time < bounds.0);
            let end = waveform.captures.partition_point(|time| *time < bounds.1);
            let captures = &waveform.captures[first..end];
            let mut sum = 0.0;
            let mut known = 0;
            let mut unknown = 0;
            for time in captures.iter().copied() {
                self.work.spend(1)?;
                match waveform.value_at(time) {
                    Value::Real(Some(value)) => {
                        sum += value;
                        known += 1;
                    }
                    _ => unknown += 1,
                }
            }
            let value = if unknown > 0 && self.options.unknown == UnknownPolicy::Gap {
                None
            } else {
                sum.is_finite().then_some(sum)
            };
            return Ok((
                Value::Real(value),
                Support {
                    requested: bounds.1 - bounds.0,
                    known: bounds.1 - bounds.0,
                    captures: Some(CaptureSupport {
                        known,
                        total: captures.len(),
                    }),
                    status: if unknown > 0 {
                        MeasurementStatus::UnknownData
                    } else if value.is_none() {
                        MeasurementStatus::NonFinite
                    } else {
                        MeasurementStatus::Complete
                    },
                },
            ));
        }
        let requested = bounds.1 - bounds.0;
        let mut known = 0_u64;
        let mut area = 0.0;
        let mut compensation = 0.0;
        let mut extremum: Option<f64> = None;
        let mut unknown = false;
        let weight = tick_weight(self.options.time_base);
        let mut start = bounds.0;
        let mut next_event = waveform
            .events
            .partition_point(|event| event.time <= bounds.0);
        while start < bounds.1 {
            self.work.spend(1)?;
            let next = waveform
                .events
                .get(next_event)
                .filter(|event| event.time < bounds.1)
                .map_or(bounds.1, |event| event.time);
            next_event += 1;
            let duration = next - start;
            match waveform.value_at(start) {
                Value::Real(Some(value)) => {
                    known += duration;
                    let term = value
                        * duration as f64
                        * if reduction == Reduction::Integral {
                            weight
                        } else {
                            1.0
                        };
                    let delta = term - compensation;
                    let total = area + delta;
                    compensation = (total - area) - delta;
                    area = total;
                    extremum = Some(match (reduction, extremum) {
                        (Reduction::Min, Some(current)) => current.min(value),
                        (Reduction::Max, Some(current)) => current.max(value),
                        _ => value,
                    });
                }
                _ => unknown = true,
            }
            start = next;
        }
        let strict_unknown = unknown && self.options.unknown == UnknownPolicy::Gap;
        let value = match reduction {
            Reduction::Mean if known > 0 && !strict_unknown => {
                let divisor = match self.options.unknown {
                    UnknownPolicy::Gap => requested,
                    UnknownPolicy::Skip => known,
                };
                (area / divisor as f64)
                    .is_finite()
                    .then_some(area / divisor as f64)
            }
            Reduction::Integral if !strict_unknown => area.is_finite().then_some(area),
            Reduction::Min | Reduction::Max if known > 0 && !strict_unknown => extremum,
            _ => None,
        };
        Ok((
            Value::Real(value),
            Support {
                requested,
                known,
                captures: None,
                status: if unknown {
                    MeasurementStatus::UnknownData
                } else if known == 0 && reduction != Reduction::Integral {
                    MeasurementStatus::NoKnownData
                } else if value.is_none() {
                    MeasurementStatus::NonFinite
                } else {
                    MeasurementStatus::Complete
                },
            },
        ))
    }
}

#[derive(Clone, Copy)]
struct Scalar {
    value: Value,
    support: Option<Support>,
}

#[derive(Clone, Copy)]
struct Support {
    requested: u64,
    known: u64,
    captures: Option<CaptureSupport>,
    status: MeasurementStatus,
}

fn positive(value: u64, name: &str) -> Result<u64> {
    ensure!(
        value > 0,
        "{name} must be a positive whole number of timeline ticks."
    );
    Ok(value)
}

fn tick_weight(base: crate::wave::timeline::TimeBase<'_>) -> f64 {
    if base.unit.is_some() {
        1.0
    } else {
        10_f64.powi(i32::from(base.timescale))
    }
}

struct Grid {
    end: u64,
    step: u64,
    next: Option<u64>,
}

impl Grid {
    fn new(domain: (u64, u64), step: u64) -> Self {
        Self {
            end: domain.1,
            step,
            next: Some(domain.0),
        }
    }
}

impl Iterator for Grid {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = current
            .checked_add(self.step)
            .filter(|next| *next <= self.end);
        Some(current)
    }
}

struct Work<'a> {
    cancelled: &'a AtomicBool,
    steps: usize,
}

impl Work<'_> {
    fn spend(&mut self, count: usize) -> Result<()> {
        ensure!(
            !self.cancelled.load(Ordering::Relaxed),
            "Computation cancelled."
        );
        self.steps = self
            .steps
            .checked_add(count)
            .context("Formula work counter overflowed.")?;
        ensure!(
            self.steps <= MAX_WORK,
            "Formula exceeds 10,000,000 evaluation steps; increase sampling intervals or reduce the range."
        );
        Ok(())
    }
}

struct HeldCursor<'a> {
    waveform: &'a Waveform,
    index: Option<usize>,
    time: u64,
}

impl<'a> HeldCursor<'a> {
    fn new(waveform: &'a Waveform, time: u64) -> Self {
        let index = waveform
            .events
            .partition_point(|event| event.time <= time)
            .checked_sub(1);
        Self {
            waveform,
            index,
            time,
        }
    }

    fn advance(
        &mut self,
        target: u64,
        work: &mut Work<'_>,
        mut add: impl FnMut(u64, Option<f64>),
    ) -> Result<()> {
        while self.time < target {
            work.spend(1)?;
            let next = self.index.map_or(0, |index| index + 1);
            let end = if next < self.waveform.events.len() {
                self.waveform.events[next].time.min(target)
            } else {
                target
            };
            ensure!(end >= self.time, "Formula history is not ordered.");
            let value = self.index.map(|index| self.waveform.events[index].value);
            let value = match value {
                Some(Value::Real(value)) => value,
                _ => None,
            };
            add(end - self.time, value);
            self.time = end;
            if next < self.waveform.events.len() && self.waveform.events[next].time <= target {
                self.index = Some(next);
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct FormulaHistory {
    events: Buffer<Event>,
    output: Type,
    domain: (u64, u64),
    captured: bool,
    captures: Buffer<u64>,
    coverage: Option<Buffer<(u64, f64)>>,
    warnings: Buffer<String>,
}

impl FormulaHistory {
    fn from_waveform(
        waveform: Waveform,
        warnings: Buffer<String>,
        coverage: Option<Buffer<(u64, f64)>>,
        cancelled: &AtomicBool,
    ) -> Result<Self> {
        ensure!(!cancelled.load(Ordering::Relaxed), "Computation cancelled.");
        let captured = waveform.captured();
        let mut history = Self {
            events: waveform.events,
            output: waveform.ty,
            domain: waveform.domain,
            captured,
            captures: waveform.captures,
            coverage,
            warnings,
        };
        if let Some(end) = waveform.domain.1.checked_add(1) {
            let gap = match waveform.ty {
                Type::Real => Value::Real(None),
                Type::Boolean => Value::Boolean(None),
            };
            if history
                .events
                .last()
                .is_none_or(|event| !event.value.same(gap))
            {
                ensure!(
                    history.events.len() < MAX_EVENTS,
                    "Formula exceeds 1,000,000 output events."
                );
                history.events.push(Event {
                    time: end,
                    value: gap,
                })?;
            }
        }
        Ok(history)
    }

    #[must_use]
    pub fn capture_times(&self) -> &[u64] {
        &self.captures
    }

    #[must_use]
    pub fn coverage_at(&self, time: u64) -> Option<f64> {
        if time < self.domain.0 || time > self.domain.1 {
            return None;
        }
        let coverage = self.coverage.as_ref()?;
        let index = coverage
            .partition_point(|(sample, _)| *sample <= time)
            .checked_sub(1)?;
        Some(coverage[index].1)
    }

    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    #[must_use]
    pub fn domain(&self) -> (u64, u64) {
        self.domain
    }
}

impl SignalHistory for FormulaHistory {
    fn resident_bytes(&self) -> u64 {
        self.events.resident_bytes()
            + self.captures.resident_bytes()
            + self.coverage.as_ref().map_or(0, Buffer::resident_bytes)
            + self.warnings.resident_bytes()
    }

    fn shape(&self) -> SignalShape {
        self.output.shape()
    }

    fn len(&self) -> usize {
        self.events.len()
    }

    fn time(&self, index: usize) -> u64 {
        self.events[index].time
    }

    fn value(&self, index: Option<usize>) -> WaveValue {
        match index
            .and_then(|index| self.events.get(index))
            .map(|event| event.value)
        {
            Some(Value::Real(Some(value))) => WaveValue::Real(value),
            Some(Value::Boolean(Some(value))) => {
                WaveValue::Bits(if value { "1" } else { "0" }.into())
            }
            _ => WaveValue::Unavailable,
        }
    }

    fn bit(&self, index: Option<usize>) -> Bit {
        match index
            .and_then(|index| self.events.get(index))
            .map(|event| event.value)
        {
            Some(Value::Boolean(Some(true))) => Bit::One,
            Some(Value::Boolean(Some(false))) => Bit::Zero,
            _ => Bit::Unavailable,
        }
    }

    fn capture_times(&self) -> Option<&[u64]> {
        self.captured.then_some(&self.captures)
    }

    fn coverage_at(&self, time: u64) -> Option<f64> {
        self.coverage_at(time)
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}
