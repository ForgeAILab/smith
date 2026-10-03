//! Smith's client-neutral session accounting, commands, and local reports.
//!
//! Terminal and headless clients share the same cache projection, usage
//! counters, catalog pricing, and append-only usage log. These modules consume
//! runtime events and plain values; terminal drawing belongs to `smith-tui`.

pub mod agent_report;
pub mod cache;
pub mod commands;
pub mod context_report;
mod format;
pub mod goal_report;
pub mod help_report;
pub mod local_result;
pub mod status;
pub mod status_report;
pub mod time_display;
pub mod timeline_report;
pub mod usage_log;
