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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;

use agent_runtime::agent::external::{
    ExternalAgentBackend, ExternalAgentEvent, ExternalSessionId, ExternalTurnRequest,
    ExternalTurnStream,
};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::usage::{CounterKind, UsageDelta};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Which installed coding agent a harness drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliAgentKind {
    /// Anthropic's Claude Code CLI.
    ClaudeCode,
    /// OpenAI's Codex CLI.
    Codex,
}

impl CliAgentKind {
    /// The configuration token that selects this harness.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
        }
    }

    /// Parses a configured harness name.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude-code" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

/// Resolved settings for one harness.
#[derive(Debug, Clone)]
pub struct CliAgentSettings {
    /// Absolute path to the CLI, resolved without a shell.
    pub executable: PathBuf,
    /// Model to request, when the owner selected one.
    pub model: Option<String>,
    /// Fixed non-secret arguments appended to the built argv.
    pub args: Vec<String>,
    /// Working directory for the child process.
    pub cwd: PathBuf,
    /// Environment overlaid on the inherited ambient environment.
    pub env: BTreeMap<String, String>,
    /// Whether the CLI may run its own tools.
    ///
    /// Off by default. When enabled, the CLI executes reads, writes, and
    /// commands Smith never approved, never scoped to the workspace, and
    /// cannot record as tool history.
    pub allow_own_tools: bool,
    /// Instructions appended to the CLI's own system prompt.
    pub instructions: Option<String>,
}

/// An external agent backend driving one installed CLI.
#[derive(Debug)]
pub struct CliAgentBackend {
    kind: CliAgentKind,
    settings: CliAgentSettings,
}

impl CliAgentBackend {
    /// Builds a backend for a resolved harness.
    pub fn new(kind: CliAgentKind, settings: CliAgentSettings) -> Self {
        Self { kind, settings }
    }

    /// Which CLI this backend drives.
    pub fn kind(&self) -> CliAgentKind {
        self.kind
    }

    /// Builds the argv for one turn.
    ///
    /// The prompt is passed as an argument for Codex and on stdin for Claude,
    /// matching what each CLI accepts in non-interactive mode.
    fn argv(&self, prompt: &str, resume: Option<&str>) -> Vec<String> {
        let mut args = Vec::new();
        match self.kind {
            CliAgentKind::ClaudeCode => {
                args.push("-p".to_owned());
                args.push(prompt.to_owned());
                args.push("--output-format".to_owned());
                args.push("stream-json".to_owned());
                // stream-json requires --verbose in non-interactive mode.
                args.push("--verbose".to_owned());
                if self.settings.allow_own_tools {
                    // The CLI still prompts by default, which would deadlock a
                    // non-interactive run, so name a non-prompting mode.
                    args.push("--permission-mode".to_owned());
                    args.push("acceptEdits".to_owned());
                } else {
                    // An empty allow-list is how Claude is told to run without
                    // tools at all, rather than trusting its default.
                    args.push("--allowedTools".to_owned());
                    args.push(String::new());
                }
                if let Some(model) = &self.settings.model {
                    args.push("--model".to_owned());
                    args.push(model.clone());
                }
                if let Some(instructions) = &self.settings.instructions {
                    args.push("--append-system-prompt".to_owned());
                    args.push(instructions.clone());
                }
                if let Some(session) = resume {
                    args.push("--resume".to_owned());
                    args.push(session.to_owned());
                }
            }
            CliAgentKind::Codex => {
                args.push("exec".to_owned());
                if let Some(session) = resume {
                    args.push("resume".to_owned());
                    args.push(session.to_owned());
                }
                args.push("--json".to_owned());
                // Smith already decided this directory is the workspace; the
                // CLI's own git check would refuse to run outside a repo.
                args.push("--skip-git-repo-check".to_owned());
                args.push("-C".to_owned());
                args.push(self.settings.cwd.display().to_string());
                args.push("--sandbox".to_owned());
                args.push(if self.settings.allow_own_tools {
                    "workspace-write".to_owned()
                } else {
                    "read-only".to_owned()
                });
                if let Some(model) = &self.settings.model {
                    args.push("-m".to_owned());
                    args.push(model.clone());
                }
                args.push(prompt.to_owned());
            }
        }
        args.extend(self.settings.args.iter().cloned());
        args
    }
}

