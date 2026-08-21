//! The shape of a `config.toml`, exactly as written.
//!
//! This is the *file* model, not the resolved model. Every field is optional
//! because any one layer supplies only part of the answer; [`crate::resolve`]
//! decides which layer's part wins and produces the typed run configuration.
//!
//! Three rules here are load-bearing:
//!
//! - `deny_unknown_fields` on every table. A misspelled key that is silently
//!   ignored is worse than a refused start, because the user goes on believing
//!   a limit is in force that never was.
//! - No built-in model limits. Smith never guesses a real model's context
//!   window; limits are either written in `[models."<provider>/<model>"]` or
//!   supplied by a catalog source when the runtime is composed.
//! - A provider's `kind` is an open string, not an enum. The set of adapters
//!   lives in the pinned Agent Runtime registry, so a closed Smith enum would
//!   make every upstream adapter wait for a Smith release.
//!
//! Nothing in this module reads a secret, runs a command, or touches the
//! network. A `credential` is a reference string whose *shape* is validated
//! during resolution. An `api_key` is accepted only from user configuration
//! and is secret-bearing from deserialization onward.

use std::collections::BTreeMap;
use std::fmt;

use agent_runtime_core::store::Secret;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// A plaintext value present in owner-only user configuration.
///
/// Serialization is intentionally the one place this wrapper exposes its
/// contents: setup must be able to persist the reviewed user config. Debug and
/// display surfaces remain redacted.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ConfigSecret(String);

impl ConfigSecret {
    /// Wraps a secret entered by setup.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Copies the value into Agent Runtime's redaction-safe secret type.
    pub fn to_secret(&self) -> Secret {
        Secret::new(self.0.clone())
    }

    /// Whether configuration supplied no credential bytes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for ConfigSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConfigSecret([redacted])")
    }
}

impl fmt::Display for ConfigSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl Drop for ConfigSecret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Authority-narrowing posture of one Smith agent mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentPosture {
    /// Normal coding workflow. The resolved run still decides actual authority.
    #[default]
    Build,
    /// Read-only inspection and planning.
    Plan,
    /// Read-only change review and findings.
    Review,
}

impl AgentPosture {
    /// Parses the stable configuration spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "build" => Some(Self::Build),
            "plan" => Some(Self::Plan),
            "review" => Some(Self::Review),
            _ => None,
        }
    }

    /// The stable configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
            Self::Review => "review",
        }
    }

    /// Every supported posture spelling.
    pub fn spellings() -> &'static [&'static str] {
        &["build", "plan", "review"]
    }

    /// Whether the posture is guaranteed read-only.
    pub fn is_read_only(self) -> bool {
        matches!(self, Self::Plan | Self::Review)
    }
}

/// A placement where a named agent profile may be selected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileUse {
    /// The interactive or headless root agent.
    #[default]
    Main,
    /// An explicit depth-one child agent.
    Child,
}

impl ProfileUse {
    /// Parses the stable configuration spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "main" => Some(Self::Main),
            "child" => Some(Self::Child),
            _ => None,
        }
    }

    /// The stable configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Child => "child",
        }
    }

    /// Every supported placement spelling.
    pub fn spellings() -> &'static [&'static str] {
        &["main", "child"]
    }
}

/// One named root-agent mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModeSection {
    /// Built-in authority-narrowing behavior this name selects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posture: Option<AgentPosture>,
    /// Bounded user-facing explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One named direct-child preset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChildAgentSection {
    /// Read-only posture applied to the child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posture: Option<AgentPosture>,
    /// Bounded user-facing explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One `config.toml` file.
