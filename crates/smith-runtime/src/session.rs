//! Where a Smith session lives on disk.
//!
//! Agent Runtime owns the canonical [`SessionSnapshot`]; Smith owns three
//! questions a neutral runtime cannot answer: *where* a snapshot is kept,
//! *how* a half-written one is prevented from ever being observed, and *which
//! component state may enter this ordinary JSON store*. This module answers
//! those host-policy questions without introducing a second message
//! vocabulary or rewriting canonical conversation content.
//!
//! Four decisions are worth stating, because each one is a failure mode that
//! only shows up after a crash or a version skew:
//!
//! - **The root is injectable.** `~/.smith/sessions/<project-id>/` is the
//!   default shape, not a hard-coded constant. A test that writes into a real
//!   home directory is a test that corrupts the developer running it.
//! - **Writes are atomic.** A snapshot is written to a sibling temporary file,
//!   fsynced, and renamed over the target. Within a filesystem a rename is
//!   atomic, so a concurrent reader sees the previous snapshot or the new one
//!   and never a truncated one. Writing in place would make every crash during
//!   `save` a corrupted session.
//! - **The on-disk version is explicit and checked first.** The stored record
//!   wraps the host-prepared snapshot in a [`SNAPSHOT_SCHEMA_VERSION`]
//!   envelope, and load reads that version *before* attempting the full parse.
//!   The standard host removes registered credential literals before calling
//!   this store. A future record then fails with "this build cannot read
//!   version N" instead of silently deserializing into a snapshot missing
//!   whatever that version added.
//! - **Component state is classified.** Ordinary JSON snapshots retain only
//!   records explicitly marked `RedactionSafe`. Sensitive namespaces are
//!   omitted both while saving and while loading a legacy/tampered record.
//!   Exact sensitive state belongs in Smith's authenticated checkpoint store,
//!   never this plaintext persistence layer.
//!
//! Resume wiring and rebuilding a snapshot from a crashed journal are
//! deliberately not here; they are separate tasks that build on this contract.

use std::path::{Path, PathBuf};

use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::content::Role;
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::metadata::Metadata;
use agent_runtime_core::store::{SessionSnapshot, SessionStateSensitivity, SessionStore};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::private_storage::{ensure_private_directory, read_private, write_private_atomically};

/// The on-disk schema version of a persisted snapshot record.
///
/// Bumped whenever the stored envelope changes shape in a way an older build
/// cannot read. It is deliberately independent of the runtime's own snapshot
/// type: Smith controls the file format, Agent Runtime controls the payload.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Schema version of bounded metadata used by session pickers.
pub const LISTING_SCHEMA_VERSION: u32 = 1;

/// The file extension of a persisted snapshot, and the marker [`FileSessionStore::list`]
/// enumerates by.
const SNAPSHOT_SUFFIX: &str = ".snapshot.json";

/// The protected latest-turn checkpoint for one session.
const CHECKPOINT_SUFFIX: &str = ".checkpoint.bin";
const CHECKPOINT_LOCK_SUFFIX: &str = ".checkpoint.lock";
const SESSION_LIFECYCLE_LOCK_SUFFIX: &str = ".session.lock";

/// The directory the default root lives under, relative to the user's home.
const DEFAULT_STATE_DIR: &str = ".smith";

/// A project's identifier, used as one path component of the session root.
///
/// A newtype rather than a bare `String` because this value becomes part of a
/// filesystem path: `..` or a separator inside it would place sessions
/// somewhere other than the session root. Deriving the identifier from a
/// project path belongs to configuration resolution, not here; this type only
/// guarantees that whatever was derived is safe to join.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectId(String);

impl ProjectId {
    /// Validates `value` as a single, safe path component.
    pub fn new(value: impl Into<String>) -> Result<Self, RuntimeError> {
        let value = value.into();
        safe_component(&value, "project id")?;
        Ok(Self(value))
    }

