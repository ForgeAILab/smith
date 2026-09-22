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
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_runtime::ability::SealedAbilities;
use agent_runtime::ability::activation::{ActivationContext, FailClosedPolicy};
use agent_runtime::agent::config::{DowngradePolicy, LoopConfig};
use agent_runtime::capability::{ActivationBudget, CapabilityResolver};
use agent_runtime::context::{
    CompactionPolicy, ContextBudget, ContextPolicy, ProviderCacheCapability, StructuralCompactor,
};
use agent_runtime::harness::{
    CreateGoalTool, GetGoalTool, GoalComponent, MemoryContributor, MemorySource, QuestionnaireTool,
    SemanticSummaryCoordinator, SummaryModel, TodoComponent, UpdateGoalTool, WriteTodosTool,
};
use agent_runtime::hub::{ScopeIdentity, ScopeInputs};
use agent_runtime::provider::anthropic::{AnthropicConfig, AnthropicProvider};
use agent_runtime::provider::command::{
    CommandAdapter, CommandConfigError, CommandPreflightError, CommandProcessConfig,
    CommandProvider, CommandProviderBuildError,
};
use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, usage_event};
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
use agent_runtime_core::catalog::{Modality, ModelLimits, ModelRecord};
use agent_runtime_core::catalog::{ModelCatalogSource, ModelProfileError, ResolvedModelProfile};
use agent_runtime_core::checkpoint::{CHECKPOINT_SCHEMA_VERSION, CheckpointStore};
use agent_runtime_core::clock::{Clock, Deadline, SystemClock};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::interaction::{InteractionBroker, InteractionReadiness};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::provider::{
    CacheEndpointIdentity, Capabilities, FinishReason, ModelId, PromptCacheControl, Provider,
    ProviderAttemptPurpose, ProviderCacheBehavior, ProviderCacheContract, ProviderError,
    ProviderStreamEvent, SyntheticConformance,
};
use agent_runtime_core::provider_credential::{
    ProviderCredentialSource, ProviderCredentialTarget, StaticProviderCredentialSource,
};
use agent_runtime_core::security::{PermissionSet, SecurityResource};
use agent_runtime_core::store::{Secret, SecretStore, SessionStore};
use agent_runtime_core::tool::Tool;
use agent_runtime_core::workspace::Workspace;
use reqwest::Url;
use smith_config::catalog::OPENAI_ENDPOINT;
use smith_config::credential::{
    CredentialEnroller, CredentialError, CredentialRef, CredentialRefError, CredentialResolver,
};
use smith_config::model::{
    AgentPosture, ApprovalMode, AutoApprovalMount, AutoApprovalPermission, AutoApprovalRisk,
    KIND_ANTHROPIC_MESSAGES, KIND_CHATGPT_RESPONSES, KIND_COMMAND_JSONL, KIND_FAKE,
    KIND_GEMINI_INTERACTIONS, KIND_OPENAI_COMPATIBLE, KIND_OPENAI_RESPONSES, KIND_XAI_RESPONSES,
    ProfileUse,
};
use smith_config::output_budget::{OutputBudget, resolve_output_budget};
use smith_config::resolve::{
    AutoApprovalRule, CommandWorkingDirectory, McpValue, ResolvedConfig, ResolvedProvider, Source,
    Sourced,
};
use smith_config::setup::trusted_model;

use agent_runtime_core::check_set::ActionClass;
use agent_runtime_core::grant::SecurityCheckMode;

use smith_host::rotation::{HeadlessRotation, RotationPolicy};

