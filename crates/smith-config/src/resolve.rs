//! Layering, precedence, provenance, and diagnostics.
//!
//! Resolution answers two questions at once: *what is the value* and *why is it
//! that value*. The second question is not decoration — a user staring at an
//! unexpected model name needs to know which of eight layers supplied it, and a
//! wrong answer there costs more debugging time than the wrong value did.
//!
//! Every layer therefore contributes the same thing: a flat list of
//! `(key, value, source)` triples. The layers are appended in precedence order,
//! lowest first:
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
//! The last contribution for a key wins and the earlier ones are kept as the
//! entries it overrode, which is exactly what `smith config explain <key>`
//! shows. Precedence is one rule applied to one list rather than a merge
//! routine per section, so a new setting cannot accidentally layer differently
//! from its neighbours.
//!
//! Two inputs are injected rather than read from the process: the user root
//! (`~`) and the environment map. Tests then never depend on a real home
//! directory and never mutate process-wide state, and `smith-cli` keeps
//! ownership of real argument parsing.
//!
//! Nothing here executes anything. Declarative project settings may be read
//! before the project is trusted, so this module reads and validates files and
//! stops there: a `credential` is checked for *shape* only, and resolving that
//! reference into a secret happens later, behind the trust boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use agent_runtime_core::store::Secret;
use sha2::{Digest, Sha256};

use crate::model::{
    AgentModeSection, AgentPosture, ApprovalMode, ApprovalSection, BackgroundExit,
    BackgroundSection, ChildAgentSection, ConfigFile, ContextSection, KIND_ANTHROPIC_MESSAGES,
    KIND_FAKE, KIND_OPENAI_COMPATIBLE, LimitsSection, PersistenceSection, ProfileUse,
    ReasoningDialect, ReasoningOnlyBehavior,
};

/// The directory Smith keeps its per-user and per-project state in.
pub const SMITH_DIR: &str = ".smith";

/// The file every layered configuration file is named.
pub const CONFIG_FILE: &str = "config.toml";

/// The project file that is not meant to be committed.
pub const LOCAL_CONFIG_FILE: &str = "config.local.toml";

/// The prefix every Smith environment variable carries.
pub const ENV_PREFIX: &str = "SMITH_";

/// The schemes a `credential` value may use.
///
/// Each names a place a secret can be *fetched from*. A value with no scheme
/// is treated as an inline key and refused.
pub const CREDENTIAL_SCHEMES: &[&str] = &["keychain", "env", "file"];

/// Header names that carry authorization and must never be written inline.
const AUTH_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
];

/// What kind of value a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Text,
    Secret,
    Integer,
    Flag,
    List,
}

/// Every active setting: the keys a profile, environment variable, flag, or
/// session override may address.
///
/// Declarations — `providers.*`, `profiles.*`, `models.*` — are deliberately
/// absent. Those are named tables the user defines in a file; the layers above
/// a file *select* among them rather than redefine them.
const SETTINGS: &[(&str, ValueKind)] = &[
    ("agent", ValueKind::Text),
    ("agent_order", ValueKind::List),
    ("profile_order", ValueKind::List),
    ("approval.auto_approve", ValueKind::List),
    ("approval.mode", ValueKind::Text),
    ("background.exit_policy", ValueKind::Text),
    ("background.max_children", ValueKind::Integer),
    ("background.max_monitors", ValueKind::Integer),
    ("context.capability_budget", ValueKind::Integer),
    (
        "context.compaction_high_watermark_percent",
        ValueKind::Integer,
    ),
    (
        "context.compaction_low_watermark_percent",
        ValueKind::Integer,
    ),
    ("context.idle_compaction_ms", ValueKind::Integer),
    ("context.max_estimated_slack", ValueKind::Integer),
    ("context.output_reserve", ValueKind::Integer),
    ("context.reasoning_reserve", ValueKind::Integer),
    ("limits.max_retries", ValueKind::Integer),
    ("limits.max_tool_steps", ValueKind::Integer),
    ("limits.tool_output_limit_bytes", ValueKind::Integer),
    ("limits.turn_time_limit_ms", ValueKind::Integer),
    ("max_output_tokens", ValueKind::Integer),
    ("model", ValueKind::Text),
    ("persistence.enabled", ValueKind::Flag),
    ("persistence.journal_events", ValueKind::Flag),
    ("persistence.checkpoint_key", ValueKind::Secret),
    ("persistence.checkpoint_key_credential", ValueKind::Text),
    ("persistence.sessions_dir", ValueKind::Text),
    ("profile", ValueKind::Text),
    ("provider", ValueKind::Text),
    ("reasoning.effort", ValueKind::Text),
    ("reasoning.enabled", ValueKind::Flag),
];

/// One layer of the precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// Smith's own defaults. Never model limits.
    BuiltIn,
    /// `~/.smith/config.toml`.
    UserFile,
    /// `<project>/.smith/config.toml`.
    ProjectFile,
    /// `<project>/.smith/config.local.toml`.
    ProjectLocalFile,
    /// The selected profile's own settings.
    Profile,
    /// `SMITH_*` environment variables.
    Environment,
    /// Command-line flags.
    CommandLine,
    /// Explicit per-session overrides, e.g. switching model mid-session.
    SessionOverride,
}

impl Layer {
    /// A short name for diagnostics and status lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::BuiltIn => "built-in default",
            Self::UserFile => "user config",
            Self::ProjectFile => "project config",
            Self::ProjectLocalFile => "project-local config",
            Self::Profile => "profile",
            Self::Environment => "environment",
            Self::CommandLine => "command line",
            Self::SessionOverride => "session override",
        }
    }

    /// The layer's rank, lowest precedence first.
    pub fn precedence(self) -> u8 {
        match self {
            Self::BuiltIn => 0,
            Self::UserFile => 1,
            Self::ProjectFile => 2,
            Self::ProjectLocalFile => 3,
            Self::Profile => 4,
            Self::Environment => 5,
            Self::CommandLine => 6,
            Self::SessionOverride => 7,
        }
    }

    /// Every layer, lowest precedence first.
    pub fn all() -> &'static [Layer] {
        &[
            Self::BuiltIn,
            Self::UserFile,
            Self::ProjectFile,
            Self::ProjectLocalFile,
            Self::Profile,
            Self::Environment,
            Self::CommandLine,
            Self::SessionOverride,
        ]
    }
}

/// Where one value came from: which layer, which file, and which key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The layer that supplied the value.
    pub layer: Layer,
    /// The file it was written in, for the file-backed layers.
    pub file: Option<PathBuf>,
    /// The key as addressed in that layer: a dotted config key, an environment
    /// variable name, or a setting key for flags and session overrides.
    pub key: String,
}

impl Source {
    /// A value Smith supplied itself.
    pub fn built_in(key: impl Into<String>) -> Self {
        Self {
            layer: Layer::BuiltIn,
            file: None,
            key: key.into(),
        }
    }

    /// A value written in a file.
    pub fn file(layer: Layer, path: impl Into<PathBuf>, key: impl Into<String>) -> Self {
        Self {
            layer,
            file: Some(path.into()),
            key: key.into(),
        }
    }

    /// A value taken from an environment variable.
    pub fn environment(name: impl Into<String>) -> Self {
        Self {
            layer: Layer::Environment,
            file: None,
            key: name.into(),
        }
    }

    /// A value supplied by a command-line flag.
    pub fn flag(key: impl Into<String>) -> Self {
        Self {
            layer: Layer::CommandLine,
            file: None,
            key: key.into(),
        }
    }

    /// A value supplied as an explicit session override.
    pub fn session(key: impl Into<String>) -> Self {
        Self {
            layer: Layer::SessionOverride,
            file: None,
            key: key.into(),
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.layer, &self.file) {
            (Layer::BuiltIn, _) => write!(f, "built-in default `{}`", self.key),
            (Layer::Environment, _) => write!(f, "environment variable `{}`", self.key),
            (Layer::CommandLine, _) => {
                write!(f, "command-line flag `--{}`", flag_spelling(&self.key))
            }
            (Layer::SessionOverride, _) => write!(f, "session override `{}`", self.key),
            (Layer::Profile, Some(path)) => write!(
                f,
                "`{}` key `{}` (selected profile)",
                path.display(),
                self.key
            ),
            (_, Some(path)) => write!(f, "`{}` key `{}`", path.display(), self.key),
            (layer, None) => write!(f, "{} `{}`", layer.label(), self.key),
        }
    }
}

/// The flag spelling of a setting key, for diagnostics.
fn flag_spelling(key: &str) -> String {
    key.replace(['.', '_'], "-")
}

/// A value together with the source that supplied it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sourced<T> {
    /// The resolved value.
    pub value: T,
    /// Where it came from.
    pub source: Source,
}

impl<T> Sourced<T> {
    /// Pairs a value with its source.
    pub fn new(value: T, source: Source) -> Self {
        Self { value, source }
    }
}

/// A configured value in the untyped form the layers agree on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    /// A string.
    Text(String),
    /// A secret-bearing string from owner-only user configuration.
    Secret(Secret),
    /// An integer. TOML integers are 64-bit; range checks happen when the
    /// value is read into its typed field.
    Integer(i64),
    /// A boolean.
    Flag(bool),
    /// A list of strings.
    List(Vec<String>),
}

impl SettingValue {
    /// The kind of value this holds.
    fn kind(&self) -> ValueKind {
        match self {
            Self::Text(_) => ValueKind::Text,
            Self::Secret(_) => ValueKind::Secret,
            Self::Integer(_) => ValueKind::Integer,
            Self::Flag(_) => ValueKind::Flag,
            Self::List(_) => ValueKind::List,
        }
    }
}

impl fmt::Display for SettingValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => write!(f, "{text}"),
            Self::Secret(secret) => write!(f, "{secret}"),
            Self::Integer(number) => write!(f, "{number}"),
            Self::Flag(flag) => write!(f, "{flag}"),
            Self::List(items) => write!(f, "{}", items.join(", ")),
        }
    }
}

/// One layer's answer for one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The value that layer supplied.
    pub value: SettingValue,
    /// Where it came from.
    pub source: Source,
}

/// Why a key has the value it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    /// The key that was explained.
    pub key: String,
    /// The winning value.
    pub value: SettingValue,
    /// The source that supplied it.
    pub source: Source,
    /// What it overrode, highest precedence first.
    pub overridden: Vec<Entry>,
}

/// Every key that was configured, with the layers that spoke for it.
///
/// This is what powers `smith config explain <key>`: the data, not its
/// rendering, because the CLI owns presentation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provenance {
    entries: BTreeMap<String, Vec<Entry>>,
}

impl Provenance {
    /// Every configured key, in deterministic order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The source that supplied `key`'s winning value.
    pub fn source(&self, key: &str) -> Option<&Source> {
        self.winner(key).map(|entry| &entry.source)
    }

    /// The winning value for `key`, its source, and the entries it overrode.
    ///
    /// A key with no value is an error rather than an empty answer, and the two
    /// reasons for that are told apart: a setting nobody configured reads as
    /// unset, while a key Smith has never heard of reads as the typo it is.
    pub fn explain(&self, key: &str) -> Result<Explanation, ConfigError> {
        let absent = || {
            if SETTINGS.iter().any(|(setting, _)| *setting == key) {
                ConfigError::MissingSetting {
                    key: key.to_owned(),
                    message: "no layer supplied a value".to_owned(),
                }
            } else {
                ConfigError::UnknownKey {
                    key: key.to_owned(),
                    source: None,
                    location: None,
                    suggestions: nearest(key, self.keys()),
                }
            }
        };
        let entries = self.entries.get(key).ok_or_else(absent)?;
        let winner = entries.last().ok_or_else(absent)?;
        let mut overridden: Vec<Entry> = entries[..entries.len() - 1].to_vec();
        overridden.reverse();
        Ok(Explanation {
            key: key.to_owned(),
            value: winner.value.clone(),
            source: winner.source.clone(),
            overridden,
        })
    }

    fn winner(&self, key: &str) -> Option<&Entry> {
        self.entries.get(key).and_then(|entries| entries.last())
    }

    fn extend(&mut self, contributions: Vec<Contribution>) {
        for contribution in contributions {
            self.entries
                .entry(contribution.key)
                .or_default()
                .push(Entry {
                    value: contribution.value,
                    source: contribution.source,
                });
        }
    }
}

/// One layer's answer for one key, before it enters the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Contribution {
    key: String,
    value: SettingValue,
    source: Source,
}

