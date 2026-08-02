//! Configuration layers, sourced values, overrides, and explanation ledger.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use agent_runtime_core::store::Secret;

use crate::model::{ApprovalMode, BackgroundExit};

use super::provider::nearest;
use super::types::ConfigError;

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
pub(super) const AUTH_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
];

/// What kind of value a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ValueKind {
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
pub(super) const SETTINGS: &[(&str, ValueKind)] = &[
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
    pub(super) fn kind(&self) -> ValueKind {
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

    pub(super) fn winner(&self, key: &str) -> Option<&Entry> {
        self.entries.get(key).and_then(|entries| entries.last())
    }

    pub(super) fn extend(&mut self, contributions: Vec<Contribution>) {
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
pub(super) struct Contribution {
    pub(super) key: String,
    pub(super) value: SettingValue,
    pub(super) source: Source,
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
    pub(super) fn contributions(&self, layer: Layer) -> Vec<Contribution> {
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