///
/// The same shape serves `~/.smith/config.toml`, `<project>/.smith/config.toml`,
/// and `<project>/.smith/config.local.toml`; the layer a file occupies is a
/// property of where it was found, not of what it contains.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    /// The profile to select when no higher layer selects one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    /// Stable order used by idle-composer profile cycling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_order: Option<Vec<String>>,
    /// Root agent mode selected when no higher layer supplies one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
    /// Stable order used by idle-composer agent cycling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_order: Option<Vec<String>>,
    /// Named root-agent modes: `[agent_modes.<name>]`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agent_modes: BTreeMap<String, AgentModeSection>,
    /// Named depth-one child presets: `[child_agents.<name>]`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub child_agents: BTreeMap<String, ChildAgentSection>,
    /// Named profiles: `[profiles.<name>]`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, ProfileSection>,
    /// Named providers: `[providers.<name>]`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, ProviderSection>,
    /// Model limits keyed `"<provider>/<model>"`: `[models."acme/example-model"]`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, ModelSection>,
    /// Default reasoning selection for the active provider/model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningSection>,
    /// Context reserves, budgets, and compaction watermarks: `[context]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextSection>,
    /// Loop limits: `[limits]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<LimitsSection>,
    /// Session persistence policy: `[persistence]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence: Option<PersistenceSection>,
    /// Tool approval policy: `[approval]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalSection>,
    /// Background-work policy for monitors and child agents: `[background]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<BackgroundSection>,
    /// Local cache-miss notice policy: `[cache]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<CacheSection>,
    /// Model Context Protocol servers: `[mcp.servers.<name>]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpSection>,
}

/// The Model Context Protocol servers one layer declares.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpSection {
    /// Named servers: `[mcp.servers.<name>]`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub servers: BTreeMap<String, McpServerSection>,
}

/// One MCP server declaration.
///
/// A declaration names exactly one transport: a `command` Smith spawns, or a
/// `url` it connects to. Naming both is a contradiction rather than a
/// preference, because there is no defensible rule for which one a user meant.
///
/// Nothing here is executed by reading it. Spawning the command requires an
/// executable-trust decision over the *resolved* invocation, which is a
/// separate question asked once per content digest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerSection {
    /// The program to run for a local server spoken to over its stdio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The command's arguments, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Environment variables set for a spawned server.
    ///
    /// A value is either a credential *reference* — `keychain:`, `authfile:`,
    /// `env:`, or `file:` — resolved through the same secret path as a
    /// provider's, or a literal. A literal under a credential-bearing variable
    /// name is refused outside owner-only user configuration, because a
    /// repository file is the wrong place to keep a token.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// The endpoint of a remote server reached over streamable HTTP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// A credential reference sent to a remote server as a bearer token.
    ///
    /// The same references a provider's `credential` takes, resolved through
    /// the same path. Writing the token itself is refused: this is a
    /// repository file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// Extra headers sent to a remote server.
    ///
    /// Values follow the same rule as `env`: a credential reference is
    /// resolved, and a literal under an authorization-bearing header name is
    /// refused rather than redacted.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Whether this run uses the server at all. Omitted means enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// One profile: a coherent selection a user can switch to by name.
///
/// A profile may override any *active* setting, which is why the policy
/// sub-tables repeat here. It cannot declare providers or models — those are
/// shared by every profile in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSection {
    /// Optional single parent profile whose unset fields this profile inherits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    /// Bounded user-facing explanation of this agent preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Authority-narrowing behavior this profile selects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posture: Option<AgentPosture>,
    /// Main/child placements where the profile is selectable.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "use")]
    pub uses: Option<Vec<ProfileUse>>,
    /// Bounded additive developer instructions for this agent preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Whether a main-agent runtime exposes direct-child delegation.
    ///
    /// Omitted preserves the existing enabled behavior. Child runtimes never
    /// delegate regardless of this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<bool>,
    /// The name of a provider declared in `[providers.<name>]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The model this profile sends to that provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Root agent mode selected with this provider/model profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The generation cap this profile asks the provider for.
    ///
    /// This is the request-time ask. The model's own ceiling belongs in
    /// `[models."<provider>/<model>"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Profile-scoped reasoning defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningSection>,
    /// Profile-scoped `[context]` overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextSection>,
    /// Profile-scoped `[child_agents]` wait policy overrides.
    ///
    /// This is deliberately separate from the top-level named child-agent
    /// presets.  The latter describe who may run; this table bounds how a
    /// parent waits for one of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_agents: Option<ChildAgentPolicySection>,
    /// Profile-scoped `[limits]` overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<LimitsSection>,
    /// Profile-scoped `[approval]` overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalSection>,
    /// Profile-scoped `[background]` overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<BackgroundSection>,
    /// Profile-scoped `[cache]` overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<CacheSection>,
}