/// Values a host supplies directly, as already-parsed types.
///
/// The same shape serves command-line flags and per-session overrides: they
/// differ in precedence, not in what they can say. `smith-cli` owns real
/// argument parsing and fills this in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    /// Select a root agent mode by name.
    pub agent: Option<String>,
    /// Select a profile by name.
    pub profile: Option<String>,
    /// Select a declared provider by name.
    pub provider: Option<String>,
    /// Select a model.
    pub model: Option<String>,
    /// The generation cap asked of the provider.
    pub max_output_tokens: Option<u32>,
    /// Explicit thinking state for subsequent turns.
    pub reasoning_enabled: Option<bool>,
    /// Provider-advertised effort name.
    pub reasoning_effort: Option<String>,
    /// Tokens held back for the response.
    pub output_reserve: Option<u32>,
    /// Tokens held back for reasoning and continuation input.
    pub reasoning_reserve: Option<u32>,
    /// Cap on tool-schema and ability tokens.
    pub capability_budget: Option<u32>,
    /// Estimated-plan rejection margin.
    pub max_estimated_slack: Option<u32>,
    /// Compaction's high watermark, in percent of the input budget.
    pub compaction_high_watermark_percent: Option<u8>,
    /// Compaction's low watermark, in percent of the input budget.
    pub compaction_low_watermark_percent: Option<u8>,
    /// Idle time before one automatic compaction, in milliseconds.
    pub idle_compaction_ms: Option<u64>,
    /// Retries per provider attempt.
    pub max_retries: Option<u32>,
    /// Tool calls allowed in one turn.
    pub max_tool_steps: Option<u32>,
    /// Wall-clock ceiling for one turn, in milliseconds.
    pub turn_time_limit_ms: Option<u64>,
    /// Bytes of tool output kept inline.
    pub tool_output_limit_bytes: Option<u64>,
    /// Whether sessions are saved.
    pub persistence_enabled: Option<bool>,
    /// Where sessions are saved.
    pub sessions_dir: Option<String>,
    /// Whether canonical events are journaled.
    pub journal_events: Option<bool>,
    /// How approval is answered.
    pub approval_mode: Option<ApprovalMode>,
    /// Tools that never prompt.
    pub auto_approve: Option<Vec<String>>,
    /// What to do about active background work at exit.
    pub background_exit: Option<BackgroundExit>,
    /// Child agents allowed at once.
    pub max_children: Option<u32>,
    /// Monitors allowed at once.
    pub max_monitors: Option<u32>,
}

impl Overrides {
    /// Turns the set fields into contributions for `layer`.
    fn contributions(&self, layer: Layer) -> Vec<Contribution> {
        let mut out = Vec::new();
        let mut push = |key: &str, value: SettingValue| {
            let source = match layer {
                Layer::SessionOverride => Source::session(key.to_owned()),
                _ => Source::flag(key.to_owned()),
            };
            out.push(Contribution {
                key: key.to_owned(),
                value,
                source,
            });
        };

        if let Some(value) = &self.profile {
            push("profile", SettingValue::Text(value.clone()));
        }
        if let Some(value) = &self.agent {
            push("agent", SettingValue::Text(value.clone()));
        }
        if let Some(value) = &self.provider {
            push("provider", SettingValue::Text(value.clone()));
        }
        if let Some(value) = &self.model {
            push("model", SettingValue::Text(value.clone()));
        }
        if let Some(value) = self.max_output_tokens {
            push("max_output_tokens", SettingValue::Integer(value.into()));
        }
        if let Some(value) = self.reasoning_enabled {
            push("reasoning.enabled", SettingValue::Flag(value));
        }
        if let Some(value) = &self.reasoning_effort {
            push("reasoning.effort", SettingValue::Text(value.clone()));
        }
        if let Some(value) = self.output_reserve {
            push(
                "context.output_reserve",
                SettingValue::Integer(value.into()),
            );
        }
        if let Some(value) = self.reasoning_reserve {
            push(
                "context.reasoning_reserve",
                SettingValue::Integer(value.into()),
            );
        }
        if let Some(value) = self.capability_budget {
            push(
                "context.capability_budget",
                SettingValue::Integer(value.into()),
            );
        }
        if let Some(value) = self.max_estimated_slack {
            push(
                "context.max_estimated_slack",
                SettingValue::Integer(value.into()),
            );
        }
        if let Some(value) = self.compaction_high_watermark_percent {
            push(
                "context.compaction_high_watermark_percent",
                SettingValue::Integer(value.into()),
            );
        }
        if let Some(value) = self.compaction_low_watermark_percent {
            push(
                "context.compaction_low_watermark_percent",
                SettingValue::Integer(value.into()),
            );
        }
        if let Some(value) = self.idle_compaction_ms {
            push(
                "context.idle_compaction_ms",
                SettingValue::Integer(clamp_to_i64(value)),
            );
        }
        if let Some(value) = self.max_retries {
            push("limits.max_retries", SettingValue::Integer(value.into()));
        }
        if let Some(value) = self.max_tool_steps {
            push("limits.max_tool_steps", SettingValue::Integer(value.into()));
        }
        if let Some(value) = self.turn_time_limit_ms {
            push(
                "limits.turn_time_limit_ms",
                SettingValue::Integer(clamp_to_i64(value)),
            );
        }
        if let Some(value) = self.tool_output_limit_bytes {
            push(
                "limits.tool_output_limit_bytes",
                SettingValue::Integer(clamp_to_i64(value)),
            );
        }
        if let Some(value) = self.persistence_enabled {
            push("persistence.enabled", SettingValue::Flag(value));
        }
        if let Some(value) = &self.sessions_dir {
            push(
                "persistence.sessions_dir",
                SettingValue::Text(value.clone()),
            );
        }
        if let Some(value) = self.journal_events {
            push("persistence.journal_events", SettingValue::Flag(value));
        }
        if let Some(value) = self.approval_mode {
            push(
                "approval.mode",
                SettingValue::Text(value.as_str().to_owned()),
            );
        }
        if let Some(value) = &self.auto_approve {
            push("approval.auto_approve", SettingValue::List(value.clone()));
        }
        if let Some(value) = self.background_exit {
            push(
                "background.exit_policy",
                SettingValue::Text(value.as_str().to_owned()),
            );
        }
        if let Some(value) = self.max_children {
            push(
                "background.max_children",
                SettingValue::Integer(value.into()),
            );
        }
        if let Some(value) = self.max_monitors {
            push(
                "background.max_monitors",
                SettingValue::Integer(value.into()),
            );
        }
        out
    }
}

/// A `u64` setting as a TOML integer. Values above `i64::MAX` are millisecond
/// or byte counts no run will reach, so saturating is honest here.
fn clamp_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Everything resolution needs that comes from outside this crate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolveRequest {
    /// Where the project walk starts, usually the working directory.
    pub start_dir: PathBuf,
    /// The user root that holds `.smith`. Injected so tests never read or
    /// write a real home directory; `None` asks the operating system.
    pub home_dir: Option<PathBuf>,
    /// The process environment, injected rather than read, so that tests never
    /// mutate process-wide state.
    pub env: BTreeMap<String, String>,
    /// Values from parsed command-line flags.
    pub cli: Overrides,
    /// Values set explicitly for this session.
    pub session: Overrides,
    /// Placement the selected agent profile must support.
    pub profile_use: ProfileUse,
}

impl ResolveRequest {
    /// A request that walks up from `start_dir` with no environment or
    /// overrides.
    pub fn new(start_dir: impl Into<PathBuf>) -> Self {
        Self {
            start_dir: start_dir.into(),
            ..Self::default()
        }
    }

    /// Uses `home` as the user root instead of the operating system's.
    pub fn with_home_dir(mut self, home: impl Into<PathBuf>) -> Self {
        self.home_dir = Some(home.into());
        self
    }

    /// Supplies the environment to read `SMITH_*` variables from.
    pub fn with_env<K, V>(mut self, env: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.env = env
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        self
    }

    /// Supplies values parsed from command-line flags.
    pub fn with_cli(mut self, cli: Overrides) -> Self {
        self.cli = cli;
        self
    }

    /// Supplies values set explicitly for this session.
    pub fn with_session(mut self, session: Overrides) -> Self {
        self.session = session;
        self
    }

    /// Requires the selected profile to support `placement`.
    pub fn with_profile_use(mut self, placement: ProfileUse) -> Self {
        self.profile_use = placement;
        self
    }
}

/// One configuration file that was found and read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedFile {
    /// The layer it occupies.
    pub layer: Layer,
    /// Where it is.
    pub path: PathBuf,
}

/// What discovery found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The nearest ancestor of the start directory that contains `.smith`.
    pub project_root: Option<PathBuf>,
    /// `<project_root>/.smith`.
    pub project_dir: Option<PathBuf>,
    /// `<home>/.smith`, where user state lives.
    pub user_dir: PathBuf,
    /// The configuration files that existed, lowest layer first.
    pub files: Vec<LoadedFile>,
}

/// A fully resolved run configuration: one typed value per setting, each
/// carrying the source that supplied it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    /// The selected profile, if any layer selected one.
    pub profile: Option<Sourced<String>>,
    /// Selected unified agent profile plus transition-release legacy adapters.
    pub agent: ResolvedAgent,
    /// The selected provider and its options.
    pub provider: ResolvedProvider,
    /// The selected model.
    pub model: Sourced<String>,
    /// The generation cap asked of the provider.
    pub max_output_tokens: Option<Sourced<u32>>,
    /// Configured limits for the selected `"<provider>/<model>"` pair.
    pub model_limits: ResolvedModelLimits,
    /// Layered reasoning defaults. Omitted values preserve provider behavior.
    pub reasoning: ResolvedReasoning,
    /// Exact control metadata for the selected configured model.
    pub model_reasoning: ResolvedModelReasoning,
    /// Context reserves, budgets, and watermarks.
    pub context: ResolvedContext,
    /// Loop limits.
    pub limits: ResolvedLimits,
    /// Session persistence policy.
    pub persistence: ResolvedPersistence,
    /// Approval policy.
    pub approval: ResolvedApproval,
    /// Background-work policy.
    pub background: ResolvedBackground,
}

/// The selected provider, validated against the options its kind supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProvider {
    /// The provider name, as declared in `[providers.<name>]`.
    pub name: Sourced<String>,
    /// The shared adapter kind it maps to.
    pub kind: Sourced<String>,
    /// The endpoint, for kinds that have one.
    pub base_url: Option<Sourced<String>>,
    /// A validated credential *reference*. Never a secret value: this crate
    /// checks the shape and hands the reference on.
    pub credential: Option<Sourced<String>>,
    /// A plaintext user-config key, kept redaction-safe in memory.
    pub api_key: Option<Sourced<Secret>>,
    /// Extra request headers.
    pub headers: BTreeMap<String, Sourced<String>>,
    /// Provider-specific response normalization.
    pub response: ResolvedProviderResponse,
}

/// Resolved response compatibility for one provider.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedProviderResponse {
    /// How a successful attempt containing only reasoning is classified.
    pub reasoning_only: Option<Sourced<ReasoningOnlyBehavior>>,
}

/// Configured limits for the selected model.
///
/// Every field is optional and none is ever defaulted: Smith does not guess a
/// real model's limits. A missing limit is the composition step's problem to
/// solve from a catalog, or to refuse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedModelLimits {
    /// Total context window, in tokens.
    pub context_tokens: Option<Sourced<u32>>,
    /// The largest input Smith may plan for, in tokens.
    pub max_input_tokens: Option<Sourced<u32>>,
    /// The largest output the model can produce, in tokens.
    pub max_output_tokens: Option<Sourced<u32>>,
}

/// Layered, source-explainable reasoning defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedReasoning {
    /// Requested enabled state.
    pub enabled: Option<Sourced<bool>>,
    /// Requested effort name.
    pub effort: Option<Sourced<String>>,
}

/// Exact source-explainable controls for the selected model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedModelReasoning {
    /// Whether explicit on/off is supported.
    pub toggle: Option<Sourced<bool>>,
    /// Whether reasoning is mandatory-on.
    pub mandatory: Option<Sourced<bool>>,
    /// Ordered supported effort names.
    pub efforts: Option<Sourced<Vec<String>>>,
    /// Provider/model default state.
    pub default_enabled: Option<Sourced<bool>>,
    /// Provider/model default effort.
    pub default_effort: Option<Sourced<String>>,
    /// Exact provider request dialect.
    pub dialect: Option<Sourced<ReasoningDialect>>,
}

/// Resolved context policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedContext {
    /// Tokens held back for the response. Model-dependent, so it has no
    /// built-in default.
    pub output_reserve: Option<Sourced<u32>>,
    /// Tokens held back for reasoning and continuation input.
    pub reasoning_reserve: Sourced<u32>,
    /// Cap on tool-schema and ability tokens.
    pub capability_budget: Option<Sourced<u32>>,
    /// Estimated-plan rejection margin.
    pub max_estimated_slack: Option<Sourced<u32>>,
    /// Share of the input budget at which compaction becomes due.
    pub compaction_high_watermark_percent: Sourced<u8>,
    /// Share of the input budget compaction aims to leave behind.
    pub compaction_low_watermark_percent: Sourced<u8>,
    /// Idle time before one automatic compaction, in milliseconds.
    pub idle_compaction_ms: Sourced<u64>,
}

/// Resolved loop limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLimits {
    /// Retries per provider attempt.
    pub max_retries: Sourced<u32>,
    /// Tool calls allowed in one turn.
    pub max_tool_steps: Sourced<u32>,
    /// Wall-clock ceiling for one turn, in milliseconds.
    pub turn_time_limit_ms: Sourced<u64>,
    /// Bytes of tool output kept inline.
    pub tool_output_limit_bytes: Sourced<u64>,
}

