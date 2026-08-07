//! The one place a Smith runtime is composed.
//!
//! Every Smith host — the interactive TUI, `smith -p`, deterministic tests,
//! direct child sessions, and a future Forge adapter — builds its runtime here.
//! Presentation may differ between them; runtime policy may not, and the only
//! way to guarantee that is for there to be exactly one function that maps a
//! resolved Smith configuration onto [`RuntimeBuilder`]. An entry point that
//! composed its own runtime would be a second policy that drifts silently.
//!
//! Nothing about a terminal, a stream of frames, or an output format appears
//! here. What a host *does* own arrives as parameters: the approval surface,
//! the workspace boundary, extra tools, stores, observers, a clock, and — for
//! tests and development — a provider to use instead of the configured one.
//! [`HostSurface`] records which presentation asked, and deliberately changes
//! nothing: two surfaces that pass the same configuration and adapters get
//! byte-identical [`RuntimePolicy`].
//!
//! # Startup order
//!
//! `design.md` fixes the order, and the reason is that each step can fail in a
//! way the user must see before the next step costs anything:
//!
//! 1. discover the project, load the declarative layers, select a profile,
//!    validate and explain it — [`smith_config::resolve`], upstream of here;
//! 2. confirm executable project trust where any is needed — the host's step,
//!    also upstream;
//! 3. select the provider adapter;
//! 4. resolve credentials;
//! 5. resolve the model profile;
//! 6. build the provider, then the runtime.
//!
//! [`build`] owns steps 3 to 6 and fails closed at every one of them. Required
//! host policy is checked before all of it, because that check needs no I/O at
//! all: a run with no workspace must not reach a credential service, let alone
//! a provider. Nothing here opens a socket, so a configuration failure is
//! always reported before any provider network I/O — and, because a host enters
//! its terminal only after this function returns, before the alternate screen.
//!
//! # The secret boundary
//!
//! A resolved [`Secret`] exists between step 4 and step 6. The OpenAI-compatible
//! path moves it into a host-injected static credential source at provider
//! construction; the adapter acquires that source only at its trusted request
//! boundary. When persistence is enabled, the same value
//! is registered with the host's non-printing redactor so a reflected
//! credential cannot reach a journal or saved snapshot. It is never stored on
//! the run request, in [`RuntimePolicy`], or in any error. Every type on that
//! path — `Secret`, `DefaultRedactor`, `ProviderError`, [`FactoryError`] —
//! renders locators and classifications rather than values.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use agent_runtime::ability::SealedAbilities;
use agent_runtime::ability::activation::{ActivationContext, FailClosedPolicy};
use agent_runtime::agent::config::{DowngradePolicy, LoopConfig};
use agent_runtime::capability::{ActivationBudget, CapabilityResolver};
use agent_runtime::context::{
    CompactionPolicy, ContextBudget, ContextPolicy, ProviderCacheCapability, StructuralCompactor,
};
use agent_runtime::harness::{
    ArtifactOffloader, ArtifactReadTool, CreateGoalTool, GetGoalTool, GoalComponent,
    MemoryContributor, MemorySource, QuestionnaireTool, SemanticSummaryCoordinator, SummaryModel,
    TodoComponent, UpdateGoalTool, WriteTodosTool,
};
use agent_runtime::hub::{ScopeIdentity, ScopeInputs};
use agent_runtime::provider::anthropic::{AnthropicConfig, AnthropicProvider};
use agent_runtime::provider::fake::FakeProvider;
use agent_runtime::provider::gemini::{GeminiInteractionsConfig, GeminiInteractionsProvider};
use agent_runtime::provider::openai::{OpenAiConfig, OpenAiProvider};
use agent_runtime::provider::responses::{ResponsesConfig, ResponsesProvider};
use agent_runtime::provider::retry::RetryPolicy;
use agent_runtime::registry::Permission;
use agent_runtime::registry::RegistryRevision;
use agent_runtime::runtime::{Runtime, RuntimeBuilder};
use agent_runtime_core::approval::{AllowAll, ApprovalPolicy, DenyAll};
use agent_runtime_core::artifact::ArtifactStore;
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::catalog::{ModelCatalogSource, ModelProfileError, ResolvedModelProfile};
use agent_runtime_core::catalog::{ModelLimits, ModelRecord};
use agent_runtime_core::checkpoint::{CHECKPOINT_SCHEMA_VERSION, CheckpointStore};
use agent_runtime_core::clock::{Clock, Deadline, SystemClock};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::interaction::{InteractionBroker, InteractionReadiness};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::provider::{ModelId, PromptCacheControl, Provider, ProviderError};
use agent_runtime_core::provider_credential::{
    ProviderCredentialSource, ProviderCredentialTarget, StaticProviderCredentialSource,
};
use agent_runtime_core::store::{Secret, SecretStore, SessionStore};
use agent_runtime_core::tool::Tool;
use agent_runtime_core::workspace::Workspace;
use async_trait::async_trait;
use reqwest::Url;
use smith_config::credential::{
    CredentialEnroller, CredentialError, CredentialRef, CredentialRefError, CredentialResolver,
};
use smith_config::model::{
    AgentPosture, ApprovalMode, KIND_ANTHROPIC_MESSAGES, KIND_CHATGPT_RESPONSES, KIND_FAKE,
    KIND_GEMINI_INTERACTIONS, KIND_OPENAI_COMPATIBLE, KIND_OPENAI_RESPONSES, KIND_XAI_RESPONSES,
    ProfileUse,
};
use smith_config::resolve::{ResolvedConfig, ResolvedProvider};
use smith_config::setup::trusted_model;

use agent_runtime_core::check_set::ActionClass;
use agent_runtime_core::grant::SecurityCheckMode;

use smith_host::rotation::{HeadlessRotation, RotationPolicy};

use crate::abilities::{INTERACTION_READY_CONFIG, seal_tool_abilities};
use crate::authority::SmithToolAuthority;
use crate::background_tasks::RegistryBackgroundTaskHost;
use crate::budget_notice::{BudgetNoticeComponent, DEFAULT_NOTICE_THRESHOLD_TOKENS};
use crate::catalog::{CatalogLayers, ProfileResolution};
use crate::chatgpt::{
    ChatGptCredentialSource, ChatGptOAuthClient, ChatGptProvider, ChatGptProviderConfig,
    ChatGptTokenBundle,
};
use crate::checkpoint::{BarrierCheckpointStore, CheckpointBarrier, SmithCheckpointSetup};
use crate::delegation::{
    AgentTool, AgentToolProfile, DelegationAuthority, SmithChildFactory, SmithChildRoute,
    SmithDelegation,
};
use crate::journal::DefaultRedactor;
use crate::memory::SmithMemorySource;
use crate::pool::CredentialPool;
use crate::project_instructions::{ProjectInstructionsIdentity, ProjectInstructionsSnapshot};
use crate::prompt::{
    AgentProfilePrompt, DynamicPromptContext, SmithPromptContributor, render_fragments,
};
use crate::reasoning::{
    ReasoningDialectProvider, ReasoningInterceptor, ReasoningRuntimePolicy,
    resolve_reasoning_policy,
};
use crate::renewable::{BundleRefresher, RenewableBundle, RenewableMemberSources};
use crate::rotation::{
    PoolCredentialSource, PoolMemberSources, PooledProvider, SharedPool, StaticMemberSources,
};
use crate::skills::{ResolvedSmithSkills, SkillIndexEntry, SmithSkillSources};
use crate::summary::{
    SemanticSummaryRuntimePolicy, SmithProviderSummaryModel, SmithSemanticSummaryConfig,
};
use crate::transport::{ReqwestTransport, TransportConfig};
use crate::xai::{XaiCredentialSource, XaiOAuthClient, XaiTokenBundle};

/// The reply the deterministic development provider gives.
///
/// A `fake` provider with nothing injected is a development or smoke-test
/// composition, so it answers rather than failing — but it answers something
/// that cannot be mistaken for a model.
pub const DEVELOPMENT_REPLY: &str = "This session is running Smith's deterministic fake provider; \
     configure a real provider to talk to a model.";

/// The schema revision of the context policy Smith derives from configuration.
///
/// Bumped when the *shape* of that derivation changes. The resolved reserves
/// are appended to it in [`RuntimePolicy::context_policy`], because the policy
/// revision is what identifies the policy in every downstream plan and cache
/// fingerprint: two runs with different reserves must not share one.
pub const CONTEXT_POLICY_REVISION: &str = "smith-context-policy-1";

/// The schema revision of the semantic compaction policy Smith derives from
/// the resolved input budget and configured percentage watermarks.
pub const COMPACTION_POLICY_REVISION: &str = "smith-compaction-policy-1";

/// The revision recorded for the cache capability Smith derives from the
/// selected adapter's declared [`PromptCacheControl`].
pub const CACHE_CAPABILITY_REVISION: &str = "smith-provider-cache-2";

/// The default bound on the runtime's event broadcast buffer.
pub const DEFAULT_EVENT_BUFFER: usize = 1_024;

/// The default bounded-shutdown grace period, in milliseconds.
pub const DEFAULT_SHUTDOWN_TIMEOUT_MS: u64 = 5_000;

/// Maximum time Smith waits for a credential service lookup.
///
/// Platform keychains may wait on an unlock or access-control prompt. A
/// bounded lookup keeps that prompt from turning headless startup or setup
/// preflight into an indefinite hang.
pub const DEFAULT_CREDENTIAL_TIMEOUT_MS: u64 = 30_000;

/// Adapter kinds compiled into this Smith build.
///
/// Setup uses this list to hide descriptors it cannot actually compose.
pub const AVAILABLE_ADAPTER_KINDS: &[&str] = &[
    KIND_OPENAI_COMPATIBLE,
    KIND_OPENAI_RESPONSES,
    KIND_ANTHROPIC_MESSAGES,
    KIND_CHATGPT_RESPONSES,
    KIND_XAI_RESPONSES,
    KIND_GEMINI_INTERACTIONS,
    KIND_FAKE,
];

/// Which Smith presentation asked for this runtime.
///
/// Declared metadata only. It is recorded on the built [`SmithRuntime`] so a
/// manifest or a diagnostic can say where a run came from, and it is
/// deliberately absent from [`RuntimePolicy`]: the moment a surface can change
/// what the runtime does, "one composition path" stops being true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostSurface {
    /// The interactive terminal client.
    Terminal,
    /// A non-interactive run, such as `smith -p`.
    Headless,
    /// A direct child session started by the root agent.
    Child,
}

/// Whether this composition can durably resume exact in-flight work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MidTurnDurability {
    /// An authenticated protected checkpoint store is installed.
    Available,
    /// Exact state is not stored; redacted completed-turn snapshots may still
    /// be available according to persistence policy.
    Unavailable,
}

impl MidTurnDurability {
    /// Stable status spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
        }
    }
}

impl HostSurface {
    /// A stable lowercase slug.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Headless => "headless",
            Self::Child => "child",
        }
    }
}

/// One resolved Smith run, plus the host policy a neutral runtime cannot decide.
///
/// The configuration half is already typed and validated by
/// [`smith_config::resolve`]; the rest is what the host injects. Fields are
/// public and the struct is built with update syntax:
///
/// ```ignore
/// let request = RuntimeRequest {
///     workspace: Some(Arc::new(workspace)),
///     approval: Some(Arc::new(approval)),
///     ..RuntimeRequest::new(config, HostSurface::Terminal)
/// };
/// ```
#[derive(Debug)]
pub struct RuntimeRequest {
    /// The resolved, provenance-carrying run configuration.
    pub config: ResolvedConfig,
    /// Which presentation is composing this runtime.
    pub surface: HostSurface,
    /// A complete product-instruction override. When absent, Smith's versioned
    /// prompt sections are composed through [`crate::prompt`].
    pub system_prompt: Option<String>,
    /// Immutable root project instructions already validated by the host.
    ///
    /// The factory performs no ambient file discovery. A complete
    /// `system_prompt` override retains replacement semantics and ignores this
    /// snapshot.
    pub project_instructions: Option<ProjectInstructionsSnapshot>,
    /// The workspace boundary tools resolve paths through. Required: the shared
    /// runtime would otherwise fall back to denying everything silently.
    pub workspace: Option<Arc<dyn Workspace>>,
    /// The approval surface. Required when `approval.mode` is `ask`, since a
    /// question with nobody to answer it is a hang or a silent denial.
    pub approval: Option<Arc<dyn ApprovalPolicy>>,
    /// Authority-free task interaction surface. The questionnaire tool is
    /// installed for root runtimes, but its schema is advertised only while
    /// this broker reports readiness.
    pub interaction: Option<Arc<dyn InteractionBroker>>,
    /// The surface that answers a credential-rotation offer.
    ///
    /// Only consulted when the provider declares more than one credential.
    /// Absent means no surface can answer, which declines: an unattended run
    /// keeps the account it started on rather than spending another one.
    pub rotation: Option<Arc<dyn RotationPolicy>>,
    /// Live pool state, when the host wants to observe or seed it.
    ///
    /// A host supplies this to start on a remembered account, to draw usage
    /// meters, and to persist a switch. Absent, the factory builds its own and
    /// the choice lasts only for the session.
    pub credential_pool: Option<SharedPool>,
    /// Tools registered in addition to Smith's built-ins.
    pub tools: Vec<Arc<dyn Tool>>,
    /// Optional host-owned recorder wrapped around built-in mutating tools.
    pub change_recorder: Option<Arc<smith_tools::ChangeRecorder>>,
    /// Smith-owned, descriptor-first skill sources.
    pub skills: SmithSkillSources,
    /// Optional Smith-owned bounded memory policy and records.
    pub memory: Option<Arc<SmithMemorySource>>,
    /// Optional semantic-summary policy. Standard persistent hosts install
    /// Smith's default; direct embedders opt in explicitly.
    pub semantic_summary: Option<SmithSemanticSummaryConfig>,
    /// Whether [`smith_tools::all`] is registered. A read-only child view sets
    /// this to `false` and supplies its own narrower set.
    pub built_in_tools: bool,
    /// Where session snapshots are persisted.
    pub session_store: Option<Arc<dyn SessionStore>>,
    /// An already-initialized exact turn checkpoint store.
    pub checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    /// Deferred Smith protected-store setup. This runs only after ordinary
    /// factory preflight has resolved the provider and credential.
    pub checkpoint_setup: Option<SmithCheckpointSetup>,
    /// Optional host-owned durability boundary ordered before each protected
    /// checkpoint publication.
    pub checkpoint_barrier: Option<Arc<dyn CheckpointBarrier>>,
    /// The secret store exposed to the runtime, for hosts that need one.
    pub secret_store: Option<Arc<dyn SecretStore>>,
    /// Session-private artifact storage. When present, Smith registers the
    /// bounded reader and offloads oversized exact tool outcomes before the
    /// runtime applies its model-facing bound.
    pub artifact_store: Option<Arc<dyn ArtifactStore>>,
    /// Canonical event observers, such as the JSON Lines journal.
    pub observers: Vec<Arc<dyn EventObserver>>,
    /// The shared persistence redactor. A standard Smith host injects this
    /// before construction so the credential resolved here can be registered
    /// without returning or retaining its raw value in host policy.
    pub persistence_redactor: Option<DefaultRedactor>,
    /// The clock, for deterministic tests.
    pub clock: Option<Arc<dyn Clock>>,
    /// How a configured credential reference becomes a secret.
    pub credentials: Option<CredentialResolver>,
    /// A provider to use instead of constructing the configured one, for tests
    /// and development. The configured adapter kind is still validated.
    pub provider: Option<Arc<dyn Provider>>,
    /// How the production HTTP transport behaves on the wire.
    pub transport: TransportConfig,
    /// How long a platform credential lookup may wait for access.
    pub credential_timeout_ms: u64,
    /// Model-metadata layers below Smith's own configuration.
    pub catalog_sources: Vec<Arc<dyn ModelCatalogSource>>,
    /// The frozen normalized Models.dev snapshot, when the host loaded one.
    /// Supplies advertised reasoning controls for catalog-mapped endpoints;
    /// absence only removes that refinement.
    pub model_catalog: Option<Arc<smith_config::catalog::CatalogSnapshot>>,
    /// Fully resolved child-enabled profiles preflighted before child dispatch.
    pub child_profiles: Vec<ChildProfileRequest>,
    /// The runtime's event broadcast buffer.
    pub event_buffer: usize,
    /// The bounded-shutdown grace period, in milliseconds.
    pub shutdown_timeout_ms: u64,
}

