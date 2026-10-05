//! Rendering facade for the terminal surface.

mod approval;
mod composer;
mod helpers;
mod layout;
pub(crate) mod lists;
mod markdown;
mod modal;
mod reports;
mod transcript;
pub(crate) mod wrap;

pub(crate) use transcript::TranscriptCache;

pub use layout::{
    MIN_HEIGHT, MIN_WIDTH, SurfaceLayout, draw, draw_synced, draw_with_screen, layout,
    selected_text,
};