use crate::abilities::{INTERACTION_READY_CONFIG, seal_tool_abilities};
use crate::authority::SmithToolAuthority;
use crate::background_tasks::BackgroundServices;
use crate::budget_notice::{BudgetNoticeComponent, DEFAULT_NOTICE_THRESHOLD_TOKENS};
use crate::catalog::{CatalogLayers, ProfileResolution};
use crate::chatgpt::{
    ChatGptCredentialSource, ChatGptOAuthClient, ChatGptProvider, ChatGptProviderConfig,
    ChatGptTokenBundle,
};
use crate::checkpoint::{BarrierCheckpointStore, CheckpointBarrier, SmithCheckpointSetup};
use crate::command_provider::{
    CommandAdapterConfigError, CommandJsonlAdapter, CommandProtocolProvider,
};
use crate::delegation::{
    AgentTool, AgentToolProfile, DelegationAuthority, DelegationWaitPolicy, SmithChildFactory,
    SmithChildRoute, SmithDelegation,
};
use crate::harness::{
    HarnessIdentity, HarnessResolutionReport, ResolvedHarness, ResolvedModule, TrustedNativeModule,
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
use crate::tool_output::ToolOutputContextPolicy;
use crate::transport::{ReqwestTransport, TransportConfig};
use crate::xai::{XaiCacheIdentityProvider, XaiCredentialSource, XaiOAuthClient, XaiTokenBundle};

mod authority;
mod capabilities;
mod compose;
mod context_policy;
mod delegation;
mod persistence;
mod provider;
mod resolve;

/// The reply the deterministic development provider gives.
///
/// A `fake` provider with nothing injected is a development or smoke-test
/// composition, so it answers rather than failing — but it answers something
/// that cannot be mistaken for a model.
pub const DEVELOPMENT_REPLY: &str = "This session is running Smith's deterministic fake provider; \
     configure a real provider to talk to a model.";

/// The share of a model's input budget the capability lane gets when
/// configuration does not name one, as a percentage.
///
/// See [`derived_capability_budget`] for why this is a share rather than a
/// count.
const CAPABILITY_BUDGET_PERCENT: u32 = 15;

/// The floor and ceiling on a derived capability budget.
///
/// The floor holds Smith's built-in tool schemas with room for the reference
/// sections beside them; the ceiling stops a very large window from reserving
/// far more of the capability lane than any activation could use.
const MIN_DERIVED_CAPABILITY_BUDGET: u32 = 8_192;
/// See [`MIN_DERIVED_CAPABILITY_BUDGET`].
const MAX_DERIVED_CAPABILITY_BUDGET: u32 = 65_536;

/// The share of the capability budget skill instructions may take, as a
/// percentage. See [`context_policy::skill_instruction_budget`].
const SKILL_BUDGET_PERCENT: u32 = 10;

/// The floor under the skill share, so one reference section always fits.
///
/// Sections run to roughly 2.7k tokens, so this clears the largest with room
/// for the outline that routes to it.
const MIN_SKILL_INSTRUCTION_BUDGET: u32 = 4_096;

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
/// Host-owned endpoint/credential partition revision folded into Runtime's
/// opaque cache identity. Bump whenever the label inputs or normalization
/// change so persisted comparison baselines retire rather than transfer.
pub const CACHE_ENDPOINT_IDENTITY_REVISION: &str = "smith-cache-endpoint-1";

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
    KIND_COMMAND_JSONL,
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
    /// Trusted in-process Rust contributions registered in addition to Smith's
    /// built-ins. This is an embedding API, not a sandboxed plugin surface.
    pub trusted_native: TrustedNativeModule,
    /// Connected MCP servers, whose tools are registered alongside the
    /// built-ins.
    ///
    /// The supervisor is owned by the host and outlives one runtime, so a
    /// server that connects after composition is picked up at the next
    /// rebuild rather than being lost. Absent means no server is declared.
    pub mcp: Option<Arc<crate::mcp::McpSupervisor>>,
    /// Explicit host-owned background process services. Standard Smith hosts
    /// install one before capability assembly; direct embedders may leave it
    /// absent to get the deliberate unavailable adapter.
    pub background_services: Option<BackgroundServices>,
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
            trusted_native: TrustedNativeModule::default(),
            mcp: None,
            background_services: None,
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
    /// The installed coding agent this run's turns execute on, when the
    /// profile selected one. Surfaces use it to offer the CLI's models rather
    /// than the provider catalog, and to label the turn.
    pub harness: Option<smith_config::resolve::ResolvedHarness>,
    /// Active agent profile selected for this run.
    pub agent_profile: String,
    /// Deterministic effective agent-profile revision.
    pub agent_profile_revision: String,
    /// Placements allowed by the effective profile declaration.
    pub agent_profile_uses: Vec<ProfileUse>,
    /// Source of the effective authority posture, without instruction text.
    pub agent_profile_source: String,
    /// Whether direct-child delegation is enabled for this composition.
    pub agent_delegation: bool,
    /// Source of the effective delegation setting.
    pub agent_delegation_source: String,
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
    /// Opaque endpoint/tenant partition supplied to Runtime cache planning.
    pub cache_endpoint_identity: Option<CacheEndpointIdentity>,
    /// The credential *reference*, never its value.
    pub credential: Option<String>,
    /// Resolved approval mode enforced for authority-bearing actions.
    pub approval_mode: ApprovalMode,
    /// The selected model.
    pub model: ModelId,
    /// The frozen profile every request is planned against.
    pub model_profile: ResolvedModelProfile,
    /// Active named context window when the binding offers selectable windows.
    pub context_window: Option<String>,
    /// Available named context windows for the selected provider/model pair.
    pub context_windows: Vec<String>,
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
    /// Effective inline threshold and bounded artifact retrieval policy.
    pub tool_output_context: ToolOutputContextPolicy,
    /// Whether full tool outcomes can actually be retained by this composition.
    pub artifact_offloading: bool,
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
            .field("agent_delegation", &self.agent_delegation)
            .field("agent_delegation_source", &self.agent_delegation_source)
            .field("agent_profile_legacy", &self.agent_profile_legacy)
            .field("agent_posture", &self.agent_posture)
            .field("provider_name", &self.provider_name)
            .field("provider_kind", &self.provider_kind)
            .field("endpoint", &self.endpoint)
            .field("cache_endpoint_identity", &self.cache_endpoint_identity)
            .field("credential", &self.credential)
            .field("approval_mode", &self.approval_mode)
            .field("model", &self.model)
            .field("model_profile", &self.model_profile)
            .field("context_window", &self.context_window)
            .field("context_windows", &self.context_windows)
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
    background_services: Option<BackgroundServices>,
    harness_identity: HarnessIdentity,
    harness_modules: Arc<[ResolvedModule]>,
    harness_report: HarnessResolutionReport,
    image_history: Arc<crate::image_history::SessionImageHistory>,
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

    /// Host-owned background services retained for this runtime's lifetime.
    pub fn background_services(&self) -> Option<&BackgroundServices> {
        self.background_services.as_ref()
    }

    /// Immutable identity of the resolved product harness.
    pub fn harness_identity(&self) -> &HarnessIdentity {
        &self.harness_identity
    }

    /// Trust-, provenance-, contribution-, and grant-bearing module evidence.
    pub fn harness_modules(&self) -> &[ResolvedModule] {
        &self.harness_modules
    }

    /// Bounded non-secret harness resolution report.
    pub fn harness_report(&self) -> &HarnessResolutionReport {
        &self.harness_report
    }

    /// Canonical active-session image history used by `generate_image`.
    pub fn image_history(&self) -> &Arc<crate::image_history::SessionImageHistory> {
        &self.image_history
    }
}

