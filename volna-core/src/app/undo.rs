//! The app's side of undo and redo (`volna/README.md`, "Undo and redo"): step boundaries,
//! collecting the journaled fields' inverses, applying edits, and showing what a flip
//! changed.

use anyhow::{Context as _, Result};
use web_time::Instant;

use super::{App, Event};
use crate::history::{Context, Edit, Step, Structure};
use crate::panels::content::Prop;
use crate::panels::{Panel, PanelId, Panels};

impl App {
    #[must_use]
    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    #[must_use]
    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.history.can_undo() || self.gesture_open()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub(super) fn gesture_open(&self) -> bool {
        self.panels.iter().any(Panel::dragging)
    }

    pub(super) fn begin_step(&mut self, now: Instant) {
        self.history.begin_command(self.panels.focused_id(), now);
    }

    pub(super) fn end_step(&mut self, now: Instant) {
        self.collect_edits();
        if self.history.is_recording() && !self.gesture_open() {
            self.commit_step(now);
        }
    }

    pub(super) fn collect_edits(&mut self) {
        self.doc.take_edits(&mut self.history);
        for panel in self.panels.iter_mut() {
            panel.take_edits(&self.doc.clocks, &mut self.history);
        }
        debug_assert_eq!(
            crate::history::pending(),
            0,
            "a journaled field changed but its owner did not hand the edit over"
        );
    }

    pub(super) fn restructure<T>(
        &mut self,
        label: Option<String>,
        op: impl FnOnce(&mut Panels) -> Result<(T, Vec<Panel>)>,
    ) -> Result<T> {
        self.collect_edits();
        let revision = self.panels.revision();
        let (layout, focused, before) = self.panels.structure();
        let (value, removed) = op(&mut self.panels)?;
        let (after_layout, _, after) = self.panels.structure();
        let close: Vec<PanelId> = after.difference(&before).copied().collect();
        let mut reopen: Vec<Panel> = removed
            .into_iter()
            .filter(|p| !p.kind.is_settings())
            .collect();
        for panel in &mut reopen {
            panel.kind.retire(&mut self.doc);
        }
        let notices = self.adopt_panels(&close);
        self.events.extend(notices.into_iter().map(Event::Notice));
        if revision != self.panels.revision() {
            self.layout_changed();
        }
        if close.is_empty() && reopen.is_empty() && layout.same_shape(&after_layout) {
            return Ok(value);
        }
        self.history.record(
            Edit::Layout(Box::new(Structure {
                layout,
                focused,
                close,
                reopen,
            })),
            label,
        );
        Ok(value)
    }

    pub(super) fn commit_step(&mut self, now: Instant) {
        let Some(mut step) = self.history.take_open() else {
            return;
        };
        step.after = self.context_after(&step);
        if let Some(mut earlier) = self.history.take_merge_target(&step) {
            earlier.absorb(step);
            step = earlier;
        }
        self.prune(&mut step);
        if step.label.is_empty() {
            step.label = "Edit".into();
        }
        let changed = !step.is_empty();
        self.history.push(step);
        if changed {
            self.announcement = None;
            self.workspace.scheduler.changed(now);
            self.changed();
        }
    }

    fn prune(&self, step: &mut Step) {
        let rows_edits = |panel: PanelId| {
            step.edits
                .iter()
                .filter(|e| matches!(e, Edit::Rows { panel: p, .. } if *p == panel))
                .count()
        };
        let noop: Vec<bool> = step
            .edits
            .iter()
            .map(|edit| match edit {
                Edit::Rows { panel, splices } => {
                    rows_edits(*panel) == 1
                        && self.panels.waves(*panel).is_some_and(|w| {
                            splices.iter().all(|s| {
                                s.in_place()
                                    && w.items().get(s.at..s.at + s.remove).is_some_and(|now| {
                                        now.iter().zip(&s.insert).all(|(a, b)| a.same(b))
                                    })
                            })
                        })
                }
                Edit::Markers { markers, .. } => markers.as_slice() == self.doc.markers(),
                Edit::Prop { panel, prop } => {
                    let Some(p) = self.panels.get(*panel) else {
                        return false;
                    };
                    p.matches_prop(prop)
                }
                Edit::Layout(s) => {
                    s.close.is_empty()
                        && s.reopen.is_empty()
                        && s.layout.same_shape(&self.panels.structure().0)
                }
                Edit::Traces(crate::document::TraceEdit::Rename(trace, name)) => self
                    .doc
                    .traces()
                    .get(*trace)
                    .is_some_and(|slot| slot.rename == *name),
                Edit::Traces(_) => false,
            })
            .collect();
        let mut k = 0;
        step.retain_edits(|_| {
            k += 1;
            !noop[k - 1]
        });
    }

    fn context_after(&self, step: &Step) -> Context {
        Context {
            focus: Some(self.panels.focused_id()),
            rows: step
                .before
                .rows
                .iter()
                .filter_map(|(id, _)| Some((*id, self.panels.waves(*id)?.row_selection())))
                .collect(),
        }
    }