    /// The identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Rejects any value that would not stay inside the directory it is joined to.
///
/// Empty names, `.`, `..`, path separators, and control characters are all
/// refused. Sanitizing instead of refusing would silently map two distinct
/// sessions onto one file.
fn safe_component(value: &str, what: &str) -> Result<(), RuntimeError> {
    let bad = value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control);
    if bad {
        return Err(RuntimeError::new(
            ErrorKind::Config,
            format!("{what} `{value}` is not a usable file name"),
        ));
    }
    Ok(())
}

/// The directory one project's sessions are stored in, and the file names
/// within it.
///
/// Both the snapshot store and the event journal resolve their paths through
/// this type so the two can never disagree about where a session lives.
#[derive(Debug, Clone)]
pub struct SessionPaths {
    directory: PathBuf,
    project: ProjectId,
}

impl SessionPaths {
    /// Roots a project's sessions at `<state_root>/sessions/<project>`.
    ///
    /// `state_root` is the injection point: production passes the user's
    /// `~/.smith`, tests pass a temporary directory.
    pub fn new(state_root: impl AsRef<Path>, project: &ProjectId) -> Self {
        Self::from_sessions_dir(state_root.as_ref().join("sessions"), project)
    }

    /// Roots a project's sessions directly below `sessions_dir`.
    ///
    /// Resolved Smith configuration already names `~/.smith/sessions`, while
    /// [`SessionPaths::new`] accepts the broader state root. Keeping both
    /// constructors explicit prevents a host from accidentally producing
    /// `~/.smith/sessions/sessions/<project>`.
    pub fn from_sessions_dir(sessions_dir: impl AsRef<Path>, project: &ProjectId) -> Self {
        Self {
            directory: sessions_dir.as_ref().join(project.as_str()),
            project: project.clone(),
        }
    }

    /// Roots a project's sessions under the user's `~/.smith`.
    ///
    /// Fails when the home directory cannot be determined, rather than falling
    /// back to the working directory: writing session state into whatever
    /// directory the user happened to launch from is worse than not starting.
    pub fn under_home(project: &ProjectId) -> Result<Self, RuntimeError> {
        let home = dirs::home_dir().ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::Config,
                "no home directory is available for Smith's session state",
            )
        })?;
        Ok(Self::new(home.join(DEFAULT_STATE_DIR), project))
    }

    /// The directory sessions for this project are stored in.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Stable project identity bound into protected records.
    pub fn project(&self) -> &ProjectId {
        &self.project
    }

    /// The snapshot file for `session`.
    pub fn snapshot(&self, session: &SessionId) -> Result<PathBuf, RuntimeError> {
        safe_component(session.as_str(), "session id")?;
        Ok(self
            .directory
            .join(format!("{}{SNAPSHOT_SUFFIX}", session.as_str())))
    }

    /// The protected latest-turn checkpoint for `session`.
    pub fn checkpoint(&self, session: &SessionId) -> Result<PathBuf, RuntimeError> {
        safe_component(session.as_str(), "session id")?;
        Ok(self
            .directory
            .join(format!("{}{CHECKPOINT_SUFFIX}", session.as_str())))
    }

    /// Cross-process writer lease for `session`'s protected checkpoint.
    pub(crate) fn checkpoint_lock(&self, session: &SessionId) -> Result<PathBuf, RuntimeError> {
        safe_component(session.as_str(), "session id")?;
        Ok(self
            .directory
            .join(format!("{}{CHECKPOINT_LOCK_SUFFIX}", session.as_str())))
    }

    /// Cross-process lifecycle lease held while one host owns `session`.
    pub(crate) fn lifecycle_lock(&self, session: &SessionId) -> Result<PathBuf, RuntimeError> {
        safe_component(session.as_str(), "session id")?;
        Ok(self.directory.join(format!(
            "{}{SESSION_LIFECYCLE_LOCK_SUFFIX}",
            session.as_str()
        )))
    }

    /// The event-journal file for `session`.
    pub fn journal(&self, session: &SessionId) -> Result<PathBuf, RuntimeError> {
        safe_component(session.as_str(), "session id")?;
        Ok(self.directory.join(format!("{}.jsonl", session.as_str())))
    }

    /// The bounded per-task output spool directory for `session`.
    ///
    /// Background task output is not conversation content and is never
    /// replayed into canonical history, so it lives in its own subdirectory
    /// rather than beside the snapshot and journal files. The directory is
    /// created lazily by the background-task registry on first spawn, not
    /// here.
    pub fn tasks_dir(&self, session: &SessionId) -> Result<PathBuf, RuntimeError> {
        safe_component(session.as_str(), "session id")?;
        Ok(self.directory.join(format!("{}.tasks", session.as_str())))
    }

    /// The metadata-only change attribution journal for `session`.
    pub fn changes(&self, session: &SessionId) -> Result<PathBuf, RuntimeError> {
        safe_component(session.as_str(), "session id")?;
        Ok(self
            .directory
            .join(format!("{}.changes.jsonl", session.as_str())))
    }

    /// Owner-only store for session-attributed artifacts in this project.
    ///
    /// Artifacts are not workspace files. The store keeps owner metadata
    /// inside this user-state directory and authorizes every read against the
    /// runtime-owned session identity.
    pub fn artifacts_directory(&self) -> PathBuf {
        self.directory.join(".artifacts")
    }

    /// Creates the session directory if it does not exist yet.
    pub async fn ensure_directory(&self) -> Result<(), RuntimeError> {
        ensure_private_directory(&self.directory).await
    }
}

