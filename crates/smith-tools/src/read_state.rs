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
//! 3. **Same version at commit.** The read's handle-bound identity, length,
//!    timestamp, and content hash are carried in the prepared call and checked
//!    at the atomic replace/delete commit point.

use smith_host::workspace::FileVersion;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// One completed read of one canonical path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadObservation {
    /// Whether the read returned the entire file rather than a line window.
    pub full: bool,
    /// Exact identity and content returned by the completed handle read.
    pub version: FileVersion,
}

/// Why a destructive whole-file operation is not authorized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadDefect {
    /// The session never read this path.
    Unread,
    /// The session read only part of it.
    Partial,
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
    pub fn record(&self, path: PathBuf, full: bool, version: FileVersion) {
        if let Ok(mut reads) = self.reads.lock() {
            reads.insert(path, ReadObservation { full, version });
        }
    }

    /// The most recent observation of one canonical path.
    pub fn observation(&self, path: &Path) -> Option<ReadObservation> {
        self.reads.lock().ok()?.get(path).cloned()
    }

    /// Checks whether a destructive whole-file operation is authorized.
    pub fn expected_version(&self, path: &Path) -> Result<FileVersion, ReadDefect> {
        let Some(observation) = self.observation(path) else {
            return Err(ReadDefect::Unread);
        };
        if !observation.full {
            return Err(ReadDefect::Partial);
        }
        Ok(observation.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, body: &str) {
        std::fs::write(path, body).expect("write");
    }

    fn version(path: &Path) -> FileVersion {
        let workspace = smith_host::workspace::ProjectWorkspace::new(path.parent().unwrap())
            .expect("workspace");
        workspace
            .read_bounded(path.file_name().unwrap(), 1024)
            .expect("read")
            .version
    }

    #[test]
    fn an_unread_path_is_refused() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("a.rs");
        touch(&path, "fn a() {}\n");

        let recorder = ReadRecorder::new();
        assert_eq!(recorder.expected_version(&path), Err(ReadDefect::Unread));
    }

    #[test]
    fn a_partial_read_does_not_authorize() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("a.rs");
        touch(&path, "fn a() {}\n");

        let recorder = ReadRecorder::new();
        recorder.record(path.clone(), false, version(&path));
        assert_eq!(recorder.expected_version(&path), Err(ReadDefect::Partial));
    }

    #[test]
    fn a_full_read_returns_the_version_to_check_at_commit() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("a.rs");
        touch(&path, "fn a() {}\n");

        let recorder = ReadRecorder::new();
        let expected = version(&path);
        recorder.record(path.clone(), true, expected.clone());
        assert_eq!(recorder.expected_version(&path), Ok(expected));
    }

    #[test]
    fn a_later_partial_read_demotes_an_earlier_full_one() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("a.rs");
        touch(&path, "fn a() {}\n");

        let recorder = ReadRecorder::new();
        recorder.record(path.clone(), true, version(&path));
        recorder.record(path.clone(), false, version(&path));
        assert_eq!(recorder.expected_version(&path), Err(ReadDefect::Partial));
    }

    #[test]
    fn each_defect_names_a_different_next_action() {
        let messages =
            [ReadDefect::Unread, ReadDefect::Partial].map(|defect| defect.message("src/a.rs"));
        assert!(messages[0].contains("has not been read"), "{messages:?}");
        assert!(messages[1].contains("only read in part"), "{messages:?}");
        assert!(messages.iter().all(|message| message.contains("src/a.rs")));
    }
}
