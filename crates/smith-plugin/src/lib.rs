//! Inspect and store plugin bundles without prompting or deciding trust.
//!
//! Inspection returns an inventory and an installation-independent digest.
//! Prepared installs and updates own a temporary snapshot; only an explicit
//! commit changes the installed record. No plugin-supplied program is run.

mod diff;
mod digest;
mod manifest;
mod marketplace;
mod mcp;
mod modules;
pub use modules::{ModuleCapabilities, WasmModuleComponent, endpoint_parts};
mod scan;
mod source;
mod store;

pub use diff::{ComponentChange, InventoryDifference};
pub use manifest::{PluginManifest, read_manifest, validate_name};
pub use marketplace::{Marketplace, MarketplacePlugin, MarketplaceRecord};
pub use mcp::{McpDeclaration, PathVariables, needs_configuration, substitute_variables};
pub use scan::{
    CommandComponent, CommandSource, NotLoadedComponent, PluginInventory, SkillComponent, inspect,
};
pub use source::{FetchedSource, GitFetcher, PluginSource};
pub use store::{InstalledPlugin, InstalledState, PluginStore, PreparedPlugin, PreparedUpdate};

/// Failures of inspection, fetching, or filesystem transactions.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// Filesystem access failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON could not be parsed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// A persisted record could not be parsed.
    #[error(transparent)]
    TomlRead(#[from] toml::de::Error),
    /// A persisted record could not be encoded.
    #[error(transparent)]
    TomlWrite(#[from] toml::ser::Error),
    /// The manifest or source has an invalid shape.
    #[error("{0}")]
    Invalid(String),
    /// A source is recognised but cannot be fetched.
    #[error("{kind} sources are not supported")]
    UnsupportedSource {
        /// The unsupported source type.
        kind: String,
    },
    /// The configured git executable is missing.
    #[error("git is unavailable: cannot find `{program}`; install git or configure its path")]
    GitUnavailable {
        /// The executable that could not be found.
        program: String,
    },
    /// Git exited unsuccessfully.
    #[error("git failed: {0}")]
    Git(String),
    /// A copy or manifest path escaped its root.
    #[error("`{0}` escapes the plugin root")]
    PathEscape(std::path::PathBuf),
    /// The prepared snapshot changed before commit.
    #[error("plugin content changed after inspection; inspect and confirm it again")]
    Changed,
}

/// Result type for plugin operations.
pub type Result<T> = std::result::Result<T, PluginError>;
