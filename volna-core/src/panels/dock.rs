//! Workspace regions share panel identity and history, but have independent trees.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{Axis, Layout, MAX_PANELS, PanelId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SideDock {
    pub layout: Layout,
    /// Width in design pixels, independent of interface zoom.
    pub width: f32,
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockLayout {
    pub center: Option<Layout>,
    pub right: Option<SideDock>,
}

impl From<Layout> for DockLayout {
    fn from(center: Layout) -> Self {
        Self {
            center: Some(center),
            right: None,
        }
    }
}

impl DockLayout {
    pub fn single(id: PanelId) -> Self {
        Layout::single(id).into()
    }

    pub(crate) fn trees(&self) -> impl Iterator<Item = &Layout> {
        self.center
            .iter()
            .chain(self.right.iter().map(|dock| &dock.layout))
    }

    fn trees_mut(&mut self) -> impl Iterator<Item = &mut Layout> {
        self.center
            .iter_mut()
            .chain(self.right.iter_mut().map(|dock| &mut dock.layout))
    }

    pub fn panels(&self) -> Vec<PanelId> {
        self.trees().flat_map(Layout::panels).collect()
    }

    pub fn visible(&self) -> Vec<PanelId> {
        self.center
            .iter()
            .chain(
                self.right
                    .iter()
                    .filter(|dock| dock.open)
                    .map(|dock| &dock.layout),
            )
            .flat_map(Layout::visible)
            .collect()
    }

    pub fn placement(&self, id: PanelId) -> Option<Placement> {
        if self
            .center
            .as_ref()
            .is_some_and(|tree| tree.active_for(id).is_some())
        {
            Some(Placement::Center)
        } else if self
            .right
            .as_ref()
            .is_some_and(|dock| dock.layout.active_for(id).is_some())
        {
            Some(Placement::Right)
        } else {
            None
        }
    }

    pub fn active_for(&self, id: PanelId) -> Option<PanelId> {
        self.trees().find_map(|tree| tree.active_for(id))
    }

    pub fn validate(&self, expected: &BTreeSet<PanelId>) -> Result<()> {
        ensure!(
            !expected.is_empty() && expected.len() <= MAX_PANELS,
            "invalid panel count"
        );
        let mut seen = BTreeSet::new();
        let mut nodes = 0;
        for tree in self.trees() {
            tree.validate_node(0, &mut nodes, &mut seen)?;
        }
        ensure!(
            &seen == expected,
            "layout panel IDs do not match the workspace"
        );
        if let Some(dock) = &self.right {
            ensure!(
                dock.width.is_finite() && dock.width > 0.0,
                "invalid right dock width"
            );
        }
        Ok(())
    }

    pub fn same_shape(&self, other: &Self) -> bool {
        fn same(a: Option<&Layout>, b: Option<&Layout>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => a.same_shape(b),
                (None, None) => true,
                _ => false,
            }
        }
        same(self.center.as_ref(), other.center.as_ref())
            && same(
                self.right.as_ref().map(|dock| &dock.layout),
                other.right.as_ref().map(|dock| &dock.layout),
            )
    }

    pub(crate) fn group_mut(&mut self, id: PanelId) -> Option<&mut Layout> {
        self.trees_mut().find_map(|tree| tree.group_mut(id))
    }

    pub(crate) fn activate(&mut self, id: PanelId) -> bool {
        if self.placement(id) == Some(Placement::Right) {
            self.right.as_mut().unwrap().open = true;
        }
        self.trees_mut().any(|tree| tree.activate(id))
    }

    pub(crate) fn split(&mut self, id: PanelId, new: PanelId, axis: Axis) {
        self.group_mut(id)
            .expect("known panel")
            .split(id, new, axis);
    }

    pub(crate) fn replace(&mut self, id: PanelId, new: PanelId) {
        if let Some(group) = self.group_mut(id) {
            group.replace(id, new);
        }
    }

    pub(crate) fn remove(&mut self, id: PanelId) -> Option<PanelId> {
        self.group_mut(id).and_then(|group| group.remove(id))
    }

    pub(crate) fn normalized(mut self) -> Option<Self> {
        self.center = self.center.and_then(Layout::normalized);
        self.right = self.right.and_then(|dock| {
            Some(SideDock {
                layout: dock.layout.normalized()?,
                ..dock
            })
        });
        (self.center.is_some() || self.right.is_some()).then_some(self)
    }

    pub(crate) fn insert(&mut self, id: PanelId, placement: Placement) {
        let tree = match placement {
            Placement::Center => self.center.get_or_insert_with(|| Layout::single(id)),
            Placement::Right => {
                let dock = self.right.get_or_insert_with(|| SideDock {
                    layout: Layout::single(id),
                    width: 420.0,
                    open: true,
                });
                dock.open = true;
                &mut dock.layout
            }
        };
        if tree.active_for(id).is_some() {
            tree.activate(id);
            return;
        }
        let first = tree.panels()[0];
        if let Some(Layout::Tabs { tabs, active }) = tree.group_mut(first) {
            tabs.push(id);
            *active = id;
        }
    }
}