/// Resolved persistence policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPersistence {
    /// Whether sessions are saved.
    pub enabled: Sourced<bool>,
    /// Where they are saved.
    pub sessions_dir: Sourced<PathBuf>,
    /// Whether canonical runtime events are journaled.
    pub journal_events: Sourced<bool>,
    /// Explicit no-prompt checkpoint key, when configured.
    pub checkpoint_key: Option<Sourced<Secret>>,
    /// Protected credential reference for a checkpoint key, when configured.
    pub checkpoint_key_credential: Option<Sourced<String>>,
}

/// Resolved agent profiles plus one-release legacy mode/preset adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgent {
    /// Deprecated selected root mode retained by the compatibility adapter.
    pub active: Sourced<String>,
    /// Deprecated root-mode order retained by the compatibility adapter.
    pub order: Sourced<Vec<String>>,
    /// Deprecated root-mode declarations retained for one transition release.
    pub modes: BTreeMap<String, ResolvedAgentMode>,
    /// Deprecated child-preset declarations retained for one transition release.
    pub child_presets: BTreeMap<String, ResolvedChildAgent>,
    /// The effective agent profile applied to this run.
    pub profile: ResolvedAgentProfile,
    /// Every declared or compatibility-adapted agent profile.
    pub profiles: BTreeMap<String, ResolvedAgentProfile>,
    /// Stable user-facing order of main-enabled profiles.
    pub profile_order: Sourced<Vec<String>>,
}

impl ResolvedAgent {
    /// The active profile's authority-narrowing posture.
    pub fn active_posture(&self) -> AgentPosture {
        self.profile.posture.value
    }

    /// Returns the named profile when it is enabled for direct children.
    pub fn child_profile(&self, name: &str) -> Option<&ResolvedAgentProfile> {
        self.profiles
            .get(name)
            .filter(|profile| profile.supports(ProfileUse::Child))
    }
}

/// One resolved reusable agent profile.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedAgentProfile {
    /// Stable profile name.
    pub name: String,
    /// Authority-narrowing posture.
    pub posture: Sourced<AgentPosture>,
    /// Bounded display description.
    pub description: Option<Sourced<String>>,
    /// Bounded additive developer instructions.
    pub instructions: Option<Sourced<String>>,
    /// Placements where the profile is selectable.
    pub uses: Sourced<Vec<ProfileUse>>,
    /// Effective provider preference, when declared or inherited.
    pub provider: Option<Sourced<String>>,
    /// Effective model preference, when declared or inherited.
    pub model: Option<Sourced<String>>,
    /// Deterministic behavior/source revision safe for status and persistence.
    pub revision: String,
    /// Whether this entry came from the transition adapter.
    pub legacy: bool,
}

impl fmt::Debug for ResolvedAgentProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedAgentProfile")
            .field("name", &self.name)
            .field("posture", &self.posture)
            .field("description", &self.description)
            .field("has_instructions", &self.instructions.is_some())
            .field("uses", &self.uses)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("revision", &self.revision)
            .field("legacy", &self.legacy)
            .finish()
    }
}

impl ResolvedAgentProfile {
    /// Whether this profile is available at `placement`.
    pub fn supports(&self, placement: ProfileUse) -> bool {
        self.uses.value.contains(&placement)
    }
}

/// One resolved root-agent mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgentMode {
    /// Authority-narrowing posture.
    pub posture: Sourced<AgentPosture>,
    /// Bounded display description.
    pub description: Option<Sourced<String>>,
}

/// One resolved direct-child preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedChildAgent {
    /// Read-only posture.
    pub posture: Sourced<AgentPosture>,
    /// Bounded display description.
    pub description: Option<Sourced<String>>,
}

/// Resolved approval policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedApproval {
    /// How a call that needs a decision is answered.
    pub mode: Sourced<ApprovalMode>,
    /// Tools that never prompt.
    pub auto_approve: Option<Sourced<Vec<String>>>,
}

/// Resolved background-work policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBackground {
    /// What to do about active work at exit.
    pub exit_policy: Sourced<BackgroundExit>,
    /// Child agents allowed at once.
    pub max_children: Sourced<u32>,
    /// Monitors allowed at once.
    pub max_monitors: Sourced<u32>,
}

/// The result of resolving one run's configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// What discovery found.
    pub layout: Layout,
    /// The typed configuration.
    pub config: ResolvedConfig,
    /// Why each key has the value it has.
    pub provenance: Provenance,
}

/// The result of inspecting configuration before deciding whether setup is
/// appropriate.
///
/// [`ConfigReadiness::Ready`] is exactly the ordinary [`resolve`] result.
/// `Unconfigured` is deliberately narrower than "resolution failed": it is
/// returned only when no layer expresses provider/model setup intent and the
/// missing run selection is the only reason resolution cannot finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigReadiness {
    /// Every required run setting resolved and validated.
    Ready(Box<Resolution>),
    /// No layer contains provider/model setup intent.
    Unconfigured(SetupContext),
    /// Configuration contains intent but cannot be used as written.
    Invalid(ConfigError),
}

/// Locations discovered for a genuinely unconfigured setup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupContext {
    /// The same user/project layout ordinary resolution discovered.
    pub layout: Layout,
}

/// Where in a file something is, counted from one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// The line.
    pub line: u32,
    /// The column.
    pub column: u32,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

/// What an unusable reference was pointing at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// A root agent mode selected by configuration or the TUI.
    AgentMode,
    /// A direct-child preset selected by an explicit user invocation.
    ChildAgent,
    /// A profile named by `default_profile`, a flag, or an override.
    Profile,
    /// A provider named by a profile or a higher layer.
    Provider,
}

impl ReferenceKind {
    /// The word used in diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentMode => "agent mode",
            Self::ChildAgent => "child agent",
            Self::Profile => "profile",
            Self::Provider => "provider",
        }
    }
}

