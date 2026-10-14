//! Headless-only signal handling, kept alive through persistence and shutdown.

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

pub(super) fn interrupt(host: &HostSession) -> Result<()> {
    host.session()
        .interrupt_current_turn(CancelReason::UserRequested)?;
    Ok(())
}
