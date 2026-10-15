//! Smith's deterministic layered configuration resolver.
//!
//! Public paths remain rooted here while resolution responsibilities live in
//! private child modules.

mod agent;
mod harness;
mod load;
mod mcp;
mod modules;
mod plugins;
mod provenance;
mod provider;
mod types;
mod vision;
mod web;

pub use load::{env_name, inspect, resolve};
pub use plugins::{PluginSelection, resolve_plugin_selection};
pub use provenance::*;
pub use types::*;
pub use vision::image_input;
