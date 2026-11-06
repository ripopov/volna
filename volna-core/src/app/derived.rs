//! Draft computations publish atomically and become one undoable row edit on Apply.

use super::App;
use crate::computed::{
    Command, ComputationStatus, Draft, Editor, Input, InputDraft, Interpretation, Key, Operation,
    input_domain,
};
use crate::panels::PanelId;
use crate::trace::Traced;
use crate::wave::model::RowSource;
use anyhow::{Context, Result, ensure};
use std::collections::HashSet;
use volna_trace::data::{SignalShape, VarId};

impl App {
    #[must_use]
    pub fn derived_preview(&self) -> Option<&ComputationStatus> {
        self.computations
            .status(self.derived_editor.as_ref()?.preview.as_ref()?)
    }
    #[must_use]
    pub fn can_derive_signal(&self, panel: PanelId) -> bool {
        self.panels.waves(panel).is_some_and(|w| {
            !w.selected.is_empty()
                && w.selected.iter().all(|&r| {
                    w.signal(r).is_some_and(|s| {
                        if w.selected.len() == 1 {
                            s.numeric_kind().is_some() || s.source.computed().is_some()
                        } else {
                            (s.numeric_kind().is_some() || s.source.computed().is_some())
                                && (s.source.signal().is_some() || s.source.computed().is_some())
                        }
                    })
                })
        })
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
    fn edit_derived(&mut self, panel: PanelId, command: Command) -> Result<()> {
        if let Command::Open(row) = command {
            let waves = self
                .panels
                .waves(panel)
                .context("Waveform panel was closed.")?;
            let row = row
                .or_else(|| waves.selected.first().copied())
                .context("Select recorded numeric signals, then choose Derive.")?;
            let signal = waves
                .signal(row)
                .context("Select a recorded numeric signal.")?;
            let (inputs, definition) = if let RowSource::Computed { inputs, definition } =
                &signal.source
                && !(waves.selected.len() > 1 && waves.selected.contains(&row))
            {
                (inputs.clone(), Some(definition))
            } else {
                let selected = if waves.selected.contains(&row) {
                    waves.selected.iter().copied().collect::<Vec<_>>()
                } else {
                    vec![row]
                };
                let mut inputs = Vec::new();
                for r in selected {
                    let s = waves
                        .signal(r)
                        .context("Select recorded numeric signals.")?;
                    ensure!(
                        s.source.signal().is_some() || s.source.computed().is_some(),
                        "Choose recorded or applied waveform inputs."
                    );
                    let kind = if s.source.computed().is_some() {
                        if s.source_shape == SignalShape::Real {
                            crate::data::NumericKind::Real
                        } else {
                            crate::data::NumericKind::Unsigned
                        }
                    } else {
                        s.numeric_kind().context("Choose a numeric input format.")?
                    };
                    let mut input = Input {
                        alias: String::new(),
                        source: s.source.clone(),
                        shape: s.source_shape,
                        interpretation: Interpretation {
                            kind,
                            projection: s.projection,
                        },
                    };
                    input.alias =
                        unique_alias(&s.name, inputs.iter().map(|i: &Input| i.alias.as_str()));
                    input.validate()?;
                    inputs.push(input);
                }
                (inputs, None)
            };
            // Unavailable inputs must remain editable; Preview requires a current domain.
            let domain = input_domain(&inputs, &self.doc).unwrap_or_else(|_| self.doc.limits());
            let interval = (domain.1 - domain.0).div_ceil(2000).max(1);
            let mut draft = Draft {
                name: format!("{}_mean", signal.name),
                window: duration_text(interval.saturating_mul(16), self.doc.time_base()),
                interval: duration_text(interval, self.doc.time_base()),
                partial: false,
                skip_unknown: false,
                formula: None,
                scope: crate::computed::MeasurementScope::MarkerIntervals,
            };
            if inputs.len() > 1 {
                draft.name = "formula".into();
                draft.formula = Some(inputs[0].alias.clone());
            }
            if let Some(d) = definition {
                draft.name.clone_from(&d.name);
                match &d.operation {
                    Operation::MovingAverage { settings } => {
                        draft.window = duration_text(settings.window, self.doc.time_base());
                        draft.interval = duration_text(settings.interval, self.doc.time_base());
                        draft.partial = settings.partial;
                        draft.skip_unknown = settings.skip_unknown;
                    }
                    Operation::Formula {
                        expression,
                        settings,
                    } => {
                        draft.formula = Some(expression.clone());
                        draft.interval = duration_text(settings.default_step, self.doc.time_base());
                        draft.skip_unknown =
                            settings.unknown == crate::computed::UnknownPolicy::Skip;
                        draft.scope = settings.scope.clone();
                    }
                }
            }
            let (definition_id, definition_revision) = definition
                .as_ref()
                .map(|definition| (definition.id, definition.revision))
                .unwrap_or_default();
            let mut display_definition =
                draft.definition(domain, self.doc.limits(), self.doc.time_base(), &inputs)?;
            display_definition.id = definition_id;
            display_definition.revision = definition_revision;
            let last_result = definition.and_then(|_| {
                if let Some(history) = &signal.history {
                    Some(crate::computed::DerivedOutput::Waveform(history.clone()))
                } else {
                    signal
                        .measurement
                        .as_ref()
                        .map(|measurement| match measurement {
                            crate::computed::MeasurementResult::Scalar(measurement) => {
                                crate::computed::DerivedOutput::Measurement(*measurement)
                            }
                            crate::computed::MeasurementResult::Table(rows) => {
                                crate::computed::DerivedOutput::IntervalTable(rows.clone())
                            }
                        })
                }
            });
            self.next_derived_editor += 1;
            self.derived_editor = Some(Editor {
                id: self.next_derived_editor,
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
                preview: None,
                calculation_revision: 0,
                error: None,
                stopped: false,
                display_definition,
                display_inputs: inputs,
                last_result,
                reveal_preview: true,
            });
            if let Some(waves) = self.panels.waves_mut(panel) {
                waves.menu_dismiss();
            }
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
                    editor.calculation_revision += 1;
                    editor.preview = None;
                    editor.stopped = false;
                }
                editor.draft = draft;
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
                editor.preview = None;
                editor.stopped = false;
                let inputs = editor.inputs(&self.doc)?;
                editor.domain = input_domain(&inputs, &self.doc)?;
                let definition =
                    editor.definition(self.doc.limits(), self.doc.time_base(), &inputs)?;
                editor.preview = Some(Key::new(&self.doc, &inputs, &definition)?);
                editor.display_definition = definition;
                self.computations.retry();
            }
            Command::Stop => {
                editor.preview = None;
                editor.stopped = true;
                editor.error = None;
            }
            Command::Close => {
                self.derived_editor = None;
                return Ok(());
            }
            Command::Apply | Command::ApplyTable => {
                let inputs = editor.inputs(&self.doc)?;
                editor.domain = input_domain(&inputs, &self.doc)?;
                let mut definition =
                    editor.definition(self.doc.limits(), self.doc.time_base(), &inputs)?;
                let current = Key::new(&self.doc, &inputs, &definition)?;
                ensure!(
                    editor.preview.as_ref() == Some(&current),
                    "Wait for the current draft to finish updating."
                );
                let output = match self.computations.status(&current) {
                    Some(ComputationStatus::Ready(history)) => {
                        crate::computed::DerivedOutput::Waveform(history.clone())
                    }
                    Some(ComputationStatus::ReadyMeasurement(measurement)) => {
                        crate::computed::DerivedOutput::Measurement(*measurement)
                    }
                    Some(ComputationStatus::ReadyTable(rows)) => {
                        crate::computed::DerivedOutput::IntervalTable(rows.clone())
                    }
                    _ => anyhow::bail!("Wait for the live preview to finish updating."),
                };
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
                    self.derived_editor = None;
                    return Ok(());
                }
                if let Operation::Formula { settings, .. } = &mut definition.operation {
                    match &output {
                        crate::computed::DerivedOutput::IntervalTable(rows) => {
                            settings.scope = crate::computed::MeasurementScope::IntervalSnapshot {
                                intervals: rows
                                    .iter()
                                    .filter(|row| row.bounds.0 < row.bounds.1)
                                    .map(|row| row.bounds)
                                    .collect(),
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
                self.derived_editor = None;
                return Ok(());
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
            Command::Open(_) => unreachable!(),
        }
        if inputs_changed {
            editor.calculation_revision += 1;
            editor.preview = None;
            editor.stopped = false;
        }
        let inputs = editor.inputs(&self.doc)?;
        editor.domain = input_domain(&inputs, &self.doc)?;
        editor.error = editor
            .definition(self.doc.limits(), self.doc.time_base(), &inputs)
            .and_then(|d| Key::new(&self.doc, &inputs, &d))
            .err()
            .map(|e| format!("{e:#}"));
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
                    .and_then(|e| e.preview.clone()),
            )
            .collect()
    }
    pub(super) fn derived_sources(&self) -> HashSet<Traced<volna_trace::data::SignalRef>> {
        self.derived_demand()
            .into_iter()
            .flat_map(|key| key.signals().collect::<Vec<_>>())
            .collect()
    }

    pub(super) fn sync_derived(&mut self) -> Vec<crate::LoadRequest> {
        if self
            .derived_editor
            .as_ref()
            .is_some_and(|e| self.panels.waves(e.panel).is_none())
        {
            self.derived_editor = None;
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
        let sources = self.derived_sources();
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
            editor.preview = None;
            editor.error = Some(
                "The source row changed or was removed. Reopen the derived-signal editor.".into(),
            );
            return;
        }
        let Some(editor) = self
            .derived_editor
            .as_mut()
            .filter(|editor| !editor.stopped && editor.preview.is_some())
        else {
            return;
        };
        let current = editor.inputs(&self.doc).and_then(|inputs| {
            editor.domain = input_domain(&inputs, &self.doc)?;
            let definition = editor.definition(self.doc.limits(), self.doc.time_base(), &inputs)?;
            Key::new(&self.doc, &inputs, &definition)
        });
        match current {
            Ok(key) => {
                editor.preview = Some(key);
                editor.error = None;
            }
            // Retain the active key so a later marker edit can repair the scope.
            Err(error) => editor.error = Some(format!("{error:#}")),
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
        let Some(waves) = self.panels.waves_mut(editor.panel) else {
            return;
        };
        if waves.revision() == editor.revision {
            return;
        }
        if let Some(preview) = &waves.preview
            && preview.revision == waves.revision()
        {
            editor.row = preview.source_row;
            editor.revision = preview.revision;
            self.sync_derived_row();
        } else {
            waves.preview = None;
        }
    }

    pub(super) fn sync_derived_row(&mut self) {
        for waves in self.panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
            waves.preview = None;
        }
        let Some(editor) = &mut self.derived_editor else {
            return;
        };
        let Some(waves) = self
            .panels
            .waves_mut(editor.panel)
            .filter(|w| w.revision() == editor.revision)
        else {
            return;
        };
        if let Ok((definition, inputs)) = editor.inputs(&self.doc).and_then(|inputs| {
            editor
                .definition(self.doc.limits(), self.doc.time_base(), &inputs)
                .map(|definition| (definition, inputs))
        }) {
            editor.display_definition = definition;
            editor.display_inputs = inputs;
        }
        let computation = editor
            .preview
            .as_ref()
            .and_then(|key| self.computations.status(key));
        let output = match computation {
            Some(ComputationStatus::Ready(history)) => {
                Some(crate::computed::DerivedOutput::Waveform(history.clone()))
            }
            Some(ComputationStatus::ReadyMeasurement(measurement)) => {
                Some(crate::computed::DerivedOutput::Measurement(*measurement))
            }
            Some(ComputationStatus::ReadyTable(rows)) => {
                Some(crate::computed::DerivedOutput::IntervalTable(rows.clone()))
            }
            _ => None,
        };
        let ready = editor.error.is_none() && output.is_some();
        if let Some(output) = output {
            editor.last_result = Some(output);
        }
        let status = if editor.error.is_some() {
            "Invalid settings · outdated"
        } else if editor.stopped {
            "Stopped · outdated"
        } else {
            match computation {
                Some(
                    ComputationStatus::Ready(_)
                    | ComputationStatus::ReadyMeasurement(_)
                    | ComputationStatus::ReadyTable(_),
                ) => "Live preview",
                Some(ComputationStatus::Waiting) => "Loading input…",
                Some(ComputationStatus::Failed(_)) => "Failed · outdated",
                _ => "Updating…",
            }
        };
        let Ok((at, entry, editing)) = waves.derived_row(
            crate::wave::model::DerivedPlacement {
                row: editor.row,
                revision: editor.revision,
                editing: editor.editing,
            },
            editor.display_definition.clone(),
            editor.display_inputs.clone(),
            editor.last_result.clone(),
            &self.doc.translators,
        ) else {
            return;
        };
        waves.preview = Some(crate::wave::model::PreviewRow {
            source_row: editor.row,
            revision: editor.revision,
            index: if editing { at } else { waves.items().len() },
            before: at,
            entry,
            status: status.into(),
            stale: !ready,
        });
    }
}

fn duration_text(ticks: u64, base: crate::wave::timeline::TimeBase<'_>) -> String {
    let text = crate::wave::timeline::format_time(ticks as f64, base);
    if crate::moving_average::duration(&text, base).ok() == Some(ticks) {
        text
    } else {
        format!("{ticks} ticks")
    }
}

fn unique_alias<'a>(name: &str, inputs: impl Iterator<Item = &'a str>) -> String {
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
    let base = if crate::computed::formula::valid_alias(&base) {
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
