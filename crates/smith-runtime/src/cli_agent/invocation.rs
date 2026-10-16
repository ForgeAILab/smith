//! CLI arguments and the lifetime of one running turn.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use crate::cli_bridge::{
    BRIDGE_SERVER_NAME, BridgeDispatch, BridgeEndpoint, BridgeOmission, BridgedSet,
    RuntimeBridgeDispatch,
};

use agent_runtime::agent::external::{
    ExternalAgentBackend, ExternalAgentEvent, ExternalTurnRequest, ExternalTurnStream,
};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, Command};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::{
    BridgeInvocation, CliAgentBackend, CliAgentKind, CliAgentSettings, TurnState, normalize,
};

pub(crate) const MISSING_EXECUTABLE_WARNING: &str =
    "Smith executable path is unknown; CLI capability bridge is unavailable; CLI may outlive Smith";

impl CliAgentBackend {
    /// Builds a backend for a resolved harness.
    pub(crate) fn new(kind: CliAgentKind, settings: CliAgentSettings) -> Self {
        Self {
            kind,
            settings,
            reported_omissions: Default::default(),
            warned_missing_executable: Default::default(),
            #[cfg(test)]
            missing_executable_warning_count: Default::default(),
            #[cfg(test)]
            spawn_override: None,
            #[cfg(test)]
            endpoint_base_override: None,
        }
    }

    /// Builds the argv for one turn.
    ///
    /// Claude takes the prompt after `-p`; Codex takes it as the last argument,
    /// matching what each CLI accepts in non-interactive mode.
    pub(super) fn argv(
        &self,
        prompt: &str,
        resume: Option<&str>,
    ) -> Result<Vec<String>, RuntimeError> {
        let bridge = self
            .settings
            .bridge
            .as_ref()
            .map(|bridge| {
                Ok::<_, RuntimeError>((
                    bridge.server.as_str(),
                    bridge_path(&bridge.relay_executable)?,
                    bridge_path(&bridge.socket)?,
                ))
            })
            .transpose()?;
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
                    // Nobody can answer a prompt in a non-interactive run, so
                    // every prompt would be a denial. Owner arguments follow
                    // and may name a narrower mode; the last one wins.
                    args.push("--permission-mode".to_owned());
                    args.push("bypassPermissions".to_owned());
                }
                if let Some((server, executable, socket)) = bridge {
                    args.push("--allowedTools".to_owned());
                    args.push(format!("mcp__{server}"));
                    args.push("--mcp-config".to_owned());
                    args.push(
                        serde_json::json!({
                            "mcpServers": {
                                (server): {"args": ["mcp-relay", socket], "command": executable}
                            }
                        })
                        .to_string(),
                    );
                } else if !self.settings.allow_own_tools {
                    // The empty allow-list keeps Claude's existing tool policy
                    // when there are no Smith tools to pre-allow.
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
                args.extend(self.settings.args.iter().cloned());
            }
            CliAgentKind::Codex => {
                args.push("exec".to_owned());
                args.push("--json".to_owned());
                // Smith already decided this directory is the workspace; the
                // CLI's own git check would refuse to run outside a repo.
                args.push("--skip-git-repo-check".to_owned());
                args.push("-C".to_owned());
                args.push(self.settings.cwd.display().to_string());
                // `exec` cannot ask for approval, and the workspace-write
                // sandbox has no network, so own tools mean no sandbox.
                if self.settings.allow_own_tools {
                    args.push("--dangerously-bypass-approvals-and-sandbox".to_owned());
                } else {
                    args.push("--sandbox".to_owned());
                    args.push("read-only".to_owned());
                }
                if let Some(model) = &self.settings.model {
                    args.push("-m".to_owned());
                    args.push(model.clone());
                }
                args.extend(self.settings.args.iter().cloned());
                if let Some((server, executable, socket)) = bridge {
                    // JSON string encoding is also a TOML basic string for
                    // every path Smith can pass, so Codex parses these values.
                    let command =
                        serde_json::to_string(executable).expect("a string serializes as JSON");
                    let socket =
                        serde_json::to_string(socket).expect("a string serializes as JSON");
                    // Codex otherwise runs the turn without a server still
                    // starting, telling the model about tools it does not have.
                    for value in [
                        format!("mcp_servers.{server}.command={command}"),
                        format!("mcp_servers.{server}.args=[\"mcp-relay\",{socket}]"),
                        format!("mcp_servers.{server}.default_tools_approval_mode=\"approve\""),
                        format!("mcp_servers.{server}.tool_timeout_sec=604800"),
                        format!("mcp_servers.{server}.required=true"),
                        format!("mcp_servers.{server}.startup_timeout_sec=60"),
                    ] {
                        args.push("-c".to_owned());
                        args.push(value);
                    }
                }
                // Resume accepts fewer flags than exec, so exec flags must
                // precede the subcommand on continued turns as well.
                if let Some(session) = resume {
                    args.push("resume".to_owned());
                    args.push(session.to_owned());
                }
                args.push(prompt.to_owned());
            }
        }
        Ok(args)
    }

    fn apply_environment(&self, command: &mut Command) {
        // Inherited, not cleared: the CLI's own login lives here.
        command.envs(&self.settings.env);
        if self.kind == CliAgentKind::ClaudeCode && self.settings.bridge.is_some() {
            // This follows the owner's overlay: a CLI that times out first
            // abandons a call Smith is still deciding.
            command.env("MCP_TOOL_TIMEOUT", "604800000");
            command.env("MCP_TIMEOUT", "60000");
        }
    }

    /// Relay and supervisor share the one executable supplied by the host.
    pub(super) fn bridge_invocation(&self, socket: &Path) -> Option<BridgeInvocation> {
        self.settings
            .relay_executable
            .as_ref()
            .map(|executable| BridgeInvocation {
                server: BRIDGE_SERVER_NAME.to_owned(),
                relay_executable: executable.clone(),
                socket: socket.to_owned(),
            })
    }

    /// The host's Smith path wraps the CLI's unchanged argv when available.
    pub(super) fn turn_command(&self, args: &[String]) -> Command {
        let mut command = if let Some(supervisor) = self.settings.relay_executable.as_deref() {
            let mut command = Command::new(supervisor);
            command.args(["harness-supervise", "--"]);
            command.arg(&self.settings.executable);
            command.stdin(Stdio::piped());
            command
        } else {
            let mut command = Command::new(&self.settings.executable);
            command.stdin(Stdio::null());
            command
        };
        command.args(args).current_dir(&self.settings.cwd);
        self.apply_environment(&mut command);
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        command
    }

    fn spawn_turn(&self, command: &mut Command) -> std::io::Result<Child> {
        #[cfg(test)]
        if let Some(spawn) = self.spawn_override {
            return spawn(command);
        }
        command.spawn()
    }
}

