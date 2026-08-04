//! Rendering facade for the terminal surface.

mod approval;
mod composer;
mod helpers;
mod layout;
mod modal;
mod transcript;

pub use layout::{MIN_HEIGHT, MIN_WIDTH, draw, draw_synced, selected_text};