/// A configuration problem, always naming what the user has to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// No user root could be determined and none was supplied.
    NoHomeDirectory,
    /// A file exists but could not be read.
    Unreadable {
        /// The file.
        path: PathBuf,
        /// The operating system's reason.
        message: String,
    },
    /// A file could not be parsed, or held a value of the wrong type.
    Malformed {
        /// The file.
        path: PathBuf,
        /// Where in the file, when the parser reported a position.
        location: Option<Position>,
        /// The parser's explanation.
        message: String,
    },
    /// A key nothing knows about.
    UnknownKey {
        /// The key as written.
        key: String,
        /// Where it was written, when it came from a layer rather than from a
        /// direct lookup.
        source: Option<Source>,
        /// Where in the file, when the parser reported a position.
        location: Option<Position>,
        /// Known keys close enough to be worth suggesting.
        suggestions: Vec<String>,
    },
    /// A key whose value cannot be used as written.
    InvalidValue {
        /// Where the value was written.
        source: Source,
        /// What is wrong with it.
        message: String,
    },
    /// One layer supplied the same setting twice.
    Ambiguous {
        /// The setting they disagree about.
        key: String,
        /// Every source in that layer that set it.
        sources: Vec<Source>,
    },
    /// A secret was written where a reference belongs.
    PlaintextSecret {
        /// Where it was written.
        source: Source,
        /// What to write instead.
        message: String,
    },
    /// A provider option its adapter kind cannot use.
    IncompatibleOption {
        /// Where the option was written.
        source: Source,
        /// The adapter kind it was written for.
        kind: String,
        /// Why the two do not fit.
        message: String,
    },
    /// A reference to a profile or provider that is not defined.
    UnusableReference {
        /// Where the reference was written.
        source: Source,
        /// What kind of thing was referenced.
        what: ReferenceKind,
        /// The name that was referenced.
        name: String,
        /// Names that are defined and close enough to suggest.
        suggestions: Vec<String>,
    },
    /// A setting a run cannot start without.
    MissingSetting {
        /// The setting.
        key: String,
        /// Where a value for it could come from.
        message: String,
    },
    /// A configured value could not be represented as a setting. Reaching this
    /// means the file model and the flattener have drifted apart.
    Unrepresentable {
        /// What could not be represented.
        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHomeDirectory => write!(
                f,
                "no home directory is available; pass an explicit user root"
            ),
            Self::Unreadable { path, message } => {
                write!(f, "`{}` could not be read: {message}", path.display())
            }
            Self::Malformed {
                path,
                location,
                message,
            } => {
                write!(f, "`{}`", path.display())?;
                if let Some(location) = location {
                    write!(f, " {location}")?;
                }
                write!(f, ": {message}")
            }
            Self::UnknownKey {
                key,
                source,
                location,
                suggestions,
            } => {
                match source {
                    Some(source) => write!(f, "{source}")?,
                    None => write!(f, "`{key}`")?,
                }
                if let Some(location) = location {
                    write!(f, " ({location})")?;
                }
                write!(f, " is not a known setting")?;
                write_suggestions(f, suggestions)
            }
            Self::InvalidValue { source, message } => write!(f, "{source}: {message}"),
            Self::Ambiguous { key, sources } => {
                write!(f, "`{key}` is set more than once in the same layer: ")?;
                for (index, source) in sources.iter().enumerate() {
                    if index > 0 {
                        write!(f, " and ")?;
                    }
                    write!(f, "{source}")?;
                }
                Ok(())
            }
            Self::PlaintextSecret { source, message } => write!(
                f,
                "{source} looks like a secret written in plain text: {message}"
            ),
            Self::IncompatibleOption {
                source,
                kind,
                message,
            } => write!(
                f,
                "{source} cannot be used by a `{kind}` provider: {message}"
            ),
            Self::UnusableReference {
                source,
                what,
                name,
                suggestions,
            } => {
                write!(
                    f,
                    "{source} names {} `{name}`, which is not defined",
                    what.as_str()
                )?;
                write_suggestions(f, suggestions)
            }
            Self::MissingSetting { key, message } => {
                write!(f, "`{key}` is not set: {message}")
            }
            Self::Unrepresentable { message } => {
                write!(f, "configuration could not be normalized: {message}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

fn write_suggestions(f: &mut fmt::Formatter<'_>, suggestions: &[String]) -> fmt::Result {
    match suggestions {
        [] => Ok(()),
        [only] => write!(f, "; did you mean `{only}`?"),
        many => {
            write!(f, "; did you mean ")?;
            for (index, suggestion) in many.iter().enumerate() {
                if index > 0 {
                    write!(f, " or ")?;
                }
                write!(f, "`{suggestion}`")?;
            }
            write!(f, "?")
        }
    }
}

/// Resolves one run's configuration.
///
/// The order is deliberate: discover, read files, select a profile, apply the
/// higher layers, then validate. A user must learn that a profile is unusable
/// before Smith bothers to check watermarks, and must learn all of it before a
/// terminal is entered or a request is sent.
pub fn resolve(request: &ResolveRequest) -> Result<Resolution, ConfigError> {
    let layout = discover(request)?;

    let mut declared = Declarations::default();
    let mut file_layers: Vec<Vec<Contribution>> = Vec::new();
    let defaults = built_in_defaults(&layout.user_dir);
    declared.absorb(&defaults, Layer::BuiltIn, None);
    file_layers.push(contributions_of(&defaults, Layer::BuiltIn, None)?);
    for file in &layout.files {
        let loaded = load(&file.path, file.layer)?;
        declared.absorb(&loaded.file, file.layer, Some(&file.path));
        file_layers.push(loaded.contributions);
    }

    let env = env_contributions(&request.env)?;
    let cli = request.cli.contributions(Layer::CommandLine);
    let session = request.session.contributions(Layer::SessionOverride);

    let agent_profiles = resolve_agent_profiles(&file_layers, &declared)?;
    let selected = select_profile(&file_layers, &env, &cli, &session, &declared)?;
    let profile_layer = selected
        .as_ref()
        .map(|profile| profile_contributions(&file_layers, &profile.value, &declared))
        .transpose()?
        .unwrap_or_default();

    let mut provenance = Provenance::default();
    for layer in &file_layers {
        provenance.extend(layer.clone());
    }
    provenance.extend(profile_layer);
    provenance.extend(env);
    provenance.extend(cli);
    provenance.extend(session);
    apply_product_model_defaults(&mut provenance);

    let config = extract(
        &provenance,
        selected,
        &declared,
        agent_profiles,
        request.profile_use,
    )?;
    Ok(Resolution {
        layout,
        config,
        provenance,
    })
}

fn apply_product_model_defaults(provenance: &mut Provenance) {
    if provenance.winner("max_output_tokens").is_some() {
        return;
    }
    let (Some(provider), Some(model)) = (provenance.winner("provider"), provenance.winner("model"))
    else {
        return;
    };
    let (SettingValue::Text(provider), SettingValue::Text(model)) = (&provider.value, &model.value)
    else {
        return;
    };
    let Some(record) = crate::setup::trusted_model(provider, model) else {
        return;
    };
    if record.request_output_tokens == 0 {
        return;
    }
    provenance.extend(vec![Contribution {
        key: "max_output_tokens".to_owned(),
        value: SettingValue::Integer(i64::from(record.request_output_tokens)),
        source: Source::built_in(format!(
            "trusted catalog {}@{} models.\"{}/{}\".request_output_tokens",
            record.catalog, record.revision, record.provider, record.model
        )),
    }]);
}

/// Inspects whether an invocation is ready, genuinely unconfigured, or
/// invalid.
///
/// This is intentionally a wrapper around [`resolve`], rather than a second
/// resolver. A ready result therefore cannot drift from normal startup. When
/// resolution fails, Smith performs only enough declarative inspection to
/// decide whether setup is safe to offer; malformed and partial user intent
/// remains the original error.
pub fn inspect(request: &ResolveRequest) -> ConfigReadiness {
    match resolve(request) {
        Ok(resolution) => ConfigReadiness::Ready(Box::new(resolution)),
        Err(error) => {
            let missing_selection = matches!(
                &error,
                ConfigError::MissingSetting { key, .. }
                    if matches!(key.as_str(), "provider" | "model")
            );
            if missing_selection {
                match setup_intent(request) {
                    Ok((layout, false)) => ConfigReadiness::Unconfigured(SetupContext { layout }),
                    Ok((_, true)) | Err(_) => ConfigReadiness::Invalid(error),
                }
            } else {
                ConfigReadiness::Invalid(error)
            }
        }
    }
}

/// Returns the discovered layout and whether any layer expresses
/// provider/model setup intent.
fn setup_intent(request: &ResolveRequest) -> Result<(Layout, bool), ConfigError> {
    let layout = discover(request)?;
    let mut intent = false;
    for loaded in &layout.files {
        let file = load(&loaded.path, loaded.layer)?.file;
        intent |= file.default_profile.is_some()
            || !file.profiles.is_empty()
            || !file.providers.is_empty()
            || !file.models.is_empty();
    }

    intent |= request.env.keys().any(|name| {
        matches!(
            name.to_ascii_uppercase().as_str(),
            "SMITH_PROFILE" | "SMITH_PROVIDER" | "SMITH_MODEL"
        )
    });
    intent |= request.cli.profile.is_some()
        || request.cli.provider.is_some()
        || request.cli.model.is_some()
        || request.session.profile.is_some()
        || request.session.provider.is_some()
        || request.session.model.is_some();
    Ok((layout, intent))
}

/// Finds the project's `.smith` directory and the user's.
///
/// The project root is the nearest ancestor of the start directory that
/// contains a `.smith` directory. The user root is excluded from that walk: a
/// project opened inside the home directory must not adopt `~/.smith` as its
/// project layer, or user state would silently become project configuration.
fn discover(request: &ResolveRequest) -> Result<Layout, ConfigError> {
    let home = match &request.home_dir {
        Some(home) => home.clone(),
        None => dirs::home_dir().ok_or(ConfigError::NoHomeDirectory)?,
    };
    let home = canonical_or_given(&home);
    let user_dir = home.join(SMITH_DIR);

    let start = canonical_or_given(&request.start_dir);
    let project_root = start
        .ancestors()
        .find(|dir| **dir != *home.as_path() && dir.join(SMITH_DIR).is_dir())
        .map(Path::to_path_buf);
    let project_dir = project_root.as_ref().map(|root| root.join(SMITH_DIR));

    let mut files = Vec::new();
    let user_config = user_dir.join(CONFIG_FILE);
    if user_config.is_file() {
        files.push(LoadedFile {
            layer: Layer::UserFile,
            path: user_config,
        });
    }
    if let Some(dir) = &project_dir {
        let project_config = dir.join(CONFIG_FILE);
        if project_config.is_file() {
            files.push(LoadedFile {
                layer: Layer::ProjectFile,
                path: project_config,
            });
        }
        let local_config = dir.join(LOCAL_CONFIG_FILE);
        if local_config.is_file() {
            files.push(LoadedFile {
                layer: Layer::ProjectLocalFile,
                path: local_config,
            });
        }
    }

    Ok(Layout {
        project_root,
        project_dir,
        user_dir,
        files,
    })
}

/// Resolves symlinks where possible; a path that does not exist yet is used as
/// written so discovery can report what was looked for.
fn canonical_or_given(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Smith's own defaults.
///
/// Deliberately absent: every model limit, and every reserve that depends on
/// one. A default context window would be a claim about a real model that
/// Smith is in no position to make.
fn built_in_defaults(user_dir: &Path) -> ConfigFile {
    ConfigFile {
        default_agent: Some("build".to_owned()),
        agent_order: Some(vec![
            "build".to_owned(),
            "plan".to_owned(),
            "review".to_owned(),
        ]),
        agent_modes: BTreeMap::from([
            (
                "build".to_owned(),
                AgentModeSection {
                    posture: Some(AgentPosture::Build),
                    description: Some("coding workflow with policy-bounded mutation".to_owned()),
                },
            ),
            (
                "plan".to_owned(),
                AgentModeSection {
                    posture: Some(AgentPosture::Plan),
                    description: Some("read-only inspection and planning".to_owned()),
                },
            ),
            (
                "review".to_owned(),
                AgentModeSection {
                    posture: Some(AgentPosture::Review),
                    description: Some("read-only change review and findings".to_owned()),
                },
            ),
        ]),
        child_agents: BTreeMap::from([
            (
                "explore".to_owned(),
                ChildAgentSection {
                    posture: Some(AgentPosture::Plan),
                    description: Some("read-only repository exploration".to_owned()),
                },
            ),
            (
                "review".to_owned(),
                ChildAgentSection {
                    posture: Some(AgentPosture::Review),
                    description: Some("read-only independent review".to_owned()),
                },
            ),
        ]),
        context: Some(ContextSection {
            reasoning_reserve: Some(0),
            compaction_high_watermark_percent: Some(85),
            compaction_low_watermark_percent: Some(60),
            // One hour of meaningful inactivity, per the context-lifetime
            // policy.
            idle_compaction_ms: Some(60 * 60 * 1000),
            ..ContextSection::default()
        }),
        limits: Some(LimitsSection {
            max_retries: Some(2),
            max_tool_steps: Some(64),
            turn_time_limit_ms: Some(10 * 60 * 1000),
            tool_output_limit_bytes: Some(64 * 1024),
        }),
        persistence: Some(PersistenceSection {
            enabled: Some(true),
            sessions_dir: Some(user_dir.join("sessions").to_string_lossy().into_owned()),
            journal_events: Some(true),
            checkpoint_key: None,
            checkpoint_key_credential: None,
        }),
        // Approval and background-work defaults fail closed: ask before acting,
        // and refuse to exit while work is still running.
        approval: Some(ApprovalSection {
            mode: Some(ApprovalMode::Ask),
            auto_approve: None,
        }),
        background: Some(BackgroundSection {
            exit_policy: Some(BackgroundExit::Error),
            max_children: Some(4),
            max_monitors: Some(8),
        }),
        ..ConfigFile::default()
    }
}

/// One file's parsed contents plus the contributions it makes.
struct Loaded {
    file: ConfigFile,
    contributions: Vec<Contribution>,
}

/// The named tables a file declares, and where each was declared.
#[derive(Debug, Default)]
struct Declarations {
    agent_modes: BTreeMap<String, Source>,
    child_agents: BTreeMap<String, Source>,
    profiles: BTreeMap<String, Source>,
    providers: BTreeMap<String, Source>,
}

impl Declarations {
    fn absorb(&mut self, file: &ConfigFile, layer: Layer, path: Option<&Path>) {
        for name in file.agent_modes.keys() {
            let key = join_key(&["agent_modes", name]);
            self.agent_modes
                .insert(name.clone(), source_for(layer, path, key));
        }
        for name in file.child_agents.keys() {
            let key = join_key(&["child_agents", name]);
            self.child_agents
                .insert(name.clone(), source_for(layer, path, key));
        }
        for name in file.profiles.keys() {
            let key = join_key(&["profiles", name]);
            self.profiles
                .insert(name.clone(), source_for(layer, path, key));
        }
        for name in file.providers.keys() {
            let key = join_key(&["providers", name]);
            self.providers
                .insert(name.clone(), source_for(layer, path, key));
        }
    }
}

/// Reads and parses one file.
fn load(path: &Path, layer: Layer) -> Result<Loaded, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|err| ConfigError::Unreadable {
        path: path.to_path_buf(),
        message: err.to_string(),
    })?;
    let file = ConfigFile::parse(&text).map_err(|err| parse_error(path, layer, &text, &err))?;
    validate_inline_secret_file(path, layer, &file)?;
    let contributions = contributions_of(&file, layer, Some(path))?;
    Ok(Loaded {
        file,
        contributions,
    })
}

fn validate_inline_secret_file(
    path: &Path,
    layer: Layer,
    file: &ConfigFile,
) -> Result<(), ConfigError> {
    let has_inline_provider_key = file
        .providers
        .values()
        .any(|provider| provider.api_key.is_some());
    let checkpoint_key = file
        .persistence
        .as_ref()
        .is_some_and(|persistence| persistence.checkpoint_key.is_some());
    let checkpoint_credential = file
        .persistence
        .as_ref()
        .is_some_and(|persistence| persistence.checkpoint_key_credential.is_some());
    if layer != Layer::UserFile && (checkpoint_key || checkpoint_credential) {
        return Err(ConfigError::InvalidValue {
            source: Source::file(layer, path, "persistence.checkpoint_key"),
            message:
                "checkpoint protection is user-scoped; project configuration cannot supply or redirect its key"
                    .to_owned(),
        });
    }

    let has_inline = has_inline_provider_key || checkpoint_key;
    if !has_inline {
        return Ok(());
    }
    let source = Source::file(
        layer,
        path,
        if checkpoint_key {
            "persistence.checkpoint_key"
        } else {
            "providers.<name>.api_key"
        },
    );
    if layer != Layer::UserFile {
        return Err(ConfigError::PlaintextSecret {
            source,
            message: "inline keys are allowed only in `~/.smith/config.toml`; project files must use a credential reference"
                .to_owned(),
        });
    }

    #[cfg(not(unix))]
    {
        return Err(ConfigError::PlaintextSecret {
            source,
            message:
                "inline keys are unavailable because this platform cannot enforce Unix owner-only permissions"
                    .to_owned(),
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let metadata =
            std::fs::symlink_metadata(path).map_err(|error| ConfigError::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ConfigError::PlaintextSecret {
                source,
                message: "an inline key requires a regular, non-symlink user config".to_owned(),
            });
        }
        if metadata.uid() != rustix::process::getuid().as_raw() {
            return Err(ConfigError::PlaintextSecret {
                source,
                message: "an inline key requires a user config owned by the current user"
                    .to_owned(),
            });
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ConfigError::PlaintextSecret {
                source,
                message:
                    "an inline key requires owner-only config permissions; run `chmod 600 ~/.smith/config.toml` and rotate an exposed key"
                        .to_owned(),
            });
        }
    }
    Ok(())
}

/// Turns a parser error into the most specific diagnostic it supports.
///
/// serde reports an unknown field together with the fields it expected, which
/// is exactly the candidate list a "did you mean" needs. When that shape is not
/// present the raw message is still reported, so a parser change degrades the
/// suggestion rather than the diagnostic.
fn parse_error(path: &Path, layer: Layer, text: &str, err: &toml::de::Error) -> ConfigError {
    let location = err.span().map(|span| position(text, span.start));
    let message = err.message();
    if let Some((key, candidates)) = unknown_field(message) {
        return ConfigError::UnknownKey {
            suggestions: nearest(&key, candidates.iter().map(String::as_str)),
            source: Some(Source::file(layer, path, key.clone())),
            key,
            location,
        };
    }
    ConfigError::Malformed {
        path: path.to_path_buf(),
        location,
        message: message.to_owned(),
    }
}

/// Extracts the offending field and the expected fields from serde's
/// unknown-field message.
fn unknown_field(message: &str) -> Option<(String, Vec<String>)> {
    let rest = message.strip_prefix("unknown field `")?;
    let (key, rest) = rest.split_once('`')?;
    let candidates = rest
        .split_once("expected one of ")
        .map(|(_, list)| list)
        .or_else(|| rest.split_once("expected ").map(|(_, list)| list))
        .map(|list| {
            list.split(',')
                .filter_map(|item| item.trim().strip_prefix('`'))
                .filter_map(|item| item.strip_suffix('`'))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some((key.to_owned(), candidates))
}

/// The line and column a byte offset falls on.
fn position(text: &str, offset: usize) -> Position {
    let consumed = &text[..offset.min(text.len())];
    let line = consumed.matches('\n').count() + 1;
    let column = consumed
        .rfind('\n')
        .map_or(consumed.len(), |index| consumed.len() - index - 1)
        + 1;
    Position {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        column: u32::try_from(column).unwrap_or(u32::MAX),
    }
}

/// Flattens a file into one contribution per written value.
///
/// The file is serialized and walked rather than destructured field by field:
/// a new setting in [`crate::model`] then layers like every other setting
/// instead of waiting for someone to remember to wire it up here.
fn contributions_of(
    file: &ConfigFile,
    layer: Layer,
    path: Option<&Path>,
) -> Result<Vec<Contribution>, ConfigError> {
    let table = toml::Table::try_from(file).map_err(|err| ConfigError::Unrepresentable {
        message: err.to_string(),
    })?;
    let mut out = Vec::new();
    flatten(&table, &mut Vec::new(), layer, path, &mut out)?;

    // `default_profile` is how a file selects the active profile, so it is also
    // that file's contribution to the `profile` setting.
    if let Some(name) = &file.default_profile {
        out.push(Contribution {
            key: "profile".to_owned(),
            value: SettingValue::Text(name.clone()),
            source: source_for(layer, path, "default_profile"),
        });
    }
    if let Some(name) = &file.default_agent {
        out.push(Contribution {
            key: "agent".to_owned(),
            value: SettingValue::Text(name.clone()),
            source: source_for(layer, path, "default_agent"),
        });
    }
    Ok(out)
}

fn flatten(
    table: &toml::Table,
    prefix: &mut Vec<String>,
    layer: Layer,
    path: Option<&Path>,
    out: &mut Vec<Contribution>,
) -> Result<(), ConfigError> {
    for (name, value) in table {
        prefix.push(name.clone());
        let result = match value {
            toml::Value::Table(inner) => flatten(inner, prefix, layer, path, out),
            other
                if prefix.last().is_some_and(|segment| {
                    matches!(segment.as_str(), "api_key" | "checkpoint_key")
                }) =>
            {
                let key = join_owned(prefix);
                let source = source_for(layer, path, key.clone());
                if layer != Layer::UserFile {
                    Err(ConfigError::PlaintextSecret {
                        source,
                        message: "inline keys are allowed only in owner-only user configuration"
                            .to_owned(),
                    })
                } else if let Some(value) = other.as_str() {
                    if value.is_empty() {
                        Err(ConfigError::InvalidValue {
                            source,
                            message: "an inline key cannot be empty".to_owned(),
                        })
                    } else {
                        out.push(Contribution {
                            key,
                            value: SettingValue::Secret(Secret::new(value)),
                            source,
                        });
                        Ok(())
                    }
                } else {
                    Err(ConfigError::Unrepresentable {
                        message: format!("`{key}` must be a string"),
                    })
                }
            }
            other => match setting_value(other) {
                Some(setting) => {
                    let key = join_owned(prefix);
                    out.push(Contribution {
                        key: key.clone(),
                        value: setting,
                        source: source_for(layer, path, key),
                    });
                    Ok(())
                }
                None => Err(ConfigError::Unrepresentable {
                    message: format!("`{}` holds an unsupported value", join_owned(prefix)),
                }),
            },
        };
        prefix.pop();
        result?;
    }
    Ok(())
}

fn setting_value(value: &toml::Value) -> Option<SettingValue> {
    match value {
        toml::Value::String(text) => Some(SettingValue::Text(text.clone())),
        toml::Value::Integer(number) => Some(SettingValue::Integer(*number)),
        toml::Value::Boolean(flag) => Some(SettingValue::Flag(*flag)),
        toml::Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<Vec<String>>>()
            .map(SettingValue::List),
        _ => None,
    }
}

fn source_for(layer: Layer, path: Option<&Path>, key: impl Into<String>) -> Source {
    match path {
        Some(path) => Source::file(layer, path, key),
        None => Source::built_in(key),
    }
}

/// Quotes a key segment when it holds anything but the bare-key characters
/// TOML allows, so a key round-trips into `smith config explain`.
fn join_key(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| quote_segment(part))
        .collect::<Vec<_>>()
        .join(".")
}

fn join_owned(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| quote_segment(part))
        .collect::<Vec<_>>()
        .join(".")
}

fn quote_segment(part: &str) -> String {
    let bare = !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        part.to_owned()
    } else {
        format!("\"{}\"", part.replace('"', "\\\""))
    }
}

/// Reads `SMITH_*` variables into contributions.
///
/// Variable names are matched without regard to case, because an environment
/// map can hold both `SMITH_MODEL` and `smith_model` and a run must not depend
/// on which one the map happened to iterate first. Two names for one setting is
/// therefore the same-layer ambiguity this rejects.
fn env_contributions(env: &BTreeMap<String, String>) -> Result<Vec<Contribution>, ConfigError> {
    let mut claimed: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for (name, value) in env {
        let upper = name.to_uppercase();
        if !upper.starts_with(ENV_PREFIX) {
            continue;
        }
        match setting_for_env(&upper) {
            Some(key) => claimed
                .entry(key)
                .or_default()
                .push((name.as_str(), value.as_str())),
            None => {
                return Err(ConfigError::UnknownKey {
                    key: name.clone(),
                    source: Some(Source::environment(name.clone())),
                    location: None,
                    suggestions: nearest_env(&upper),
                });
            }
        }
    }

    let mut out = Vec::new();
    for (key, hits) in claimed {
        if hits.len() > 1 {
            return Err(ConfigError::Ambiguous {
                key: key.to_owned(),
                sources: hits
                    .iter()
                    .map(|(name, _)| Source::environment(*name))
                    .collect(),
            });
        }
        let (name, raw) = hits[0];
        let source = Source::environment(name);
        let value = parse_text(raw, kind_of(key), &source)?;
        out.push(Contribution {
            key: key.to_owned(),
            value,
            source,
        });
    }
    Ok(out)
}

/// The setting an upper-cased environment variable name addresses.
fn setting_for_env(name: &str) -> Option<&'static str> {
    SETTINGS
        .iter()
        .find(|(key, _)| env_name(key) == name)
        .map(|(key, _)| *key)
}