/// Local cache presentation policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheSection {
    /// Whether significant cache-miss notices are shown on local surfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miss_notices: Option<bool>,
}

/// How Smith handles optional synthetic provider-cache maintenance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheMaintenanceMode {
    /// Do not schedule synthetic cache work.
    #[default]
    Off,
    /// Record plans and observations without synthetic provider I/O.
    Observe,
    /// Permit bounded synthetic work after all capability and host gates pass.
    Adaptive,
}

impl CacheMaintenanceMode {
    /// Parses a stable configuration spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "observe" => Some(Self::Observe),
            "adaptive" => Some(Self::Adaptive),
            _ => None,
        }
    }

    /// Stable configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Observe => "observe",
            Self::Adaptive => "adaptive",
        }
    }

    /// All accepted spellings.
    pub fn spellings() -> &'static [&'static str] {
        &["off", "observe", "adaptive"]
    }
}

/// Layered lifecycle policy under `[context.cache]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextCacheSection {
    /// Requested synthetic maintenance mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance: Option<CacheMaintenanceMode>,
    /// Meaningful inactivity before the idle boundary, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inactivity_limit_ms: Option<u64>,
    /// Maximum bounded parent hold while a child remains active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hold_while_child_ms: Option<u64>,
    /// Synthetic requests permitted per parked interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_maintenance_calls: Option<u8>,
    /// Exact input-token budget; zero selects the resolved plan/model budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_maintenance_input_tokens: Option<u32>,
    /// Generated output-token budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_maintenance_output_tokens: Option<u32>,
    /// Deadline for a synthetic request, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_deadline_ms: Option<u64>,
    /// Early scheduling margin before a declared retention boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive_margin_ms: Option<u64>,
    /// Jitter applied to the early scheduling margin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive_jitter_percent: Option<u8>,
    /// Whether a same-model handoff summary may consume the allowance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_checkpoint: Option<bool>,
    /// Whether idle compaction is enabled at the meaningful-inactivity limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_compaction: Option<bool>,
    /// Whether the redaction-safe resume capsule projection is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_capsule: Option<bool>,
}

/// Parent wait bounds under `[profiles.<name>.child_agents]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChildAgentPolicySection {
    /// Default timeout for `agent.wait`, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_default_timeout_ms: Option<u64>,
    /// Maximum accepted `agent.wait.timeout_ms`, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_max_timeout_ms: Option<u64>,
}

/// One provider declaration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSection {
    /// The shared adapter this provider maps to, e.g. `openai-compatible`.
    ///
    /// Whether the pinned runtime actually ships that adapter is decided when
    /// the runtime is composed; resolution here only checks that the options
    /// given suit the kinds Smith knows how to configure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The API base URL, e.g. `https://api.example.test/v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// A scheme-prefixed reference to the credential — `keychain:`, `env:`, or
    /// `file:` — never the key itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// An ordered pool of credential references, for a provider the user holds
    /// several accounts on.
    ///
    /// Each entry takes the same forms as [`Self::credential`], and the first
    /// is the default active member. Mutually exclusive with
    /// [`Self::credential`], which is the same declaration spelled for one
    /// account and resolves as a pool of one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<String>,
    /// Offer rotation to another pool member once the active member's
    /// server-reported usage reaches this percentage.
    ///
    /// Only meaningful with a pool of more than one. Absent means rotation is
    /// offered on exhaustion alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate_at_percent: Option<u8>,
    /// A plaintext API key accepted only from owner-only user configuration.
    ///
    /// This is mutually exclusive with [`Self::credential`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<ConfigSecret>,
    /// Extra request headers the adapter sends unchanged.
    ///
    /// Authorization-bearing header names are refused during resolution: an
    /// inline token here would be a plaintext secret in a repository file.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Provider-specific response normalization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<ProviderResponseSection>,
}

/// Narrow response-compatibility options for one provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderResponseSection {
    /// How a successful attempt containing only reasoning is classified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_only: Option<ReasoningOnlyBehavior>,
}

