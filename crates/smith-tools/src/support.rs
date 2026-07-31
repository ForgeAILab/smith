//! Shared plumbing for the built-in tools.
//!
//! Three concerns recur in every tool and are centralized here so a single
//! tool cannot get them subtly wrong:
//!
//! - **Every path goes through the workspace.** [`resolve`] is the only way a
//!   tool turns an argument into a `PathBuf`, so the boundary check cannot be
//!   forgotten in one tool and enforced in the rest.
//! - **Reads are bounded before they happen.** [`read_bounded`] refuses a file
//!   larger than its cap instead of loading it and truncating afterwards; a
//!   2 GiB file should not become 2 GiB of resident memory on its way to a
//!   1 KiB result.
//! - **Binary content never reaches the model.** [`looks_binary`] rejects it up
//!   front, because a screenful of mojibake wastes tokens and tells the model
//!   nothing.

use std::path::PathBuf;

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::tool::InvocationContext;
use serde_json::Value;

/// The largest file a read-shaped tool will load, regardless of output limits.
pub const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;

/// How much of a file is inspected when deciding whether it is binary.
const SNIFF_BYTES: usize = 8192;

/// Resolves a path argument against the session's workspace boundary.
///
/// This is the choke point for containment: the workspace canonicalizes and
/// rejects anything outside the project root, so `../../etc/passwd` fails here
/// rather than in whichever tool forgot to check.
pub fn resolve(ctx: &InvocationContext, path: &str) -> Result<PathBuf, RuntimeError> {
    ctx.workspace.resolve(path).map(PathBuf::from)
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
}
