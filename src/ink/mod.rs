//! Freehand ink: stroke data, smoothing, erasing and undo/redo.

pub mod eraser;
pub mod history;
pub mod smooth;
pub mod store;
pub mod stroke;

pub use history::History;
pub use store::Store;
pub use stroke::Stroke;
