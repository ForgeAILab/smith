//! Smith's configuration layer.
//!
//! Smith resolves every product decision — provider, model, limits, prompts,
//! persistence, approval, credentials — before it constructs a runtime. The
//! shared runtime is deliberately neutral about all of that, so this crate is
//! where the answers come from and where a user can find out *why* a value
//! won: every resolved field carries the layer it came from.
//!
//! The layering, lowest precedence first:
//!
//! ```text
//! built-in defaults
//! → ~/.smith/config.toml
//! → <project>/.smith/config.toml
//! → <project>/.smith/config.local.toml
//! → selected profile
//! → SMITH_* environment variables
//! → CLI flags
//! → explicit per-session overrides
//! ```
//!
//! Nothing here performs provider I/O; a configuration failure must be visible
//! before a terminal is entered or a request is sent.

pub mod catalog;
pub mod credential;
pub mod inventory;
pub mod model;
pub mod resolve;
pub mod setup;
pub mod trust;
pub mod user_config;
