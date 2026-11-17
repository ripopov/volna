//! Shared metadata dividers for linked timelines in the same dock column.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{Axis, DockLayout, Layout, PanelId, PanelKind, Panels};
use crate::geometry::{Point, Rect};
use crate::pipeline::model::Drag as PipelineDrag;
use crate::wave::{layout::MIN_COLUMN, model::Drag as WaveDrag};

const MIN_TIMELINE_WIDTH: f32 = 160.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct TimelineMargins {
    pub metadata_width: Option<f32>,
    pub trailing_width: f32,
}

impl TimelineMargins {
    pub(crate) fn auto_metadata_limit(self, bounds: Rect) -> f32 {
        (bounds.width() * 0.5 - self.trailing_width).max(0.0)
    }

    pub(crate) fn content(self, bounds: Rect) -> Rect {
        Rect::from_xywh(
            bounds.left(),
            bounds.top(),
            (bounds.width() - self.trailing_width).max(0.0),
            bounds.height(),
        )
    }

    pub(crate) fn gutter(self, bounds: Rect) -> Rect {
        let content = self.content(bounds);
        Rect::from_xywh(
            content.right(),
            bounds.top(),
            bounds.width() - content.width(),
            bounds.height(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineDivider {
    pub panels: Vec<PanelId>,
    pub width: f32,
    pub(crate) manual: bool,
    // Rendering clamps the derived width; this anchor preserves reversible scope toggles.
    pub(crate) scope_offset: f64,
}

fn columns(layout: &Layout, out: &mut Vec<Vec<Vec<PanelId>>>) -> Vec<Vec<PanelId>> {
    match layout {
        Layout::Tabs { tabs, .. } => vec![tabs.clone()],
        Layout::Split {
            split: Axis::Vertical,
            children,
            ..
        } => children
            .iter()
            .flat_map(|child| columns(child, out))
            .collect(),
        Layout::Split { children, .. } => {
            for child in children {
                let column = columns(child, out);
                out.push(column);
            }
            Vec::new()
        }
    }
}

impl Panels {
    pub(crate) fn timeline_dividers(&self, layout: &DockLayout) -> Vec<TimelineDivider> {
        let mut all = Vec::new();
        for tree in layout.trees() {
            let root = columns(tree, &mut all);
            all.push(root);
        }
        all.into_iter()
            .filter_map(|column| {
                let groups: Vec<Vec<_>> = column
                    .into_iter()
                    .map(|tabs| {
                        tabs.into_iter()
                            .filter(|id| {
                                self.get(*id).is_some_and(|p| {
                                    matches!(p.kind, PanelKind::Waves(_) | PanelKind::Pipeline(_))
                                        && p.kind.nav().is_some_and(|nav| nav.link.viewport)
                                })
                            })
                            .collect()
                    })
                    .filter(|tabs: &Vec<_>| !tabs.is_empty())
                    .collect();
                if groups.len() < 2 {
                    return None;
                }
                let mut panels: Vec<_> = groups.into_iter().flatten().collect();
                panels.sort_unstable();
                if let Some(old) = self.timeline.iter().find(|old| old.panels == panels) {
                    return Some(old.clone());
                }
                let mut width: f32 = 0.0;
                let mut manual = false;
                for id in &panels {
                    if let Some(old) = self.timeline.iter().find(|old| old.panels.contains(id)) {
                        manual |= old.manual;
                        width = width.max(self.requested_timeline_width(old));
                    } else {
                        width = width.max(match &self.get(*id)?.kind {
                            PanelKind::Waves(w) => {
                                manual |= w.manual_columns;
                                w.names_width.max(MIN_COLUMN)
                                    + w.scope.width()
                                    + w.values_width.max(MIN_COLUMN)
                            }
                            PanelKind::Pipeline(p) => p.label_width.clamp(
                                crate::pipeline::layout::LABEL_W_MIN,
                                crate::pipeline::layout::LABEL_W_MAX,
                            ),
                            _ => unreachable!(),
                        });
                    }
                }
                let scope_offset = self.scope_offset(&panels);
                Some(TimelineDivider {
                    panels,
                    width: width.max(1.0),
                    manual,
                    scope_offset,
                })
            })
            .collect()
    }

    pub(crate) fn sync_timeline_dividers(&mut self) {
        if self.timeline_revision != Some(self.revision) {
            self.timeline = self.timeline_dividers(&self.layout);
            self.timeline_revision = Some(self.revision);
        }
    }

    fn navigator_width(&self, id: PanelId) -> f32 {
        self.pipeline(id).map_or(0.0, |p| p.navigator.width())
    }

    pub(crate) fn timeline_margins(&self, id: PanelId, bounds: Rect, zoom: f32) -> TimelineMargins {
        let divider = self.timeline.iter().find(|d| d.panels.contains(&id));
        let width = divider.map_or_else(
            || self.navigator_width(id),
            |d| {
                d.panels
                    .iter()
                    .map(|&id| self.navigator_width(id))
                    .fold(0.0, f32::max)
            },
        );
        let trailing_width = (width * zoom).min(bounds.width() * 0.25);
        let mut margins = TimelineMargins {
            trailing_width,
            metadata_width: None,
        };
        margins.metadata_width = divider.map(|d| {
            let width = self.requested_timeline_width(d);
            if d.manual {
                self.clamp_timeline_width(d, width, margins.content(bounds), zoom) * zoom
            } else {
                (width.max(self.timeline_minimum(d)) * zoom)
                    .min(margins.auto_metadata_limit(bounds))
            }
        });
        margins
    }

    fn clamp_timeline_width(
        &self,
        divider: &TimelineDivider,
        width: f32,
        bounds: Rect,
        zoom: f32,
    ) -> f32 {
        let available = bounds.width() / zoom;
        // Leave room to drag in either direction from the automatic split, even with a navigator.
        let compact_min = available * 0.25;
        let min = self.timeline_minimum(divider).min(compact_min);
        let max = available - MIN_TIMELINE_WIDTH.min(compact_min);
        width.clamp(min, max)
    }

    fn scope_offset(&self, panels: &[PanelId]) -> f64 {
        panels
            .iter()
            .filter_map(|&id| self.waves(id))
            .map(|waves| waves.scope.timeline_offset())
            .sum()
    }

    fn requested_timeline_width(&self, divider: &TimelineDivider) -> f32 {
        (f64::from(divider.width) + self.scope_offset(&divider.panels) - divider.scope_offset)
            .clamp(0.0, f64::from(f32::MAX)) as f32
    }

    fn timeline_minimum(&self, divider: &TimelineDivider) -> f32 {
        divider
            .panels
            .iter()
            .filter_map(|&id| self.get(id))
            .map(|panel| match &panel.kind {
                PanelKind::Waves(waves) => 2.0 * MIN_COLUMN + waves.scope.width().min(MIN_COLUMN),
                _ => crate::pipeline::layout::LABEL_W_MIN,
            })
            .fold(0.0, f32::max)
    }

    pub(crate) fn resize_timeline(&mut self, id: PanelId, position: Point) -> bool {
        let Some(index) = self.timeline.iter().position(|d| d.panels.contains(&id)) else {
            return false;
        };
        let Some(panel) = self.get(id) else {
            return false;
        };
        let (bounds, zoom, grab) = match &panel.kind {
            PanelKind::Waves(w) if let Some(WaveDrag::ValuesSplit { grab }) = w.drag => {
                (w.last_layout().bounds, w.last_layout().zoom, grab)
            }
            PanelKind::Pipeline(p) if let Some(PipelineDrag::LabelSplit { grab }) = p.drag => {
                (p.last_layout().bounds, p.last_layout().zoom, grab)
            }
            _ => return false,
        };
        let bounds = self.timeline_margins(id, bounds, zoom).content(bounds);
        if bounds.width() <= 0.0 {
            return true;
        }
        let width = (position.x - grab - bounds.left()) / zoom;
        let scope_offset = self.scope_offset(&self.timeline[index].panels);
        self.timeline[index].manual = true;
        self.timeline[index].scope_offset = scope_offset;
        self.timeline[index].width =
            self.clamp_timeline_width(&self.timeline[index], width, bounds, zoom);
        match &mut self.get_mut(id).unwrap().kind {
            PanelKind::Waves(w) => w.pointer = Some(position),
            PanelKind::Pipeline(p) => p.pointer = Some(position),
            _ => unreachable!(),
        }
        true
    }

    pub(crate) fn restore_timeline_dividers(
        &mut self,
        dividers: Vec<TimelineDivider>,
    ) -> Result<()> {
        let expected = self.timeline_dividers(&self.layout);
        ensure!(
            dividers.len() == expected.len(),
            "invalid timeline divider groups"
        );
        let mut seen = std::collections::BTreeSet::new();
        for divider in &dividers {
            ensure!(
                divider.width.is_finite()
                    && divider.width > 0.0
                    && divider.scope_offset.is_finite(),
                "invalid timeline divider width"
            );
            ensure!(
                expected.iter().any(|d| d.panels == divider.panels),
                "invalid timeline divider membership"
            );
            ensure!(
                divider.panels.iter().all(|id| seen.insert(*id)),
                "duplicate timeline divider panel"
            );
        }
        self.timeline = dividers;
        Ok(())
    }
}
