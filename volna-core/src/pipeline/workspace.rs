//! The pipeline panel's workspace schema, capture, validation and resolution.

use super::{
    FocusStyle, PipelineModel, PipelineSource, RowOrder, RowView, StageVisibility, TrackSource,
};
use crate::clock::ClockView;
use crate::panels::workspace::valid_viewport;
use crate::panels::{Panel, PanelId, PanelKind, workspace::RestoreContext};
use crate::trace::{TraceId, Traced};
use crate::wave::{model::Link, viewport::Viewport};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub(crate) const VERSION: u32 = 9;

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
    #[serde(default, skip_serializing_if = "RowOrder::is_default")]
    order: RowOrder,
    #[serde(default, skip_serializing_if = "StageVisibility::is_empty")]
    hidden_stages: StageVisibility,
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
        order: p.order.clone(),
        hidden_stages: p.stage_visibility.clone(),
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
    ensure!(saved.order.valid(), "invalid pipeline row order");
    ensure!(
        saved.hidden_stages.valid(),
        "invalid pipeline hidden stages"
    );

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
    p.order = saved.order;
    p.order.normalize();
    p.stage_visibility = saved.hidden_stages;
    p.stage_visibility.normalize();
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
        assert_eq!(restored.order, RowOrder::default());
    }

    // User scenario: an engineer saves a workspace with custom stage-aware row ordering
    // (FromStage + Hide + disabled sort-key follow, and multi-stage Stages) and reopens it.
    // Requirement: RowOrder round-trips cleanly across workspace v8 persistence, and malformed
    // empty stage lists are rejected during restore.
    #[test]
    fn pipeline_workspace_round_trips_row_order_and_rejects_empty_stage_order() {
        use super::super::{RowOrderMode, UnmatchedPolicy};

        fn make_ctx<'a>(
            doc: &'a crate::Document,
            traces: &'a Traces<'a>,
            report: &'a mut crate::workspace::RestoreReport,
        ) -> RestoreContext<'a> {
            RestoreContext {
                doc,
                traces,
                budget: volna_trace::remote::memory::MemoryBudget::new(1 << 20),
                detail_items: 64,
                report,
                row_count: 0,
            }
        }

        for order in [
            RowOrder {
                mode: RowOrderMode::FromStage {
                    stage: "execute".into(),
                },
                unmatched: UnmatchedPolicy::Hide,
                follow_sort_key: false,
            },
            RowOrder {
                mode: RowOrderMode::Stages {
                    stages: vec!["issue".into(), "memory_wait".into()],
                },
                unmatched: UnmatchedPolicy::FallbackStart,
                follow_sort_key: true,
            },
            RowOrder {
                mode: RowOrderMode::End,
                unmatched: UnmatchedPolicy::FallbackEnd,
                follow_sort_key: true,
            },
        ] {
            let mut model = PipelineModel::new(
                TrackSource::Unresolved {
                    trace: TraceId::A,
                    path: vec!["sm[0]".into(), "Instruction".into()],
                },
                Link::default(),
            );
            model.order = order.clone();
            let saved = save(&model, PanelId(9), None).unwrap();
            let doc = crate::Document::new();
            let traces = Traces::default();
            let mut report = crate::workspace::RestoreReport::default();
            let mut ctx = make_ctx(&doc, &traces, &mut report);
            let restored = restore(&saved, &mut ctx).unwrap();
            assert_eq!(restored.kind.pipeline().unwrap().order, order);
        }

        // Invalid empty stage list must fail validation on restore.
        let mut model = PipelineModel::new(
            TrackSource::Unresolved {
                trace: TraceId::A,
                path: vec!["sm[0]".into(), "Instruction".into()],
            },
            Link::default(),
        );
        model.order = RowOrder {
            mode: RowOrderMode::Stages { stages: vec![] },
            ..RowOrder::default()
        };
        let bad = save(&model, PanelId(10), None).unwrap();
        let doc = crate::Document::new();
        let traces = Traces::default();
        let mut report = crate::workspace::RestoreReport::default();
        let mut ctx = make_ctx(&doc, &traces, &mut report);
        assert!(restore(&bad, &mut ctx).is_err());
    }

    // User scenario: an engineer hides two primary-lane stages alongside a custom row order,
    // saves the workspace, and restores it.
    // Requirement: StageVisibility round-trips cleanly across workspace v9 persistence, omits
    // `hidden_stages` when empty, and rejects empty stage names on restore.
    #[test]
    fn pipeline_workspace_round_trips_hidden_stages_and_rejects_empty_stage_name() {
        let mut model = PipelineModel::new(
            TrackSource::Unresolved {
                trace: TraceId::A,
                path: vec!["sm[0]".into(), "Instruction".into()],
            },
            Link::default(),
        );
        let default_saved = save(&model, PanelId(11), None).unwrap();
        assert!(
            !default_saved.get().contains("hidden_stages"),
            "default empty hidden_stages should be omitted from workspace JSON"
        );

        model.stage_visibility.toggle("fetch".into());
        model.stage_visibility.toggle("decode".into());
        let saved = save(&model, PanelId(11), None).unwrap();
        assert!(saved.get().contains("hidden_stages"));

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
        assert_eq!(restored.stage_visibility, model.stage_visibility);
        assert!(restored.stage_visibility.is_hidden("fetch"));
        assert!(restored.stage_visibility.is_hidden("decode"));
        assert!(!restored.stage_visibility.is_hidden("exec"));

        // Malformed workspace with an empty hidden stage name must be rejected.
        let bad_json = saved.get().replace(r#""decode""#, r#""""#);
        let bad_raw = serde_json::value::RawValue::from_string(bad_json).unwrap();
        let mut report = crate::workspace::RestoreReport::default();
        let mut ctx = RestoreContext {
            doc: &doc,
            traces: &traces,
            budget: volna_trace::remote::memory::MemoryBudget::new(1 << 20),
            detail_items: 64,
            report: &mut report,
            row_count: 0,
        };
        assert!(restore(&bad_raw, &mut ctx).is_err());
    }
}