impl RuntimeRequest {
    /// A request carrying `config`, presented by `surface`, with no host
    /// adapters injected yet.
    pub fn new(config: ResolvedConfig, surface: HostSurface) -> Self {
        Self {
            config,
            surface,
            system_prompt: None,
            project_instructions: None,
            workspace: None,
            approval: None,
            interaction: None,
            rotation: None,
            credential_pool: None,
            tools: Vec::new(),
            change_recorder: None,
            skills: crate::built_in_skills::built_in_sources(),
            memory: None,
            semantic_summary: None,
            built_in_tools: true,
            session_store: None,
            checkpoint_store: None,
            checkpoint_setup: None,
            checkpoint_barrier: None,
            secret_store: None,
            artifact_store: None,
            observers: Vec::new(),
            persistence_redactor: None,
            clock: None,
            credentials: None,
            provider: None,
            transport: TransportConfig::default(),
            credential_timeout_ms: DEFAULT_CREDENTIAL_TIMEOUT_MS,
            catalog_sources: Vec::new(),
            model_catalog: None,
            child_profiles: Vec::new(),
            event_buffer: DEFAULT_EVENT_BUFFER,
            shutdown_timeout_ms: DEFAULT_SHUTDOWN_TIMEOUT_MS,
        }
    }
}

/// One child-enabled profile resolved through the normal Smith configuration path.
#[derive(Debug, Clone)]
pub struct ChildProfileRequest {
    /// Profile-selected, provenance-carrying child configuration.
    pub config: ResolvedConfig,
    /// Catalog layers applicable to that profile's provider/model.
    pub catalog_sources: Vec<Arc<dyn ModelCatalogSource>>,
}

/// What one composition actually mapped onto the shared builder.
///
/// This is the evidence for "the TUI and `smith -p` run the same runtime": two
/// hosts that resolved the same configuration and injected the same adapters
/// produce equal policies, and a test can say so. It is also what a status line
/// or a run manifest reads, which is why it holds no adapter handles and no
/// secret — only values that are safe to display.
#[derive(Clone, PartialEq)]
pub struct RuntimePolicy {
    /// Active agent profile selected for this run.
    pub agent_profile: String,
    /// Deterministic effective agent-profile revision.
    pub agent_profile_revision: String,
    /// Placements allowed by the effective profile declaration.
    pub agent_profile_uses: Vec<ProfileUse>,
    /// Source of the effective authority posture, without instruction text.
    pub agent_profile_source: String,
    /// Whether the profile came from the transition-release legacy adapter.
    pub agent_profile_legacy: bool,
    /// Authority-narrowing behavior behind the selected mode name.
    pub agent_posture: AgentPosture,
    /// The provider name, as declared in `[providers.<name>]`.
    pub provider_name: String,
    /// The shared adapter kind it mapped to.
    pub provider_kind: String,
    /// The endpoint, normalized to scheme, host, port, and path.
    pub endpoint: Option<String>,
    /// The credential *reference*, never its value.
    pub credential: Option<String>,
    /// Resolved approval mode enforced for authority-bearing actions.
    pub approval_mode: ApprovalMode,
    /// The selected model.
    pub model: ModelId,
    /// The frozen profile every request is planned against.
    pub model_profile: ResolvedModelProfile,
    /// Exact controls and effective reasoning selection for this run.
    pub reasoning: ReasoningRuntimePolicy,
    /// The reserves and sub-budget planning enforces.
    pub context_policy: ContextPolicy,
    /// The semantic compaction thresholds derived from the enforced input
    /// budget.
    pub compaction_policy: CompactionPolicy,
    /// The product instructions sent as system content.
    pub system_prompt: String,
    /// Activated project-instruction source and exact revision, without body.
    pub project_instructions: Option<ProjectInstructionsIdentity>,
    /// Provider attempts allowed per request, including the first.
    pub max_attempts: u32,
    /// Tool calls allowed in one turn. `None` leaves the turn unbounded,
    /// ending when the model stops calling tools or another limit trips.
    pub max_tool_steps: Option<u32>,
    /// The wall-clock ceiling for one turn, in milliseconds. `None` leaves the
    /// turn without a wall-clock deadline.
    pub turn_time_limit_ms: Option<u64>,
    /// The model-facing tool output limit.
    pub output_limit: usize,
    /// The generation cap asked of the provider.
    pub max_output_tokens: Option<u32>,
    /// The registered tool names, in registration order.
    pub tools: Vec<String>,
    /// Resolved activatable skill names in deterministic order.
    pub skills: Vec<String>,
    /// Installed memory source revision.
    pub memory_revision: Option<RegistryRevision>,
    /// Semantic-summary model, purpose, spend, and retention policy.
    pub semantic_summary: Option<SemanticSummaryRuntimePolicy>,
    /// The runtime's event broadcast buffer.
    pub event_buffer: usize,
    /// The bounded-shutdown grace period, in milliseconds.
    pub shutdown_timeout_ms: u64,
    /// Whether exact protected mid-turn recovery was successfully installed.
    pub mid_turn_durability: MidTurnDurability,
}

impl fmt::Debug for RuntimePolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimePolicy")
            .field("agent_profile", &self.agent_profile)
            .field("agent_profile_revision", &self.agent_profile_revision)
            .field("agent_profile_uses", &self.agent_profile_uses)
            .field("agent_profile_source", &self.agent_profile_source)
            .field("agent_profile_legacy", &self.agent_profile_legacy)
            .field("agent_posture", &self.agent_posture)
            .field("provider_name", &self.provider_name)
            .field("provider_kind", &self.provider_kind)
            .field("endpoint", &self.endpoint)
            .field("credential", &self.credential)
            .field("approval_mode", &self.approval_mode)
            .field("model", &self.model)
            .field("model_profile", &self.model_profile)
            .field("reasoning", &self.reasoning)
            .field("context_policy", &self.context_policy)
            .field("compaction_policy", &self.compaction_policy)
            .field(
                "prompt_fragment_count",
                &self.system_prompt.matches("<smith-section ").count(),
            )
            .field("project_instructions", &self.project_instructions)
            .field("max_attempts", &self.max_attempts)
            .field("max_tool_steps", &self.max_tool_steps)
            .field("turn_time_limit_ms", &self.turn_time_limit_ms)
            .field("output_limit", &self.output_limit)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("tools", &self.tools)
            .field("skills", &self.skills)
            .field("memory_revision", &self.memory_revision)
            .field("semantic_summary", &self.semantic_summary)
            .field("event_buffer", &self.event_buffer)
            .field("shutdown_timeout_ms", &self.shutdown_timeout_ms)
            .field("mid_turn_durability", &self.mid_turn_durability)
            .finish()
    }
}

/// A built runtime, the policy it was built from, and the surface that asked.
#[derive(Debug, Clone)]
pub struct SmithRuntime {
    runtime: Runtime,
    policy: Arc<RuntimePolicy>,
    profile: Arc<ProfileResolution>,
    abilities: Arc<SealedAbilities>,
    skill_index: Arc<[SkillIndexEntry]>,
    checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    artifact_store: Option<Arc<dyn ArtifactStore>>,
    surface: HostSurface,
    delegation: Option<SmithDelegation>,
    goal_component: Option<Arc<GoalComponent>>,
}

impl SmithRuntime {
    /// The shared runtime. Cheap to clone; sessions start from it.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// What this composition mapped onto the shared builder.
    pub fn policy(&self) -> &RuntimePolicy {
        &self.policy
    }

    /// The resolved model profile together with every catalog layer that
    /// offered a limit, for configuration diagnostics.
    pub fn profile(&self) -> &ProfileResolution {
        &self.profile
    }

    /// The sealed descriptor-first view of every tool this composition owns.
    pub fn abilities(&self) -> &SealedAbilities {
        &self.abilities
    }

    /// Bounded Smith source index, including workspace metadata refused by
    /// trust policy.
    pub fn skill_index(&self) -> &[SkillIndexEntry] {
        &self.skill_index
    }

    /// The initialized exact checkpoint store, when protected durability is
    /// available for this composition.
    pub fn checkpoint_store(&self) -> Option<&Arc<dyn CheckpointStore>> {
        self.checkpoint_store.as_ref()
    }

    /// Protected artifact storage installed for this composition.
    pub fn artifact_store(&self) -> Option<&Arc<dyn ArtifactStore>> {
        self.artifact_store.as_ref()
    }

    /// Which presentation composed this runtime.
    pub fn surface(&self) -> HostSurface {
        self.surface
    }

    /// The delegation surface, when this composition registered the `agent`
    /// tool (root surfaces only — a child runtime never has one).
    pub fn delegation(&self) -> Option<&SmithDelegation> {
        self.delegation.as_ref()
    }

    /// Standard persistent-goal component for eligible root sessions.
    pub fn goal_component(&self) -> Option<&Arc<GoalComponent>> {
        self.goal_component.as_ref()
    }
}

/// Why a resolved configuration could not become a runtime.
///
/// Every variant names what the user has to change, and none can carry a
/// credential: the payloads are provider names, references, and classified
/// failures from types that redact themselves.
#[derive(Debug, thiserror::Error)]
pub enum FactoryError {
    /// Two tools attempted to register the same stable ability name.
    #[error("Smith could not seal its ability catalog: {0}")]
    AbilityRegistry(#[source] agent_runtime::registry::NameConflict),

    /// The configured adapter kind is not one the pinned runtime ships.
    #[error(
        "provider `{provider}` selects the `{kind}` adapter, which this build of Agent Runtime \
         does not ship; the available kinds are `{KIND_OPENAI_COMPATIBLE}`, \
         `{KIND_OPENAI_RESPONSES}`, `{KIND_ANTHROPIC_MESSAGES}`, \
         `{KIND_CHATGPT_RESPONSES}`, `{KIND_XAI_RESPONSES}`, `{KIND_GEMINI_INTERACTIONS}`, and \
         `{KIND_FAKE}`"
    )]
    AdapterUnavailable {
        /// The provider that selected it.
        provider: String,
        /// The adapter kind it selected.
        kind: String,
    },
    /// The configured endpoint cannot be used as written.
    #[error("provider `{provider}` has an unusable `base_url`: {message}")]
    Endpoint {
        /// The provider whose endpoint is unusable.
        provider: String,
        /// What is wrong with it. Never the URL itself, which may carry a key.
        message: String,
    },
    /// A configured credential is not a usable reference.
    #[error("provider `{provider}` has an unusable `credential`: {source}")]
    CredentialReference {
        /// The provider whose credential is unusable.
        provider: String,
        /// Why the reference could not be parsed.
        source: CredentialRefError,
    },
    /// A credential reference did not resolve to a secret.
    #[error(transparent)]
    Credential(#[from] CredentialError),
    /// The credential lookup did not finish.
    #[error("the provider credential lookup did not complete")]
    CredentialTask,
    /// The platform credential service did not answer within the startup
    /// boundary.
    #[error(
        "the provider credential lookup did not complete within {timeout_ms} ms; \
         unlock or allow the platform credential service, or use an `env:<VAR>` reference"
    )]
    CredentialTimeout {
        /// Configured lookup boundary.
        timeout_ms: u64,
    },
    /// Smith's protected ChatGPT token bundle or OAuth client is unusable.
    #[error("the experimental ChatGPT connection is unusable: {0}")]
    ChatGptAuth(#[source] crate::chatgpt::ChatGptAuthError),
    /// The stored xAI login could not be read or renewed.
    #[error("the configured xAI session is unusable")]
    XaiAuth(#[source] crate::xai::XaiAuthError),
    /// No layer supplied enforceable limits for the selected model.
    #[error(
        "provider `{provider}` cannot plan against model `{model}`: {source}. Declare \
         `[models.\"{provider}/{model}\"]` with `context_tokens`, `max_input_tokens`, and \
         `max_output_tokens`, or register a catalog source that does"
    )]
    ModelProfile {
        /// The provider serving the model.
        provider: String,
        /// The model that could not be resolved.
        model: ModelId,
        /// The shared resolver's structured failure.
        source: ModelProfileError,
    },
    /// A reasoning request cannot be represented by the exact binding.
    #[error("provider `{provider}` cannot apply reasoning controls to model `{model}`: {message}")]
    Reasoning {
        /// Serving provider identity.
        provider: String,
        /// Selected model.
        model: ModelId,
        /// Redaction-safe validation detail and alternatives.
        message: String,
    },
    /// The configured context reserves leave no room to plan in.
    #[error("the configured context reserves cannot be planned against: {message}")]
    ContextReserve {
        /// Which reserves conflict with which limits.
        message: String,
    },
    /// A host adapter the composition requires was not supplied.
    #[error("this run needs a {what}: {message}")]
    MissingHostPolicy {
        /// The adapter that is missing.
        what: &'static str,
        /// Why the run cannot proceed without it.
        message: String,
    },
    /// The production transport could not be built.
    #[error("the provider transport could not be built: {0}")]
    Transport(ProviderError),
    /// The shared runtime refused the composition.
    #[error("the shared runtime refused this composition: {0}")]
    Runtime(RuntimeError),
}

