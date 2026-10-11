//! The Transaction panel: one recorded transaction, prepared for reading.
//! The view is a pure function of resident data ([`view()`]); the model owns
//! which record a panel shows, its history, the reader's preferences, and
//! explicitly requested parent tracing across loaded and pending generators.

pub mod model;
pub mod parents;
pub mod view;

pub use model::{
    MAX_HISTORY, PANEL_BYTES, ShownRecord, TransactionCommand, TransactionModel, TxPanelState,
};
pub use parents::{ParentNode, ParentTrace, ParentTraceState};
pub use view::{
    AttrRow, Chip, EventRow, EventTick, Identity, LaneCell, LaneRow, RefRole, RefRow, Section,
    SectionKey, StageRow, Timing, TxView, ViewPrefs, view,
};

pub(crate) mod workspace;
