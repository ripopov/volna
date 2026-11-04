//! The moving-average editor commits only successfully previewed definitions.

use super::App;
use crate::moving_average::{Command, ComputationStatus, Draft, Editor, Key};
use crate::panels::PanelId;
use crate::wave::model::RowSource;
use anyhow::{Context, Result, ensure};
use std::collections::HashSet;

impl App {
    #[must_use]
    pub fn moving_average_preview(&self) -> Option<&ComputationStatus> {
        self.averages
            .status(&self.moving_average_editor.as_ref()?.preview?)
    }

    #[must_use]
    pub fn can_derive_moving_average(&self, panel: PanelId) -> bool {
        self.panels.waves(panel).is_some_and(|w| {
            w.selected.len() == 1
                && w.selected
                    .first()
                    .and_then(|&r| w.signal(r))
                    .is_some_and(|s| s.numeric_kind().is_some())
        })
    }

    pub(super) fn moving_average_command(&mut self, panel: PanelId, command: Command) {
        if let Err(error) = self.edit_moving_average(panel, command) {
            if let Some(editor) = &mut self.moving_average_editor {
                editor.error = Some(format!("{error:#}"));
            } else {
                self.events.push(crate::Event::Notice(format!("{error:#}")));
            }
        }
        self.sync_moving_average_row();
        self.changed();
    }

    fn edit_moving_average(&mut self, panel: PanelId, command: Command) -> Result<()> {
        if let Command::Open(row) = command {
            let waves = self
                .panels
                .waves(panel)
                .context("Waveform panel was closed.")?;
            let row = row
                .or_else(|| {
                    (waves.selected.len() == 1)
                        .then(|| waves.selected.first().copied())
                        .flatten()
                })
                .context("Select one numeric signal, then choose Derive.")?;
            let signal = waves
                .signal(row)
                .context("Select a numeric recorded signal.")?;
            let interpretation = signal.source.moving_average().map_or_else(
                || {
                    Ok(crate::moving_average::Interpretation {
                        kind: signal.numeric_kind().context(
                            "Choose a numeric value format before deriving a moving average.",
                        )?,
                        projection: signal.projection,
                    })
                },
                |definition| Ok::<_, anyhow::Error>(definition.interpretation),
            )?;
            interpretation.validate(signal.source_shape)?;
            let (input, definition) = match &signal.source {
                RowSource::MovingAverage { input, definition } => {
                    (input.as_ref().clone(), Some(definition))
                }
                source => (source.clone(), None),
            };
            ensure!(
                input.signal().is_some(),
                "The input is missing or ambiguous in this trace."
            );
            let domain = definition.map_or_else(
                || {
                    self.doc
                        .traces()
                        .get(input.trace())
                        .and_then(crate::trace::TraceSlot::range)
                        .unwrap_or(self.doc.limits())
                },
                |d| (d.settings.origin, d.settings.end),
            );
            let interval = (domain.1 - domain.0).div_ceil(2000).max(1);
            let (name, window, interval, partial, skip_unknown) = definition.map_or_else(
                || {
                    (
                        format!("{}_mean", signal.name),
                        interval.saturating_mul(16),
                        interval,
                        false,
                        false,
                    )
                },
                |d| {
                    (
                        d.name.clone(),
                        d.settings.window,
                        d.settings.interval,
                        d.settings.partial,
                        d.settings.skip_unknown,
                    )
                },
            );
            let draft = Draft {
                name,
                window: duration_text(window, self.doc.time_base()),
                interval: duration_text(interval, self.doc.time_base()),
                partial,
                skip_unknown,
            };
            let display_definition =
                draft.definition(domain, self.doc.time_base(), interpretation)?;
            let last_result = definition.and_then(|_| signal.history.clone());
            self.next_average_editor += 1;
            self.moving_average_editor = Some(Editor {
                id: self.next_average_editor,
                panel,
                row,
                revision: waves.revision(),
                input,
                interpretation,
                editing: definition.is_some(),
                draft,
                domain,
                preview: None,
                calculation_revision: 0,
                error: None,
                stopped: false,
                display_definition,
                last_result,
                reveal_preview: true,
            });
            if let Some(waves) = self.panels.waves_mut(panel) {
                waves.menu_dismiss();
            }
            return self.edit_moving_average(panel, Command::Preview);
        }
        let editor = self
            .moving_average_editor
            .as_mut()
            .filter(|e| e.panel == panel)
            .context("Open the moving-average editor first.")?;
        match command {
            Command::Update(draft) => {
                if editor.draft != draft {
                    let calculation_changed = editor.draft.window != draft.window
                        || editor.draft.interval != draft.interval
                        || editor.draft.partial != draft.partial
                        || editor.draft.skip_unknown != draft.skip_unknown;
                    editor.draft = draft;
                    if calculation_changed {
                        editor.calculation_revision += 1;
                        editor.preview = None;
                        editor.stopped = false;
                    }
                    editor.error = editor
                        .draft
                        .definition(editor.domain, self.doc.time_base(), editor.interpretation)
                        .err()
                        .map(|e| format!("{e:#}"));
                }
            }
            Command::Preview => {
                editor.preview = None;
                editor.stopped = false;
                let mut calculation = editor.draft.clone();
                calculation.name.clone_from(&editor.display_definition.name);
                let definition = calculation.definition(
                    editor.domain,
                    self.doc.time_base(),
                    editor.interpretation,
                )?;
                editor.preview = Some(Key {
                    generation: self.doc.generation(),
                    signal: editor.input.signal().context("Input is unavailable.")?,
                    settings: definition.settings,
                    interpretation: definition.interpretation,
                });
                editor.display_definition = definition;
                editor.error = editor
                    .draft
                    .definition(editor.domain, self.doc.time_base(), editor.interpretation)
                    .err()
                    .map(|e| format!("{e:#}"));
                self.averages.retry();
            }
            Command::Stop => {
                editor.preview = None;
                editor.stopped = true;
                editor.error = None;
            }
            Command::Close => self.moving_average_editor = None,
            Command::Apply => {
                let definition = editor.draft.definition(
                    editor.domain,
                    self.doc.time_base(),
                    editor.interpretation,
                )?;
                let key = editor
                    .preview
                    .context("Wait for the live preview before applying.")?;
                ensure!(
                    key.settings == definition.settings
                        && key.interpretation == definition.interpretation
                        && key.generation == self.doc.generation(),
                    "Wait for the current draft to finish updating."
                );
                let Some(ComputationStatus::Ready(history)) = self.averages.status(&key) else {
                    anyhow::bail!("Wait for the live preview to finish updating.");
                };
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
                self.panels
                    .waves_mut(panel)
                    .context("Waveform panel was closed.")?
                    .apply_moving_average(
                        editor.row,
                        editor.revision,
                        definition,
                        history.clone(),
                        &self.doc.translators,
                    )?;
                self.moving_average_editor = None;
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
                self.moving_average_editor = None;
            }
            Command::Open(_) => unreachable!(),
        }
        Ok(())
    }

