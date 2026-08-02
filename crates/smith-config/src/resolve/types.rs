//! Configuration resolution types, provenance, and shared internal state.
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

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use agent_runtime_core::store::Secret;

use crate::model::{
    AgentPosture, ApprovalMode, BackgroundExit, ProfileUse, ReasoningDialect, ReasoningOnlyBehavior,
};

use super::provenance::*;

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

include!("tests/mod.rs");
