//! Pipeline opening resolves on data delivery or a host geometry report.

use super::{Action, App, Command, PanelsCommand, PointerEvent, TransactionCommand};
use crate::marker::LaneVerb;
use crate::panels::PanelId;
use crate::table::TableCommand;

impl App {
    #[must_use]
    pub fn panel_measurement(&self, panel: PanelId) -> Option<Command> {
        let size = self.panels.pipeline(panel)?.opening_measurement()?;
        Some(Command::PipelineMeasured(panel, size))
    }

    pub(super) fn initialize_pipelines(&mut self) {
        let mut changed = false;
        for pipeline in self.panels.pipelines_mut() {
            changed |= pipeline.initialize_view(&mut self.doc);
        }
        if changed {
            self.workspace.scheduler.changed(super::Instant::now());
            self.changed();
        }
    }

    pub(super) fn cancel_pipeline_opening(&mut self, command: &Command) {
        let target = match command {
            Command::Action(
                Action::ZoomIn
                | Action::ZoomOut
                | Action::ZoomFit
                | Action::ZoomToCursor
                | Action::PanPageLeft
                | Action::PanPageRight
                | Action::GoToStart
                | Action::GoToEnd
                | Action::GoToCursor
                | Action::PanLeft
                | Action::PanRight
                | Action::NextEdge
                | Action::PrevEdge
                | Action::NextMarker
                | Action::PrevMarker
                | Action::GoToMarker(_)
                | Action::JumpBack
                | Action::ZoomToMeasurement
                | Action::NextCycle
                | Action::PrevCycle
                | Action::IncreaseRowHeight
                | Action::DecreaseRowHeight
                | Action::ResetRowHeight
                | Action::MoveSelectionUp
                | Action::MoveSelectionDown
                | Action::ToggleViewportLink,
            )
            | Command::Lane(
                LaneVerb::GoTo(_) | LaneVerb::ZoomToSpan(..) | LaneVerb::ZoomToMeasurement,
            )
            | Command::Clocks(super::ClockCommand::GoToCycle(_)) => self.panels.focused_id(),
            Command::Pointer(
                panel,
                PointerEvent::Down { .. } | PointerEvent::Wheel { .. } | PointerEvent::Pinch { .. },
            )
            | Command::PipelineActivity(panel, _)
            | Command::PanToTransactionStart { panel, .. }
            | Command::SelectTransaction { panel, .. }
            | Command::ZoomToSpan(panel, ..)
            | Command::RevealTransaction { panel }
            | Command::Transaction(
                panel,
                TransactionCommand::FitLifetime | TransactionCommand::Cursor(_),
            )
            | Command::Panels(PanelsCommand::ToggleLink { panel, .. })
            | Command::Table(
                panel,
                TableCommand::Select(_)
                | TableCommand::GoTo(_)
                | TableCommand::First
                | TableCommand::Last
                | TableCommand::Previous
                | TableCommand::Next
                | TableCommand::Page(_),
            )
            | Command::MenuSelect(
                panel,
                super::MenuAction::Lane(
                    LaneVerb::GoTo(_) | LaneVerb::ZoomToSpan(..) | LaneVerb::ZoomToMeasurement,
                ),
            ) => *panel,
            _ => return,
        };
        let shared = matches!(
            command,
            Command::Transaction(
                _,
                TransactionCommand::FitLifetime | TransactionCommand::Cursor(_)
            )
        ) || self
            .panels
            .get(target)
            .and_then(|panel| panel.kind.nav())
            .is_some_and(|nav| nav.link.viewport);
        for panel in self.panels.iter_mut() {
            if let Some(pipeline) = panel.kind.pipeline_mut()
                && (panel.id == target || shared && pipeline.nav.link.viewport)
            {
                pipeline.cancel_opening();
            }
        }
    }
}
