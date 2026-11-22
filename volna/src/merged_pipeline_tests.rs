//! End-to-end GPUI integration tests for merged multi-generator pipeline panels using the
//! realistic 8-slot-per-partition synthetic GPU trace (`volna/examples/gpu_transaction_trace.vtr`).

use super::*;
use gpui_kit::{VisualTestContext, px, size};
use volna_core::{
    analysis::AnalysisLoad,
    pipeline::{FocusStyle, FollowActivity, PipelineSourceCommand, Rows, SourceLoadStatus},
    trace::Traced,
    wave::viewport::Viewport,
};
use volna_trace::data::transactions::TrackRef;

fn gpu_trace_path() -> std::path::PathBuf {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/gpu_transaction_trace.vtr"
    )
    .into()
}

fn find_tracks_matching(
    ws: &Workspace,
    predicate: impl Fn(&str) -> bool,
) -> Vec<(String, Traced<TrackRef>)> {
    let mut matches: Vec<_> = ws
        .app
        .transaction_generators()
        .into_iter()
        .filter(|(path, _)| predicate(path))
        .collect();
    matches.sort_by(|a, b| a.0.cmp(&b.0));
    matches
}

fn open_gpu_trace(
    cx: &mut TestAppContext,
) -> (gpui_kit::WindowHandle<Workspace>, VisualTestContext) {
    init(cx);
    let host = cx.add_window(Workspace::new);
    host.update(cx, |ws, _, cx| {
        ws.open_path(gpu_trace_path(), cx);
    })
    .unwrap();
    let vcx = VisualTestContext::from_window(host.into(), cx);
    vcx.simulate_resize(size(px(1280.0), px(840.0)));
    vcx.run_until_parked();
    (host, vcx)
}

fn pump_frames(vcx: &mut VisualTestContext, count: usize) {
    for _ in 0..count {
        vcx.run_until_parked();
        vcx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
    }
    vcx.run_until_parked();
}

// User scenario: an engineer opens all 8 warp_slot[0..7]/Instruction generators from
// sm[0]/simt_partition[0] in a single merged pipeline panel and inspects interleaved execution.
// Requirement: all 8 sources load cleanly, rows equal the sum across all 8 slots ordered
// non-decreasingly by begin and interleaved across slots, badges resolve to w0..w7, shared
// StagePalette covers instruction stages, aggregate occupancy computes, and both themes paint.
#[gpui_kit::test]
fn gpu_trace_merges_eight_warp_slots_interleaved_with_badges_palette_and_occupancy(
    cx: &mut TestAppContext,
) {
    let (host, mut vcx) = open_gpu_trace(cx);
    let (panel, p0_tracks) = host
        .update(&mut vcx, |ws, window, cx| {
            let p0 = find_tracks_matching(ws, |path| {
                path.contains("sm[0].simt_partition[0].warp_slot[")
                    && path.ends_with(".Instruction")
            });
            assert_eq!(
                p0.len(),
                8,
                "expected 8 warp slots in sm[0].simt_partition[0]"
            );
            let tracks: Vec<_> = p0.iter().map(|(_, t)| *t).collect();
            ws.dispatch(
                Command::OpenMergedPipeline {
                    tracks: tracks.clone(),
                },
                Some(window),
                cx,
            );
            (ws.app.panels.focused_id(), tracks)
        })
        .unwrap();

    pump_frames(&mut vcx, 3);

    host.update(&mut vcx, |ws, _, _| {
        let pipeline = ws
            .app
            .panels
            .pipeline(panel)
            .expect("merged pipeline panel");
        assert_eq!(pipeline.sources.len(), 8);
        assert!(pipeline.is_merged());

        let mut expected_total = 0usize;
        for (i, &track) in p0_tracks.iter().enumerate() {
            let status = pipeline.source_status(&ws.app.doc, i);
            match status {
                SourceLoadStatus::Ready { count } => {
                    assert!(
                        count >= 2000,
                        "slot {i} expected >=2000 instructions, got {count}"
                    );
                    expected_total += count;
                }
                other => panic!("expected Ready for source {i} ({track:?}), got {other:?}"),
            }
        }

        let Rows::Ready(set) = pipeline.rows(&ws.app.doc) else {
            panic!("merged pipeline rows should be Ready");
        };
        assert_eq!(set.len(), expected_total);

        // Verify monotonic begin ordering and genuine multi-warp interleaving.
        let mut prev_begin = 0u64;
        let mut transitions = 0usize;
        let mut prev_src = None;
        for r in 0..set.len() {
            let (src, tx) = set.get(r).unwrap();
            assert!(
                tx.begin >= prev_begin,
                "row {r} begin {} < prev {}",
                tx.begin,
                prev_begin
            );
            prev_begin = tx.begin;
            if let Some(ps) = prev_src
                && ps != src
            {
                transitions += 1;
            }
            prev_src = Some(src);
        }
        assert!(
            transitions > 1000,
            "expected rich multi-warp interleaving, got only {transitions} source switches"
        );

        let badges: Vec<&str> = pipeline
            .badges()
            .labels
            .iter()
            .map(|l| l.badge.as_str())
            .collect();
        assert_eq!(badges, ["w0", "w1", "w2", "w3", "w4", "w5", "w6", "w7"]);
        let title = ws.app.panels.get(panel).unwrap().title();
        assert!(
            title.contains("simt_partition[0]"),
            "unexpected title: {title}"
        );
        assert!(title.contains("(8)"), "unexpected title: {title}");

        let stage_names = pipeline.palette().names();
        for required in [
            "fetch",
            "decode",
            "scheduler_wait",
            "issue",
            "execute",
            "memory_wait",
            "barrier_wait",
            "writeback",
        ] {
            assert!(
                stage_names.iter().any(|s| s == required),
                "shared StagePalette missing stage {required:?}; got {stage_names:?}"
            );
        }

        let trace = pipeline.primary_trace().unwrap();
        match ws.app.doc.occupancy(trace, set.generators()) {
            Some(AnalysisLoad::Ready(summary)) => {
                assert!(
                    summary.maximum() >= 8,
                    "expected aggregate occupancy >= 8 across 8 warps, got {}",
                    summary.maximum()
                );
            }
            _ => panic!("expected aggregate occupancy Ready"),
        }
    })
    .unwrap();

    // Verify rendering in both dark and light themes.
    for dark in [true, false] {
        host.update(&mut vcx, |_, _, cx| {
            crate::theme::set(crate::theme::CoreTheme::volna(dark), cx);
            cx.notify();
        })
        .unwrap();
        pump_frames(&mut vcx, 1);
    }
}