/// The shared adapter a configured provider kind maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Adapter {
    /// Agent Runtime's OpenAI-compatible Chat-Completions adapter.
    OpenAiCompatible,
    /// Agent Runtime's native stateless OpenAI Responses adapter.
    OpenAiResponses,
    /// Agent Runtime's native Anthropic Messages API adapter.
    AnthropicMessages,
    /// Smith's experimental direct ChatGPT Codex Responses adapter.
    ChatGptResponses,
    /// The Responses adapter driven by a renewable xAI browser login.
    XaiResponses,
    /// Agent Runtime's native stateless Gemini Interactions adapter.
    GeminiInteractions,
    /// Agent Runtime's deterministic fake.
    Fake,
}

/// Safe evidence that resolved configuration can pass the factory's
/// credential, adapter, endpoint, model-profile, and context-policy boundary.
///
/// Producing this value does not construct a provider transport, tool
/// registry, approval channel, runtime, session, observer, or journal. It is
/// intended for setup before the normal host is allowed to exist.
#[derive(Debug, Clone, PartialEq)]
pub struct FactoryPreflight {
    /// Reviewed provider identity.
    pub provider_name: String,
    /// Adapter kind shipped by this build.
    pub provider_kind: String,
    /// Normalized endpoint, when the adapter uses one.
    pub endpoint: Option<String>,
    /// Credential reference that successfully resolved, never its value.
    ///
    /// With a pool, this is the member preflight would start on.
    pub credential: Option<String>,
    /// Every declared pool member's reference, in declared order.
    ///
    /// References, never values. A single-credential provider reports a pool
    /// of one, so a consumer needs no separate case.
    pub credentials: Vec<String>,
    /// Selected model.
    pub model: ModelId,
    /// Immutable limits the eventual runtime will receive.
    pub model_profile: ResolvedModelProfile,
    /// Derived reserves the eventual runtime will receive.
    pub context_policy: ContextPolicy,
}

struct PreparedFactoryInputs {
    provider_name: String,
    provider_kind: String,
    adapter: Adapter,
    endpoint: Option<String>,
    secret: Option<Secret>,
    model: ModelId,
    profile: ProfileResolution,
    reasoning: ReasoningRuntimePolicy,
    context_policy: ContextPolicy,
    compaction_policy: CompactionPolicy,
    loop_config: LoopConfig,
}

/// Product context assembled before the runtime builder is configured.
struct PromptStage {
    project_instructions: Option<ProjectInstructionsSnapshot>,
    contributor: SmithPromptContributor,
    rendered: String,
    skills: ResolvedSmithSkills,
    memory: Option<MemoryContributor>,
}

/// Model-visible tools plus the stateful components wired around them.
struct CapabilityStage {
    tools: Vec<Arc<dyn Tool>>,
    abilities: SealedAbilities,
    todo: Option<Arc<TodoComponent>>,
    goal: Option<Arc<GoalComponent>>,
    delegation_slot:
        Option<Arc<std::sync::OnceLock<agent_runtime::delegation::DelegationCoordinator>>>,
}

/// Exact checkpoint state prepared independently of completed-turn storage.
struct DurabilityStage {
    root_store: Option<Arc<dyn CheckpointStore>>,
    child_store: Option<Arc<dyn CheckpointStore>>,
    status: MidTurnDurability,
}

type SummaryStage = Option<(
    Arc<SemanticSummaryCoordinator>,
    SemanticSummaryRuntimePolicy,
)>;

/// Display-safe evidence assembled before the runtime builder consumes policy.
struct PolicyStage {
    policy: RuntimePolicy,
}

/// The neutral runtime after the configured builder has accepted every hook.
struct BuilderStage {
    runtime: Runtime,
}

/// Root-only child delegation output assembled after the runtime exists.
struct DelegationStage {
    delegation: Option<SmithDelegation>,
}

fn assemble_policy(policy: RuntimePolicy) -> PolicyStage {
    PolicyStage { policy }
}

fn build_runtime(builder: RuntimeBuilder) -> Result<BuilderStage, FactoryError> {
    Ok(BuilderStage {
        runtime: builder.build().map_err(FactoryError::Runtime)?,
    })
}

fn assemble_delegation(delegation: Option<SmithDelegation>) -> DelegationStage {
    DelegationStage { delegation }
}

/// Validates setup through the same factory input derivation used by
/// [`build`], without crossing the runtime-construction boundary.
pub async fn preflight(request: &RuntimeRequest) -> Result<FactoryPreflight, FactoryError> {
    require_workspace(request)?;
    // Every member's reference is parsed before any provider I/O, not just the
    // one this session starts on. A pool whose second entry is malformed must
    // fail here, while the user is looking at setup — not in an hour, when the
    // first account is spent and rotation reaches for a reference that was
    // never going to work.
    validate_pool_references(request)?;
    let prepared = prepare_factory_inputs(request).await?;
    Ok(FactoryPreflight {
        provider_name: prepared.provider_name,
        provider_kind: prepared.provider_kind,
        endpoint: prepared.endpoint,
        credential: request
            .config
            .provider
            .credential()
            .map(|reference| reference.value.clone()),
        credentials: request
            .config
            .provider
            .credentials
            .iter()
            .map(|reference| reference.value.clone())
            .collect(),
        model: prepared.model,
        model_profile: prepared.profile.profile,
        context_policy: prepared.context_policy,
    })
}

fn prepare_prompt_stage(
    request: &RuntimeRequest,
    loop_config: &mut LoopConfig,
) -> Result<PromptStage, FactoryError> {
    let agent_profile = &request.config.agent.profile;
    let project_instructions = request
        .system_prompt
        .is_none()
        .then(|| request.project_instructions.clone())
        .flatten();
    let prompt_context = DynamicPromptContext {
        project_instructions: project_instructions.clone(),
        agent_profile: Some(AgentProfilePrompt {
            name: agent_profile.name.clone(),
            posture: request.config.agent.active_posture(),
            instructions: agent_profile
                .instructions
                .as_ref()
                .map(|instructions| instructions.value.clone()),
            revision: agent_profile.revision.clone(),
        }),
        todo_planning: todo_planning_eligible(request),
        questionnaire: questionnaire_eligible(request),
        delegation: delegation_eligible(request),
        ..DynamicPromptContext::default()
    };
    let contributor = match request.system_prompt.clone() {
        Some(prompt) => SmithPromptContributor::override_prompt(prompt),
        None => SmithPromptContributor::new(&prompt_context),
    };
    let skills = request.skills.resolve().map_err(FactoryError::Runtime)?;
    let memory = request
        .memory
        .clone()
        .map(|source| {
            let source: Arc<dyn MemorySource> = source;
            MemoryContributor::new(source)
        })
        .transpose()
        .map_err(FactoryError::Runtime)?;
    let rendered = render_fragments(contributor.fragments());

    // Product instructions enter the immutable context plan as independently
    // versioned fragments. Keeping this compatibility field populated would
    // send a second, unbudgeted copy through the legacy planner path.
    loop_config.system_prompt = None;

    Ok(PromptStage {
        project_instructions,
        contributor,
        rendered,
        skills,
        memory,
    })
}

fn prepare_provider_stage(
    request: &RuntimeRequest,
    adapter: Adapter,
    endpoint: Option<String>,
    secret: Option<Secret>,
    profile: &ResolvedModelProfile,
    reasoning: &ReasoningRuntimePolicy,
) -> Result<Arc<dyn Provider>, FactoryError> {
    if let (Some(secret), Some(redactor)) = (&secret, &request.persistence_redactor) {
        redactor.register_secret(secret);
    }
    // Built before the adapter so both halves share one pool: the credential
    // source reads the active member from it, and the rotation decorator
    // writes to it. Two pools would mean rotation changed a member the
    // adapter never consulted.
    let pool = credential_pool_for(request);
    let provider = match request.provider.clone() {
        Some(provider) => provider,
        None => construct(
            adapter,
            request,
            profile,
            endpoint,
            secret,
            &reasoning.efforts,
            pool.as_ref(),
        )?,
    };
    let provider = crate::response::apply_response_policy(
        provider,
        request
            .config
            .provider
            .response
            .reasoning_only
            .as_ref()
            .map(|policy| policy.value),
    );
    let provider = match reasoning.dialect {
        Some(dialect) => {
            Arc::new(ReasoningDialectProvider::new(provider, dialect)) as Arc<dyn Provider>
        }
        None => provider,
    };
    Ok(apply_credential_pool(request, provider, pool))
}

/// The credential source an adapter leases from.
///
/// With a pool this reads whichever member is active at the moment of the
/// attempt, which is what makes a rotation take effect without rebuilding the
/// adapter. The already-resolved secret seeds the active member's source so it
/// is not read from the credential service twice at startup.
fn credential_source(
    request: &RuntimeRequest,
    pool: Option<&SharedPool>,
    secret: Secret,
) -> Arc<dyn ProviderCredentialSource> {
    match (pool, request.credentials.clone()) {
        (Some(pool), Some(resolver)) => {
            let source = PoolCredentialSource::new(
                pool.clone(),
                Arc::new(StaticMemberSources::new(resolver)),
            );
            let active = pool.read(CredentialPool::active_position);
            source.seed(
                active,
                Arc::new(StaticProviderCredentialSource::new(secret)),
            );
            Arc::new(source) as Arc<dyn ProviderCredentialSource>
        }
        // No pool, or nothing to resolve one with: the session has a single
        // secret and it is already in hand.
        _ => Arc::new(StaticProviderCredentialSource::new(secret))
            as Arc<dyn ProviderCredentialSource>,
    }
}

/// The credential source for a browser-login adapter.
///
/// With a pool, each member is its own renewable source persisting to its own
/// reference; the already-built active source is seeded so the startup secret
/// is not read from the credential service twice. Without one, the active
/// source is the whole story — exactly the pre-pool behavior.
fn renewable_credential_source<B: RenewableBundle>(
    request: &RuntimeRequest,
    pool: Option<&SharedPool>,
    target: &ProviderCredentialTarget,
    refresher: Arc<dyn BundleRefresher<B>>,
    active: Arc<dyn ProviderCredentialSource>,
) -> Arc<dyn ProviderCredentialSource> {
    match (pool, request.credentials.clone()) {
        (Some(pool), Some(resolver)) => {
            let members = RenewableMemberSources::new(
                resolver,
                target.clone(),
                refresher,
                request.persistence_redactor.clone(),
            );
            let source = PoolCredentialSource::new(pool.clone(), Arc::new(members));
            let position = pool.read(CredentialPool::active_position);
            source.seed(position, active);
            Arc::new(source) as Arc<dyn ProviderCredentialSource>
        }
        _ => active,
    }
}

/// The reference whose secret authorizes this session's first attempt.
///
/// With a pool this is the *remembered active* member, not the first declared
/// one: a session resumed on the second account must authenticate as the
/// second account, not lease the first account's secret from the active
/// position until an invalidation happens to correct it.
fn active_credential_reference(request: &RuntimeRequest) -> Option<String> {
    if let Some(pool) = credential_pool_for(request)
        && let Some(reference) =
            pool.read(|pool| pool.active().map(|member| member.reference.clone()))
    {
        return Some(reference);
    }
    request
        .config
        .provider
        .credential()
        .map(|reference| reference.value.clone())
}

/// Builds the pool for this provider, or `None` when it declares one account.
fn credential_pool_for(request: &RuntimeRequest) -> Option<SharedPool> {
    if !request.config.provider.has_pool() {
        return None;
    }
    // A host-supplied pool is already seeded with the remembered account and
    // is the handle the surfaces draw from, so it wins over building a fresh
    // one that would silently start back at the first member.
    Some(request.credential_pool.clone().unwrap_or_else(|| {
        SharedPool::new(CredentialPool::new(
            request.config.provider.name.value.clone(),
            request
                .config
                .provider
                .credentials
                .iter()
                .map(|reference| reference.value.clone()),
            request
                .config
                .provider
                .rotate_at_percent
                .as_ref()
                .map(|threshold| threshold.value),
        ))
    }))
}

/// Wraps `provider` so a spent account can offer to move to another one.
///
/// Outermost on purpose. Rotation replays the whole attempt, so it has to sit
/// outside the response-policy and reasoning-dialect decorators — a replay
/// that skipped them would produce a differently normalized turn than the one
/// it replaced.
fn apply_credential_pool(
    request: &RuntimeRequest,
    provider: Arc<dyn Provider>,
    pool: Option<SharedPool>,
) -> Arc<dyn Provider> {
    // A provider with one account behaves exactly as it did before pools
    // existed: no wrapper, no offer, no extra state.
    let Some(pool) = pool else {
        return provider;
    };
    // No surface to ask means declining, which `HeadlessRotation` does while
    // recording the exhaustion for machine output.
    let policy = request
        .rotation
        .clone()
        .unwrap_or_else(|| Arc::new(HeadlessRotation::new()) as Arc<dyn RotationPolicy>);
    Arc::new(PooledProvider::new(
        provider,
        pool,
        policy,
        request
            .clock
            .clone()
            .unwrap_or_else(|| Arc::new(SystemClock) as Arc<dyn Clock>),
    )) as Arc<dyn Provider>
}

