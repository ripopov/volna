//! Formula editing support with live binding completion and function signatures.

use super::*;
use gpui_kit::component::input::{CompletionProvider, Rope};
use gpui_kit::{App, Task};
use lsp_types::{CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse};
use std::{rc::Rc, sync::Arc};
use volna_core::computed::language::{self, Diagnostic, FUNCTIONS};

pub(super) struct Analysis {
    pub syntax: language::Syntax,
    bindings: Vec<volna_core::computed::InputDraft>,
    generation: u64,
    pub result: Result<volna_core::computed::Program, Diagnostic>,
}

impl Analysis {
    fn new(
        expression: String,
        bindings: Vec<volna_core::computed::InputDraft>,
        doc: &volna_core::Document,
    ) -> Self {
        let result = bindings
            .iter()
            .map(|input| input.resolve(doc))
            .collect::<anyhow::Result<Vec<_>>>()
            .and_then(|inputs| volna_core::computed::Program::with_bindings(&expression, &inputs));
        let result = result.map_err(|error| Diagnostic::from_error(&expression, &error));
        Self {
            syntax: language::Syntax::new(expression),
            bindings,
            generation: doc.generation(),
            result,
        }
    }
}

impl DerivedEditor {
    pub(super) fn analysis(&mut self, cx: &Context<Self>) -> Option<Arc<Analysis>> {
        let owner = self.owner.upgrade()?;
        let ws = owner.read(cx);
        let editor = ws
            .app
            .derived_editor
            .as_ref()
            .filter(|editor| editor.id == self.id)?;
        let expression = self.expression.read(cx).value().to_string();
        if let Some(analysis) = &self.analysis
            && analysis.syntax.expression() == expression
            && analysis.bindings == editor.bindings
            && analysis.generation == ws.app.doc.generation()
        {
            return Some(analysis.clone());
        }
        let analysis = Arc::new(Analysis::new(
            expression,
            editor.bindings.clone(),
            &ws.app.doc,
        ));
        self.analysis = Some(analysis.clone());
        Some(analysis)
    }
}

struct Completions(WeakEntity<Workspace>);

pub(super) fn state(
    owner: &WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) -> EditorState {
    let mut state = EditorState::new(window, cx)
        .language("volna-formula")
        .line_number(false)
        .placeholder("Enter a formula, or choose a preset below");
    state.set_highlighter_factory(
        Rc::new(|_| {
            Some(Box::new(Highlighting {
                syntax: language::Syntax::new(""),
            }))
        }),
        cx,
    );
    state.lsp_mut().completion_provider = Some(Rc::new(Completions(owner.clone())));
    state
}

impl CompletionProvider for Completions {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let syntax = language::Syntax::new(text.to_string());
        let Some(prefix) = syntax.completion_prefix(offset) else {
            return Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        };
        let mut items = FUNCTIONS
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, signature)| CompletionItem {
                label: (*name).into(),
                detail: Some((*signature).into()),
                kind: Some(CompletionItemKind::FUNCTION),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        if let Some(owner) = self.0.upgrade()
            && let Some(editor) = &owner.read(cx).app.derived_editor
        {
            items.extend(
                editor
                    .bindings
                    .iter()
                    .filter(|input| input.alias.starts_with(prefix))
                    .map(|input| CompletionItem {
                        label: input.alias.clone(),
                        detail: Some(input.label(&owner.read(cx).app.doc)),
                        kind: Some(CompletionItemKind::VARIABLE),
                        ..Default::default()
                    }),
            );
        }
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, text: &str, _: &mut App) -> bool {
        !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }
}

struct Highlighting {
    syntax: language::Syntax,
}

impl gpui_kit::component::input::InputHighlighter for Highlighting {
    fn language(&self) -> gpui_kit::SharedString {
        "volna-formula".into()
    }

    fn update(
        &mut self,
        _: Option<gpui_kit::component::input::InputEdit>,
        text: &Rope,
        _: bool,
        _: &mut Window,
        _: &mut Context<EditorState>,
    ) {
        self.syntax = language::Syntax::new(text.to_string());
    }

    fn styles(
        &self,
        range: &std::ops::Range<usize>,
        resolver: &dyn gpui_kit::component::input::HighlightStyleResolver,
    ) -> Vec<(std::ops::Range<usize>, gpui_kit::HighlightStyle)> {
        let mut result = Vec::new();
        let mut cursor = range.start;
        for token in self.syntax.tokens() {
            let start = token.range.start.max(range.start);
            let end = token.range.end.min(range.end);
            if start >= end {
                continue;
            }
            if cursor < start {
                result.push((cursor..start, Default::default()));
            }
            result.push((
                start..end,
                resolver
                    .style(match token.kind {
                        language::TokenKind::Number | language::TokenKind::Duration => "number",
                        language::TokenKind::Identifier => "variable",
                        language::TokenKind::Function => "function",
                        language::TokenKind::Keyword => "keyword",
                        language::TokenKind::String => "string",
                        language::TokenKind::Operator => "operator",
                        language::TokenKind::Punctuation | language::TokenKind::Invalid => "",
                    })
                    .unwrap_or_default(),
            ));
            cursor = end;
        }
        if cursor < range.end {
            result.push((cursor..range.end, Default::default()));
        }
        result
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<gpui_kit::component::input::FoldRange> {
        Vec::new()
    }
}

pub(super) fn adapt_example(
    template: &str,
    inputs: &[volna_core::computed::InputDraft],
    doc: &volna_core::Document,
) -> String {
    let real = inputs
        .iter()
        .filter(|i| i.effective_shape(doc) != volna_trace::data::SignalShape::Bit)
        .collect::<Vec<_>>();
    let boolean = inputs
        .iter()
        .filter(|i| i.effective_shape(doc) == volna_trace::data::SignalShape::Bit)
        .collect::<Vec<_>>();
    let mut result = String::new();
    let mut cursor = 0;
    let syntax = language::Syntax::new(template);
    for token in syntax.tokens() {
        result.push_str(&template[cursor..token.range.start]);
        let word = &template[token.range.clone()];
        let binding = if token.kind == language::TokenKind::Identifier {
            match word {
                "a" => real.first(),
                "b" => real.get(1),
                "valid" => boolean.first(),
                "ready" => boolean.get(1),
                "clk" => boolean
                    .iter()
                    .find(|i| i.alias.contains("clk"))
                    .or_else(|| boolean.first()),
                _ => None,
            }
        } else {
            None
        };
        result.push_str(binding.map_or(word, |input| input.alias.as_str()));
        cursor = token.range.end;
    }
    result.push_str(&template[cursor..]);
    result
}
