//! Filesystem-free lexical containment for factory integration tests.

use std::path::{Component, Path, PathBuf};

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::workspace::Workspace;

/// Models `ProjectWorkspace`'s relative paths without requiring `/repo` to exist.
#[derive(Debug, Clone)]
pub(crate) struct LexicalWorkspace {
    root: String,
}

impl LexicalWorkspace {
    pub(crate) fn new(root: impl Into<String>) -> Self {
        Self { root: root.into() }
    }

    fn resolve_lexically(&self, path: &str) -> Option<PathBuf> {
        let root = Path::new(&self.root);
        let path = Path::new(path);
        let relative = if path.is_absolute() {
            path.strip_prefix(root).ok()?
        } else {
            path
        };
        let mut resolved = root.to_path_buf();
        for component in relative.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if resolved == root {
                        return None;
                    }
                    resolved.pop();
                }
                Component::Normal(name) => resolved.push(name),
                _ => return None,
            }
        }
        Some(resolved)
    }
}

impl Workspace for LexicalWorkspace {
    fn root(&self) -> &str {
        &self.root
    }

    fn contains(&self, path: &str) -> bool {
        self.resolve_lexically(path).is_some()
    }

    fn resolve(&self, path: &str) -> Result<String, RuntimeError> {
        self.resolve_lexically(path)
            .map(|resolved| resolved.to_string_lossy().into_owned())
            .ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::Workspace,
                    format!("path `{path}` is outside workspace `{}`", self.root),
                )
            })
    }
}

#[test]
fn lexical_paths_stay_inside_the_workspace() {
    let workspace = LexicalWorkspace::new("/repo");
    assert_eq!(workspace.root(), "/repo");
    for (path, resolved) in [
        ("/repo", "/repo"),
        (".", "/repo"),
        ("project:files", "/repo/project:files"),
        ("src/main.rs", "/repo/src/main.rs"),
        ("/repo/src/main.rs", "/repo/src/main.rs"),
        ("src/../main.rs", "/repo/main.rs"),
        ("/repo/src/../main.rs", "/repo/main.rs"),
    ] {
        assert!(workspace.contains(path), "{path}");
        assert_eq!(workspace.resolve(path).unwrap(), resolved);
    }
    for path in [
        "/outside",
        "/repository/main.rs",
        "../outside",
        "src/../../outside",
        "/repo/../outside",
        "../repo/main.rs",
    ] {
        assert!(!workspace.contains(path), "{path}");
        assert_eq!(
            workspace.resolve(path).unwrap_err().kind,
            ErrorKind::Workspace
        );
    }
}