// User scenario: an engineer focuses warp_slot[2] in Solo (Dim) mode to inspect it in peer
// context, switches to Isolate (Hide) mode to collapse to warp_slot[2]'s rows while keeping the
// time viewport anchored, then clicks Show All to restore all 8 slots.
// Requirement: Solo keeps row count unchanged while dimming peers; Isolate narrows rows strictly
// to warp_slot[2], preserves the time window, and updates FollowActivity cleanly; Clear restores.
#[gpui_kit::test]
fn gpu_trace_solo_dim_and_isolate_preserve_time_window_and_update_follow_activity(
    cx: &mut TestAppContext,
) {
    let (host, mut vcx) = open_gpu_trace(cx);
    let panel = host
        .update(&mut vcx, |ws, window, cx| {
            let tracks: Vec<_> = find_tracks_matching(ws, |path| {
                path.contains("sm[0].simt_partition[0].warp_slot[")
                    && path.ends_with(".Instruction")
            })
            .into_iter()
            .map(|(_, t)| t)
            .collect();
            ws.dispatch(Command::OpenMergedPipeline { tracks }, Some(window), cx);
            ws.app.panels.focused_id()
        })
        .unwrap();
    pump_frames(&mut vcx, 3);

    let (total_rows, w2_rows, anchored_vp) = host
        .update(&mut vcx, |ws, _, _| {
            ws.app.doc.navigation.animation = volna_core::settings::Animation::Off;
            let (lo, hi) = ws.app.doc.limits();
            let mid = lo + (hi - lo) / 4;
            let vp = Viewport {
                start: mid as f64,
                end: (mid + (hi - lo) / 20).max(mid + 100) as f64,
            };
            ws.app.doc.shared.viewport.set(vp);
            let p = ws.app.panels.pipeline_mut(panel).unwrap();
            p.follow = FollowActivity::Following;
            let total = p.row_count(&ws.app.doc);
            let SourceLoadStatus::Ready { count: w2 } = p.source_status(&ws.app.doc, 2) else {
                panic!("warp_slot[2] should be Ready");
            };
            (total, w2, vp)
        })
        .unwrap();
    pump_frames(&mut vcx, 1);

    // 1. Solo (Dim peers) on warp_slot[2].
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::ToggleFocus(2)),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, _, _| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.focused_source, Some(2));
        assert_eq!(p.focus_style, FocusStyle::Dim);
        assert_eq!(p.row_count(&ws.app.doc), total_rows);
        assert!(!p.is_source_dimmed(2));
        for peer in [0, 1, 3, 4, 5, 6, 7] {
            assert!(
                p.is_source_dimmed(peer),
                "peer {peer} should be dimmed in Solo mode"
            );
        }
        assert_eq!(p.nav.viewport(&ws.app.doc), anchored_vp);
    })
    .unwrap();

    // 2. Switch to Isolate (Hide peers).
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::PipelineSources(
                panel,
                PipelineSourceCommand::SetFocusStyle(FocusStyle::Isolate),
            ),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, _, _| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.focus_style, FocusStyle::Isolate);
        assert_eq!(p.row_count(&ws.app.doc), w2_rows);
        assert_eq!(
            p.nav.viewport(&ws.app.doc),
            anchored_vp,
            "Isolate must keep time viewport anchored"
        );
        let Rows::Ready(set) = p.rows(&ws.app.doc) else {
            panic!("isolated rows should be Ready");
        };
        for r in [0, set.len() / 2, set.len() - 1] {
            assert_eq!(
                set.get(r).unwrap().0,
                2,
                "isolated row {r} must belong to w2"
            );
        }
        let act = p.activity(&ws.app.doc);
        assert!(
            act.visible > 0 || act.above > 0 || act.below > 0,
            "FollowActivity should report activity in isolated view"
        );
        if let Some(target) = act.target {
            assert!(target < w2_rows);
        }
    })
    .unwrap();

    // 3. Clear focus & restore all 8 slots.
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::ClearFocusAndShowAll),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, _, _| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.focused_source, None);
        assert_eq!(p.focus_style, FocusStyle::Dim);
        assert_eq!(p.row_count(&ws.app.doc), total_rows);
        assert!((0..8).all(|i| !p.is_source_dimmed(i)));
    })
    .unwrap();
}

