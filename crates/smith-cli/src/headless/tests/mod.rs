//! Shared fixtures for headless output, turn-flow, and background tests.

use std::sync::Arc;

use smith_runtime::client::LimitKind;

use agent_runtime::provider::fake::{
    FakeProvider, ScriptedStream, tool_call_fragments, usage_event,
};
use agent_runtime_core::approval::{AllowAll, DenyAll};
use agent_runtime_core::artifact::{
    ArtifactDigest, ArtifactId, ArtifactProvenance, ArtifactRead, ArtifactRef, ArtifactRetention,
    ArtifactSensitivity, MAX_ARTIFACT_READ_BYTES,
};
use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::goal::{GoalTokenUsage, GoalUsageProvenance};
use agent_runtime_core::ids::{AttemptId, EventId, GoalId, RequestId, TurnId};
use agent_runtime_core::provider::{
    Capabilities, FinishReason, Provider, ProviderError, ProviderErrorKind, ProviderStreamEvent,
};
use agent_runtime_core::usage::{CounterKind, Provenance, UsageRecord, UsageSource};
use smith_config::resolve::{ResolveRequest, resolve};
use smith_host::{InteractionNotice, InteractiveInteraction, ProjectWorkspace};
use smith_runtime::checkpoint::{CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError};
use smith_runtime::client::CacheState;
use smith_runtime::factory::{HostSurface, RuntimeRequest};
use smith_runtime::host::HostSessionRequest;

use super::*;

// These tests drive complete host/runtime turns while the Rust harness runs
// the rest of this binary's suite in parallel. Keep the watchdog generous
// enough for a contended hosted runner; it detects a real deadlock without
// turning scheduler latency into a product failure.
const HEADLESS_TEST_WATCHDOG: Duration = Duration::from_secs(10);

#[derive(Debug)]
struct TestCheckpointKeys;

impl CheckpointKeyProvider for TestCheckpointKeys {
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
        Ok(CheckpointKey::new([0x53; 32]))
    }
}

fn host_request(runtime: RuntimeRequest, project: &std::path::Path) -> HostSessionRequest {
    HostSessionRequest::new(runtime, project).checkpoint_keys(Arc::new(TestCheckpointKeys))
}

fn terminal_stream_result(lines: &[serde_json::Value]) -> &serde_json::Value {
    let result = lines.last().expect("terminal result");
    assert_eq!(result["type"], "result", "{lines:?}");
    let controller_positions = lines
        .iter()
        .enumerate()
        .filter_map(|(position, line)| (line["type"] == "cache_controller").then_some(position))
        .collect::<Vec<_>>();
    assert_eq!(controller_positions.len(), 1, "{lines:?}");
    let shutdown_position = lines
        .iter()
        .position(|line| line["event"]["payload"]["event"] == "session_shutdown")
        .expect("stream omitted canonical shutdown");
    assert!(
        shutdown_position < controller_positions[0] && controller_positions[0] < lines.len() - 1,
        "terminal stream tail is out of order: {lines:?}"
    );
    assert_eq!(
        lines[controller_positions[0]]["controller"], result["cache"]["controller"],
        "controller envelope and terminal result diverged"
    );
    result
}

mod background;
mod flow;
mod output;
