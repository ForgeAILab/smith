//! Shared plumbing for the built-in tools.
//!
//! Three concerns recur in every tool and are centralized here so a single
//! tool cannot get them subtly wrong:
//!
//! - **Every path goes through the workspace.** [`resolve`] is the only way a
//!   tool turns an argument into a display path, while actual filesystem I/O
//!   goes through Smith's open project-directory capability. Ordinary tools
//!   never accept an out-of-project path, even after approval.
//! - **Reads are handle-bound and bounded.** [`read_bounded`] opens once,
//!   inspects that handle, reads at most `max_bytes + 1`, and returns a version
//!   token for exactly those bytes.
//! - **Binary content never reaches the model.** [`looks_binary`] rejects it up
//!   front, because a screenful of mojibake wastes tokens and tells the model
//!   nothing.

use std::path::{Path, PathBuf};

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::tool::{InvocationContext, PreparationContext};
use agent_runtime_core::workspace::Workspace;
use serde_json::Value;
use smith_host::workspace::{FileVersion, ProjectWorkspace};

/// The largest file a read-shaped tool will load, regardless of output limits.
pub const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;

/// How much of a file is inspected when deciding whether it is binary.
const SNIFF_BYTES: usize = 8192;

/// Resolves a path argument against the session's workspace boundary.
///
/// This is the presentation-side choke point for containment. The returned
/// string is not an I/O capability; callers must use [`project_workspace`] for
/// the actual operation.
pub fn resolve(ctx: &InvocationContext, path: &str) -> Result<PathBuf, RuntimeError> {
    ctx.workspace.resolve(path).map(PathBuf::from)
}

/// Returns the concrete Smith capability behind a runtime context.
pub fn project_workspace(ctx: &InvocationContext) -> Result<&ProjectWorkspace, RuntimeError> {
    ProjectWorkspace::from_workspace(ctx.workspace.as_ref()).ok_or_else(|| {
        RuntimeError::new(
            ErrorKind::Workspace,
            "Smith built-in filesystem tools require a ProjectWorkspace capability",
        )
    })
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
    let canonical = workspace.resolve(raw)?;
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

/// UTF-8 text and the exact version returned from one capability read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedText {
    /// Decoded contents.
    pub text: String,
    /// Version for the same bytes.
    pub version: FileVersion,
}

/// Reads a file as UTF-8 text through the project capability.
pub async fn read_bounded(
    ctx: &InvocationContext,
    path: &Path,
    max_bytes: u64,
) -> Result<BoundedText, RuntimeError> {
    read_bounded_from_workspace(project_workspace(ctx)?.clone(), path, max_bytes).await
}

/// Reads bounded UTF-8 text through an already resolved project capability.
/// Host-side attachment materialization uses this without fabricating a tool
/// invocation context.
pub async fn read_bounded_from_workspace(
    workspace: ProjectWorkspace,
    path: &Path,
    max_bytes: u64,
) -> Result<BoundedText, RuntimeError> {
    let path = path.to_owned();
    let display = path.clone();
    let max_bytes = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    let read = tokio::task::spawn_blocking(move || workspace.read_bounded(path, max_bytes))
        .await
        .map_err(|_| RuntimeError::internal("workspace read task failed"))??;
    if looks_binary(&read.bytes) {
        return Err(invalid(format!(
            "`{}` looks like binary content and cannot be shown as text",
            display.display()
        )));
    }
    let text = String::from_utf8(read.bytes).map_err(|_| {
        invalid(format!(
            "`{}` is not valid UTF-8 and cannot be shown as text",
            display.display()
        ))
    })?;
    Ok(BoundedText {
        text,
        version: read.version,
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
        let (dir, ctx) = crate::testing::project();
        let path = dir.path().join("big.txt");
        tokio::fs::write(&path, "x".repeat(2048)).await.unwrap();

        let err = read_bounded(&ctx, &path, 1024).await.unwrap_err();
        assert!(err.message.contains("over the"), "{err:?}");
        assert!(read_bounded(&ctx, &path, 4096).await.is_ok());
    }

    #[tokio::test]
    async fn a_binary_file_is_refused_as_text() {
        let (dir, ctx) = crate::testing::project();
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, [0x7f, b'E', b'L', b'F', 0x00, 0x01])
            .await
            .unwrap();

        let err = read_bounded(&ctx, &path, MAX_READ_BYTES).await.unwrap_err();
        assert!(err.message.contains("binary"), "{err:?}");
    }

    #[tokio::test]
    async fn a_directory_is_refused_with_a_pointer_to_the_right_tool() {
        let (dir, ctx) = crate::testing::project();
        let err = read_bounded(&ctx, dir.path(), MAX_READ_BYTES)
            .await
            .unwrap_err();
        assert!(err.message.contains("`list` tool"), "{err:?}");
    }

    #[test]
    fn an_outside_path_is_refused_during_preparation() {
        let (_dir, ctx) = crate::testing::project();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("note.txt");
        std::fs::write(&file, "hi").unwrap();

        let err = prepare_workspace_path(ctx.workspace.as_ref(), file.to_str().unwrap())
            .expect_err("ordinary tools cannot request host paths");
        assert_eq!(err.kind, ErrorKind::Workspace);
    }

    #[test]
    fn a_relative_traversal_is_refused_during_preparation() {
        let (_dir, ctx) = crate::testing::project();
        let err = prepare_workspace_path(ctx.workspace.as_ref(), "../escape.txt")
            .expect_err("traversal must fail closed");
        assert_eq!(err.kind, ErrorKind::Workspace);
    }

    #[test]
    fn an_outside_path_is_refused_at_invoke() {
        let (_dir, ctx) = crate::testing::project();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("note.txt");
        std::fs::write(&file, "hi").unwrap();
        let canonical = file.canonicalize().unwrap();

        let err = resolve(&ctx, canonical.to_str().unwrap()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Workspace);
    }
}