// User scenario: an engineer adds a warp slot from simt_partition[1] to a simt_partition[0]
// panel, sets a custom alias override, tests adding a heterogeneous generator for the non-blocking
// stage warning, then removes the added sources.
// Requirement: badges adapt automatically from w0..w7 to 0:0..1:0, custom alias overrides verbatim,
// disjoint stage vocabularies raise a non-blocking notice, and removal reverts badges cleanly.
#[gpui_kit::test]
fn gpu_trace_dynamic_composition_adapts_cross_partition_badges_aliases_and_disjoint_notice(
    cx: &mut TestAppContext,
) {
    let (host, mut vcx) = open_gpu_trace(cx);
    let (panel, p1_w0, hetero_track) = host
        .update(&mut vcx, |ws, window, cx| {
            let p0: Vec<_> = find_tracks_matching(ws, |path| {
                path.contains("sm[0].simt_partition[0].warp_slot[")
                    && path.ends_with(".Instruction")
            })
            .into_iter()
            .map(|(_, t)| t)
            .collect();
            let p1_w0 = find_tracks_matching(ws, |path| {
                path.contains("sm[0].simt_partition[1].warp_slot[0].Instruction")
            })[0]
                .1;
            let hetero = find_tracks_matching(ws, |path| {
                path.contains("memory_fabric") && path.ends_with(".MemoryPacket")
            })[0]
                .1;
            ws.dispatch(Command::OpenMergedPipeline { tracks: p0 }, Some(window), cx);
            (ws.app.panels.focused_id(), p1_w0, hetero)
        })
        .unwrap();
    pump_frames(&mut vcx, 3);

    // Add simt_partition[1]/warp_slot[0]/Instruction -> badges adapt to 0:0..0:7, 1:0.
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::Add(p1_w0)),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, window, cx| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        let badges: Vec<&str> = p.badges().labels.iter().map(|l| l.badge.as_str()).collect();
        assert_eq!(
            badges,
            [
                "0:0", "0:1", "0:2", "0:3", "0:4", "0:5", "0:6", "0:7", "1:0"
            ]
        );
        assert!(p.disjoint_notice().is_none());

        // Set custom alias on the newly added cross-partition source.
        ws.dispatch(
            Command::PipelineSources(
                panel,
                PipelineSourceCommand::SetAlias(8, Some("p1w0".into())),
            ),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, window, cx| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.badges().labels[8].badge, "p1w0");
        assert_eq!(p.badges().labels[8].auto_badge, "1:0");

        // Add a heterogeneous generator -> non-blocking disjoint-stage notice appears.
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::Add(hetero_track)),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, window, cx| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.sources.len(), 10);
        assert!(
            p.disjoint_notice().is_some(),
            "mixing Instruction with MemoryPacket/Workgroup should surface a non-blocking notice"
        );

        // Remove heterogeneous source and cross-partition source -> badges revert to w0..w7.
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::Remove(9)),
            Some(window),
            cx,
        );
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::Remove(8)),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, _, _| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.sources.len(), 8);
        let badges: Vec<&str> = p.badges().labels.iter().map(|l| l.badge.as_str()).collect();
        assert_eq!(badges, ["w0", "w1", "w2", "w3", "w4", "w5", "w6", "w7"]);
        assert!(p.disjoint_notice().is_none());
        assert!(
            ws.app.doc.track(p1_w0).is_none(),
            "removed source track should be released cleanly"
        );
    })
    .unwrap();
}

// User scenario: an engineer selects an instruction with a depends_on relation in a merged
// 8-warp panel, inspects its relation arrows, and reveals a transaction from another warp slot.
// Requirement: selection maps to the exact merged row and originating generator, relation arrows
// render across merged row coordinates, and reveal_record scrolls directly to the target row.
#[gpui_kit::test]
fn gpu_trace_selection_dependency_arrows_and_reveal_record_work_across_merged_rows(
    cx: &mut TestAppContext,
) {
    let (host, mut vcx) = open_gpu_trace(cx);
    let panel = host
        .update(&mut vcx, |ws, window, cx| {
            let tracks: Vec<_> = find_tracks_matching(ws, |path| {
                path.contains("sm[0].simt_partition[0].warp_slot[")
                    && path.ends_with(".Instruction")
            })
            .into_iter()
            .map(|(_, t)| t)
            .collect();
            ws.dispatch(Command::OpenMergedPipeline { tracks }, Some(window), cx);
            ws.app.panels.focused_id()
        })
        .unwrap();
    pump_frames(&mut vcx, 3);

    // Find a merged row whose transaction has an intra-warp depends_on relation in the same panel,
    // plus a distant target transaction from a different warp slot.
    let (dep_row, dep_track, dep_id, distant_row, distant_track, distant_id) = host
        .update(&mut vcx, |ws, _, _| {
            let p = ws.app.panels.pipeline(panel).unwrap();
            let Rows::Ready(set) = p.rows(&ws.app.doc) else {
                panic!("rows should be Ready");
            };
            let mut dep = None;
            for r in 0..set.len() {
                let (_, trace, tx) = set.get_with_trace(r).unwrap();
                let traced = Traced::new(trace, tx.generator);
                if let Some(generator) = ws.app.doc.resident_generator(traced)
                    && generator.relations_of(tx.id).any(|edge| {
                        edge.from_generator
                            .and_then(|g| set.row_of(trace, g, edge.relation.from))
                            .is_some()
                            && edge
                                .to_generator
                                .and_then(|g| set.row_of(trace, g, edge.relation.to))
                                .is_some()
                    })
                {
                    dep = Some((r, traced, tx.id));
                    break;
                }
            }
            let (dep_row, dep_track, dep_id) =
                dep.expect("GPU trace should contain depends_on relations");
            let far_idx = (set.len() * 3 / 4).min(set.len() - 1);
            let (_, far_trace, far_tx) = set.get_with_trace(far_idx).unwrap();
            (
                dep_row,
                dep_track,
                dep_id,
                far_idx,
                Traced::new(far_trace, far_tx.generator),
                far_tx.id,
            )
        })
        .unwrap();

    // Select the instruction with relations and reveal it.
    host.update(&mut vcx, |ws, _, _| {
        ws.app.doc.navigation.animation = volna_core::settings::Animation::Off;
        let now = volna_core::Instant::now();
        let p = ws.app.panels.pipeline_mut(panel).unwrap();
        assert!(p.select_row(&mut ws.app.doc, panel, dep_row));
        let sel = ws.app.doc.selection().expect("selection set");
        assert_eq!(sel.track, dep_track);
        assert_eq!(sel.id, dep_id);
        assert_eq!(p.selected_row(&ws.app.doc), Some(dep_row));
        assert!(p.reveal_record(&mut ws.app.doc, dep_track, dep_id, now));
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    // Verify painting renders the selected row & relation arrows cleanly.
    host.update(&mut vcx, |ws, _, _| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert!(
            p.last_layout().row_range.contains(&dep_row),
            "revealed row {dep_row} should be inside visible row_range {:?}",
            p.last_layout().row_range
        );
    })
    .unwrap();

    // Now reveal a distant transaction via Command::SelectTransaction + Command::RevealTransaction
    // (simulating navigating from the Record details / Table panel).
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::SelectTransaction {
                panel: ws.app.panels.first_free_transaction().unwrap_or(panel),
                track: distant_track,
                id: distant_id,
                cursor: None,
            },
            Some(window),
            cx,
        );
        if let Some(tx_panel) = ws.app.panels.first_free_transaction() {
            ws.dispatch(
                Command::RevealTransaction { panel: tx_panel },
                Some(window),
                cx,
            );
        }
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, _, _| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.selected_row(&ws.app.doc), Some(distant_row));
        assert!(
            p.last_layout().row_range.contains(&distant_row),
            "distant revealed row {distant_row} should be visible in {:?}",
            p.last_layout().row_range
        );
    })
    .unwrap();
}

