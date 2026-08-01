//! Bounded, immutable project instructions activated by a standard Smith host.
//!
//! The runtime factory remains an explicit composition boundary: it never
//! reaches into a workspace to discover ambient files. Standard hosts call
//! [`discover`] once before construction and carry the resulting snapshot on
//! the runtime request. Children then inherit the already composed prompt
//! contributor, so one agent tree cannot observe mixed file revisions.

use std::fmt;
use std::fs;
use std::path::Path;

use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::error::RuntimeError;

/// Root-level project instruction file Smith recognizes in this release.
pub const PROJECT_INSTRUCTIONS_FILE: &str = "AGENTS.md";

/// Maximum exact project-instruction body accepted by the host.
pub const MAX_PROJECT_INSTRUCTIONS_BYTES: usize = 32 * 1024;

/// Redaction-safe identity exposed through runtime composition diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInstructionsIdentity {
    /// Canonical project-relative source label.
    pub source: String,
    /// Revision derived from the source label and exact body.
    pub revision: RegistryRevision,
}

/// One exact project-instruction body frozen for a constructed runtime.
#[derive(Clone, PartialEq, Eq)]
pub struct ProjectInstructionsSnapshot {
    source: String,
    revision: RegistryRevision,
    body: String,
}

impl ProjectInstructionsSnapshot {
    /// Creates the root instruction snapshot direct embedders may supply.
    ///
    /// Standard hosts use [`discover`] so path, file type, UTF-8, and size are
    /// all validated before this constructor is reached.
    pub fn from_body(body: impl Into<String>) -> Result<Self, RuntimeError> {
        let body = body.into();
        if body.len() > MAX_PROJECT_INSTRUCTIONS_BYTES {
            return Err(RuntimeError::config(format!(
                "`{PROJECT_INSTRUCTIONS_FILE}` is {} bytes, over the \
                 {MAX_PROJECT_INSTRUCTIONS_BYTES}-byte project-instruction limit",
                body.len()
            )));
        }
        let source = PROJECT_INSTRUCTIONS_FILE.to_owned();
        let revision = RegistryRevision::from_content(format!("{source}\0{body}"));
        Ok(Self {
            source,
            revision,
            body,
        })
    }

    /// Canonical project-relative source label.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Exact content-derived revision.
    pub fn revision(&self) -> &RegistryRevision {
        &self.revision
    }

    /// Exact bounded body supplied only at the authoritative prompt boundary.
    pub(crate) fn body(&self) -> &str {
        &self.body
    }

    /// Redaction-safe composition identity.
    pub fn identity(&self) -> ProjectInstructionsIdentity {
        ProjectInstructionsIdentity {
            source: self.source.clone(),
            revision: self.revision.clone(),
        }
    }
}

impl fmt::Debug for ProjectInstructionsSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectInstructionsSnapshot")
            .field("source", &self.source)
            .field("revision", &self.revision)
            .field("bytes", &self.body.len())
            .finish()
    }
}

