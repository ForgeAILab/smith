//! Shared plumbing for the built-in tools.
//!
//! Three concerns recur in every tool and are centralized here so a single
//! tool cannot get them subtly wrong:
//!
//! - **Every path goes through the workspace.** [`resolve`] is the only way a
//!   tool turns an argument into a `PathBuf`, so the boundary check cannot be
//!   forgotten in one tool and enforced in the rest. A path outside the
//!   project is not refused here: it prepares with a host-mounted resource,
//!   and the authority routes that resource to the approval policy instead of
//!   allowing it unattended.
//! - **Reads are bounded before they happen.** [`read_bounded`] refuses a file
//!   larger than its cap instead of loading it and truncating afterwards; a
//!   2 GiB file should not become 2 GiB of resident memory on its way to a
//!   1 KiB result.
//! - **Binary content never reaches the model.** [`looks_binary`] rejects it up
//!   front, because a screenful of mojibake wastes tokens and tells the model
//!   nothing.

use std::path::{Component, Path, PathBuf};

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::tool::{InvocationContext, PreparationContext};
use agent_runtime_core::workspace::Workspace;
use serde_json::Value;

/// The largest file a read-shaped tool will load, regardless of output limits.
pub const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;

/// The resource mount recorded for a path outside the project: the host
/// filesystem root, so an out-of-workspace grant can never be mistaken for a
/// project-relative one.
pub const HOST_MOUNT: &str = "/";

/// How much of a file is inspected when deciding whether it is binary.
const SNIFF_BYTES: usize = 8192;

/// Resolves a path argument against the session's workspace boundary.
///
/// This is the choke point for containment. In-workspace paths defer to the
/// workspace's canonicalization. A path the workspace refuses is accepted
/// only when it re-resolves to itself, which is sound because invocation
/// receives fingerprint-bound prepared arguments: an out-of-workspace path
/// here was canonicalized by [`prepare_path_argument`] and authorized —
/// through approval — for exactly that path. The round-trip check keeps this
/// fail-closed against filesystem drift (a symlink swap) between preparation
/// and invocation.
pub fn resolve(ctx: &InvocationContext, path: &str) -> Result<PathBuf, RuntimeError> {
    match ctx.workspace.resolve(path) {
        Ok(resolved) => Ok(PathBuf::from(resolved)),
        Err(err) if err.kind == ErrorKind::Workspace => {
            match canonicalize_lexically(Path::new(ctx.workspace.root()), path) {
                Some(resolved) if resolved == Path::new(path) => Ok(resolved),
                _ => Err(err),
            }
        }
        Err(err) => Err(err),
    }
}

/// One workspace path canonicalized before authorization or approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPath {
    /// Canonical host path used as the immutable tool argument.
    pub canonical: String,
    /// Structural filesystem resource relative to the workspace mount.
    pub resource: SecurityResource,
    /// Project-relative path suitable for bounded human-facing metadata.
    pub display: String,
}

/// Canonicalizes one filesystem argument and replaces it in `arguments`.
///
/// The model may supply a relative path, an absolute in-workspace path, or no
/// path when the tool defines a root default. Preparation resolves that once,
/// stores the canonical path in the immutable arguments, and derives the
/// structurally equivalent security resource. Invocation resolves the
/// canonical value again only as a fail-closed workspace/TOCTOU check.
pub fn prepare_path_argument(
    arguments: &mut Value,
    key: &str,
    default: Option<&str>,
    ctx: &PreparationContext,
) -> Result<PreparedPath, RuntimeError> {
    let raw = match arguments.get(key) {
        Some(value) => value
            .as_str()
            .ok_or_else(|| invalid(format!("`{key}` must be a string")))?
            .to_owned(),
        None => default
            .ok_or_else(|| invalid(format!("`{key}` is required and must be a string")))?
            .to_owned(),
    };
    let prepared = prepare_workspace_path(ctx.workspace.as_ref(), &raw)?;
    let object = arguments
        .as_object_mut()
        .ok_or_else(|| invalid("tool arguments must be a JSON object"))?;
    object.insert(key.to_owned(), Value::String(prepared.canonical.clone()));
    Ok(prepared)
}