// User scenario: an engineer adds a source, reorders sources, sets a custom alias, splits the
// merged pipeline panel, and uses Undo/Redo and panel close.
// Requirement: structural source edits undo/redo cleanly with proper track retain/release, split
// panels share tracks safely without leaking on close, and reducing to 1 source clears focus/hidden.
#[gpui_kit::test]
fn gpu_trace_undo_redo_split_and_single_source_downgrade_keep_tracks_consistent(
    cx: &mut TestAppContext,
) {
    let (host, mut vcx) = open_gpu_trace(cx);
    let (panel, w0, w1, w2) = host
        .update(&mut vcx, |ws, window, cx| {
            let p0 = find_tracks_matching(ws, |path| {
                path.contains("sm[0].simt_partition[0].warp_slot[")
                    && path.ends_with(".Instruction")
            });
            let (w0, w1, w2) = (p0[0].1, p0[1].1, p0[2].1);
            ws.dispatch(
                Command::OpenMergedPipeline {
                    tracks: vec![w0, w1],
                },
                Some(window),
                cx,
            );
            (ws.app.panels.focused_id(), w0, w1, w2)
        })
        .unwrap();
    pump_frames(&mut vcx, 3);

    // Add w2, reorder w2 to front, alias it, then Undo all three and Redo them.
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::Add(w2)),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, window, cx| {
        assert_eq!(ws.app.panels.pipeline(panel).unwrap().sources.len(), 3);
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::Reorder { from: 2, to: 0 }),
            Some(window),
            cx,
        );
        ws.dispatch(
            Command::PipelineSources(
                panel,
                PipelineSourceCommand::SetAlias(0, Some("top".into())),
            ),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, window, cx| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.badges().labels[0].badge, "top");
        assert_eq!(p.sources[0].track.track(), Some(w2));

        // Undo alias, reorder, add -> back to [w0, w1], and w2 released cleanly.
        ws.dispatch(Command::Undo, Some(window), cx);
        ws.dispatch(Command::Undo, Some(window), cx);
        ws.dispatch(Command::Undo, Some(window), cx);
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, window, cx| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.sources.len(), 2);
        assert_eq!(p.sources[0].track.track(), Some(w0));
        assert_eq!(p.sources[1].track.track(), Some(w1));
        assert!(
            ws.app.doc.track(w2).is_none(),
            "undoing Add must release the added track"
        );

        // Redo Add -> w2 retained again.
        ws.dispatch(Command::Redo, Some(window), cx);
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    // Split panel -> both hold tracks; closing split leaves original tracks loaded.
    let split_panel = host
        .update(&mut vcx, |ws, window, cx| {
            assert_eq!(ws.app.panels.pipeline(panel).unwrap().sources.len(), 3);
            ws.dispatch(
                Command::Panels(volna_core::panels::PanelsCommand::Split {
                    panel,
                    axis: volna_core::panels::Axis::Horizontal,
                }),
                Some(window),
                cx,
            );
            ws.app.panels.focused_id()
        })
        .unwrap();
    assert_ne!(split_panel, panel);
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, window, cx| {
        assert_eq!(
            ws.app.panels.pipeline(split_panel).unwrap().sources.len(),
            3
        );
        ws.dispatch(
            Command::Panels(volna_core::panels::PanelsCommand::Close(split_panel)),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    // Isolate w1, then remove w2 and w1 down to sole w0 -> w0 must become visible & unfocused.
    host.update(&mut vcx, |ws, window, cx| {
        assert!(ws.app.doc.track(w0).is_some());
        assert!(ws.app.doc.track(w1).is_some());
        assert!(ws.app.doc.track(w2).is_some());
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::Isolate(1)),
            Some(window),
            cx,
        );
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::Remove(2)),
            Some(window),
            cx,
        );
        ws.dispatch(
            Command::PipelineSources(panel, PipelineSourceCommand::Remove(1)),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, _, _| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.sources.len(), 1);
        assert!(
            p.sources[0].visible,
            "sole remaining source must be visible"
        );
        assert_eq!(p.focused_source, None);
        assert_eq!(p.focus_style, FocusStyle::Dim);
        assert!(ws.app.doc.track(w0).is_some());
        assert!(ws.app.doc.track(w1).is_none());
        assert!(ws.app.doc.track(w2).is_none());
    })
    .unwrap();
}