fn prepare_summary_stage(
    request: &RuntimeRequest,
    provider: Arc<dyn Provider>,
    provider_name: &str,
    model: &ModelId,
    limits: ModelLimits,
    clock: Arc<dyn Clock>,
) -> Result<SummaryStage, FactoryError> {
    let Some(config) = request.semantic_summary.clone() else {
        return Ok(None);
    };
    // The coordinator measures context pressure against a budget it cannot
    // discover for itself. Smith already refuses to guess a model's limits, so
    // the resolved input ceiling is the only honest source for it.
    let mut config = config;
    if config.policy.input_budget_tokens == 0 {
        config.policy.input_budget_tokens = u64::from(limits.max_input_tokens);
    }
    config.validate().map_err(FactoryError::Runtime)?;
    let store = request.artifact_store.clone().ok_or_else(|| {
        FactoryError::Runtime(RuntimeError::config(
            "semantic summaries require a protected artifact store for originals",
        ))
    })?;
    let summary_model: Arc<dyn SummaryModel> = match config.model.clone() {
        Some(model) => model,
        None => Arc::new(
            SmithProviderSummaryModel::new(
                provider,
                provider_name.to_owned(),
                model.clone(),
                clock,
                config.max_output_tokens,
                config.timeout_ms,
            )
            .map_err(FactoryError::Runtime)?,
        ),
    };
    let policy = SemanticSummaryRuntimePolicy {
        purpose: agent_runtime::harness::SEMANTIC_SUMMARY_PURPOSE.into(),
        model: summary_model.id().to_owned(),
        revision: config.policy.revision.clone(),
        min_turns: config.policy.min_turns,
        trigger_percent: config.policy.trigger_percent,
        input_budget_tokens: config.policy.input_budget_tokens,
        retain_turns: config.policy.retain_turns,
        max_usage_tokens: config.policy.max_usage_tokens,
        retention: config.policy.retention,
    };
    let coordinator = Arc::new(
        SemanticSummaryCoordinator::new(store, summary_model, config.policy)
            .map_err(FactoryError::Runtime)?,
    );
    Ok(Some((coordinator, policy)))
}

fn prepare_capability_stage(
    request: &RuntimeRequest,
    agent_tool_profiles: Vec<AgentToolProfile>,
) -> Result<CapabilityStage, FactoryError> {
    // The seam `shell`, `task_output`, and `task_stop` reach through to the
    // background task registry (`smith_tools::background`). Idempotent, like
    // the registry's own singleton: composing more than one runtime in this
    // process must not panic, and every session reaches the same registry
    // regardless of which composition installed the adapter first.
    smith_tools::background::install(std::sync::Arc::new(RegistryBackgroundTaskHost));

    let mut tools = tools(request);
    // No tool, no projected plan state: a posture that cannot write a plan
    // should not carry one in its context either.
    let todo = todo_planning_eligible(request).then(|| Arc::new(TodoComponent::public()));
    let goal = goal_component_eligible(request).then(|| Arc::new(GoalComponent::public()));
    if goal.is_some() {
        tools.extend([
            Arc::new(GetGoalTool::new()) as Arc<dyn Tool>,
            Arc::new(CreateGoalTool::new()) as Arc<dyn Tool>,
            Arc::new(UpdateGoalTool::new()) as Arc<dyn Tool>,
        ]);
    }
    let host_tool_names = request
        .tools
        .iter()
        .map(|tool| tool.spec().name)
        .collect::<BTreeSet<_>>();
    let mut ability_sources = tools
        .iter()
        .map(|tool| {
            if host_tool_names.contains(&tool.spec().name) {
                agent_runtime::registry::RegistrySource::Host
            } else {
                agent_runtime::registry::RegistrySource::BuiltIn
            }
        })
        .collect::<Vec<_>>();

    let delegation_slot = if matches!(request.surface, HostSurface::Child) {
        None
    } else {
        let slot = Arc::new(std::sync::OnceLock::new());
        tools.push(
            Arc::new(AgentTool::new(slot.clone()).with_profiles(agent_tool_profiles))
                as Arc<dyn Tool>,
        );
        ability_sources.push(agent_runtime::registry::RegistrySource::BuiltIn);
        Some(slot)
    };
    let abilities = seal_tool_abilities(tools.iter().cloned().zip(ability_sources))
        .map_err(FactoryError::AbilityRegistry)?;

    Ok(CapabilityStage {
        tools,
        abilities,
        todo,
        goal,
        delegation_slot,
    })
}

async fn prepare_durability_stage(
    request: &RuntimeRequest,
) -> Result<DurabilityStage, FactoryError> {
    if request.checkpoint_store.is_some() && request.checkpoint_setup.is_some() {
        return Err(FactoryError::Runtime(RuntimeError::config(
            "checkpoint_store and checkpoint_setup cannot both be supplied",
        )));
    }
    let store = match (
        request.checkpoint_store.clone(),
        request.checkpoint_setup.as_ref(),
    ) {
        (Some(store), None) => Some(store),
        (None, Some(setup)) => match setup.initialize().await {
            Ok(store) => Some(store),
            Err(error) => {
                tracing::warn!(
                    schema_version = CHECKPOINT_SCHEMA_VERSION,
                    %error,
                    "exact mid-turn durability is unavailable; completed-turn persistence remains enabled"
                );
                None
            }
        },
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!("conflicting checkpoint inputs returned above"),
    };
    // Child sessions share the protected store, never the root journal
    // barrier, which is scoped to the root writer.
    let child_store = store.clone();
    let root_store = match (store, request.checkpoint_barrier.clone()) {
        (Some(store), Some(barrier)) => {
            Some(Arc::new(BarrierCheckpointStore::new(store, barrier)) as Arc<dyn CheckpointStore>)
        }
        (store, None) => store,
        (None, Some(_)) => None,
    };
    let status = if root_store.is_some() {
        MidTurnDurability::Available
    } else {
        MidTurnDurability::Unavailable
    };
    Ok(DurabilityStage {
        root_store,
        child_store,
        status,
    })
}

/// Builds the runtime one resolved Smith run needs.
///
/// Async because credential resolution is: the platform credential service is
/// synchronous and may block on an unlock prompt, so it runs on a blocking
/// thread rather than on the executor a provider stream will share.
pub async fn build(request: RuntimeRequest) -> Result<SmithRuntime, FactoryError> {
    // Host policy first. It costs nothing to check and everything to get wrong,
    // and failing here means a misconfigured run never reaches a keychain.
    let workspace = require_workspace(&request)?;
    let approval = approval(&request)?;
    let PreparedFactoryInputs {
        provider_name,
        provider_kind,
        adapter,
        endpoint,
        secret,
        model,
        profile,
        reasoning,
        context_policy,
        compaction_policy,
        mut loop_config,
    } = prepare_factory_inputs(&request).await?;
    let agent_posture = request.config.agent.active_posture();
    let agent_profile = request.config.agent.profile.clone();
    let agent_profile_name = agent_profile.name.clone();
    let prompt = prepare_prompt_stage(&request, &mut loop_config)?;
    let config = &request.config;

    // The only boundary the secret crosses.
    let provider = prepare_provider_stage(
        &request,
        adapter,
        endpoint.clone(),
        secret,
        &profile.profile,
        &reasoning,
    )?;

    let clock: Arc<dyn Clock> = request
        .clock
        .clone()
        .unwrap_or_else(|| Arc::new(SystemClock));
    let child_profile_routes =
        prepare_child_profile_routes(&request, prompt.project_instructions.as_ref()).await?;
    // Built from the exact same routes `SmithChildFactory.profile_routes`
    // resolves below, so the model-facing `agent` tool can never advertise or
    // accept a profile name the factory would fail to route.
    let agent_tool_profiles: Vec<AgentToolProfile> = child_profile_routes
        .values()
        .map(|route| AgentToolProfile {
            name: route.agent_profile_name.clone(),
            revision: route.agent_profile_revision.clone(),
            provider: route.provider_name.clone(),
            model: route.model.clone(),
        })
        .collect();
    let semantic_summary = prepare_summary_stage(
        &request,
        provider.clone(),
        &provider_name,
        &model,
        profile.profile.limits,
        clock.clone(),
    )?;
    // Warns the model before the compaction boundary. Only meaningful when
    // summarization is configured — without it there is no boundary to warn
    // about, only the structural watermarks, which reclaim silently.
    let budget_notice = semantic_summary.as_ref().and_then(|_| {
        BudgetNoticeComponent::new(
            u64::from(profile.profile.limits.max_input_tokens),
            DEFAULT_NOTICE_THRESHOLD_TOKENS,
        )
        .ok()
        .map(Arc::new)
    });
    let capabilities = prepare_capability_stage(&request, agent_tool_profiles)?;
    let durability = prepare_durability_stage(&request).await?;

    let policy = assemble_policy(RuntimePolicy {
        agent_profile: agent_profile_name.clone(),
        agent_profile_revision: agent_profile.revision.clone(),
        agent_profile_uses: agent_profile.uses.value.clone(),
        agent_profile_source: agent_profile.posture.source.to_string(),
        agent_profile_legacy: agent_profile.legacy,
        agent_posture,
        provider_name: provider_name.clone(),
        provider_kind: provider_kind.clone(),
        endpoint,
        credential: config
            .provider
            .credential()
            .map(|reference| reference.value.clone()),
        approval_mode: config.approval.mode.value,
        model: model.clone(),
        model_profile: profile.profile.clone(),
        reasoning: reasoning.clone(),
        context_policy: context_policy.clone(),
        compaction_policy: compaction_policy.clone(),
        system_prompt: prompt.rendered.clone(),
        project_instructions: prompt
            .project_instructions
            .as_ref()
            .map(ProjectInstructionsSnapshot::identity),
        max_attempts: loop_config.retry.max_attempts,
        max_tool_steps: loop_config.max_tool_steps,
        turn_time_limit_ms: loop_config.turn_time_limit_ms,
        output_limit: loop_config.output_limit,
        max_output_tokens: loop_config.max_output_tokens,
        tools: capabilities
            .tools
            .iter()
            .map(|tool| tool.spec().name)
            .collect(),
        skills: prompt
            .skills
            .abilities()
            .iter()
            .map(|ability| ability.name().to_owned())
            .collect(),
        memory_revision: request.memory.as_ref().map(|source| source.revision()),
        semantic_summary: semantic_summary.as_ref().map(|(_, policy)| policy.clone()),
        event_buffer: request.event_buffer,
        shutdown_timeout_ms: request.shutdown_timeout_ms,
        mid_turn_durability: durability.status,
    });

    let tool_authority = Arc::new(SmithToolAuthority::new(workspace.root()));
    let tool_coverage = tool_authority.coverage().clone();
    let interaction_ready = !matches!(request.surface, HostSurface::Child)
        && request
            .interaction
            .as_ref()
            .is_some_and(|broker| broker.readiness() == InteractionReadiness::Ready);
    let activation_context = if interaction_ready {
        ActivationContext::new().with_ready_config([INTERACTION_READY_CONFIG])
    } else {
        ActivationContext::new()
    };
    let scope_inputs = ScopeInputs::new().with_identity(
        ScopeIdentity::new()
            .with_workspace(workspace.root())
            // Terminal/headless/embedded are projections over one Smith
            // agent policy and must derive the same canonical view. Only a
            // delegated child has a distinct execution identity.
            .with_agent(if matches!(request.surface, HostSurface::Child) {
                "smith-child"
            } else {
                agent_profile_name.as_str()
            }),
    );
    let activation_budget = ActivationBudget::new(
        ContextBudget::from_limits(&profile.profile.limits, &context_policy).capability_budget,
        8,
    );
    let mut builder = RuntimeBuilder::new(model.clone())
        .provider(provider.clone())
        .provider_name(provider_name.clone())
        // The profile rather than the catalog: the run must plan against
        // exactly the limits validated above, and passing a catalog as well
        // would be dead weight because an explicit profile always outranks it.
        .model_profile(profile.profile.clone())
        .loop_config(loop_config.clone())
        .model_interceptor(Arc::new(ReasoningInterceptor::new(&reasoning)))
        .context_policy(context_policy.clone())
        .compactor(StructuralCompactor::new(compaction_policy))
        // Declared explicitly so the shared planner records Smith's answer
        // rather than its own "unspecified" placeholder in plan fingerprints.
        // The answer is the adapter's own declaration: an implicit-prefix
        // provider caches the stable run, and reporting `none` here made
        // every plan claim the provider could reuse nothing.
        .cache_capability(ProviderCacheCapability::from_control(
            RegistryRevision::new(CACHE_CAPABILITY_REVISION),
            provider_kind.clone(),
            provider
                .capabilities(&model)
                .map(|capabilities| capabilities.prompt_cache)
                .unwrap_or_default(),
        ))
        .security_check(
            tool_authority,
            SecurityCheckMode::Authoritative,
            tool_coverage,
            ActionClass::new("smith-built-in-tools"),
        )
        .approval(approval.clone())
        .workspace(workspace.clone())
        .tools(capabilities.tools.clone())
        .live_ability_routing()
        .scope_inputs(scope_inputs)
        .capability_resolver(Arc::new(CapabilityResolver::new()))
        .activation_policy(Arc::new(FailClosedPolicy))
        .activation_context(activation_context)
        .activation_budget(activation_budget)
        .context_contributor(Arc::new(prompt.contributor.clone()))
        .clock(clock.clone())
        .event_buffer(request.event_buffer)
        .shutdown_timeout_ms(request.shutdown_timeout_ms);
    if let Some(component) = &capabilities.todo {
        builder = builder
            .context_contributor(component.clone())
            .tool_output_processor(component.clone())
            .turn_commit_hook(component.clone());
    }
    if let Some(component) = &capabilities.goal {
        builder = builder
            .context_contributor(component.clone())
            .model_interceptor(component.clone())
            .tool_output_processor(component.clone())
            .turn_commit_hook(component.clone());
    }
    if let Some(contributor) = prompt.memory.clone() {
        builder = builder.context_contributor(Arc::new(contributor));
    }
    if let Some((coordinator, _)) = &semantic_summary {
        builder = builder
            .history_projector(coordinator.clone())
            .turn_commit_hook(coordinator.clone());
    }
    if let Some(component) = &budget_notice {
        builder = builder
            .context_contributor(component.clone())
            .turn_commit_hook(component.clone());
    }
    for descriptor in capabilities.abilities.descriptors() {
        builder = builder.tool_ability_descriptor(descriptor);
    }
    for skill in prompt.skills.abilities().iter().cloned() {
        builder = builder.ability(skill);
    }
    if let Some(interaction) = request.interaction.clone() {
        builder = builder.interaction_broker(interaction);
    }
    if capabilities.delegation_slot.is_some() {
        let authority = Arc::new(DelegationAuthority::new());
        let coverage = authority.coverage().clone();
        builder = builder.security_check(
            authority,
            SecurityCheckMode::Authoritative,
            coverage,
            ActionClass::new("smith-delegation"),
        );
    }
    if let Some(store) = request.session_store.clone() {
        builder = builder.session_store(store);
    }
    if let Some(store) = durability.root_store.clone() {
        builder = builder.checkpoint_store(store);
    }
    if let Some(store) = request.secret_store.clone() {
        builder = builder.secret_store(store);
    }
    if let Some(store) = request.artifact_store.clone() {
        let offloader = ArtifactOffloader::new(store)
            .with_threshold_bytes(loop_config.output_limit)
            .map_err(FactoryError::Runtime)?;
        builder = builder.tool_output_processor(Arc::new(offloader));
    }
    for observer in request.observers.iter().cloned() {
        builder = builder.observer(observer);
    }

    let built = build_runtime(builder)?;
    let delegation = assemble_delegation(capabilities.delegation_slot.clone().map(|slot| {
        SmithDelegation {
            factory: Arc::new(SmithChildFactory {
                default_route: SmithChildRoute {
                    provider,
                    provider_name,
                    provider_kind,
                    model,
                    model_profile: profile.profile.clone(),
                    context_policy,
                    loop_config,
                    prompt_contributor: prompt.contributor.clone(),
                    agent_profile_name: agent_profile.name.clone(),
                    agent_profile_revision: agent_profile.revision.clone(),
                    agent_profile_posture: agent_profile.posture.value,
                    read_only: agent_profile.posture.value.is_read_only(),
                },
                profile_routes: child_profile_routes,
                approval,
                workspace,
                clock,
                artifact_store: request.artifact_store.clone(),
                session_store: request.session_store.clone(),
                checkpoint_store: durability.child_store,
                skills: prompt.skills.abilities().to_vec(),
                memory: prompt.memory,
                semantic_summary: semantic_summary
                    .as_ref()
                    .map(|(coordinator, _)| coordinator.clone()),
            }),
            slot,
        }
    }));
    Ok(SmithRuntime {
        runtime: built.runtime,
        policy: Arc::new(policy.policy),
        profile: Arc::new(profile),
        abilities: Arc::new(capabilities.abilities),
        skill_index: Arc::from(prompt.skills.index().to_vec().into_boxed_slice()),
        checkpoint_store: durability.root_store,
        artifact_store: request.artifact_store,
        surface: request.surface,
        delegation: delegation.delegation,
        goal_component: capabilities.goal,
    })
}