fn prepare_workspace_path(
    workspace: &dyn Workspace,
    raw: &str,
) -> Result<PreparedPath, RuntimeError> {
    let canonical = match workspace.resolve(raw) {
        Ok(canonical) => canonical,
        Err(err) if err.kind == ErrorKind::Workspace => {
            return prepare_outside_path(workspace, raw, err);
        }
        Err(err) => return Err(err),
    };
    let root = std::path::Path::new(workspace.root());
    let path = std::path::Path::new(&canonical);
    let relative = path.strip_prefix(root).map_err(|_| {
        RuntimeError::new(
            ErrorKind::Workspace,
            format!(
                "prepared path `{canonical}` is outside `{}`",
                root.display()
            ),
        )
    })?;
    let mut segments = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(segment) => {
                segments.push(segment.to_string_lossy().into_owned());
            }
            std::path::Component::CurDir => {}
            _ => {
                return Err(RuntimeError::new(
                    ErrorKind::Workspace,
                    format!("prepared path `{canonical}` is not structurally canonical"),
                ));
            }
        }
    }
    let display = if segments.is_empty() {
        ".".to_owned()
    } else {
        segments.join("/")
    };
    Ok(PreparedPath {
        canonical,
        resource: SecurityResource::filesystem(workspace.root(), segments),
        display,
    })
}

/// Prepares a path the workspace refused.
///
/// The path is canonicalized the same way the boundary would have, then
/// carried on a [`HOST_MOUNT`] resource: structurally distinct from every
/// project-relative resource, so the authority cannot mistake it for an
/// unattended workspace read and must send it through approval. Display uses
/// the absolute path — the user deciding the prompt needs to see exactly
/// what would be touched.
fn prepare_outside_path(
    workspace: &dyn Workspace,
    raw: &str,
    refusal: RuntimeError,
) -> Result<PreparedPath, RuntimeError> {
    let root = Path::new(workspace.root());
    let Some(resolved) = canonicalize_lexically(root, raw) else {
        return Err(refusal);
    };
    if resolved.starts_with(root) {
        // The boundary refused something the lexical fallback would place
        // inside it; the boundary wins.
        return Err(refusal);
    }
    let canonical = resolved.to_string_lossy().into_owned();
    let mut segments = Vec::new();
    for component in resolved.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => {}
            Component::Normal(segment) => {
                segments.push(segment.to_string_lossy().into_owned());
            }
            _ => {
                return Err(RuntimeError::new(
                    ErrorKind::Workspace,
                    format!("prepared path `{canonical}` is not structurally canonical"),
                ));
            }
        }
    }
    Ok(PreparedPath {
        display: canonical.clone(),
        resource: SecurityResource::filesystem(HOST_MOUNT, segments),
        canonical,
    })
}

/// Canonicalizes `raw` against `root` without requiring containment.
///
/// Mirrors the workspace's own resolution: an existing path canonicalizes
/// fully; for one that does not exist yet, the nearest existing ancestor is
/// canonicalized and the remaining components are applied lexically — `..`
/// pops, `.` is dropped — so a traversal cannot climb through a
/// not-yet-created directory.
fn canonicalize_lexically(root: &Path, raw: &str) -> Option<PathBuf> {
    let candidate = {
        let raw = Path::new(raw);
        if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            root.join(raw)
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

/// Reads a required string argument.
pub fn require_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, RuntimeError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("`{key}` is required and must be a string")))
}

/// Reads an optional string argument.
pub fn optional_str<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

/// Reads an optional non-negative integer argument.
pub fn optional_usize(arguments: &Value, key: &str) -> Option<usize> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