// User scenario: an engineer merges all 16 warp slots across two SIMT partitions (30k+ rows),
// clicks a row's source badge in the label column to Solo it, verifies FollowActivity tracks only
// the soloed source, and presses Escape to restore all 16 slots.
#[gpui_kit::test]
fn gpu_trace_sixteen_warp_slots_label_badge_solo_and_escape(cx: &mut TestAppContext) {
    let (host, mut vcx) = open_gpu_trace(cx);
    let panel = host
        .update(&mut vcx, |ws, window, cx| {
            let tracks: Vec<_> = find_tracks_matching(ws, |path| {
                path.contains("sm[0].simt_partition[")
                    && path.contains(".warp_slot[")
                    && path.ends_with(".Instruction")
            })
            .into_iter()
            .map(|(_, t)| t)
            .collect();
            assert_eq!(
                tracks.len(),
                16,
                "expected 16 warp slots across 2 partitions"
            );
            ws.dispatch(Command::OpenMergedPipeline { tracks }, Some(window), cx);
            ws.app.panels.focused_id()
        })
        .unwrap();
    pump_frames(&mut vcx, 4);

    let (badge_pt, expected_source) = host
        .update(&mut vcx, |ws, _, _| {
            let p = ws.app.panels.pipeline(panel).unwrap();
            assert_eq!(p.sources.len(), 16);
            assert!(p.row_count(&ws.app.doc) >= 20_000);
            assert_eq!(p.badges().labels[0].badge, "0:0");
            assert_eq!(p.badges().labels[15].badge, "1:7");

            let layout = p.last_layout();
            let row = layout.row_range.start;
            let Rows::Ready(set) = p.rows(&ws.app.doc) else {
                panic!("rows should be Ready");
            };
            let (src, _) = set.get(row).unwrap();
            let pt = volna_core::geometry::point(
                layout.badge_left + layout.badge_width * 0.5,
                layout.row_y(row) + layout.rows.row_px * 0.5,
            );
            assert_eq!(p.label_badge_at(&ws.app.doc, pt), Some(src));
            (pt, src)
        })
        .unwrap();

    // Clicking the badge through PipelineModel::pointer toggles Solo (Dim) on that row's source.
    host.update(&mut vcx, |ws, _, _| {
        let now = volna_core::Instant::now();
        let p = ws.app.panels.pipeline_mut(panel).unwrap();
        p.pointer(
            &mut ws.app.doc,
            panel,
            volna_core::wave::PointerEvent::Down {
                position: badge_pt,
                button: volna_core::geometry::MouseButton::Left,
                modifiers: volna_core::geometry::Modifiers::default(),
            },
            now,
        );
        assert_eq!(p.focused_source, Some(expected_source));
        assert_eq!(p.focus_style, FocusStyle::Dim);
        let act = p.activity(&ws.app.doc);
        if let Some(target) = act.target {
            let Rows::Ready(set) = p.rows(&ws.app.doc) else {
                panic!("rows should be Ready");
            };
            assert_eq!(
                set.get(target).unwrap().0,
                expected_source,
                "Solo FollowActivity target must belong to the soloed source"
            );
        }
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    // Escape clears Solo focus & restores full view across both themes.
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::Action(volna_core::app::Action::ClearSelection),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, _, cx| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.focused_source, None);
        crate::theme::set(crate::theme::CoreTheme::volna(false), cx);
        cx.notify();
    })
    .unwrap();
    pump_frames(&mut vcx, 1);
}

