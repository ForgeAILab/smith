//! The `shell` tool.
//!
//! Runs a command in the project root and returns its combined output.
//!
//! The hard part is not running the command; it is making sure nothing outlives
//! the invocation. A build script that spawns a watcher, a test runner that
//! forks workers — killing only the direct child leaves those orphaned, holding
//! ports and burning CPU after the user pressed Escape. So the child gets its
//! **own process group**, and cancellation signals the whole group.
//!
//! Signalling uses `nix`'s safe `killpg` wrapper rather than raw `libc`, which
//! is what lets this crate keep `unsafe_code = "forbid"`.

use std::process::Stdio;
use std::time::Duration;

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::tool::{InvocationContext, Tool, ToolEffects, ToolOutcome};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::support::{invalid, optional_str, optional_usize, require_str, resolve};

/// The default time a command may run.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// The longest timeout a caller may request.
const MAX_TIMEOUT_MS: u64 = 600_000;

/// The most output bytes retained. Beyond this the head is kept: a compiler's
/// first error is what matters, and the thousandth is noise.
const MAX_OUTPUT_BYTES: usize = 128 * 1024;

/// How long a signalled process group gets to exit before it is killed.
const GRACE: Duration = Duration::from_millis(500);

/// Runs a shell command inside the project.
#[derive(Debug, Default, Clone, Copy)]
pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Run a shell command in the project root. Returns combined stdout and \
         stderr with the exit status. Commands are killed at the timeout, along \
         with any processes they spawned."
    }

    fn input_schema(&self) -> Value {
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
                    "description": "Time limit in milliseconds. Defaults to 120000, maximum 600000."
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn effects(&self) -> ToolEffects {
        ToolEffects::read_only()
            .with_write("project:files")
            .with_spawn()
            .with_network()
    }

    async fn invoke(
        &self,
        arguments: Value,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let command = require_str(&arguments, "command")?;
        if command.trim().is_empty() {
            return Err(invalid("`command` must not be empty"));
        }
        let cwd = resolve(ctx, optional_str(&arguments, "cwd").unwrap_or("."))?;
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

        let collector = tokio::spawn(async move {
            let mut output = String::new();
            let mut truncated = false;
            while let Some(line) = lines_rx.recv().await {
                if output.len() + line.len() + 1 > MAX_OUTPUT_BYTES {
                    truncated = true;
                    continue;
                }
                output.push_str(&line);
                output.push('\n');
            }
            (output, truncated)
        });

        let group = child.id();
        let status = tokio::select! {
            status = child.wait() => Outcome::Exited(status),
            () = sleep_ms(timeout_ms) => {
                stop_group(&mut child, group).await;
                Outcome::TimedOut
            }
            () = ctx.cancel.cancelled() => {
                stop_group(&mut child, group).await;
                Outcome::Cancelled
            }
        };

        let (output, truncated) = collector.await.unwrap_or_else(|_| (String::new(), false));

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
                    "{}\n[killed after {timeout_ms}ms, with its process group]",
                    render(&output, truncated)
                ))],
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
                    content: vec![agent_runtime_core::content::ContentPart::text(rendered)],
                    is_error: !success,
                })
            }
        }
    }
}

enum Outcome {
    Exited(std::io::Result<std::process::ExitStatus>),
    TimedOut,
    Cancelled,
}

fn render(output: &str, truncated: bool) -> String {
    let body = if output.is_empty() {
        "(no output)".to_owned()
    } else {
        output.trim_end().to_owned()
    };
    if truncated {
        format!("{body}\n[output truncated at {MAX_OUTPUT_BYTES} bytes]")
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
        let outcome = ShellTool
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
        let outcome = ShellTool
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
        let outcome = ShellTool
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
        let outcome = ShellTool
            .invoke(json!({"command": "true"}), &ctx)
            .await
            .unwrap();
        assert!(text_of(&outcome).contains("(no output)"));
    }

    #[tokio::test]
    async fn the_command_runs_in_the_project_root() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();

        let outcome = ShellTool
            .invoke(json!({"command": "ls"}), &ctx)
            .await
            .unwrap();
        assert!(text_of(&outcome).contains("marker.txt"));
    }

    #[tokio::test]
    async fn a_cwd_outside_the_project_is_refused() {
        let (_dir, ctx) = project();
        let err = ShellTool
            .invoke(json!({"command": "ls", "cwd": "../.."}), &ctx)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Workspace);
    }

    #[tokio::test]
    async fn a_slow_command_is_killed_at_its_timeout() {
        let (_dir, ctx) = project();
        let outcome = ShellTool
            .invoke(json!({"command": "sleep 30", "timeout_ms": 300}), &ctx)
            .await
            .unwrap();

        assert!(outcome.is_error);
        assert_eq!(outcome.value["timed_out"], true);
        assert!(text_of(&outcome).contains("killed after 300ms"));
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
        let outcome = ShellTool
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

        let err = ShellTool
            .invoke(json!({"command": "sleep 30"}), &ctx)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Cancelled);
    }

    #[tokio::test]
    async fn flooding_output_is_truncated_rather_than_unbounded() {
        let (_dir, ctx) = project();
        let outcome = ShellTool
            .invoke(
                json!({"command": "for i in $(seq 1 200000); do echo 'a line of output'; done"}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(outcome.value["truncated"], true);
        assert!(text_of(&outcome).len() < MAX_OUTPUT_BYTES * 2);
    }

    #[tokio::test]
    async fn an_oversized_timeout_is_clamped_not_honored() {
        let (_dir, ctx) = project();
        let outcome = ShellTool
            .invoke(json!({"command": "true", "timeout_ms": 99_000_000}), &ctx)
            .await
            .unwrap();
        assert!(!outcome.is_error);
    }

    #[tokio::test]
    async fn an_empty_command_is_rejected() {
        let (_dir, ctx) = project();
        assert!(
            ShellTool
                .invoke(json!({"command": "   "}), &ctx)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn the_tool_declares_spawn_and_write_effects() {
        let effects = ShellTool.effects();
        assert!(effects.spawns_process());
        assert!(effects.mutates());
    }
}
