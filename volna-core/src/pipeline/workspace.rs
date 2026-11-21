//! The pipeline panel's workspace schema, capture, validation and resolution.

use super::{FocusStyle, PipelineModel, PipelineSource, RowView, TrackSource};
use crate::clock::ClockView;
use crate::panels::workspace::valid_viewport;
use crate::panels::{Panel, PanelId, PanelKind, workspace::RestoreContext};
use crate::trace::{TraceId, Traced};
use crate::wave::{model::Link, viewport::Viewport};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub(crate) const VERSION: u32 = 7;

fn default_true() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if requires &T"
)]
fn is_true(value: &bool) -> bool {
    *value
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if requires &T"
)]
fn is_default_focus_style(style: &FocusStyle) -> bool {
    *style == FocusStyle::default()
}

#[derive(Serialize, Deserialize)]
struct SavedPipelineSource {
    #[serde(default, skip_serializing_if = "TraceId::is_a")]
    trace: TraceId,
    track: Vec<String>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PipelinePanel {
    #[serde(default)]
    follow: crate::pipeline::FollowActivity,
    id: PanelId,
    kind: String,
    version: u32,
    title: Option<String>,
    sources: Vec<SavedPipelineSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    focused_source: Option<usize>,
    #[serde(default, skip_serializing_if = "is_default_focus_style")]
    focus_style: FocusStyle,
    link: Link,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    viewport: Option<Viewport>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::panels::workspace::present_raw"
    )]
    cursor: Option<Box<RawValue>>,
    rows: RowView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    row_cap: Option<f32>,
    label_width: f32,
    navigator: super::NavigatorSettings,
    occupancy: super::OccupancySettings,
    #[serde(default, skip_serializing_if = "ClockView::is_default")]
    clocks: ClockView,
}

pub(crate) fn save(p: &PipelineModel, id: PanelId, title: Option<String>) -> Result<Box<RawValue>> {
    let sources = p
        .sources
        .iter()
        .map(|s| SavedPipelineSource {
            trace: s.track.trace(),
            track: s.track.path().to_vec(),
            visible: s.visible,
            alias: s.alias.clone(),
        })
        .collect();
    Ok(serde_json::value::to_raw_value(&PipelinePanel {
        follow: p.follow,
        id,
        kind: "pipeline".into(),
        version: VERSION,
        title,
        sources,
        focused_source: p.focused_source,
        focus_style: p.focus_style,
        link: p.nav.link,
        viewport: (!p.nav.link.viewport).then(|| p.nav.local_viewport.target()),
        cursor: (!p.nav.link.cursor)
            .then(|| serde_json::value::to_raw_value(&p.nav.local_cursor))
            .transpose()?,
        rows: p.rows.target(),
        row_cap: (p.row_cap != crate::pipeline::zoom::ROW_PX_CAP).then_some(p.row_cap),
        label_width: p.label_width,
        navigator: p.navigator,
        occupancy: p.occupancy,
        clocks: p.nav.clocks().clone(),
    })?)
}