    pub(super) fn moving_average_demand(&self) -> HashSet<Key> {
        self.panels
            .iter()
            .filter_map(|p| p.kind.waves())
            .flat_map(crate::wave::WaveModel::items)
            .filter_map(|e| {
                let RowSource::MovingAverage { input, definition } = &e.signal()?.source else {
                    return None;
                };
                Some(Key {
                    generation: self.doc.generation(),
                    signal: input.signal()?,
                    settings: definition.settings,
                    interpretation: definition.interpretation,
                })
            })
            .chain(self.moving_average_editor.as_ref().and_then(|e| e.preview))
            .collect()
    }

    pub(super) fn sync_moving_averages(&mut self) -> Vec<crate::LoadRequest> {
        if self
            .moving_average_editor
            .as_ref()
            .is_some_and(|e| self.panels.waves(e.panel).is_none())
        {
            self.moving_average_editor = None;
        }
        let wanted = self.moving_average_demand();
        let resident = self.resident_histories();
        let requests = self.averages.sync(
            wanted.clone(),
            &resident,
            self.doc.generation(),
            &self.viewer_memory_budget(),
        );
        for key in wanted {
            if self.averages.needs_source(key.signal) {
                self.doc.request_signal(key.signal);
            }
        }
        self.attach_moving_averages();
        requests
    }

    pub(super) fn attach_moving_averages(&mut self) {
        for waves in self.panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
            waves.sync_moving_averages(self.doc.generation(), &self.averages);
        }
        self.sync_moving_average_row();
    }

    pub(super) fn sync_moving_average_source(&mut self) {
        let Some(editor) = &mut self.moving_average_editor else {
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
            self.sync_moving_average_row();
        } else {
            waves.preview = None;
        }
    }

    pub(super) fn sync_moving_average_row(&mut self) {
        for waves in self.panels.iter_mut().filter_map(|p| p.kind.waves_mut()) {
            waves.preview = None;
        }
        let Some(editor) = &mut self.moving_average_editor else {
            return;
        };
        let Some(waves) = self
            .panels
            .waves_mut(editor.panel)
            .filter(|w| w.revision() == editor.revision)
        else {
            return;
        };
        if let Ok(definition) =
            editor
                .draft
                .definition(editor.domain, self.doc.time_base(), editor.interpretation)
        {
            editor.display_definition = definition;
        }
        let computation = editor.preview.and_then(|key| self.averages.status(&key));
        if let Some(ComputationStatus::Ready(history)) = computation {
            editor.last_result = Some(history.clone());
        }
        let ready =
            editor.error.is_none() && matches!(computation, Some(ComputationStatus::Ready(_)));
        let status = if editor.error.is_some() {
            "Invalid settings · outdated"
        } else if editor.stopped {
            "Stopped · outdated"
        } else {
            match computation {
                Some(ComputationStatus::Ready(_)) => "Live preview",
                Some(ComputationStatus::Waiting) => "Loading input…",
                Some(ComputationStatus::Failed(_)) => "Failed · outdated",
                _ => "Updating…",
            }
        };
        let Ok((at, entry, editing)) = waves.moving_average_row(
            editor.row,
            editor.display_definition.clone(),
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
