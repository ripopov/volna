//! Reusable, trace-local sampling rules; the first choice for each trace is its default.

use crate::{
    Document,
    trace::{TraceId, Traced},
    wave::model::{MenuAction, MenuItem},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use volna_trace::data::{SignalShape, VarId, source::Lookup};

const MAX_CHOICES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingMode {
    RisingEdge,
    ValueChanges,
}

impl SamplingMode {
    pub const ALL: [Self; 2] = [Self::RisingEdge, Self::ValueChanges];
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::RisingEdge => "Rising edge",
            Self::ValueChanges => "Value changes",
        }
    }
    #[must_use]
    pub fn accepts(self, shape: SignalShape) -> bool {
        match self {
            Self::RisingEdge => shape == SignalShape::Bit,
            Self::ValueChanges => {
                matches!(shape, SignalShape::Bit | SignalShape::Vector { width: 2.. })
            }
        }
    }
    #[must_use]
    pub fn trigger(self, alias: &str) -> String {
        match self {
            Self::RisingEdge => format!("@posedge {alias}"),
            Self::ValueChanges => format!("@{alias}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamplingChoice {
    pub trace: TraceId,
    pub path: Vec<String>,
    pub nth: Option<usize>,
    pub mode: SamplingMode,
}
impl SamplingChoice {
    pub fn new(doc: &Document, var: Traced<VarId>, mode: SamplingMode) -> Result<Self> {
        let h = doc
            .hierarchy(var.trace)
            .context("Sampling trace is unavailable.")?;
        let v = h
            .get_var(var.item)
            .context("Sampling signal is unavailable.")?;
        ensure!(
            mode.accepts(v.shape),
            "Choose a bit for rising edges, or a bit/bus for value changes."
        );
        let (path, nth) = h.var_path(var.item);
        Ok(Self {
            trace: var.trace,
            path,
            nth,
            mode,
        })
    }
    #[must_use]
    pub fn resolve(&self, doc: &Document) -> Option<Traced<VarId>> {
        let h = doc.hierarchy(self.trace)?;
        let Lookup::Found(var) = h.find_var(&self.path, self.nth) else {
            return None;
        };
        self.mode
            .accepts(h.var(var).shape)
            .then_some(Traced::new(self.trace, var))
    }
    #[must_use]
    pub fn label(&self) -> String {
        let occurrence = self
            .nth
            .map_or_else(String::new, |n| format!(" (occurrence {})", n + 1));
        format!(
            "{}: {}{occurrence} · {}",
            self.trace,
            self.path.join("."),
            self.mode.label()
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SamplingCommand {
    Register(Traced<VarId>, SamplingMode),
    MakeDefault(SamplingChoice),
    Remove(SamplingChoice),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamplingChoices {
    choices: Vec<SamplingChoice>,
}
impl SamplingChoices {
    pub fn iter(&self) -> impl Iterator<Item = &SamplingChoice> {
        self.choices.iter()
    }
    #[must_use]
    pub fn default_for(&self, trace: TraceId) -> Option<&SamplingChoice> {
        self.choices.iter().find(|c| c.trace == trace)
    }
    pub fn register(&mut self, choice: SamplingChoice) -> Result<()> {
        if !self.choices.contains(&choice) {
            ensure!(
                self.choices.len() < MAX_CHOICES,
                "Remove an unused sampling choice before adding another."
            );
            self.choices.push(choice);
        }
        Ok(())
    }
    pub fn make_default(&mut self, choice: SamplingChoice) -> Result<()> {
        self.register(choice.clone())?;
        self.remove(&choice);
        self.choices.insert(0, choice);
        Ok(())
    }
    pub fn remove(&mut self, choice: &SamplingChoice) {
        self.choices.retain(|c| c != choice);
    }
    pub(crate) fn remove_trace(&mut self, trace: TraceId) {
        self.choices.retain(|choice| choice.trace != trace);
    }
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.choices.len() <= MAX_CHOICES,
            "Too many sampling choices."
        );
        for (i, choice) in self.choices.iter().enumerate() {
            ensure!(
                !choice.path.is_empty() && !self.choices[..i].contains(choice),
                "Invalid or duplicate sampling choice."
            );
        }
        Ok(())
    }
}

impl Document {
    #[must_use]
    pub fn sampling_menu(&self, var: Traced<VarId>) -> Vec<MenuItem> {
        let choices = SamplingMode::ALL.into_iter().filter_map(|mode| {
            let choice = SamplingChoice::new(self, var, mode).ok()?;
            let registered = self.sampling.iter().any(|c| c == &choice);
            Some(MenuItem::choice(
                MenuAction::Sampling(SamplingCommand::Register(var, mode)),
                mode.label(),
                registered,
            ))
        });
        let mut items: Vec<_> = choices.collect();
        for choice in self
            .sampling
            .iter()
            .filter(|c| c.resolve(self) == Some(var))
        {
            let is_default = self.sampling.default_for(var.trace) == Some(choice);
            items.push(MenuItem {
                disabled: is_default,
                ..MenuItem::plain(
                    MenuAction::Sampling(SamplingCommand::MakeDefault(choice.clone())),
                    &format!(
                        "{} · {}",
                        if is_default {
                            "Default"
                        } else {
                            "Make default"
                        },
                        choice.mode.label()
                    ),
                )
            });
            items.push(MenuItem::plain(
                MenuAction::Sampling(SamplingCommand::Remove(choice.clone())),
                &format!("Remove · {}", choice.mode.label()),
            ));
        }
        items
    }
}
