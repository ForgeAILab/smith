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
//!
//! The session id is only known once the host has started, and starting it
//! is when MCP servers are spawned and most likely to fail. [`start`]
//! therefore installs the subscriber first, holding events in a bounded
//! buffer that [`init`] writes ahead of everything else in the session's log.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use agent_runtime_core::ids::SessionId;
use tracing_subscriber::EnvFilter;

/// Selects the log level. Unset defaults to `warn`, the level the
/// observability gaps this module exists for are already emitted at.
const LEVEL_ENV_VAR: &str = "SMITH_LOG";

/// The most a process holds before its session's log file exists.
const PENDING_BYTES: usize = 1024 * 1024;

/// Guards against naming more than one session's log file.
///
/// `tracing`'s global dispatcher can be set at most once, but a single Smith
/// process can start more than one session — the interactive loop restarts
/// the host on a profile switch or a reconnect. Only the first session names
/// the log file; retargeting an already-installed subscriber mid-process
/// would be more confusing than sticking with one file. Claimed
/// unconditionally on the first call, even if that call's own log file fails
/// to open, rather than retried: a process that starts several sessions is
/// exactly the case where silently switching a working log to a different
/// file mid-run would be more confusing than sticking with "no log this run."
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Where formatted events go: held until the log file exists, then the file.
static SINK: OnceLock<Arc<Mutex<Sink>>> = OnceLock::new();

enum Sink {
    Pending { buffer: Vec<u8>, dropped: usize },
    File(std::fs::File),
    Closed,
}

impl Sink {
    fn new() -> Self {
        Self::Pending {
            buffer: Vec::new(),
            dropped: 0,
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        match self {
            Self::Pending { buffer, dropped } => {
                if buffer.len() + bytes.len() <= PENDING_BYTES {
                    buffer.extend_from_slice(bytes);
                } else {
                    *dropped += bytes.len();
                }
            }
            // Write failures are swallowed for the same reason open failures
            // are: there is nowhere left to report them.
            Self::File(file) => {
                let _ = file.write_all(bytes);
            }
            Self::Closed => {}
        }
    }

    /// Moves to `file`, writing what was held first.
    fn attach(&mut self, file: Option<std::fs::File>) {
        let previous = std::mem::replace(self, Self::Closed);
        let Some(mut file) = file else {
            return;
        };
        if let Self::Pending { buffer, dropped } = previous {
            let _ = file.write_all(&buffer);
            if dropped > 0 {
                let _ = writeln!(
                    file,
                    "smith: {dropped} bytes of log output from before this session were dropped"
                );
            }
        }
        *self = Self::File(file);
    }
}

struct SinkWriter(Arc<Mutex<Sink>>);

impl Write for SinkWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut sink) = self.0.lock() {
            sink.write(bytes);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Installs the process-wide subscriber, holding events until [`init`].
///
/// Idempotent. Installation failure — a subscriber already installed — is
/// swallowed: it must degrade the same way an unopenable log file does,
/// silently, never a panic and never a stray write of its own.
pub(crate) fn start() {
    sink();
}

fn sink() -> Arc<Mutex<Sink>> {
    Arc::clone(SINK.get_or_init(|| {
        let sink = Arc::new(Mutex::new(Sink::new()));
        let filter =
            EnvFilter::try_from_env(LEVEL_ENV_VAR).unwrap_or_else(|_| EnvFilter::new("warn"));
        let writer = Arc::clone(&sink);
        // The writer replaces the builder's default stdout destination
        // outright, and `with_ansi(false)` keeps color escapes that would
        // only confuse a `tail`/`less` reader out of the file.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(move || SinkWriter(Arc::clone(&writer)))
            .with_ansi(false)
            .try_init();
        sink
    }))
}

/// Names `session`'s log file as the destination, if this is the first call
/// in the process, and writes what was held since [`start`] into it.
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
    let sink = sink();
    let file = smith_runtime::session::open_session_log_under_home(session)
        .await
        .ok();
    if let Ok(mut sink) = sink.lock() {
        sink.attach(file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_events_are_written_first_and_the_hold_is_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.log");
        let mut sink = Sink::new();
        sink.write(b"before the session\n");
        sink.write(&vec![b'x'; PENDING_BYTES]);
        sink.attach(Some(std::fs::File::create(&path).unwrap()));
        sink.write(b"after the session\n");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("before the session\n"), "{text}");
        assert!(
            text.contains(&format!(
                "smith: {PENDING_BYTES} bytes of log output from before this session were dropped"
            )),
            "{text}"
        );
        assert!(text.ends_with("after the session\n"), "{text}");
    }

    #[test]
    fn without_a_file_nothing_more_is_held() {
        let mut sink = Sink::new();
        sink.attach(None);
        sink.write(b"discarded");
        assert!(matches!(sink, Sink::Closed));
    }
}
