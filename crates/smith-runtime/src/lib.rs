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

pub mod abilities;
pub mod artifact;
mod authority;
pub mod background_tasks;
pub mod budget_notice;
pub mod built_in_skills;
pub mod cache_controller;
pub mod cache_lifecycle;
pub mod catalog;
pub mod chatgpt;
pub mod checkpoint;
pub mod delegation;
pub mod factory;
pub mod host;
pub mod journal;
pub mod mcp;
pub mod memory;
pub mod model_catalog;
pub mod pool;
pub mod pool_state;
mod private_storage;
pub mod project_instructions;
pub mod prompt;
pub mod reasoning;
pub mod renewable;
pub mod response;
pub mod resume_capsule;
pub mod rotation;
pub mod session;
pub mod skills;
pub mod summary;
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
