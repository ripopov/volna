//! Shared metadata dividers for linked timelines in the same dock column.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{Axis, DockLayout, Layout, PanelId, PanelKind, Panels};
use crate::geometry::{Point, Rect};
use crate::pipeline::model::Drag as PipelineDrag;
use crate::wave::{layout::MIN_COLUMN, model::Drag as WaveDrag};

const MIN_TIMELINE_WIDTH: f32 = 160.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineDivider {
    pub panels: Vec<PanelId>,
    pub width: f32,
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
                let mut width: f32 = 0.0;
                for id in &panels {
                    if let Some(old) = self.timeline.iter().find(|old| old.panels.contains(id)) {
                        width = width.max(old.width);
                    } else {
                        width = width.max(match &self.get(*id)?.kind {
                            PanelKind::Waves(w) => {
                                w.names_width.max(MIN_COLUMN) + w.values_width.max(MIN_COLUMN)
                            }
                            PanelKind::Pipeline(p) => p.label_width.clamp(
                                crate::pipeline::layout::LABEL_W_MIN,
                                crate::pipeline::layout::LABEL_W_MAX,
                            ),
                            _ => unreachable!(),
                        });
                    }
                }
                Some(TimelineDivider { panels, width })
            })
            .collect()
    }

    pub(crate) fn sync_timeline_dividers(&mut self) {
        if self.timeline_revision != Some(self.revision) {
            self.timeline = self.timeline_dividers(&self.layout);
            self.timeline_revision = Some(self.revision);
        }
    }

    pub(crate) fn timeline_metadata(&self, id: PanelId, bounds: Rect, zoom: f32) -> Option<f32> {
        let divider = self.timeline.iter().find(|d| d.panels.contains(&id))?;
        Some(self.clamp_timeline_width(divider, divider.width, bounds, zoom) * zoom)
    }

    fn clamp_timeline_width(
        &self,
        divider: &TimelineDivider,
        width: f32,
        bounds: Rect,
        zoom: f32,
    ) -> f32 {
        let min = self.timeline_minimum(divider);
        let max = (bounds.width() / zoom - MIN_TIMELINE_WIDTH).max(0.0);
        width.clamp(min.min(max), max)
    }

    fn timeline_minimum(&self, divider: &TimelineDivider) -> f32 {
        if divider.panels.iter().any(|id| self.waves(*id).is_some()) {
            2.0 * MIN_COLUMN
        } else {
            crate::pipeline::layout::LABEL_W_MIN
        }
    }

    pub(crate) fn resize_timeline(&mut self, id: PanelId, position: Point) -> bool {
        let Some(index) = self.timeline.iter().position(|d| d.panels.contains(&id)) else {
            return false;
        };
        let Some(panel) = self.get(id) else {
            return false;
        };
        let (bounds, zoom) = match &panel.kind {
            PanelKind::Waves(w) if w.drag == Some(WaveDrag::ValuesSplit) => {
                (w.last_layout().bounds, w.last_layout().zoom)
            }
            PanelKind::Pipeline(p) if p.drag == Some(PipelineDrag::LabelSplit) => {
                (p.last_layout().bounds, p.last_layout().zoom)
            }
            _ => return false,
        };
        let width = (position.x - bounds.left()) / zoom;
        let min = self.timeline_minimum(&self.timeline[index]);
        self.timeline[index].width = self
            .clamp_timeline_width(&self.timeline[index], width, bounds, zoom)
            .max(min);
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
                divider.width.is_finite() && divider.width > 0.0,
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