/// Classification of a successful reasoning-only provider attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningOnlyBehavior {
    /// Preserve reasoning as reasoning, which is also the omitted default.
    Reasoning,
    /// Emit a non-redacted reasoning-only completion once as visible text.
    Text,
}

impl ReasoningOnlyBehavior {
    /// Parses the stable configuration spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "reasoning" => Some(Self::Reasoning),
            "text" => Some(Self::Text),
            _ => None,
        }
    }

    /// The stable configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reasoning => "reasoning",
            Self::Text => "text",
        }
    }
}

/// Enforceable limits for one `"<provider>/<model>"` pair.
///
/// Each field is independently optional because a catalog source may supply
/// the rest. Smith supplies none of them by default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSection {
    /// Total context window, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,
    /// The largest input Smith may plan for, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    /// The largest output the model can produce, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Exact, owner-controlled reasoning capability metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ModelReasoningSection>,
}

/// A layered request for the provider/model reasoning state.
///
/// Omission preserves the provider default. In particular, Smith never turns
/// reasoning on merely because a model is known to support it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReasoningSection {
    /// Explicit thinking state for subsequent turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Provider-advertised effort name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// Exact request dialect for one trusted provider/model binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningDialect {
    /// OpenAI-compatible top-level `reasoning_effort`.
    OpenaiEffort,
    /// OpenRouter's unified `reasoning` object.
    Openrouter,
    /// Z.AI's `thinking.type` object.
    ZaiThinking,
    /// Gemini's native `generation_config.thinking_level` field.
    GeminiThinking,
}

impl ReasoningDialect {
    /// Every dialect, in documentation order. Parsing and error messages
    /// derive from this list so a new dialect cannot miss either.
    pub const ALL: [Self; 4] = [
        Self::OpenaiEffort,
        Self::Openrouter,
        Self::ZaiThinking,
        Self::GeminiThinking,
    ];

    /// Stable configuration/status spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenaiEffort => "openai-effort",
            Self::Openrouter => "openrouter",
            Self::ZaiThinking => "zai-thinking",
            Self::GeminiThinking => "gemini-thinking",
        }
    }
}

/// Rich control metadata for one exact configured model.
///
/// A Models.dev `reasoning = true` boolean deliberately does not populate
/// this shape: presence is not evidence that an API exposes controls.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelReasoningSection {
    /// Whether an explicit on/off state can be represented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toggle: Option<bool>,
    /// Whether the provider forbids turning reasoning off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mandatory: Option<bool>,
    /// Ordered effort names accepted by this exact binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<String>>,
    /// Provider/model default thinking state, when documented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_enabled: Option<bool>,
    /// Provider/model default effort, when documented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
    /// Exact request-body dialect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialect: Option<ReasoningDialect>,
}

/// Context reserves, sub-budgets, and compaction watermarks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSection {
    /// Tokens held back for the model's response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_reserve: Option<u32>,
    /// Tokens held back for reasoning and continuation input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_reserve: Option<u32>,
    /// A cap on tokens spent on tool schemas and ability instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_budget: Option<u32>,
    /// Reject an estimated plan that lands within this many tokens of the
    /// input budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_estimated_slack: Option<u32>,
    /// The share of the input budget at which compaction becomes due, in
    /// percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_high_watermark_percent: Option<u8>,
    /// The share of the input budget compaction aims to leave behind, in
    /// percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_low_watermark_percent: Option<u8>,
    /// Meaningful inactivity after which the session compacts once, in
    /// milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_compaction_ms: Option<u64>,
    /// Adaptive cache lifecycle policy.  The legacy
    /// `idle_compaction_ms` field above is retained as a bounded alias for
    /// `cache.inactivity_limit_ms` during the transition release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<ContextCacheSection>,
}

/// Loop limits for one turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsSection {
    /// Retries allowed per provider attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    /// Tool calls one turn may make before the loop stops. A value of `0`
    /// removes the ceiling, leaving the tool loop bounded only by the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_steps: Option<u32>,
    /// Wall-clock ceiling for one turn, in milliseconds. A value of `0`
    /// removes the ceiling, leaving the turn without a wall-clock deadline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_time_limit_ms: Option<u64>,
    /// Bytes of tool output kept inline before it is truncated or spilled to a
    /// sidecar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_output_limit_bytes: Option<u64>,
}