/// One entry of [`FileSessionStore::list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListing {
    /// The session's identifier.
    pub id: SessionId,
    /// The snapshot file.
    pub path: PathBuf,
    /// The record's on-disk schema version.
    pub schema_version: u32,
    /// When the snapshot was last saved, or `None` when the record's schema
    /// version is newer than this build understands. A session written by a
    /// newer Smith is still worth listing — resuming it is what fails, not
    /// knowing it exists.
    pub updated: Option<Timestamp>,
    /// Completed turn count, when listing metadata is understood.
    pub turn_count: Option<u64>,
    /// Provider used by the latest recorded turn, when known.
    pub provider: Option<String>,
    /// Model used by the latest recorded turn, when known.
    pub model: Option<String>,
    /// Bounded, single-line text from the latest user message.
    pub user_preview: Option<String>,
}

/// The persisted form of a snapshot: an explicit version plus the
/// host-prepared runtime payload.
#[derive(Debug, Serialize, Deserialize)]
struct StoredSnapshot {
    schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    listing: Option<StoredListingMetadata>,
    snapshot: SessionSnapshot,
}

/// Just enough of a stored record to decide whether this build may parse it.
///
/// Read first, and separately, so an unknown version produces a compatibility
/// error rather than a confusing failure from inside the payload.
#[derive(Debug, Deserialize)]
struct StoredVersion {
    schema_version: u32,
}

/// Just enough of a stored record to list it without materializing history.
#[derive(Debug, Deserialize)]
struct StoredIdentity {
    #[serde(default)]
    listing: Option<StoredListingMetadata>,
    snapshot: StoredIdentityPayload,
}

#[derive(Debug, Deserialize)]
struct StoredIdentityPayload {
    updated: Timestamp,
}

/// Versioned, bounded metadata prepared while saving a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredListingMetadata {
    schema_version: u32,
    turn_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_preview: Option<String>,
}

/// A [`SessionStore`] backed by one versioned JSON file per session.
///
/// Snapshots sit beside their event journal in the same session directory, so
/// a session is one directory listing rather than a join across two trees.
#[derive(Debug)]
pub struct FileSessionStore {
    paths: SessionPaths,
}

impl FileSessionStore {
    /// A store over `paths`.
    pub fn new(paths: SessionPaths) -> Self {
        Self { paths }
    }

    /// The paths this store reads and writes.
    pub fn paths(&self) -> &SessionPaths {
        &self.paths
    }