fn bridge_path(path: &Path) -> Result<&str, RuntimeError> {
    path.to_str().ok_or_else(|| {
        RuntimeError::new(
            ErrorKind::Config,
            format!("CLI bridge path is not valid UTF-8: {path:?}"),
        )
    })
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
        // Runtime lists discovery tools only when a non-tool capability is
        // authorized, so their presence already answers the skills question.
        let bridged = BridgedSet::new(request.tools.specs(), true);
        let omissions = bridged.omissions().to_vec();
        let endpoint = if !bridged.is_empty() && self.settings.relay_executable.is_some() {
            let dispatch: Arc<dyn BridgeDispatch> = Arc::new(RuntimeBridgeDispatch(request.tools));
            // Per-backend injection avoids changing ambient TMPDIR for tests
            // running concurrently, and cannot exist in a production build.
            #[cfg(test)]
            let endpoint = match &self.endpoint_base_override {
                Some(base) => BridgeEndpoint::bind_at_base(bridged, dispatch, base)?,
                None => BridgeEndpoint::bind(bridged, dispatch)?,
            };
            #[cfg(not(test))]
            let endpoint = BridgeEndpoint::bind(bridged, dispatch)?;
            Some(endpoint)
        } else {
            None
        };
        let mut settings = self.settings.clone();
        settings.bridge = endpoint
            .as_ref()
            .and_then(|endpoint| self.bridge_invocation(endpoint.socket_path()));
        let invocation = Self::new(self.kind, settings);
        let args = invocation.argv(&prompt, resume.as_deref())?;

        if self.settings.relay_executable.is_none()
            && !self
                .warned_missing_executable
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            #[cfg(test)]
            self.missing_executable_warning_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tracing::warn!("{MISSING_EXECUTABLE_WARNING}");
        }
        let mut command = invocation.turn_command(&args);

        let child = self.spawn_turn(&mut command).map_err(|error| {
            RuntimeError::new(
                ErrorKind::Config,
                format!(
                    "could not start `{}`: {error}",
                    self.settings.executable.display()
                ),
            )
        })?;
        // The guard exists before the lazy stream, so dropping an unpolled
        // stream also stops the launcher and its descendants.
        let mut process = TurnProcess::new(child, endpoint);
        let stdout = process.child.stdout.take().ok_or_else(|| {
            RuntimeError::new(ErrorKind::Provider, "CLI agent produced no stdout stream")
        })?;

        let kind = self.kind;
        let cancel = request.cancel.clone();
        // The session backend owns the ledger; only successfully launched,
        // non-cancelled turns claim these session-level notices.
        let omission_diagnostics = record_turn_diagnostics(
            false,
            false,
            cancel.is_cancelled(),
            &self.reported_omissions,
            &omissions,
            kind,
        );
        let mut state = TurnState::for_bridge(invocation.settings.bridge.as_ref());
        let stream = async_stream::stream! {
            let mut lines = BufReader::new(stdout).lines();
            let mut cancelled = false;
            if !cancel.is_cancelled() {
                for message in omission_diagnostics {
                    yield ExternalAgentEvent::Diagnostic { message };
                }
            }

            loop {
                let next = tokio::select! {
                    _ = cancel.cancelled() => {
                        cancelled = true;
                        None
                    },
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
                    if terminal {
                        // The result deliberately ends the group immediately;
                        // the launcher alone was already killed here. Codex
                        // 0.162.1 resumed its thread afterwards in the live check.
                        // The CLI's remaining hooks and telemetry are cut short.
                        process.finish(kind).await;
                        state.terminated = true;
                        for message in turn_diagnostics(
                            process.bridge_offered, process.connected, cancel.is_cancelled(), &[], &[], kind,
                        ) {
                            yield ExternalAgentEvent::Diagnostic { message };
                        }
                    }
                    yield event;
                    if terminal {
                        break;
                    }
                }
                if state.terminated {
                    break;
                }
            }

            process.finish(kind).await;

            if !state.terminated {
                // The stream ended without the CLI reporting a terminal
                // result: a truncated run, not a completed one.
                let mut message = "CLI agent exited before reporting a result".to_owned();
                if !cancelled
                    && let Some(stderr) = &mut process.stderr
                    && let Some(reason) = stderr.reason().await
                    && !cancel.is_cancelled()
                {
                    message.push_str(": ");
                    message.push_str(&reason);
                }
                for message in turn_diagnostics(
                    process.bridge_offered, process.connected, cancelled || cancel.is_cancelled(), &[], &[], kind,
                ) {
                    yield ExternalAgentEvent::Diagnostic { message };
                }
                yield ExternalAgentEvent::Failed { message };
            }
        };

        Ok(Box::pin(stream))
    }
}

