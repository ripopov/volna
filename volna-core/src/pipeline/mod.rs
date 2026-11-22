//! The pipeline panel: Konata-style rows of stage cells over any transaction track, sharing
//! the document's time axis with the wave panels and owning a local row axis.

pub mod activity;
pub use activity::{Activity, ActivityCommand, FollowActivity};
pub mod layout;
pub mod model;
pub mod navigator;
pub mod plot;
pub use navigator::{NavigatorCommand, NavigatorSettings};
pub use plot::{OccupancyCommand, OccupancySettings};
pub(crate) mod occupancy;
mod opening;
pub mod order;
pub use opening::ViewportSize;
pub use order::{
    RowOrder, RowOrderCommand, RowOrderMode, RowSortMeta, StageOrderOption, UnmatchedPolicy,
};
pub mod paint;
pub mod palette;
mod raster;
mod row_set;
mod selection;
mod stage_layout;
pub mod stages;
pub use selection::ZoomSelection;
pub use stages::{StageVisibility, StageVisibilityCommand};
pub mod rows;
pub mod sources;
pub mod zoom;

pub use layout::PipelineLayout;
pub use model::{Hit, PipelineCellContext, PipelineModel, RowBoundary, Rows, TrackSource};
pub use palette::{StagePalette, StageStyle, StageSwatch};
pub use rows::RowView;
pub use sources::{
    FocusStyle, PipelineSource, PipelineSourceCommand, SourceBadges, SourceLabel, SourceLoadStatus,
    compute_source_badges,
};

pub(crate) mod workspace;
