//! The `shell` tool.
//!
//! Runs a command in the project root and returns its combined output.
//!
//! The hard part is not running the command; it is making sure nothing outlives
//! the invocation — unless the caller explicitly asked it to. A build script
//! that spawns a watcher, a test runner that forks workers — killing only the
//! direct child leaves those orphaned, holding ports and burning CPU after the
//! user pressed Escape. So the child gets its **own process group**, and
//! cancellation signals the whole group.
//!
//! Signalling uses `nix`'s safe `killpg` wrapper rather than raw `libc`, which
//! is what lets this crate keep `unsafe_code = "forbid"`.
//!
//! `run_in_background` and manual backgrounding (ctrl+b in the TUI) are the
//! two ways ownership of that process group moves from this invocation to the
//! session's background task registry, through the [`crate::background`]
//! seam. Neither path exists without an injected host service: without it, an
//! explicit `run_in_background` request fails clearly, and a foreground call
//! simply has nothing to hand off to, so it runs exactly as it always has.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::security::{PermissionSet, SecurityResource};
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolCallDisplay, ToolEffects,
    ToolOutcome, ToolSpec,
};
use agent_runtime_registry::Permission;
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

use crate::background;
use crate::support::{
    invalid, optional_bool, optional_str, optional_usize, prepare_path_argument, require_str,
    resolve,
};

/// The default time a command may run.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// The longest timeout a caller may request.
const MAX_TIMEOUT_MS: u64 = 600_000;

/// Hard process-output capture boundary.
///
/// Ordinary model-facing output is bounded later by Agent Runtime. Standard
/// Smith hosts first offload results above 64 KiB into their protected
/// artifact store, so this larger limit is solely a memory/resource guard.
const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

/// How long a signalled process group gets to exit before it is killed.
const GRACE: Duration = Duration::from_millis(500);

/// Resource kind used for the deliberately unsandboxed same-user host shell.
///
/// A working directory below the project is only process setup; it does not
/// constrain the filesystem, inherited environment, child processes, network,
/// or data egress available to `sh -c`.
pub const HOST_SHELL_RESOURCE_KIND: &str = "host-shell";

const HOST_SHELL_ENVIRONMENT_POLICY: &str = "inherit-host-environment-v1";

/// Runs a shell command through an explicitly composed background-task host.
#[derive(Debug, Clone)]
pub struct ShellTool {
    background: Arc<dyn background::BackgroundTaskHost>,
}

