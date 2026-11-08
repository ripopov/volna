//! Content lifecycle and journal dispatch.

use anyhow::{Context, Result};

use super::{Panel, PanelKind};
use crate::document::Document;
use crate::wave::model::Resident;

pub(crate) enum Prop {
    Title(Option<String>),
    Clocks {
        rulers: Option<Vec<crate::clock::ClockKey>>,
        origin: Option<u64>,
    },
    Table(crate::table::model::Prop),
    Transaction(crate::transaction::model::Prop),
}

impl Prop {
    pub(crate) fn kind(&self) -> (u8, u8) {
        match self {
            Self::Title(_) => (0, 0),
            Self::Clocks { .. } => (1, 0),
            Self::Table(prop) => (2, prop.kind()),
            Self::Transaction(prop) => (3, prop.kind()),
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        match self {
            Self::Title(t) => t.as_ref().map_or(0, String::len),
            Self::Clocks { rulers, .. } => rulers.iter().flatten().map(|k| k.item.len() + 24).sum(),
            Self::Table(prop) => prop.bytes(),
            Self::Transaction(prop) => prop.bytes(),
        }
    }
}

impl crate::trace::Retime for Prop {
    fn retime(&mut self, by: crate::trace::Rescale) {
        if let Self::Clocks { origin, .. } = self {
            origin.retime(by);
        }
        if let Self::Transaction(prop) = self {
            prop.retime(by);
        }
    }
}

impl PanelKind {
    pub(crate) fn adopt(&mut self, doc: &mut Document, resident: &Resident) -> Result<()> {
        match self {
            Self::Waves(w) => {
                if w.needs_attach() {
                    w.attach_rows(doc, resident);
                }
                Ok(())
            }
            Self::Pipeline(p) => p.attach(doc),
            Self::Table(t) => {
                let result = t.attach(doc, resident);
                if let Err(error) = &result {
                    t.state = crate::table::TableState::Failed(error.to_string());
                }
                result
            }
            Self::Transaction(t) => t.attach(doc),
            Self::Start | Self::Settings | Self::Unsupported(_) => Ok(()),
        }
    }

    pub(crate) fn retire(&mut self, doc: &mut Document) {
        match self {
            Self::Waves(w) => w.detach_rows(),
            Self::Pipeline(p) => p.detach(doc),
            Self::Table(t) => t.park(doc),
            Self::Transaction(t) => t.park(doc),
            Self::Start | Self::Settings | Self::Unsupported(_) => {}
        }
    }

    pub(crate) fn forget_trace(
        &mut self,
        doc: &mut Document,
        trace: crate::trace::TraceId,
        label: Option<&str>,
    ) {
        if label.is_some()
            && let Some(nav) = self.nav_mut()
        {
            nav.forget_trace(trace);
        }
        match self {
            Self::Waves(w) => {
                if let Some(label) = label {
                    w.remove_trace_rows(trace, label.to_owned());
                }
            }
            Self::Transaction(t) if label.is_none() || !t.pinned() => {
                t.forget_trace(doc, trace);
            }
            _ => {}
        }
    }
}

impl Panel {
    pub(crate) fn matches_prop(&self, prop: &Prop) -> bool {
        match prop {
            Prop::Title(title) => self.title.get() == title,
            Prop::Clocks { rulers, origin } => self.kind.nav().is_some_and(|nav| {
                nav.clocks().rulers == *rulers && nav.clocks().origin == *origin
            }),
            Prop::Table(prop) => self.kind.table().is_some_and(|t| t.matches_prop(prop)),
            Prop::Transaction(prop) => self
                .kind
                .transaction()
                .is_some_and(|t| t.matches_prop(prop)),
        }
    }

    pub(crate) fn swap_prop(&mut self, doc: &mut Document, prop: Prop) -> Result<Prop> {
        Ok(match prop {
            Prop::Title(title) => Prop::Title(self.title.swap(title)),
            Prop::Clocks { rulers, origin } => {
                let nav = self.kind.nav_mut().context("not a timed panel")?;
                let (rulers, origin) = nav.swap_clocks((rulers, origin));
                Prop::Clocks { rulers, origin }
            }
            Prop::Table(prop) => Prop::Table(
                self.kind
                    .table_mut()
                    .context("not a table")?
                    .swap_prop(prop),
            ),
            Prop::Transaction(prop) => Prop::Transaction(
                self.kind
                    .transaction_mut()
                    .context("not a transaction panel")?
                    .swap_prop(doc, prop),
            ),
        })
    }

    pub(crate) fn selection_reveal_ready(&self, doc: &Document) -> bool {
        let Some(selected) = self.kind.nav().and_then(|nav| nav.selection_to_reveal(doc)) else {
            return false;
        };
        match &self.kind {
            PanelKind::Waves(waves) => {
                waves.last_layout().waves.height() > 0.0
                    && doc
                        .resident_generator(selected.track)
                        .is_some_and(|generator| generator.transaction(selected.id).is_some())
                    && waves.items().iter().any(|entry| {
                        entry
                            .lane()
                            .is_some_and(|lane| lane.track() == Some(selected.track))
                    })
            }
            PanelKind::Pipeline(pipeline) => {
                pipeline.last_layout().cells.height() > 0.0 && pipeline.selected_row(doc).is_some()
            }
            PanelKind::Table(table) => {
                table.layout.body.height() > 0.0
                    && table.source.generator_track() == Some(selected.track)
                    && table.active_ordinal().is_some()
            }
            _ => false,
        }
    }

    pub(crate) fn reveal_selection(
        &mut self,
        doc: &mut Document,
        now: crate::Instant,
        force: bool,
    ) -> bool {
        let Some(nav) = self.kind.nav_mut() else {
            return false;
        };
        if nav.selection(doc).is_none() {
            nav.did_reveal_selection(None);
            return false;
        }
        if force {
            nav.did_reveal_selection(None);
        }
        let Some(selected) = nav.selection_to_reveal(doc) else {
            return false;
        };
        if selected.origin == self.id && !force {
            nav.did_reveal_selection(Some(selected));
            return false;
        }
        let revealed = match &mut self.kind {
            PanelKind::Pipeline(pipeline) => pipeline.reveal_selection(doc, now),
            PanelKind::Table(table) => {
                table.follow_selection(doc);
                table.reveal_selection(doc)
            }
            PanelKind::Waves(waves) => {
                if waves.last_layout().waves.height() <= 0.0 {
                    return false;
                }
                if !waves.reveal_transaction(doc, selected.track, selected.id) {
                    return false;
                }
                if selected.origin != self.id
                    && let Some((begin, end)) =
                        doc.resident_generator(selected.track)
                            .and_then(|generator| {
                                generator
                                    .transaction(selected.id)
                                    .map(|tx| (tx.begin, tx.end))
                            })
                {
                    waves.nav.reveal_record(doc, begin, end, now);
                }
                true
            }
            _ => false,
        };
        if revealed {
            self.kind
                .nav_mut()
                .unwrap()
                .did_reveal_selection(Some(selected));
        }
        revealed
    }
}