async fn prepare_child_profile_routes(
    request: &RuntimeRequest,
    project_instructions: Option<&ProjectInstructionsSnapshot>,
) -> Result<BTreeMap<String, SmithChildRoute>, FactoryError> {
    let mut routes = BTreeMap::new();
    for child in &request.child_profiles {
        let mut route_request = RuntimeRequest::new(child.config.clone(), HostSurface::Child);
        route_request.project_instructions = project_instructions.cloned();
        route_request.workspace = request.workspace.clone();
        route_request.approval = request.approval.clone();
        route_request.credentials = request.credentials.clone();
        route_request.transport = request.transport.clone();
        route_request.credential_timeout_ms = request.credential_timeout_ms;
        route_request.catalog_sources = child.catalog_sources.clone();
        route_request.model_catalog = request.model_catalog.clone();
        route_request.persistence_redactor = request.persistence_redactor.clone();

        let PreparedFactoryInputs {
            provider_name,
            provider_kind,
            adapter,
            endpoint,
            secret,
            model,
            profile,
            reasoning,
            context_policy,
            compaction_policy: _,
            mut loop_config,
        } = prepare_factory_inputs(&route_request).await?;
        if let (Some(secret), Some(redactor)) = (&secret, &route_request.persistence_redactor) {
            redactor.register_secret(secret);
        }
        // This is a per-route rebuild, which starts from the route's own
        // configuration rather than the session's live pool.
        let route_pool = credential_pool_for(&route_request);
        let provider = construct(
            adapter,
            &route_request,
            &profile.profile,
            endpoint,
            secret,
            &reasoning.efforts,
            route_pool.as_ref(),
        )?;
        let provider = crate::response::apply_response_policy(
            provider,
            route_request
                .config
                .provider
                .response
                .reasoning_only
                .as_ref()
                .map(|policy| policy.value),
        );
        let provider = match reasoning.dialect {
            Some(dialect) => {
                Arc::new(ReasoningDialectProvider::new(provider, dialect)) as Arc<dyn Provider>
            }
            None => provider,
        };
        let agent_profile = &route_request.config.agent.profile;
        let prompt_context = DynamicPromptContext {
            project_instructions: project_instructions.cloned(),
            agent_profile: Some(AgentProfilePrompt {
                name: agent_profile.name.clone(),
                posture: agent_profile.posture.value,
                instructions: agent_profile
                    .instructions
                    .as_ref()
                    .map(|instructions| instructions.value.clone()),
                revision: agent_profile.revision.clone(),
            }),
            // A child always runs on `HostSurface::Child`, so it registers
            // neither the root questionnaire nor a delegation tool of its own.
            todo_planning: !agent_profile.posture.value.is_read_only(),
            questionnaire: false,
            delegation: false,
            ..DynamicPromptContext::default()
        };
        let prompt_contributor = SmithPromptContributor::new(&prompt_context);
        loop_config.system_prompt = None;
        let route_key =
            crate::delegation::profile_route_key(&agent_profile.name, &agent_profile.revision);
        let replaced = routes.insert(
            route_key.clone(),
            SmithChildRoute {
                provider,
                provider_name,
                provider_kind,
                model,
                model_profile: profile.profile,
                context_policy,
                loop_config,
                prompt_contributor,
                agent_profile_name: agent_profile.name.clone(),
                agent_profile_revision: agent_profile.revision.clone(),
                agent_profile_posture: agent_profile.posture.value,
                read_only: agent_profile.posture.value.is_read_only(),
            },
        );
        if replaced.is_some() {
            return Err(FactoryError::Runtime(RuntimeError::conflict(format!(
                "duplicate child profile route `{route_key}`"
            ))));
        }
    }
    Ok(routes)
}

fn require_workspace(request: &RuntimeRequest) -> Result<Arc<dyn Workspace>, FactoryError> {
    request
        .workspace
        .clone()
        .ok_or_else(|| FactoryError::MissingHostPolicy {
            what: "workspace",
            message: "tools resolve every path through it, so a run without one could only \
                      deny each call with no explanation"
                .to_owned(),
        })
}

async fn prepare_factory_inputs(
    request: &RuntimeRequest,
) -> Result<PreparedFactoryInputs, FactoryError> {
    let config = &request.config;
    let provider_name = config.provider.name.value.clone();
    let provider_kind = config.provider.kind.value.clone();
    let model = ModelId::new(config.model.value.clone());

    // An adapter this build does not ship is never routed through a different
    // wire protocol.
    let adapter = adapter(&config.provider)?;
    let endpoint = match adapter {
        Adapter::OpenAiCompatible => Some(endpoint(&config.provider, None)?),
        // Generic over the Responses protocol, so there is no default to fall
        // back to: the endpoint names which deployment is being talked to.
        Adapter::OpenAiResponses => Some(endpoint(&config.provider, None)?),
        Adapter::AnthropicMessages => Some(endpoint(
            &config.provider,
            Some(smith_config::model::ANTHROPIC_DEFAULT_ENDPOINT),
        )?),
        Adapter::ChatGptResponses => Some(endpoint(
            &config.provider,
            Some(smith_config::setup::CHATGPT_ENDPOINT),
        )?),
        Adapter::XaiResponses => Some(endpoint(
            &config.provider,
            Some(smith_config::setup::XAI_ENDPOINT),
        )?),
        Adapter::GeminiInteractions => Some(smith_config::catalog::GEMINI_ENDPOINT.to_owned()),
        Adapter::Fake => None,
    };

    let mut layers = CatalogLayers::new(provider_name.clone(), model.clone())
        .with_sources(request.catalog_sources.iter().map(Arc::clone));
    if let Some(trusted) = trusted_model(&provider_name, model.as_str()) {
        layers = layers.with_embedded(ModelRecord::new().with_limits(ModelLimits::new(
            trusted.context_tokens,
            trusted.max_input_tokens,
            trusted.max_output_tokens,
        )));
    }
    let mut profile = layers
        .with_configured_limits(&config.model_limits)
        .resolve()
        .map_err(|source| FactoryError::ModelProfile {
            provider: provider_name.clone(),
            model: model.clone(),
            source,
        })?;
    let catalog_controls = request.model_catalog.as_deref().and_then(|snapshot| {
        let catalog_provider =
            smith_config::catalog::catalog_provider_for(&provider_kind, endpoint.as_deref())?;
        snapshot
            .provider(catalog_provider)?
            .models
            .get(model.as_str())?
            .reasoning_controls
            .as_ref()
    });
    let reasoning = resolve_reasoning_policy(
        config,
        &profile.profile,
        endpoint.as_deref(),
        catalog_controls,
    )
    .map_err(|message| FactoryError::Reasoning {
        provider: provider_name.clone(),
        model: model.clone(),
        message,
    })?;
    if reasoning.support == agent_runtime_core::provider::ReasoningSupport::Controllable {
        profile.profile.capabilities.reasoning =
            agent_runtime_core::provider::ReasoningSupport::Controllable;
    }

    // Capability and requested-value validation precede credential lookup, so
    // an invalid effort never opens a keychain prompt.
    let secret = match (
        &request.provider,
        active_credential_reference(request),
        &config.provider.api_key,
    ) {
        (None, _, Some(api_key)) => Some(api_key.value.clone()),
        (None, Some(reference), None) => Some(secret(request, &reference).await?),
        _ => None,
    };
    if adapter == Adapter::ChatGptResponses {
        let secret = secret.as_ref().ok_or(FactoryError::ChatGptAuth(
            crate::chatgpt::ChatGptAuthError::InvalidBundle,
        ))?;
        ChatGptTokenBundle::from_secret(secret).map_err(FactoryError::ChatGptAuth)?;
    }
    // Rejected here rather than at the first model call, so a login that never
    // completed is reported while the user is still in setup.
    if adapter == Adapter::XaiResponses {
        let secret = secret.as_ref().ok_or(FactoryError::XaiAuth(
            crate::xai::XaiAuthError::InvalidBundle,
        ))?;
        XaiTokenBundle::from_secret(secret).map_err(FactoryError::XaiAuth)?;
    }
    // A stored login on the generic Responses kind is the one shape that fails
    // silently: the adapter would send the whole bundle as the bearer and the
    // endpoint would answer "incorrect API key", pointing the user at their
    // key when the fault is the adapter. No real API key parses as a bundle,
    // so this cannot catch a working configuration.
    if adapter == Adapter::OpenAiResponses
        && let Some(secret) = secret.as_ref()
        && XaiTokenBundle::from_secret(secret).is_ok()
    {
        return Err(FactoryError::Runtime(RuntimeError::config(format!(
            "provider `{}` stores a browser login but uses the `{KIND_OPENAI_RESPONSES}` adapter, \
             which sends its credential verbatim; change its `kind` to `{KIND_XAI_RESPONSES}`",
            request.config.provider.name.value
        ))));
    }
    let context_policy = context_policy(config, &profile.profile)?;
    let compaction_policy = compaction_policy(config, &profile.profile, &context_policy);
    let mut loop_config = loop_config(request, &model);
    loop_config.reasoning = reasoning.request_config();

    Ok(PreparedFactoryInputs {
        provider_name,
        provider_kind,
        adapter,
        endpoint,
        secret,
        model,
        profile,
        reasoning,
        context_policy,
        compaction_policy,
        loop_config,
    })
}

/// Maps a configured provider kind onto a shared adapter.
///
/// The check runs even when a provider is injected: which adapters exist is a
/// property of the pinned runtime, and a profile naming one it does not have is
/// a configuration error whether or not this particular run would have used it.
fn adapter(provider: &ResolvedProvider) -> Result<Adapter, FactoryError> {
    match provider.kind.value.as_str() {
        KIND_OPENAI_COMPATIBLE => Ok(Adapter::OpenAiCompatible),
        KIND_OPENAI_RESPONSES => Ok(Adapter::OpenAiResponses),
        KIND_ANTHROPIC_MESSAGES => Ok(Adapter::AnthropicMessages),
        KIND_CHATGPT_RESPONSES => Ok(Adapter::ChatGptResponses),
        KIND_XAI_RESPONSES => Ok(Adapter::XaiResponses),
        KIND_GEMINI_INTERACTIONS => Ok(Adapter::GeminiInteractions),
        KIND_FAKE => Ok(Adapter::Fake),
        kind => Err(FactoryError::AdapterUnavailable {
            provider: provider.name.value.clone(),
            kind: kind.to_owned(),
        }),
    }
}