/// Decisions are independent of endpoint construction, including an empty bridged set.
fn turn_diagnostics(
    bridge_offered: bool,
    connected: bool,
    cancelled: bool,
    reported: &[BridgeOmission],
    omissions: &[BridgeOmission],
    kind: CliAgentKind,
) -> Vec<String> {
    if cancelled {
        return Vec::new();
    }
    let harness = smith_config::cli_agents::catalog_entry(kind.as_str())
        .expect("a supported CLI has a catalog entry")
        .description;
    let already_reported = reported
        .iter()
        .filter_map(omission_message)
        .collect::<Vec<_>>();
    let mut messages = Vec::new();
    for message in omissions.iter().filter_map(omission_message) {
        if !already_reported.contains(&message) && !messages.contains(&message) {
            messages.push(message);
        }
    }
    if bridge_offered && !connected {
        messages.push(format!(
            "{harness} did not load Smith's tools for this turn: it never connected to the bridge Smith offered, so it ran without them."
        ));
    }
    messages
}

fn omission_message(omission: &BridgeOmission) -> Option<String> {
    match omission {
        BridgeOmission::NameTooLong { name, bytes } => Some(format!(
            "Smith omitted tool `{name}` from the CLI bridge: its name is {bytes} bytes, exceeding the 128-byte limit."
        )),
        BridgeOmission::NameCollision { normalized, names } => {
            let mut names = names.clone();
            names.sort();
            let names = names
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "Smith omitted tools {names} from the CLI bridge: their names collide as `{normalized}` after the CLI's normalization."
            ))
        }
        BridgeOmission::Excluded { .. }
        | BridgeOmission::NoAuthorizedSkills { .. }
        | BridgeOmission::NoSupportingTool { .. } => None,
    }
}

fn record_turn_diagnostics(
    bridge_offered: bool,
    connected: bool,
    cancelled: bool,
    reported: &std::sync::Mutex<Vec<BridgeOmission>>,
    omissions: &[BridgeOmission],
    kind: CliAgentKind,
) -> Vec<String> {
    let mut reported = reported.lock().expect("bridge omissions");
    let messages = turn_diagnostics(
        bridge_offered,
        connected,
        cancelled,
        &reported,
        omissions,
        kind,
    );
    if !cancelled {
        for omission in omissions {
            if let Some(message) = omission_message(omission)
                && messages.contains(&message)
                && !reported
                    .iter()
                    .any(|old| omission_message(old).as_ref() == Some(&message))
            {
                tracing::warn!(
                    harness = kind.as_str(),
                    ?omission,
                    "CLI bridge omitted an unrepresentable tool name"
                );
                reported.push(omission.clone());
            }
        }
    }
    messages
}

/// Owns the process group and endpoint, including before the stream is polled.
struct TurnProcess {
    child: Child,
    // No descendant inherits this write end (spawn uses close-on-exec). Its
    // closure also protects abrupt Smith exits where Drop cannot run.
    lifetime: Option<ChildStdin>,
    endpoint: Option<BridgeEndpoint>,
    bridge_offered: bool,
    connected: bool,
    stderr: Option<StderrTail>,
    #[cfg(unix)]
    group: Option<nix::unistd::Pid>,
}

impl TurnProcess {
    fn new(mut child: Child, endpoint: Option<BridgeEndpoint>) -> Self {
        #[cfg(unix)]
        let group = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .map(nix::unistd::Pid::from_raw);
        let stderr = child.stderr.take().map(StderrTail::new);
        let lifetime = child.stdin.take();
        Self {
            child,
            lifetime,
            bridge_offered: endpoint.is_some(),
            connected: false,
            endpoint,
            stderr,
            #[cfg(unix)]
            group,
        }
    }

    async fn finish(&mut self, kind: CliAgentKind) {
        self.stop();
        let _ = self.child.wait().await;
        if let Some(endpoint) = self.endpoint.take() {
            self.connected = endpoint.connected();
            if !self.connected {
                tracing::warn!(harness = kind.as_str(), "CLI did not load Smith's tools");
            }
            endpoint.close().await;
        }
    }

