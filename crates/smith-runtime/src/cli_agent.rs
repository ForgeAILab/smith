//! External agent backends for installed coding CLIs.
//!
//! Claude Code and Codex are agents, not models: each owns a conversation,
//! runs its own tools under its own permission policy, and streams its own
//! events. These adapters translate one Smith turn into a bounded CLI
//! invocation and normalize the CLI's machine output into Agent Runtime's
//! external agent events, so a harness turn is as visible in the transcript as
//! a native one.
//!
//! Two deliberate differences from [`crate::command_provider`], which reaches a
//! *model* through a subprocess:
//!
//! - The CLI's own session is continued rather than replaying history. That is
//!   the whole point: re-sending a full transcript every turn is what made the
//!   model-shaped approach slow and expensive.
//! - The ambient environment is inherited. An installed coding CLI depends on
//!   its own login, `PATH`, and home directory; clearing the environment makes
//!   it report "not logged in" rather than doing any work.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use agent_runtime::agent::external::{ExternalAgentEvent, ExternalSessionId};

mod claude;
mod codex;
#[cfg(test)]
mod fixture_tests;
mod invocation;
#[cfg(all(test, unix))]
mod process_tests;
mod selection;

pub(crate) use selection::turn_execution;

/// Which installed coding agent a harness drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CliAgentKind {
    /// Anthropic's Claude Code CLI.
    ClaudeCode,
    /// OpenAI's Codex CLI.
    Codex,
}

impl CliAgentKind {
    /// The configuration token that selects this harness.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
        }
    }

    /// Parses a configured harness name.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "claude-code" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

/// Resolved settings for one harness.
#[derive(Debug, Clone)]
pub(crate) struct CliAgentSettings {
    /// Absolute path to the CLI, resolved without a shell.
    pub(crate) executable: PathBuf,
    /// Model to request, when the owner selected one.
    pub(crate) model: Option<String>,
    /// Fixed non-secret arguments appended to the built argv.
    pub(crate) args: Vec<String>,
    /// Working directory for the child process.
    pub(crate) cwd: PathBuf,
    /// Environment overlaid on the inherited ambient environment.
    pub(crate) env: BTreeMap<String, String>,
    /// Whether the CLI may run its own tools.
    ///
    /// Off by default. When enabled, the CLI executes reads, writes, and
    /// commands Smith never approved, never scoped to the workspace, and
    /// cannot record as tool history.
    pub(crate) allow_own_tools: bool,
    /// Instructions appended to the CLI's own system prompt.
    pub(crate) instructions: Option<String>,
    /// Running Smith binary used for the per-turn relay and process supervisor.
    pub(crate) relay_executable: Option<PathBuf>,
    /// Turn-scoped MCP relay whose calls Smith dispatches and the CLI echoes.
    pub(crate) bridge: Option<BridgeInvocation>,
}

/// Relay declaration supplied by the bridge owner for one invocation.
#[derive(Debug, Clone)]
pub(crate) struct BridgeInvocation {
    pub(crate) server: String,
    pub(crate) relay_executable: PathBuf,
    pub(crate) socket: PathBuf,
}

/// An external agent backend driving one installed CLI.
#[derive(Debug)]
pub(crate) struct CliAgentBackend {
    kind: CliAgentKind,
    settings: CliAgentSettings,
    reported_omissions: std::sync::Mutex<Vec<crate::cli_bridge::BridgeOmission>>,
    /// Repeated turns on an embedding host need only one missing-path warning.
    warned_missing_executable: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    spawn_override: Option<SpawnOverride>,
    #[cfg(test)]
    endpoint_base_override: Option<PathBuf>,
}

/// Tests inspect the command passed by run_turn without a second supervisor.
#[cfg(test)]
type SpawnOverride = fn(&mut tokio::process::Command) -> std::io::Result<tokio::process::Child>;