    pub(super) fn cancel_gesture(&mut self) -> bool {
        if !self.gesture_open() {
            return false;
        }
        for panel in self.panels.iter_mut() {
            panel.cancel_drag();
        }
        self.collect_edits();
        if let Some(mut step) = self.history.take_open() {
            match self.flip(&mut step) {
                Ok(()) => self.restore_context(&step, &step.before),
                Err(error) => self.history_broken(error),
            }
        }
        self.changed();
        true
    }

    pub(super) fn undo(&mut self, now: Instant) {
        if self.cancel_gesture() {
            return;
        }
        let Some(mut step) = self.history.pop_undo() else {
            self.announce("Nothing to undo".into());
            return;
        };
        if let Err(error) = self.flip(&mut step) {
            self.history_broken(error);
            return;
        }
        self.restore_context(&step, &step.before);
        self.announce(format!("Undid {}", step.label));
        self.history.push_undone(step);
        self.workspace.scheduler.changed(now);
    }

    pub(super) fn redo(&mut self, now: Instant) {
        if self.gesture_open() {
            return;
        }
        let Some(mut step) = self.history.pop_redo() else {
            self.announce("Nothing to redo".into());
            return;
        };
        if let Err(error) = self.flip(&mut step) {
            self.history_broken(error);
            return;
        }
        self.restore_context(&step, &step.after);
        self.announce(format!("Redid {}", step.label));
        self.history.push_redone(step);
        self.workspace.scheduler.changed(now);
    }

    pub(super) fn announce(&mut self, text: String) {
        self.events.push(Event::Announce(text.clone()));
        self.announcement = Some(text);
        self.changed();
    }

    fn flip(&mut self, step: &mut Step) -> Result<()> {
        let mut flipped = Vec::with_capacity(step.edits.len());
        for edit in step.edits.drain(..).rev() {
            flipped.push(self.apply(edit)?);
        }
        step.edits = flipped;
        Ok(())
    }

    pub(crate) fn apply(&mut self, edit: Edit) -> Result<Edit> {
        Ok(match edit {
            Edit::Rows { panel, mut splices } => {
                let detached = splices
                    .iter()
                    .flat_map(|s| &s.insert)
                    .filter_map(|e| e.signal())
                    .any(|s| {
                        s.source.signal().is_some() && s.history.is_none() && s.error.is_none()
                    });
                if detached {
                    let resident = self.resident_histories();
                    crate::wave::model::attach_rows(
                        splices.iter_mut().flat_map(|s| &mut s.insert),
                        &mut self.doc,
                        &resident,
                    );
                }
                let waves = self
                    .panels
                    .waves_mut(panel)
                    .with_context(|| format!("no wave panel {}", panel.0))?;
                let splices = waves.splice(splices).map_err(anyhow::Error::msg)?;
                Edit::Rows { panel, splices }
            }
            Edit::Markers { markers, attached } => {
                let (markers, attached) = self.doc.swap_markers(markers, attached);
                Edit::Markers { markers, attached }
            }
            Edit::Prop { panel, prop } => Edit::Prop {
                panel,
                prop: self.swap_prop(panel, prop)?,
            },
            Edit::Traces(edit) => Edit::Traces(self.apply_trace_edit(edit)?),
            Edit::Layout(structure) => {
                let mut inverse = self.panels.install(*structure)?;
                for panel in &mut inverse.reopen {
                    panel.kind.retire(&mut self.doc);
                }
                let notices = self.adopt_panels(&inverse.close);
                self.events.extend(notices.into_iter().map(Event::Notice));
                self.layout_changed();
                Edit::Layout(Box::new(inverse))
            }
        })
    }

    fn swap_prop(&mut self, id: PanelId, prop: Prop) -> Result<Prop> {
        if let Prop::Title(title) = prop {
            let old = self.panels.swap_title(id, title)?;
            self.layout_changed();
            return Ok(Prop::Title(old));
        }
        self.panels
            .get_mut(id)
            .with_context(|| format!("no panel {}", id.0))?
            .swap_prop(&mut self.doc, prop)
    }

    fn restore_context(&mut self, step: &Step, context: &Context) {
        let touched = step.touched();
        let layout = step.has_layout();
        let focus = context
            .focus
            .filter(|f| layout || touched.contains(f))
            .or_else(|| touched.last().copied())
            .filter(|&id| self.panels.get(id).is_some());
        if let Some(id) = focus
            && self.panels.focus(id).unwrap_or(false)
        {
            self.layout_changed();
        }
        for (id, selection) in &context.rows {
            if let Some(waves) = self.panels.waves_mut(*id) {
                waves.set_row_selection(selection.clone());
                if let Some(&first) = waves.selected.first() {
                    waves.reveal_entry(first);
                }
            }
        }
        self.changed();
    }

    fn history_broken(&mut self, error: anyhow::Error) {
        debug_assert!(false, "undo journal out of step: {error:#}");
        self.history.clear();
        self.announcement = None;
        self.events
            .push(Event::Notice(format!("Undo history cleared: {error:#}")));
        self.changed();
    }
}
