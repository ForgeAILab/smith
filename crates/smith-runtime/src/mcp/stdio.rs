//! Smith owns child lifetimes independently of connections retained by tools.
//!
//! Servers share Smith's process group so group-directed signals (including
//! SIGKILL, Ctrl-C, and terminal hangup) still reach them. Smith signals each
//! server by PID during exit cleanup. If only Smith is killed by SIGKILL,
//! servers must exit on stdin EOF; no watchdog enforces their lifetime.

use std::io;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock, Weak};
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
}

impl ServerProcess {
    fn signal(&self, signal: Signal) {
        let child = self.child.lock().expect("MCP child");
        if let Some(pid) = child.as_ref().and_then(Child::id) {
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

pub(super) fn spawn(config: &McpServerConfig) -> io::Result<StdioTransport> {
    let McpTransport::Stdio {
        command,
        args,
        env,
        cwd,
    } = &config.transport
    else {
        unreachable!("stdio declaration checked by connector");
    };
    let mut command = Command::new(command);
    command
        .args(args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }

    // Serialize admission with exit cleanup so a late connector cannot spawn
    // a child after the exit snapshot has been taken.
    let mut processes = processes().lock().expect("MCP processes");
    if processes.exiting {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Smith is exiting",
        ));
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("piped MCP stdout");
    let stdin = child.stdin.take().expect("piped MCP stdin");
    let process = Arc::new(ServerProcess {
        child: Mutex::new(Some(child)),
    });
    processes.children.retain(|child| child.strong_count() > 0);
    processes.children.push(Arc::downgrade(&process));
    Ok(StdioTransport {
        io: AsyncRwTransport::new(stdout, stdin),
        process,
    })
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

    fn spawn_shell(script: &str) -> StdioTransport {
        spawn(&McpServerConfig::new(
            "stdio-test",
            McpTransport::Stdio {
                command: "sh".into(),
                args: vec!["-c".into(), script.into()],
                env: Default::default(),
                cwd: None,
            },
        ))
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

        tokio::time::timeout(Duration::from_secs(2), transport.close())
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
            Duration::from_secs(2),
            shutdown_processes(vec![Arc::clone(&transport.process)]),
        )
        .await
        .expect("SIGTERM-ignoring server should be killed within the bounds");

        assert_reaped(&transport.process, pid);
    }
}
