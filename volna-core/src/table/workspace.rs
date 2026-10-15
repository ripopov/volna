//! The table panel's workspace schema, capture, validation and resolution.

use super::{SignalSource, TableModel, TableSource, columns::ColumnSet};
use crate::panels::{Panel, PanelId, PanelKind, workspace::RestoreContext};
use crate::pipeline::TrackSource;
use crate::trace::{TraceId, Traced};
use crate::wave::model::Link;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use volna_trace::data::source::Lookup;

pub(crate) const VERSION: u32 = 4;

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SavedTableSource {
    Generator {
        #[serde(default, skip_serializing_if = "TraceId::is_a")]
        trace: TraceId,
        path: Vec<String>,
    },
    Signals {
        signals: Vec<SavedTableSignal>,
    },
}

#[derive(Serialize, Deserialize)]
struct SavedTableSignal {
    #[serde(default, skip_serializing_if = "TraceId::is_a")]
    trace: TraceId,
    path: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nth: Option<usize>,
}

#[derive(Serialize, Deserialize)]
struct TablePanel {
    id: PanelId,
    kind: String,
    version: u32,
    title: Option<String>,
    source: SavedTableSource,
    columns: ColumnSet,
    widths: Vec<Option<f32>>,
    link: Link,
}

pub(crate) fn save(
    table: &TableModel,
    id: PanelId,
    title: Option<String>,
) -> Result<Box<RawValue>> {
    let source = match &table.source {
        TableSource::Generator(track) => SavedTableSource::Generator {
            trace: track.trace(),
            path: track.path().to_vec(),
        },
        TableSource::Signals(signals) => SavedTableSource::Signals {
            signals: signals
                .iter()
                .map(|signal| SavedTableSignal {
                    trace: signal.trace,
                    path: signal.path.clone(),
                    nth: signal.nth,
                })
                .collect(),
        },
    };
    Ok(serde_json::value::to_raw_value(&TablePanel {
        id,
        kind: "table".into(),
        version: VERSION,
        title,
        source,
        columns: table.columns.get().clone(),
        widths: table.widths.get().clone(),
        link: table.nav.link,
    })?)
}

pub(crate) fn restore(raw: &RawValue, ctx: &mut RestoreContext<'_>) -> Result<Panel> {
    let saved: TablePanel = serde_json::from_str(raw.get()).context("invalid table panel")?;
    let source = match saved.source {
        SavedTableSource::Generator { trace, path } => {
            ensure!(!path.is_empty(), "empty table generator path");
            let found = ctx
                .traces
                .hierarchy(trace)
                .map_or(Lookup::Missing, |h| h.find_generator(&path));
            match found {
                Lookup::Found(generator) => {
                    let h = ctx.traces.hierarchy(trace).expect("found in it");
                    TableSource::Generator(TrackSource::Resolved {
                        track: Traced::new(trace, h.generators()[generator].track),
                        path,
                    })
                }
                other => {
                    ctx.report
                        .push(format!("{other:?} table generator: {path:?}"));
                    TableSource::Generator(TrackSource::Unresolved { trace, path })
                }
            }
        }
        SavedTableSource::Signals { signals } => {
            ensure!(!signals.is_empty(), "empty table signal set");
            let mut restored = Vec::with_capacity(signals.len());
            for signal in signals {
                ensure!(!signal.path.is_empty(), "empty table signal path");
                let trace = signal.trace;
                let h = ctx.traces.hierarchy(trace);
                let found = h.map_or(Lookup::Missing, |h| h.find_var(&signal.path, signal.nth));
                let (var, reference, name) = match (found, h) {
                    (Lookup::Found(var), Some(h)) => (
                        Some(var),
                        Some(Traced::new(trace, h.var(var).signal)),
                        h.full_name(var),
                    ),
                    (other, _) => {
                        ctx.report
                            .push(format!("{other:?} table signal: {:?}", signal.path));
                        (None, None, signal.path.join("."))
                    }
                };
                restored.push(SignalSource {
                    trace,
                    path: signal.path,
                    nth: signal.nth,
                    var,
                    signal: reference,
                    name,
                });
            }
            TableSource::Signals(restored)
        }
    };
    let mut table = TableModel::new(source, saved.link, ctx.budget.clone());
    ensure!(
        saved.widths.len() == saved.columns.len(),
        "invalid table column widths"
    );
    ensure!(
        saved.widths.iter().flatten().all(|width| width.is_finite()
            && (super::layout::MIN_COLUMN_WIDTH..=super::layout::MAX_COLUMN_WIDTH).contains(width)),
        "invalid table column width"
    );
    table.widths.restore(saved.widths);
    match (&table.source, &saved.columns) {
        (TableSource::Generator(_), ColumnSet::Transactions(columns)) => {
            ensure!(columns.validate(), "invalid transaction columns");
        }
        (TableSource::Signals(signals), ColumnSet::Signals { time, visible }) => ensure!(
            visible.len() == signals.len() && (*time || visible.iter().any(|v| *v)),
            "invalid signal columns"
        ),
        _ => anyhow::bail!("table columns do not match source"),
    }
    table.columns.restore(saved.columns);
    Ok(Panel {
        id: saved.id,
        title: crate::history::Journaled::new(saved.title),
        kind: PanelKind::Table(Box::new(table)),
    })
}