pub(crate) fn restore(raw: &RawValue, ctx: &mut RestoreContext<'_>) -> Result<Panel> {
    let saved: PipelinePanel = serde_json::from_str(raw.get()).context("invalid pipeline panel")?;
    ensure!(!saved.sources.is_empty(), "empty pipeline sources");
    ensure!(
        saved
            .focused_source
            .is_none_or(|idx| idx < saved.sources.len()),
        "invalid focused pipeline source"
    );
    ensure!(
        saved.rows.top.is_finite()
            && saved.rows.row_px.is_finite()
            && (crate::pipeline::rows::ROW_PX_MIN..=crate::pipeline::rows::ROW_PX_MAX)
                .contains(&saved.rows.row_px),
        "invalid pipeline rows"
    );
    ensure!(
        saved.row_cap.is_none_or(|cap| {
            (crate::pipeline::zoom::ROW_PX_CAP..=crate::pipeline::rows::ROW_PX_MAX).contains(&cap)
        }),
        "invalid pipeline row cap"
    );
    ensure!(
        saved.label_width.is_finite()
            && (crate::pipeline::layout::LABEL_W_MIN..=crate::pipeline::layout::LABEL_W_MAX)
                .contains(&saved.label_width),
        "invalid pipeline label width"
    );
    ensure!(
        saved.link.viewport == saved.viewport.is_none(),
        "local viewport must exist exactly when unlinked"
    );
    ensure!(
        saved.link.cursor == saved.cursor.is_none(),
        "local cursor must exist exactly when unlinked"
    );
    ensure!(saved.navigator.valid(), "invalid pipeline navigator width");
    ensure!(saved.occupancy.valid(), "invalid pipeline occupancy height");

    let mut sources = Vec::with_capacity(saved.sources.len());
    for (slot, entry) in saved.sources.into_iter().enumerate() {
        ensure!(!entry.track.is_empty(), "empty pipeline track path");
        let trace = entry.trace;
        let track = if let Some(t) = ctx
            .traces
            .tracks(trace)
            .iter()
            .find(|t| t.path == entry.track)
        {
            TrackSource::Resolved {
                track: Traced::new(trace, t.id),
                path: entry.track,
            }
        } else {
            ctx.report
                .push(format!("Missing pipeline track: {:?}", entry.track));
            TrackSource::Unresolved {
                trace,
                path: entry.track,
            }
        };
        sources.push(PipelineSource {
            track,
            visible: entry.visible,
            alias: super::sources::clean_alias(entry.alias.as_deref()),
            color_slot: slot,
        });
    }

    let mut p = PipelineModel::from_sources(sources, saved.link);
    p.set_memory_budget(Some(ctx.budget.clone()));
    if p.sources.len() == 1 {
        p.sources[0].visible = true;
        p.focused_source = None;
        p.focus_style = FocusStyle::Dim;
    } else {
        p.focused_source = saved.focused_source;
        p.focus_style = saved.focus_style;
    }
    p.follow = saved.follow;
    p.nav.restore_clocks(saved.clocks);
    if let Some(v) = saved.viewport {
        valid_viewport(v)?;
        p.nav.local_viewport.set(v);
    }
    if let Some(c) = saved.cursor {
        p.nav.local_cursor = serde_json::from_str(c.get()).context("invalid local cursor")?;
    }
    p.rows.set(saved.rows);
    p.row_cap = saved.row_cap.unwrap_or(crate::pipeline::zoom::ROW_PX_CAP);
    p.label_width = saved.label_width;
    p.navigator = saved.navigator;
    p.occupancy = saved.occupancy;
    Ok(Panel {
        id: saved.id,
        title: crate::history::Journaled::new(saved.title),
        kind: PanelKind::Pipeline(Box::new(p)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::workspace::Traces;

    // User scenario: an engineer configures a merged multi-source pipeline panel with custom
    // aliases, a hidden source, and Isolate focus mode, saves the workspace, and restores it.
    #[test]
    fn merged_pipeline_round_trips_sources_aliases_visibility_and_focus() {
        let mut model = PipelineModel::from_sources(
            vec![
                PipelineSource {
                    track: TrackSource::Unresolved {
                        trace: TraceId::A,
                        path: vec!["sm[0]".into(), "warp_slot[0]".into(), "Instruction".into()],
                    },
                    visible: true,
                    alias: Some("main".into()),
                    color_slot: 0,
                },
                PipelineSource {
                    track: TrackSource::Unresolved {
                        trace: TraceId::A,
                        path: vec!["sm[0]".into(), "warp_slot[1]".into(), "Instruction".into()],
                    },
                    visible: false,
                    alias: None,
                    color_slot: 1,
                },
            ],
            Link::default(),
        );
        model.focused_source = Some(0);
        model.focus_style = FocusStyle::Isolate;

        let saved = save(&model, PanelId(7), None).unwrap();
        let doc = crate::Document::new();
        let traces = Traces::default();
        let mut report = crate::workspace::RestoreReport::default();
        let mut ctx = RestoreContext {
            doc: &doc,
            traces: &traces,
            budget: volna_trace::remote::memory::MemoryBudget::new(1 << 20),
            detail_items: 64,
            report: &mut report,
            row_count: 0,
        };
        let restored = restore(&saved, &mut ctx).unwrap();
        let restored = restored.kind.pipeline().unwrap();
        assert_eq!(restored.sources.len(), 2);
        assert_eq!(restored.sources[0].alias.as_deref(), Some("main"));
        assert!(restored.sources[0].visible);
        assert!(!restored.sources[1].visible);
        assert_eq!(restored.focused_source, Some(0));
        assert_eq!(restored.focus_style, FocusStyle::Isolate);
        assert_eq!(restored.badges().labels[0].badge, "main");
        assert_eq!(restored.badges().labels[1].badge, "w1");
    }
}
