//! Draft computations publish atomically and become one undoable row edit on Apply.

use super::App;
use crate::computed::{
    Command, ComputationStatus, Definition, Draft, DraftPreview, Editor, EditorTool, Input,
    InputDraft, Interpretation, Key, Operation, input_domain,
};
use crate::panels::PanelId;
use crate::trace::Traced;
use crate::wave::model::{DraftRows, RowSource};
use anyhow::{Context, Result, ensure};
use std::collections::HashSet;
use volna_trace::data::{SignalShape, VarId};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Create,
    Edit,
    Measure,
}

impl Editor {
    fn prepare(&self, doc: &crate::Document) -> Result<(Vec<Input>, Definition, Key)> {
        let inputs = self.inputs(doc)?;
        let definition = self.definition(doc.limits(), doc.time_base(), &inputs)?;
        let key = Key::new(doc, &inputs, &definition)?;
        Ok((inputs, definition, key))
    }

    fn refresh_preview(&mut self, doc: &crate::Document) -> Result<()> {
        self.preview = DraftPreview::Active(None);
        let (inputs, definition, key) = self.prepare(doc)?;
        self.domain = key.domain();
        self.display_definition = definition;
        self.display_inputs = inputs;
        self.preview = DraftPreview::Active(Some(key));
        self.error = None;
        Ok(())
    }

    fn track_row(&mut self, row: Option<usize>, revision: u64) {
        if let Some(row) = row {
            self.row = row;
            self.revision = revision;
        }
    }
}