/// Whether and where sessions are persisted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PersistenceSection {
    /// Whether sessions are saved at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Where session snapshots and journals live. Defaults to
    /// `~/.smith/sessions`; session state never belongs in the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions_dir: Option<String>,
    /// Whether canonical runtime events are journaled alongside snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_events: Option<bool>,
    /// A 32-byte checkpoint key encoded as 64 hexadecimal characters.
    ///
    /// Accepted only from owner-only user configuration. Environment input
    /// uses the corresponding `SMITH_CHECKPOINT_KEY` setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_key: Option<ConfigSecret>,
    /// Optional protected credential reference for the checkpoint key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_key_credential: Option<String>,
}

/// How tool approval is answered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalSection {
    /// The policy applied to a tool call that is not pre-approved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ApprovalMode>,
    /// Tools that never prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_approve: Option<Vec<String>>,
    /// Prepared-call-scoped automatic approval rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto: Vec<AutoApprovalRuleSection>,
}

/// One revisioned automatic-approval rule as written in configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutoApprovalRuleSection {
    /// Schema revision. Only revision 1 is currently defined.
    pub revision: u32,
    /// Module-qualified tool identity, for example `smith/edit`.
    pub tool: String,
    /// Exact prepared operations this rule covers.
    pub operations: Vec<AutoApprovalOperation>,
    /// Maximum typed permissions the prepared call may request.
    pub permissions: Vec<AutoApprovalPermission>,
    /// Maximum derived risk accepted by the rule.
    pub max_risk: AutoApprovalRisk,
    /// Resource mount class. Scoped rules currently support only `workspace`.
    pub mount: AutoApprovalMount,
    /// Project-relative glob patterns covered by the rule.
    pub paths: Vec<String>,
    /// Optional RFC 3339 expiry timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Optional total number of matching calls allowed for this policy instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u32>,
}

/// Prepared operation names understood by scoped automatic approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoApprovalOperation {
    /// Exact string replacement.
    Replace,
    /// Create a new file.
    Create,
    /// Replace a complete existing file after a versioned read.
    Overwrite,
    /// Delete an existing file after a versioned read.
    Delete,
}

impl AutoApprovalOperation {
    /// Stable prepared-argument spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Create => "create",
            Self::Overwrite => "overwrite",
            Self::Delete => "delete",
        }
    }
}

/// Permission names accepted in a scoped automatic-approval ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum AutoApprovalPermission {
    /// Project filesystem read.
    #[serde(rename = "fs.read")]
    FsRead,
    /// Project filesystem write.
    #[serde(rename = "fs.write")]
    FsWrite,
    /// Project filesystem create.
    #[serde(rename = "fs.create")]
    FsCreate,
    /// Project filesystem delete.
    #[serde(rename = "fs.delete")]
    FsDelete,
    /// Same-user host filesystem read.
    #[serde(rename = "host.fs.read")]
    HostFsRead,
    /// Same-user host filesystem write.
    #[serde(rename = "host.fs.write")]
    HostFsWrite,
    /// External service read.
    #[serde(rename = "external.read")]
    ExternalRead,
    /// External service write.
    #[serde(rename = "external.write")]
    ExternalWrite,
    /// Process creation.
    #[serde(rename = "process.spawn")]
    ProcessSpawn,
    /// Network access.
    #[serde(rename = "net.http")]
    NetHttp,
    /// Data egress.
    #[serde(rename = "data.egress")]
    DataEgress,
    /// Credential use.
    #[serde(rename = "credential.use")]
    CredentialUse,
    /// Standard-input read.
    #[serde(rename = "stdio.read")]
    StdioRead,
    /// Standard-output write.
    #[serde(rename = "stdio.write")]
    StdioWrite,
    /// Clock read.
    #[serde(rename = "clock.read")]
    ClockRead,
    /// Randomness read.
    #[serde(rename = "random.read")]
    RandomRead,
}