impl ShellTool {
    /// Builds the tool with the background-task owner for this runtime.
    pub fn new(background: Arc<dyn background::BackgroundTaskHost>) -> Self {
        Self { background }
    }
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new(background::unavailable())
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "shell",
            "Run an unsandboxed same-user host shell command with the project as its \
             initial working directory. The command can access host files, inherited \
             environment and credentials, child processes, the network, and data \
             egress outside the project. Returns combined stdout/stderr and the exit \
             status; killed at the timeout along with anything it \
             spawned. Set `run_in_background` for a long command and poll it with \
             `task_output` instead of blocking the turn.",
            json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command line to run, interpreted by `sh -c`."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory, relative to the project root. Defaults to the root."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Time limit in milliseconds. Defaults to 120000, max 600000. Optional for a background task; if omitted, it has no deadline."
                    },
                    "run_in_background": {
                        "type": "boolean",
                        "description": "Run in the background instead of waiting. Returns immediately with a task ID; the process keeps running. Poll output with `task_output`, stop early with `task_stop`. No deadline unless `timeout_ms` is set. Defaults to false."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
            ToolEffects::new(vec![])
                .with_host_read(HOST_SHELL_RESOURCE_KIND)
                .with_host_write(HOST_SHELL_RESOURCE_KIND, "host:filesystem")
                .with_spawn()
                .with_network()
                .with_data_egress_to("host-network:any"),
        )
        .with_permission_upper_bound(shell_permissions())
    }

    async fn prepare(
        &self,
        mut arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let command = require_str(&arguments, "command")?.to_owned();
        if command.trim().is_empty() {
            return Err(invalid("`command` must not be empty"));
        }
        let cwd = prepare_path_argument(&mut arguments, "cwd", Some("."), ctx)?;
        let run_in_background = optional_bool(&arguments, "run_in_background").unwrap_or(false);
        // A background task's deadline is opt-in: injecting the foreground
        // default here would silently kill every background command at 120s,
        // defeating the one thing `run_in_background` exists for. An explicit
        // `timeout_ms` is still honored and clamped, same as foreground.
        if run_in_background {
            if let Some(ms) = optional_usize(&arguments, "timeout_ms") {
                let clamped = (ms as u64).clamp(1, MAX_TIMEOUT_MS);
                arguments
                    .as_object_mut()
                    .ok_or_else(|| invalid("tool arguments must be a JSON object"))?
                    .insert("timeout_ms".to_owned(), Value::from(clamped));
            }
        } else {
            let timeout_ms = optional_usize(&arguments, "timeout_ms")
                .map_or(DEFAULT_TIMEOUT_MS, |ms| (ms as u64).min(MAX_TIMEOUT_MS))
                .max(1);
            arguments
                .as_object_mut()
                .ok_or_else(|| invalid("tool arguments must be a JSON object"))?
                .insert("timeout_ms".to_owned(), Value::from(timeout_ms));
        }
        let effects = ToolEffects::new(vec![])
            .with_host_read(HOST_SHELL_RESOURCE_KIND)
            .with_host_write(HOST_SHELL_RESOURCE_KIND, "host:filesystem")
            .with_spawn()
            .with_network()
            .with_data_egress_to("host-network:any");
        let action_revision = host_shell_action_revision(
            &command,
            &cwd.display,
            run_in_background,
            optional_usize(&arguments, "timeout_ms").map(|value| value as u64),
        );
        Ok(PreparedToolCall::new(
            ctx.call_id.clone(),
            "shell",
            arguments,
            shell_permissions(),
            SecurityResource::other(HOST_SHELL_RESOURCE_KIND, action_revision),
            effects,
            ToolCallDisplay::new(format!("Run unsandboxed host shell in {}", cwd.display))
                .with_detail(format!(
                    "{command}\nHost access: same-user files, inherited environment and credentials, child processes, network, and data egress"
                )),
        ))
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let arguments = prepared.into_arguments();
        let command = require_str(&arguments, "command")?;
        if command.trim().is_empty() {
            return Err(invalid("`command` must not be empty"));
        }
        let cwd = resolve(ctx, optional_str(&arguments, "cwd").unwrap_or("."))?;
        let run_in_background = optional_bool(&arguments, "run_in_background").unwrap_or(false);

        if run_in_background {
            // No default deadline here, deliberately: `prepare` only carried a
            // `timeout_ms` through if the caller supplied one.
            let timeout_ms = optional_usize(&arguments, "timeout_ms")
                .map(|ms| (ms as u64).clamp(1, MAX_TIMEOUT_MS));
            let spawned = self
                .background
                .spawn(ctx.session.clone(), command.to_owned(), cwd, timeout_ms)
                .await?;
            return Ok(ToolOutcome {
                value: json!({
                    "command": command,
                    "task_id": spawned.task_id,
                    "spool_ref": spawned.spool_ref,
                    "running": true,
                }),
                content: vec![agent_runtime_core::content::ContentPart::text(format!(
                    "Started in the background as task {}. Poll `task_output` with this \
                     task ID to read its output, or stop it with `task_stop`.",
                    spawned.task_id
                ))]
                .into(),
                is_error: false,
            });
        }

        let timeout_ms = optional_usize(&arguments, "timeout_ms")
            .map_or(DEFAULT_TIMEOUT_MS, |ms| (ms as u64).min(MAX_TIMEOUT_MS))
            .max(1);

        let mut child = spawn(command, &cwd)?;

        // stdout and stderr are merged in arrival order, because a compiler
        // writes diagnostics to stderr and progress to stdout, and reading them
        // separately loses which happened first.
        let stdout = child.stdout.take().ok_or_else(|| internal("stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| internal("stderr"))?;
        let (lines_tx, mut lines_rx) = tokio::sync::mpsc::channel::<String>(256);
        for (stream, tx) in [
            (
                Box::new(stdout) as Box<dyn tokio::io::AsyncRead + Unpin + Send>,
                lines_tx.clone(),
            ),
            (Box::new(stderr), lines_tx),
        ] {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if tx.send(line).await.is_err() {
                        break;
                    }
                }
            });
        }

        // A host lets the user rescue this call mid-flight (ctrl+b in the
        // TUI); without one there is nowhere to hand the process off to, so
        // this stays `None` and the extra `select!` arm never fires.
        let mut background_rx = self
            .background
            .register_foreground_signal(ctx.session.clone());

        let group = child.id();
        let mut output = String::new();
        let mut truncated = false;

        // Lines are accumulated inline, in the same loop that watches for
        // exit/timeout/cancel/backgrounding, rather than in a detached
        // collector task: the manual-backgrounding arm needs both the
        // output captured so far *and* the still-open receiver in hand at
        // the moment it fires, to pass on to the registry.
        let status = {
            let terminal = async {
                tokio::select! {
                    status = child.wait() => Outcome::Exited(status),
                    () = sleep_ms(timeout_ms) => {
                        stop_group(&mut child, group).await;
                        Outcome::TimedOut
                    }
                    () = ctx.cancel.cancelled() => {
                        stop_group(&mut child, group).await;
                        Outcome::Cancelled
                    }
                    () = recv_background_signal(&mut background_rx) => Outcome::Backgrounded,
                }
            };
            tokio::pin!(terminal);

            let mut lines_open = true;
            loop {
                tokio::select! {
                    maybe_line = lines_rx.recv(), if lines_open => {
                        match maybe_line {
                            Some(line) => accumulate(&mut output, &mut truncated, line),
                            None => lines_open = false,
                        }
                    }
                    outcome = &mut terminal => break outcome,
                }
            }
        };

        if !matches!(status, Outcome::Backgrounded) {
            // The process is done (or killed); drain whatever its readers
            // still had buffered so the captured output matches exactly what
            // the old detached-collector shape produced.
            while let Some(line) = lines_rx.recv().await {
                accumulate(&mut output, &mut truncated, line);
            }
        }

        match status {
            Outcome::Cancelled => Err(RuntimeError::new(
                ErrorKind::Cancelled,
                "the command was cancelled and its process group stopped",
            )),
            Outcome::TimedOut => Ok(ToolOutcome {
                value: json!({
                    "command": command,
                    "timed_out": true,
                    "timeout_ms": timeout_ms,
                    "truncated": truncated,
                }),
                content: vec![agent_runtime_core::content::ContentPart::text(format!(
                    "{}\n[killed after {timeout_ms}ms, with its process group]\n\
                     The command did not finish within its {timeout_ms}ms timeout. Options: \
                     raise `timeout_ms` (up to {MAX_TIMEOUT_MS}), narrow the command so it \
                     does less per call, or rerun with `run_in_background: true` to let it \
                     keep running as a session-owned task and read its result with \
                     `task_output`.",
                    render(&output, truncated)
                ))]
                .into(),
                is_error: true,
            }),
            Outcome::Exited(status) => {
                let code = status
                    .as_ref()
                    .ok()
                    .and_then(std::process::ExitStatus::code);
                let success = status.as_ref().is_ok_and(std::process::ExitStatus::success);
                let mut rendered = render(&output, truncated);
                if !success {
                    // A silent non-zero exit reads as success to a model
                    // skimming for output, so the status is always stated.
                    rendered.push_str(&match code {
                        Some(code) => format!("\n[exit status {code}]"),
                        None => "\n[terminated by a signal]".to_owned(),
                    });
                }
                Ok(ToolOutcome {
                    value: json!({
                        "command": command,
                        "exit_code": code,
                        "success": success,
                        "timed_out": false,
                        "truncated": truncated,
                    }),
                    content: vec![agent_runtime_core::content::ContentPart::text(rendered)].into(),
                    is_error: !success,
                })
            }
            Outcome::Backgrounded => {
                let spawned = self
                    .background
                    .adopt(
                        ctx.session.clone(),
                        command.to_owned(),
                        cwd,
                        child,
                        group,
                        output.clone(),
                        lines_rx,
                    )
                    .await?;
                Ok(ToolOutcome {
                    value: json!({
                        "command": command,
                        "task_id": spawned.task_id,
                        "spool_ref": spawned.spool_ref,
                        "running": true,
                        "backgrounded_by_user": true,
                    }),
                    content: vec![agent_runtime_core::content::ContentPart::text(format!(
                        "{}\n[moved to the background by the user as task {}; it has NOT \
                         completed — poll `task_output` with this task ID for its result, or \
                         stop it with `task_stop`]",
                        render(&output, truncated),
                        spawned.task_id
                    ))]
                    .into(),
                    is_error: false,
                })
            }
        }
    }
}