impl App {
    #[must_use]
    pub fn derived_preview(&self) -> Option<&ComputationStatus> {
        let editor = self.derived_editor.as_ref()?;
        if editor.error.is_some() {
            return None;
        }
        self.computations.status(editor.preview_key()?)
    }
    pub fn recorded_input(&self, var: Traced<VarId>) -> Result<Input> {
        let v = self
            .doc
            .hierarchy(var.trace)
            .and_then(|h| h.get_var(var.item))
            .context("Recorded signal is unavailable.")?;
        let kind = if v.shape == SignalShape::Real {
            crate::data::NumericKind::Real
        } else {
            crate::data::NumericKind::Unsigned
        };
        let input = Input {
            alias: "input".into(),
            source: RowSource::Resolved {
                trace: var.trace,
                var: var.item,
                signal: v.signal,
            },
            shape: v.shape,
            interpretation: Interpretation {
                kind,
                projection: None,
            },
        };
        input.validate()?;
        Ok(input)
    }
    pub(super) fn derived_command(&mut self, panel: PanelId, command: Command) {
        if let Err(error) = self.edit_derived(panel, command) {
            if let Some(editor) = &mut self.derived_editor {
                editor.error = Some(format!("{error:#}"));
            } else {
                self.events.push(crate::Event::Notice(format!("{error:#}")));
            }
        }
        self.sync_derived_row();
        self.changed();
    }
    fn open_formula(&mut self, panel: PanelId, row: Option<usize>, purpose: Purpose) -> Result<()> {
        let waves = self
            .panels
            .waves(panel)
            .context("Waveform panel was closed.")?;
        let row = match row {
            Some(row) if purpose != Purpose::Create || !waves.selected.contains(&row) => row,
            _ => *waves
                .selected
                .last()
                .context("Select waveform inputs, then choose Derive.")?,
        };
        let signal = waves
            .signal(row)
            .context("Select a recorded numeric signal.")?;
        let (mut inputs, definition) = if purpose == Purpose::Edit {
            let RowSource::Computed { inputs, definition } = &signal.source else {
                anyhow::bail!("Select an applied formula to edit.");
            };
            (inputs.clone(), Some(definition))
        } else {
            let selected = if purpose == Purpose::Create && waves.selected.contains(&row) {
                waves.selected.iter().copied().collect::<Vec<_>>()
            } else {
                vec![row]
            };
            ensure!(
                selected.len() <= crate::computed::formula::MAX_INPUTS,
                "Select at most 16 waveform inputs."
            );
            let mut inputs = Vec::with_capacity(selected.len());
            for r in selected {
                let s = waves
                    .signal(r)
                    .context("Select recorded numeric signals.")?;
                let alias = unique_alias(&s.name, inputs.iter().map(|i: &Input| i.alias.as_str()));
                inputs.push(waveform_input(s, alias)?);
            }
            (inputs, None)
        };
        // Unavailable inputs must remain editable; Preview requires a current domain.
        let domain = input_domain(&inputs, &self.doc).unwrap_or_else(|_| self.doc.limits());
        let interval = (domain.1 - domain.0).div_ceil(2000).max(1);
        let mut draft = Draft {
            name: format!("{}_derived", signal.name),
            interval: duration_text(interval, self.doc.time_base()),
            skip_unknown: true,
            formula: inputs
                .first()
                .map_or_else(|| "0".into(), |input| input.alias.clone()),
            scope: crate::computed::MeasurementScope::WholeRange,
        };
        if inputs.len() > 1 {
            draft.name = "formula".into();
        }
        if let Some(d) = definition {
            draft.name.clone_from(&d.name);
            match &d.operation {
                Operation::Formula {
                    expression,
                    settings,
                } => {
                    draft.formula.clone_from(expression);
                    draft.interval = duration_text(settings.default_step, self.doc.time_base());
                    draft.skip_unknown = settings.unknown == crate::computed::UnknownPolicy::Skip;
                    draft.scope = settings.scope.clone();
                }
            }
        }
        if purpose == Purpose::Measure {
            let value = if inputs[0].ty() == crate::computed::FormulaType::Boolean {
                format!("real({})", draft.formula)
            } else {
                draft.formula
            };
            draft.formula = format!("interval_mean({value})");
            draft.name = format!("{} measurement", signal.name);
        }
        if purpose == Purpose::Create
            && let Some(first) = inputs.first()
            && inputs.iter().all(|input| {
                self.doc.traces().iter().all(|trace| {
                    trace.id == first.source.trace() || !input.source.uses_trace(trace.id)
                })
            })
            && let Some(choice) = self.doc.sampling.default_for(first.source.trace())
        {
            let mut sampled_draft = draft.clone();
            let mut sampled_inputs = inputs.clone();
            match self.sample_draft(&mut sampled_draft, &mut sampled_inputs, choice) {
                Ok(()) => {
                    draft = sampled_draft;
                    inputs = sampled_inputs;
                }
                Err(error) => self.events.push(crate::Event::Notice(format!(
                    "Default sampling was not applied: {error:#}"
                ))),
            }
        }
        let (definition_id, definition_revision) = definition
            .as_ref()
            .map(|definition| (definition.id, definition.revision))
            .unwrap_or_default();
        let mut display_definition =
            draft.definition(self.doc.limits(), self.doc.time_base(), &inputs)?;
        display_definition.id = definition_id;
        display_definition.revision = definition_revision;
        let tool = if purpose == Purpose::Measure {
            let suspended = self
                .derived_editor
                .take()
                .and_then(|editor| match editor.tool {
                    EditorTool::Measure { suspended } => suspended,
                    EditorTool::Formula => Some(Box::new(editor)),
                });
            EditorTool::Measure { suspended }
        } else {
            EditorTool::Formula
        };
        self.next_derived_editor += 1;
        self.derived_editor = Some(Editor {
            id: self.next_derived_editor,
            tool,
            panel,
            row,
            revision: waves.revision(),
            definition_id,
            definition_revision,
            bindings: inputs
                .iter()
                .cloned()
                .map(|i| InputDraft::new(i, &self.doc))
                .collect(),
            editing: definition.is_some(),
            draft,
            domain,
            preview: DraftPreview::Idle,
            calculation_revision: 0,
            error: None,
            display_definition,
            display_inputs: inputs,
            reveal_preview: true,
        });
        if let Some(waves) = self.panels.waves_mut(panel) {
            waves.menu_dismiss();
        }
        self.edit_derived(panel, Command::Preview)
    }
    fn close_formula(&mut self) -> Result<()> {
        let Some(Editor {
            tool: EditorTool::Measure {
                suspended: Some(parked),
            },
            ..
        }) = self.derived_editor.take()
        else {
            return Ok(());
        };
        let mut editor = *parked;
        let Some(waves) = self.panels.waves(editor.panel) else {
            return Ok(());
        };
        editor.track_row(waves.draft_rows.suspended, waves.revision());
        let panel = editor.panel;
        let resume = !editor.stopped() && editor.revision == waves.revision();
        editor.reveal_preview = true;
        self.derived_editor = Some(editor);
        if resume {
            self.edit_derived(panel, Command::Preview)?;
        }
        Ok(())
    }
    fn edit_derived(&mut self, panel: PanelId, command: Command) -> Result<()> {
        match command {
            Command::Create(row) => return self.open_formula(panel, row, Purpose::Create),
            Command::Edit(row) => return self.open_formula(panel, Some(row), Purpose::Edit),
            Command::Measure(row) => return self.open_formula(panel, Some(row), Purpose::Measure),
            _ => {}
        }
        if let Command::Sample {
            var,
            mode,
            make_default,
        } = command
        {
            self.sample_editor(panel, var, mode, make_default)?;
            return self.edit_derived(panel, Command::Preview);
        }
        if command == Command::SampleOnInputChanges {
            let editor = self
                .derived_editor
                .as_mut()
                .filter(|e| e.panel == panel)
                .context("Open the formula editor first.")?;
            let sampling_aliases = crate::computed::sampling_only_aliases(&editor.draft.formula)?;
            editor.draft.formula = crate::computed::input_change_expression(&editor.draft.formula)?;
            editor
                .bindings
                .retain(|input| !sampling_aliases.contains(&input.alias));
            editor.invalidate_preview();
            return self.edit_derived(panel, Command::Preview);
        }
        let replacement = match &command {
            Command::AddInput(var) | Command::Rebind(_, var) => Some(self.recorded_input(*var)?),
            _ => None,
        };
        let editor = self
            .derived_editor
            .as_mut()
            .filter(|e| e.panel == panel)
            .context("Open the derived-signal editor first.")?;
        let mut inputs_changed = false;
        match command {
            Command::Update(draft) => {
                if editor.draft.calculation_changed(&draft) {
                    editor.invalidate_preview();
                }
                editor.draft = draft;
            }
            Command::BindRow(row) => {
                let waves = self
                    .panels
                    .waves_mut(panel)
                    .context("Waveform panel was closed.")?;
                waves.menu_dismiss();
                let signal = waves.signal(row).context("Input row was removed.")?;
                ensure!(
                    !editor.editing || editor.row != row,
                    "A formula cannot bind itself."
                );
                ensure!(
                    editor.bindings.len() < crate::computed::formula::MAX_INPUTS,
                    "Use at most 16 waveform inputs."
                );
                let alias = unique_alias(
                    &signal.name,
                    editor.bindings.iter().map(|i| i.alias.as_str()),
                );
                editor
                    .bindings
                    .push(InputDraft::new(waveform_input(signal, alias)?, &self.doc));
                editor.invalidate_preview();
                return self.edit_derived(panel, Command::Preview);
            }
            Command::AddInput(_) => {
                ensure!(
                    editor.bindings.len() < crate::computed::formula::MAX_INPUTS,
                    "Use at most 16 recorded or applied waveform inputs."
                );
                let mut input = replacement.unwrap();
                let name = input
                    .source
                    .locator(&self.doc)
                    .0
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "input".into());
                input.alias = unique_alias(&name, editor.bindings.iter().map(|i| i.alias.as_str()));
                editor.bindings.push(InputDraft::new(input, &self.doc));
                inputs_changed = true;
            }
            Command::Rebind(index, _) => {
                let old = editor.bindings.get(index).context("Input was removed.")?;
                let mut input = replacement.unwrap();
                input.alias.clone_from(&old.alias);
                editor.bindings[index] = InputDraft::new(input, &self.doc);
                inputs_changed = true;
            }
            Command::RemoveInput(index) => {
                ensure!(index < editor.bindings.len(), "Input was removed.");
                editor.bindings.remove(index);
                inputs_changed = true;
            }
            Command::SetInput {
                index,
                alias,
                kind,
                slice,
            } => {
                let input = editor
                    .bindings
                    .get_mut(index)
                    .context("Input was removed.")?;
                if input.alias != alias || input.kind != kind || input.slice != slice {
                    input.alias = alias;
                    input.kind = kind;
                    input.slice = slice;
                    inputs_changed = true;
                }
            }
            Command::Preview => {
                editor.refresh_preview(&self.doc)?;
                self.computations.retry();
                return Ok(());
            }
            Command::Stop => {
                editor.preview = DraftPreview::Stopped;
                editor.error = None;
                return Ok(());
            }
            Command::Close => return self.close_formula(),
            Command::ResumeFormula => {
                let original = editor
                    .suspended_formula_mut()
                    .context("No suspended formula to resume.")?
                    .panel;
                if original != panel && self.panels.waves(original).is_some() {
                    self.panel_command(crate::panels::PanelsCommand::Focus(original));
                }
                return self.close_formula();
            }
            Command::Unpin(index) => {
                let pins = &mut self
                    .panels
                    .waves_mut(panel)
                    .context("Waveform panel was closed.")?
                    .pinned_readings;
                if index < pins.len() {
                    pins.remove(index);
                }
                return Ok(());
            }
            Command::Pin | Command::Apply | Command::ApplyTable => {
                let (inputs, mut definition, current) = editor.prepare(&self.doc)?;
                editor.domain = current.domain();
                ensure!(
                    editor.preview_key() == Some(&current),
                    "Wait for the current draft to finish updating."
                );
                let output = self
                    .computations
                    .status(&current)
                    .and_then(ComputationStatus::output)
                    .context("Wait for the live preview to finish updating.")?;
                if command == Command::Pin {
                    let crate::computed::DerivedOutput::Measurement(reading) = output else {
                        anyhow::bail!("Select a single interval to pin.");
                    };
                    ensure!(
                        reading.value.is_some(),
                        "The interval has no defined reading."
                    );
                    let pin = crate::computed::PinnedReading {
                        expression: editor.draft.formula.clone(),
                        sources: inputs
                            .iter()
                            .map(|input| format!("{} = {}", input.alias, input.label(&self.doc)))
                            .collect::<Vec<_>>()
                            .join(", "),
                        reading,
                    };
                    self.panels
                        .waves_mut(panel)
                        .context("Waveform panel was closed.")?
                        .pinned_readings
                        .push(pin);
                    return Ok(());
                }
                if matches!(command, Command::ApplyTable) {
                    let data = crate::table::statistics::Statistics::from_output(
                        definition.name,
                        &output,
                        editor.domain,
                        &self.viewer_memory_budget(),
                    )?;
                    self.try_open_table_source(
                        crate::table::TableSource::Statistics(std::sync::Arc::new(data)),
                        panel,
                    )?;
                    return self.close_formula();
                }
                let Operation::Formula { settings, .. } = &mut definition.operation;
                match &output {
                    crate::computed::DerivedOutput::IntervalTable(rows) => {
                        settings.scope = crate::computed::MeasurementScope::IntervalSnapshot {
                            intervals: rows.iter().map(|row| row.bounds).collect(),
                        };
                    }
                    crate::computed::DerivedOutput::Measurement(value) => {
                        settings.scope = crate::computed::MeasurementScope::Explicit {
                            start: value.bounds.0,
                            end: value.bounds.1,
                        };
                    }
                    crate::computed::DerivedOutput::Waveform(_) => {}
                }
                let count: usize = self
                    .panels
                    .iter()
                    .filter_map(|p| p.kind.waves())
                    .map(|w| w.items().len())
                    .sum();
                ensure!(
                    editor.editing || count < crate::workspace::MAX_ROWS,
                    "Workspace row limit reached."
                );
                if definition.id.0 == 0 {
                    self.next_formula_id = self
                        .next_formula_id
                        .checked_add(1)
                        .context("Formula identities exhausted.")?;
                    definition.id = crate::computed::FormulaId(self.next_formula_id);
                }
                definition.revision = if editor.editing {
                    editor
                        .definition_revision
                        .checked_add(1)
                        .context("Formula revision exhausted.")?
                } else {
                    0
                };
                self.panels
                    .waves_mut(panel)
                    .context("Waveform panel was closed.")?
                    .apply_derived(
                        crate::wave::model::DerivedPlacement {
                            row: editor.row,
                            revision: editor.revision,
                            editing: editor.editing,
                        },
                        definition,
                        inputs,
                        output,
                        &self.doc.translators,
                    )?;
                return self.close_formula();
            }
            Command::Remove => {
                ensure!(editor.editing, "This definition has not been added yet.");
                let waves = self
                    .panels
                    .waves_mut(panel)
                    .context("Waveform panel was closed.")?;
                ensure!(
                    waves.revision() == editor.revision,
                    "Rows changed while editing; reopen the editor."
                );
                waves.selected = std::collections::BTreeSet::from([editor.row]);
                waves.remove_selected();
                self.derived_editor = None;
                return Ok(());
            }
            Command::Create(_)
            | Command::Edit(_)
            | Command::Measure(_)
            | Command::Sample { .. }
            | Command::SampleOnInputChanges => {
                unreachable!()
            }
        }
        if inputs_changed {
            editor.invalidate_preview();
        }
        let (inputs, definition, key) = editor.prepare(&self.doc)?;
        editor.domain = key.domain();
        editor.display_inputs = inputs;
        editor.display_definition = definition;
        editor.error = None;
        Ok(())
    }
    pub(super) fn derived_demand(&self) -> HashSet<Key> {
        self.panels
            .iter()
            .filter_map(|p| p.kind.waves())
            .flat_map(crate::wave::WaveModel::items)
            .filter_map(|e| {
                let RowSource::Computed { inputs, definition } = &e.signal()?.source else {
                    return None;
                };
                Key::new(&self.doc, inputs, definition).ok()
            })
            .chain(
                self.derived_editor
                    .as_ref()
                    .filter(|e| e.error.is_none())
                    .and_then(|e| e.preview_key().cloned()),
            )
            .collect()
    }
    pub(super) fn derived_sources(&self) -> HashSet<Traced<volna_trace::data::SignalRef>> {
        self.derived_demand()
            .iter()
            .flat_map(Key::signals)
            .collect()
    }

    pub(super) fn sync_derived(&mut self) -> Vec<crate::LoadRequest> {
        if let Some(editor) = &self.derived_editor
            && self.panels.waves(editor.panel).is_none()
        {
            self.derived_command(editor.panel, Command::Close);
        }
        self.doc.formula_definitions.clear();
        for (inputs, definition) in self
            .panels
            .iter()
            .filter_map(|panel| panel.kind.waves())
            .flat_map(crate::wave::WaveModel::items)
            .filter_map(|entry| match &entry.signal()?.source {
                RowSource::Computed { inputs, definition } => Some((inputs, definition)),
                _ => None,
            })
        {
            self.next_formula_id = self.next_formula_id.max(definition.id.0);
            self.doc
                .formula_definitions
                .entry(definition.id)
                .and_modify(|current| *current = None)
                .or_insert_with(|| Some((inputs.clone(), definition.clone())));
        }
        self.refresh_derived_preview();
        let wanted = self.derived_demand();
        let sources: HashSet<_> = wanted.iter().flat_map(Key::signals).collect();
        let resident = self.resident_histories();
        let requests = self.computations.sync(
            wanted,
            &sources,
            &resident,
            self.doc.generation(),
            &self.viewer_memory_budget(),
        );
        for signal in sources {
            if self.computations.needs_source(signal) {
                self.doc.request_signal(signal);
            }
        }
        self.attach_derived();
        requests
    }

    fn refresh_derived_preview(&mut self) {
        if let Some(editor) = &mut self.derived_editor
            && self
                .panels
                .waves(editor.panel)
                .is_some_and(|waves| waves.revision() != editor.revision)
        {
            editor.preview = DraftPreview::Idle;
            editor.error = Some(
                "The source row changed or was removed. Reopen the derived-signal editor.".into(),
            );
            return;
        }
        let Some(editor) = self
            .derived_editor
            .as_mut()
            .filter(|editor| matches!(editor.preview, DraftPreview::Active(_)))
        else {
            return;
        };
        if let Err(error) = editor.refresh_preview(&self.doc) {
            editor.error = Some(format!("{error:#}"));
        }
    }
    pub(super) fn attach_derived(&mut self) {
        for waves in self.panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
            waves.sync_derived(&self.doc, &self.computations);
        }
        self.sync_derived_row();
    }
    pub(super) fn sync_derived_source(&mut self) {
        let Some(editor) = &mut self.derived_editor else {
            return;
        };
        if let Some(suspended) = editor.suspended_formula_mut()
            && let Some(waves) = self.panels.waves(suspended.panel)
        {
            suspended.track_row(waves.draft_rows.suspended, waves.revision());
        }
        let Some(waves) = self.panels.waves(editor.panel) else {
            return;
        };
        let revision = waves.revision();
        if editor.revision != revision {
            editor.track_row(waves.draft_rows.active, revision);
            self.sync_derived_row();
        }
    }

    fn sync_derived_row(&mut self) {
        for waves in self.panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
            waves.preview = None;
            waves.draft_rows = DraftRows::default();
        }
        let Some(editor) = &mut self.derived_editor else {
            return;
        };
        if let Some(suspended) = editor.suspended_formula_mut()
            && let Some(waves) = self.panels.waves_mut(suspended.panel)
            && suspended.revision == waves.revision()
        {
            waves.draft_rows.suspended = Some(suspended.row);
        }
        let Some(waves) = self.panels.waves_mut(editor.panel) else {
            return;
        };
        let revision = waves.revision();
        let active = (editor.revision == revision).then_some(editor.row);
        waves.draft_rows.active = active;
        let Some(row) = active else {
            return;
        };
        let computation = editor
            .preview_key()
            .and_then(|key| self.computations.status(key));
        let output = computation.and_then(ComputationStatus::output);
        let status = if editor.error.is_some() {
            "Invalid settings"
        } else if editor.stopped() {
            "Stopped"
        } else {
            match computation {
                _ if output.is_some() => "Live preview",
                Some(ComputationStatus::Waiting) => "Loading input…",
                Some(ComputationStatus::Failed(_)) => "Failed",
                _ => "Updating…",
            }
        };
        let Ok((at, mut entry, editing)) = waves.derived_row(
            crate::wave::model::DerivedPlacement {
                row,
                revision,
                editing: editor.editing,
            },
            editor.display_definition.clone(),
            editor.display_inputs.clone(),
            output.filter(|_| editor.error.is_none()),
            &self.doc.translators,
        ) else {
            return;
        };
        if let Some(signal) = entry.signal_mut()
            && signal.height.is_default()
        {
            signal.height = signal.height.step(1);
        }
        waves.preview = Some(crate::wave::model::PreviewRow {
            revision,
            index: if editing { at } else { waves.items().len() },
            before: at,
            entry,
            status: status.into(),
        });
    }
}