// User scenario: an engineer opens a merged 8-warp GPU pipeline panel, reorders rows by a
// primary-lane stage, switches unmatched policy (FallbackEnd -> FallbackStart -> Hide), tries
// FromStage and End modes, uses Undo/Redo, splits the panel, and inspects the Order popover in
// both dark and light themes.
// Requirement: RowOrder sorts rows monotonically by sort key, status bar reflects active order,
// Hide filters unmatched rows, Undo/Redo restores prior RowOrder, split clones inherit RowOrder,
// and both themes paint cleanly with the Order popover open.
#[gpui_kit::test]
fn gpu_trace_stage_aware_row_ordering_popover_undo_redo_clone_and_themes(cx: &mut TestAppContext) {
    use volna_core::pipeline::{RowOrder, RowOrderCommand, RowOrderMode, UnmatchedPolicy};

    let (host, mut vcx) = open_gpu_trace(cx);
    let panel = host
        .update(&mut vcx, |ws, window, cx| {
            let p0 = find_tracks_matching(ws, |path| {
                path.contains("sm[0].simt_partition[0].warp_slot[")
                    && path.ends_with(".Instruction")
            });
            let tracks: Vec<_> = p0.iter().map(|(_, t)| *t).collect();
            ws.dispatch(Command::OpenMergedPipeline { tracks }, Some(window), cx);
            ws.app.panels.focused_id()
        })
        .unwrap();
    pump_frames(&mut vcx, 3);

    let (stage_name, later_stage, total_rows) = host
        .update(&mut vcx, |ws, _, _| {
            let p = ws.app.panels.pipeline(panel).unwrap();
            assert_eq!(p.order, RowOrder::default());
            assert_eq!(ws.app.status().pipeline_order, None);
            let options = p.stage_order_options(&ws.app.doc);
            assert!(
                options.len() >= 3,
                "expected at least 3 primary-lane stages in GPU trace, got {options:?}"
            );
            for opt in &options {
                assert!((0.0..=1.0).contains(&opt.coverage));
                assert!(opt.total_transactions > 0);
            }
            (
                options[1].name.clone(),
                options[2].name.clone(),
                p.row_count(&ws.app.doc),
            )
        })
        .unwrap();

    // Order by a single primary-lane stage.
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::PipelineOrder(
                panel,
                RowOrderCommand::SetMode(RowOrderMode::Stages {
                    stages: vec![stage_name.clone()],
                }),
            ),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 2);

    let unmatched = host
        .update(&mut vcx, |ws, _, _| {
            let p = ws.app.panels.pipeline(panel).unwrap();
            assert_eq!(
                p.order.mode,
                RowOrderMode::Stages {
                    stages: vec![stage_name.clone()]
                }
            );
            let status_order = ws
                .app
                .status()
                .pipeline_order
                .expect("status bar should reflect active row order");
            assert!(
                status_order.contains(&stage_name),
                "expected status order '{status_order}' to mention '{stage_name}'"
            );
            let Rows::Ready(set) = p.rows(&ws.app.doc) else {
                panic!("rows should be Ready");
            };
            assert_eq!(set.len(), total_rows);
            let mut prev_key = 0u64;
            for r in 0..set.len().min(500) {
                let meta = set
                    .sort_meta(r)
                    .expect("non-Start order populates sort_meta");
                assert!(meta.sort_key >= prev_key);
                prev_key = meta.sort_key;
            }
            set.unmatched_count()
        })
        .unwrap();

    // Switch unmatched policy to Hide and verify row count shrinks by exact unmatched count.
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::PipelineOrder(panel, RowOrderCommand::SetUnmatched(UnmatchedPolicy::Hide)),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, _, _| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.order.unmatched, UnmatchedPolicy::Hide);
        assert_eq!(p.row_count(&ws.app.doc), total_rows - unmatched);
    })
    .unwrap();

    // Switch to FromStage and verify Undo/Redo round-trips cleanly.
    host.update(&mut vcx, |ws, window, cx| {
        ws.dispatch(
            Command::PipelineOrder(
                panel,
                RowOrderCommand::SetMode(RowOrderMode::FromStage {
                    stage: later_stage.clone(),
                }),
            ),
            Some(window),
            cx,
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, window, cx| {
        assert_eq!(
            ws.app.panels.pipeline(panel).unwrap().order.mode,
            RowOrderMode::FromStage {
                stage: later_stage.clone()
            }
        );
        ws.dispatch(Command::Undo, Some(window), cx);
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, window, cx| {
        assert_eq!(
            ws.app.panels.pipeline(panel).unwrap().order.mode,
            RowOrderMode::Stages {
                stages: vec![stage_name.clone()]
            }
        );
        ws.dispatch(Command::Redo, Some(window), cx);
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    // Open the Order popover via the real toolbar button, click "End (tx.end)" inside it,
    // and paint in both dark and light themes before splitting.
    vcx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    let order_btn = vcx
        .debug_bounds("pipeline-order-btn")
        .expect("pipeline-order-btn should be rendered in pipeline toolbar");
    vcx.simulate_click(order_btn.center(), gpui_kit::Modifiers::default());
    pump_frames(&mut vcx, 2);

    assert!(
        vcx.debug_bounds("pipeline-order-popover").is_some(),
        "Order popover should open after clicking pipeline-order-btn (btn={order_btn:?})"
    );
    let end_btn = vcx
        .debug_bounds("pipeline-order-end")
        .expect("Order popover should render End (tx.end) button");
    vcx.simulate_click(end_btn.center(), gpui_kit::Modifiers::default());
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, _, _| {
        assert_eq!(
            ws.app.panels.pipeline(panel).unwrap().order.mode,
            RowOrderMode::End
        );
    })
    .unwrap();

    // Split panel clones the active RowOrder onto the new panel.
    let split_panel = host
        .update(&mut vcx, |ws, window, cx| {
            ws.dispatch(
                Command::Panels(volna_core::panels::PanelsCommand::Split {
                    panel,
                    axis: volna_core::panels::Axis::Horizontal,
                }),
                Some(window),
                cx,
            );
            ws.app.panels.focused_id()
        })
        .unwrap();
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, _, cx| {
        assert_ne!(split_panel, panel);
        assert_eq!(
            ws.app.panels.pipeline(split_panel).unwrap().order,
            ws.app.panels.pipeline(panel).unwrap().order
        );
        crate::theme::set(crate::theme::CoreTheme::volna(false), cx);
        cx.notify();
    })
    .unwrap();
    pump_frames(&mut vcx, 1);
}

// User scenario: an engineer right-clicks directly on a stage cell in a merged pipeline panel
// to sort rows by that stage, and also right-drags across the canvas to pan without accidentally
// popping open the context menu.
// Requirement: unmoved right-click opens `WaveMenuKind::PipelineCell` with stage order and
// Solo/Isolate items; choosing "Order rows by <stage>" updates `RowOrder`; right-drag pans
// without opening a menu.
#[gpui_kit::test]
fn gpu_trace_right_click_stage_cell_opens_order_and_source_context_menu(cx: &mut TestAppContext) {
    use volna_core::pipeline::{RowOrderCommand, RowOrderMode};
    use volna_core::wave::model::{MenuAction, WaveMenuKind};

    let (host, mut vcx) = open_gpu_trace(cx);
    let panel = host
        .update(&mut vcx, |ws, window, cx| {
            let p0 = find_tracks_matching(ws, |path| {
                path.contains("sm[0].simt_partition[0].warp_slot[")
                    && path.ends_with(".Instruction")
            });
            let tracks: Vec<_> = p0.iter().take(4).map(|(_, t)| *t).collect();
            ws.dispatch(Command::OpenMergedPipeline { tracks }, Some(window), cx);
            ws.app.panels.focused_id()
        })
        .unwrap();
    pump_frames(&mut vcx, 3);

    // Find a visible primary-lane stage cell inside the cells viewport.
    let (cell_pt, stage_name) = host
        .update(&mut vcx, |ws, _, _| {
            let p = ws.app.panels.pipeline(panel).unwrap();
            let layout = p.last_layout();
            let vp = p.nav.viewport(&ws.app.doc);
            let Rows::Ready(set) = p.rows(&ws.app.doc) else {
                panic!("rows should be Ready");
            };
            let primary = p.palette().primary_lane().to_owned();
            for r in layout.row_range.clone() {
                let Some((_, tx)) = set.get(r) else { continue };
                let y = layout.row_y(r) + layout.rows.row_px * 0.5;
                if y < layout.cells.top() || y >= layout.cells.bottom() {
                    continue;
                }
                for st in &tx.stages {
                    if st.lane != primary {
                        continue;
                    }
                    let se = volna_core::pipeline::PipelineModel::stage_end(tx, st);
                    if se <= st.begin {
                        continue;
                    }
                    let mid_t = f64::midpoint(st.begin as f64, se as f64);
                    if mid_t <= vp.start || mid_t >= vp.end {
                        continue;
                    }
                    let x = layout.cells.left() + vp.x_of(mid_t, layout.cells_width_f64()) as f32;
                    let pt = volna_core::geometry::point(x, y);
                    if let Some(ctx) = p.cell_context_at(&ws.app.doc, pt)
                        && ctx.stage_name.as_deref() == Some(st.name.as_str())
                    {
                        return (pt, st.name.clone());
                    }
                }
            }
            panic!("expected at least one visible primary-lane stage cell in opening viewport");
        })
        .unwrap();

    // Right-drag (> 3px threshold) pans without opening the cell context menu.
    host.update(&mut vcx, |ws, window, cx| {
        let drag_end = volna_core::geometry::point(cell_pt.x + 24.0, cell_pt.y);
        for ev in [
            volna_core::wave::PointerEvent::Down {
                position: cell_pt,
                button: volna_core::geometry::MouseButton::Right,
                modifiers: volna_core::geometry::Modifiers::default(),
            },
            volna_core::wave::PointerEvent::Move { position: drag_end },
            volna_core::wave::PointerEvent::Up,
        ] {
            ws.dispatch(Command::Pointer(panel, ev), Some(window), cx);
        }
        assert!(
            ws.app.menu().is_none(),
            "right-drag must pan without opening context menu"
        );
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    // Re-locate a stage cell after panning and send an unmoved right-click.
    let (click_pt, clicked_stage) = host
        .update(&mut vcx, |ws, _, _| {
            let p = ws.app.panels.pipeline(panel).unwrap();
            let layout = p.last_layout();
            let vp = p.nav.viewport(&ws.app.doc);
            let Rows::Ready(set) = p.rows(&ws.app.doc) else {
                panic!("rows should be Ready");
            };
            let primary = p.palette().primary_lane().to_owned();
            for r in layout.row_range.clone() {
                let Some((_, tx)) = set.get(r) else { continue };
                let y = layout.row_y(r) + layout.rows.row_px * 0.5;
                if y < layout.cells.top() || y >= layout.cells.bottom() {
                    continue;
                }
                for st in &tx.stages {
                    if st.lane != primary {
                        continue;
                    }
                    let se = volna_core::pipeline::PipelineModel::stage_end(tx, st);
                    let mid_t = f64::midpoint(st.begin as f64, se as f64);
                    if se <= st.begin || mid_t <= vp.start || mid_t >= vp.end {
                        continue;
                    }
                    let x = layout.cells.left() + vp.x_of(mid_t, layout.cells_width_f64()) as f32;
                    let pt = volna_core::geometry::point(x, y);
                    if let Some(ctx) = p.cell_context_at(&ws.app.doc, pt)
                        && let Some(name) = ctx.stage_name
                    {
                        return (pt, name);
                    }
                }
            }
            (cell_pt, stage_name)
        })
        .unwrap();

    host.update(&mut vcx, |ws, window, cx| {
        for ev in [
            volna_core::wave::PointerEvent::Down {
                position: click_pt,
                button: volna_core::geometry::MouseButton::Right,
                modifiers: volna_core::geometry::Modifiers::default(),
            },
            volna_core::wave::PointerEvent::Up,
        ] {
            ws.dispatch(Command::Pointer(panel, ev), Some(window), cx);
        }
        let menu = ws.app.menu().expect("right-click on stage cell opens menu");
        assert_eq!(menu.kind, WaveMenuKind::PipelineCell);
        let target_action =
            MenuAction::PipelineOrder(RowOrderCommand::SetMode(RowOrderMode::Stages {
                stages: vec![clicked_stage.clone()],
            }));
        assert!(
            menu.items().any(|item| item.action == target_action),
            "context menu should offer ordering by clicked stage '{clicked_stage}'"
        );
        assert!(
            menu.items().any(|item| matches!(
                item.action,
                MenuAction::PipelineSource(PipelineSourceCommand::Isolate(_))
            )),
            "merged cell context menu should offer Isolate"
        );
        ws.dispatch(Command::MenuSelect(panel, target_action), Some(window), cx);
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, _, _| {
        assert!(ws.app.menu().is_none());
        assert_eq!(
            ws.app.panels.pipeline(panel).unwrap().order.mode,
            RowOrderMode::Stages {
                stages: vec![clicked_stage]
            }
        );
    })
    .unwrap();
}

// User scenario: an engineer opens the Stages & Order popover on a merged GPU pipeline panel,
// toggles visibility on two primary-lane stages via the left eye icon, sees the toolbar hidden
// badge and status bar notice update without affecting row order, splits the panel to verify
// hidden stages carry over, tests Undo/Redo, clicks "Show all" to restore visibility, and renders
// in both dark and light themes.
// Requirement: Stages & Order popover exposes only primary-lane stages with separate visibility
// toggles, toolbar displays a clear hidden-count badge whenever stages are hidden, "Show all"
// resets visibility, Undo/Redo and split inheritance preserve StageVisibility, and both themes
// paint cleanly.
#[gpui_kit::test]
fn gpu_trace_stages_and_order_visibility_toolbar_badge_show_all_undo_split_and_themes(
    cx: &mut TestAppContext,
) {
    use volna_core::pipeline::RowOrder;

    let (host, mut vcx) = open_gpu_trace(cx);
    let panel = host
        .update(&mut vcx, |ws, window, cx| {
            let p0 = find_tracks_matching(ws, |path| {
                path.contains("sm[0].simt_partition[0].warp_slot[")
                    && path.ends_with(".Instruction")
            });
            let tracks: Vec<_> = p0.iter().take(4).map(|(_, t)| *t).collect();
            ws.dispatch(Command::OpenMergedPipeline { tracks }, Some(window), cx);
            ws.app.panels.focused_id()
        })
        .unwrap();
    pump_frames(&mut vcx, 3);

    let (first_stage, second_stage, total_rows) = host
        .update(&mut vcx, |ws, _, _| {
            let p = ws.app.panels.pipeline(panel).unwrap();
            assert!(p.stage_visibility.is_empty());
            assert_eq!(ws.app.status().pipeline_order, None);
            let options = p.stage_order_options(&ws.app.doc);
            assert!(
                options.len() >= 2,
                "expected at least 2 primary-lane stages in GPU trace"
            );
            (
                options[0].name.clone(),
                options[1].name.clone(),
                p.row_count(&ws.app.doc),
            )
        })
        .unwrap();

    // Initially no hidden badge appears in the toolbar.
    vcx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    assert!(
        vcx.debug_bounds("pipeline-stages-hidden-badge").is_none(),
        "toolbar hidden-count badge must not appear when all stages are visible"
    );

    // Open the Stages & Order popover via the real toolbar button.
    let order_btn = vcx
        .debug_bounds("pipeline-order-btn")
        .expect("pipeline-order-btn should be rendered in pipeline toolbar");
    vcx.simulate_click(order_btn.center(), gpui_kit::Modifiers::default());
    pump_frames(&mut vcx, 2);
    assert!(
        vcx.debug_bounds("pipeline-order-popover").is_some(),
        "Stages & Order popover should open"
    );
    assert!(
        vcx.debug_bounds("pipeline-stages-show-all").is_none(),
        "Show all button should stay hidden when no stages are hidden"
    );

    // Click the eye toggle for the first two primary-lane stages.
    let vis_first = vcx
        .debug_bounds("pipeline-stage-vis-0")
        .expect("eye toggle should exist for first primary-lane stage");
    vcx.simulate_click(vis_first.center(), gpui_kit::Modifiers::default());
    pump_frames(&mut vcx, 2);

    let vis_second = vcx
        .debug_bounds("pipeline-stage-vis-1")
        .expect("eye toggle should exist for second primary-lane stage");
    vcx.simulate_click(vis_second.center(), gpui_kit::Modifiers::default());
    pump_frames(&mut vcx, 2);

    // Verify model state, unchanged row count & order, toolbar badge, status bar, and Show all.
    host.update(&mut vcx, |ws, _, _| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.stage_visibility.hidden_count(), 2);
        assert!(p.stage_visibility.is_hidden(&first_stage));
        assert!(p.stage_visibility.is_hidden(&second_stage));
        assert_eq!(
            p.order,
            RowOrder::default(),
            "toggling stage visibility must not alter row order"
        );
        assert_eq!(
            p.row_count(&ws.app.doc),
            total_rows,
            "hiding stages must keep all transaction rows in place"
        );
        let status = ws
            .app
            .status()
            .pipeline_order
            .expect("status bar should indicate hidden stages");
        assert!(
            status.contains('2') && status.contains("hidden"),
            "expected status '{status}' to mention 2 hidden stages"
        );
    })
    .unwrap();
    assert!(
        vcx.debug_bounds("pipeline-stages-hidden-badge").is_some(),
        "toolbar must display hidden-count badge when stages are hidden"
    );
    assert!(
        vcx.debug_bounds("pipeline-stages-show-all").is_some(),
        "popover must display Show all action when stages are hidden"
    );

    // Split panel inherits hidden stages onto the cloned panel.
    let split_panel = host
        .update(&mut vcx, |ws, window, cx| {
            ws.dispatch(
                Command::Panels(volna_core::panels::PanelsCommand::Split {
                    panel,
                    axis: volna_core::panels::Axis::Horizontal,
                }),
                Some(window),
                cx,
            );
            ws.app.panels.focused_id()
        })
        .unwrap();
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, _, _| {
        assert_ne!(split_panel, panel);
        assert_eq!(
            ws.app
                .panels
                .pipeline(split_panel)
                .unwrap()
                .stage_visibility,
            ws.app.panels.pipeline(panel).unwrap().stage_visibility
        );
    })
    .unwrap();

    // Paint in light theme with hidden stages and popover open, then switch back to dark.
    host.update(&mut vcx, |_, _, cx| {
        crate::theme::set(crate::theme::CoreTheme::volna(false), cx);
        cx.notify();
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    // Click "Show all" in the popover to restore visibility of all stages on the original panel.
    let show_all_btn = vcx
        .debug_bounds("pipeline-stages-show-all")
        .expect("Show all button should still be visible in open popover");
    vcx.simulate_click(show_all_btn.center(), gpui_kit::Modifiers::default());
    pump_frames(&mut vcx, 2);

    host.update(&mut vcx, |ws, window, cx| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert!(
            p.stage_visibility.is_empty(),
            "Show all should clear all hidden stages"
        );
        // Undo restores the 2 hidden stages; Redo clears them again.
        ws.dispatch(Command::Undo, Some(window), cx);
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, window, cx| {
        let p = ws.app.panels.pipeline(panel).unwrap();
        assert_eq!(p.stage_visibility.hidden_count(), 2);
        assert!(p.stage_visibility.is_hidden(&first_stage));
        assert!(p.stage_visibility.is_hidden(&second_stage));
        ws.dispatch(Command::Redo, Some(window), cx);
    })
    .unwrap();
    pump_frames(&mut vcx, 1);

    host.update(&mut vcx, |ws, _, _| {
        assert!(
            ws.app
                .panels
                .pipeline(panel)
                .unwrap()
                .stage_visibility
                .is_empty()
        );
    })
    .unwrap();
}