/// Coarse risk ceiling for a scoped automatic-approval rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoApprovalRisk {
    /// No meaningful authority.
    None,
    /// Read-only or similarly narrow authority.
    Low,
    /// Reversible project writes.
    Medium,
    /// Deletion or other irreversible authority.
    High,
}

/// Resource classes supported by scoped automatic approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoApprovalMount {
    /// The exact project workspace mount supplied to the runtime.
    Workspace,
}

/// What Smith does with a tool call that needs a decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalMode {
    /// Ask the user. With no surface to ask on, the call is denied.
    #[default]
    Ask,
    /// Deny without asking.
    Deny,
    /// Allow without asking. Only an explicit choice reaches this, because an
    /// unattended run must otherwise fail closed.
    AllowAll,
}

/// Limits and exit behavior for monitors and child agents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackgroundSection {
    /// What a non-interactive run does when work is still active at exit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_policy: Option<BackgroundExit>,
    /// Child agents that may run at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_children: Option<u32>,
    /// Monitors that may run at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_monitors: Option<u32>,
}

/// What to do about still-running background work when a run ends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundExit {
    /// Report the active work and exit non-zero rather than orphan it.
    #[default]
    Error,
    /// Wait for the work to finish.
    Wait,
    /// Stop the work, then exit.
    Stop,
}

impl ApprovalMode {
    /// The spelling used in configuration and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Deny => "deny",
            Self::AllowAll => "allow-all",
        }
    }

    /// Parses the configured spelling.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ask" => Some(Self::Ask),
            "deny" => Some(Self::Deny),
            "allow-all" => Some(Self::AllowAll),
            _ => None,
        }
    }

    /// Every spelling, for diagnostics that list the alternatives.
    pub fn spellings() -> &'static [&'static str] {
        &["ask", "deny", "allow-all"]
    }
}

impl BackgroundExit {
    /// The spelling used in configuration and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Wait => "wait",
            Self::Stop => "stop",
        }
    }

    /// Parses the configured spelling.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "error" => Some(Self::Error),
            "wait" => Some(Self::Wait),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }

    /// Every spelling, for diagnostics that list the alternatives.
    pub fn spellings() -> &'static [&'static str] {
        &["error", "wait", "stop"]
    }
}

/// The adapter kind Smith's first production provider maps to.
pub const KIND_OPENAI_COMPATIBLE: &str = "openai-compatible";

/// The adapter kind for the native Anthropic Messages API.
pub const KIND_ANTHROPIC_MESSAGES: &str = "anthropic-messages";

/// Smith's experimental direct ChatGPT Codex Responses adapter.
pub const KIND_CHATGPT_RESPONSES: &str = "chatgpt-responses";

/// The adapter kind for a stateless OpenAI Responses endpoint.
///
/// Deliberately not named for a vendor: the adapter is generic over the
/// Responses wire protocol and takes its endpoint from `base_url`. xAI's Grok
/// is the first fixture-verified deployment, at `https://api.x.ai/v1`.
pub const KIND_OPENAI_RESPONSES: &str = "openai-responses";

/// The Responses wire protocol carrying a renewable xAI browser login.
///
/// Same wire format as `openai-responses`, different credential: a Grok login
/// yields an OAuth bundle that expires within hours, not a bearer that can be
/// sent verbatim. Splitting the kind is what lets the factory attach a source
/// that unwraps the bundle and renews it; an `openai-responses` provider
/// pointed at a login would put the whole JSON blob in the `Authorization`
/// header, which is exactly the request xAI rejects.
pub const KIND_XAI_RESPONSES: &str = "xai-responses";

/// The native Google Gemini Interactions adapter.
pub const KIND_GEMINI_INTERACTIONS: &str = "gemini-interactions";

/// The endpoint an `anthropic-messages` provider uses when none is configured.
pub const ANTHROPIC_DEFAULT_ENDPOINT: &str = "https://api.anthropic.com/v1";

/// The adapter kind deterministic tests and development runs use.
pub const KIND_FAKE: &str = "fake";