/// Validates the configured endpoint and normalizes it for the adapter.
///
/// `default` supplies the endpoint for adapters whose wire protocol has one
/// well-known home (the Anthropic Messages API); without it, a missing
/// `base_url` is a configuration error.
///
/// No message repeats the URL. A base URL is the other place a key is known to
/// be pasted — as userinfo or as a query parameter — and both are refused here
/// rather than forwarded, because a credential in a URL ends up in a log the
/// moment anything prints the request target.
fn endpoint(provider: &ResolvedProvider, default: Option<&str>) -> Result<String, FactoryError> {
    let refuse = |message: &str| FactoryError::Endpoint {
        provider: provider.name.value.clone(),
        message: message.to_owned(),
    };
    let configured = match (&provider.base_url, default) {
        (Some(configured), _) => configured,
        (None, Some(default)) => return Ok(default.trim_end_matches('/').to_owned()),
        (None, None) => {
            return Err(refuse("the provider needs the endpoint it talks to"));
        }
    };

    let url = Url::parse(&configured.value).map_err(|_| refuse("it is not an absolute URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(refuse("only `http` and `https` endpoints are supported"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refuse(
            "it carries credentials in the URL; move them to the provider's `credential` reference",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(refuse(
            "it must be a plain endpoint; per-request options belong in `headers` and secrets in \
             `credential`",
        ));
    }
    let host = url.host_str().ok_or_else(|| refuse("it names no host"))?;

    let mut endpoint = format!("{}://{host}", url.scheme());
    if let Some(port) = url.port() {
        endpoint.push(':');
        endpoint.push_str(&port.to_string());
    }
    // The adapter appends its own path segment, so a trailing slash here would
    // produce `…/v1//chat/completions`.
    endpoint.push_str(url.path().trim_end_matches('/'));
    Ok(endpoint)
}

/// Resolves a configured credential reference into a secret.
///
/// The resolver's backend is synchronous and may wait on an unlock prompt. A
/// dedicated thread keeps that wait off the executor; a bounded async receive
/// lets startup fail actionably even if the platform call itself cannot be
/// cancelled.
/// Parses every declared pool member's reference, resolving none of them.
///
/// Parsing is free and offline; resolving opens the credential service and can
/// prompt. Checking shape for all members while reading the value of only the
/// active one is what lets a misconfigured pool fail early without turning
/// startup into one keychain prompt per account.
fn validate_pool_references(request: &RuntimeRequest) -> Result<(), FactoryError> {
    for reference in &request.config.provider.credentials {
        CredentialRef::parse(&reference.value).map_err(|source| {
            FactoryError::CredentialReference {
                provider: request.config.provider.name.value.clone(),
                source,
            }
        })?;
    }
    Ok(())
}

async fn secret(request: &RuntimeRequest, reference: &str) -> Result<Secret, FactoryError> {
    let provider = request.config.provider.name.value.clone();
    let reference =
        CredentialRef::parse(reference).map_err(|source| FactoryError::CredentialReference {
            provider: provider.clone(),
            source,
        })?;
    let resolver = request
        .credentials
        .clone()
        .ok_or_else(|| FactoryError::MissingHostPolicy {
            what: "credential resolver",
            message: format!(
                "provider `{provider}` configures the credential `{reference}`, and nothing was \
                 supplied that can resolve it"
            ),
        })?;

    let timeout_ms = request.credential_timeout_ms.max(1);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("smith-credential-lookup".into())
        .spawn(move || {
            let _ = sender.send(resolver.resolve_blocking(&reference));
        })
        .map_err(|_| FactoryError::CredentialTask)?;

    match tokio::time::timeout(Duration::from_millis(timeout_ms), receiver).await {
        Ok(Ok(result)) => result.map_err(FactoryError::Credential),
        Ok(Err(_)) => Err(FactoryError::CredentialTask),
        Err(_) => Err(FactoryError::CredentialTimeout { timeout_ms }),
    }
}

/// Reads every pool member's server-reported usage once at session start and
/// records it in the credential pool, so the account surfaces can show a
/// measured number for each account before it has served an attempt.
///
/// The active member reuses the source already built for the session; the
/// others are built from their references, which for this provider are
/// `authfile:` entries that resolve without prompting. Members run
/// sequentially so two token refreshes never race on the auth file.
///
/// Best-effort by design: a failure leaves that member unmeasured, and the
/// surfaces already render an unmeasured member honestly as "usage unknown".
/// Requires a running Tokio runtime; without one there is no session about to
/// serve attempts, so there is nothing to probe for.
fn spawn_chatgpt_usage_probe(
    pool: SharedPool,
    members: Option<Arc<dyn PoolMemberSources>>,
    active_source: Arc<dyn ProviderCredentialSource>,
    target: ProviderCredentialTarget,
    active_account: String,
) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        let (active_position, roster) = pool.read(|pool| {
            (
                pool.active_position(),
                pool.members()
                    .iter()
                    .map(|member| (member.position, member.reference.clone()))
                    .collect::<Vec<_>>(),
            )
        });
        for (position, reference) in roster {
            let source = if position == active_position {
                active_source.clone()
            } else if let Some(members) = &members {
                match members.build(position, &reference).await {
                    Ok(source) => source,
                    Err(_) => continue,
                }
            } else {
                continue;
            };
            let Ok(lease) = source
                .acquire(
                    &target,
                    crate::chatgpt::CHATGPT_CREDENTIAL_MINIMUM_VALIDITY_MS,
                    &Cancellation::new(),
                    Deadline::never(),
                )
                .await
            else {
                continue;
            };
            // Only the active member may fall back to the account identity
            // frozen at construction: pairing another member's token with it
            // would query the wrong account's usage.
            let account = match (lease.account(), position == active_position) {
                (Some(account), _) => account.to_owned(),
                (None, true) => active_account.clone(),
                (None, false) => continue,
            };
            if let Ok(snapshot) =
                crate::chatgpt::fetch_usage_snapshot(lease.secret().expose(), &account).await
            {
                pool.write(|pool| pool.record_snapshot(position, snapshot));
            }
        }
    });
}

/// Constructs the configured provider.
fn construct(
    adapter: Adapter,
    request: &RuntimeRequest,
    profile: &ResolvedModelProfile,
    endpoint: Option<String>,
    secret: Option<Secret>,
    supported_thinking_levels: &[String],
    pool: Option<&SharedPool>,
) -> Result<Arc<dyn Provider>, FactoryError> {
    match adapter {
        Adapter::Fake => Ok(Arc::new(FakeProvider::text_reply(DEVELOPMENT_REPLY))),
        Adapter::OpenAiCompatible => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let mut config = OpenAiConfig::new(
                endpoint.unwrap_or_default(),
                request.config.model.value.clone(),
            );
            // The resolved profile governs request validation, so the adapter
            // is told what the profile declared rather than a provider-wide
            // guess about every model the endpoint might serve.
            config.capabilities = profile.capabilities.clone();
            config.extra_headers = request
                .config
                .provider
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.value.clone()))
                .collect();
            match secret {
                Some(secret) => {
                    let target =
                        ProviderCredentialTarget::new(request.config.provider.name.value.clone())
                            .map_err(|error| {
                            FactoryError::Runtime(RuntimeError::config(error.to_string()))
                        })?;
                    let source = credential_source(request, pool, secret);
                    let provider =
                        OpenAiProvider::with_credential_source(transport, config, target, source)
                            .map_err(FactoryError::Transport)?;
                    Ok(Arc::new(provider))
                }
                None => Ok(Arc::new(OpenAiProvider::new(transport, config))),
            }
        }
        Adapter::OpenAiResponses => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let mut config = ResponsesConfig::new(
                endpoint.unwrap_or_default(),
                request.config.model.value.clone(),
            );
            // The resolved profile governs request validation, so the adapter
            // is told what the profile declared rather than a provider-wide
            // guess about every model the endpoint might serve.
            config.capabilities = profile.capabilities.clone();
            config.extra_headers = request
                .config
                .provider
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.value.clone()))
                .collect();
            let target = ProviderCredentialTarget::new(request.config.provider.name.value.clone())
                .map_err(|error| FactoryError::Runtime(RuntimeError::config(error.to_string())))?;
            match secret {
                Some(secret) => {
                    let source = credential_source(request, pool, secret);
                    let provider = ResponsesProvider::with_credential_source(
                        transport, config, target, source,
                    )
                    .map_err(FactoryError::Transport)?;
                    Ok(Arc::new(provider))
                }
                None => Ok(Arc::new(
                    ResponsesProvider::new(transport, config).map_err(FactoryError::Transport)?,
                )),
            }
        }
        Adapter::AnthropicMessages => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let mut config = AnthropicConfig::new(
                endpoint.unwrap_or_default(),
                request.config.model.value.clone(),
            );
            // The resolved profile governs request validation, so the adapter
            // is told what the profile declared rather than a provider-wide
            // guess about every model the endpoint might serve.
            config.capabilities = profile.capabilities.clone();
            config.extra_headers = request
                .config
                .provider
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.value.clone()))
                .collect();
            // Anthropic retains its reviewed static compatibility path until
            // that upstream adapter exposes the credential-source contract.
            config.api_key = secret;
            Ok(Arc::new(AnthropicProvider::new(transport, config)))
        }
        Adapter::ChatGptResponses => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let secret = secret.ok_or(FactoryError::ChatGptAuth(
                crate::chatgpt::ChatGptAuthError::InvalidBundle,
            ))?;
            let bundle =
                ChatGptTokenBundle::from_secret(&secret).map_err(FactoryError::ChatGptAuth)?;
            let account_id = bundle.account_id().to_owned();
            // The active member's reference, which is where its renewed bundle
            // goes back, so a refresh survives the process rather than being
            // re-earned every launch.
            let reference = active_credential_reference(request)
                .ok_or(FactoryError::ChatGptAuth(
                    crate::chatgpt::ChatGptAuthError::InvalidBundle,
                ))
                .and_then(|reference| {
                    CredentialRef::parse(&reference).map_err(|source| {
                        FactoryError::CredentialReference {
                            provider: request.config.provider.name.value.clone(),
                            source,
                        }
                    })
                })?;
            let target = ProviderCredentialTarget::new(request.config.provider.name.value.clone())
                .map_err(|error| FactoryError::Runtime(RuntimeError::config(error.to_string())))?;
            let refresher: Arc<dyn BundleRefresher<ChatGptTokenBundle>> =
                Arc::new(ChatGptOAuthClient::new().map_err(FactoryError::ChatGptAuth)?);
            let active = Arc::new(ChatGptCredentialSource::new(
                target.clone(),
                reference,
                bundle,
                CredentialEnroller::new(),
                refresher.clone(),
                request.persistence_redactor.clone(),
                Arc::new(SystemClock),
            )) as Arc<dyn ProviderCredentialSource>;
            if let Some(pool) = pool {
                let members = request.credentials.clone().map(|resolver| {
                    Arc::new(RenewableMemberSources::new(
                        resolver,
                        target.clone(),
                        refresher.clone(),
                        request.persistence_redactor.clone(),
                    )) as Arc<dyn PoolMemberSources>
                });
                spawn_chatgpt_usage_probe(
                    pool.clone(),
                    members,
                    active.clone(),
                    target.clone(),
                    account_id.clone(),
                );
            }
            let source = renewable_credential_source(request, pool, &target, refresher, active);
            let config = ChatGptProviderConfig::new(
                request.config.model.value.clone(),
                profile.capabilities.clone(),
                account_id,
            )
            .map_err(FactoryError::Transport)?;
            Ok(Arc::new(ChatGptProvider::new(
                transport, config, target, source,
            )))
        }
        Adapter::XaiResponses => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let mut config = ResponsesConfig::new(
                endpoint.unwrap_or_default(),
                request.config.model.value.clone(),
            );
            config.capabilities = profile.capabilities.clone();
            // xAI runs an implicit prefix cache on its own: a repeated-prefix
            // session reports large cached reads with no request field asked
            // of the adapter. Declare it so cache planning and reporting
            // reflect the reuse instead of claiming none exists.
            config.capabilities.prompt_cache = PromptCacheControl::Implicit;
            config.extra_headers = request
                .config
                .provider
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.value.clone()))
                .collect();
            let secret = secret.ok_or(FactoryError::XaiAuth(
                crate::xai::XaiAuthError::InvalidBundle,
            ))?;
            let bundle = XaiTokenBundle::from_secret(&secret).map_err(FactoryError::XaiAuth)?;
            // The active member's reference, which is where its renewed bundle
            // goes back, so a refresh survives the process rather than being
            // re-earned every launch.
            let reference = active_credential_reference(request)
                .ok_or(FactoryError::XaiAuth(
                    crate::xai::XaiAuthError::InvalidBundle,
                ))
                .and_then(|reference| {
                    CredentialRef::parse(&reference).map_err(|source| {
                        FactoryError::CredentialReference {
                            provider: request.config.provider.name.value.clone(),
                            source,
                        }
                    })
                })?;
            let target = ProviderCredentialTarget::new(request.config.provider.name.value.clone())
                .map_err(|error| FactoryError::Runtime(RuntimeError::config(error.to_string())))?;
            let refresher: Arc<dyn BundleRefresher<XaiTokenBundle>> =
                Arc::new(XaiOAuthClient::new().map_err(FactoryError::XaiAuth)?);
            let active = Arc::new(XaiCredentialSource::new(
                target.clone(),
                reference,
                bundle,
                CredentialEnroller::new(),
                refresher.clone(),
                request.persistence_redactor.clone(),
                Arc::new(SystemClock),
            )) as Arc<dyn ProviderCredentialSource>;
            let source = renewable_credential_source(request, pool, &target, refresher, active);
            let provider =
                ResponsesProvider::with_credential_source(transport, config, target, source)
                    .map_err(FactoryError::Transport)?;
            Ok(Arc::new(provider))
        }
        Adapter::GeminiInteractions => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let secret = secret.ok_or_else(|| {
                FactoryError::Runtime(RuntimeError::config(
                    "native Gemini provider credential is not configured",
                ))
            })?;
            let mut config = GeminiInteractionsConfig::new(
                endpoint.unwrap_or_else(|| smith_config::catalog::GEMINI_ENDPOINT.to_owned()),
                request.config.model.value.clone(),
            );
            config.capabilities = profile.capabilities.clone();
            // Models.dev describes the model; the native adapter owns the
            // implicit prefix-cache behavior it actually drives.
            config.capabilities.prompt_cache = PromptCacheControl::Implicit;
            config =
                config.with_supported_thinking_levels(supported_thinking_levels.iter().cloned());
            let target = ProviderCredentialTarget::new(request.config.provider.name.value.clone())
                .map_err(|error| FactoryError::Runtime(RuntimeError::config(error.to_string())))?;
            let source = credential_source(request, pool, secret);
            let provider = GeminiInteractionsProvider::with_credential_source(
                transport, config, target, source,
            )
            .map_err(FactoryError::Transport)?;
            Ok(Arc::new(provider))
        }
    }
}