/// The environment variable name for a setting key.
pub fn env_name(key: &str) -> String {
    if key == "persistence.checkpoint_key" {
        return "SMITH_CHECKPOINT_KEY".to_owned();
    }
    format!("{ENV_PREFIX}{}", key.replace('.', "_").to_uppercase())
}

fn nearest_env(name: &str) -> Vec<String> {
    let candidates: Vec<String> = SETTINGS.iter().map(|(key, _)| env_name(key)).collect();
    nearest(name, candidates.iter().map(String::as_str))
}

fn kind_of(key: &str) -> ValueKind {
    SETTINGS
        .iter()
        .find(|(setting, _)| *setting == key)
        .map_or(ValueKind::Text, |(_, kind)| *kind)
}

/// Converts a textual value — an environment variable's — into the setting's
/// kind.
fn parse_text(raw: &str, kind: ValueKind, source: &Source) -> Result<SettingValue, ConfigError> {
    match kind {
        ValueKind::Text => Ok(SettingValue::Text(raw.to_owned())),
        ValueKind::Secret => Ok(SettingValue::Secret(Secret::new(raw))),
        ValueKind::Integer => raw
            .trim()
            .parse::<i64>()
            .map(SettingValue::Integer)
            .map_err(|_| ConfigError::InvalidValue {
                source: source.clone(),
                message: format!("`{raw}` is not a whole number"),
            }),
        ValueKind::Flag => match raw.trim() {
            "true" => Ok(SettingValue::Flag(true)),
            "false" => Ok(SettingValue::Flag(false)),
            other => Err(ConfigError::InvalidValue {
                source: source.clone(),
                message: format!("`{other}` is not `true` or `false`"),
            }),
        },
        ValueKind::List => Ok(SettingValue::List(
            raw.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect(),
        )),
    }
}

/// Picks the profile the run uses, and proves it exists.
fn select_profile(
    file_layers: &[Vec<Contribution>],
    env: &[Contribution],
    cli: &[Contribution],
    session: &[Contribution],
    declared: &Declarations,
) -> Result<Option<Sourced<String>>, ConfigError> {
    let mut winner: Option<&Contribution> = None;
    for layer in file_layers
        .iter()
        .map(Vec::as_slice)
        .chain([env, cli, session])
    {
        if let Some(found) = layer
            .iter()
            .rev()
            .find(|contribution| contribution.key == "profile")
        {
            winner = Some(found);
        }
    }

    let Some(winner) = winner else {
        return Ok(None);
    };
    let SettingValue::Text(name) = &winner.value else {
        return Err(ConfigError::InvalidValue {
            source: winner.source.clone(),
            message: "a profile is named by a string".to_owned(),
        });
    };
    if !declared.profiles.contains_key(name) {
        return Err(ConfigError::UnusableReference {
            source: winner.source.clone(),
            what: ReferenceKind::Profile,
            name: name.clone(),
            suggestions: nearest(name, declared.profiles.keys().map(String::as_str)),
        });
    }
    Ok(Some(Sourced::new(name.clone(), winner.source.clone())))
}

/// Lifts the selected profile's settings into the layer above the files.
///
/// The contribution keeps pointing at the file and key the value was written
/// in — provenance should name where a user can go and change it — while the
/// layer records why it beat the plain file settings.
fn profile_contributions(
    file_layers: &[Vec<Contribution>],
    profile: &str,
    declared: &Declarations,
) -> Result<Vec<Contribution>, ConfigError> {
    const MAX_PROFILE_INHERITANCE_DEPTH: usize = 16;

    fn collect(
        file_layers: &[Vec<Contribution>],
        profile: &str,
        declared: &Declarations,
        stack: &mut Vec<String>,
        depth: usize,
    ) -> Result<BTreeMap<String, Contribution>, ConfigError> {
        let declaration =
            declared
                .profiles
                .get(profile)
                .ok_or_else(|| ConfigError::UnusableReference {
                    source: stack
                        .last()
                        .and_then(|child| declared.profiles.get(child))
                        .cloned()
                        .unwrap_or_else(|| Source::built_in("profile")),
                    what: ReferenceKind::Profile,
                    name: profile.to_owned(),
                    suggestions: nearest(profile, declared.profiles.keys().map(String::as_str)),
                })?;
        if depth >= MAX_PROFILE_INHERITANCE_DEPTH {
            return Err(ConfigError::InvalidValue {
                source: declaration.clone(),
                message: format!(
                    "profile inheritance exceeds {MAX_PROFILE_INHERITANCE_DEPTH} levels"
                ),
            });
        }
        if let Some(start) = stack.iter().position(|name| name == profile) {
            let mut cycle = stack[start..].to_vec();
            cycle.push(profile.to_owned());
            return Err(ConfigError::InvalidValue {
                source: declaration.clone(),
                message: format!("profile inheritance cycle: {}", cycle.join(" -> ")),
            });
        }

        stack.push(profile.to_owned());
        let prefix = format!("{}.", join_key(&["profiles", profile]));
        let mut own = BTreeMap::<String, Contribution>::new();
        for layer in file_layers {
            for contribution in layer {
                let Some(rest) = contribution.key.strip_prefix(&prefix) else {
                    continue;
                };
                own.insert(rest.to_owned(), contribution.clone());
            }
        }

        let parent = match own.remove("extends") {
            Some(contribution) => match contribution.value {
                SettingValue::Text(parent) => Some((parent, contribution.source)),
                other => {
                    stack.pop();
                    return Err(ConfigError::InvalidValue {
                        source: contribution.source,
                        message: format!(
                            "profile `extends` must be a string, found {}",
                            describe(other.kind())
                        ),
                    });
                }
            },
            None => None,
        };

        let mut winners = if let Some((parent, source)) = parent {
            if !declared.profiles.contains_key(&parent) {
                stack.pop();
                return Err(ConfigError::UnusableReference {
                    source,
                    what: ReferenceKind::Profile,
                    name: parent.clone(),
                    suggestions: nearest(&parent, declared.profiles.keys().map(String::as_str)),
                });
            }
            collect(file_layers, &parent, declared, stack, depth + 1)?
        } else {
            BTreeMap::new()
        };
        stack.pop();

        for (key, contribution) in own {
            winners.insert(key, contribution);
        }
        Ok(winners)
    }

    Ok(collect(file_layers, profile, declared, &mut Vec::new(), 0)?
        .into_iter()
        .map(|(key, contribution)| Contribution {
            key,
            value: contribution.value,
            source: Source {
                layer: Layer::Profile,
                file: contribution.source.file,
                key: contribution.source.key,
            },
        })
        .collect())
}