impl ConfigFile {
    /// Parses one file's text.
    ///
    /// The parser's error is returned unchanged: it carries both the span the
    /// caller needs to name a line and the "expected one of" list the caller
    /// needs to suggest a near miss.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_shape_parses() {
        let file = ConfigFile::parse(
            r#"
            default_profile = "work"

            [profiles.work]
            provider = "acme"
            model = "example-model"
            max_output_tokens = 4096

            [providers.acme]
            kind = "openai-compatible"
            base_url = "https://api.example.test/v1"
            credential = "keychain:smith/acme"

            [models."acme/example-model"]
            context_tokens = 128000
            max_input_tokens = 124000
            max_output_tokens = 4096

            [context]
            output_reserve = 4096
            reasoning_reserve = 0
            capability_budget = 12000
            "#,
        )
        .expect("the documented shape");

        assert_eq!(file.default_profile.as_deref(), Some("work"));
        assert_eq!(
            file.profiles["work"].model.as_deref(),
            Some("example-model")
        );
        assert_eq!(
            file.providers["acme"].kind.as_deref(),
            Some(KIND_OPENAI_COMPATIBLE)
        );
        assert_eq!(
            file.models["acme/example-model"].context_tokens,
            Some(128_000)
        );
        assert_eq!(
            file.context.expect("a context table").capability_budget,
            Some(12_000)
        );
    }

    #[test]
    fn an_unknown_key_is_refused_rather_than_ignored() {
        let error = ConfigFile::parse("[context]\noutput_reserv = 10\n").unwrap_err();
        assert!(error.message().contains("unknown field"), "{error}");
        assert!(error.message().contains("output_reserve"), "{error}");
    }

    #[test]
    fn a_wrong_type_names_what_was_expected() {
        let error = ConfigFile::parse("[limits]\nmax_tool_steps = \"lots\"\n").unwrap_err();
        assert!(error.message().contains("invalid type"), "{error}");
    }

    #[test]
    fn nothing_is_configured_by_default() {
        let file = ConfigFile::default();
        assert!(file.models.is_empty());
        assert!(file.context.is_none());
        assert!(file.limits.is_none());
    }

    #[test]
    fn config_secrets_serialize_only_for_the_config_file_and_never_render() {
        let secret = "sk-config-secret-must-not-render";
        let file = ConfigFile {
            providers: BTreeMap::from([(
                "acme".to_owned(),
                ProviderSection {
                    api_key: Some(ConfigSecret::new(secret)),
                    ..ProviderSection::default()
                },
            )]),
            ..ConfigFile::default()
        };

        assert_eq!(
            format!("{}", file.providers["acme"].api_key.as_ref().unwrap()),
            "[redacted]"
        );
        let debug = format!("{file:?}");
        assert!(!debug.contains(secret), "{debug}");
        assert!(debug.contains("[redacted]"), "{debug}");

        let serialized = toml::to_string(&file).expect("a writable user config");
        assert!(serialized.contains(secret), "{serialized}");
        let reparsed = ConfigFile::parse(&serialized).expect("the serialized config");
        assert_eq!(
            reparsed.providers["acme"]
                .api_key
                .as_ref()
                .expect("an inline key")
                .to_secret()
                .expose(),
            secret
        );
    }

    #[test]
    fn policy_spellings_agree_with_the_parsed_form() {
        for spelling in ApprovalMode::spellings() {
            assert_eq!(
                ApprovalMode::parse(spelling)
                    .expect("a known mode")
                    .as_str(),
                *spelling
            );
        }
        for spelling in BackgroundExit::spellings() {
            assert_eq!(
                BackgroundExit::parse(spelling)
                    .expect("a known policy")
                    .as_str(),
                *spelling
            );
        }

        // The hand-written spellings and serde's must agree: diagnostics list
        // one set and files are parsed with the other.
        let file = ConfigFile::parse(
            "[approval]\nmode = \"allow-all\"\n\n[background]\nexit_policy = \"stop\"\n",
        )
        .expect("policies");
        assert_eq!(
            file.approval.expect("approval").mode,
            Some(ApprovalMode::AllowAll)
        );
        assert_eq!(
            file.background.expect("background").exit_policy,
            Some(BackgroundExit::Stop)
        );
    }
}