    fn stop(&mut self) {
        // The group is killed before waiting or dropping the child. Taking
        // the id prevents a later drop from signalling a reused group id.
        #[cfg(unix)]
        if let Some(group) = self.group.take() {
            let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
        }
        let _ = self.child.start_kill();
        drop(self.lifetime.take());
        if let Some(stderr) = &mut self.stderr {
            stderr.stop();
        }
    }
}

impl Drop for TurnProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

const STDERR_TAIL_BYTES: usize = 4 * 1024;
const STDERR_REASON_CHARS: usize = 500;

/// Drains independently of stream polling, but never outlives its process guard.
struct StderrTail {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<Vec<u8>>,
}

impl StderrTail {
    fn new(mut stderr: ChildStderr) -> Self {
        let (stop, mut stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut tail = Vec::with_capacity(STDERR_TAIL_BYTES);
            let mut buffer = [0; STDERR_TAIL_BYTES];
            loop {
                let read = tokio::select! {
                    biased;
                    _ = &mut stopped => {
                        // Snapshot the queued bytes: waiting for EOF could
                        // hang on a descendant that inherited this pipe.
                        #[cfg(unix)]
                        drain_queued_stderr(&stderr, &mut buffer, &mut tail);
                        break;
                    },
                    read = stderr.read(&mut buffer) => read,
                };
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(count) => append_stderr_tail(&mut tail, &buffer[..count]),
                }
            }
            tail
        });
        Self {
            stop: Some(stop),
            task,
        }
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }

    async fn reason(&mut self) -> Option<String> {
        let tail = (&mut self.task).await.ok()?;
        let text = String::from_utf8_lossy(&tail);
        let line = text.lines().rev().find_map(|line| {
            let printable: String = line.chars().filter(|ch| !ch.is_control()).collect();
            let printable = printable.trim();
            (!printable.is_empty()).then(|| printable.to_owned())
        })?;
        if line.chars().count() > STDERR_REASON_CHARS {
            let mut reason: String = line.chars().take(STDERR_REASON_CHARS - 1).collect();
            reason.push('…');
            Some(reason)
        } else {
            Some(line)
        }
    }
}

impl Drop for StderrTail {
    fn drop(&mut self) {
        // A dropped, even unpolled, turn releases the reader and its pipe.
        self.task.abort();
    }
}

fn append_stderr_tail(tail: &mut Vec<u8>, bytes: &[u8]) {
    let discard = (tail.len() + bytes.len()).saturating_sub(STDERR_TAIL_BYTES);
    drop(tail.drain(..discard));
    tail.extend_from_slice(bytes);
}

