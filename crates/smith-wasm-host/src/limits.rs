use serde::Serialize;
use std::time::Duration;

/// Per-instance and per-call ceilings. Host configuration may only narrow ABI bounds.
#[derive(Debug, Clone)]
pub struct WasmLimits {
    /// Maximum total linear memory in one store.
    pub memory_bytes: usize,
    /// Tool call deadline.
    pub tool_deadline: Duration,
    /// Describe, mount and pipeline deadline.
    pub component_deadline: Duration,
    /// Status refresh deadline.
    pub status_deadline: Duration,
    /// Aggregate argument/result bytes, no greater than 1 MiB.
    pub payload_bytes: usize,
}
impl Default for WasmLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            tool_deadline: Duration::from_secs(5),
            component_deadline: Duration::from_millis(500),
            status_deadline: Duration::from_millis(5),
            payload_bytes: 1024 * 1024,
        }
    }
}

/// Isolated, typed invocation and loading failures. Messages contain no guest secrets.
#[derive(Debug, Clone, thiserror::Error)]
pub enum WasmError {
    /// Guest trapped.
    #[error("module trapped")]
    Trap,
    /// Guest exceeded the epoch deadline.
    #[error("module deadline exceeded")]
    Deadline,
    /// Guest exceeded store resource limits.
    #[error("module memory/resource limit exceeded")]
    Memory,
    /// ABI payload bound exceeded.
    #[error("module payload too large: {0}")]
    OversizedPayload(String),
    /// Invalid declaration or patch.
    #[error("malformed module result: {0}")]
    MalformedResult(String),
    /// Guest requested an ungranted host function.
    #[error("capability denied: {0}")]
    CapabilityDenied(String),
    /// Repeated failures permanently disabled this session instance.
    #[error("module disabled: {0}")]
    Disabled(String),
    /// Unsupported interface package version.
    #[error(
        "unsupported module interface {found}; supported smith:module@0.1.0 (compatible 0.1.x; no previous package yet)"
    )]
    InterfaceVersion {
        /// Found package/version.
        found: String,
    },
    /// Compilation, missing imports or host I/O.
    #[error("module load failed: {0}")]
    Load(String),
}
impl WasmError {
    pub(crate) fn guest(error: wasmtime::Error, memory_denied: bool) -> Self {
        if memory_denied {
            return Self::Memory;
        }
        if error.downcast_ref::<wasmtime::Trap>() == Some(&wasmtime::Trap::Interrupt) {
            Self::Deadline
        } else {
            Self::Trap
        }
    }
}

pub(crate) fn bounded<T: Serialize>(value: &T, limits: &WasmLimits) -> Result<(), WasmError> {
    crate::bounds::bounded(value, limits)
}
pub(crate) fn identifier(id: &str) -> Result<(), WasmError> {
    if id.len() > 128 {
        return Err(WasmError::OversizedPayload("identifier".into()));
    }
    if id.is_empty()
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        Err(WasmError::MalformedResult("invalid identifier".into()))
    } else {
        Ok(())
    }
}

// The ABI carries ToolOutcome as one opaque JSON string, not a WIT list/tree.
// Its inner arbitrary JSON keys must not acquire typed-record field limits.
pub(crate) fn bounded_outcome(
    outcome: &agent_runtime_core::tool::ToolOutcome,
    limits: &WasmLimits,
) -> Result<(), WasmError> {
    let bytes = serde_json::to_vec(outcome)
        .map_err(|_| WasmError::MalformedResult("outcome encoding".into()))?;
    if bytes.len() > limits.payload_bytes.min(1024 * 1024) {
        return Err(WasmError::OversizedPayload("tool outcome".into()));
    }
    Ok(())
}

pub(crate) fn revision(revision: &str) -> Result<(), WasmError> {
    if revision.len() > 128 {
        return Err(WasmError::OversizedPayload("revision".into()));
    }
    if revision.trim().is_empty() {
        return Err(WasmError::MalformedResult("empty revision".into()));
    }
    Ok(())
}
