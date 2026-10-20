//! Value-returning module contracts, independent of Smith configuration and hosts.

mod contributions;
mod http;
mod model;
pub use http::{
    MODULE_HTTP_BODY_LIMIT, ModuleHttp, ModuleHttpFuture, ModuleHttpMethod, ModuleHttpRequest,
    ModuleHttpResponse,
};
pub use model::{
    ModuleModel, ModuleModelFuture, ModuleModelInvocation, ModuleModelRequest, ModuleModelResponse,
};
mod planner;
mod sandbox;
pub use sandbox::{ModuleCapability, ModuleHealth};

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
    MAX_STATUS_LABEL_CHARS, ModuleContribution, ModuleNotice, ModulePollFuture, ModuleWatcher,
    PipelineComponent, SlashCommand, StatusItem, StatusSeverity, StatusSource,
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

/// Resolved web settings, independent of the configuration layers.
#[derive(Debug, Clone, Default)]
pub struct WebSettings {
    /// Whether fetches may connect to private and local addresses.
    pub allow_private_networks: bool,
    /// Backend route; absent selects keyless DuckDuckGo.
    pub search: Option<SearchBackendConfig>,
}

/// Backend selection with credentials already resolved by the host.
#[derive(Debug, Clone, Default)]
pub enum SearchBackendConfig {
    /// Search is off or handled by the provider.
    Disabled,
    /// Keyless, best-effort HTML search.
    #[default]
    DuckDuckGo,
    /// Brave web search with its subscription token.
    Brave {
        /// Resolved token, absent when credential acquisition failed.
        api_key: Option<agent_runtime_core::store::Secret>,
    },
    /// Tavily search with a bearer token.
    Tavily {
        /// Resolved token, absent when credential acquisition failed.
        api_key: Option<agent_runtime_core::store::Secret>,
    },
    /// User-selected SearXNG instance with JSON search enabled.
    SearXng {
        /// Absolute instance URL, absent when not configured.
        url: Option<String>,
    },
}

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

/// Session accounting for separately attributed vision provider work.
pub trait VisionUsageRecorder: agent_runtime::harness::TurnCommitHook {
    /// Retains reported usage, including partial usage from interrupted calls.
    fn record(&self, session: &SessionId, record: agent_runtime_core::usage::UsageRecord);
}

/// Tool-free vision route resolved independently of the main model.
#[derive(Debug, Clone)]
pub struct VisionBinding {
    /// Constructed provider, including credentials and response policy.
    pub provider: Arc<dyn agent_runtime_core::provider::Provider>,
    /// Provider identity for usage attribution.
    pub provider_name: String,
    /// Selected image-input model.
    pub model: agent_runtime_core::provider::ModelId,
    /// Exact provider endpoint declared to the approval gate.
    pub endpoint: String,
    /// Maximum requested output tokens for one answer.
    pub output_token_cap: u32,
    /// Host-owned session usage accounting.
    pub usage: Arc<dyn VisionUsageRecorder>,
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
    /// Canonical workspace root for confined module file access.
    pub workspace_root: Option<PathBuf>,
    /// Host-selected user data directory.
    pub user_dir: PathBuf,
    /// Child mounts bind their health by the actual invocation session id.
    pub is_child_session: bool,
    /// Effective session authority posture.
    pub posture: ModulePosture,
    /// Shared provider HTTP transport contract.
    pub transport: Arc<dyn HttpTransport>,
    /// Bounded guest HTTP service, separate from provider POST serialization.
    pub http: Option<Arc<dyn ModuleHttp>>,
    /// Optional tool-only model service; sandbox mediation remains host-owned.
    pub model: Option<Arc<dyn ModuleModel>>,
    /// Selected provider image route, if supported.
    pub image_binding: Option<ImageBinding>,
    /// Separate vision route, only resolved for a text-only main model.
    pub vision: Option<VisionBinding>,
    /// Whether vision has no selection, distinct from an explicit opt-out.
    pub vision_unset: bool,
    /// Main-enabled image-input profiles in profile order, excluding the active profile.
    pub image_capable_profiles: Vec<String>,
    /// Resolved web route and private-network policy; absent uses defaults.
    pub web: Option<WebSettings>,
    /// Whether the active main model accepts image input at this composition.
    pub main_model_accepts_images: bool,
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
            .field("has_model", &self.model.is_some())
            .field("has_image_binding", &self.image_binding.is_some())
            .field("has_vision", &self.vision.is_some())
            .field("vision_unset", &self.vision_unset)
            .field("image_capable_profiles", &self.image_capable_profiles)
            .field("has_web_settings", &self.web.is_some())
            .field("main_model_accepts_images", &self.main_model_accepts_images)
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
    /// Owned dependency ids for adapters whose metadata is loaded at runtime.
    fn requirement_ids(&self) -> Vec<String> {
        self.requirements()
            .iter()
            .map(|id| (*id).to_owned())
            .collect()
    }
    /// Host-owned session health pool for filtering disabled sandboxed tools.
    /// Native modules normally return None; this handle is never exported to guests.
    fn session_health(&self) -> Option<Arc<ModuleHealth>> {
        None
    }
    /// Constructs all contributions atomically, or explains inactivity/failure.
    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError>;
}