/// Chooses the approval policy.
///
/// An injected surface always wins. Without one, `deny` and `allow-all` are
/// complete answers on their own, but `ask` is not: the run would either hang
/// on a question nobody receives or deny every call without saying why.
fn approval(request: &RuntimeRequest) -> Result<Arc<dyn ApprovalPolicy>, FactoryError> {
    let policy: Arc<dyn ApprovalPolicy> = if let Some(policy) = request.approval.clone() {
        policy
    } else {
        match request.config.approval.mode.value {
            ApprovalMode::Deny => Arc::new(DenyAll),
            ApprovalMode::AllowAll => Arc::new(AllowAll),
            ApprovalMode::Ask => {
                return Err(FactoryError::MissingHostPolicy {
                    what: "approval surface",
                    message: format!(
                        "`approval.mode` is `{}`, so the host must supply what answers the question; \
                         set `approval.mode` to `{}` or `{}` for an unattended run",
                        ApprovalMode::Ask.as_str(),
                        ApprovalMode::Deny.as_str(),
                        ApprovalMode::AllowAll.as_str()
                    ),
                });
            }
        }
    };

    let auto_approve = request
        .config
        .approval
        .auto_approve
        .as_ref()
        .map(|tools| tools.value.iter().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    if auto_approve.is_empty() {
        Ok(policy)
    } else {
        Ok(Arc::new(AutoApprove {
            tools: auto_approve,
            fallback: policy,
        }))
    }
}

/// Allows only explicitly named tools before consulting the configured gate.
///
/// This wrapper belongs in the one factory rather than in either surface so
/// `approval.auto_approve` cannot mean something different in the TUI and
/// `smith -p`.
#[derive(Debug)]
struct AutoApprove {
    tools: BTreeSet<String>,
    fallback: Arc<dyn ApprovalPolicy>,
}

#[async_trait]
impl ApprovalPolicy for AutoApprove {
    async fn decide(
        &self,
        request: &agent_runtime_core::approval::ApprovalRequest,
    ) -> agent_runtime_core::approval::ApprovalDecision {
        if self.tools.contains(request.prepared().tool()) {
            agent_runtime_core::approval::ApprovalDecision::Allow
        } else {
            self.fallback.decide(request).await
        }
    }
}

/// Derives the context policy from configuration and the resolved limits.
///
/// `context.output_reserve` has no built-in default because it depends on the
/// model, so it falls back to the generation cap this profile asks for and then
/// to the model's own declared ceiling. Both are declared values rather than
/// guesses — the ceiling comes from the profile that just resolved — which is
/// what keeps "never default a context window" intact while still producing an
/// enforceable reserve.
fn context_policy(
    config: &ResolvedConfig,
    profile: &ResolvedModelProfile,
) -> Result<ContextPolicy, FactoryError> {
    let reasoning_reserve = config.context.reasoning_reserve.value;
    let output_reserve = config
        .context
        .output_reserve
        .as_ref()
        .or(config.max_output_tokens.as_ref())
        .map_or(profile.limits.max_output_tokens, |sourced| sourced.value);

    let held_back = output_reserve.saturating_add(reasoning_reserve);
    if held_back >= profile.limits.context_tokens {
        return Err(FactoryError::ContextReserve {
            message: format!(
                "an output reserve of {output_reserve} and a reasoning reserve of \
                 {reasoning_reserve} leave nothing of model `{}`'s {} token window for input",
                profile.model, profile.limits.context_tokens
            ),
        });
    }

    let capability_budget = config.context.capability_budget.as_ref().map(|s| s.value);
    let max_estimated_slack = config.context.max_estimated_slack.as_ref().map(|s| s.value);
    let mut policy = ContextPolicy::new(
        RegistryRevision::new(policy_revision(
            output_reserve,
            reasoning_reserve,
            capability_budget,
            max_estimated_slack,
        )),
        output_reserve,
        reasoning_reserve,
    );
    if let Some(budget) = capability_budget {
        policy = policy.with_capability_budget(budget);
    }
    if let Some(slack) = max_estimated_slack {
        policy = policy.with_max_estimated_slack(slack);
    }
    Ok(policy)
}

/// Resolves Smith's percentage watermarks against the same enforceable input
/// budget the shared planner uses, then gives the result a revision identity.
fn compaction_policy(
    config: &ResolvedConfig,
    profile: &ResolvedModelProfile,
    context_policy: &ContextPolicy,
) -> CompactionPolicy {
    let input_budget = ContextBudget::from_limits(&profile.limits, context_policy).input_budget;
    let high_percent = config.context.compaction_high_watermark_percent.value;
    let low_percent = config.context.compaction_low_watermark_percent.value;
    let high_watermark = percentage_of(input_budget, high_percent);
    let low_watermark = percentage_of(input_budget, low_percent);
    CompactionPolicy::new(
        RegistryRevision::new(format!(
            "{COMPACTION_POLICY_REVISION}/high={high_watermark}/low={low_watermark}"
        )),
        high_watermark,
        low_watermark,
    )
}

/// Floors one configured percentage of a token budget without overflowing the
/// intermediate multiplication.
fn percentage_of(tokens: u32, percent: u8) -> u32 {
    let scaled = u64::from(tokens).saturating_mul(u64::from(percent)) / 100;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// The revision identifying one resolved context policy.
///
/// The reserves are part of the identity, not just the schema version: the
/// revision is what a plan and cache fingerprint carry, so two runs budgeted
/// differently must not present the same one.
fn policy_revision(
    output_reserve: u32,
    reasoning_reserve: u32,
    capability_budget: Option<u32>,
    max_estimated_slack: Option<u32>,
) -> String {
    let optional = |value: Option<u32>| match value {
        Some(value) => value.to_string(),
        None => "none".to_owned(),
    };
    format!(
        "{CONTEXT_POLICY_REVISION}/out={output_reserve}/reason={reasoning_reserve}/cap={}/slack={}",
        optional(capability_budget),
        optional(max_estimated_slack)
    )
}

/// Maps the configured prompt and loop limits onto the shared loop.
///
/// Set as one value rather than through the builder's individual setters
/// because the generation cap has no setter of its own, and splitting one
/// coherent loop configuration across two mechanisms is how half of it ends up
/// forgotten.
fn loop_config(request: &RuntimeRequest, model: &ModelId) -> LoopConfig {
    let config = &request.config;
    let mut loop_config = LoopConfig::new(model.clone());
    // Smith installs product instructions through `SmithPromptContributor` so
    // every section remains independently positioned, fingerprinted, and
    // budgeted. The legacy field stays empty to prevent a duplicate copy.
    loop_config.system_prompt = None;
    // A configured value of 0 removes the tool-loop ceiling, the same way it
    // removes the wall-clock one below: the limit is optional on the shared
    // loop, and `None` lets a turn run as many steps as the model asks for.
    loop_config.max_tool_steps =
        (config.limits.max_tool_steps.value != 0).then_some(config.limits.max_tool_steps.value);
    loop_config.retry = RetryPolicy {
        // Configuration counts retries *after* the first attempt; the shared
        // policy counts attempts including it.
        max_attempts: config.limits.max_retries.value.saturating_add(1),
        ..RetryPolicy::default()
    };
    // A configured value of 0 removes the wall-clock ceiling: the limit is
    // optional on the shared loop, and `None` leaves a turn unbounded by time.
    loop_config.turn_time_limit_ms = (config.limits.turn_time_limit_ms.value != 0)
        .then_some(config.limits.turn_time_limit_ms.value);
    loop_config.output_limit =
        usize::try_from(config.limits.tool_output_limit_bytes.value).unwrap_or(usize::MAX);
    loop_config.max_output_tokens = config.max_output_tokens.as_ref().map(|s| s.value);
    // Explicit rather than inherited: an unsupported capability must fail
    // before network I/O unless a named downgrade was configured, and Smith
    // configuration has no downgrade keys to configure one with yet.
    loop_config.downgrade = DowngradePolicy::strict();
    loop_config
}

/// The tools this run registers.
fn tools(request: &RuntimeRequest) -> Vec<Arc<dyn Tool>> {
    let read_only = request.config.agent.active_posture().is_read_only();
    let mut tools = if request.built_in_tools {
        request
            .change_recorder
            .as_ref()
            .map_or_else(smith_tools::all, |recorder| {
                smith_tools::observed_tools(recorder.clone())
            })
    } else {
        Vec::new()
    };
    if read_only {
        tools.retain(|tool| tool.spec().effects.is_read_only());
    }
    if questionnaire_eligible(request) {
        tools.push(Arc::new(QuestionnaireTool::new()));
    }
    if let Some(store) = request.artifact_store.clone() {
        tools.push(Arc::new(ArtifactReadTool::new(store)));
    }
    if todo_planning_eligible(request) {
        tools.push(Arc::new(WriteTodosTool::new()));
    }
    tools.extend(
        request
            .tools
            .iter()
            .filter(|tool| !read_only || read_only_extension(tool.spec()))
            .map(Arc::clone),
    );
    tools
}

fn goal_component_eligible(request: &RuntimeRequest) -> bool {
    request.config.persistence.enabled.value && !matches!(request.surface, HostSurface::Child)
}

// The three predicates below decide both whether a tool is registered and
// whether its instruction section is contributed. They exist as named
// functions precisely so those two decisions cannot drift apart: a run whose
// prompt describes a capability it did not register is a run that will try to
// call a tool that is not there.

/// Whether this run registers the root questionnaire tool.
fn questionnaire_eligible(request: &RuntimeRequest) -> bool {
    !matches!(request.surface, HostSurface::Child)
}

/// Whether this run registers the child-delegation `agent` tool.
fn delegation_eligible(request: &RuntimeRequest) -> bool {
    !matches!(request.surface, HostSurface::Child)
}

/// Whether this run registers the todo-planning tool.
///
/// A read-only posture's deliverable *is* a plan or a review, so a second
/// parallel plan in tool state is redundant with the answer itself.
fn todo_planning_eligible(request: &RuntimeRequest) -> bool {
    !request.config.agent.active_posture().is_read_only()
}

fn read_only_extension(spec: agent_runtime_core::tool::ToolSpec) -> bool {
    spec.effects.is_read_only()
        && spec
            .permission_upper_bound
            .iter()
            .all(|permission| matches!(permission, Permission::FsRead | Permission::ClockRead))
}

#[cfg(test)]
mod tests {
    use super::*;

    use agent_runtime_core::approval::{ApprovalOrigin, ApprovalRequest};
    use agent_runtime_core::catalog::ModelLimits;
    use agent_runtime_core::clock::Deadline;
    use agent_runtime_core::ids::ToolCallId;
    use agent_runtime_core::tool::{PreparedToolCall, ToolCallDisplay, ToolEffects};
    use smith_config::model::ProfileUse;
    use smith_config::resolve::{
        ResolvedAgent, ResolvedAgentMode, ResolvedAgentProfile, ResolvedContext, Source, Sourced,
    };
    use smith_host::HeadlessApproval;

    const TOKEN: &str = "sk-live-4kQm2ZpX8vRt7nLb1cWs9aYe";

    fn sourced<T>(value: T) -> Sourced<T> {
        Sourced::new(value, Source::built_in("test"))
    }

    fn provider(kind: &str, base_url: Option<&str>) -> ResolvedProvider {
        ResolvedProvider {
            name: sourced("acme".to_owned()),
            kind: sourced(kind.to_owned()),
            base_url: base_url.map(|url| sourced(url.to_owned())),
            credentials: Vec::new(),
            rotate_at_percent: None,
            api_key: None,
            headers: Default::default(),
            response: Default::default(),
        }
    }

    fn context(output_reserve: Option<u32>, reasoning_reserve: u32) -> ResolvedContext {
        ResolvedContext {
            output_reserve: output_reserve.map(sourced),
            reasoning_reserve: sourced(reasoning_reserve),
            capability_budget: None,
            max_estimated_slack: None,
            compaction_high_watermark_percent: sourced(85),
            compaction_low_watermark_percent: sourced(60),
            idle_compaction_ms: sourced(3_600_000),
        }
    }

    fn profile(limits: ModelLimits) -> ResolvedModelProfile {
        ResolvedModelProfile::explicit("acme", ModelId::new("example-model"), limits)
    }

    fn resolved_config() -> ResolvedConfig {
        ResolvedConfig {
            profile: None,
            agent: agent(AgentPosture::Build),
            provider: provider(KIND_FAKE, None),
            model: sourced("example-model".to_owned()),
            max_output_tokens: None,
            model_limits: Default::default(),
            reasoning: Default::default(),
            model_reasoning: Default::default(),
            context: context(None, 0),
            limits: limits(),
            persistence: persistence(),
            approval: approval_config(ApprovalMode::Ask),
            background: background(),
        }
    }

    #[test]
    fn reasoning_boolean_remains_fixed_and_omission_preserves_provider_default() {
        let config = resolved_config();
        let mut model_profile = profile(ModelLimits::new(128_000, 124_000, 4_096));
        model_profile.capabilities.reasoning =
            agent_runtime_core::provider::ReasoningSupport::Fixed;

        let policy = resolve_reasoning_policy(&config, &model_profile, None, None)
            .expect("presence-only profile");
        assert_eq!(
            policy.support,
            agent_runtime_core::provider::ReasoningSupport::Fixed
        );
        assert_eq!(
            policy.switch,
            crate::reasoning::ReasoningSwitch::Unavailable
        );
        assert!(policy.efforts.is_empty());
        assert!(policy.request_config().is_none());
    }

    #[test]
    fn explicit_reasoning_metadata_accepts_only_advertised_efforts() {
        let mut config = resolved_config();
        config.model_reasoning = smith_config::resolve::ResolvedModelReasoning {
            toggle: sourced(true).into(),
            mandatory: sourced(false).into(),
            efforts: sourced(vec!["none".to_owned(), "low".to_owned(), "high".to_owned()]).into(),
            default_enabled: sourced(true).into(),
            default_effort: sourced("low".to_owned()).into(),
            dialect: sourced(smith_config::model::ReasoningDialect::OpenaiEffort).into(),
        };
        config.reasoning.enabled = Some(sourced(true));
        config.reasoning.effort = Some(sourced("high".to_owned()));
        let model_profile = profile(ModelLimits::new(128_000, 124_000, 4_096));

        let policy = resolve_reasoning_policy(&config, &model_profile, None, None)
            .expect("advertised effort");
        assert_eq!(policy.effective_state(), "on");
        assert_eq!(policy.effective_effort(), "high");
        assert_eq!(
            policy
                .request_config()
                .and_then(|reasoning| reasoning.effort),
            Some("high".to_owned())
        );

        config.reasoning.effort = Some(sourced("extreme".to_owned()));
        let error = resolve_reasoning_policy(&config, &model_profile, None, None)
            .expect_err("unadvertised effort");
        assert!(error.contains("extreme"), "{error}");
        assert!(error.contains("none, low, high"), "{error}");
    }

    #[test]
    fn an_adapter_this_build_does_not_ship_is_never_routed_elsewhere() {
        let err = adapter(&provider("grpc-frontier", None)).expect_err("unavailable");
        assert!(matches!(err, FactoryError::AdapterUnavailable { .. }));
        assert!(err.to_string().contains("grpc-frontier"));
        assert!(err.to_string().contains(KIND_OPENAI_COMPATIBLE));
        assert!(err.to_string().contains(KIND_ANTHROPIC_MESSAGES));

        assert_eq!(
            adapter(&provider(
                KIND_OPENAI_COMPATIBLE,
                Some("https://api.example.test/v1")
            ))
            .expect("a known kind"),
            Adapter::OpenAiCompatible
        );
        assert_eq!(
            adapter(&provider(KIND_ANTHROPIC_MESSAGES, None)).expect("a known kind"),
            Adapter::AnthropicMessages
        );
        assert_eq!(
            adapter(&provider(
                KIND_CHATGPT_RESPONSES,
                Some(smith_config::setup::CHATGPT_ENDPOINT),
            ))
            .expect("a known kind"),
            Adapter::ChatGptResponses
        );
        assert_eq!(
            adapter(&provider(KIND_GEMINI_INTERACTIONS, None)).expect("a known kind"),
            Adapter::GeminiInteractions
        );
        assert_eq!(
            adapter(&provider(KIND_FAKE, None)).expect("a known kind"),
            Adapter::Fake
        );
    }

    #[test]
    fn an_anthropic_provider_defaults_to_the_official_endpoint() {
        let defaulted = endpoint(
            &provider(KIND_ANTHROPIC_MESSAGES, None),
            Some(smith_config::model::ANTHROPIC_DEFAULT_ENDPOINT),
        )
        .expect("the default endpoint");
        assert_eq!(defaulted, "https://api.anthropic.com/v1");

        // A configured endpoint (e.g. a gateway) still wins and is validated.
        let configured = endpoint(
            &provider(
                KIND_ANTHROPIC_MESSAGES,
                Some("https://claude-gw.example.test/v1/"),
            ),
            Some(smith_config::model::ANTHROPIC_DEFAULT_ENDPOINT),
        )
        .expect("a configured endpoint");
        assert_eq!(configured, "https://claude-gw.example.test/v1");

        let err = endpoint(
            &provider(
                KIND_ANTHROPIC_MESSAGES,
                Some("https://key@claude-gw.example.test/v1"),
            ),
            Some(smith_config::model::ANTHROPIC_DEFAULT_ENDPOINT),
        )
        .expect_err("credentials in the URL are refused even with a default");
        assert!(err.to_string().contains("unusable"));
    }

    #[test]
    fn an_endpoint_keeps_its_path_and_loses_its_trailing_slash() {
        let endpoint = endpoint(
            &provider(
                KIND_OPENAI_COMPATIBLE,
                Some("https://api.example.test:8443/v1/"),
            ),
            None,
        )
        .expect("an endpoint");
        assert_eq!(endpoint, "https://api.example.test:8443/v1");
    }

    #[test]
    fn an_endpoint_carrying_a_credential_is_refused_without_being_quoted() {
        for url in [
            &format!("https://smith:{TOKEN}@api.example.test/v1"),
            &format!("https://api.example.test/v1?api_key={TOKEN}"),
        ] {
            let err = endpoint(&provider(KIND_OPENAI_COMPATIBLE, Some(url)), None)
                .expect_err("a refused endpoint");
            let rendered = format!("{err} {err:?}");
            assert!(!rendered.contains(TOKEN), "{rendered}");
        }
    }

    #[test]
    fn an_endpoint_must_be_an_absolute_http_url() {
        for url in ["api.example.test/v1", "ftp://api.example.test/v1"] {
            assert!(
                endpoint(&provider(KIND_OPENAI_COMPATIBLE, Some(url)), None).is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn an_absent_output_reserve_falls_back_to_a_declared_limit_not_a_guess() {
        let profile = profile(ModelLimits::new(128_000, 124_000, 4_096));
        let mut config = ResolvedConfig {
            profile: None,
            agent: agent(AgentPosture::Build),
            provider: provider(KIND_FAKE, None),
            model: sourced("example-model".to_owned()),
            max_output_tokens: None,
            model_limits: Default::default(),
            reasoning: Default::default(),
            model_reasoning: Default::default(),
            context: context(None, 0),
            limits: limits(),
            persistence: persistence(),
            approval: approval_config(ApprovalMode::Ask),
            background: background(),
        };

        // Nothing configured: the model's own declared ceiling.
        let policy = context_policy(&config, &profile).expect("a policy");
        assert_eq!(policy.output_reserve, 4_096);

        // The profile's generation ask outranks the ceiling.
        config.max_output_tokens = Some(sourced(1_024));
        assert_eq!(
            context_policy(&config, &profile)
                .expect("a policy")
                .output_reserve,
            1_024
        );

        // And an explicit reserve outranks both.
        config.context = context(Some(8_192), 0);
        assert_eq!(
            context_policy(&config, &profile)
                .expect("a policy")
                .output_reserve,
            8_192
        );
    }

    #[test]
    fn reserves_that_consume_the_whole_window_fail_instead_of_planning() {
        let profile = profile(ModelLimits::new(8_000, 8_000, 4_096));
        let config = ResolvedConfig {
            profile: None,
            agent: agent(AgentPosture::Build),
            provider: provider(KIND_FAKE, None),
            model: sourced("example-model".to_owned()),
            max_output_tokens: None,
            model_limits: Default::default(),
            reasoning: Default::default(),
            model_reasoning: Default::default(),
            context: context(Some(6_000), 2_000),
            limits: limits(),
            persistence: persistence(),
            approval: approval_config(ApprovalMode::Ask),
            background: background(),
        };

        let err = context_policy(&config, &profile).expect_err("no room to plan");
        assert!(matches!(err, FactoryError::ContextReserve { .. }));
    }

    #[test]
    fn a_changed_reserve_changes_the_policy_revision() {
        assert_ne!(
            policy_revision(4_096, 0, None, None),
            policy_revision(8_192, 0, None, None)
        );
        assert_ne!(
            policy_revision(4_096, 0, None, None),
            policy_revision(4_096, 0, Some(12_000), None)
        );
        assert!(policy_revision(4_096, 0, None, None).starts_with(CONTEXT_POLICY_REVISION));
    }

    #[test]
    fn compaction_watermarks_are_derived_from_the_enforced_input_budget() {
        let profile = profile(ModelLimits::new(1_000, 900, 200));
        let mut config = ResolvedConfig {
            profile: None,
            agent: agent(AgentPosture::Build),
            provider: provider(KIND_FAKE, None),
            model: sourced("example-model".to_owned()),
            max_output_tokens: None,
            model_limits: Default::default(),
            reasoning: Default::default(),
            model_reasoning: Default::default(),
            context: context(Some(100), 0),
            limits: limits(),
            persistence: persistence(),
            approval: approval_config(ApprovalMode::Ask),
            background: background(),
        };

        let context_policy = context_policy(&config, &profile).expect("a context policy");
        let compact = compaction_policy(&config, &profile, &context_policy);
        assert_eq!(
            ContextBudget::from_limits(&profile.limits, &context_policy).input_budget,
            900
        );
        assert_eq!(compact.high_watermark, 765);
        assert_eq!(compact.low_watermark, 540);
        assert_eq!(
            compact.revision.as_str(),
            "smith-compaction-policy-1/high=765/low=540"
        );

        config.context.compaction_low_watermark_percent = sourced(50);
        let changed = compaction_policy(&config, &profile, &context_policy);
        assert_eq!(changed.low_watermark, 450);
        assert_ne!(compact.revision, changed.revision);
    }

    #[tokio::test]
    async fn auto_approve_is_shared_factory_policy_and_falls_back_for_other_tools() {
        let mut config = ResolvedConfig {
            profile: None,
            agent: agent(AgentPosture::Build),
            provider: provider(KIND_FAKE, None),
            model: sourced("example-model".to_owned()),
            max_output_tokens: None,
            model_limits: Default::default(),
            reasoning: Default::default(),
            model_reasoning: Default::default(),
            context: context(None, 0),
            limits: limits(),
            persistence: persistence(),
            approval: approval_config(ApprovalMode::Ask),
            background: background(),
        };
        config.approval.auto_approve = Some(sourced(vec!["edit".to_owned()]));
        let fallback = Arc::new(HeadlessApproval::new());
        let mut request = RuntimeRequest::new(config, HostSurface::Headless);
        request.approval = Some(fallback.clone());
        let policy = approval(&request).expect("a composed policy");
        let call = |tool: &str| {
            let effects = ToolEffects::read_only().with_write("project:files");
            let (permissions, resource) = effects.authorization_request(tool, "project");
            ApprovalRequest::new(
                PreparedToolCall::new(
                    ToolCallId::new(format!("call-{tool}")),
                    tool,
                    serde_json::json!({"path": "file.txt"}),
                    permissions,
                    resource,
                    effects,
                    ToolCallDisplay::new(format!("Run {tool}")),
                ),
                Deadline::never(),
                ApprovalOrigin::new(
                    agent_runtime_core::ids::SessionId::new("session-1"),
                    agent_runtime_core::ids::RequestId::new("request-1"),
                ),
            )
        };

        assert!(policy.decide(&call("edit")).await.is_allowed());
        assert!(
            fallback.required().is_none(),
            "the explicit allowlist consulted the fallback"
        );
        assert!(!policy.decide(&call("shell")).await.is_allowed());
        assert_eq!(
            fallback.required().expect("a denied fallback").tool,
            "shell"
        );
    }

    #[test]
    fn persistent_goals_are_composed_only_for_persisted_root_surfaces() {
        let root = RuntimeRequest::new(resolved_config(), HostSurface::Terminal);
        assert!(goal_component_eligible(&root));

        let child = RuntimeRequest::new(resolved_config(), HostSurface::Child);
        assert!(!goal_component_eligible(&child));

        let mut config = resolved_config();
        config.persistence.enabled = sourced(false);
        let ephemeral = RuntimeRequest::new(config, HostSurface::Headless);
        assert!(!goal_component_eligible(&ephemeral));
    }

    fn limits() -> smith_config::resolve::ResolvedLimits {
        smith_config::resolve::ResolvedLimits {
            max_retries: sourced(2),
            max_tool_steps: sourced(0),
            turn_time_limit_ms: sourced(0),
            tool_output_limit_bytes: sourced(65_536),
        }
    }

    fn persistence() -> smith_config::resolve::ResolvedPersistence {
        smith_config::resolve::ResolvedPersistence {
            enabled: sourced(true),
            sessions_dir: sourced("/state/sessions".into()),
            journal_events: sourced(true),
            checkpoint_key: None,
            checkpoint_key_credential: None,
        }
    }

    fn agent(posture: AgentPosture) -> ResolvedAgent {
        let name = posture.as_str().to_owned();
        let profile = ResolvedAgentProfile {
            name: name.clone(),
            posture: sourced(posture),
            description: None,
            instructions: None,
            uses: sourced(vec![ProfileUse::Main]),
            provider: None,
            model: None,
            revision: format!("test-{name}-profile-1"),
            legacy: false,
        };
        ResolvedAgent {
            active: sourced(name.clone()),
            order: sourced(vec![name.clone()]),
            modes: std::collections::BTreeMap::from([(
                name.clone(),
                ResolvedAgentMode {
                    posture: sourced(posture),
                    description: None,
                },
            )]),
            child_presets: Default::default(),
            profile: profile.clone(),
            profiles: std::collections::BTreeMap::from([(name.clone(), profile)]),
            profile_order: sourced(vec![name]),
        }
    }

    fn approval_config(mode: ApprovalMode) -> smith_config::resolve::ResolvedApproval {
        smith_config::resolve::ResolvedApproval {
            mode: sourced(mode),
            auto_approve: None,
        }
    }

    fn background() -> smith_config::resolve::ResolvedBackground {
        smith_config::resolve::ResolvedBackground {
            exit_policy: sourced(smith_config::model::BackgroundExit::Error),
            max_children: sourced(4),
            max_monitors: sourced(8),
        }
    }
}