fn resolve_agent_profiles(
    file_layers: &[Vec<Contribution>],
    declared: &Declarations,
) -> Result<BTreeMap<String, ResolvedAgentProfile>, ConfigError> {
    let mut global = Provenance::default();
    for layer in file_layers {
        global.extend(layer.clone());
    }

    let mut profiles = BTreeMap::new();
    for (name, declaration) in &declared.profiles {
        validate_agent_name(name, declaration, "agent profile")?;
        let mut effective = Provenance::default();
        effective.extend(profile_contributions(file_layers, name, declared)?);
        profiles.insert(
            name.clone(),
            resolved_profile(name, declaration, &effective, &global, false)?,
        );
    }

    for (name, declaration) in &declared.agent_modes {
        validate_agent_name(name, declaration, "agent mode")?;
        let scope = join_key(&["agent_modes", name]);
        let posture = resolve_agent_posture(&global, &format!("{scope}.posture"), declaration)?;
        let description = bounded_description(&global, &format!("{scope}.description"))?;
        let uses = Sourced::new(vec![ProfileUse::Main], declaration.clone());
        let revision = agent_profile_revision(
            name,
            &posture,
            description.as_ref(),
            None,
            &uses,
            None,
            None,
            true,
        );
        merge_legacy_profile(
            &mut profiles,
            ResolvedAgentProfile {
                name: name.clone(),
                posture,
                description,
                instructions: None,
                uses,
                provider: None,
                model: None,
                revision,
                legacy: true,
            },
            declaration,
        )?;
    }

    for (name, declaration) in &declared.child_agents {
        validate_agent_name(name, declaration, "child agent")?;
        let scope = join_key(&["child_agents", name]);
        let posture = resolve_agent_posture(&global, &format!("{scope}.posture"), declaration)?;
        if !posture.value.is_read_only() {
            return Err(ConfigError::InvalidValue {
                source: posture.source,
                message: format!(
                    "child agent `{name}` must use a read-only `plan` or `review` posture"
                ),
            });
        }
        let description = bounded_description(&global, &format!("{scope}.description"))?;
        let uses = Sourced::new(vec![ProfileUse::Child], declaration.clone());
        let revision = agent_profile_revision(
            name,
            &posture,
            description.as_ref(),
            None,
            &uses,
            None,
            None,
            true,
        );
        merge_legacy_profile(
            &mut profiles,
            ResolvedAgentProfile {
                name: name.clone(),
                posture,
                description,
                instructions: None,
                uses,
                provider: None,
                model: None,
                revision,
                legacy: true,
            },
            declaration,
        )?;
    }
    Ok(profiles)
}

fn resolved_profile(
    name: &str,
    declaration: &Source,
    effective: &Provenance,
    global: &Provenance,
    legacy: bool,
) -> Result<ResolvedAgentProfile, ConfigError> {
    let posture = match text(effective, "posture")? {
        Some(raw) => {
            let posture =
                AgentPosture::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
                    source: raw.source.clone(),
                    message: format!(
                        "`{}` is not an agent posture; the postures are {}",
                        raw.value,
                        list_spellings(AgentPosture::spellings())
                    ),
                })?;
            Sourced::new(posture, raw.source)
        }
        None => match text(effective, "agent")? {
            Some(mode) => {
                let mode_key = format!("{}.posture", join_key(&["agent_modes", &mode.value]));
                let mode_source = global.source(&mode_key).cloned().ok_or_else(|| {
                    ConfigError::UnusableReference {
                        source: mode.source.clone(),
                        what: ReferenceKind::AgentMode,
                        name: mode.value.clone(),
                        suggestions: Vec::new(),
                    }
                })?;
                resolve_agent_posture(global, &mode_key, &mode_source)?
            }
            None => match text(global, "agent")? {
                Some(mode) => {
                    let mode_key = format!("{}.posture", join_key(&["agent_modes", &mode.value]));
                    let mode_source = global.source(&mode_key).cloned().ok_or_else(|| {
                        ConfigError::UnusableReference {
                            source: mode.source.clone(),
                            what: ReferenceKind::AgentMode,
                            name: mode.value.clone(),
                            suggestions: Vec::new(),
                        }
                    })?;
                    resolve_agent_posture(global, &mode_key, &mode_source)?
                }
                None => Sourced::new(AgentPosture::Build, declaration.clone()),
            },
        },
    };
    let description = bounded_description(effective, "description")?;
    let instructions = bounded_instructions(effective, "instructions")?;
    let uses = profile_uses(effective, declaration)?;
    let provider = text(effective, "provider")?;
    let model = text(effective, "model")?;
    let revision = agent_profile_revision(
        name,
        &posture,
        description.as_ref(),
        instructions.as_ref(),
        &uses,
        provider.as_ref(),
        model.as_ref(),
        legacy,
    );
    Ok(ResolvedAgentProfile {
        name: name.to_owned(),
        posture,
        description,
        instructions,
        uses,
        provider,
        model,
        revision,
        legacy,
    })
}

fn merge_legacy_profile(
    profiles: &mut BTreeMap<String, ResolvedAgentProfile>,
    incoming: ResolvedAgentProfile,
    source: &Source,
) -> Result<(), ConfigError> {
    let Some(existing) = profiles.get_mut(&incoming.name) else {
        profiles.insert(incoming.name.clone(), incoming);
        return Ok(());
    };
    if !existing.legacy {
        if source.layer == Layer::BuiltIn {
            return Ok(());
        }
        return Err(ConfigError::Ambiguous {
            key: format!("profiles.{}", quote_segment(&incoming.name)),
            sources: vec![existing.posture.source.clone(), source.clone()],
        });
    }
    if existing.posture.value != incoming.posture.value {
        return Err(ConfigError::Ambiguous {
            key: format!("legacy agent profile `{}`", incoming.name),
            sources: vec![existing.posture.source.clone(), source.clone()],
        });
    }
    for placement in incoming.uses.value {
        if !existing.uses.value.contains(&placement) {
            existing.uses.value.push(placement);
        }
    }
    existing.uses.value.sort();
    existing.revision = agent_profile_revision(
        &existing.name,
        &existing.posture,
        existing.description.as_ref(),
        existing.instructions.as_ref(),
        &existing.uses,
        existing.provider.as_ref(),
        existing.model.as_ref(),
        true,
    );
    Ok(())
}

fn profile_uses(
    provenance: &Provenance,
    declaration: &Source,
) -> Result<Sourced<Vec<ProfileUse>>, ConfigError> {
    let Some(raw) = list(provenance, "use")? else {
        return Ok(Sourced::new(vec![ProfileUse::Main], declaration.clone()));
    };
    if raw.value.is_empty() {
        return Err(ConfigError::InvalidValue {
            source: raw.source,
            message: "profile `use` must contain `main`, `child`, or both".to_owned(),
        });
    }
    let mut seen = BTreeSet::new();
    let mut uses = Vec::new();
    for value in raw.value {
        let placement = ProfileUse::parse(&value).ok_or_else(|| ConfigError::InvalidValue {
            source: raw.source.clone(),
            message: format!(
                "`{value}` is not a profile placement; expected {}",
                list_spellings(ProfileUse::spellings())
            ),
        })?;
        if !seen.insert(placement) {
            return Err(ConfigError::InvalidValue {
                source: raw.source.clone(),
                message: format!("profile `use` contains duplicate `{value}`"),
            });
        }
        uses.push(placement);
    }
    Ok(Sourced::new(uses, raw.source))
}

fn bounded_instructions(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<String>>, ConfigError> {
    const MAX_PROFILE_INSTRUCTIONS_BYTES: usize = 32 * 1024;
    let instructions = text(provenance, key)?;
    if let Some(instructions) = &instructions
        && (instructions.value.trim().is_empty()
            || instructions.value.len() > MAX_PROFILE_INSTRUCTIONS_BYTES)
    {
        return Err(ConfigError::InvalidValue {
            source: instructions.source.clone(),
            message: format!(
                "profile instructions must contain 1 to {MAX_PROFILE_INSTRUCTIONS_BYTES} UTF-8 bytes"
            ),
        });
    }
    Ok(instructions)
}

#[allow(clippy::too_many_arguments)]
fn agent_profile_revision(
    name: &str,
    posture: &Sourced<AgentPosture>,
    description: Option<&Sourced<String>>,
    instructions: Option<&Sourced<String>>,
    uses: &Sourced<Vec<ProfileUse>>,
    provider: Option<&Sourced<String>>,
    model: Option<&Sourced<String>>,
    legacy: bool,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"smith-agent-profile-1\0");
    digest.update(name.as_bytes());
    digest.update([0]);
    digest.update(posture.value.as_str().as_bytes());
    digest.update([0]);
    digest.update(posture.source.to_string().as_bytes());
    for placement in &uses.value {
        digest.update([0]);
        digest.update(placement.as_str().as_bytes());
    }
    digest.update([0]);
    digest.update(uses.source.to_string().as_bytes());
    for value in [description, instructions, provider, model]
        .into_iter()
        .flatten()
    {
        digest.update([0]);
        digest.update(value.value.as_bytes());
        digest.update([0]);
        digest.update(value.source.to_string().as_bytes());
    }
    digest.update([u8::from(legacy)]);
    format!("{:x}", digest.finalize())
}

/// Reads the ledger into the typed configuration and validates it.
fn extract(
    provenance: &Provenance,
    profile: Option<Sourced<String>>,
    declared: &Declarations,
    agent_profiles: BTreeMap<String, ResolvedAgentProfile>,
    profile_use: ProfileUse,
) -> Result<ResolvedConfig, ConfigError> {
    let provider_name =
        text(provenance, "provider")?.ok_or_else(|| ConfigError::MissingSetting {
            key: "provider".to_owned(),
            message: "select one with a profile, `SMITH_PROVIDER`, or `--provider`".to_owned(),
        })?;
    if !declared.providers.contains_key(&provider_name.value) {
        return Err(ConfigError::UnusableReference {
            suggestions: nearest(
                &provider_name.value,
                declared.providers.keys().map(String::as_str),
            ),
            source: provider_name.source,
            what: ReferenceKind::Provider,
            name: provider_name.value,
        });
    }
    let model = text(provenance, "model")?.ok_or_else(|| ConfigError::MissingSetting {
        key: "model".to_owned(),
        message: "select one with a profile, `SMITH_MODEL`, or `--model`".to_owned(),
    })?;

    let agent = resolve_agent(
        provenance,
        declared,
        profile.as_ref(),
        agent_profiles,
        profile_use,
    )?;
    let provider = resolve_provider(provenance, provider_name)?;
    let model_limits = resolve_model_limits(provenance, &provider.name.value, &model.value)?;
    let reasoning = ResolvedReasoning {
        enabled: flag(provenance, "reasoning.enabled")?,
        effort: text(provenance, "reasoning.effort")?,
    };
    let model_reasoning = resolve_model_reasoning(provenance, &provider.name.value, &model.value)?;
    let context = resolve_context(provenance)?;
    let limits = resolve_limits(provenance)?;
    let persistence = resolve_persistence(provenance)?;
    let approval = resolve_approval(provenance)?;
    let background = resolve_background(provenance)?;

    Ok(ResolvedConfig {
        profile,
        agent,
        provider,
        model,
        max_output_tokens: optional_u32(provenance, "max_output_tokens")?,
        model_limits,
        reasoning,
        model_reasoning,
        context,
        limits,
        persistence,
        approval,
        background,
    })
}