/// Discovers one exact root `AGENTS.md` snapshot.
///
/// Absence is ordinary. A present file fails closed when it cannot be used
/// exactly; Smith never truncates or silently skips declared instructions.
pub fn discover(
    project_root: impl AsRef<Path>,
) -> Result<Option<ProjectInstructionsSnapshot>, RuntimeError> {
    let requested_root = project_root.as_ref();
    let root = requested_root.canonicalize().map_err(|error| {
        RuntimeError::config(format!(
            "project root `{}` cannot be resolved while loading `{PROJECT_INSTRUCTIONS_FILE}`: \
             {error}",
            requested_root.display()
        ))
    })?;
    let path = root.join(PROJECT_INSTRUCTIONS_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(RuntimeError::config(format!(
                "project instructions `{}` cannot be inspected: {error}",
                path.display()
            )));
        }
    };

    if metadata.file_type().is_symlink() {
        return Err(RuntimeError::config(format!(
            "project instructions `{}` must be a regular non-symlinked UTF-8 file",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(RuntimeError::config(format!(
            "project instructions `{}` are not a regular file",
            path.display()
        )));
    }
    let max_bytes = u64::try_from(MAX_PROJECT_INSTRUCTIONS_BYTES).unwrap_or(u64::MAX);
    if metadata.len() > max_bytes {
        return Err(RuntimeError::config(format!(
            "project instructions `{}` are {} bytes, over the \
             {MAX_PROJECT_INSTRUCTIONS_BYTES}-byte limit",
            path.display(),
            metadata.len()
        )));
    }

    let canonical = path.canonicalize().map_err(|error| {
        RuntimeError::config(format!(
            "project instructions `{}` cannot be resolved: {error}",
            path.display()
        ))
    })?;
    if !canonical.starts_with(&root) {
        return Err(RuntimeError::config(format!(
            "project instructions `{}` resolve outside project `{}`",
            path.display(),
            root.display()
        )));
    }

    let bytes = fs::read(&canonical).map_err(|error| {
        RuntimeError::config(format!(
            "project instructions `{}` cannot be read: {error}",
            canonical.display()
        ))
    })?;
    if bytes.len() > MAX_PROJECT_INSTRUCTIONS_BYTES {
        return Err(RuntimeError::config(format!(
            "project instructions `{}` changed while being read and exceed the \
             {MAX_PROJECT_INSTRUCTIONS_BYTES}-byte limit",
            canonical.display()
        )));
    }
    let body = String::from_utf8(bytes).map_err(|_| {
        RuntimeError::config(format!(
            "project instructions `{}` are not valid UTF-8",
            canonical.display()
        ))
    })?;
    ProjectInstructionsSnapshot::from_body(body).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_root_file_contributes_nothing() {
        let root = tempfile::tempdir().expect("a project root");
        assert_eq!(discover(root.path()).expect("absence is valid"), None);
    }

    #[test]
    fn a_valid_file_is_exact_revisioned_and_debug_safe() {
        let root = tempfile::tempdir().expect("a project root");
        let marker = "PROJECT_INSTRUCTION_SECRET_MARKER";
        fs::write(root.path().join(PROJECT_INSTRUCTIONS_FILE), marker).expect("instructions");

        let snapshot = discover(root.path())
            .expect("valid discovery")
            .expect("a snapshot");
        assert_eq!(snapshot.source(), PROJECT_INSTRUCTIONS_FILE);
        assert_eq!(snapshot.body(), marker);
        let debug = format!("{snapshot:?}");
        assert!(debug.contains(PROJECT_INSTRUCTIONS_FILE));
        assert!(debug.contains(snapshot.revision().as_str()));
        assert!(!debug.contains(marker), "{debug}");
    }

    #[test]
    fn content_changes_receive_distinct_revisions() {
        let root = tempfile::tempdir().expect("a project root");
        let path = root.path().join(PROJECT_INSTRUCTIONS_FILE);
        fs::write(&path, "revision one").expect("first instructions");
        let first = discover(root.path())
            .expect("first discovery")
            .expect("first snapshot");
        fs::write(&path, "revision two").expect("changed instructions");
        let second = discover(root.path())
            .expect("second discovery")
            .expect("second snapshot");

        assert_ne!(first.revision(), second.revision());
        assert_eq!(first.body(), "revision one");
        assert_eq!(second.body(), "revision two");
    }

    #[test]
    fn an_oversized_file_fails_without_truncation() {
        let root = tempfile::tempdir().expect("a project root");
        fs::write(
            root.path().join(PROJECT_INSTRUCTIONS_FILE),
            vec![b'x'; MAX_PROJECT_INSTRUCTIONS_BYTES + 1],
        )
        .expect("oversized instructions");

        let error = discover(root.path()).expect_err("oversized instructions fail");
        assert!(error.to_string().contains("over the 32768-byte limit"));
    }

    #[test]
    fn invalid_utf8_fails_without_partial_activation() {
        let root = tempfile::tempdir().expect("a project root");
        fs::write(
            root.path().join(PROJECT_INSTRUCTIONS_FILE),
            [b'v', b'a', b'l', b'i', b'd', 0xff, b'x'],
        )
        .expect("invalid instructions");

        let error = discover(root.path()).expect_err("invalid UTF-8 fails");
        assert!(error.to_string().contains("not valid UTF-8"));
    }

    #[test]
    fn a_non_regular_instruction_path_fails() {
        let root = tempfile::tempdir().expect("a project root");
        fs::create_dir(root.path().join(PROJECT_INSTRUCTIONS_FILE)).expect("instruction directory");

        let error = discover(root.path()).expect_err("a directory is not instructions");
        assert!(error.to_string().contains("not a regular file"));
    }

    #[test]
    fn a_lexically_noncanonical_root_resolves_to_the_same_source() {
        let root = tempfile::tempdir().expect("a project root");
        fs::write(root.path().join(PROJECT_INSTRUCTIONS_FILE), "canonical").expect("instructions");

        let snapshot = discover(root.path().join("."))
            .expect("canonical discovery")
            .expect("a snapshot");
        assert_eq!(snapshot.source(), PROJECT_INSTRUCTIONS_FILE);
        assert_eq!(snapshot.body(), "canonical");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_refused_even_when_its_target_is_readable() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("a project root");
        let outside = tempfile::tempdir().expect("an outside root");
        let target = outside.path().join("instructions.md");
        fs::write(&target, "outside guidance").expect("outside instructions");
        symlink(&target, root.path().join(PROJECT_INSTRUCTIONS_FILE)).expect("instruction symlink");

        let error = discover(root.path()).expect_err("symlinked instructions fail");
        assert!(error.to_string().contains("non-symlinked"));
        assert!(!error.to_string().contains("outside guidance"));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_fails_with_its_path() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("a project root");
        let path = root.path().join(PROJECT_INSTRUCTIONS_FILE);
        fs::write(&path, "unreadable").expect("instructions");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("remove permissions");
        let outcome = discover(root.path());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("restore permissions");

        let error = outcome.expect_err("unreadable instructions fail");
        assert!(error.to_string().contains(PROJECT_INSTRUCTIONS_FILE));
        assert!(error.to_string().contains("cannot be read"));
    }
}
