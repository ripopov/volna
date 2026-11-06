use super::{
    Binary, Duration, DurationUnit, MAX_DEPTH, MAX_NODES, Node, NodeKind, Program, Reduction,
    ResultKind, Trigger, Type, TypedNode, Unary, UnknownPolicy, Warmup,
};
use anyhow::{Context, Result, bail, ensure};

use super::language::{self, Diagnostic, SyntaxToken, TokenKind};
use std::ops::Range;

pub(super) fn reserved(name: &str) -> bool {
    matches!(name, "true" | "false" | "posedge")
        || language::FUNCTIONS
            .iter()
            .any(|(function, _)| *function == name)
}

#[derive(Clone, Debug)]
enum Token {
    Number(u64),
    Duration(Duration),
    Identifier(String),
    Boolean(bool),
    Policy(&'static str),
    Operator(&'static str),
    LeftParen,
    RightParen,
    Comma,
    Colon,
    Capture,
}

fn decimal(text: &str) -> Result<(u128, i32)> {
    let (mantissa, exponent) = text
        .split_once(['e', 'E'])
        .map_or((text, "0"), |(a, b)| (a, b));
    let exponent = exponent
        .parse::<i32>()
        .context("Invalid duration exponent.")?;
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}")
        .parse::<u128>()
        .context("Duration is too large.")?;
    let scale = exponent
        .checked_sub(fraction.len() as i32)
        .context("Duration is too large.")?;
    Ok((digits, scale))
}

fn unit(name: &str) -> DurationUnit {
    match name {
        "fs" => DurationUnit::Si(-15),
        "ps" => DurationUnit::Si(-12),
        "ns" => DurationUnit::Si(-9),
        "us" => DurationUnit::Si(-6),
        "ms" => DurationUnit::Si(-3),
        "s" => DurationUnit::Si(0),
        "ticks" => DurationUnit::Ticks,
        other => DurationUnit::Logical(other.into()),
    }
}

fn number(text: &str, syntax: &SyntaxToken) -> Result<Token> {
    let raw = &text[syntax.range.start..syntax.number_end];
    let value = raw.parse::<f64>().context("Invalid numeric constant.")?;
    ensure!(value.is_finite(), "Numeric constants must be finite.");
    if syntax.kind == TokenKind::Duration {
        let (digits, scale) = decimal(raw)?;
        Ok(Token::Duration(Duration {
            digits,
            scale,
            unit: unit(text[syntax.number_end..syntax.range.end].trim()),
        }))
    } else {
        Ok(Token::Number(value.to_bits()))
    }
}

fn located(error: anyhow::Error, range: Range<usize>, source: &str) -> anyhow::Error {
    if error.downcast_ref::<Diagnostic>().is_some() {
        error
    } else {
        Diagnostic {
            source: source.into(),
            range,
            message: format!("{error:#}"),
        }
        .into()
    }
}

fn token(text: &str, syntax: &SyntaxToken) -> Result<Token> {
    let raw = &text[syntax.range.clone()];
    match syntax.kind {
        TokenKind::Number | TokenKind::Duration => number(text, syntax),
        TokenKind::Identifier | TokenKind::Function | TokenKind::Keyword => Ok(match raw {
            "true" => Token::Boolean(true),
            "false" => Token::Boolean(false),
            _ => Token::Identifier(raw.into()),
        }),
        TokenKind::String => {
            ensure!(
                raw.len() >= 2 && raw.ends_with('"'),
                "Unterminated policy literal."
            );
            Ok(Token::Policy(match &raw[1..raw.len() - 1] {
                "full" => "full",
                "available" => "available",
                "gap" => "gap",
                "skip" => "skip",
                policy => bail!("Unknown policy literal '\"{policy}\"'."),
            }))
        }
        _ => Ok(match raw {
            "||" => Token::Operator("||"),
            "&&" => Token::Operator("&&"),
            "==" => Token::Operator("=="),
            "!=" => Token::Operator("!="),
            "<=" => Token::Operator("<="),
            ">=" => Token::Operator(">="),
            "<" => Token::Operator("<"),
            ">" => Token::Operator(">"),
            "+" => Token::Operator("+"),
            "-" => Token::Operator("-"),
            "*" => Token::Operator("*"),
            "/" => Token::Operator("/"),
            "%" => Token::Operator("%"),
            "!" => Token::Operator("!"),
            "(" => Token::LeftParen,
            ")" => Token::RightParen,
            "," => Token::Comma,
            ":" => Token::Colon,
            "@" => Token::Capture,
            _ => bail!("Unexpected token in formula."),
        }),
    }
}

pub(super) fn parse(
    expression: &str,
    inputs: &[(String, Type)],
    captured: &[bool],
) -> Result<Program> {
    let syntax = language::Syntax::new(expression);
    let tokens = syntax
        .tokens()
        .iter()
        .map(|syntax| {
            token(expression, syntax)
                .map_err(|error| located(error, syntax.range.clone(), expression))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut parser = Parser {
        source: expression,
        tokens,
        ranges: syntax.tokens.into_iter().map(|token| token.range).collect(),
        end: expression.len(),
        at: 0,
        nodes: Vec::new(),
        inputs,
        captured,
        used_inputs: vec![false; inputs.len()],
    };
    let result = (|| {
        let root = parser.expression(0)?;
        ensure!(
            parser.at == parser.tokens.len(),
            "Unexpected token in formula."
        );
        let root = &parser.nodes[root.node];
        let kind = match root.kind {
            NodeKind::Scalar => {
                ensure!(
                    root.ty == Type::Real,
                    "Scalar measurements must produce Real values."
                );
                ResultKind::ScalarMeasurement
            }
            NodeKind::Waveform { captured: true } => ResultKind::CapturedWaveform,
            NodeKind::Literal | NodeKind::Waveform { captured: false } => ResultKind::Waveform,
        };
        Ok(Program {
            output: root.ty,
            kind,
            nodes: std::mem::take(&mut parser.nodes),
            used_inputs: std::mem::take(&mut parser.used_inputs),
        })
    })();
    result.map_err(|error| {
        located(
            error,
            parser
                .ranges
                .get(parser.at)
                .cloned()
                .unwrap_or(parser.end..parser.end),
            expression,
        )
    })
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    ranges: Vec<Range<usize>>,
    end: usize,
    at: usize,
    nodes: Vec<TypedNode>,
    inputs: &'a [(String, Type)],
    captured: &'a [bool],
    used_inputs: Vec<bool>,
}

#[derive(Clone, Copy)]
struct Expr {
    node: usize,
    ty: Type,
    kind: NodeKind,
}

impl Parser<'_> {
    fn push(&mut self, node: Node, ty: Type, kind: NodeKind) -> Result<Expr> {
        ensure!(
            self.nodes.len() < MAX_NODES,
            "Formula is limited to {MAX_NODES} operations."
        );
        let index = self.nodes.len();
        self.nodes.push(TypedNode { node, ty, kind });
        Ok(Expr {
            node: index,
            ty,
            kind,
        })
    }

    fn number(&mut self, value: f64) -> Result<Expr> {
        self.push(Node::Number(value.to_bits()), Type::Real, NodeKind::Literal)
    }

    fn consume_operator(&mut self, operator: &str) -> bool {
        if matches!(self.tokens.get(self.at), Some(Token::Operator(found)) if *found == operator) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn consume(&mut self, expected: &Token) -> bool {
        let matches = matches!(
            (self.tokens.get(self.at), expected),
            (Some(Token::LeftParen), Token::LeftParen)
                | (Some(Token::RightParen), Token::RightParen)
                | (Some(Token::Comma), Token::Comma)
                | (Some(Token::Colon), Token::Colon)
                | (Some(Token::Capture), Token::Capture)
        );
        if matches {
            self.at += 1;
        }
        matches
    }

    fn expression(&mut self, depth: usize) -> Result<Expr> {
        ensure!(
            depth < MAX_DEPTH,
            "Formula nesting exceeds {MAX_DEPTH} levels."
        );
        let value = self.logical_or(depth)?;
        if !self.consume(&Token::Capture) {
            return Ok(value);
        }
        ensure!(
            value.kind.is_waveform_or_literal(),
            "A scalar result cannot be captured."
        );
        let trigger = match self.tokens.get(self.at).cloned() {
            Some(Token::Duration(duration)) => {
                self.at += 1;
                Trigger::Periodic(duration)
            }
            Some(Token::Identifier(name)) if name == "posedge" => {
                self.at += 1;
                Trigger::Posedge(self.clock(depth)?)
            }
            Some(Token::Identifier(name)) => {
                self.at += 1;
                Trigger::Changes(self.clock_named(&name)?)
            }
            _ => bail!("Expected a duration, posedge clock, or Boolean clock after '@'."),
        };
        ensure!(
            !matches!(self.tokens.get(self.at), Some(Token::Capture)),
            "Only one '@' suffix is allowed at an expression level."
        );
        self.push(
            Node::Capture {
                x: value.node,
                trigger,
            },
            value.ty,
            NodeKind::Waveform { captured: true },
        )
    }

    fn clock(&mut self, depth: usize) -> Result<usize> {
        ensure!(
            depth < MAX_DEPTH,
            "Formula nesting exceeds {MAX_DEPTH} levels."
        );
        let Some(Token::Identifier(name)) = self.tokens.get(self.at).cloned() else {
            bail!("The capture clock must be one bound Boolean alias.");
        };
        self.at += 1;
        self.clock_named(&name)
    }

    fn clock_named(&mut self, name: &str) -> Result<usize> {
        let (index, (_, ty)) = self
            .inputs
            .iter()
            .enumerate()
            .find(|(_, (alias, _))| alias == name)
            .with_context(|| format!("Unknown capture clock '{name}'."))?;
        ensure!(
            *ty == Type::Boolean,
            "Capture clocks must be Boolean bindings."
        );
        self.used_inputs[index] = true;
        self.push(
            Node::Input(index),
            Type::Boolean,
            NodeKind::Waveform {
                captured: self.captured[index],
            },
        )
        .map(|clock| clock.node)
    }

    fn logical_or(&mut self, depth: usize) -> Result<Expr> {
        let mut left = self.logical_and(depth)?;
        while self.consume_operator("||") {
            let right = self.logical_and(depth + 1)?;
            left = self.binary(Binary::Or, left, right)?;
        }
        Ok(left)
    }

    fn logical_and(&mut self, depth: usize) -> Result<Expr> {
        let mut left = self.equality(depth)?;
        while self.consume_operator("&&") {
            let right = self.equality(depth + 1)?;
            left = self.binary(Binary::And, left, right)?;
        }
        Ok(left)
    }

    fn equality(&mut self, depth: usize) -> Result<Expr> {
        let mut left = self.comparison(depth)?;
        let operator = if self.consume_operator("==") {
            Some(Binary::Equal)
        } else if self.consume_operator("!=") {
            Some(Binary::NotEqual)
        } else {
            None
        };
        if let Some(operator) = operator {
            let right = self.comparison(depth + 1)?;
            ensure!(
                !matches!(self.tokens.get(self.at), Some(Token::Operator("==" | "!="))),
                "Equality operators are not associative; use explicit Boolean combinations."
            );
            left = self.binary(operator, left, right)?;
        }
        Ok(left)
    }

    fn comparison(&mut self, depth: usize) -> Result<Expr> {
        let mut left = self.additive(depth)?;
        let operator = ["<=", ">=", "<", ">"]
            .into_iter()
            .find(|operator| self.consume_operator(operator))
            .map(|operator| match operator {
                "<=" => Binary::LessEqual,
                ">=" => Binary::GreaterEqual,
                "<" => Binary::Less,
                _ => Binary::Greater,
            });
        if let Some(operator) = operator {
            let right = self.additive(depth + 1)?;
            ensure!(
                !matches!(
                    self.tokens.get(self.at),
                    Some(Token::Operator("<" | "<=" | ">" | ">="))
                ),
                "Comparison operators are not associative; use explicit Boolean combinations."
            );
            left = self.binary(operator, left, right)?;
        }
        Ok(left)
    }

    fn additive(&mut self, depth: usize) -> Result<Expr> {
        let mut left = self.multiplicative(depth)?;
        loop {
            let operator = if self.consume_operator("+") {
                Some(Binary::Add)
            } else if self.consume_operator("-") {
                Some(Binary::Subtract)
            } else {
                None
            };
            let Some(operator) = operator else { break };
            let right = self.multiplicative(depth + 1)?;
            left = self.binary(operator, left, right)?;
        }
        Ok(left)
    }

    fn multiplicative(&mut self, depth: usize) -> Result<Expr> {
        let mut left = self.unary(depth)?;
        loop {
            let operator = if self.consume_operator("*") {
                Some(Binary::Multiply)
            } else if self.consume_operator("/") {
                Some(Binary::Divide)
            } else if self.consume_operator("%") {
                Some(Binary::Remainder)
            } else {
                None
            };
            let Some(operator) = operator else { break };
            let right = self.unary(depth + 1)?;
            left = self.binary(operator, left, right)?;
        }
        Ok(left)
    }

    fn unary(&mut self, depth: usize) -> Result<Expr> {
        ensure!(
            depth < MAX_DEPTH,
            "Formula nesting exceeds {MAX_DEPTH} levels."
        );
        for (token, operator) in [
            ("-", Unary::Negate),
            ("+", Unary::Positive),
            ("!", Unary::Not),
        ] {
            if self.consume_operator(token) {
                let value = self.unary(depth + 1)?;
                match operator {
                    Unary::Not => ensure!(value.ty == Type::Boolean, "'!' requires a Boolean."),
                    _ => ensure!(value.ty == Type::Real, "Unary '{token}' requires a number."),
                }
                let kind = value.kind.pointwise();
                return self.push(Node::Unary(operator, value.node), value.ty, kind);
            }
        }
        self.primary(depth + 1)
    }

    fn primary(&mut self, depth: usize) -> Result<Expr> {
        ensure!(
            depth < MAX_DEPTH,
            "Formula nesting exceeds {MAX_DEPTH} levels."
        );
        if self.consume(&Token::LeftParen) {
            let value = self.expression(depth + 1)?;
            ensure!(self.consume(&Token::RightParen), "Expected ')' in formula.");
            return Ok(value);
        }
        match self.tokens.get(self.at).cloned() {
            Some(Token::Number(value)) => {
                self.at += 1;
                self.push(Node::Number(value), Type::Real, NodeKind::Literal)
            }
            Some(Token::Boolean(value)) => {
                self.at += 1;
                self.push(Node::Boolean(value), Type::Boolean, NodeKind::Literal)
            }
            Some(Token::Identifier(name)) => {
                let range = self.ranges[self.at].clone();
                self.at += 1;
                if self.consume(&Token::LeftParen) {
                    return self
                        .call(&name, depth + 1)
                        .map_err(|error| located(error, range, self.source));
                }
                let (index, (_, ty)) = self
                    .inputs
                    .iter()
                    .enumerate()
                    .find(|(_, (alias, _))| alias == &name)
                    .with_context(|| {
                        if self.inputs.is_empty() {
                            format!("Unknown input '{name}'. Add an input binding first.")
                        } else {
                            format!(
                                "Unknown input '{name}'. Available aliases: {}.",
                                self.inputs
                                    .iter()
                                    .map(|(alias, _)| alias.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        }
                    })
                    .map_err(|error| located(error, range, self.source))?;
                self.used_inputs[index] = true;
                self.push(
                    Node::Input(index),
                    *ty,
                    NodeKind::Waveform {
                        captured: self.captured[index],
                    },
                )
            }
            _ => bail!("Expected an input, number, function, or '(' in formula."),
        }
    }

    fn binary(&mut self, operator: Binary, left: Expr, right: Expr) -> Result<Expr> {
        let kind = match (left.kind, right.kind) {
            (NodeKind::Scalar, NodeKind::Waveform { .. })
            | (NodeKind::Waveform { .. }, NodeKind::Scalar) => {
                bail!("Waveforms and scalar measurements cannot be mixed.")
            }
            (NodeKind::Scalar, _) | (_, NodeKind::Scalar) => NodeKind::Scalar,
            (NodeKind::Waveform { .. }, _) | (_, NodeKind::Waveform { .. }) => {
                NodeKind::Waveform { captured: false }
            }
            _ => NodeKind::Literal,
        };
        let ty = match operator {
            Binary::And | Binary::Or => {
                ensure!(
                    left.ty == Type::Boolean && right.ty == Type::Boolean,
                    "Boolean operators require Boolean operands."
                );
                Type::Boolean
            }
            Binary::Equal | Binary::NotEqual => {
                ensure!(
                    left.ty == right.ty,
                    "Equality operands must have the same type."
                );
                Type::Boolean
            }
            Binary::Less | Binary::LessEqual | Binary::Greater | Binary::GreaterEqual => {
                ensure!(
                    left.ty == Type::Real && right.ty == Type::Real,
                    "Comparisons require Real operands."
                );
                Type::Boolean
            }
            _ => {
                ensure!(
                    left.ty == Type::Real && right.ty == Type::Real,
                    "Arithmetic requires numbers; use real(input) for Boolean inputs."
                );
                Type::Real
            }
        };
        self.push(Node::Binary(operator, left.node, right.node), ty, kind)
    }

    fn call(&mut self, name: &str, depth: usize) -> Result<Expr> {
        ensure!(reserved(name), "Unknown function '{name}'.");
        let mut args = Arguments::default();
        if !self.consume(&Token::RightParen) {
            let mut named = false;
            loop {
                let name_token = match self.tokens.get(self.at).cloned() {
                    Some(Token::Identifier(name))
                        if matches!(self.tokens.get(self.at + 1), Some(Token::Colon)) =>
                    {
                        self.at += 2;
                        named = true;
                        Some(name)
                    }
                    _ => None,
                };
                ensure!(
                    named || name_token.is_none(),
                    "Positional arguments must precede named arguments."
                );
                if let Some(name) = &name_token {
                    ensure!(
                        !args.named.iter().any(|(other, _)| other == name),
                        "Argument '{name}' is used more than once."
                    );
                }
                let value = match self.tokens.get(self.at).cloned() {
                    Some(Token::Duration(duration)) => {
                        self.at += 1;
                        Argument::Duration(duration)
                    }
                    Some(Token::Policy(policy)) => {
                        self.at += 1;
                        Argument::Policy(policy)
                    }
                    _ => Argument::Expression(self.expression(depth + 1)?),
                };
                if let Some(name) = name_token {
                    args.named.push((name, value));
                } else {
                    args.positional.push(value);
                }
                if self.consume(&Token::RightParen) {
                    break;
                }
                ensure!(
                    self.consume(&Token::Comma),
                    "Expected ',' or ')' after argument."
                );
                ensure!(
                    !matches!(self.tokens.get(self.at), Some(Token::RightParen)),
                    "Trailing commas are not allowed."
                );
            }
        }
        match name {
            "abs" | "real" | "bool" => {
                let value = args.one_expression(name)?;
                let (operator, ty) = match name {
                    "abs" => {
                        ensure!(value.ty == Type::Real, "abs requires a Real input.");
                        (Unary::Abs, Type::Real)
                    }
                    "real" => (Unary::Real, Type::Real),
                    _ => (Unary::Boolean, Type::Boolean),
                };
                let kind = value.kind.pointwise();
                self.push(Node::Unary(operator, value.node), ty, kind)
            }
            "min" | "max" => {
                let (left, right) = args.two_expressions(name)?;
                ensure!(
                    left.ty == Type::Real && right.ty == Type::Real,
                    "{name} requires two Real inputs."
                );
                self.binary(
                    if name == "min" {
                        Binary::Min
                    } else {
                        Binary::Max
                    },
                    left,
                    right,
                )
            }
            "moving_avg" => {
                args.positional_len(name, 2)?;
                let x = args.expression(0)?;
                let window = args.duration(1)?;
                Self::real_waveform(x, "moving_avg")?;
                let step = args.named_duration("step")?;
                let warmup = match args.named_policy("warmup")? {
                    Some("full") | None => Warmup::Full,
                    Some("available") => Warmup::Available,
                    _ => bail!("moving_avg warmup must be \"full\" or \"available\"."),
                };
                let unknown = match args.named_policy("unknown")? {
                    Some("gap") | None => UnknownPolicy::Gap,
                    Some("skip") => UnknownPolicy::Skip,
                    _ => bail!("moving_avg unknown must be \"gap\" or \"skip\"."),
                };
                args.finish(name)?;
                self.push(
                    Node::MovingAverage {
                        x: x.node,
                        window,
                        step,
                        warmup,
                        unknown,
                    },
                    Type::Real,
                    NodeKind::Waveform { captured: false },
                )
            }
            "lowpass" => {
                args.positional_len(name, 2)?;
                let x = args.expression(0)?;
                let tau = args.duration(1)?;
                Self::real_waveform(x, name)?;
                let step = args.named_duration("step")?;
                args.finish(name)?;
                self.push(
                    Node::Lowpass {
                        x: x.node,
                        tau,
                        step,
                    },
                    Type::Real,
                    NodeKind::Waveform { captured: false },
                )
            }
            "integrate" => {
                args.positional_len(name, 1)?;
                let x = args.expression(0)?;
                Self::real_waveform(x, name)?;
                let initial = match args.named_expression("initial")? {
                    Some(value) => {
                        Self::literal_real(value, "integrate initial")?;
                        value
                    }
                    None => self.number(0.0)?,
                };
                let step = args.named_duration("step")?;
                args.finish(name)?;
                self.push(
                    Node::Integrate {
                        x: x.node,
                        initial: initial.node,
                        step,
                    },
                    Type::Real,
                    NodeKind::Waveform { captured: false },
                )
            }
            "sine" => {
                args.positional_len(name, 0)?;
                let period = args.required_duration("period")?;
                let amplitude = self.generator_number(&mut args, "amplitude", 1.0)?;
                let offset = self.generator_number(&mut args, "offset", 0.0)?;
                let phase = self.generator_number(&mut args, "phase", 0.0)?;
                let step = args.named_duration("step")?;
                args.finish(name)?;
                self.push(
                    Node::Sine {
                        period,
                        amplitude: amplitude.node,
                        offset: offset.node,
                        phase: phase.node,
                        step,
                    },
                    Type::Real,
                    NodeKind::Waveform { captured: false },
                )
            }
            "clock" => {
                args.positional_len(name, 0)?;
                let period = args.required_duration("period")?;
                let high = args.required_duration("high")?;
                let phase = args.named_duration("phase")?.unwrap_or(Duration {
                    digits: 0,
                    scale: 0,
                    unit: DurationUnit::Ticks,
                });
                args.finish(name)?;
                self.push(
                    Node::Clock {
                        period,
                        high,
                        phase,
                    },
                    Type::Boolean,
                    NodeKind::Waveform { captured: false },
                )
            }
            "interval_mean" | "interval_integral" | "interval_min" | "interval_max"
            | "interval_sum" => {
                let x = args.one_expression(name)?;
                ensure!(
                    x.ty == Type::Real && x.kind.is_waveform_or_literal(),
                    "{name} requires a Real waveform."
                );
                let reduction = match name {
                    "interval_mean" => Reduction::Mean,
                    "interval_integral" => Reduction::Integral,
                    "interval_min" => Reduction::Min,
                    "interval_max" => Reduction::Max,
                    _ => Reduction::Sum,
                };
                if reduction == Reduction::Sum {
                    ensure!(
                        x.kind == NodeKind::Waveform { captured: true },
                        "interval_sum needs sample times. Add @ after the entire value expression, for example: interval_sum((a - b) @ 1 us)."
                    );
                }
                self.push(
                    Node::Reduce {
                        reduction,
                        x: x.node,
                    },
                    Type::Real,
                    NodeKind::Scalar,
                )
            }
            _ => bail!("Unknown function '{name}'."),
        }
    }

    fn real_waveform(value: Expr, name: &str) -> Result<()> {
        ensure!(
            value.ty == Type::Real && value.kind.is_waveform_or_literal(),
            "{name} requires a Real waveform."
        );
        Ok(())
    }

    fn literal_real(value: Expr, name: &str) -> Result<()> {
        ensure!(
            value.ty == Type::Real && value.kind == NodeKind::Literal,
            "{name} must be a finite numeric literal expression."
        );
        Ok(())
    }

    fn generator_number(&mut self, args: &mut Arguments, name: &str, default: f64) -> Result<Expr> {
        match args.named_expression(name)? {
            Some(value) => {
                Self::literal_real(value, name)?;
                Ok(value)
            }
            None => self.number(default),
        }
    }
}

#[derive(Default)]
struct Arguments {
    positional: Vec<Argument>,
    named: Vec<(String, Argument)>,
}

enum Argument {
    Expression(Expr),
    Duration(Duration),
    Policy(&'static str),
}

impl Arguments {
    fn positional_len(&self, name: &str, count: usize) -> Result<()> {
        ensure!(
            self.positional.len() == count,
            "{name} takes {count} positional arguments."
        );
        Ok(())
    }

    fn expression(&self, index: usize) -> Result<Expr> {
        match self.positional.get(index) {
            Some(Argument::Expression(value)) => Ok(*value),
            _ => bail!("Expected an expression argument."),
        }
    }

    fn duration(&self, index: usize) -> Result<Duration> {
        match self.positional.get(index) {
            Some(Argument::Duration(duration)) => Ok(duration.clone()),
            _ => bail!("Expected a duration argument."),
        }
    }

    fn one_expression(self, name: &str) -> Result<Expr> {
        self.positional_len(name, 1)?;
        let value = self.expression(0)?;
        self.finish(name)?;
        Ok(value)
    }

    fn two_expressions(self, name: &str) -> Result<(Expr, Expr)> {
        self.positional_len(name, 2)?;
        let values = (self.expression(0)?, self.expression(1)?);
        self.finish(name)?;
        Ok(values)
    }

    fn named(&mut self, name: &str) -> Option<Argument> {
        let index = self.named.iter().position(|(other, _)| other == name)?;
        Some(self.named.remove(index).1)
    }

    fn named_expression(&mut self, name: &str) -> Result<Option<Expr>> {
        match self.named(name) {
            Some(Argument::Expression(value)) => Ok(Some(value)),
            Some(_) => bail!("Argument '{name}' must be an expression."),
            None => Ok(None),
        }
    }

    fn named_duration(&mut self, name: &str) -> Result<Option<Duration>> {
        match self.named(name) {
            Some(Argument::Duration(duration)) => Ok(Some(duration)),
            Some(_) => bail!("Argument '{name}' must be a duration."),
            None => Ok(None),
        }
    }

    fn required_duration(&mut self, name: &str) -> Result<Duration> {
        self.named_duration(name)?
            .with_context(|| format!("Missing required named argument '{name}'."))
    }

    fn named_policy(&mut self, name: &str) -> Result<Option<&'static str>> {
        match self.named(name) {
            Some(Argument::Policy(policy)) => Ok(Some(policy)),
            Some(_) => bail!("Argument '{name}' must be a policy literal."),
            None => Ok(None),
        }
    }

    fn finish(&self, name: &str) -> Result<()> {
        ensure!(
            self.named.is_empty(),
            "Unknown argument '{}' for {name}.",
            self.named[0].0
        );
        Ok(())
    }
}
