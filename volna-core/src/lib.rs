//! Volna core: everything the viewer decides, with no GUI toolkit attached.

pub mod analysis;
pub mod app;
pub mod clock;
pub mod color;
pub mod data;
pub mod document;
pub mod frames;
pub mod geometry;
pub mod history;
pub mod icons;
pub mod marker;
pub mod measure;
pub mod nav;
pub mod panels;
pub mod pipeline;
pub mod remote;
pub mod scene;
pub mod selection;
pub mod session;
pub mod settings;
pub mod sidebar;
pub mod table;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod theme;
pub mod trace;
pub mod transaction;
pub mod wave;

pub use app::{Action, App, Command, Event};
pub use color::Color;
pub use document::Document;
pub use scene::{FontRole, Scene, TextMeasure};
pub use session::{LoadRequest, LoadResult};
pub use theme::Theme;

pub use web_time::Instant;

pub mod workspace;