#[async_trait]
impl ExternalAgentBackend for CliAgentBackend {
    async fn run_turn(
        &self,
        request: ExternalTurnRequest,
    ) -> Result<ExternalTurnStream, RuntimeError> {
        // A CLI takes one prompt string; non-text parts have no place to go,
        // so they are dropped rather than silently misrepresented.
        let prompt = request
            .input
            .parts
            .iter()
            .filter_map(agent_runtime_core::content::ContentPart::as_text)
            .collect::<Vec<_>>()
            .join("\n");
        let resume = request.resume.as_ref().map(|id| id.as_str().to_owned());
        let args = self.argv(&prompt, resume.as_deref());

        let mut command = Command::new(&self.settings.executable);
        command.args(&args);
        command.current_dir(&self.settings.cwd);
        // Inherited, not cleared: the CLI's own login lives here.
        for (key, value) in &self.settings.env {
            command.env(key, value);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|error| {
            RuntimeError::new(
                ErrorKind::Config,
                format!(
                    "could not start `{}`: {error}",
                    self.settings.executable.display()
                ),
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            RuntimeError::new(ErrorKind::Provider, "CLI agent produced no stdout stream")
        })?;

        let kind = self.kind;
        let cancel = request.cancel.clone();
        let stream = async_stream::stream! {
            let mut lines = BufReader::new(stdout).lines();
            let mut state = TurnState::default();

            loop {
                let next = tokio::select! {
                    _ = cancel.cancelled() => None,
                    line = lines.next_line() => line.unwrap_or_default(),
                };
                let Some(line) = next else { break };
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    // A CLI may interleave non-JSON diagnostics; they are not
                    // turn content and not a failure.
                    continue;
                };
                for event in normalize(kind, &value, &mut state) {
                    let terminal = event.is_terminal();
                    yield event;
                    if terminal {
                        state.terminated = true;
                        break;
                    }
                }
                if state.terminated {
                    break;
                }
            }

            let _ = child.start_kill();

            if !state.terminated {
                // The stream ended without the CLI reporting a terminal
                // result: a truncated run, not a completed one.
                yield ExternalAgentEvent::Failed {
                    message: "CLI agent exited before reporting a result".to_owned(),
                };
            }
        };

        Ok(Box::pin(stream))
    }
}

/// Per-turn normalization state.
#[derive(Debug, Default)]
struct TurnState {
    terminated: bool,
}