fn resolve_agent(
    provenance: &Provenance,
    declared: &Declarations,
    selected_profile: Option<&Sourced<String>>,
    profiles: BTreeMap<String, ResolvedAgentProfile>,
    profile_use: ProfileUse,
) -> Result<ResolvedAgent, ConfigError> {
    let active = required_text(provenance, "agent")?;
    if !declared.agent_modes.contains_key(&active.value) {
        return Err(ConfigError::UnusableReference {
            source: active.source,
            what: ReferenceKind::AgentMode,
            name: active.value.clone(),
            suggestions: nearest(
                &active.value,
                declared.agent_modes.keys().map(String::as_str),
            ),
        });
    }

    let order = list(provenance, "agent_order")?.ok_or_else(|| missing("agent_order"))?;
    if order.value.is_empty() {
        return Err(ConfigError::InvalidValue {
            source: order.source,
            message: "`agent_order` must contain at least one declared mode".to_owned(),
        });
    }
    let mut seen = BTreeSet::new();
    for name in &order.value {
        if !seen.insert(name) {
            return Err(ConfigError::InvalidValue {
                source: order.source.clone(),
                message: format!("`agent_order` contains duplicate mode `{name}`"),
            });
        }
        if !declared.agent_modes.contains_key(name) {
            return Err(ConfigError::UnusableReference {
                source: order.source.clone(),
                what: ReferenceKind::AgentMode,
                name: name.clone(),
                suggestions: nearest(name, declared.agent_modes.keys().map(String::as_str)),
            });
        }
    }
    if !seen.contains(&active.value) {
        return Err(ConfigError::InvalidValue {
            source: order.source.clone(),
            message: format!(
                "active agent mode `{}` must appear in `agent_order`",
                active.value
            ),
        });
    }

    let mut modes = BTreeMap::new();
    for (name, declaration) in &declared.agent_modes {
        validate_agent_name(name, declaration, "agent mode")?;
        let scope = join_key(&["agent_modes", name]);
        let posture = resolve_agent_posture(provenance, &format!("{scope}.posture"), declaration)?;
        let description = bounded_description(provenance, &format!("{scope}.description"))?;
        modes.insert(
            name.clone(),
            ResolvedAgentMode {
                posture,
                description,
            },
        );
    }

    let mut child_presets = BTreeMap::new();
    for (name, declaration) in &declared.child_agents {
        validate_agent_name(name, declaration, "child agent")?;
        let scope = join_key(&["child_agents", name]);
        let posture = resolve_agent_posture(provenance, &format!("{scope}.posture"), declaration)?;
        if !posture.value.is_read_only() {
            return Err(ConfigError::InvalidValue {
                source: posture.source,
                message: format!(
                    "child agent `{name}` must use a read-only `plan` or `review` posture"
                ),
            });
        }
        let description = bounded_description(provenance, &format!("{scope}.description"))?;
        child_presets.insert(
            name.clone(),
            ResolvedChildAgent {
                posture,
                description,
            },
        );
    }

    let profile = match selected_profile {
        Some(selected) => profiles
            .get(&selected.value)
            .cloned()
            .expect("selected profile was resolved from declared profiles"),
        None => profiles
            .get(&active.value)
            .cloned()
            .unwrap_or_else(|| ResolvedAgentProfile {
                name: active.value.clone(),
                posture: modes
                    .get(&active.value)
                    .expect("resolved active agent is declared")
                    .posture
                    .clone(),
                description: modes
                    .get(&active.value)
                    .and_then(|mode| mode.description.clone()),
                instructions: None,
                uses: Sourced::new(vec![ProfileUse::Main], active.source.clone()),
                provider: None,
                model: None,
                revision: agent_profile_revision(
                    &active.value,
                    &modes
                        .get(&active.value)
                        .expect("resolved active agent is declared")
                        .posture,
                    modes
                        .get(&active.value)
                        .and_then(|mode| mode.description.as_ref()),
                    None,
                    &Sourced::new(vec![ProfileUse::Main], active.source.clone()),
                    None,
                    None,
                    true,
                ),
                legacy: true,
            }),
    };
    if !profile.supports(profile_use) {
        return Err(ConfigError::InvalidValue {
            source: selected_profile.map_or_else(
                || profile.uses.source.clone(),
                |selected| selected.source.clone(),
            ),
            message: format!(
                "profile `{}` is not enabled for {}-agent use",
                profile.name,
                profile_use.as_str()
            ),
        });
    }

    let profile_order = match list(provenance, "profile_order")? {
        Some(order) => {
            if order.value.is_empty() {
                return Err(ConfigError::InvalidValue {
                    source: order.source,
                    message: "`profile_order` must contain at least one main-enabled profile"
                        .to_owned(),
                });
            }
            let mut seen = BTreeSet::new();
            for name in &order.value {
                if !seen.insert(name) {
                    return Err(ConfigError::InvalidValue {
                        source: order.source.clone(),
                        message: format!("`profile_order` contains duplicate profile `{name}`"),
                    });
                }
                let Some(candidate) = profiles.get(name) else {
                    return Err(ConfigError::UnusableReference {
                        source: order.source.clone(),
                        what: ReferenceKind::Profile,
                        name: name.clone(),
                        suggestions: nearest(name, profiles.keys().map(String::as_str)),
                    });
                };
                if !candidate.supports(ProfileUse::Main) {
                    return Err(ConfigError::InvalidValue {
                        source: order.source.clone(),
                        message: format!("`profile_order` names child-only profile `{name}`"),
                    });
                }
            }
            if profile_use == ProfileUse::Main && !seen.contains(&profile.name) {
                return Err(ConfigError::InvalidValue {
                    source: order.source.clone(),
                    message: format!(
                        "active profile `{}` must appear in `profile_order`",
                        profile.name
                    ),
                });
            }
            order
        }
        None => {
            let mut order = profiles
                .values()
                .filter(|candidate| !candidate.legacy && candidate.supports(ProfileUse::Main))
                .map(|candidate| candidate.name.clone())
                .collect::<Vec<_>>();
            if !order.contains(&profile.name) {
                order.insert(0, profile.name.clone());
            }
            Sourced::new(
                order,
                selected_profile
                    .map_or_else(|| profile.uses.source.clone(), |value| value.source.clone()),
            )
        }
    };

    Ok(ResolvedAgent {
        active,
        order,
        modes,
        child_presets,
        profile,
        profiles,
        profile_order,
    })
}

fn validate_agent_name(name: &str, source: &Source, kind: &str) -> Result<(), ConfigError> {
    if name.is_empty()
        || name.len() > 32
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ConfigError::InvalidValue {
            source: source.clone(),
            message: format!(
                "a {kind} name must contain 1 to 32 ASCII letters, digits, `-`, or `_`"
            ),
        });
    }
    Ok(())
}

fn resolve_agent_posture(
    provenance: &Provenance,
    key: &str,
    declaration: &Source,
) -> Result<Sourced<AgentPosture>, ConfigError> {
    let raw = text(provenance, key)?.ok_or_else(|| ConfigError::InvalidValue {
        source: declaration.clone(),
        message: "an agent definition must declare a `posture`".to_owned(),
    })?;
    let posture = AgentPosture::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
        source: raw.source.clone(),
        message: format!(
            "`{}` is not an agent posture; the postures are {}",
            raw.value,
            list_spellings(AgentPosture::spellings())
        ),
    })?;
    Ok(Sourced::new(posture, raw.source))
}

fn bounded_description(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<String>>, ConfigError> {
    let description = text(provenance, key)?;
    if let Some(description) = &description
        && (description.value.is_empty() || description.value.chars().count() > 160)
    {
        return Err(ConfigError::InvalidValue {
            source: description.source.clone(),
            message: "an agent description must contain 1 to 160 characters".to_owned(),
        });
    }
    Ok(description)
}

fn resolve_provider(
    provenance: &Provenance,
    name: Sourced<String>,
) -> Result<ResolvedProvider, ConfigError> {
    let scope = join_key(&["providers", name.value.as_str()]);
    let kind =
        text(provenance, &format!("{scope}.kind"))?.ok_or_else(|| ConfigError::MissingSetting {
            key: format!("{scope}.kind"),
            message: format!(
                "provider `{}` must name the shared adapter it uses",
                name.value
            ),
        })?;
    let base_url = text(provenance, &format!("{scope}.base_url"))?;
    let credential = text(provenance, &format!("{scope}.credential"))?;
    let api_key = secret(provenance, &format!("{scope}.api_key"))?;
    let reasoning_only = text(provenance, &format!("{scope}.response.reasoning_only"))?
        .map(|value| {
            ReasoningOnlyBehavior::parse(&value.value)
                .map(|behavior| Sourced::new(behavior, value.source.clone()))
                .ok_or_else(|| ConfigError::InvalidValue {
                    source: value.source,
                    message: "`reasoning_only` must be `reasoning` or `text`".to_owned(),
                })
        })
        .transpose()?;

    let header_prefix = format!("{scope}.headers.");
    let header_keys: Vec<String> = provenance
        .keys()
        .filter(|key| key.starts_with(&header_prefix))
        .map(str::to_owned)
        .collect();
    let mut headers = BTreeMap::new();
    for key in header_keys {
        let header = unquote_segment(&key[header_prefix.len()..]);
        if let Some(value) = text(provenance, &key)? {
            headers.insert(header, value);
        }
    }

    let provider = ResolvedProvider {
        name,
        kind,
        base_url,
        credential,
        api_key,
        headers,
        response: ResolvedProviderResponse { reasoning_only },
    };
    validate_provider(&provider)?;
    Ok(provider)
}

/// Checks the options against what the adapter kind can use.
///
/// Kinds Smith does not know are not rejected here: which adapters exist is a
/// property of the pinned runtime's registry, and reporting an unavailable
/// adapter belongs to the step that consults it. The secret rules apply to
/// every kind, because they protect the file rather than the adapter.
fn validate_provider(provider: &ResolvedProvider) -> Result<(), ConfigError> {
    if provider.credential.is_some() && provider.api_key.is_some() {
        let source = provider
            .api_key
            .as_ref()
            .map(|value| value.source.clone())
            .expect("the inline key was checked as present");
        return Err(ConfigError::InvalidValue {
            source,
            message: "choose exactly one credential source: `credential` or `api_key`".to_owned(),
        });
    }
    if let Some(credential) = &provider.credential {
        validate_credential(credential)?;
    }
    for (name, value) in &provider.headers {
        if AUTH_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(ConfigError::PlaintextSecret {
                source: value.source.clone(),
                message: format!(
                    "set the provider's `credential` reference instead of writing an `{name}` header"
                ),
            });
        }
    }
    if !matches!(
        provider.kind.value.as_str(),
        KIND_OPENAI_COMPATIBLE | KIND_ANTHROPIC_MESSAGES
    ) && let Some(policy) = &provider.response.reasoning_only
    {
        return Err(ConfigError::IncompatibleOption {
            source: policy.source.clone(),
            kind: provider.kind.value.clone(),
            message: "`response.reasoning_only` requires an adapter that exposes reasoning events"
                .to_owned(),
        });
    }

    match provider.kind.value.as_str() {
        KIND_OPENAI_COMPATIBLE => {
            if provider.base_url.is_none() {
                return Err(ConfigError::MissingSetting {
                    key: join_key(&["providers", &provider.name.value, "base_url"]),
                    message: format!(
                        "an `{KIND_OPENAI_COMPATIBLE}` provider needs the endpoint it talks to"
                    ),
                });
            }
        }
        // The Messages API has one well-known endpoint, so `base_url` is
        // optional and defaults at provider construction; everything else a
        // provider table carries (credential, headers) applies unchanged.
        KIND_ANTHROPIC_MESSAGES => {}
        KIND_FAKE => {
            for (source, option) in [
                (provider.base_url.as_ref(), "base_url"),
                (provider.credential.as_ref(), "credential"),
            ] {
                if let Some(sourced) = source {
                    return Err(ConfigError::IncompatibleOption {
                        source: sourced.source.clone(),
                        kind: KIND_FAKE.to_owned(),
                        message: format!(
                            "the deterministic provider sends nothing, so `{option}` would never be used"
                        ),
                    });
                }
            }
            if let Some(api_key) = &provider.api_key {
                return Err(ConfigError::IncompatibleOption {
                    source: api_key.source.clone(),
                    kind: KIND_FAKE.to_owned(),
                    message:
                        "the deterministic provider sends nothing, so `api_key` would never be used"
                            .to_owned(),
                });
            }
            if let Some(value) = provider.headers.values().next() {
                return Err(ConfigError::IncompatibleOption {
                    source: value.source.clone(),
                    kind: KIND_FAKE.to_owned(),
                    message: "the deterministic provider sends no requests to add headers to"
                        .to_owned(),
                });
            }
        }
        _ => {}
    }
    Ok(())
}

/// Checks that a credential is a reference rather than the secret itself.
fn validate_credential(credential: &Sourced<String>) -> Result<(), ConfigError> {
    let refused = || ConfigError::PlaintextSecret {
        source: credential.source.clone(),
        message: format!(
            "write a reference such as `keychain:smith/<provider>`; the schemes are {}",
            CREDENTIAL_SCHEMES
                .iter()
                .map(|scheme| format!("`{scheme}:`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let Some((scheme, rest)) = credential.value.split_once(':') else {
        return Err(refused());
    };
    if !CREDENTIAL_SCHEMES.contains(&scheme) || rest.trim().is_empty() {
        return Err(refused());
    }
    Ok(())
}

fn resolve_model_limits(
    provenance: &Provenance,
    provider: &str,
    model: &str,
) -> Result<ResolvedModelLimits, ConfigError> {
    let scope = join_key(&["models", &format!("{provider}/{model}")]);
    Ok(ResolvedModelLimits {
        context_tokens: optional_u32(provenance, &format!("{scope}.context_tokens"))?,
        max_input_tokens: optional_u32(provenance, &format!("{scope}.max_input_tokens"))?,
        max_output_tokens: optional_u32(provenance, &format!("{scope}.max_output_tokens"))?,
    })
}

fn resolve_model_reasoning(
    provenance: &Provenance,
    provider: &str,
    model: &str,
) -> Result<ResolvedModelReasoning, ConfigError> {
    let scope = format!(
        "{}.reasoning",
        join_key(&["models", &format!("{provider}/{model}")])
    );
    let dialect = text(provenance, &format!("{scope}.dialect"))?
        .map(|raw| {
            let Some(value) = ReasoningDialect::ALL
                .into_iter()
                .find(|dialect| dialect.as_str() == raw.value)
            else {
                let supported = ReasoningDialect::ALL
                    .map(|dialect| format!("`{}`", dialect.as_str()))
                    .join(", ");
                return Err(ConfigError::InvalidValue {
                    source: raw.source,
                    message: format!(
                        "`{}` is not a reasoning dialect; use {supported}",
                        raw.value
                    ),
                });
            };
            Ok(Sourced::new(value, raw.source))
        })
        .transpose()?;
    let efforts = list(provenance, &format!("{scope}.efforts"))?;
    if let Some(efforts) = &efforts {
        if efforts.value.is_empty() {
            return Err(ConfigError::InvalidValue {
                source: efforts.source.clone(),
                message: "`efforts` must contain at least one advertised value".to_owned(),
            });
        }
        let mut seen = BTreeSet::new();
        for effort in &efforts.value {
            if effort.is_empty()
                || effort.len() > 32
                || !effort
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(ConfigError::InvalidValue {
                    source: efforts.source.clone(),
                    message: format!(
                        "reasoning effort `{effort}` must contain 1 to 32 ASCII letters, digits, `-`, or `_`"
                    ),
                });
            }
            if !seen.insert(effort) {
                return Err(ConfigError::InvalidValue {
                    source: efforts.source.clone(),
                    message: format!("`efforts` contains duplicate value `{effort}`"),
                });
            }
        }
    }
    Ok(ResolvedModelReasoning {
        toggle: flag(provenance, &format!("{scope}.toggle"))?,
        mandatory: flag(provenance, &format!("{scope}.mandatory"))?,
        efforts,
        default_enabled: flag(provenance, &format!("{scope}.default_enabled"))?,
        default_effort: text(provenance, &format!("{scope}.default_effort"))?,
        dialect,
    })
}

fn resolve_context(provenance: &Provenance) -> Result<ResolvedContext, ConfigError> {
    let high = required_percent(provenance, "context.compaction_high_watermark_percent")?;
    let low = required_percent(provenance, "context.compaction_low_watermark_percent")?;
    if low.value >= high.value {
        return Err(ConfigError::InvalidValue {
            source: low.source,
            message: format!(
                "compaction must leave room below the watermark it triggers at ({}%)",
                high.value
            ),
        });
    }
    Ok(ResolvedContext {
        output_reserve: optional_u32(provenance, "context.output_reserve")?,
        reasoning_reserve: required_u32(provenance, "context.reasoning_reserve")?,
        capability_budget: optional_u32(provenance, "context.capability_budget")?,
        max_estimated_slack: optional_u32(provenance, "context.max_estimated_slack")?,
        compaction_high_watermark_percent: high,
        compaction_low_watermark_percent: low,
        idle_compaction_ms: required_u64(provenance, "context.idle_compaction_ms")?,
    })
}

fn resolve_limits(provenance: &Provenance) -> Result<ResolvedLimits, ConfigError> {
    Ok(ResolvedLimits {
        max_retries: required_u32(provenance, "limits.max_retries")?,
        max_tool_steps: required_u32(provenance, "limits.max_tool_steps")?,
        turn_time_limit_ms: required_u64(provenance, "limits.turn_time_limit_ms")?,
        tool_output_limit_bytes: required_u64(provenance, "limits.tool_output_limit_bytes")?,
    })
}

fn resolve_persistence(provenance: &Provenance) -> Result<ResolvedPersistence, ConfigError> {
    let sessions_dir = required_text(provenance, "persistence.sessions_dir")?;
    let checkpoint_key = secret(provenance, "persistence.checkpoint_key")?;
    let checkpoint_key_credential = text(provenance, "persistence.checkpoint_key_credential")?;
    if let (Some(key), Some(_credential)) = (&checkpoint_key, &checkpoint_key_credential) {
        return Err(ConfigError::InvalidValue {
            source: key.source.clone(),
            message: "choose exactly one checkpoint key source: `checkpoint_key` or `checkpoint_key_credential`"
                .to_owned(),
        });
    }
    if let Some(key) = &checkpoint_key {
        let exposed = key.value.expose();
        if exposed.len() != 64 || !exposed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ConfigError::InvalidValue {
                source: key.source.clone(),
                message:
                    "`checkpoint_key` must encode exactly 32 bytes as 64 hexadecimal characters"
                        .to_owned(),
            });
        }
    }
    if let Some(credential) = &checkpoint_key_credential {
        validate_credential(credential)?;
    }
    Ok(ResolvedPersistence {
        enabled: required_flag(provenance, "persistence.enabled")?,
        sessions_dir: Sourced::new(PathBuf::from(sessions_dir.value), sessions_dir.source),
        journal_events: required_flag(provenance, "persistence.journal_events")?,
        checkpoint_key,
        checkpoint_key_credential,
    })
}

