//! Scope-column mode and width invariants shared by input, layout and persistence.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Saved", into = "Saved")]
pub struct ScopeColumn {
    collapsed: bool,
    expanded_width: f32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    collapsed: bool,
    expanded_width: f32,
}

impl Default for ScopeColumn {
    fn default() -> Self {
        Self {
            collapsed: false,
            expanded_width: 160.0,
        }
    }
}

impl ScopeColumn {
    pub const RAIL_WIDTH: f32 = 20.0;
    const SNAP_WIDTH: f32 = 48.0;
    const MIN_EXPANDED_WIDTH: f32 = 72.0;

    #[must_use]
    pub fn width(self) -> f32 {
        if self.collapsed {
            Self::RAIL_WIDTH
        } else {
            self.expanded_width
        }
    }

    #[must_use]
    pub fn is_collapsed(self) -> bool {
        self.collapsed
    }

    pub fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
    }

    pub fn resize(&mut self, width: f32) {
        if !width.is_finite() {
            return;
        }
        self.collapsed = width < Self::SNAP_WIDTH;
        if !self.collapsed {
            self.expanded_width = width.max(Self::MIN_EXPANDED_WIDTH);
        }
    }

    pub(crate) fn timeline_offset(self) -> f64 {
        if self.collapsed {
            f64::from(Self::RAIL_WIDTH) - f64::from(self.expanded_width)
        } else {
            0.0
        }
    }
}

impl TryFrom<Saved> for ScopeColumn {
    type Error = &'static str;

    fn try_from(saved: Saved) -> Result<Self, Self::Error> {
        if !saved.expanded_width.is_finite() || saved.expanded_width < Self::MIN_EXPANDED_WIDTH {
            return Err("invalid expanded scope width");
        }
        Ok(Self {
            collapsed: saved.collapsed,
            expanded_width: saved.expanded_width,
        })
    }
}

impl From<ScopeColumn> for Saved {
    fn from(column: ScopeColumn) -> Self {
        Self {
            collapsed: column.collapsed,
            expanded_width: column.expanded_width,
        }
    }
}
