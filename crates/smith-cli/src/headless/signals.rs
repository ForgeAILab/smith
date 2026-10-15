//! Headless cancellation and signal handling through process-exit cleanup.

use std::cell::Cell;

use agent_runtime_core::cancel::CancelReason;
use anyhow::Result;
use smith_runtime::host::HostSession;
#[cfg(unix)]
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::Notify;

#[derive(Default)]
pub(super) struct StopState {
    interrupted: Cell<bool>,
    changed: Notify,
}

impl StopState {
    pub(super) fn interrupted(&self) -> bool {
        self.interrupted.get()
    }

    pub(super) async fn changed(&self) {
        self.changed.notified().await;
    }
}

pub(super) struct StopSignals {
    #[cfg(unix)]
    interrupt: Signal,
    #[cfg(unix)]
    terminate: Signal,
    #[cfg(unix)]
    hangup: Signal,
}

impl StopSignals {
    pub(super) fn new() -> Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            interrupt: signal(SignalKind::interrupt())?,
            #[cfg(unix)]
            terminate: signal(SignalKind::terminate())?,
            #[cfg(unix)]
            hangup: signal(SignalKind::hangup())?,
        })
    }

    pub(super) async fn watch(&mut self, host: &HostSession, state: &StopState) {
        loop {
            self.next().await;
            if state.interrupted.replace(true) {
                // A second signal must not wait for tool cleanup or storage.
                #[cfg(unix)]
                smith_runtime::mcp::kill_stdio_servers();
                std::process::exit(1);
            }
            // Also used by the TUI's user interrupt. The normal turn driver
            // commits cancellation results before headless shutdown saves them.
            let _ = interrupt(host);
            state.changed.notify_one();
        }
    }

    async fn next(&mut self) {
        #[cfg(unix)]
        tokio::select! {
            _ = self.interrupt.recv() => {},
            _ = self.terminate.recv() => {},
            _ = self.hangup.recv() => {},
        }
        #[cfg(not(unix))]
        std::future::pending::<()>().await;
    }
}

#[cfg(unix)]
pub(crate) async fn shutdown_stdio_servers() {
    // Session persistence is already finished. Keep a final signal able to
    // bypass the MCP grace period, including after the headless watcher ends.
    let cleanup = smith_runtime::mcp::shutdown_stdio_servers();
    if let Ok(mut signals) = StopSignals::new() {
        tokio::select! {
            biased;
            _ = signals.next() => {
                smith_runtime::mcp::kill_stdio_servers();
                std::process::exit(1);
            },
            _ = cleanup => {},
        }
    } else {
        cleanup.await;
    }
}

pub(super) fn interrupt(host: &HostSession) -> Result<()> {
    host.session()
        .interrupt_current_turn(CancelReason::UserRequested)?;
    Ok(())
}
