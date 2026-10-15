//! One budget for process exit, including cleanup after the host has stopped.

use std::future::Future;
use std::sync::Arc;
#[cfg(not(test))]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use smith_runtime::background_tasks::TaskStatus;
use smith_runtime::host::HostSession;

const EXIT_DEADLINE: Duration = Duration::from_secs(10);
const MCP_RESERVE: Duration = Duration::from_secs(2);
const DEADLINE_ERROR: &str = "exit deadline reached";

pub(crate) struct ExitClock {
    deadline: Instant,
    reserve: Duration,
    expired: AtomicBool,
}

impl ExitClock {
    fn new(limit: Duration, reserve: Duration) -> Self {
        Self {
            deadline: Instant::now() + limit,
            reserve,
            expired: AtomicBool::new(false),
        }
    }

    fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    fn host_budget_at(&self, now: Instant) -> Duration {
        self.deadline
            .saturating_duration_since(now)
            .saturating_sub(self.reserve)
    }

    async fn shutdown<F, T>(&self, shutdown: F) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        // HostSession caches only completed shutdowns. Once a timeout drops
        // its future, later cleanup must not restart the same shutdown.
        if self.expired.load(Ordering::SeqCst) {
            anyhow::bail!(DEADLINE_ERROR);
        }
        match tokio::time::timeout(self.host_budget_at(Instant::now()), shutdown).await {
            Ok(result) => result,
            Err(_) => {
                self.expired.store(true, Ordering::SeqCst);
                anyhow::bail!(DEADLINE_ERROR)
            }
        }
    }
}

#[cfg(not(test))]
static EXIT_CLOCK: OnceLock<Arc<ExitClock>> = OnceLock::new();

pub(crate) fn start() -> Arc<ExitClock> {
    #[cfg(not(test))]
    {
        EXIT_CLOCK
            .get_or_init(|| {
                let clock = Arc::new(ExitClock::new(EXIT_DEADLINE, MCP_RESERVE));
                start_backstop(clock.deadline);
                clock
            })
            .clone()
    }
    #[cfg(test)]
    {
        // Unit tests run many independent CLI lifetimes inside one process.
        TEST_SHUTDOWN
            .try_with(Arc::clone)
            .unwrap_or_else(|_| Arc::new(ExitClock::new(EXIT_DEADLINE, MCP_RESERVE)))
    }
}

pub(crate) async fn shutdown_host(host: &HostSession) -> Result<()> {
    let clock = start();
    let shutdown = async {
        #[cfg(test)]
        if TEST_SHUTDOWN.try_with(|_| ()).is_ok() {
            std::future::pending::<()>().await;
        }
        host.shutdown()
            .await
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("{error}"))
    };
    let result = clock.shutdown(shutdown).await;
    if clock.expired.load(Ordering::SeqCst) {
        host.background_tasks()
            .stop_all_session_tasks(host.session().id(), TaskStatus::Shutdown);
        // Injected unit-test exits must not kill servers owned by other tests.
        #[cfg(all(unix, not(test)))]
        smith_runtime::mcp::kill_stdio_servers();
    }
    result
}

/// Final MCP cleanup shares the process clock, including errors before a host
/// was created. The thread remains armed through Tokio runtime destruction.
#[cfg(unix)]
pub(crate) async fn finish() -> Result<()> {
    let clock = start();
    if tokio::time::timeout(clock.remaining(), crate::headless::shutdown_stdio_servers())
        .await
        .is_err()
    {
        smith_runtime::mcp::kill_stdio_servers();
        anyhow::bail!(DEADLINE_ERROR);
    }
    Ok(())
}

fn start_backstop(deadline: Instant) {
    std::thread::spawn(move || {
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        crate::terminal::restore_best_effort();
        #[cfg(unix)]
        smith_runtime::mcp::kill_stdio_servers();
        // Do not take stdio's global locks: a blocked runtime may hold them.
        #[cfg(unix)]
        use std::io::Write;
        #[cfg(unix)]
        use std::os::fd::AsFd;
        #[cfg(unix)]
        if let Ok(stderr) = std::io::stderr().as_fd().try_clone_to_owned() {
            let mut stderr = std::fs::File::from(stderr);
            let _ = writeln!(
                stderr,
                "smith: exit deadline reached (10s); stopped remaining work"
            );
        }
        #[cfg(not(unix))]
        eprintln!("smith: exit deadline reached (10s); stopped remaining work");
        std::process::exit(1);
    });
}

#[cfg(test)]
tokio::task_local! {
    static TEST_SHUTDOWN: Arc<ExitClock>;
}

#[cfg(test)]
pub(crate) async fn with_hung_shutdown<T>(limit: Duration, future: impl Future<Output = T>) -> T {
    TEST_SHUTDOWN
        .scope(Arc::new(ExitClock::new(limit, Duration::ZERO)), future)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exit_deadline_bounds_shutdown_and_cannot_restart_it() {
        let clock = ExitClock::new(Duration::from_millis(80), Duration::from_millis(60));
        let error = clock
            .shutdown(std::future::pending::<Result<()>>())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), DEADLINE_ERROR);
        assert_eq!(
            clock.host_budget_at(clock.deadline - Duration::from_millis(80)),
            Duration::from_millis(20),
            "MCP cleanup retains its reserve"
        );
        let polled = AtomicBool::new(false);
        assert!(
            clock
                .shutdown(async {
                    polled.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .await
                .is_err()
        );
        assert!(
            !polled.load(Ordering::SeqCst),
            "timed out shutdown cannot restart"
        );
    }

    #[tokio::test]
    async fn exit_deadline_is_shared_across_shutdown_steps() {
        let clock = ExitClock::new(Duration::from_millis(80), Duration::from_millis(20));
        let deadline = clock.deadline;
        clock.shutdown(async { Ok(()) }).await.unwrap();
        assert_eq!(clock.deadline, deadline);
        assert_eq!(
            clock.host_budget_at(deadline - Duration::from_millis(50)),
            Duration::from_millis(30)
        );
        assert_eq!(
            clock.host_budget_at(deadline - Duration::from_millis(10)),
            Duration::ZERO
        );
        assert!(
            clock
                .shutdown(std::future::pending::<Result<()>>())
                .await
                .is_err()
        );
    }

    #[test]
    #[ignore = "re-executed by exit_deadline_backstop_exits_a_blocked_process"]
    fn exit_deadline_backstop_child() {
        let stdout = std::io::stdout();
        let stderr = std::io::stderr();
        let _stdout = stdout.lock();
        let _stderr = stderr.lock();
        start_backstop(Instant::now() + Duration::from_millis(30));
        loop {
            std::thread::park();
        }
    }

    #[test]
    fn exit_deadline_backstop_exits_a_blocked_process() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "exit_deadline::tests::exit_deadline_backstop_child",
            ])
            .output()
            .expect("backstop child");
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("smith: exit deadline reached (10s); stopped remaining work"),
            "{stderr}"
        );
    }
}
