//! Formula syntax shared by parsing and interactive editors.

use std::{fmt, ops::Range};

pub const FUNCTIONS: &[(&str, &str)] = &[
    ("abs", "abs(x)"),
    ("min", "min(a, b)"),
    ("max", "max(a, b)"),
    ("real", "real(boolean)"),
    ("bool", "bool(number)"),
    (
        "moving_avg",
        "moving_avg(x, window, step: …, warmup: \"full\" | \"available\", unknown: \"gap\" | \"skip\")",
    ),
    ("lowpass", "lowpass(x, tau, step: …)"),
    ("integrate", "integrate(x, initial: 0, step: …)"),
    (
        "sine",
        "sine(period: …, amplitude: 1, offset: 0, phase: 0, step: …)",
    ),
    ("clock", "clock(period: …, high: …, phase: …)"),
    ("interval_mean", "interval_mean(x)"),
    ("interval_integral", "interval_integral(x)"),
    ("interval_min", "interval_min(x)"),
    ("interval_max", "interval_max(x)"),
    ("interval_sum", "interval_sum(captured_waveform)"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub source: String,
    pub range: Range<usize>,
    pub message: String,
}

impl Diagnostic {
    #[must_use]
    pub fn from_error(expression: &str, error: &anyhow::Error) -> Self {
        error
            .downcast_ref::<Self>()
            .filter(|diagnostic| diagnostic.source == expression)
            .cloned()
            .unwrap_or_else(|| Self {
                source: expression.into(),
                range: 0..expression.len(),
                message: format!("{error:#}"),
            })
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Diagnostic {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Number,
    Duration,
    Identifier,
    Function,
    Keyword,
    String,
    Operator,
    Punctuation,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxToken {
    pub range: Range<usize>,
    pub kind: TokenKind,
    pub(crate) number_end: usize,
}

#[derive(Clone, Debug)]
pub struct Syntax {
    expression: String,
    pub(super) tokens: Vec<SyntaxToken>,
}

impl Syntax {
    #[must_use]
    pub fn new(expression: impl Into<String>) -> Self {
        let expression = expression.into();
        let tokens = scan(&expression);
        Self { expression, tokens }
    }

    #[must_use]
    pub fn expression(&self) -> &str {
        &self.expression
    }

    #[must_use]
    pub fn tokens(&self) -> &[SyntaxToken] {
        &self.tokens
    }

    #[must_use]
    pub fn signature(&self, offset: usize) -> Option<&'static str> {
        let mut stack = Vec::new();
        let mut previous = None;
        let expression = &self.expression;
        for token in self
            .tokens
            .iter()
            .take_while(|token| token.range.end <= offset)
        {
            match &expression[token.range.clone()] {
                "(" => stack.push(previous.and_then(|name| {
                    FUNCTIONS
                        .iter()
                        .find(|(function, _)| *function == name)
                        .map(|(_, signature)| *signature)
                })),
                ")" => {
                    stack.pop();
                }
                _ => {}
            }
            previous = Some(&expression[token.range.clone()]);
        }
        stack.into_iter().rev().flatten().next()
    }

    #[must_use]
    pub fn completion_prefix(&self, offset: usize) -> Option<&str> {
        let expression = &self.expression;
        expression.get(..offset)?;
        match self
            .tokens
            .iter()
            .find(|token| token.range.start < offset && offset <= token.range.end)
        {
            Some(token) => match token.kind {
                TokenKind::Identifier | TokenKind::Function | TokenKind::Keyword => {
                    expression.get(token.range.start..offset)
                }
                TokenKind::Number | TokenKind::Duration | TokenKind::String => None,
                _ => Some(""),
            },
            None => Some(""),
        }
    }
}

/// Retains incomplete tokens so an editor can highlight unfinished expressions.
fn scan(expression: &str) -> Vec<SyntaxToken> {
    let mut tokens = Vec::new();
    let bytes = expression.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        let start = at;
        let mut number_end = at;
        let kind = if bytes[at].is_ascii_digit() || bytes[at] == b'.' {
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            if bytes.get(at) == Some(&b'.') {
                at += 1;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
            }
            if matches!(bytes.get(at), Some(b'e' | b'E')) {
                at += 1;
                if matches!(bytes.get(at), Some(b'+' | b'-')) {
                    at += 1;
                }
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
            }
            number_end = at;
            let mut following = at;
            while bytes.get(following).is_some_and(u8::is_ascii_whitespace) {
                following += 1;
            }
            if bytes
                .get(following)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
            {
                at = following + 1;
                while bytes
                    .get(at)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    at += 1;
                }
                TokenKind::Duration
            } else {
                TokenKind::Number
            }
        } else if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            at += 1;
            while bytes
                .get(at)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
            {
                at += 1;
            }
            let word = &expression[start..at];
            if FUNCTIONS.iter().any(|(name, _)| *name == word) {
                TokenKind::Function
            } else if matches!(word, "true" | "false" | "posedge") {
                TokenKind::Keyword
            } else {
                TokenKind::Identifier
            }
        } else if bytes[at] == b'"' {
            at += 1;
            at = expression[at..]
                .find('"')
                .map_or(bytes.len(), |end| at + end + 1);
            TokenKind::String
        } else {
            let c = expression[at..]
                .chars()
                .next()
                .expect("remaining character");
            at += c.len_utf8();
            if matches!(c, '|' | '&' | '=' | '!' | '<' | '>') && bytes.get(at) == Some(&b'=')
                || matches!(c, '|' | '&') && bytes.get(at) == Some(&(c as u8))
            {
                at += 1;
            }
            if "+-*/%<>=!&|@".contains(c) {
                TokenKind::Operator
            } else if "(),:".contains(c) {
                TokenKind::Punctuation
            } else {
                TokenKind::Invalid
            }
        };
        tokens.push(SyntaxToken {
            range: start..at,
            kind,
            number_end,
        });
    }
    tokens
}