/// Reads an optional boolean argument.
pub fn optional_bool(arguments: &Value, key: &str) -> Option<bool> {
    arguments.get(key).and_then(Value::as_bool)
}

/// Builds an invalid-argument error.
pub fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::Tool, message)
}

/// Stops early when the invocation was cancelled or its deadline elapsed.
///
/// Tools that walk a tree or stream output must call this inside their loop;
/// checking only on entry would let a search of a huge repository run for
/// minutes after the user pressed Escape.
pub fn check_stop(ctx: &InvocationContext) -> Result<(), RuntimeError> {
    if ctx.should_stop() {
        return Err(RuntimeError::new(
            ErrorKind::Cancelled,
            "the tool stopped before finishing: cancelled or past its deadline",
        ));
    }
    Ok(())
}

/// Whether the bytes look like binary content rather than text.
///
/// A NUL byte is the signal: it cannot appear in valid UTF-8 text but is
/// ubiquitous in object files, images, and archives.
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(SNIFF_BYTES).any(|byte| *byte == 0)
}

/// Reads a file as UTF-8 text, refusing oversized or binary content.
pub async fn read_bounded(path: &std::path::Path, max_bytes: u64) -> Result<String, RuntimeError> {
    let metadata = tokio::fs::metadata(path).await.map_err(|err| {
        RuntimeError::new(
            ErrorKind::Tool,
            format!("cannot read `{}`: {err}", path.display()),
        )
    })?;

    if metadata.is_dir() {
        return Err(invalid(format!(
            "`{}` is a directory; use the `list` tool",
            path.display()
        )));
    }
    // Checked before reading, not after: the point is to never hold the bytes.
    if metadata.len() > max_bytes {
        return Err(invalid(format!(
            "`{}` is {} bytes, over the {max_bytes}-byte limit; read a line range instead",
            path.display(),
            metadata.len()
        )));
    }

    let bytes = tokio::fs::read(path).await.map_err(|err| {
        RuntimeError::new(
            ErrorKind::Tool,
            format!("cannot read `{}`: {err}", path.display()),
        )
    })?;
    if looks_binary(&bytes) {
        return Err(invalid(format!(
            "`{}` looks like binary content and cannot be shown as text",
            path.display()
        )));
    }
    String::from_utf8(bytes).map_err(|_| {
        invalid(format!(
            "`{}` is not valid UTF-8 and cannot be shown as text",
            path.display()
        ))
    })
}

