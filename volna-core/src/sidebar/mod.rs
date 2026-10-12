//! Hierarchy browser models: a container tree and a separate, filterable member list.

pub mod activity;
pub mod icons;
pub mod members;
pub mod scopes;

pub use activity::ActivityModel;
pub use members::MemberListModel;
pub use scopes::{ScopeTreeModel, TreeNode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Space,
    Escape,
    Char(String),
}
