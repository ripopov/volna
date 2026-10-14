//! Local time-to-row navigation over the resident interval index.

use super::{PipelineModel, Rows};
use crate::document::Document;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowActivity {
    Off,
    #[default]
    Following,
    Suspended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityCommand {
    Toggle,
    RevealAbove,
    RevealBelow,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Activity {
    pub above: usize,
    pub below: usize,
    pub visible: usize,
    pub nearest_above: Option<usize>,
    pub nearest_below: Option<usize>,
    pub target: Option<usize>,
    pub earlier: bool,
    pub later: bool,
}

impl PipelineModel {
    #[must_use]
    pub fn activity(&self, doc: &Document) -> Activity {
        let Rows::Ready(set) = self.rows(doc) else {
            return Activity::default();
        };
        let view = self.nav.viewport(doc);
        let anchor = self
            .nav
            .cursor(doc)
            .map(|t| t as f64)
            .filter(|&t| t >= view.start && t <= view.end)
            .unwrap_or(view.start + (view.end - view.start) * 0.5);
        let rows = self
            .rows
            .target()
            .zoomed(self.last_layout().zoom.max(f32::EPSILON));
        let visible = f64::from(self.last_layout().cells.height() / rows.row_px);
        set.activity(view, rows.top..rows.top + visible, anchor)
    }

    pub fn activity_command(&mut self, doc: &Document, command: ActivityCommand) {
        match command {
            ActivityCommand::Toggle => {
                self.follow = match self.follow {
                    FollowActivity::Following => FollowActivity::Off,
                    _ => FollowActivity::Following,
                };
                _ = self.follow_activity(doc);
            }
            ActivityCommand::RevealAbove | ActivityCommand::RevealBelow => {
                let activity = self.activity(doc);
                let row = match command {
                    ActivityCommand::RevealAbove => activity.nearest_above,
                    _ => activity.nearest_below,
                };
                if let Some(row) = row {
                    self.suspend_follow();
                    self.reveal_activity_row(doc, row);
                }
            }
        }
    }

    pub(super) fn suspend_follow(&mut self) {
        if self.follow == FollowActivity::Following {
            self.follow = FollowActivity::Suspended;
        }
    }

    pub(super) fn follow_activity(&mut self, doc: &Document) -> Option<Activity> {
        if self.follow != FollowActivity::Following || self.last_layout().cells.height() <= 0.0 {
            return None;
        }
        let activity = self.activity(doc);
        if let Some(row) = activity.target {
            let rows = self.rows.target().zoomed(self.last_layout().zoom);
            let visible = f64::from(self.last_layout().cells.height() / rows.row_px);
            let position = row as f64 + 0.5;
            if position < rows.top + visible * 0.2 || position > rows.top + visible * 0.8 {
                self.reveal_activity_row(doc, row);
                return None;
            }
        }
        Some(activity)
    }

    fn reveal_activity_row(&mut self, doc: &Document, row: usize) {
        let zoom = self.last_layout().zoom;
        let mut rows = self.rows.target().zoomed(zoom);
        rows.top =
            row as f64 + 0.5 - f64::from(self.last_layout().cells.height() / rows.row_px) * 0.5;
        rows.clamp(self.last_layout().cells.height(), self.row_count(doc), zoom);
        self.rows.set(rows.unzoomed(zoom));
    }
}
