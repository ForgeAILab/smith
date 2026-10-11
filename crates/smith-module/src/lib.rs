//! Value-returning module contracts, independent of Smith configuration and hosts.

mod contributions;
mod planner;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use agent_runtime::provider::transport::HttpTransport;
use agent_runtime_core::content::Message;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::provider_credential::{ProviderCredentialSource, ProviderCredentialTarget};

pub use contributions::{
    MAX_STATUS_LABEL_CHARS, ModuleContribution, PipelineComponent, SlashCommand, StatusItem,
    StatusSeverity, StatusSource,
};
pub use planner::{
    ModuleBlockReason, ModuleReport, ModuleState, MountPlan, MountedModule, mount_modules,
};

/// Plain resolved setting, without configuration-layer or parser dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    /// Text such as an image model, quality, or size.
    String(String),
    /// A resolved switch.
    Bool(bool),
    /// A resolved integral limit.
    Integer(i64),
}

/// Only the settings belonging to the module being mounted.
pub type ModuleSettings = BTreeMap<String, SettingValue>;

/// Authority posture handed to native modules; it does not grant capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModulePosture {
    /// The host admits only read-only tools.
    ReadOnly,
    /// Tools still require their ordinary approval and security checks.
    ReadWrite,
}

/// Provider-selected image route, expressed solely through shared contracts.
#[derive(Debug, Clone)]
pub struct ImageBinding {
    /// Provider endpoint for the images API.
    pub endpoint: String,
    /// Credential acquisition target for this route.
    pub target: ProviderCredentialTarget,
    /// Live credential source, including host-owned rotation.
    pub credentials: Arc<dyn ProviderCredentialSource>,
    /// Whether the route uses the ChatGPT image protocol.
    pub chatgpt: bool,
}

/// Read-only access to canonical history of live or resumed host sessions.
/// Visitors borrow history only for the duration of the call.
pub trait SessionHistory: Send + Sync + fmt::Debug {
    /// Visits canonical messages without maintaining a second history cache.
    fn with_history(
        &self,
        session: &SessionId,
        visitor: &mut dyn FnMut(&[Message]),
    ) -> Result<(), RuntimeError>;
}

/// Named host facts and services needed to mount one module.
#[derive(Clone)]
pub struct ModuleContext {
    /// This module's own resolved plain settings.
    pub settings: ModuleSettings,
    /// Host-selected user data directory.
    pub user_dir: PathBuf,
    /// Effective session authority posture.
    pub posture: ModulePosture,
    /// Shared provider HTTP transport contract.
    pub transport: Arc<dyn HttpTransport>,
    /// Selected provider image route, if supported.
    pub image_binding: Option<ImageBinding>,
    /// Read-only canonical history service for recent conversation inputs.
    pub session_history: Option<Arc<dyn SessionHistory>>,
    /// Whether the host installed semantic summaries for this composition.
    pub semantic_summary_enabled: bool,
    /// Resolved input ceiling for context-pressure components.
    pub max_input_tokens: u32,
    /// Whether the host requested optional built-in tools.
    pub built_in_tools: bool,
}

impl fmt::Debug for ModuleContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModuleContext")
            .field("setting_keys", &self.settings.keys().collect::<Vec<_>>())
            .field("posture", &self.posture)
            .field("has_image_binding", &self.image_binding.is_some())
            .field("has_session_history", &self.session_history.is_some())
            .field("semantic_summary_enabled", &self.semantic_summary_enabled)
            .field("max_input_tokens", &self.max_input_tokens)
            .field("built_in_tools", &self.built_in_tools)
            .finish_non_exhaustive()
    }
}

/// A compiled native feature. Mounting returns values, never a registrar.
pub trait Module: Send + Sync + fmt::Debug {
    /// Stable configuration id, without the evidence namespace prefix.
    fn id(&self) -> &str;
    /// Exact implementation revision recorded in composition evidence.
    fn revision(&self) -> &str;
    /// One-line user-facing description.
    fn description(&self) -> &str;
    /// Selection when no explicit host choice exists.
    fn default_enabled(&self) -> bool;
    /// Module ids that must have mounted successfully first.
    fn requirements(&self) -> &[&str] {
        &[]
    }
    /// Constructs all contributions atomically, or explains inactivity/failure.
    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError>;
}

/// Successful construction or a conditional module's explainable inactivity.
#[derive(Debug, Clone)]
pub enum Mounted {
    /// Complete set of contributions, including an intentionally empty set.
    Contributions(Vec<ModuleContribution>),
    /// Host facts make this selected module inapplicable to the session.
    Inactive {
        /// Safe user-facing explanation, never credentials or setting values.
        reason: String,
    },
}

/// Safe user-facing mount failure. Native modules must redact secrets themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleError(pub String);

impl fmt::Display for ModuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ModuleError {}

/// Build provenance, usable by configuration and clients without the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleOrigin {
    /// A module maintained in Smith's source.
    FirstParty,
    /// Trusted native code supplied by another crate.
    ThirdParty {
        /// Crate compiled into this binary.
        crate_name: String,
    },
}

/// Plain catalog entry, including first-party modules omitted from this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDescriptor {
    /// Stable configuration id.
    pub id: String,
    /// One-line description for listings.
    pub description: String,
    /// Declared default selection.
    pub default_enabled: bool,
    /// First-party or named third-party provenance.
    pub origin: ModuleOrigin,
    /// Whether this binary contains the implementation.
    pub compiled_in: bool,
}

/// One explicit entry in the host's compiled-in module list.
#[derive(Debug, Clone)]
pub struct CompiledModule {
    /// Native implementation compiled into the host.
    pub module: Arc<dyn Module>,
    /// Build provenance supplied by the composition root.
    pub origin: ModuleOrigin,
}

impl CompiledModule {
    /// Plain metadata for handing the compiled catalog to configuration.
    pub fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            id: self.module.id().to_owned(),
            description: self.module.description().to_owned(),
            default_enabled: self.module.default_enabled(),
            origin: self.origin.clone(),
            compiled_in: true,
        }
    }
}

/// Explicit composition input until layered module selection is implemented.
#[derive(Debug, Clone, Default)]
pub struct ModuleComposition {
    /// The sole compiled-in list supplied by the host.
    pub compiled: Vec<CompiledModule>,
    /// Additional catalog entries, normally first-party modules not built.
    pub known: Vec<ModuleDescriptor>,
    /// Fully resolved enabled ids; mounting never reads configuration keys.
    pub enabled: BTreeSet<String>,
    /// Resolved own settings indexed by module id.
    pub settings: BTreeMap<String, ModuleSettings>,
}

impl ModuleComposition {
    /// Uses declared defaults until the host has layered module selection.
    pub fn with_defaults(compiled: Vec<CompiledModule>, known: Vec<ModuleDescriptor>) -> Self {
        let enabled = compiled
            .iter()
            .map(CompiledModule::descriptor)
            .chain(known.iter().cloned())
            .filter(|descriptor| descriptor.default_enabled)
            .map(|descriptor| descriptor.id)
            .collect();
        Self {
            compiled,
            known,
            enabled,
            settings: BTreeMap::new(),
        }
    }
}