fn host_shell_action_revision(
    command: &str,
    cwd: &str,
    run_in_background: bool,
    timeout_ms: Option<u64>,
) -> String {
    let mut digest = Sha256::new();
    for field in [
        "smith-host-shell-action-v1",
        command,
        cwd,
        if run_in_background {
            "background"
        } else {
            "foreground"
        },
        HOST_SHELL_ENVIRONMENT_POLICY,
    ] {
        digest.update(field.len().to_be_bytes());
        digest.update(field.as_bytes());
    }
    match timeout_ms {
        Some(timeout_ms) => {
            digest.update([1]);
            digest.update(timeout_ms.to_be_bytes());
        }
        None => digest.update([0]),
    }
    format!("sha256:{:x}", digest.finalize())
}

/// Waits for a manual-backgrounding signal, or never resolves without one.
///
/// A plain `async fn` rather than an inline block so the `select!` branch
/// stays readable; called fresh each poll, which is safe because it borrows
/// rather than consumes the receiver.
async fn recv_background_signal(rx: &mut Option<oneshot::Receiver<()>>) {
    match rx {
        Some(receiver) => {
            let _ = receiver.await;
        }
        None => std::future::pending::<()>().await,
    }
}

/// Appends one line to the captured output, truncating rather than growing
/// past [`MAX_CAPTURE_BYTES`] once a chatty command exceeds it.
fn accumulate(output: &mut String, truncated: &mut bool, line: String) {
    if output.len() + line.len() + 1 > MAX_CAPTURE_BYTES {
        *truncated = true;
        return;
    }
    output.push_str(&line);
    output.push('\n');
}

