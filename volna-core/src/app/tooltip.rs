//! Hover intent for marker and measurement details in timeline panels.

use std::time::Duration;

use super::{App, Command, Instant, LaneHit, PanelId, PanelKind, Point, PointerEvent};
use crate::wave::overlay::MarkerLane;

#[derive(Default)]
pub(super) struct MarkerTooltip {
    hover: Option<Hover>,
}

struct Hover {
    panel: PanelId,
    position: Point,
    target: Option<LaneHit>,
    deadline: Option<Instant>,
    observed_at: Instant,
}

impl Hover {
    fn retarget(&mut self, target: Option<LaneHit>, delay: Duration) -> bool {
        if self.target == target {
            return false;
        }
        self.deadline = target
            .as_ref()
            .filter(|_| !delay.is_zero())
            .map(|_| self.observed_at + delay);
        self.target = target;
        true
    }
}

impl MarkerTooltip {
    fn panel(&self) -> Option<PanelId> {
        self.hover.as_ref().map(|hover| hover.panel)
    }

    fn clear(&mut self) -> bool {
        self.hover
            .take()
            .is_some_and(|hover| hover.target.is_some())
    }

    fn update(
        &mut self,
        panel: PanelId,
        position: Point,
        target: Option<LaneHit>,
        now: Instant,
        delay: Duration,
    ) -> bool {
        let changed = self.panel() != Some(panel) && self.clear();
        let hover = self.hover.get_or_insert(Hover {
            panel,
            position,
            target: None,
            deadline: None,
            observed_at: now,
        });
        hover.position = position;
        hover.observed_at = now;
        hover.retarget(target, delay) || changed
    }

    pub(super) fn relayout(&mut self, panel: PanelId, lane: &MarkerLane, delay: Duration) {
        if let Some(hover) = &mut self.hover
            && hover.panel == panel
        {
            hover.retarget(lane.hit(hover.position), delay);
        }
    }

    pub(super) fn visible(&self, panel: PanelId) -> bool {
        self.hover.as_ref().is_some_and(|hover| {
            hover.panel == panel && hover.target.is_some() && hover.deadline.is_none()
        })
    }

    pub(super) fn tick(&mut self, now: Instant) -> bool {
        let Some(hover) = &mut self.hover else {
            return false;
        };
        hover.observed_at = now;
        if hover.deadline.is_some_and(|deadline| now >= deadline) {
            hover.deadline = None;
            return true;
        }
        false
    }
}

impl App {
    /// Hosts can stay idle until this deadline, then call `tick`; input and layout can reschedule it.
    #[must_use]
    pub fn tooltip_deadline(&self) -> Option<Instant> {
        self.marker_tooltip
            .hover
            .as_ref()
            .and_then(|hover| hover.deadline)
    }

    pub(super) fn update_marker_tooltip(&mut self, command: &Command, now: Instant) {
        let delay = self.settings.resolved().appearance.tooltip_delay;
        let changed = match command {
            Command::Pointer(id, PointerEvent::Move { position }) => {
                let lane = self
                    .panels
                    .get(*id)
                    .filter(|panel| !panel.dragging())
                    .and_then(|panel| {
                        let bounds = match &panel.kind {
                            PanelKind::Waves(waves) => waves.last_layout().bounds,
                            PanelKind::Pipeline(pipeline) => pipeline.last_layout().bounds,
                            _ => return None,
                        };
                        bounds
                            .contains(*position)
                            .then(|| panel.kind.marker_lane())
                            .flatten()
                    });
                match lane {
                    Some(lane) => {
                        self.marker_tooltip
                            .update(*id, *position, lane.hit(*position), now, delay)
                    }
                    None if self.marker_tooltip.panel() == Some(*id) => self.marker_tooltip.clear(),
                    None => false,
                }
            }
            Command::Pointer(id, PointerEvent::Leave)
                if self.marker_tooltip.panel() != Some(*id) =>
            {
                false
            }
            _ => self.marker_tooltip.clear(),
        };
        if changed {
            self.changed();
        }
    }
}
