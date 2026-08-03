//! What the session has actually read, and how recently.
//!
//! Smith's exact-match `replace` needs no read precondition: an `old_string`
//! either matches the bytes on disk or it does not, so a model editing from a
//! stale mental image simply fails. `overwrite` and `delete` have no such
//! self-check — they destroy content that nothing in the call proves the model
//! ever saw. This module supplies the missing proof.
//!
//! Three conditions, all of which `claude-code` enforces through its own
//! `readFileState` and all of which are load-bearing:
//!
//! 1. **Read at all.** Otherwise the model is overwriting a file it inferred
//!    the contents of.
//! 2. **Read in full.** An `offset`/`limit` read shows a window; replacing the
//!    whole file from a window silently drops everything outside it.
//! 3. **Unmodified since.** This is the one that catches the user editing the
//!    file in their own editor between the agent's read and its write. Without
//!    it, "the agent clobbered my changes" is a supported workflow.
//!
//! Staleness is checked by comparing the modification time captured at read
//! against the one on disk now, rather than against a wall clock. A file
//! rewritten within the same clock tick as the read still compares unequal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// One completed read of one canonical path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadObservation {
    /// Whether the read returned the entire file rather than a line window.
    pub full: bool,
    /// The target's modification time when the read completed. `None` when the
    /// platform did not report one, which fails the staleness check closed.
    pub modified: Option<SystemTime>,
}

/// Why a destructive whole-file operation is not authorized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadDefect {
    /// The session never read this path.
    Unread,
    /// The session read only part of it.
    Partial,
    /// The file changed after the session read it.
    Stale,
}

impl ReadDefect {
    /// The exact refusal a model should see.
    ///
    /// The three cases read differently on purpose: each one implies a
    /// different next action — read it, read all of it, or read it again.
    pub fn message(self, display: &str) -> String {
        match self {
            ReadDefect::Unread => format!(
                "`{display}` has not been read in this session; read it before replacing or \
                 deleting it"
            ),
            ReadDefect::Partial => format!(
                "`{display}` was only read in part; read the whole file before replacing or \
                 deleting it"
            ),
            ReadDefect::Stale => format!(
                "`{display}` changed since it was read; read it again before replacing or \
                 deleting it"
            ),
        }
    }
}

/// Session-scoped record of what has been read.
///
/// Tools stay pure functions of their arguments and the workspace; anything
/// that has to remember a previous call lives here, beside
/// [`ChangeRecorder`](crate::change::ChangeRecorder), and is supplied by the
/// observing wrapper.
#[derive(Debug, Default)]
pub struct ReadRecorder {
    reads: Mutex<HashMap<PathBuf, ReadObservation>>,
}

impl ReadRecorder {
    /// Creates an empty recorder.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Records one completed read, replacing any earlier observation.
    ///
    /// A later partial read of an already-fully-read file correctly demotes it:
    /// the most recent read is the one whose staleness we can reason about.
    pub fn record(&self, path: PathBuf, full: bool) {
        let modified = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok();
        if let Ok(mut reads) = self.reads.lock() {
            reads.insert(path, ReadObservation { full, modified });
        }
    }

    /// The most recent observation of one canonical path.
    pub fn observation(&self, path: &Path) -> Option<ReadObservation> {
        self.reads.lock().ok()?.get(path).copied()
    }

    /// Checks whether a destructive whole-file operation is authorized.
    pub fn authorize_destructive(&self, path: &Path) -> Result<(), ReadDefect> {
        let Some(observation) = self.observation(path) else {
            return Err(ReadDefect::Unread);
        };
        if !observation.full {
            return Err(ReadDefect::Partial);
        }
        let current = std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok();
        // Both sides absent means the platform reports no modification time at
        // all; that is a missing check rather than a passed one.
        match (observation.modified, current) {
            (Some(read), Some(now)) if read == now => Ok(()),
            _ => Err(ReadDefect::Stale),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, body: &str) {
        std::fs::write(path, body).expect("write");
    }

    #[test]
    fn an_unread_path_is_refused() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("a.rs");
        touch(&path, "fn a() {}\n");

        let recorder = ReadRecorder::new();
        assert_eq!(
            recorder.authorize_destructive(&path),
            Err(ReadDefect::Unread)
        );
    }

    #[test]
    fn a_partial_read_does_not_authorize() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("a.rs");
        touch(&path, "fn a() {}\n");

        let recorder = ReadRecorder::new();
        recorder.record(path.clone(), false);
        assert_eq!(
            recorder.authorize_destructive(&path),
            Err(ReadDefect::Partial)
        );
    }

    #[test]
    fn a_full_read_authorizes_until_the_file_changes() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("a.rs");
        touch(&path, "fn a() {}\n");

        let recorder = ReadRecorder::new();
        recorder.record(path.clone(), true);
        assert_eq!(recorder.authorize_destructive(&path), Ok(()));

        // An external write, as if the user edited the file themselves.
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| {
                file.set_modified(SystemTime::now() + std::time::Duration::from_secs(1))
            })
            .expect("retouch");
        assert_eq!(
            recorder.authorize_destructive(&path),
            Err(ReadDefect::Stale)
        );
    }

    #[test]
    fn a_later_partial_read_demotes_an_earlier_full_one() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("a.rs");
        touch(&path, "fn a() {}\n");

        let recorder = ReadRecorder::new();
        recorder.record(path.clone(), true);
        recorder.record(path.clone(), false);
        assert_eq!(
            recorder.authorize_destructive(&path),
            Err(ReadDefect::Partial)
        );
    }

    #[test]
    fn each_defect_names_a_different_next_action() {
        let messages = [ReadDefect::Unread, ReadDefect::Partial, ReadDefect::Stale]
            .map(|defect| defect.message("src/a.rs"));
        assert!(messages[0].contains("has not been read"), "{messages:?}");
        assert!(messages[1].contains("only read in part"), "{messages:?}");
        assert!(
            messages[2].contains("changed since it was read"),
            "{messages:?}"
        );
        assert!(messages.iter().all(|message| message.contains("src/a.rs")));
    }
}