fn resolve_approval(provenance: &Provenance) -> Result<ResolvedApproval, ConfigError> {
    let raw = required_text(provenance, "approval.mode")?;
    let mode = ApprovalMode::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
        source: raw.source.clone(),
        message: format!(
            "`{}` is not an approval mode; the modes are {}",
            raw.value,
            list_spellings(ApprovalMode::spellings())
        ),
    })?;
    Ok(ResolvedApproval {
        mode: Sourced::new(mode, raw.source),
        auto_approve: list(provenance, "approval.auto_approve")?,
    })
}

fn resolve_background(provenance: &Provenance) -> Result<ResolvedBackground, ConfigError> {
    let raw = required_text(provenance, "background.exit_policy")?;
    let policy = BackgroundExit::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
        source: raw.source.clone(),
        message: format!(
            "`{}` is not a background-exit policy; the policies are {}",
            raw.value,
            list_spellings(BackgroundExit::spellings())
        ),
    })?;
    Ok(ResolvedBackground {
        exit_policy: Sourced::new(policy, raw.source),
        max_children: required_u32(provenance, "background.max_children")?,
        max_monitors: required_u32(provenance, "background.max_monitors")?,
    })
}

fn list_spellings(spellings: &[&str]) -> String {
    spellings
        .iter()
        .map(|spelling| format!("`{spelling}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn text(provenance: &Provenance, key: &str) -> Result<Option<Sourced<String>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Text(value) => {
                Ok(Some(Sourced::new(value.clone(), entry.source.clone())))
            }
            other => Err(wrong_kind(entry, other, "a string")),
        },
    }
}

fn secret(provenance: &Provenance, key: &str) -> Result<Option<Sourced<Secret>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Secret(value) => {
                Ok(Some(Sourced::new(value.clone(), entry.source.clone())))
            }
            other => Err(wrong_kind(entry, other, "a secret string")),
        },
    }
}

fn integer(provenance: &Provenance, key: &str) -> Result<Option<Sourced<i64>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Integer(value) => Ok(Some(Sourced::new(*value, entry.source.clone()))),
            other => Err(wrong_kind(entry, other, "a whole number")),
        },
    }
}

fn flag(provenance: &Provenance, key: &str) -> Result<Option<Sourced<bool>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Flag(value) => Ok(Some(Sourced::new(*value, entry.source.clone()))),
            other => Err(wrong_kind(entry, other, "`true` or `false`")),
        },
    }
}

fn list(provenance: &Provenance, key: &str) -> Result<Option<Sourced<Vec<String>>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::List(value) => {
                Ok(Some(Sourced::new(value.clone(), entry.source.clone())))
            }
            other => Err(wrong_kind(entry, other, "a list of strings")),
        },
    }
}

fn wrong_kind(entry: &Entry, found: &SettingValue, expected: &str) -> ConfigError {
    ConfigError::InvalidValue {
        source: entry.source.clone(),
        message: format!("expected {expected}, found {}", describe(found.kind())),
    }
}

fn describe(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Text => "a string",
        ValueKind::Secret => "a secret string",
        ValueKind::Integer => "a whole number",
        ValueKind::Flag => "a boolean",
        ValueKind::List => "a list",
    }
}

fn optional_u32(provenance: &Provenance, key: &str) -> Result<Option<Sourced<u32>>, ConfigError> {
    integer(provenance, key)?.map(narrow_u32).transpose()
}

fn required_u32(provenance: &Provenance, key: &str) -> Result<Sourced<u32>, ConfigError> {
    narrow_u32(integer(provenance, key)?.ok_or_else(|| missing(key))?)
}

fn required_u64(provenance: &Provenance, key: &str) -> Result<Sourced<u64>, ConfigError> {
    let value = integer(provenance, key)?.ok_or_else(|| missing(key))?;
    match u64::try_from(value.value) {
        Ok(narrowed) => Ok(Sourced::new(narrowed, value.source)),
        Err(_) => Err(out_of_range(value, "0 and 9223372036854775807")),
    }
}

fn required_percent(provenance: &Provenance, key: &str) -> Result<Sourced<u8>, ConfigError> {
    let value = integer(provenance, key)?.ok_or_else(|| missing(key))?;
    match u8::try_from(value.value) {
        Ok(narrowed) if narrowed <= 100 => Ok(Sourced::new(narrowed, value.source)),
        _ => Err(out_of_range(value, "0 and 100")),
    }
}

fn required_text(provenance: &Provenance, key: &str) -> Result<Sourced<String>, ConfigError> {
    text(provenance, key)?.ok_or_else(|| missing(key))
}

fn required_flag(provenance: &Provenance, key: &str) -> Result<Sourced<bool>, ConfigError> {
    flag(provenance, key)?.ok_or_else(|| missing(key))
}

fn narrow_u32(value: Sourced<i64>) -> Result<Sourced<u32>, ConfigError> {
    match u32::try_from(value.value) {
        Ok(narrowed) => Ok(Sourced::new(narrowed, value.source)),
        Err(_) => Err(out_of_range(value, "0 and 4294967295")),
    }
}

fn out_of_range(value: Sourced<i64>, range: &str) -> ConfigError {
    ConfigError::InvalidValue {
        source: value.source,
        message: format!("{} is outside {range}", value.value),
    }
}

/// A setting no layer supplied. No file is named because none is at fault: the
/// value is absent everywhere, including from Smith's own defaults.
fn missing(key: &str) -> ConfigError {
    ConfigError::MissingSetting {
        key: key.to_owned(),
        message: "no layer supplied a value".to_owned(),
    }
}

fn unquote_segment(segment: &str) -> String {
    segment
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .map_or_else(|| segment.to_owned(), |inner| inner.replace("\\\"", "\""))
}

/// Known names close enough to `candidate` to be worth suggesting.
///
/// The threshold grows with the length of the word so that `mdel` suggests
/// `model` without a twenty-character key suggesting every other one.
fn nearest<'a>(candidate: &str, known: impl Iterator<Item = &'a str>) -> Vec<String> {
    let budget = (candidate.chars().count() / 3).max(1);
    let mut scored: Vec<(usize, &str)> = known
        .map(|name| (distance(candidate, name), name))
        .filter(|(score, _)| *score <= budget)
        .collect();
    scored.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));
    scored
        .into_iter()
        .take(3)
        .map(|(_, name)| name.to_owned())
        .collect()
}

/// Edit distance, case-insensitive, counting a swap of two neighbours as one
/// edit.
///
/// Plain Levenshtein charges two edits for a transposition, which is the most
/// common typo of all: `modle` would then be as far from `model` as a word
/// with two unrelated mistakes, and the suggestion a user most needs would be
/// the one they never see.
fn distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.to_lowercase().chars().collect();
    let right: Vec<char> = right.to_lowercase().chars().collect();
    let mut grid = vec![vec![0usize; right.len() + 1]; left.len() + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in grid[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=left.len() {
        for j in 1..=right.len() {
            let cost = usize::from(left[i - 1] != right[j - 1]);
            let mut best = (grid[i - 1][j - 1] + cost)
                .min(grid[i - 1][j] + 1)
                .min(grid[i][j - 1] + 1);
            if i > 1 && j > 1 && left[i - 1] == right[j - 2] && left[i - 2] == right[j - 1] {
                best = best.min(grid[i - 2][j - 2] + 1);
            }
            grid[i][j] = best;
        }
    }
    grid[left.len()][right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_layer_has_a_distinct_rank() {
        let ranks: Vec<u8> = Layer::all()
            .iter()
            .map(|layer| layer.precedence())
            .collect();
        assert_eq!(ranks, (0..8).collect::<Vec<u8>>());
    }

    #[test]
    fn environment_names_follow_the_setting_key() {
        assert_eq!(env_name("model"), "SMITH_MODEL");
        assert_eq!(
            env_name("context.output_reserve"),
            "SMITH_CONTEXT_OUTPUT_RESERVE"
        );
        for (key, _) in SETTINGS {
            assert_eq!(setting_for_env(&env_name(key)), Some(*key), "{key}");
        }
    }

    #[test]
    fn keys_with_punctuation_are_quoted_so_they_round_trip() {
        assert_eq!(
            join_key(&["models", "acme/example-model", "context_tokens"]),
            "models.\"acme/example-model\".context_tokens"
        );
        assert_eq!(
            unquote_segment("\"acme/example-model\""),
            "acme/example-model"
        );
        assert_eq!(unquote_segment("plain"), "plain");
    }

    #[test]
    fn a_near_miss_is_suggested_and_a_distant_one_is_not() {
        let known = ["model", "provider", "profile"];
        assert_eq!(
            nearest("modle", known.into_iter()),
            vec!["model".to_owned()]
        );
        assert!(nearest("wildly-different", known.into_iter()).is_empty());
    }

    #[test]
    fn serde_unknown_field_messages_yield_the_key_and_its_alternatives() {
        let (key, candidates) = unknown_field(
            "unknown field `output_reserv`, expected one of `output_reserve`, `reasoning_reserve`",
        )
        .expect("an unknown-field message");
        assert_eq!(key, "output_reserv");
        assert_eq!(candidates, vec!["output_reserve", "reasoning_reserve"]);
        assert!(unknown_field("invalid type: string, expected u32").is_none());
    }

    #[test]
    fn positions_count_lines_and_columns_from_one() {
        let text = "a = 1\nbb = 2\n";
        assert_eq!(position(text, 0).to_string(), "line 1, column 1");
        assert_eq!(position(text, 6).to_string(), "line 2, column 1");
        assert_eq!(position(text, 8), Position { line: 2, column: 3 });
    }

    #[test]
    fn built_in_defaults_claim_nothing_about_a_model() {
        let defaults = built_in_defaults(Path::new("/user/.smith"));
        assert!(defaults.models.is_empty());
        let context = defaults.context.expect("a context table");
        assert!(context.output_reserve.is_none());
        assert!(context.capability_budget.is_none());
        assert_eq!(context.reasoning_reserve, Some(0));
    }
}
