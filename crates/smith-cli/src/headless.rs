//! Non-interactive execution and versioned stdout contracts.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::time::Duration;

use agent_runtime_core::artifact::ArtifactRef;
use agent_runtime_core::content::{Role, UserInput};
use agent_runtime_core::goal::{GoalProjection, GoalStatus};
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::interaction::InteractionOutcomeKind;
use agent_runtime_core::provider::ProviderAttemptPurpose;
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::usage::{UsageDelta, UsageRecord};
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use smith_config::model::BackgroundExit;
use smith_host::{
    ApprovalRequired, HeadlessApproval, HeadlessInteraction, HeadlessRotation, InteractionRequired,
};
use smith_runtime::background_tasks::{BackgroundTaskInfo, BackgroundTaskRegistry, TaskStatus};
use smith_runtime::cache_controller::CacheControllerSnapshot;
use smith_runtime::client::{
    EstimationConfidence, PlanItemProjection, PlanSensitivity, SmithEvent as EventEnvelope,
    SmithEventKind as RuntimeEvent, TurnFinish,
};
use smith_runtime::host::HostSession;
use smith_runtime::journal::{EphemeralInterruptionReason, EphemeralWorkInterruption};
use smith_runtime::rotation::SharedPool;
use smith_runtime::{ChildDurability, ChildState};
use smith_tui::cache::{
    CacheLifecycleSummary, CachePrice, CacheProjection, CacheTurnSummary, CacheVisibilityState,
};

use crate::cli::OutputFormat;

mod background;
mod output;
mod run_flow;

use background::*;
use output::*;
use run_flow::*;

/// Version of Smith's result/event wrappers, independent of runtime events.
const OUTPUT_SCHEMA_VERSION: u32 = 3;

/// Stable process status used when an unattended call needs authorization.
pub(crate) const APPROVAL_REQUIRED_EXIT: u8 = 4;
/// Stable process status used when an unattended run needs task input.
pub(crate) const INTERACTION_REQUIRED_EXIT: u8 = 5;

/// How often `wait`/`stop` re-poll the registry for a terminal state.
const BACKGROUND_TASK_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Ceiling on how long `stop` waits for tasks to acknowledge the stop signal.
/// Generous relative to the worker's own ~500 ms kill grace period: this
/// bounds the headless exit, not the kill itself.
const BACKGROUND_STOP_POLL_BOUND: Duration = Duration::from_secs(5);

/// The result of presenting one non-interactive run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outcome {
    /// Stable process exit code.
    pub exit_code: u8,
}

/// The out-of-band brokers a headless turn consults when the runtime asks for
/// something stdin would normally supply. Each is absent when the surface was
/// started without that capability; a headless turn never blocks on one.
#[derive(Default, Clone, Copy)]
pub(crate) struct HeadlessBrokers<'a> {
    pub(crate) approval: Option<&'a HeadlessApproval>,
    pub(crate) interaction: Option<&'a HeadlessInteraction>,
    pub(crate) rotation: Option<&'a HeadlessRotation>,
    pub(crate) credential_pool: Option<&'a SharedPool>,
    /// The exact active-model price reference, when the catalog supplies one.
    pub(crate) cache_price: Option<CachePrice>,
    /// Layered local notice policy.
    pub(crate) cache_miss_notices: bool,
}

/// Runs one turn, preserving canonical event order for stream JSON.
pub(crate) async fn run(
    host: &HostSession,
    prompt: String,
    format: OutputFormat,
    brokers: HeadlessBrokers<'_>,
    background_exit: BackgroundExit,
) -> Result<Outcome> {
    let stdout = io::stdout();
    let stderr = io::stderr();
    run_with_io(
        host,
        prompt,
        format,
        brokers,
        background_exit,
        &mut stdout.lock(),
        &mut stderr.lock(),
    )
    .await
}

#[cfg(test)]
mod tests;