/// Renders a path relative to the workspace root for display.
///
/// Absolute paths in tool output leak the user's directory layout into the
/// transcript and the model's context for no benefit.
pub fn display_path(ctx: &InvocationContext, path: &std::path::Path) -> String {
    let root = ctx.workspace.root();
    path.strip_prefix(root).map_or_else(
        |_| path.display().to_string(),
        |rest| rest.display().to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn binary_content_is_detected_by_its_nul_bytes() {
        assert!(!looks_binary(b"fn main() {}\n"));
        assert!(!looks_binary("caf\u{e9} \u{2615}".as_bytes()));
        assert!(looks_binary(b"\x7fELF\x02\x01\x01\x00"));
        assert!(looks_binary(&[b'a', 0, b'b']));
    }

    #[test]
    fn a_nul_beyond_the_sniff_window_is_not_inspected() {
        // Bounded work: the check must not scan a whole 4 MiB file.
        let mut bytes = vec![b'a'; SNIFF_BYTES + 10];
        bytes.push(0);
        assert!(!looks_binary(&bytes));
    }

    #[test]
    fn argument_helpers_report_the_missing_key_by_name() {
        let args = json!({"path": "src/main.rs", "limit": 40, "all": true});
        assert_eq!(require_str(&args, "path").unwrap(), "src/main.rs");
        assert_eq!(optional_usize(&args, "limit"), Some(40));
        assert_eq!(optional_bool(&args, "all"), Some(true));
        assert_eq!(optional_str(&args, "absent"), None);

        let err = require_str(&args, "pattern").unwrap_err();
        assert!(err.message.contains("`pattern` is required"), "{err:?}");
    }

    #[test]
    fn a_wrongly_typed_argument_is_rejected_rather_than_coerced() {
        let args = json!({"path": 42});
        assert!(require_str(&args, "path").is_err());
        assert_eq!(optional_usize(&json!({"limit": -1}), "limit"), None);
    }

    #[tokio::test]
    async fn an_oversized_file_is_refused_before_it_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        tokio::fs::write(&path, "x".repeat(2048)).await.unwrap();

        let err = read_bounded(&path, 1024).await.unwrap_err();
        assert!(err.message.contains("over the"), "{err:?}");
        assert!(read_bounded(&path, 4096).await.is_ok());
    }

    #[tokio::test]
    async fn a_binary_file_is_refused_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, [0x7f, b'E', b'L', b'F', 0x00, 0x01])
            .await
            .unwrap();

        let err = read_bounded(&path, MAX_READ_BYTES).await.unwrap_err();
        assert!(err.message.contains("binary"), "{err:?}");
    }

    #[tokio::test]
    async fn a_directory_is_refused_with_a_pointer_to_the_right_tool() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_bounded(dir.path(), MAX_READ_BYTES).await.unwrap_err();
        assert!(err.message.contains("`list` tool"), "{err:?}");
    }

    #[test]
    fn an_outside_path_prepares_with_a_host_resource() {
        let (_dir, ctx) = crate::testing::project();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("note.txt");
        std::fs::write(&file, "hi").unwrap();

        let prepared =
            prepare_workspace_path(ctx.workspace.as_ref(), file.to_str().unwrap()).unwrap();
        let canonical = file.canonicalize().unwrap();
        assert_eq!(prepared.canonical, canonical.to_string_lossy());
        assert_eq!(prepared.display, prepared.canonical);

        let segments: Vec<String> = canonical
            .components()
            .filter_map(|component| match component {
                Component::Normal(segment) => Some(segment.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(
            prepared.resource,
            SecurityResource::filesystem(HOST_MOUNT, segments)
        );
    }

    #[test]
    fn a_relative_traversal_prepares_as_an_outside_path() {
        let (dir, ctx) = crate::testing::project();
        let prepared = prepare_workspace_path(ctx.workspace.as_ref(), "../escape.txt").unwrap();
        let expected = dir
            .path()
            .canonicalize()
            .unwrap()
            .parent()
            .unwrap()
            .join("escape.txt");
        assert_eq!(prepared.canonical, expected.to_string_lossy());
        assert!(matches!(
            &prepared.resource,
            SecurityResource::Filesystem { mount, .. } if mount == HOST_MOUNT
        ));
    }

    #[test]
    fn an_approved_outside_path_survives_the_invoke_recheck() {
        let (_dir, ctx) = crate::testing::project();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("note.txt");
        std::fs::write(&file, "hi").unwrap();
        let canonical = file.canonicalize().unwrap();

        let resolved = resolve(&ctx, canonical.to_str().unwrap()).unwrap();
        assert_eq!(resolved, canonical);
    }

    #[test]
    fn a_non_canonical_outside_path_is_refused_at_invoke() {
        let (_dir, ctx) = crate::testing::project();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(outside.path().join("real")).unwrap();
        std::fs::write(outside.path().join("real/file.txt"), "hi").unwrap();
        std::os::unix::fs::symlink(outside.path().join("real"), outside.path().join("link"))
            .unwrap();

        // A path through the symlink is not its own canonical form, so the
        // fail-closed TOCTOU recheck must refuse it.
        let through_link = outside.path().join("link/file.txt");
        let err = resolve(&ctx, through_link.to_str().unwrap()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Workspace);
    }
}
