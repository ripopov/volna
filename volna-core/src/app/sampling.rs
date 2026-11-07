//! Register sampling choices without changing existing formula definitions.

use super::App;
use crate::{
    Event,
    computed::{Draft, Input, InputDraft},
    sampling::{SamplingChoice, SamplingCommand},
};
use anyhow::{Context, Result, ensure};

impl App {
    pub(super) fn sampling_command(&mut self, command: SamplingCommand) {
        let result = (|| -> Result<()> {
            match command {
                SamplingCommand::Register(var, mode) => {
                    let choice = SamplingChoice::new(&self.doc, var, mode)?;
                    let first = self.doc.sampling.default_for(var.trace).is_none();
                    self.doc.sampling.register(choice.clone())?;
                    self.events.push(Event::Notice(format!(
                        "{}: {}",
                        if first {
                            "Default sampling"
                        } else {
                            "Sampling choice added"
                        },
                        choice.label()
                    )));
                }
                SamplingCommand::MakeDefault(choice) => {
                    ensure!(
                        choice.resolve(&self.doc).is_some(),
                        "Sampling signal is unavailable."
                    );
                    self.doc.sampling.make_default(choice.clone())?;
                    self.events.push(Event::Notice(format!(
                        "Default sampling: {}",
                        choice.label()
                    )));
                }
                SamplingCommand::Remove(choice) => {
                    self.doc.sampling.remove(&choice);
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.events.push(Event::Notice(format!("{error:#}")));
        }
        self.changed();
    }

    pub(super) fn sample_draft(
        &self,
        draft: &mut Draft,
        inputs: &mut Vec<Input>,
        choice: &SamplingChoice,
    ) -> Result<()> {
        let sampling_aliases = crate::computed::sampling_only_aliases(&draft.formula)?;
        inputs.retain(|input| !sampling_aliases.contains(&input.alias));
        let var = choice
            .resolve(&self.doc)
            .context("Sampling signal is unavailable; choose another signal.")?;
        let mut input = self.recorded_input(var)?;
        let alias = if let Some(existing) = inputs
            .iter()
            .find(|i| i.source == input.source && i.interpretation == input.interpretation)
        {
            existing.alias.clone()
        } else {
            ensure!(
                inputs.len() < crate::computed::formula::MAX_INPUTS,
                "Use at most 16 formula inputs."
            );
            input.alias = super::derived::unique_alias(
                choice.path.last().map_or("sample", String::as_str),
                inputs.iter().map(|i| i.alias.as_str()),
            );
            let alias = input.alias.clone();
            inputs.push(input);
            alias
        };
        draft.formula = crate::computed::capture_expression(&draft.formula, choice.mode, &alias)?;
        crate::computed::Program::with_bindings(&draft.formula, inputs)?;
        Ok(())
    }

    pub(super) fn sample_editor(
        &mut self,
        panel: crate::panels::PanelId,
        var: crate::trace::Traced<volna_trace::data::VarId>,
        mode: crate::sampling::SamplingMode,
        make_default: bool,
    ) -> Result<()> {
        let choice = SamplingChoice::new(&self.doc, var, mode)?;
        let editor = self
            .derived_editor
            .as_ref()
            .filter(|e| e.panel == panel)
            .context("Open the formula editor first.")?;
        let mut inputs = editor.inputs(&self.doc)?;
        let mut draft = editor.draft.clone();
        self.sample_draft(&mut draft, &mut inputs, &choice)?;
        if make_default {
            self.doc.sampling.make_default(choice)?;
        } else {
            self.doc.sampling.register(choice)?;
        }
        let editor = self.derived_editor.as_mut().unwrap();
        editor.draft = draft;
        editor.bindings = inputs
            .into_iter()
            .map(|input| InputDraft::new(input, &self.doc))
            .collect();
        editor.calculation_revision += 1;
        editor.preview = None;
        editor.stopped = false;
        editor.error = None;
        Ok(())
    }
}