fn shell_permissions() -> PermissionSet {
    [
        Permission::HostFsRead,
        Permission::HostFsWrite,
        Permission::ProcessSpawn,
        Permission::NetHttp,
        Permission::DataEgress,
    ]
    .into_iter()
    .collect()
}

enum Outcome {
    Exited(std::io::Result<std::process::ExitStatus>),
    TimedOut,
    Cancelled,
    /// The user moved this call to the background (ctrl+b) before it
    /// finished; the registry now owns the process.
    Backgrounded,
}

fn render(output: &str, truncated: bool) -> String {
    let body = if output.is_empty() {
        "(no output)".to_owned()
    } else {
        output.trim_end().to_owned()
    };
    if truncated {
        format!("{body}\n[output truncated at {MAX_CAPTURE_BYTES} bytes]")
    } else {
        body
    }
}

fn internal(stream: &str) -> RuntimeError {
    RuntimeError::new(
        ErrorKind::Tool,
        format!("could not capture the command's {stream}"),
    )
}

async fn sleep_ms(millis: u64) {
    tokio::time::sleep(Duration::from_millis(millis)).await;
}

#[cfg(unix)]
fn spawn(command: &str, cwd: &std::path::Path) -> Result<tokio::process::Child, RuntimeError> {
    let mut builder = Command::new("sh");
    builder
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // `0` means "a new group whose id is this process's pid", which is what
    // makes the whole tree signallable as a unit.
    builder.process_group(0);
    builder
        .spawn()
        .map_err(|err| invalid(format!("cannot run `{command}`: {err}")))
}

#[cfg(not(unix))]
fn spawn(command: &str, cwd: &std::path::Path) -> Result<tokio::process::Child, RuntimeError> {
    Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| invalid(format!("cannot run `{command}`: {err}")))
}

