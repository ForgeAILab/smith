//! Smith's runtime composition.
//!
//! The shared `agent-runtime` facade owns execution: model and context
//! planning, provider normalization, the provider/tool loop, cancellation,
//! events, and usage. This crate owns everything a neutral runtime cannot
//! decide for a product — the concrete network transport, where sessions are
//! stored, what is written to the canonical journal, and the single factory
//! that maps one resolved Smith run onto `RuntimeBuilder`.
//!
//! Every Smith host — the TUI, `smith -p`, deterministic tests, child
//! sessions, and a future Forge adapter — composes a runtime through this
//! crate. Presentation may differ between hosts; runtime policy may not.

#![deny(unreachable_pub)]

pub(crate) mod abilities;
pub mod advisor;
pub mod artifact;
mod authority;
pub mod background_tasks;
pub mod built_in_skills;
pub mod cache_controller;
pub mod cache_lifecycle;
pub mod capability_limits;
pub(crate) mod catalog;
pub mod chatgpt;
pub mod checkpoint;
pub(crate) mod cli_agent;
pub mod client;
pub(crate) mod command_provider;
pub mod delegation;
pub mod factory;
pub(crate) mod gemini;
pub mod harness;
pub mod host;
pub mod journal;
pub mod mcp;
pub mod memory;
pub mod model_catalog;
pub mod pool;
pub mod pool_state;
mod private_storage;
pub mod probe;
pub mod project_instructions;
pub mod prompt;
pub mod reasoning;
pub(crate) mod renewable;
pub(crate) mod response;
pub mod resume_capsule;
pub mod rotation;
pub mod session;
pub(crate) mod session_history;
pub mod skills;
pub mod summary;
pub mod tool_output;
pub mod transport;
pub mod xai;

/// Structured direct-child spawn outcome exposed through Smith's composition
/// boundary for host surfaces.
pub use agent_runtime::delegation::{ChildDurability, ChildState, ChildStatus, SpawnOutcome};
/// The canonical session handle hosts drive after Smith has composed the
/// shared runtime. Re-exporting it here keeps production entry points on the
/// Smith composition boundary instead of depending on the full facade
/// directly.
pub use agent_runtime::runtime::SessionHandle;
