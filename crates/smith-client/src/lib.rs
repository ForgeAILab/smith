//! Smith's client-neutral session accounting and command registry.
//!
//! Terminal and headless clients share the same cache projection, usage
//! counters, catalog pricing, and append-only usage log. These modules consume
//! runtime events and plain values; terminal drawing belongs to `smith-tui`.

pub mod cache;
pub mod commands;
mod format;
pub mod status;
pub mod time_display;
pub mod usage_log;