fn waveform_input(signal: &crate::wave::model::DisplayedSignal, alias: String) -> Result<Input> {
    let kind = if signal.source.computed().is_some() {
        if signal.source_shape == SignalShape::Real {
            crate::data::NumericKind::Real
        } else {
            crate::data::NumericKind::Unsigned
        }
    } else {
        ensure!(
            signal.source.signal().is_some(),
            "Choose a recorded or applied waveform."
        );
        signal
            .numeric_kind()
            .context("Choose a numeric input format.")?
    };
    let input = Input {
        alias,
        source: signal.source.clone(),
        shape: signal.source_shape,
        interpretation: Interpretation {
            kind,
            projection: signal.projection,
        },
    };
    input.validate()?;
    Ok(input)
}

fn duration_text(ticks: u64, base: crate::wave::timeline::TimeBase<'_>) -> String {
    let text = crate::wave::timeline::format_time(ticks as f64, base);
    if crate::moving_average::duration(&text, base).ok() == Some(ticks) {
        text
    } else {
        format!("{ticks} ticks")
    }
}

pub(super) fn unique_alias<'a>(name: &str, inputs: impl Iterator<Item = &'a str>) -> String {
    let aliases = inputs.collect::<HashSet<_>>();
    let base = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect::<String>();
    let base = if crate::computed::formula::valid_alias(&base)
        && !crate::computed::language::is_function(&base)
    {
        base
    } else {
        "input".into()
    };
    if !aliases.contains(base.as_str()) {
        return base;
    }
    for n in 2.. {
        let alias = format!("{base}_{n}");
        if !aliases.contains(alias.as_str()) {
            return alias;
        }
    }
    unreachable!()
}
