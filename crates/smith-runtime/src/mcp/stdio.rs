//! Smith owns child lifetimes independently of connections retained by tools.
//!
//! Servers share Smith's process group so group-directed signals (including
//! SIGKILL, Ctrl-C, and terminal hangup) still reach them. Smith signals each
//! watchdog by PID during exit cleanup. The watchdog also stops a busy server
//! when Smith disappears without running cleanup.

use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, Once, OnceLock, Weak};
use std::time::Duration;

use agent_runtime_mcp::{McpServerConfig, McpTransport};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use rmcp::RoleClient;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::{Transport, async_rw::AsyncRwTransport};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const CLOSE_GRACE: Duration = Duration::from_secs(3);
const EXIT_GRACE: Duration = Duration::from_millis(500);
const REAP_BOUND: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[cfg(test)]
static DIRECT_SPAWN_WARNINGS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(Default)]
struct Processes {
    exiting: bool,
    children: Vec<Weak<ServerProcess>>,
}

fn processes() -> &'static Mutex<Processes> {
    static PROCESSES: OnceLock<Mutex<Processes>> = OnceLock::new();
    PROCESSES.get_or_init(Mutex::default)
}

struct ServerProcess {
    child: Mutex<Option<Child>>,
    supervised: bool,
}

impl ServerProcess {
    fn signal(&self, signal: Signal) {
        let child = self.child.lock().expect("MCP child");
        if let Some(pid) = child.as_ref().and_then(Child::id) {
            // Killing the watchdog itself would strand its server. Its TERM
            // handler kills and reaps the server even when Smith exits at once.
            let signal = if self.supervised && signal == Signal::SIGKILL {
                Signal::SIGTERM
            } else {
                signal
            };
            let _ = kill(Pid::from_raw(pid as i32), signal);
        }
    }

    fn reap(&self) -> bool {
        let mut child = self.child.lock().expect("MCP child");
        let Some(running) = child.as_mut() else {
            return true;
        };
        match running.try_wait() {
            Ok(Some(_)) => {
                child.take();
                true
            }
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(%error, "failed to reap MCP server");
                false
            }
        }
    }

    async fn kill_and_wait(&self) {
        self.signal(Signal::SIGKILL);
        let deadline = tokio::time::Instant::now() + REAP_BOUND;
        while !self.reap() {
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!("MCP server did not exit after SIGKILL within the reap bound");
                break;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        // Initialization can be cancelled before rmcp has a running service.
        // Synchronous child cleanup must also work during executor teardown.
        self.signal(Signal::SIGKILL);
    }
}

pub(super) struct StdioTransport {
    io: AsyncRwTransport<RoleClient, ChildStdout, ChildStdin>,
    process: Arc<ServerProcess>,
}

pub(super) fn spawn(
    config: &McpServerConfig,
    smith_executable: Option<&Path>,
) -> io::Result<StdioTransport> {
    let McpTransport::Stdio {
        command,
        args,
        env,
        cwd,
    } = &config.transport
    else {
        unreachable!("stdio declaration checked by connector");
    };
    let invocation = |smith_executable: Option<&Path>| {
        let mut invocation = if let Some(smith) = smith_executable {
            let mut invocation = Command::new(smith);
            invocation
                .arg("mcp-watchdog")
                .arg("--parent")
                .arg(std::process::id().to_string())
                .arg("--")
                .arg(command);
            invocation
        } else {
            Command::new(command)
        };
        invocation
            .args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(smith_executable.is_none());
        if let Some(cwd) = cwd {
            invocation.current_dir(cwd);
        }
        invocation
    };

    // Serialize admission with exit cleanup so a late connector cannot spawn
    // a child after the exit snapshot has been taken.
    let mut processes = processes().lock().expect("MCP processes");
    if processes.exiting {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Smith is exiting",
        ));
    }
    let (mut child, supervised) = if let Some(smith) = smith_executable {
        match invocation(Some(smith)).spawn() {
            Ok(child) => (child, true),
            Err(_) => {
                warn_direct_spawn();
                (invocation(None).spawn()?, false)
            }
        }
    } else {
        warn_direct_spawn();
        (invocation(None).spawn()?, false)
    };
    let stdout = child.stdout.take().expect("piped MCP stdout");
    let stdin = child.stdin.take().expect("piped MCP stdin");
    let process = Arc::new(ServerProcess {
        child: Mutex::new(Some(child)),
        supervised,
    });
    processes.children.retain(|child| child.strong_count() > 0);
    processes.children.push(Arc::downgrade(&process));
    Ok(StdioTransport {
        io: AsyncRwTransport::new(stdout, stdin),
        process,
    })
}

