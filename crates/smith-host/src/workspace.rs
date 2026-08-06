//! The project-root write boundary.
//!
//! The runtime's default is [`DenyAllWorkspace`](agent_runtime_core::workspace::DenyAllWorkspace),
//! which contains nothing. Smith replaces it with the project root the user
//! opened — and nothing above it.
//!
//! Containment is decided on **canonicalized** paths. Comparing raw strings
//! would let `"<root>/../etc/passwd"` pass a prefix check, and on macOS would
//! reject a legitimate path because `/tmp` is a symlink to `/private/tmp`.
//! Resolving both sides removes both failure modes.

use std::path::{Component, Path, PathBuf};

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::workspace::Workspace;

/// A workspace rooted at one project directory.
#[derive(Debug, Clone)]
pub struct ProjectWorkspace {
    root: PathBuf,
    display: String,
}

impl ProjectWorkspace {
    /// Roots a workspace at `root`, resolving symlinks and `..` components.
    ///
    /// The root must exist: a boundary that cannot be resolved is a boundary
    /// that cannot be enforced.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let root = root.as_ref();
        let canonical = root.canonicalize().map_err(|err| {
            RuntimeError::new(
                ErrorKind::Workspace,
                format!("project root `{}` is unusable: {err}", root.display()),
            )
        })?;
        let display = canonical.to_string_lossy().into_owned();
        Ok(Self {
            root: canonical,
            display,
        })
    }

    /// Resolves `path` against the root without requiring it to exist, so a
    /// tool may create a new file inside the boundary.
    ///
    /// Existing paths are canonicalized. For a path that does not exist yet,
    /// the nearest existing ancestor is canonicalized and the remaining
    /// components are applied lexically — `..` pops, `.` is dropped — so a
    /// traversal cannot climb out through a not-yet-created directory.
    fn resolve_lexically(&self, path: &str) -> Option<PathBuf> {
        let candidate = {
            let raw = Path::new(path);
            if raw.is_absolute() {
                raw.to_path_buf()
            } else {
                self.root.join(raw)
            }
        };

        if let Ok(canonical) = candidate.canonicalize() {
            return Some(canonical);
        }

        // Walk down from the deepest ancestor that does exist.
        let mut existing = candidate.as_path();
        let mut trailing = Vec::new();
        loop {
            let parent = existing.parent()?;
            trailing.push(existing.file_name()?.to_owned());
            existing = parent;
            if let Ok(canonical) = existing.canonicalize() {
                let mut resolved = canonical;
                for component in trailing.iter().rev() {
                    match Path::new(component).components().next() {
                        Some(Component::ParentDir) => {
                            resolved.pop();
                        }
                        Some(Component::CurDir) => {}
                        _ => resolved.push(component),
                    }
                }
                return Some(resolved);
            }
        }
    }
}

impl Workspace for ProjectWorkspace {
    fn root(&self) -> &str {
        &self.display
    }

    fn contains(&self, path: &str) -> bool {
        self.resolve_lexically(path)
            .is_some_and(|resolved| resolved.starts_with(&self.root))
    }

    fn resolve(&self, path: &str) -> Result<String, RuntimeError> {
        let resolved = self.resolve_lexically(path).ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::Workspace,
                format!("path `{path}` could not be resolved"),
            )
        })?;
        if resolved.starts_with(&self.root) {
            Ok(resolved.to_string_lossy().into_owned())
        } else {
            Err(RuntimeError::new(
                ErrorKind::Workspace,
                format!("path `{path}` is outside the project `{}`", self.display),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, ProjectWorkspace) {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::create_dir_all(dir.path().join("src")).expect("src");
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").expect("a file");
        let workspace = ProjectWorkspace::new(dir.path()).expect("a workspace");
        (dir, workspace)
    }

    #[test]
    fn existing_paths_inside_the_root_are_contained() {
        let (dir, workspace) = project();
        assert!(workspace.contains("src/main.rs"));
        assert!(workspace.contains(dir.path().join("src/main.rs").to_str().unwrap()));
    }

    #[test]
    fn a_new_file_inside_the_root_is_contained() {
        let (_dir, workspace) = project();
        assert!(workspace.contains("src/created_later.rs"));
        assert!(workspace.contains("does/not/exist/yet.txt"));
    }

    #[test]
    fn traversal_out_of_the_root_is_rejected() {
        let (_dir, workspace) = project();
        assert!(!workspace.contains("../escape.txt"));
        assert!(!workspace.contains("src/../../escape.txt"));
        assert!(!workspace.contains("/etc/passwd"));

        let err = workspace.resolve("../escape.txt").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Workspace);
    }

    #[test]
    fn traversal_through_a_missing_directory_is_rejected() {
        let (_dir, workspace) = project();
        // The middle directory does not exist, so the check cannot rely on
        // canonicalize alone.
        assert!(!workspace.contains("not_yet/../../escape.txt"));
    }

    #[test]
    fn resolve_returns_the_canonical_path() {
        let (dir, workspace) = project();
        let resolved = workspace.resolve("src/main.rs").expect("resolved");
        let expected = dir
            .path()
            .canonicalize()
            .unwrap()
            .join("src/main.rs")
            .to_string_lossy()
            .into_owned();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn a_missing_root_is_an_error_rather_than_an_empty_boundary() {
        let err = ProjectWorkspace::new("/definitely/not/a/real/project").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Workspace);
    }
}
