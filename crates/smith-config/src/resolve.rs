//! Smith's deterministic layered configuration resolver.
//!
//! Public paths remain rooted here while resolution responsibilities live in
//! private child modules.

mod agent;
mod load;
mod provenance;
mod provider;
mod types;

pub use load::{env_name, inspect, resolve};
pub use provenance::*;
pub use types::*;
