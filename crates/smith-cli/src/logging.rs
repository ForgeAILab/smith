//! Session-scoped file logging.
//!
//! Smith is a full-screen terminal client: nothing may reach stdout or
//! stderr once the alternate screen is up, or a stray line corrupts the
//! display. Until now that constraint was observed by accident rather than
//! by design — `smith-runtime` calls `tracing::warn!`/`tracing::error!` at
//! exactly the moments that matter (the journal's "record was not written",
//! the host's "the journal contains an explicit gap" notice), but with no
//! subscriber installed anywhere in the process, every one of those calls
//! was silently discarded. This module gives them a destination that cannot
//! corrupt the display instead: an owner-only file under
//! `~/.smith/logs/<session-id>.log`.

use std::sync::atomic::{AtomicBool, Ordering};

use agent_runtime_core::ids::SessionId;
use tracing_subscriber::EnvFilter;

/// Selects the log level. Unset defaults to `warn`, the level the
/// observability gaps this module exists for are already emitted at.
const LEVEL_ENV_VAR: &str = "SMITH_LOG";

/// Guards against installing more than one process-wide subscriber.
///
/// `tracing`'s global dispatcher can be set at most once, but a single Smith
/// process can start more than one session — the interactive loop restarts
/// the host on a profile switch or a reconnect. Only the first session names
/// the log file; retargeting an already-installed subscriber mid-process
/// would need a `tracing_subscriber::reload::Handle`, which the bug class
/// this module fixes does not need. Claimed unconditionally on the first
/// call, even if that call's own log file fails to open, rather than
/// retried: a process that starts several sessions is exactly the case where
/// silently switching a working log to a different file mid-run would be
/// more confusing than sticking with "no log this run."
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Installs the file-backed subscriber for `session`, if this is the first
/// call in the process.
///
/// Failure is swallowed rather than surfaced: a diagnostic log is not
/// load-bearing, and the one guarantee this function must never break is
/// that Smith's full-screen surface still starts when the log file cannot be
/// opened. Printing the failure would itself be the stray stdout/stderr
/// write this module exists to prevent, so there is nowhere left to report
/// it.
pub(crate) async fn init(session: &SessionId) {
    if INITIALIZED.swap(true, Ordering::AcqRel) {
        return;
    }
    let Ok(file) = smith_runtime::session::open_session_log_under_home(session).await else {
        return;
    };
    let filter = EnvFilter::try_from_env(LEVEL_ENV_VAR).unwrap_or_else(|_| EnvFilter::new("warn"));
    // `with_writer(file)` replaces the builder's default stdout destination
    // outright, and `with_ansi(false)` keeps color escapes that would only
    // confuse a `tail`/`less` reader out of the file. `try_init` — not
    // `init` — because the only other failure mode here, a subscriber
    // already installed, must degrade the same way an unopenable log file
    // does: silently, never a panic and never a stray write of its own.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(file)
        .with_ansi(false)
        .try_init();
}