fn warn_direct_spawn() {
    static WARNING: Once = Once::new();
    WARNING.call_once(|| {
        #[cfg(test)]
        DIRECT_SPAWN_WARNINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(
            "Smith executable unavailable; starting stdio MCP servers directly; servers may outlive Smith"
        );
    });
}

impl Transport<RoleClient> for StdioTransport {
    type Error = io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        self.io.send(item)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.io.receive().await
    }

    async fn close(&mut self) -> io::Result<()> {
        let result = self.io.close().await;
        let deadline = tokio::time::Instant::now() + CLOSE_GRACE;
        while !self.process.reap() {
            if tokio::time::Instant::now() >= deadline {
                self.process.kill_and_wait().await;
                break;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        result
    }
}

fn exiting_processes() -> Vec<Arc<ServerProcess>> {
    let mut processes = processes().lock().expect("MCP processes");
    processes.exiting = true;
    processes
        .children
        .iter()
        .filter_map(Weak::upgrade)
        .collect()
}

/// Terminates and reaps every Smith-started stdio MCP server.
///
/// Only call at process exit, after shutting down runtime sessions. Retiring
/// one supervisor must leave connections held by another runtime usable.
pub async fn shutdown_stdio_servers() {
    shutdown_processes(exiting_processes()).await;
}

async fn shutdown_processes(mut processes: Vec<Arc<ServerProcess>>) {
    for process in &processes {
        process.signal(Signal::SIGTERM);
    }
    let deadline = tokio::time::Instant::now() + EXIT_GRACE;
    loop {
        processes.retain(|process| !process.reap());
        if processes.is_empty() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    futures_util::future::join_all(processes.iter().map(|process| process.kill_and_wait())).await;
}

/// Kills all Smith-started stdio MCP servers before an immediate process exit.
pub fn kill_stdio_servers() {
    for process in exiting_processes() {
        process.signal(Signal::SIGKILL);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use nix::errno::Errno;
    use nix::sys::wait::{WaitPidFlag, waitpid};
    use std::os::unix::fs::PermissionsExt;

    fn spawn_shell(script: &str) -> StdioTransport {
        spawn(
            &McpServerConfig::new(
                "stdio-test",
                McpTransport::Stdio {
                    command: "sh".into(),
                    args: vec!["-c".into(), script.into()],
                    env: Default::default(),
                    cwd: None,
                },
            ),
            None,
        )
        .expect("spawn stdio server")
    }

    fn child_pid(process: &ServerProcess) -> Pid {
        let child = process.child.lock().expect("MCP child");
        Pid::from_raw(child.as_ref().and_then(Child::id).expect("running child") as i32)
    }

    fn assert_reaped(process: &ServerProcess, pid: Pid) {
        assert!(process.child.lock().expect("MCP child").is_none());
        assert_eq!(waitpid(pid, Some(WaitPidFlag::WNOHANG)), Err(Errno::ECHILD));
    }

    #[tokio::test]
    async fn close_reaps_server_that_exits_on_stdin_eof() {
        let mut transport = spawn_shell("read _");
        let pid = child_pid(&transport.process);

        tokio::time::timeout(Duration::from_secs(5), transport.close())
            .await
            .expect("EOF shutdown should return promptly")
            .expect("close stdio transport");

        assert_reaped(&transport.process, pid);
    }

    #[tokio::test]
    async fn shutdown_kills_and_reaps_server_ignoring_sigterm() {
        let mut transport = spawn_shell(
            r#"trap "" TERM; printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}'; sleep 30"#,
        );
        let pid = child_pid(&transport.process);
        // The notification proves the shell installed its trap before SIGTERM.
        tokio::time::timeout(Duration::from_secs(5), transport.receive())
            .await
            .expect("server readiness timeout")
            .expect("server readiness notification");

        // Exercise the shutdown core without permanently closing admission in
        // the process-wide registry used by other tests in this binary.
        tokio::time::timeout(
            Duration::from_secs(5),
            shutdown_processes(vec![Arc::clone(&transport.process)]),
        )
        .await
        .expect("SIGTERM-ignoring server should be killed within the bounds");

        assert_reaped(&transport.process, pid);
    }

    #[tokio::test]
    async fn missing_smith_path_spawns_directly_and_warns_once() {
        for _ in 0..2 {
            let mut transport = spawn_shell("read _");
            assert!(!transport.process.supervised);
            let pid = child_pid(&transport.process);
            tokio::time::timeout(Duration::from_secs(5), transport.close())
                .await
                .unwrap()
                .unwrap();
            assert_reaped(&transport.process, pid);
        }
        assert_eq!(
            DIRECT_SPAWN_WARNINGS.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn watchdog_receives_invocation_environment_directory_and_stdio() {
        let directory = tempfile::tempdir().unwrap();
        let smith = directory.path().join("smith");
        std::fs::write(
            &smith,
            format!(
                r#"#!/bin/sh
test "$1" = mcp-watchdog || exit 91
test "$2" = --parent || exit 92
test "$3" = {} || exit 93
test "$4" = -- || exit 94
shift 4
exec "$@"
"#,
                std::process::id()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&smith, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = McpServerConfig::new(
            "watchdog-test",
            McpTransport::Stdio {
                command: "sh".into(),
                args: vec![
                    "-c".into(),
                    r#"test "$MCP_WATCHDOG_TEST" = inherited || exit 95
test -f smith || exit 96
printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}'
read _
"#
                    .into(),
                ],
                env: [("MCP_WATCHDOG_TEST".into(), "inherited".into())].into(),
                cwd: Some(directory.path().to_str().unwrap().into()),
            },
        );
        let mut transport = spawn(&config, Some(&smith)).unwrap();
        assert!(transport.process.supervised);
        let pid = child_pid(&transport.process);
        tokio::time::timeout(Duration::from_secs(5), transport.receive())
            .await
            .unwrap()
            .expect("watchdog forwarded stdout");
        tokio::time::timeout(Duration::from_secs(5), transport.close())
            .await
            .unwrap()
            .unwrap();
        assert_reaped(&transport.process, pid);

        let mut transport = spawn(&config, Some(&directory.path().join("missing-smith"))).unwrap();
        assert!(!transport.process.supervised);
        let pid = child_pid(&transport.process);
        tokio::time::timeout(Duration::from_secs(5), transport.receive())
            .await
            .unwrap()
            .expect("fallback server stdout");
        tokio::time::timeout(Duration::from_secs(5), transport.close())
            .await
            .unwrap()
            .unwrap();
        assert_reaped(&transport.process, pid);
    }

    #[tokio::test]
    async fn forced_cleanup_and_drop_let_watchdog_run_its_stop_handler() {
        let directory = tempfile::tempdir().unwrap();
        let smith = directory.path().join("smith");
        std::fs::write(
            &smith,
            r#"#!/bin/sh
trap 'echo stopped > stopped; exit 0' TERM
printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}'
while :; do :; done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&smith, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = McpServerConfig::new(
            "watchdog-stop-test",
            McpTransport::Stdio {
                command: "unused-server".into(),
                args: Vec::new(),
                env: Default::default(),
                cwd: Some(directory.path().to_str().unwrap().into()),
            },
        );
        for drop_transport in [false, true] {
            let marker = directory.path().join("stopped");
            let _ = std::fs::remove_file(&marker);
            let mut transport = spawn(&config, Some(&smith)).unwrap();
            let pid = child_pid(&transport.process);
            tokio::time::timeout(Duration::from_secs(5), transport.receive())
                .await
                .unwrap()
                .expect("watchdog installed its TERM handler");
            if drop_transport {
                drop(transport);
            } else {
                // This is also the close-timeout and exit-escalation core.
                transport.process.kill_and_wait().await;
                assert_reaped(&transport.process, pid);
            }
            tokio::time::timeout(Duration::from_secs(5), async {
                while !marker.exists() || kill(pid, None) != Err(Errno::ESRCH) {
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            })
            .await
            .expect("watchdog stop handler must run and its process must be reaped");
        }
    }
}