    /// Enumerates the project's saved sessions, most recently updated first.
    ///
    /// A missing session directory is an empty list, not an error: a project
    /// that has never been opened has no sessions. An individual file that
    /// cannot be read at all is logged and skipped, because one damaged
    /// snapshot must not make every other session unlistable.
    pub async fn list(&self) -> Result<Vec<SessionListing>, RuntimeError> {
        let mut entries = match tokio::fs::read_dir(&self.paths.directory).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(io_error(format!(
                    "cannot list sessions in `{}`: {err}",
                    self.paths.directory.display()
                )));
            }
        };

        let mut listings = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|err| {
            io_error(format!(
                "cannot list sessions in `{}`: {err}",
                self.paths.directory.display()
            ))
        })? {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(id) = name.strip_suffix(SNAPSHOT_SUFFIX) else {
                continue;
            };

            let bytes = match read_private(&path).await {
                Ok(Some(bytes)) => bytes,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "skipping unreadable snapshot");
                    continue;
                }
            };
            let version = match serde_json::from_slice::<StoredVersion>(&bytes) {
                Ok(version) => version.schema_version,
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "skipping unversioned snapshot");
                    continue;
                }
            };
            let (updated, metadata) = if version == SNAPSHOT_SCHEMA_VERSION {
                serde_json::from_slice::<StoredIdentity>(&bytes)
                    .ok()
                    .map_or((None, None), |identity| {
                        (Some(identity.snapshot.updated), identity.listing)
                    })
            } else {
                (None, None)
            };
            let metadata =
                metadata.filter(|metadata| metadata.schema_version == LISTING_SCHEMA_VERSION);

            listings.push(SessionListing {
                id: SessionId::new(id),
                path,
                schema_version: version,
                updated,
                turn_count: metadata.as_ref().map(|metadata| metadata.turn_count),
                provider: metadata
                    .as_ref()
                    .and_then(|metadata| metadata.provider.clone()),
                model: metadata
                    .as_ref()
                    .and_then(|metadata| metadata.model.clone()),
                user_preview: metadata.and_then(|metadata| metadata.user_preview),
            });
        }

        // Newest first, with records this build cannot date sorted last: a
        // resume picker wants the session the user was just in at the top.
        listings.sort_by(|a, b| match (b.updated, a.updated) {
            (Some(left), Some(right)) => left.cmp(&right).then_with(|| a.id.cmp(&b.id)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.id.cmp(&b.id),
        });
        Ok(listings)
    }
}

#[async_trait]
impl SessionStore for FileSessionStore {
    async fn load(&self, id: &SessionId) -> Result<Option<SessionSnapshot>, RuntimeError> {
        let path = self.paths.snapshot(id)?;
        let Some(bytes) = read_private(&path).await? else {
            return Ok(None);
        };

        // The version gate runs before the payload parse. Reversing the order
        // would let a future record fail as "missing field", which reads like
        // corruption and invites a partial-parse workaround.
        let version = serde_json::from_slice::<StoredVersion>(&bytes)
            .map_err(|err| {
                RuntimeError::new(
                    ErrorKind::Serialization,
                    format!("session `{id}` has no readable schema version: {err}"),
                )
            })?
            .schema_version;
        if version != SNAPSHOT_SCHEMA_VERSION {
            return Err(RuntimeError::new(
                ErrorKind::Serialization,
                format!(
                    "session `{id}` was written with snapshot schema version {version}; \
                     this build reads version {SNAPSHOT_SCHEMA_VERSION}"
                ),
            )
            .with_metadata(
                Metadata::new()
                    .with("session", id.as_str())
                    .with("found_schema_version", u64::from(version))
                    .with(
                        "supported_schema_version",
                        u64::from(SNAPSHOT_SCHEMA_VERSION),
                    ),
            ));
        }

        let mut stored: StoredSnapshot = serde_json::from_slice(&bytes).map_err(|err| {
            RuntimeError::new(
                ErrorKind::Serialization,
                format!("session `{id}` could not be parsed: {err}"),
            )
        })?;
        retain_redaction_safe_extension_state(&mut stored.snapshot);
        Ok(Some(stored.snapshot))
    }

