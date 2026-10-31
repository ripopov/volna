//! The transaction panel's workspace schema, capture, validation and resolution.

use super::{ShownRecord, TransactionModel, ViewPrefs, view::SectionKey};
use crate::panels::{Panel, PanelId, PanelKind, workspace::RestoreContext};
use crate::pipeline::TrackSource;
use crate::trace::{TraceId, Traced};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub(crate) const VERSION: u32 = 3;

#[derive(Serialize, Deserialize)]
struct SavedRecord {
    #[serde(default, skip_serializing_if = "TraceId::is_a")]
    trace: TraceId,
    track: Vec<String>,
    id: u64,
    range: Option<crate::document::RecordRange>,
}

#[derive(Serialize, Deserialize)]
struct TransactionPanel {
    id: PanelId,
    kind: String,
    version: u32,
    title: Option<String>,
    pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record: Option<SavedRecord>,
    #[serde(default)]
    collapsed: Vec<String>,
    #[serde(default)]
    radix: std::collections::BTreeMap<String, String>,
}

pub(crate) fn save(
    model: &TransactionModel,
    id: PanelId,
    title: Option<String>,
) -> Result<Box<RawValue>> {
    let shown = model.shown();
    Ok(serde_json::value::to_raw_value(&TransactionPanel {
        id,
        kind: "transaction".into(),
        version: VERSION,
        title,
        pinned: model.pinned(),
        record: shown.map(|record| SavedRecord {
            trace: record.track.trace(),
            track: record.track.path().to_vec(),
            id: record.id.0,
            range: record.range,
        }),
        collapsed: model
            .prefs()
            .collapsed
            .iter()
            .map(|section| section.key().to_owned())
            .collect(),
        radix: model
            .prefs()
            .radix
            .iter()
            .map(|(key, radix)| (key.clone(), radix.name().to_owned()))
            .collect(),
    })?)
}

pub(crate) fn restore(raw: &RawValue, ctx: &mut RestoreContext<'_>) -> Result<Panel> {
    let saved: TransactionPanel =
        serde_json::from_str(raw.get()).context("invalid transaction panel")?;
    let mut model = TransactionModel::new(ctx.budget.clone(), ctx.detail_items);
    let mut prefs = ViewPrefs::default();
    for key in &saved.collapsed {
        let section = SectionKey::parse(key)
            .ok_or_else(|| anyhow::anyhow!("unknown transaction section {key:?}"))?;
        prefs.collapsed.insert(section);
    }
    for (key, name) in &saved.radix {
        let radix = crate::data::text::Radix::parse(name)
            .ok_or_else(|| anyhow::anyhow!("unknown radix {name:?}"))?;
        prefs.radix.insert(key.clone(), radix);
    }
    let record = match saved.record {
        Some(record) => {
            if let Some(range) = record.range {
                ensure!(range.begin <= range.end, "invalid record group time range");
            }
            ensure!(!record.track.is_empty(), "empty transaction track path");
            let trace = record.trace;
            let found = ctx
                .traces
                .tracks(trace)
                .iter()
                .find(|t| t.path == record.track);
            let track = if let Some(t) = found {
                TrackSource::Resolved {
                    track: Traced::new(trace, t.id),
                    path: record.track,
                }
            } else {
                ctx.report
                    .push(format!("Missing transaction track: {:?}", record.track));
                TrackSource::Unresolved {
                    trace,
                    path: record.track,
                }
            };
            Some(ShownRecord {
                track,
                id: volna_trace::data::transactions::TransactionRef(record.id),
                range: record.range,
            })
        }
        None => None,
    };
    model.restore(record, saved.pinned, prefs);
    Ok(Panel {
        id: saved.id,
        title: crate::history::Journaled::new(saved.title),
        kind: PanelKind::Transaction(Box::new(model)),
    })
}