/// Stops the command and everything it spawned.
///
/// `SIGTERM` to the group first so a well-behaved process can clean up, then
/// `SIGKILL` to whatever ignored it.
#[cfg(unix)]
async fn stop_group(child: &mut tokio::process::Child, group: Option<u32>) {
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;

    let Some(group) = group.and_then(|id| i32::try_from(id).ok()) else {
        let _ = child.kill().await;
        return;
    };
    let pid = Pid::from_raw(group);

    let _ = killpg(pid, Signal::SIGTERM);
    if tokio::time::timeout(GRACE, child.wait()).await.is_ok() {
        return;
    }
    let _ = killpg(pid, Signal::SIGKILL);
    let _ = child.wait().await;
}

#[cfg(not(unix))]
async fn stop_group(child: &mut tokio::process::Child, _group: Option<u32>) {
    let _ = child.kill().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{project, text_of};

    #[tokio::test]
    async fn a_command_returns_its_output() {
        let (_dir, ctx) = project();
        let outcome = ShellTool::default()
            .invoke(json!({"command": "echo hello"}), &ctx)
            .await
            .unwrap();

        assert!(!outcome.is_error);
        assert_eq!(outcome.value["exit_code"], 0);
        assert!(text_of(&outcome).contains("hello"));
    }

    #[tokio::test]
    async fn stdout_and_stderr_are_both_captured() {
        let (_dir, ctx) = project();
        let outcome = ShellTool::default()
            .invoke(json!({"command": "echo out; echo err 1>&2"}), &ctx)
            .await
            .unwrap();

        let text = text_of(&outcome);
        assert!(text.contains("out"), "{text}");
        assert!(text.contains("err"), "stderr must be captured too: {text}");
    }

    #[tokio::test]
    async fn a_failing_command_states_its_exit_status() {
        let (_dir, ctx) = project();
        let outcome = ShellTool::default()
            .invoke(json!({"command": "exit 3"}), &ctx)
            .await
            .unwrap();

        assert!(outcome.is_error);
        assert_eq!(outcome.value["exit_code"], 3);
        // Silence plus a zero-ish look would read as success.
        assert!(text_of(&outcome).contains("[exit status 3]"));
    }

    #[tokio::test]
    async fn a_silent_command_says_no_output_rather_than_nothing() {
        let (_dir, ctx) = project();
        let outcome = ShellTool::default()
            .invoke(json!({"command": "true"}), &ctx)
            .await
            .unwrap();
        assert!(text_of(&outcome).contains("(no output)"));
    }

    #[tokio::test]
    async fn the_command_runs_in_the_project_root() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();

        let outcome = ShellTool::default()
            .invoke(json!({"command": "ls"}), &ctx)
            .await
            .unwrap();
        assert!(text_of(&outcome).contains("marker.txt"));
    }

    #[tokio::test]
    async fn a_cwd_outside_the_project_is_refused() {
        let (_dir, ctx) = project();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("beyond-marker.txt"), "x").unwrap();

        let err = ShellTool::default()
            .invoke(
                json!({"command": "ls", "cwd": outside.path().to_str().unwrap()}),
                &ctx,
            )
            .await
            .expect_err("shell cwd remains a project-relative setup field");
        assert_eq!(err.kind, ErrorKind::Workspace);
    }

    #[tokio::test]
    async fn a_slow_command_is_killed_at_its_timeout() {
        let (_dir, ctx) = project();
        let outcome = ShellTool::default()
            .invoke(json!({"command": "sleep 30", "timeout_ms": 300}), &ctx)
            .await
            .unwrap();

        assert!(outcome.is_error);
        assert_eq!(outcome.value["timed_out"], true);
        assert!(text_of(&outcome).contains("killed after 300ms"));
    }

    #[tokio::test]
    async fn a_timeout_names_all_three_ways_out() {
        let (_dir, ctx) = project();
        let outcome = ShellTool::default()
            .invoke(json!({"command": "sleep 30", "timeout_ms": 300}), &ctx)
            .await
            .unwrap();

        let text = text_of(&outcome);
        // The kill marker survives verbatim: other tests and any tooling that
        // greps transcripts for it must not need to change.
        assert!(text.contains("killed after 300ms"), "{text}");
        assert!(text.contains("timeout_ms"), "{text}");
        assert!(text.to_lowercase().contains("narrow"), "{text}");
        assert!(text.contains("run_in_background"), "{text}");
    }

    #[tokio::test]
    async fn run_in_background_without_an_installed_host_fails_clearly() {
        let (_dir, ctx) = project();
        let err = ShellTool::default()
            .invoke(
                json!({"command": "echo hi", "run_in_background": true}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("does not provide"), "{err:?}");
    }

    #[tokio::test]
    async fn a_background_request_does_not_default_inject_a_timeout() {
        // `prepare` must not stamp the foreground 120s default onto a
        // background call — the whole point is "no deadline unless asked".
        let (_dir, ctx) = project();
        let preparation = crate::testing::preparation_context(&ctx);
        let prepared = ShellTool::default()
            .prepare(
                json!({"command": "sleep 1", "run_in_background": true}),
                &preparation,
            )
            .await
            .unwrap();
        assert!(
            prepared.arguments().get("timeout_ms").is_none(),
            "{:?}",
            prepared.arguments()
        );
    }

    #[tokio::test]
    async fn an_explicit_timeout_survives_a_background_request() {
        let (_dir, ctx) = project();
        let preparation = crate::testing::preparation_context(&ctx);
        let prepared = ShellTool::default()
            .prepare(
                json!({"command": "sleep 1", "run_in_background": true, "timeout_ms": 5_000}),
                &preparation,
            )
            .await
            .unwrap();
        assert_eq!(prepared.arguments()["timeout_ms"], 5_000);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_timeout_kills_the_whole_process_group_not_just_the_shell() {
        let (dir, ctx) = project();
        let marker = dir.path().join("orphan.txt");

        // The `sh` exits immediately; its background child would survive a
        // kill aimed only at the direct child, and would then create the file.
        let command = format!(
            "( sleep 1; echo orphaned > {} ) & sleep 30",
            marker.display()
        );
        let outcome = ShellTool::default()
            .invoke(json!({"command": command, "timeout_ms": 200}), &ctx)
            .await
            .unwrap();
        assert_eq!(outcome.value["timed_out"], true);

        // Well past when the orphan would have written, had it survived.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            !marker.exists(),
            "a spawned descendant outlived the invocation"
        );
    }

    #[tokio::test]
    async fn cancellation_stops_the_command() {
        let (_dir, ctx) = project();
        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            cancel.cancel(agent_runtime_core::cancel::CancelReason::UserRequested);
        });

        let err = ShellTool::default()
            .invoke(json!({"command": "sleep 30"}), &ctx)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Cancelled);
    }

    #[tokio::test]
    async fn flooding_output_is_truncated_rather_than_unbounded() {
        let (_dir, ctx) = project();
        let bytes = MAX_CAPTURE_BYTES + 1024 * 1024;
        let outcome = ShellTool::default()
            .invoke(
                json!({"command": format!("yes 'a line of output' | head -c {bytes}")}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(outcome.value["truncated"], true);
        assert!(text_of(&outcome).len() < MAX_CAPTURE_BYTES * 2);
    }

    #[tokio::test]
    async fn output_above_the_old_128k_cutoff_remains_exact_for_the_offloader() {
        let (_dir, ctx) = project();
        let outcome = ShellTool::default()
            .invoke(
                json!({"command": "yes 'recoverable output' | head -c 262144"}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(outcome.value["truncated"], false);
        assert!(text_of(&outcome).len() > 128 * 1024);
    }

    #[tokio::test]
    async fn an_oversized_timeout_is_clamped_not_honored() {
        let (_dir, ctx) = project();
        let outcome = ShellTool::default()
            .invoke(json!({"command": "true", "timeout_ms": 99_000_000}), &ctx)
            .await
            .unwrap();
        assert!(!outcome.is_error);
    }

    #[tokio::test]
    async fn an_empty_command_is_rejected() {
        let (_dir, ctx) = project();
        assert!(
            ShellTool::default()
                .invoke(json!({"command": "   "}), &ctx)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn the_tool_declares_spawn_and_write_effects() {
        let effects = ShellTool::default().spec().effects;
        assert!(effects.spawns_process());
        assert!(effects.mutates());
    }
}