/// One resolved installed-agent selection, minus the directory a turn runs in.
///
/// The directory is deliberately not part of it: the same profile serves the
/// root session and every child spawned onto it, and a child may run against
/// a directory of its own. Resolving the selection once and building a
/// backend per run is what keeps a child from inheriting its parent's
/// working directory or its permission to run the CLI's own tools.
#[derive(Debug, Clone)]
pub(crate) struct CliAgentPlan {
    kind: CliAgentKind,
    executable: PathBuf,
    model: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    allow_own_tools: bool,
    relay_executable: Option<PathBuf>,
}

/// What executes a turn for one resolved configuration.
#[derive(Debug, Clone)]
pub(crate) enum TurnExecution {
    /// No installed agent was selected; the resolved provider runs the turn,
    /// which leaves the composition exactly as it was before harnesses
    /// existed.
    Provider,
    /// The installed agent turns run on.
    InstalledAgent(CliAgentPlan),
    /// An installed agent was selected, but its program is not installed.
    ///
    /// Carried rather than collapsed into [`Self::Provider`]: falling back
    /// would send the agent's `cli/<kind>/<model>` id to an HTTP provider
    /// that has never heard of it, so the run would report that provider
    /// rejecting an unknown model instead of the program being missing.
    MissingProgram {
        /// The `cli/<kind>/<model>` id that selected it.
        model: String,
        /// The installed agent kind, which is also its `[harness.<kind>]` key.
        kind: String,
        /// The program that was looked for.
        program: String,
    },
}

/// Per-turn normalization state.
#[derive(Debug, Default)]
struct TurnState {
    terminated: bool,
    bridge_server: Option<String>,
    /// Claude results carry only an id, so suppressed calls stay correlated
    /// for the whole turn, including failed results.
    suppressed: BTreeSet<String>,
    /// Tool items already reported as started, so the completed item that
    /// follows resolves that row instead of opening a second one.
    invoked: BTreeSet<String>,
}

impl TurnState {
    fn for_bridge(bridge: Option<&BridgeInvocation>) -> Self {
        Self {
            bridge_server: bridge.map(|bridge| bridge.server.clone()),
            ..Self::default()
        }
    }
}

/// Translates one CLI event into zero or more runtime events.
fn normalize(
    kind: CliAgentKind,
    value: &serde_json::Value,
    state: &mut TurnState,
) -> Vec<ExternalAgentEvent> {
    match kind {
        CliAgentKind::ClaudeCode => claude::normalize_claude(value, state),
        CliAgentKind::Codex => codex::normalize_codex(value, state),
    }
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

fn session_event(id: Option<String>) -> Option<ExternalAgentEvent> {
    let session = ExternalSessionId::new(id?).ok()?;
    Some(ExternalAgentEvent::SessionStarted { session })
}

fn counter(value: &serde_json::Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::claude::claude_usage;
    use super::codex::codex_usage;
    use agent_runtime_core::usage::CounterKind;

    #[test]
    fn each_cli_reports_cache_tokens_in_its_own_convention() {
        // Claude's input count already excludes the cached prefix.
        let claude = claude_usage(&serde_json::json!({
            "input_tokens": 10,
            "cache_read_input_tokens": 25,
            "cache_creation_input_tokens": 7,
            "output_tokens": 36,
            "output_tokens_details": {"thinking_tokens": 29}
        }));
        assert_eq!(claude.get(CounterKind::InputUncached), 10);
        assert_eq!(claude.get(CounterKind::InputCached), 25);
        assert_eq!(claude.get(CounterKind::Reasoning), 29);

        // Codex's includes it, so the adapter subtracts it back out.
        let codex = codex_usage(&serde_json::json!({
            "input_tokens": 43548,
            "cached_input_tokens": 34432,
            "cache_write_input_tokens": 0,
            "output_tokens": 61,
            "reasoning_output_tokens": 5
        }));
        assert_eq!(codex.get(CounterKind::InputUncached), 43548 - 34432);
        assert_eq!(codex.get(CounterKind::InputCached), 34432);
        assert_eq!(codex.get(CounterKind::Reasoning), 5);
    }
}