    async fn save(&self, snapshot: &SessionSnapshot) -> Result<(), RuntimeError> {
        let path = self.paths.snapshot(&snapshot.id)?;
        self.paths.ensure_directory().await?;
        let mut snapshot = snapshot.clone();
        retain_redaction_safe_extension_state(&mut snapshot);
        let session_id = snapshot.id.clone();

        let stored = StoredSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            listing: Some(listing_metadata(&snapshot)),
            snapshot,
        };
        let bytes = serde_json::to_vec(&stored).map_err(|err| {
            RuntimeError::new(
                ErrorKind::Serialization,
                format!("session `{session_id}` could not be serialized: {err}"),
            )
        })?;

        write_private_atomically(&path, &bytes).await
    }
}

/// Enforces the confidentiality boundary of Smith's ordinary JSON store.
///
/// Omitting a namespace is intentional: a placeholder value could be mistaken
/// for valid resumable component state. The authenticated checkpoint retains
/// the exact snapshot separately when mid-turn durability is available.
fn retain_redaction_safe_extension_state(snapshot: &mut SessionSnapshot) {
    snapshot
        .extension_state
        .retain(|_, state| state.sensitivity == SessionStateSensitivity::RedactionSafe);
}

fn listing_metadata(snapshot: &SessionSnapshot) -> StoredListingMetadata {
    let (provider, model) = snapshot.manifests.last().map_or((None, None), |entry| {
        (
            Some(entry.manifest.model.provider.clone()),
            Some(entry.manifest.model.model.as_str().to_owned()),
        )
    });
    let user_preview = snapshot
        .history
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
        .map(|message| single_line(&message.joined_text()))
        .filter(|preview| !preview.is_empty());
    StoredListingMetadata {
        schema_version: LISTING_SCHEMA_VERSION,
        turn_count: snapshot.identity.turn,
        provider,
        model,
        user_preview,
    }
}

fn single_line(text: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 120;

    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut characters = collapsed.chars();
    let mut bounded: String = characters.by_ref().take(MAX_PREVIEW_CHARS).collect();
    if characters.next().is_some() {
        bounded.push('…');
    }
    bounded
}

/// Filesystem failures carry no runtime classification of their own, so they
/// land in [`ErrorKind::Internal`] with a message naming the operation.
fn io_error(message: String) -> RuntimeError {
    RuntimeError::new(ErrorKind::Internal, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> ProjectId {
        ProjectId::new("demo").expect("a project id")
    }

    #[test]
    fn a_project_id_that_would_escape_its_directory_is_refused() {
        assert!(ProjectId::new("..").is_err());
        assert!(ProjectId::new("a/b").is_err());
        assert!(ProjectId::new("").is_err());
        assert!(ProjectId::new("ok-1.2").is_ok());
    }

    #[test]
    fn the_default_root_shape_is_state_root_sessions_project() {
        let paths = SessionPaths::new("/state", &project());
        assert!(paths.directory().ends_with("sessions/demo"));
        let snapshot = paths.snapshot(&SessionId::new("s-1")).expect("a path");
        assert!(snapshot.ends_with("sessions/demo/s-1.snapshot.json"));
        let journal = paths.journal(&SessionId::new("s-1")).expect("a path");
        assert!(journal.ends_with("sessions/demo/s-1.jsonl"));
        let tasks = paths.tasks_dir(&SessionId::new("s-1")).expect("a path");
        assert!(tasks.ends_with("sessions/demo/s-1.tasks"));
    }

    #[test]
    fn a_session_id_that_would_escape_its_directory_is_refused() {
        let paths = SessionPaths::new("/state", &project());
        let err = paths
            .snapshot(&SessionId::new("../../escape"))
            .expect_err("an error");
        assert_eq!(err.kind, ErrorKind::Config);
        assert!(paths.journal(&SessionId::new("../../escape")).is_err());
        assert!(paths.tasks_dir(&SessionId::new("../../escape")).is_err());
    }
}
