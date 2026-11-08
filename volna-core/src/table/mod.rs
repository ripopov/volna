//! Reduced immutable table panel over one transaction generator or a fixed set of signal
//! histories.

pub mod columns;
mod copy;
pub mod filter;
pub mod format;
pub mod layout;
pub mod model;
pub mod paint;
pub mod query;
mod selection;
pub mod sort;
pub mod source;
pub mod statistics;

pub use layout::{ColumnRect, RowViewport, TableLayout};
pub use model::{
    AccessibleRow, PreparedCell, PreparedRow, PreparedWindow, RowIdentity, RowNavigation,
    TableCommand, TableModel, TableState,
};
pub use source::{SignalSource, TableSource};

pub(crate) mod workspace;