#[cfg(unix)]
fn drain_queued_stderr(stderr: &ChildStderr, buffer: &mut [u8], tail: &mut Vec<u8>) {
    // Tokio's pipe is nonblocking. Read only the snapshot, so even an escaped
    // descendant writing continuously cannot prolong shutdown.
    let mut remaining = rustix::io::ioctl_fionread(stderr).unwrap_or(0);
    while remaining > 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        match rustix::io::read(stderr, &mut buffer[..count]) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                append_stderr_tail(tail, &buffer[..count]);
                remaining -= count as u64;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_agent::BridgeInvocation;
    use crate::cli_bridge::BRIDGE_SERVER_NAME;
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn settings(allow_own_tools: bool) -> CliAgentSettings {
        CliAgentSettings {
            executable: PathBuf::from("/usr/local/bin/agent"),
            model: Some("a-model".to_owned()),
            args: Vec::new(),
            cwd: PathBuf::from("/repo"),
            env: BTreeMap::new(),
            allow_own_tools,
            instructions: None,
            bridge: None,
            relay_executable: None,
        }
    }

    #[test]
    fn bridge_diagnostics_cover_every_connection_cancellation_and_reporting_state() {
        let too_long = BridgeOmission::NameTooLong {
            name: "oversized".into(),
            bytes: 129,
        };
        let collision = BridgeOmission::NameCollision {
            normalized: "remote_read".into(),
            names: vec!["remote.read".into(), "remote_read".into()],
        };
        let omissions = vec![
            BridgeOmission::Excluded {
                name: "shell".into(),
            },
            BridgeOmission::NoAuthorizedSkills {
                name: "registry.search".into(),
            },
            BridgeOmission::NoSupportingTool {
                name: "artifact.read".into(),
            },
            too_long.clone(),
            collision.clone(),
            too_long.clone(),
        ];
        for (kind, harness) in [
            (CliAgentKind::ClaudeCode, "Claude Code"),
            (CliAgentKind::Codex, "Codex"),
        ] {
            for offered in [false, true] {
                for connected in [false, true] {
                    for cancelled in [false, true] {
                        for reported_mask in 0..4 {
                            let mut reported = Vec::new();
                            if reported_mask & 1 != 0 {
                                reported.push(too_long.clone());
                            }
                            if reported_mask & 2 != 0 {
                                reported.push(collision.clone());
                            }
                            let mut expected = Vec::new();
                            if !cancelled {
                                if reported_mask & 1 == 0 {
                                    expected.push("Smith omitted tool `oversized` from the CLI bridge: its name is 129 bytes, exceeding the 128-byte limit.".to_owned());
                                }
                                if reported_mask & 2 == 0 {
                                    expected.push("Smith omitted tools `remote.read`, `remote_read` from the CLI bridge: their names collide as `remote_read` after the CLI's normalization.".to_owned());
                                }
                                if offered && !connected {
                                    expected.push(format!("{harness} did not load Smith's tools for this turn: it never connected to the bridge Smith offered, so it ran without them."));
                                }
                            }
                            assert_eq!(
                                turn_diagnostics(
                                    offered, connected, cancelled, &reported, &omissions, kind
                                ),
                                expected
                            );
                            let without_omissions = if !cancelled && offered && !connected {
                                vec![format!(
                                    "{harness} did not load Smith's tools for this turn: it never connected to the bridge Smith offered, so it ran without them."
                                )]
                            } else {
                                Vec::new()
                            };
                            assert_eq!(
                                turn_diagnostics(
                                    offered,
                                    connected,
                                    cancelled,
                                    &reported,
                                    &[],
                                    kind
                                ),
                                without_omissions
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bridge_name_diagnostics_are_once_per_session_even_if_registration_order_changes() {
        let omission = BridgeOmission::NameCollision {
            normalized: "a_b".into(),
            names: vec!["a.b".into(), "a_b".into()],
        };
        let reordered = BridgeOmission::NameCollision {
            normalized: "a_b".into(),
            names: vec!["a_b".into(), "a.b".into()],
        };
        assert_eq!(
            turn_diagnostics(
                false,
                false,
                false,
                &[],
                std::slice::from_ref(&omission),
                CliAgentKind::Codex
            ),
            vec![
                "Smith omitted tools `a.b`, `a_b` from the CLI bridge: their names collide as `a_b` after the CLI's normalization."
            ]
        );
        assert!(
            turn_diagnostics(
                false,
                false,
                false,
                &[omission],
                &[reordered],
                CliAgentKind::Codex
            )
            .is_empty()
        );
    }

    #[test]
    fn cancelled_turn_does_not_claim_omissions_for_the_session() {
        let reported = std::sync::Mutex::new(Vec::new());
        let omissions = [BridgeOmission::NameTooLong {
            name: "oversized".into(),
            bytes: 129,
        }];
        assert!(
            record_turn_diagnostics(
                false,
                false,
                true,
                &reported,
                &omissions,
                CliAgentKind::Codex
            )
            .is_empty()
        );
        assert!(reported.lock().expect("omissions").is_empty());
        assert_eq!(
            record_turn_diagnostics(
                false,
                false,
                false,
                &reported,
                &omissions,
                CliAgentKind::Codex
            )
            .len(),
            1
        );
        assert!(
            record_turn_diagnostics(
                false,
                false,
                false,
                &reported,
                &omissions,
                CliAgentKind::Codex
            )
            .is_empty()
        );
    }

    #[test]
    fn claude_without_its_own_tools_pre_allows_nothing() {
        let backend = CliAgentBackend::new(CliAgentKind::ClaudeCode, settings(false));
        let args = backend.argv("hello", None).expect("valid invocation paths");
        let allow = args.iter().position(|arg| arg == "--allowedTools");
        assert_eq!(args[allow.expect("an explicit allow-list") + 1], "");
        assert!(!args.iter().any(|arg| arg == "--permission-mode"));
    }

    #[test]
    fn codex_runs_read_only_without_its_own_tools_and_unsandboxed_with_them() {
        let read_only = CliAgentBackend::new(CliAgentKind::Codex, settings(false));
        let args = read_only
            .argv("hello", None)
            .expect("valid invocation paths");
        let sandbox = args.iter().position(|arg| arg == "--sandbox");
        assert_eq!(args[sandbox.expect("a sandbox mode") + 1], "read-only");
        assert!(!args.iter().any(|arg| arg.starts_with("--dangerously")));

        let own_tools = CliAgentBackend::new(CliAgentKind::Codex, settings(true));
        let args = own_tools
            .argv("hello", None)
            .expect("valid invocation paths");
        assert!(
            args.iter()
                .any(|arg| arg == "--dangerously-bypass-approvals-and-sandbox")
        );
        assert!(!args.iter().any(|arg| arg == "--sandbox"));
    }

    #[test]
    fn owner_arguments_follow_the_claude_permission_mode() {
        let mut settings = settings(true);
        settings.args = vec!["--permission-mode".into(), "acceptEdits".into()];
        let backend = CliAgentBackend::new(CliAgentKind::ClaudeCode, settings);
        let args = backend.argv("hello", None).expect("valid invocation paths");
        let modes = args
            .iter()
            .enumerate()
            .filter(|(_, arg)| *arg == "--permission-mode")
            .map(|(index, _)| args[index + 1].as_str())
            .collect::<Vec<_>>();
        // The CLI keeps the last value of a repeated single-value flag.
        assert_eq!(modes, ["bypassPermissions", "acceptEdits"]);
    }

    #[test]
    fn resuming_continues_each_cli_in_its_own_dialect() {
        let claude = CliAgentBackend::new(CliAgentKind::ClaudeCode, settings(false));
        let args = claude
            .argv("next", Some("session-1"))
            .expect("valid invocation paths");
        let resume = args.iter().position(|arg| arg == "--resume");
        assert_eq!(args[resume.expect("a resume flag") + 1], "session-1");

        let codex = CliAgentBackend::new(CliAgentKind::Codex, settings(false));
        let args = codex
            .argv("next", Some("thread-1"))
            .expect("valid invocation paths");
        assert_eq!(args[0], "exec");
        let resume = args.iter().position(|arg| arg == "resume").unwrap();
        assert_eq!(
            &args[1..resume],
            [
                "--json",
                "--skip-git-repo-check",
                "-C",
                "/repo",
                "--sandbox",
                "read-only",
                "-m",
                "a-model"
            ]
        );
        assert_eq!(&args[resume..], ["resume", "thread-1", "next"]);
    }

    #[test]
    fn codex_exec_flags_precede_resume_and_the_prompt() {
        // codex-0.162.1/stderr/exec-flags-after-resume-rejected.txt records
        // the rejection of -C after resume, which only accepts resume flags.
        let backend = CliAgentBackend::new(
            CliAgentKind::Codex,
            CliAgentSettings {
                args: vec![
                    "-c".to_owned(),
                    "check_for_update_on_startup=false".to_owned(),
                ],
                ..settings(false)
            },
        );
        let fresh = backend.argv("hello", None).expect("valid invocation paths");
        let resumed = backend
            .argv("next", Some("thread-1"))
            .expect("valid invocation paths");
        let resume = resumed.iter().position(|arg| arg == "resume").unwrap();
        assert_eq!(&resumed[..resume], &fresh[..fresh.len() - 1]);
        assert_eq!(&resumed[resume..], ["resume", "thread-1", "next"]);
        assert_eq!(
            &fresh[fresh.len() - 3..],
            ["-c", "check_for_update_on_startup=false", "hello"]
        );
    }

    fn bridge() -> BridgeInvocation {
        BridgeInvocation {
            server: BRIDGE_SERVER_NAME.to_owned(),
            relay_executable: PathBuf::from("/usr/local/bin/smith"),
            socket: PathBuf::from("/tmp/smith.sock"),
        }
    }

    fn explicit_environment(backend: &CliAgentBackend) -> BTreeMap<OsString, Option<OsString>> {
        let mut command = Command::new(&backend.settings.executable);
        backend.apply_environment(&mut command);
        command
            .as_std()
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(OsString::from)))
            .collect()
    }

    fn assert_claude_argv(own_tools: bool, resumed: bool, bridged: bool) {
        let mut settings = settings(own_tools);
        settings.args = vec!["--max-turns".into(), "3".into()];
        settings.env = [("OWNER_ENTRY".into(), "owner-value".into())].into();
        settings.bridge = bridged.then(bridge);
        let backend = CliAgentBackend::new(CliAgentKind::ClaudeCode, settings);
        let args = backend
            .argv("hello", resumed.then_some("session-1"))
            .expect("valid invocation paths");
        let mut expected = match (own_tools, bridged) {
            (false, false) => vec![
                "-p",
                "hello",
                "--output-format",
                "stream-json",
                "--verbose",
                "--allowedTools",
                "",
                "--model",
                "a-model",
            ],
            (true, false) => vec![
                "-p",
                "hello",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "bypassPermissions",
                "--model",
                "a-model",
            ],
            (false, true) => vec![
                "-p",
                "hello",
                "--output-format",
                "stream-json",
                "--verbose",
                "--allowedTools",
                "mcp__smith",
                "--mcp-config",
                r#"{"mcpServers":{"smith":{"args":["mcp-relay","/tmp/smith.sock"],"command":"/usr/local/bin/smith"}}}"#,
                "--model",
                "a-model",
            ],
            (true, true) => vec![
                "-p",
                "hello",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "bypassPermissions",
                "--allowedTools",
                "mcp__smith",
                "--mcp-config",
                r#"{"mcpServers":{"smith":{"args":["mcp-relay","/tmp/smith.sock"],"command":"/usr/local/bin/smith"}}}"#,
                "--model",
                "a-model",
            ],
        };
        if resumed {
            expected.extend(["--resume", "session-1"]);
        }
        expected.extend(["--max-turns", "3"]);
        assert_eq!(args, expected);
        for excluded in [
            "--strict-mcp-config",
            "--tools",
            "--system-prompt-snapshot",
            "--append-system-prompt",
        ] {
            assert!(!args.iter().any(|arg| arg == excluded), "{excluded}");
        }
        if bridged {
            let config = args.iter().position(|arg| arg == "--mcp-config").unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&args[config + 1]).unwrap(),
                serde_json::json!({
                    "mcpServers": {
                        "smith": {
                            "command": "/usr/local/bin/smith",
                            "args": ["mcp-relay", "/tmp/smith.sock"]
                        }
                    }
                })
            );
            let allowed = args.iter().position(|arg| arg == "--allowedTools").unwrap();
            assert_eq!(args[allowed + 1], "mcp__smith");
        }
        let mut expected_env = BTreeMap::from([(
            OsString::from("OWNER_ENTRY"),
            Some(OsString::from("owner-value")),
        )]);
        if bridged {
            expected_env.insert(OsString::from("MCP_TIMEOUT"), Some(OsString::from("60000")));
            expected_env.insert(
                OsString::from("MCP_TOOL_TIMEOUT"),
                Some(OsString::from("604800000")),
            );
        }
        assert_eq!(explicit_environment(&backend), expected_env);
    }

    fn assert_codex_argv(own_tools: bool, resumed: bool, bridged: bool) {
        let mut settings = settings(own_tools);
        settings.args = vec!["-c".into(), "check_for_update_on_startup=false".into()];
        settings.env = [("OWNER_ENTRY".into(), "owner-value".into())].into();
        settings.bridge = bridged.then(bridge);
        let backend = CliAgentBackend::new(CliAgentKind::Codex, settings);
        let args = backend
            .argv("hello", resumed.then_some("thread-1"))
            .expect("valid invocation paths");
        let mut expected = if own_tools {
            vec![
                "exec",
                "--json",
                "--skip-git-repo-check",
                "-C",
                "/repo",
                "--dangerously-bypass-approvals-and-sandbox",
                "-m",
                "a-model",
                "-c",
                "check_for_update_on_startup=false",
            ]
        } else {
            vec![
                "exec",
                "--json",
                "--skip-git-repo-check",
                "-C",
                "/repo",
                "--sandbox",
                "read-only",
                "-m",
                "a-model",
                "-c",
                "check_for_update_on_startup=false",
            ]
        };
        if bridged {
            expected.extend([
                "-c",
                r#"mcp_servers.smith.command="/usr/local/bin/smith""#,
                "-c",
                r#"mcp_servers.smith.args=["mcp-relay","/tmp/smith.sock"]"#,
                "-c",
                r#"mcp_servers.smith.default_tools_approval_mode="approve""#,
                "-c",
                "mcp_servers.smith.tool_timeout_sec=604800",
                "-c",
                "mcp_servers.smith.required=true",
                "-c",
                "mcp_servers.smith.startup_timeout_sec=60",
            ]);
        }
        if resumed {
            expected.extend(["resume", "thread-1"]);
            let resume = args.iter().position(|arg| arg == "resume").unwrap();
            assert!(
                args.iter()
                    .enumerate()
                    .filter(|(_, arg)| *arg == "-c")
                    .all(|(index, _)| index < resume)
            );
        }
        expected.push("hello");
        assert_eq!(args, expected);
        assert_eq!(args.last().map(String::as_str), Some("hello"));
        assert_eq!(
            explicit_environment(&backend),
            BTreeMap::from([(
                OsString::from("OWNER_ENTRY"),
                Some(OsString::from("owner-value")),
            )])
        );
    }

    macro_rules! argv_case {
        ($name:ident, $assertion:ident, $own_tools:literal, $resumed:literal, $bridged:literal) => {
            #[test]
            fn $name() {
                $assertion($own_tools, $resumed, $bridged);
            }
        };
    }

    argv_case!(
        claude_no_own_tools_first_unbridged,
        assert_claude_argv,
        false,
        false,
        false
    );
    argv_case!(
        claude_no_own_tools_resume_unbridged,
        assert_claude_argv,
        false,
        true,
        false
    );
    argv_case!(
        claude_own_tools_first_unbridged,
        assert_claude_argv,
        true,
        false,
        false
    );
    argv_case!(
        claude_own_tools_resume_unbridged,
        assert_claude_argv,
        true,
        true,
        false
    );
    argv_case!(
        claude_no_own_tools_first_bridged,
        assert_claude_argv,
        false,
        false,
        true
    );
    argv_case!(
        claude_no_own_tools_resume_bridged,
        assert_claude_argv,
        false,
        true,
        true
    );
    argv_case!(
        claude_own_tools_first_bridged,
        assert_claude_argv,
        true,
        false,
        true
    );
    argv_case!(
        claude_own_tools_resume_bridged,
        assert_claude_argv,
        true,
        true,
        true
    );
    argv_case!(
        codex_no_own_tools_first_unbridged,
        assert_codex_argv,
        false,
        false,
        false
    );
    argv_case!(
        codex_no_own_tools_resume_unbridged,
        assert_codex_argv,
        false,
        true,
        false
    );
    argv_case!(
        codex_own_tools_first_unbridged,
        assert_codex_argv,
        true,
        false,
        false
    );
    argv_case!(
        codex_own_tools_resume_unbridged,
        assert_codex_argv,
        true,
        true,
        false
    );
    argv_case!(
        codex_no_own_tools_first_bridged,
        assert_codex_argv,
        false,
        false,
        true
    );
    argv_case!(
        codex_no_own_tools_resume_bridged,
        assert_codex_argv,
        false,
        true,
        true
    );
    argv_case!(
        codex_own_tools_first_bridged,
        assert_codex_argv,
        true,
        false,
        true
    );
    argv_case!(
        codex_own_tools_resume_bridged,
        assert_codex_argv,
        true,
        true,
        true
    );

    #[test]
    fn claude_bridge_timeouts_override_owner_environment() {
        for kind in [CliAgentKind::ClaudeCode, CliAgentKind::Codex] {
            for bridged in [false, true] {
                let mut settings = settings(false);
                settings.env = [
                    ("MCP_TOOL_TIMEOUT".into(), "5".into()),
                    ("MCP_TIMEOUT".into(), "7".into()),
                    ("OWNER_ENTRY".into(), "owner-value".into()),
                ]
                .into();
                settings.bridge = bridged.then(bridge);
                let backend = CliAgentBackend::new(kind, settings);
                let timeout = if kind == CliAgentKind::ClaudeCode && bridged {
                    "604800000"
                } else {
                    "5"
                };
                let startup_timeout = if kind == CliAgentKind::ClaudeCode && bridged {
                    "60000"
                } else {
                    "7"
                };
                assert_eq!(
                    explicit_environment(&backend),
                    BTreeMap::from([
                        (
                            OsString::from("MCP_TIMEOUT"),
                            Some(OsString::from(startup_timeout))
                        ),
                        (
                            OsString::from("MCP_TOOL_TIMEOUT"),
                            Some(OsString::from(timeout))
                        ),
                        (
                            OsString::from("OWNER_ENTRY"),
                            Some(OsString::from("owner-value"))
                        ),
                    ])
                );
            }
        }
    }

    #[test]
    fn bridge_paths_round_trip_through_json_and_toml() {
        let executable = r#"/usr/local/bin/smith space"\relay"#;
        let socket = r#"/tmp/smith space"\socket"#;
        let bridge = BridgeInvocation {
            relay_executable: executable.into(),
            socket: socket.into(),
            ..bridge()
        };
        for kind in [CliAgentKind::ClaudeCode, CliAgentKind::Codex] {
            let backend = CliAgentBackend::new(
                kind,
                CliAgentSettings {
                    bridge: Some(bridge.clone()),
                    ..settings(false)
                },
            );
            let args = backend.argv("hello", None).expect("UTF-8 paths");
            match kind {
                CliAgentKind::ClaudeCode => {
                    let config = args.iter().position(|arg| arg == "--mcp-config").unwrap();
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(&args[config + 1]).unwrap(),
                        serde_json::json!({
                            "mcpServers": {
                                "smith": {"command": executable, "args": ["mcp-relay", socket]}
                            }
                        })
                    );
                }
                CliAgentKind::Codex => {
                    let overrides = args
                        .windows(2)
                        .filter(|pair| pair[0] == "-c")
                        .map(|pair| pair[1].as_str())
                        .collect::<Vec<_>>();
                    assert_eq!(
                        overrides,
                        [
                            r#"mcp_servers.smith.command="/usr/local/bin/smith space\"\\relay""#,
                            r#"mcp_servers.smith.args=["mcp-relay","/tmp/smith space\"\\socket"]"#,
                            r#"mcp_servers.smith.default_tools_approval_mode="approve""#,
                            "mcp_servers.smith.tool_timeout_sec=604800",
                            "mcp_servers.smith.required=true",
                            "mcp_servers.smith.startup_timeout_sec=60",
                        ]
                    );
                    let expected_values = [
                        toml::Value::String(executable.into()),
                        toml::Value::Array(vec![
                            toml::Value::String("mcp-relay".into()),
                            toml::Value::String(socket.into()),
                        ]),
                        toml::Value::String("approve".into()),
                        toml::Value::Integer(604800),
                        toml::Value::Boolean(true),
                        toml::Value::Integer(60),
                    ];
                    for (override_arg, expected) in overrides.iter().zip(expected_values) {
                        let (_, value) = override_arg.split_once('=').expect("Codex override");
                        assert_eq!(
                            value.parse::<toml::Value>().expect("Codex TOML value"),
                            expected
                        );
                    }
                    let config: toml::Table = overrides.join("\n").parse().expect("Codex TOML");
                    let server = &config["mcp_servers"]["smith"];
                    assert_eq!(server["command"].as_str(), Some(executable));
                    assert_eq!(
                        server["args"].as_array().unwrap(),
                        &vec![
                            toml::Value::String("mcp-relay".into()),
                            toml::Value::String(socket.into()),
                        ]
                    );
                    assert_eq!(
                        server["default_tools_approval_mode"].as_str(),
                        Some("approve")
                    );
                    assert_eq!(server["tool_timeout_sec"].as_integer(), Some(604800));
                    assert_eq!(server["required"].as_bool(), Some(true));
                    assert_eq!(server["startup_timeout_sec"].as_integer(), Some(60));
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_bridge_paths_fail_when_building_either_invocation() {
        use std::os::unix::ffi::OsStringExt as _;

        let path = PathBuf::from(OsString::from_vec(b"/tmp/invalid-\xff.sock".to_vec()));
        for kind in [CliAgentKind::ClaudeCode, CliAgentKind::Codex] {
            for invalid_socket in [true, false] {
                let mut bridge = bridge();
                if invalid_socket {
                    bridge.socket = path.clone();
                } else {
                    bridge.relay_executable = path.clone();
                }
                let backend = CliAgentBackend::new(
                    kind,
                    CliAgentSettings {
                        executable: "/smith-must-not-start/agent".into(),
                        bridge: Some(bridge),
                        ..settings(false)
                    },
                );
                let expected = format!("CLI bridge path is not valid UTF-8: {path:?}");
                let error = backend
                    .argv("hello", None)
                    .expect_err("invalid bridge path");
                assert_eq!(error.kind, ErrorKind::Config);
                assert_eq!(error.message, expected);
            }
        }
    }
}