/// Why a resolved configuration could not become a runtime.
///
/// Every variant names what the user has to change, and none can carry a
/// credential: the payloads are provider names, references, and classified
/// failures from types that redact themselves.
#[derive(Debug, thiserror::Error)]
pub enum FactoryError {
    /// Declarative modules, trust, contributions, or grants did not resolve.
    #[error(transparent)]
    Harness(#[from] crate::harness::HarnessResolutionError),

    /// Two tools attempted to register the same stable ability name.
    #[error("Smith could not seal its ability catalog: {0}")]
    AbilityRegistry(#[source] agent_runtime::registry::NameConflict),

    /// The configured adapter kind is not one the pinned runtime ships.
    #[error(
        "provider `{provider}` selects the `{kind}` adapter, which this build of Agent Runtime \
         does not ship; the available kinds are `{KIND_OPENAI_COMPATIBLE}`, \
         `{KIND_OPENAI_RESPONSES}`, `{KIND_ANTHROPIC_MESSAGES}`, \
         `{KIND_CHATGPT_RESPONSES}`, `{KIND_XAI_RESPONSES}`, `{KIND_GEMINI_INTERACTIONS}`, and \
         `{KIND_COMMAND_JSONL}`, and `{KIND_FAKE}`"
    )]
    AdapterUnavailable {
        /// The provider that selected it.
        provider: String,
        /// The adapter kind it selected.
        kind: String,
    },
    /// Static command process settings did not resolve to an executable target.
    #[error("the command provider process declaration is unusable: {0}")]
    CommandConfiguration(#[source] CommandConfigError),
    /// The selected model cannot participate in the revision-1 command protocol.
    #[error("the command provider adapter is unusable: {0}")]
    CommandAdapter(#[source] CommandAdapterConfigError),
    /// The command adapter's fixed local model declaration was contradictory.
    #[error("the command provider model declaration is unusable: {0}")]
    CommandProviderBuild(#[source] CommandProviderBuildError),
    /// One named command environment credential could not cross the secret boundary.
    #[error("command environment variable `{variable}` could not be resolved: {source}")]
    CommandEnvironment {
        /// Environment variable named by configuration; never its value.
        variable: String,
        /// Existing redaction-safe credential failure.
        #[source]
        source: Box<FactoryError>,
    },
    /// The explicit compatibility probe failed before runtime construction.
    #[error("the command provider compatibility probe failed: {0}")]
    CommandPreflight(#[source] CommandPreflightError),
    /// The executable answered the probe but did not accept this exact contract.
    #[error("the command provider executable is incompatible with protocol revision 1")]
    CommandIncompatible,
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
    /// A requested named context window is unavailable or pinned by a flat limit.
    #[error("provider `{provider}` cannot select a context window for model `{model}`: {message}")]
    ContextWindow {
        /// Provider serving the model.
        provider: String,
        /// Selected model.
        model: ModelId,
        /// Safe explanation and valid alternatives.
        message: String,
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
    /// The selected installed coding agent is not installed on this machine.
    #[error(
        "model `{model}` runs turns on the installed agent `{kind}`, but `{program}` is not on \
         PATH; install it, or declare `[harness.{kind}]` with an absolute `executable`"
    )]
    AgentNotInstalled {
        /// The `cli/<kind>/<model>` id that selected it.
        model: String,
        /// The installed agent kind, which is also its `[harness.<kind>]` key.
        kind: String,
        /// The program that was looked for.
        program: String,
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
    /// Bounded implementation/version evidence from a command probe.
    pub command_implementation: Option<String>,
}

struct PreparedFactoryInputs {
    provider_name: String,
    provider_kind: String,
    adapter: provider::Adapter,
    endpoint: Option<String>,
    secret: Option<Secret>,
    model: ModelId,
    profile: ProfileResolution,
    context_window: context_policy::ContextWindowSelection,
    reasoning: ReasoningRuntimePolicy,
    output_budget: OutputBudget,
    context_policy: ContextPolicy,
    compaction_policy: CompactionPolicy,
    loop_config: LoopConfig,
    command: Option<provider::PreparedCommandProvider>,
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

fn summary_route_provider(config: &SmithSemanticSummaryConfig, active_provider: &str) -> String {
    config
        .provider
        .clone()
        .unwrap_or_else(|| active_provider.to_owned())
}

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

/// Validates setup through the same factory input derivation used by
/// [`build`], without crossing the runtime-construction boundary.
pub async fn preflight(request: &RuntimeRequest) -> Result<FactoryPreflight, FactoryError> {
    authority::require_workspace(request)?;
    // Every member's reference is parsed before any provider I/O, not just the
    // one this session starts on. A pool whose second entry is malformed must
    // fail here, while the user is looking at setup — not in an hour, when the
    // first account is spent and rotation reaches for a reference that was
    // never going to work.
    provider::validate_pool_references(request)?;
    let prepared = provider::prepare(request).await?;
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
        command_implementation: prepared.command.and_then(|command| command.implementation),
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
        todo_planning: capabilities::todo_planning_eligible(request),
        questionnaire: capabilities::questionnaire_eligible(request),
        delegation: capabilities::delegation_eligible(request),
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
    // The standard adapter is bound to the active run provider. A custom
    // summary adapter is an independent route and must carry the explicit
    // provider identity validated above; never infer it from the parent.
    let summary_provider = summary_route_provider(&config, provider_name);
    let policy = SemanticSummaryRuntimePolicy {
        purpose: agent_runtime::harness::SEMANTIC_SUMMARY_PURPOSE.into(),
        provider: summary_provider,
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

/// Builds the runtime one resolved Smith run needs.
///
/// Async because credential resolution is: the platform credential service is
/// synchronous and may block on an unlock prompt, so it runs on a blocking
/// thread rather than on the executor a provider stream will share.
pub async fn build(harness: ResolvedHarness) -> Result<SmithRuntime, FactoryError> {
    let resolved = resolve::accept(harness);
    let mut harness_identity = resolved.identity;
    let harness_modules = resolved.modules;
    let mut harness_report = resolved.report;
    let request = resolved.request;
    // Host policy first. It costs nothing to check and everything to get wrong,
    // and failing here means a misconfigured run never reaches a keychain.
    let authority = authority::prepare(&request)?;
    let workspace = authority.workspace;
    let approval = authority.approval;
    let PreparedFactoryInputs {
        provider_name,
        provider_kind,
        adapter,
        endpoint,
        secret,
        model,
        profile,
        context_window,
        reasoning,
        output_budget,
        context_policy,
        compaction_policy,
        mut loop_config,
        command,
    } = provider::prepare(&request).await?;
    harness_identity = harness_identity.with_output_budget(&output_budget);
    harness_report.finalize_output_budget(&harness_identity, &output_budget);
    let agent_posture = request.config.agent.active_posture();
    let agent_profile = request.config.agent.profile.clone();
    let agent_profile_name = agent_profile.name.clone();
    let prompt = prepare_prompt_stage(&request, &mut loop_config)?;
    let config = &request.config;
    let cache_endpoint_identity = provider::cache_endpoint_identity(
        &provider_name,
        &provider_kind,
        endpoint.as_deref(),
        provider::active_credential_reference(&request).as_deref(),
    );

    // The only boundary the secret crosses.
    let provider_stage = provider::construct_runtime(
        &request,
        adapter,
        endpoint.clone(),
        secret,
        &profile.profile,
        &reasoning,
        command
            .as_ref()
            .map(|command| Arc::clone(&command.provider)),
    )?;
    let provider = provider_stage.provider;
    let image_backend = provider_stage.image_backend;
    let image_history = Arc::new(crate::image_history::SessionImageHistory::default());

    let clock: Arc<dyn Clock> = request
        .clock
        .clone()
        .unwrap_or_else(|| Arc::new(SystemClock));
    let child_profile_routes =
        provider::prepare_child_profile_routes(&request, prompt.project_instructions.as_ref())
            .await?;
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
    let capabilities = capabilities::prepare(
        &request,
        agent_tool_profiles,
        image_backend,
        image_history.clone(),
    )?;
    let durability = persistence::prepare(&request).await?;

    let policy = assemble_policy(RuntimePolicy {
        harness: request.config.harness.clone(),
        agent_profile: agent_profile_name.clone(),
        agent_profile_revision: agent_profile.revision.clone(),
        agent_profile_uses: agent_profile.uses.value.clone(),
        agent_profile_source: agent_profile.posture.source.to_string(),
        agent_delegation: agent_profile.delegation.value,
        agent_delegation_source: agent_profile.delegation.source.to_string(),
        agent_profile_legacy: agent_profile.legacy,
        agent_posture,
        provider_name: provider_name.clone(),
        provider_kind: provider_kind.clone(),
        endpoint: endpoint.clone(),
        cache_endpoint_identity: cache_endpoint_identity.clone(),
        credential: config
            .provider
            .credential()
            .map(|reference| reference.value.clone()),
        approval_mode: config.approval.mode.value,
        model: model.clone(),
        model_profile: profile.profile.clone(),
        context_window: context_window
            .active
            .as_ref()
            .map(|window| window.name.clone()),
        context_windows: context_window.available.clone(),
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
        tool_output_context: ToolOutputContextPolicy::from_config(config),
        artifact_offloading: request.artifact_store.is_some(),
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
    let capability_budget =
        ContextBudget::from_limits(&profile.profile.limits, &context_policy).capability_budget;
    // Skills are the asymmetric half of the capability budget. A tool schema
    // is a few hundred tokens; a reference skill is instruction prose and can
    // be thousands, and activation is monotonic — one bound speculatively on
    // the first turn still holds its tokens on the last. Left uncapped, two
    // or three of them fill the budget and the next tool the task needs
    // overflows it, which fails the turn outright. A tenth keeps discovery
    // possible while leaving the schemas the room they need; a reference too
    // large to fit is a reference that needs splitting, not a larger share.
    let activation_budget = ActivationBudget::new(capability_budget, 8)
        .with_instruction_budget(context_policy::skill_instruction_budget(capability_budget));
    // A harness profile runs its turns on an installed CLI. The provider below
    // is still resolved and still supplies model identity and the limits the
    // runtime enforces before any work runs; it is simply never called to
    // produce a turn.
    let execution = crate::cli_agent::turn_execution(&request.config);
    if let crate::cli_agent::TurnExecution::MissingProgram {
        model,
        kind,
        program,
    } = &execution
    {
        // Refused here rather than composed and left to fail per turn: with
        // no backend to run them, every turn would go to the provider
        // carrying a `cli/...` model id it has never heard of, and the run
        // would report that rejection instead of the missing program.
        return Err(FactoryError::AgentNotInstalled {
            model: model.clone(),
            kind: kind.clone(),
            program: program.clone(),
        });
    }

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
    // A harness replaces how a turn is executed, leaving every other piece of
    // the composition above in place.
    if let crate::cli_agent::TurnExecution::InstalledAgent(plan) = &execution {
        builder = builder.external_agent(plan.backend(PathBuf::from(workspace.root()), true));
    }
    if let Some(identity) = cache_endpoint_identity.as_ref() {
        builder = builder.cache_endpoint_identity(identity.clone());
    }
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
        let offloader = ToolOutputContextPolicy::from_config(config)
            .offloader(store)
            .map_err(FactoryError::Runtime)?;
        builder = builder.tool_output_processor(Arc::new(offloader));
    }
    for observer in request.observers.iter().cloned() {
        builder = builder.observer(observer);
    }

    let built = compose::runtime(builder)?;
    let delegation = delegation::assemble(capabilities.delegation_slot.clone().map(|slot| {
        SmithDelegation {
            factory: Arc::new(SmithChildFactory {
                default_route: SmithChildRoute {
                    provider,
                    provider_name,
                    provider_kind,
                    cache_endpoint_identity,
                    model,
                    model_profile: profile.profile.clone(),
                    context_policy,
                    tool_output_context: ToolOutputContextPolicy::from_config(config),
                    loop_config,
                    prompt_contributor: prompt.contributor.clone(),
                    agent_profile_name: agent_profile.name.clone(),
                    agent_profile_revision: agent_profile.revision.clone(),
                    agent_profile_posture: agent_profile.posture.value,
                    read_only: agent_profile.posture.value.is_read_only(),
                    // An inheriting child is the parent's run narrowed, so it
                    // runs turns wherever the parent's do.
                    execution,
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
        background_services: request.background_services,
        harness_identity,
        harness_modules,
        harness_report,
        image_history,
    })
}

/// Protocol-v1 migration adapter for trusted embedders that still construct a
/// [`RuntimeRequest`] directly.
///
/// New hosts must call [`crate::harness::resolve`] themselves and pass the
/// resulting [`ResolvedHarness`] to [`build`]. This adapter performs no
/// alternate composition; it resolves and delegates to that single root.
#[doc(hidden)]
#[deprecated(
    since = "0.0.2",
    note = "resolve HarnessSpec first and call factory::build(ResolvedHarness)"
)]
pub async fn build_request(request: RuntimeRequest) -> Result<SmithRuntime, FactoryError> {
    let harness = crate::harness::resolve(crate::harness::HarnessSpec::trusted(request))?;
    build(harness).await
}

#[cfg(test)]
mod tests;