/// Safe guidance for a user-fixable inactive capability. Never include secrets or file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupHint {
    /// Short phrase describing the missing capability and its consequence.
    pub missing: String,
    /// Built-in task skill that walks the user through setup.
    pub guide: String,
    /// At most five profile names or provider/model references from configuration.
    pub candidates: Vec<String>,
    /// Configuration key whose value the candidates are for, when one fixes it.
    pub setting: Option<String>,
}

impl SetupHint {
    /// Constructs a hint with a bounded candidate list in the caller's order.
    pub fn new(
        missing: impl Into<String>,
        guide: impl Into<String>,
        candidates: Vec<String>,
    ) -> Self {
        Self {
            missing: missing.into(),
            guide: guide.into(),
            candidates: candidates.into_iter().take(5).collect(),
            setting: None,
        }
    }

    /// Names the configuration key a candidate would be assigned to.
    #[must_use]
    pub fn with_setting(mut self, key: impl Into<String>) -> Self {
        self.setting = Some(key.into());
        self
    }

    /// The developer-fragment line: the gap, the candidates, and when to
    /// raise it.
    pub fn agent_text(&self) -> String {
        let mut text = format!("{}.", self.missing);
        if !self.candidates.is_empty() {
            text.push_str(&format!(
                " {}: {}.",
                self.candidates_label(),
                self.candidates.join(", ")
            ));
        }
        match &self.setting {
            // A concrete key and values keep the agent from proposing a
            // model it read in documentation rather than in this config.
            Some(key) if !self.candidates.is_empty() => text.push_str(&format!(
                " When the user's request needs this, tell them and offer to set `{key}` to one of these in their user config; suggest no other value. Activate {} for the steps.",
                self.guide
            )),
            _ => text.push_str(&format!(
                " When the user's request needs this, tell them and activate {} to walk them through enabling it.",
                self.guide
            )),
        }
        text
    }

    /// The short status suffix shown to the user beside the inactive reason.
    pub fn status_text(&self) -> String {
        let mut text = format!("set up with {}", self.guide);
        if !self.candidates.is_empty() {
            text.push_str(&format!(
                " · {}: {}",
                self.candidates_label().to_lowercase(),
                self.candidates.join(", ")
            ));
        }
        text
    }

    fn candidates_label(&self) -> &'static str {
        if self.guide == "smith.guide.vision" {
            "Profiles that accept images"
        } else {
            "Candidates"
        }
    }
}

/// Successful construction or a conditional module's explainable inactivity.
#[derive(Debug, Clone)]
pub enum Mounted {
    /// Complete set of contributions, including an intentionally empty set.
    Contributions(Vec<ModuleContribution>),
    /// Session contributions with live failure diagnostics.
    TrackedContributions {
        /// Atomic contribution set.
        contributions: Vec<ModuleContribution>,
        /// Session-scoped health, retained by reports and clients.
        health: Arc<ModuleHealth>,
    },
    /// Host facts make this selected module inapplicable to the session.
    Inactive {
        /// Safe user-facing explanation, never credentials or setting values.
        reason: String,
        /// Optional user-fixable capability gap, carrying no authority.
        setup_hint: Option<SetupHint>,
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
    /// Sandboxed component installed in a user plugin.
    Sandboxed {
        /// Digest-trusted plugin name.
        plugin: String,
        /// Manifest requests, before narrowing.
        requested_capabilities: BTreeSet<ModuleCapability>,
        /// Effective host grants; tool approval effects are separate.
        granted_capabilities: BTreeSet<ModuleCapability>,
        /// Plain descriptions of exact effective scopes.
        capability_details: Vec<String>,
        /// Confirmed capabilities withheld by user restrictions.
        withheld_capabilities: Vec<String>,
    },
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
