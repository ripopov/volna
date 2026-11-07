//! Sampling controls edit explicit captures, leaving saved formulas self-contained.

use super::{
    Editor,
    language::{Syntax, TokenKind},
};
use crate::sampling::{SamplingChoice, SamplingMode};
use anyhow::{Result, bail, ensure};
use std::ops::Range;

struct Capture {
    range: Range<usize>,
    binding: Option<(SamplingMode, String)>,
}
fn captures(expression: &str) -> Result<Vec<Capture>> {
    let syntax = Syntax::new(expression);
    let tokens = syntax.tokens();
    let mut captures = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if &expression[token.range.clone()] != "@" {
            continue;
        }
        let mut at = i + 1;
        let rising = tokens
            .get(at)
            .is_some_and(|t| &expression[t.range.clone()] == "posedge");
        if rising {
            at += 1;
        }
        let Some(trigger) = tokens.get(at) else {
            bail!("Finish the sampling expression first.")
        };
        let binding = match trigger.kind {
            TokenKind::Identifier => Some((
                if rising {
                    SamplingMode::RisingEdge
                } else {
                    SamplingMode::ValueChanges
                },
                expression[trigger.range.clone()].into(),
            )),
            TokenKind::Duration if !rising => None,
            _ => bail!(
                "Edit the explicit sampling expression directly before choosing another sampling signal."
            ),
        };
        ensure!(
            tokens
                .get(at + 1)
                .is_none_or(|t| &expression[t.range.clone()] != "("),
            "Edit the explicit sampling expression directly before choosing another sampling signal."
        );
        captures.push(Capture {
            range: token.range.start..trigger.range.end,
            binding,
        });
    }
    Ok(captures)
}

pub(crate) fn sampling_only_aliases(expression: &str) -> Result<Vec<String>> {
    let captures = captures(expression)?;
    let value = input_change_expression(expression)?;
    let syntax = Syntax::new(&value);
    let mut aliases = Vec::new();
    for capture in captures {
        if let Some((_, alias)) = capture.binding
            && !aliases.contains(&alias)
            && !syntax
                .tokens()
                .iter()
                .any(|token| value[token.range.clone()] == alias)
        {
            aliases.push(alias);
        }
    }
    Ok(aliases)
}

pub fn input_change_expression(expression: &str) -> Result<String> {
    let mut result = expression.to_owned();
    for capture in captures(expression)?.into_iter().rev() {
        result.replace_range(capture.range, "");
    }
    Ok(result.trim_end().to_owned())
}

pub fn capture_expression(expression: &str, mode: SamplingMode, alias: &str) -> Result<String> {
    let trigger = mode.trigger(alias);
    let captures = captures(expression)?;
    let mut result = expression.to_owned();
    if !captures.is_empty() {
        for capture in captures.into_iter().rev() {
            result.replace_range(capture.range, &trigger);
        }
        return Ok(result);
    }
    let syntax = Syntax::new(expression);
    let mut edits = Vec::new();
    for argument in syntax.measurement_arguments()? {
        edits.push((argument.start, "(".to_string()));
        edits.push((argument.end, format!(") {trigger}")));
    }
    if edits.is_empty() {
        return Ok(format!("({expression}) {trigger}"));
    }
    edits.sort_by_key(|(at, _)| *at);
    for (at, text) in edits.into_iter().rev() {
        result.insert_str(at, &text);
    }
    Ok(result)
}

impl Editor {
    pub fn value_bindings(&self) -> impl Iterator<Item = &super::InputDraft> {
        let aliases = sampling_only_aliases(&self.draft.formula).unwrap_or_default();
        self.bindings
            .iter()
            .filter(move |input| !aliases.contains(&input.alias))
    }

    #[must_use]
    pub fn sampling(&self, doc: &crate::Document) -> Option<SamplingChoice> {
        let captures = captures(&self.draft.formula).ok()?;
        let binding = captures.first()?.binding.as_ref()?;
        let inputs = self.inputs(doc).ok()?;
        if super::Program::with_bindings(&self.draft.formula, &inputs)
            .ok()?
            .has_uncaptured_values()
        {
            return None;
        }
        let (mode, alias) = binding;
        if !captures.iter().all(|c| c.binding.as_ref() == Some(binding)) {
            return None;
        }
        let input = self
            .bindings
            .iter()
            .find(|i| i.alias == *alias && i.slice.trim().is_empty())?;
        SamplingChoice::new(doc, input.source.var()?, *mode).ok()
    }
}
