//! Pipeline occupancy presentation and navigation over shared concurrency summaries.

use super::{PipelineModel, Rows};
use crate::{
    analysis::AnalysisLoad,
    document::Document,
    geometry::{CursorIcon, Rect, point},
    scene::{FontRole, Scene},
    theme::Theme,
};

pub const MIN_HEIGHT: f32 = 40.0;
pub const MAX_HEIGHT: f32 = 320.0;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OccupancySettings {
    pub visible: bool,
    pub height: f32,
}

impl Default for OccupancySettings {
    fn default() -> Self {
        Self {
            visible: true,
            height: 96.0,
        }
    }
}

impl OccupancySettings {
    pub(crate) fn valid(self) -> bool {
        self.height.is_finite() && (MIN_HEIGHT..=MAX_HEIGHT).contains(&self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccupancyCommand {
    Toggle,
    Taller,
    Shorter,
}

impl PipelineModel {
    pub fn occupancy_command(&mut self, command: OccupancyCommand) {
        match command {
            OccupancyCommand::Toggle => self.occupancy.visible = !self.occupancy.visible,
            OccupancyCommand::Taller | OccupancyCommand::Shorter => {
                self.occupancy.visible = true;
                let delta = if command == OccupancyCommand::Taller {
                    16.0
                } else {
                    -16.0
                };
                self.occupancy.height =
                    (self.occupancy.height + delta).clamp(MIN_HEIGHT, MAX_HEIGHT);
            }
        }
    }

    pub(super) fn occupancy_at(&mut self, doc: &mut Document, x: f32) {
        let area = self.last_layout().occupancy;
        let time = self
            .nav
            .viewport(doc)
            .time_at(f64::from(x - area.left()), f64::from(area.width()).max(1.0));
        let (a, b) = doc.limits();
        self.nav
            .set_cursor(doc, Some((time.max(0.0) as u64).clamp(a, b)));
    }

    pub(super) fn occupancy_hover(&self, doc: &Document) -> Option<String> {
        let Rows::Ready(rows) = self.rows(doc) else {
            return None;
        };
        let trace = self.primary_trace()?;
        let load = doc.occupancy_by_ptrs(trace, rows.generator_ptrs())?;
        let AnalysisLoad::Ready(summary) = load else {
            return None;
        };
        let area = self.last_layout().occupancy;
        let viewport = self.nav.viewport(doc);
        let pointer = self.pointer?;
        let columns = f64::from(area.width().ceil().clamp(1.0, 4096.0));
        let bin = (f64::from(pointer.x - area.left()) / f64::from(area.width()) * columns).floor();
        let start = viewport.start + bin * (viewport.width() / columns);
        let end = start + viewport.width() / columns;
        let sample = summary.sample(start, end);
        Some(format!(
            "{}–{} · occupancy avg {:.2}, peak {}{}",
            crate::wave::timeline::format_time(start, doc.time_base()),
            crate::wave::timeline::format_time(end, doc.time_base()),
            sample.average,
            sample.peak,
            if sample.failed { " · errors" } else { "" }
        ))
    }
}

pub(super) fn paint(
    model: &PipelineModel,
    doc: &Document,
    scene: &mut Scene,
    theme: &Theme,
    focused: bool,
) {
    let layout = model.last_layout();
    let area = layout.occupancy;
    let split = layout.occupancy_split;
    scene.fill(split, theme.panel.bg);
    scene.fill(
        Rect::from_xywh(split.left(), split.top(), split.width(), theme.zoom),
        theme.border_variant,
    );
    scene.cursors.push((split, CursorIcon::ResizeUpDown));
    if matches!(model.drag, Some(super::model::Drag::OccupancySplit { .. })) {
        scene.window_cursor = Some(CursorIcon::ResizeUpDown);
    }
    if area.height() <= 0.0 || area.width() <= 0.0 {
        return;
    }
    let viewport = model.nav.viewport(doc);
    let highlight = model.highlight_source();
    let visible_sources = (0..model.sources.len())
        .filter(|&i| model.is_source_effectively_visible(i))
        .count();
    scene.clipped(area, |scene| {
        let load = match (model.primary_trace(), model.rows(doc)) {
            (Some(trace), Rows::Ready(rows)) if !rows.generator_ptrs().is_empty() => {
                doc.occupancy_by_ptrs(trace, rows.generator_ptrs())
            }
            _ => None,
        };
        if let Some(AnalysisLoad::Ready(summary)) = load {
            crate::occupancy_plot::paint(scene, summary, &viewport, area, theme);
            if let Some(src_idx) = highlight
                && visible_sources > 1
                && let Some((src_trace, src_gens)) = model.source_generators_ref(doc, src_idx)
                && let Some(AnalysisLoad::Ready(overlay)) = doc.occupancy(src_trace, src_gens)
                && let Some(source) = model.sources.get(src_idx)
            {
                crate::occupancy_plot::paint_overlay(
                    scene,
                    overlay,
                    summary.maximum(),
                    &viewport,
                    area,
                    theme.source_accent(source.color_slot),
                    theme,
                );
            }
        } else {
            scene.fill(area, theme.panel.bg);
            scene.text(
                point(area.left() + 6.0 * theme.zoom, area.top()),
                24.0 * theme.zoom,
                if matches!(load, Some(AnalysisLoad::Failed)) {
                    "Occupancy unavailable"
                } else if visible_sources == 0 {
                    "No visible sources"
                } else {
                    "Preparing occupancy…"
                },
                FontRole::Ui,
                theme.ui_size_small,
                theme.panel.text_muted,
            );
        }
        if let Some(cursor) = model.nav.cursor(doc) {
            let x = area.left() + viewport.x_of(cursor as f64, f64::from(area.width())) as f32;
            scene.fill(
                Rect::from_xywh(crate::geometry::snap(x), area.top(), 1.0, area.height()),
                if focused {
                    theme.wave_cursor
                } else {
                    theme.wave_cursor_inactive
                },
            );
        }
    });
    let labels = layout.occupancy_labels;
    scene.fill(labels, theme.panel.bg);
    scene.clipped(labels, |scene| {
        scene.text(
            point(labels.left() + 6.0 * theme.zoom, labels.top()),
            24.0 * theme.zoom,
            "Occupancy",
            FontRole::UiSemibold,
            theme.ui_size_small,
            theme.panel.text_muted,
        );
        if model.is_merged() && labels.height() >= 40.0 * theme.zoom {
            let subtitle = if let Some(src_idx) = highlight
                && let Some(lbl) = model.badges().labels.get(src_idx)
            {
                format!("Total + {} overlay", lbl.badge)
            } else if model.focus_style == super::FocusStyle::Isolate
                && let Some(src_idx) = model.focused_source
                && let Some(lbl) = model.badges().labels.get(src_idx)
            {
                format!("Isolated {}", lbl.badge)
            } else {
                format!("{visible_sources} sources")
            };
            scene.text(
                point(
                    labels.left() + 6.0 * theme.zoom,
                    labels.top() + 20.0 * theme.zoom,
                ),
                18.0 * theme.zoom,
                subtitle,
                FontRole::Mono,
                theme.ui_size_small * 0.9,
                theme.panel.text_placeholder,
            );
        }
    });
    scene.cursors.push((area, CursorIcon::PointingHand));
}