/// Translates one CLI event into zero or more runtime events.
fn normalize(
    kind: CliAgentKind,
    value: &serde_json::Value,
    _state: &mut TurnState,
) -> Vec<ExternalAgentEvent> {
    match kind {
        CliAgentKind::ClaudeCode => normalize_claude(value),
        CliAgentKind::Codex => normalize_codex(value),
    }
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

fn session_event(id: Option<String>) -> Option<ExternalAgentEvent> {
    let session = ExternalSessionId::new(id?).ok()?;
    Some(ExternalAgentEvent::SessionStarted { session })
}

fn normalize_claude(value: &serde_json::Value) -> Vec<ExternalAgentEvent> {
    let mut events = Vec::new();
    match value.get("type").and_then(serde_json::Value::as_str) {
        Some("system") => {
            if value.get("subtype").and_then(serde_json::Value::as_str) == Some("init") {
                events.extend(session_event(string_field(value, "session_id")));
            }
        }
        Some("assistant") => {
            let content = value
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(serde_json::Value::as_array);
            for part in content.into_iter().flatten() {
                match part.get("type").and_then(serde_json::Value::as_str) {
                    Some("text") => {
                        if let Some(text) = string_field(part, "text") {
                            events.push(ExternalAgentEvent::Text { text });
                        }
                    }
                    Some("thinking") => {
                        if let Some(text) =
                            string_field(part, "thinking").filter(|text| !text.is_empty())
                        {
                            events.push(ExternalAgentEvent::Reasoning { text });
                        }
                    }
                    Some("tool_use") => {
                        if let (Some(id), Some(name)) =
                            (string_field(part, "id"), string_field(part, "name"))
                        {
                            events.push(ExternalAgentEvent::ToolInvoked {
                                id,
                                name,
                                detail: part
                                    .get("input")
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        Some("user") => {
            let content = value
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(serde_json::Value::as_array);
            for part in content.into_iter().flatten() {
                if part.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
                    && let Some(id) = string_field(part, "tool_use_id")
                {
                    {
                        let failed = part
                            .get("is_error")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        events.push(ExternalAgentEvent::ToolCompleted {
                            id,
                            ok: !failed,
                            detail: part
                                .get("content")
                                .cloned()
                                .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                        });
                    }
                }
            }
        }
        Some("result") => {
            if let Some(usage) = value.get("usage") {
                events.push(ExternalAgentEvent::Usage {
                    usage: claude_usage(usage),
                });
            }
            // A failed Claude run can still exit zero, reporting the reason in
            // `result`. Exit status alone is not a health signal.
            let failed = value
                .get("is_error")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if failed {
                events.push(ExternalAgentEvent::Failed {
                    message: string_field(value, "result")
                        .unwrap_or_else(|| "CLI agent reported an error".to_owned()),
                });
            } else {
                events.push(ExternalAgentEvent::Completed);
            }
        }
        _ => {}
    }
    events
}

fn counter(value: &serde_json::Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

/// Claude reports `input_tokens` already exclusive of cache reads and writes,
/// so the counters are copied straight across.
fn claude_usage(usage: &serde_json::Value) -> UsageDelta {
    let reasoning = usage
        .get("output_tokens_details")
        .map(|details| counter(details, "thinking_tokens"))
        .unwrap_or(0);
    UsageDelta::new()
        .with(CounterKind::InputUncached, counter(usage, "input_tokens"))
        .with(
            CounterKind::InputCached,
            counter(usage, "cache_read_input_tokens"),
        )
        .with(
            CounterKind::CacheWrite,
            counter(usage, "cache_creation_input_tokens"),
        )
        .with(CounterKind::Output, counter(usage, "output_tokens"))
        .with(CounterKind::Reasoning, reasoning)
}

/// Codex reports `input_tokens` *inclusive* of the cached prefix, so the
/// cached count is subtracted back out to keep the counters disjoint.
fn codex_usage(usage: &serde_json::Value) -> UsageDelta {
    let cached = counter(usage, "cached_input_tokens");
    let input = counter(usage, "input_tokens").saturating_sub(cached);
    UsageDelta::new()
        .with(CounterKind::InputUncached, input)
        .with(CounterKind::InputCached, cached)
        .with(
            CounterKind::CacheWrite,
            counter(usage, "cache_write_input_tokens"),
        )
        .with(CounterKind::Output, counter(usage, "output_tokens"))
        .with(
            CounterKind::Reasoning,
            counter(usage, "reasoning_output_tokens"),
        )
}

fn normalize_codex(value: &serde_json::Value) -> Vec<ExternalAgentEvent> {
    let mut events = Vec::new();
    match value.get("type").and_then(serde_json::Value::as_str) {
        Some("thread.started") => {
            events.extend(session_event(string_field(value, "thread_id")));
        }
        Some("item.completed") => {
            let Some(item) = value.get("item") else {
                return events;
            };
            let id = string_field(item, "id").unwrap_or_else(|| "item".to_owned());
            match item.get("type").and_then(serde_json::Value::as_str) {
                Some("agent_message") => {
                    if let Some(text) = string_field(item, "text") {
                        events.push(ExternalAgentEvent::Text { text });
                    }
                }
                Some("reasoning") => {
                    if let Some(text) = string_field(item, "text") {
                        events.push(ExternalAgentEvent::Reasoning { text });
                    }
                }
                // Codex reports a tool as one completed item carrying both the
                // invocation and its outcome, so both events are emitted here.
                Some("command_execution") => {
                    let command =
                        string_field(item, "command").unwrap_or_else(|| "command".to_owned());
                    let exit = item.get("exit_code").and_then(serde_json::Value::as_i64);
                    events.push(ExternalAgentEvent::ToolInvoked {
                        id: id.clone(),
                        name: "command_execution".to_owned(),
                        detail: serde_json::json!({ "command": command }),
                    });
                    events.push(ExternalAgentEvent::ToolCompleted {
                        id,
                        ok: exit == Some(0),
                        detail: item
                            .get("aggregated_output")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    });
                }
                Some("file_change") | Some("mcp_tool_call") => {
                    let name = item
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("tool")
                        .to_owned();
                    events.push(ExternalAgentEvent::ToolInvoked {
                        id: id.clone(),
                        name,
                        detail: item.clone(),
                    });
                    events.push(ExternalAgentEvent::ToolCompleted {
                        id,
                        ok: true,
                        detail: serde_json::Value::Null,
                    });
                }
                // `error` items also carry benign local warnings — a malformed
                // agent-role file, a clamped hook timeout — so one must not
                // fail the turn on its own. Only `turn.failed` is terminal.
                _ => {}
            }
        }
        Some("turn.completed") => {
            if let Some(usage) = value.get("usage") {
                events.push(ExternalAgentEvent::Usage {
                    usage: codex_usage(usage),
                });
            }
            events.push(ExternalAgentEvent::Completed);
        }
        Some("turn.failed") => {
            let message = value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("CLI agent reported a failed turn")
                .to_owned();
            events.push(ExternalAgentEvent::Failed { message });
        }
        _ => {}
    }
    events
}

/// Builds a backend for the harness a resolved configuration selected.
///
/// Returns `None` when the profile runs on Smith's own provider/tool loop,
/// which leaves the runtime composed exactly as it was before harnesses
/// existed.
pub fn backend_for(
    config: &smith_config::resolve::ResolvedConfig,
    workspace_root: &str,
) -> Option<std::sync::Arc<dyn ExternalAgentBackend>> {
    let harness = config.harness.as_ref()?;
    let kind = CliAgentKind::parse(&harness.kind.value)?;
    let settings = CliAgentSettings {
        executable: PathBuf::from(&harness.executable.value),
        model: harness.model.as_ref().map(|model| model.value.clone()),
        args: harness.args.clone(),
        cwd: PathBuf::from(workspace_root),
        env: harness.env.clone(),
        allow_own_tools: harness.allow_own_tools.value,
        instructions: None,
    };
    Some(std::sync::Arc::new(CliAgentBackend::new(kind, settings)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(allow_own_tools: bool) -> CliAgentSettings {
        CliAgentSettings {
            executable: PathBuf::from("/usr/local/bin/agent"),
            model: Some("a-model".to_owned()),
            args: Vec::new(),
            cwd: PathBuf::from("/repo"),
            env: BTreeMap::new(),
            allow_own_tools,
            instructions: None,
        }
    }

    #[test]
    fn claude_runs_without_its_own_tools_by_default() {
        let backend = CliAgentBackend::new(CliAgentKind::ClaudeCode, settings(false));
        let args = backend.argv("hello", None);
        let allow = args.iter().position(|arg| arg == "--allowedTools");
        assert_eq!(args[allow.expect("an explicit allow-list") + 1], "");
        assert!(!args.iter().any(|arg| arg == "--permission-mode"));
    }

    #[test]
    fn codex_runs_read_only_by_default_and_writes_only_when_permitted() {
        let read_only = CliAgentBackend::new(CliAgentKind::Codex, settings(false));
        let args = read_only.argv("hello", None);
        let sandbox = args.iter().position(|arg| arg == "--sandbox");
        assert_eq!(args[sandbox.expect("a sandbox mode") + 1], "read-only");

        let writable = CliAgentBackend::new(CliAgentKind::Codex, settings(true));
        let args = writable.argv("hello", None);
        let sandbox = args.iter().position(|arg| arg == "--sandbox");
        assert_eq!(
            args[sandbox.expect("a sandbox mode") + 1],
            "workspace-write"
        );
    }

    #[test]
    fn resuming_continues_each_cli_in_its_own_dialect() {
        let claude = CliAgentBackend::new(CliAgentKind::ClaudeCode, settings(false));
        let args = claude.argv("next", Some("session-1"));
        let resume = args.iter().position(|arg| arg == "--resume");
        assert_eq!(args[resume.expect("a resume flag") + 1], "session-1");

        let codex = CliAgentBackend::new(CliAgentKind::Codex, settings(false));
        let args = codex.argv("next", Some("thread-1"));
        assert_eq!(args[0], "exec");
        assert_eq!(args[1], "resume");
        assert_eq!(args[2], "thread-1");
    }

    #[test]
    fn claude_result_reporting_an_error_fails_even_with_a_clean_exit() {
        let value = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "is_error": true,
            "result": "Not logged in · Please run /login",
            "usage": {"input_tokens": 1, "output_tokens": 0}
        });
        let events = normalize_claude(&value);
        assert!(events.iter().any(|event| matches!(
            event,
            ExternalAgentEvent::Failed { message } if message.contains("Not logged in")
        )));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ExternalAgentEvent::Completed))
        );
    }

    #[test]
    fn a_codex_error_item_is_a_warning_not_a_failed_turn() {
        let value = serde_json::json!({
            "type": "item.completed",
            "item": {"id": "item_0", "type": "error", "message": "clamping hook timeout"}
        });
        assert!(normalize_codex(&value).is_empty());
    }

    #[test]
    fn codex_command_execution_reports_both_invocation_and_outcome() {
        let value = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_5",
                "type": "command_execution",
                "command": "/bin/zsh -lc 'cat config.txt'",
                "aggregated_output": "version = 3\n",
                "exit_code": 0,
                "status": "completed"
            }
        });
        let events = normalize_codex(&value);
        assert!(matches!(events[0], ExternalAgentEvent::ToolInvoked { .. }));
        assert!(matches!(
            events[1],
            ExternalAgentEvent::ToolCompleted { ok: true, .. }
        ));
    }

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
