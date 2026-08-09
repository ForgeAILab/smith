//! Non-interactive execution and versioned stdout contracts.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::time::Duration;

use agent_runtime_core::artifact::ArtifactRef;
use agent_runtime_core::content::{Role, UserInput};
use agent_runtime_core::event::{
    EstimationConfidence, EventEnvelope, PlanItemProjection, PlanSensitivity, RuntimeEvent,
    TurnFinish,
};
use agent_runtime_core::goal::{GoalProjection, GoalStatus};
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::interaction::InteractionOutcomeKind;
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::usage::UsageDelta;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use smith_config::model::BackgroundExit;
use smith_host::{
    ApprovalRequired, HeadlessApproval, HeadlessInteraction, HeadlessRotation, InteractionRequired,
};
use smith_runtime::background_tasks::{BackgroundTaskInfo, BackgroundTaskRegistry, TaskStatus};
use smith_runtime::host::HostSession;
use smith_runtime::journal::{EphemeralInterruptionReason, EphemeralWorkInterruption};
use smith_runtime::rotation::SharedPool;
use smith_runtime::{ChildDurability, ChildState};
use smith_tui::cache::{CachePrice, CacheProjection, CacheTurnSummary, CacheVisibilityState};

use crate::cli::OutputFormat;

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

#[derive(Debug, Serialize)]
struct StreamEnvelope<'a> {
    schema_version: u32,
    #[serde(rename = "type")]
    kind: &'static str,
    event: &'a EventEnvelope,
}

#[derive(Debug, Serialize)]
struct ResultEnvelope {
    schema_version: u32,
    #[serde(rename = "type")]
    kind: &'static str,
    status: ResultStatus,
    session_id: String,
    turn_id: String,
    provider: String,
    model: String,
    output: String,
    usage: UsageOutput,
    lifecycle: LifecycleOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    goal: Option<GoalProjection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    goal_continuation_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    artifacts: Vec<ArtifactRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_required: Option<ApprovalOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interaction_required: Option<InteractionOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryOutput>,
    /// The credential-pool account this run used, when the provider declares
    /// a pool. A headless run keeps one account start to finish, so this names
    /// what the whole run was billed to.
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<AccountOutput>,
    /// What the background-exit policy did about background shell tasks that
    /// were still running when the final answer was ready. Absent when none
    /// were running, regardless of policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    background_exit: Option<BackgroundExitOutput>,
    /// The effective reasoning selection: what this run actually asked of the
    /// provider, so a caller reading token counts can tell "max was
    /// requested" apart from "the provider default applied".
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningOutput>,
    /// The final turn's last cache plan, summarizing provider prefix reuse.
    #[serde(skip_serializing_if = "Option::is_none")]
    cache: Option<CacheOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Redaction-safe projection of the run's reasoning selection.
#[derive(Debug, Serialize)]
struct ReasoningOutput {
    /// "on", "off", or "provider default".
    state: &'static str,
    /// Selected or documented effort, or "provider default".
    effort: String,
    /// Bounded description of where the selection came from.
    source: String,
}

impl ReasoningOutput {
    fn of(policy: &smith_runtime::reasoning::ReasoningRuntimePolicy) -> Self {
        Self {
            state: policy.effective_state(),
            effort: policy.effective_effort().to_owned(),
            source: policy.selection_source.clone(),
        }
    }
}

/// The last cache plan the runtime emitted for the reported turn.
#[derive(Debug, Serialize)]
struct CacheOutput {
    /// Whether the provider can reuse the plan's stable prefix.
    provider_cache_supported: bool,
    /// Tokens of prefix carried over from the previous attempt's plan.
    preserved_prefix_tokens: u32,
    /// Tokens at or after the first changed segment.
    invalidated_prefix_tokens: u32,
    /// Aggregate canonical state for the final root turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<CacheVisibilityState>,
    /// Canonical expectation and provider observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_write_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    missed_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<EstimationConfidence>,
    /// Latest completed root-turn provider cache-read share.
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read_percent: Option<u8>,
    /// Derived retry diagnostics, separate from `usage`.
    #[serde(skip_serializing_if = "Option::is_none")]
    miss_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rebilled_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idle_minutes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra_cost_micro_usd: Option<u128>,
    #[serde(skip)]
    notice: Option<String>,
}

impl CacheOutput {
    fn from_summary(summary: &CacheTurnSummary, prior: Option<Self>) -> Self {
        let prior = prior.unwrap_or(Self {
            provider_cache_supported: summary.state != CacheVisibilityState::Unsupported,
            preserved_prefix_tokens: 0,
            invalidated_prefix_tokens: 0,
            state: None,
            expected_read_tokens: None,
            observed_read_tokens: None,
            observed_write_tokens: None,
            missed_tokens: None,
            confidence: None,
            cache_read_percent: None,
            miss_count: None,
            rebilled_tokens: None,
            idle_minutes: None,
            extra_cost_micro_usd: None,
            notice: None,
        });
        Self {
            state: Some(summary.state),
            expected_read_tokens: summary.expected_read_tokens,
            observed_read_tokens: summary.observed_read_tokens,
            observed_write_tokens: summary.observed_write_tokens,
            missed_tokens: summary.missed_tokens,
            confidence: summary.confidence,
            cache_read_percent: summary.cache_read_percent,
            miss_count: Some(summary.miss_count),
            rebilled_tokens: Some(summary.rebilled_tokens),
            idle_minutes: summary.idle_minutes,
            extra_cost_micro_usd: summary.extra_cost_micro_usd,
            notice: summary.significant().then(|| summary.render_notice()),
            ..prior
        }
    }
}

/// One background shell task's state as the background-exit policy last
/// observed it.
#[derive(Debug, Serialize)]
struct BackgroundTaskOutput {
    task_id: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
}

impl BackgroundTaskOutput {
    /// Projects a task the `error` policy reported without waiting on: no
    /// poll happened, so "running" is the only state actually observed.
    fn still_running(task: &BackgroundTaskInfo) -> Self {
        Self {
            task_id: task.task_id.clone(),
            status: TaskStatus::Running.as_str(),
            exit_code: None,
        }
    }

    fn terminal(task_id: String, status: &TaskStatus) -> Self {
        Self {
            task_id,
            status: status.as_str(),
            exit_code: status.exit_code(),
        }
    }
}

/// Report of every background shell task the caller's background-exit policy
/// acted on, and which policy applied.
#[derive(Debug, Serialize)]
struct BackgroundExitOutput {
    policy: &'static str,
    tasks: Vec<BackgroundTaskOutput>,
}

/// The account a headless run used, and whether its window ran out.
#[derive(Debug, Serialize)]
struct AccountOutput {
    /// Zero-based position in the declared pool.
    position: usize,
    /// The credential reference, never its value.
    reference: String,
    /// Server-reported consumption, absent when nothing measured it.
    #[serde(skip_serializing_if = "Option::is_none")]
    used_percent: Option<f64>,
    /// Whether the run ended because this account's window was spent.
    exhausted: bool,
    /// When the spent window reopens, in Unix milliseconds, if reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    resets_at_ms: Option<u64>,
    /// How many other accounts were declared but deliberately not used.
    ///
    /// A headless run never rotates, so this is the number of accounts a
    /// script could have fallen back to had it been interactive.
    unused_members: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ResultStatus {
    Ok,
    ApprovalRequired,
    InteractionRequired,
    Failed,
    Cancelled,
    LimitReached,
}

#[derive(Debug, Serialize)]
struct UsageOutput {
    current_turn: UsageDelta,
    session: UsageDelta,
    current_turn_provenance: UsageProvenance,
    session_provenance: UsageProvenance,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum UsageProvenance {
    ProviderReported,
    Unknown,
}

impl UsageProvenance {
    fn of(delta: &UsageDelta) -> Self {
        if delta.is_empty() {
            Self::Unknown
        } else {
            Self::ProviderReported
        }
    }
}

#[derive(Debug, Serialize)]
struct ApprovalOutput {
    call_id: String,
    tool: String,
    argument_keys: Vec<String>,
    mutates: bool,
    requires_authorization: bool,
    permissions: Vec<String>,
    resource: SecurityResource,
    authority_warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deadline_at_ms: Option<u64>,
    preparation_fingerprint: String,
}

#[derive(Debug, Default, Serialize)]
struct LifecycleOutput {
    attempts_committed: u32,
    attempts_discarded: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    activation: Option<ActivationOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<PlanOutput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<ChildSessionOutput>,
}

#[derive(Debug, Serialize)]
struct ChildSessionOutput {
    child_id: String,
    child_session_id: String,
    durability: &'static str,
    state: &'static str,
    resumable: bool,
    turns_used: u32,
    /// Absent for an unbounded child.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_turns: Option<u32>,
    tokens_used: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    incompatibility: Option<String>,
}

#[derive(Debug, Serialize)]
struct ActivationOutput {
    epoch: u64,
    capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PlanOutput {
    revision: u64,
    sensitivity: PlanSensitivity,
    counts: BTreeMap<String, u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    items: Option<Vec<PlanItemProjection>>,
}

#[derive(Debug, Serialize)]
struct InteractionOutput {
    request_id: String,
    question_count: usize,
}

#[derive(Debug, Serialize)]
struct RecoveryOutput {
    reason: &'static str,
    interrupted_children: Vec<String>,
    interrupted_monitors: Vec<String>,
    interrupted_tasks: Vec<String>,
}

impl From<&EphemeralWorkInterruption> for RecoveryOutput {
    fn from(interruption: &EphemeralWorkInterruption) -> Self {
        let reason = match interruption.reason {
            EphemeralInterruptionReason::ProcessExit => "process_exit",
        };
        Self {
            reason,
            interrupted_children: interruption
                .children
                .iter()
                .map(ToString::to_string)
                .collect(),
            interrupted_monitors: interruption.monitors.clone(),
            interrupted_tasks: interruption.tasks.clone(),
        }
    }
}

impl From<InteractionRequired> for InteractionOutput {
    fn from(required: InteractionRequired) -> Self {
        Self {
            request_id: required.request_id,
            question_count: required.question_count,
        }
    }
}

impl From<ApprovalRequired> for ApprovalOutput {
    fn from(required: ApprovalRequired) -> Self {
        Self {
            call_id: required.call_id,
            tool: required.tool,
            argument_keys: required.argument_keys,
            mutates: required.mutates,
            requires_authorization: required.requires_authorization,
            permissions: required.permissions,
            resource: required.resource,
            authority_warnings: required.authority_warnings,
            deadline_at_ms: required.deadline_at_ms,
            preparation_fingerprint: required.preparation_fingerprint,
        }
    }
}

fn child_session_outputs(host: &HostSession) -> Vec<ChildSessionOutput> {
    host.runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .map(|coordinator| {
            coordinator
                .list()
                .into_iter()
                .map(|status| ChildSessionOutput {
                    child_id: status.child.to_string(),
                    child_session_id: status.session.to_string(),
                    durability: match status.durability {
                        ChildDurability::Ephemeral => "ephemeral",
                        ChildDurability::Durable => "durable",
                    },
                    state: match &status.state {
                        ChildState::Running => "running",
                        ChildState::Idle => "idle",
                        ChildState::Interrupted { .. } => "interrupted",
                        ChildState::Stopped { .. } => "stopped",
                        ChildState::Failed => "failed",
                        ChildState::Expired => "expired",
                    },
                    resumable: status.resumable(),
                    turns_used: status.turns_used,
                    max_turns: (status.max_turns != u32::MAX).then_some(status.max_turns),
                    tokens_used: status.tokens_used,
                    incompatibility: status.incompatibility,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Projects the account a headless run used.
///
/// A headless run selects its account once at session start and keeps it, so
/// this is a property of the whole run rather than of a moment in it. Absent
/// without a pool: a single-credential provider has no account to choose.
fn account_output(
    credential_pool: Option<&SharedPool>,
    rotation: Option<&HeadlessRotation>,
) -> Option<AccountOutput> {
    let pool = credential_pool?;
    let required = rotation.and_then(HeadlessRotation::required);
    let now_ms = smith_tui::accounts::now_ms();
    pool.read(|pool| {
        let active = pool.active()?;
        Some(AccountOutput {
            position: active.position,
            reference: active.reference.clone(),
            used_percent: pool.used_percent(active.position),
            exhausted: required.is_some(),
            resets_at_ms: required
                .as_ref()
                .and_then(|required| required.resets_at_ms)
                .or_else(|| pool.cooling_until(active.position, now_ms)),
            unused_members: pool.members().len().saturating_sub(1),
        })
    })
}

/// What a background-exit policy requires when the final answer is ready but
/// background shell tasks are still running. Pure and synchronous so the
/// policy choice is unit-testable without a live task registry; the async
/// waiting/stopping itself lives in [`apply_background_exit_policy`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum BackgroundExitDecision {
    /// No running background shell tasks; nothing to do.
    Clear,
    /// `error` (the default): report every running task and fail rather than
    /// let `host.shutdown()` kill them without anyone having been told.
    Error(String),
    /// `wait`: block until every running task reaches a terminal state.
    Wait,
    /// `stop`: signal every running task to stop, then await termination.
    Stop,
}

fn decide_background_exit(
    policy: BackgroundExit,
    running: &[BackgroundTaskInfo],
) -> BackgroundExitDecision {
    if running.is_empty() {
        return BackgroundExitDecision::Clear;
    }
    match policy {
        BackgroundExit::Error => BackgroundExitDecision::Error(background_task_error(running)),
        BackgroundExit::Wait => BackgroundExitDecision::Wait,
        BackgroundExit::Stop => BackgroundExitDecision::Stop,
    }
}

/// Names every running task by ID and command so the caller can act on it
/// instead of guessing what `host.shutdown()` is about to kill.
fn background_task_error(running: &[BackgroundTaskInfo]) -> String {
    let tasks = running
        .iter()
        .map(|task| format!("{} (`{}`)", task.task_id, task.command))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{} background shell task(s) still running at exit: {tasks}; rerun with \
         `--background-exit wait` to await them, `--background-exit stop` to end them, \
         or call `task_stop` before the turn ends",
        running.len()
    )
}

/// Applies the background-exit policy to whatever background shell tasks are
/// still running when the final answer is ready.
///
/// `host.shutdown()` kills every registered group regardless of policy — true
/// orphaning is not possible — so this only decides what gets reported and
/// how long the process waits before that happens.
async fn apply_background_exit_policy(
    session_id: &SessionId,
    policy: BackgroundExit,
) -> (Option<String>, Option<BackgroundExitOutput>) {
    let running = BackgroundTaskRegistry::global().running_tasks(session_id);
    match decide_background_exit(policy, &running) {
        BackgroundExitDecision::Clear => (None, None),
        BackgroundExitDecision::Error(message) => {
            let tasks = running
                .iter()
                .map(BackgroundTaskOutput::still_running)
                .collect();
            (
                Some(message),
                Some(BackgroundExitOutput {
                    policy: policy.as_str(),
                    tasks,
                }),
            )
        }
        BackgroundExitDecision::Wait => {
            let tasks = await_background_tasks(session_id, &running, None).await;
            (
                None,
                Some(BackgroundExitOutput {
                    policy: policy.as_str(),
                    tasks,
                }),
            )
        }
        BackgroundExitDecision::Stop => {
            BackgroundTaskRegistry::global()
                .stop_all_session_tasks(session_id, TaskStatus::Stopped);
            let tasks =
                await_background_tasks(session_id, &running, Some(BACKGROUND_STOP_POLL_BOUND))
                    .await;
            (
                None,
                Some(BackgroundExitOutput {
                    policy: policy.as_str(),
                    tasks,
                }),
            )
        }
    }
}

/// Polls the registry until every named task leaves the running set — or,
/// under `bound`, until that ceiling passes — then reads back each task's
/// terminal state. `wait` passes no bound: a background task may legitimately
/// run as long as the model let it; only `stop` needs a ceiling, since its
/// signal should resolve within the worker's own kill grace period.
async fn await_background_tasks(
    session_id: &SessionId,
    running: &[BackgroundTaskInfo],
    bound: Option<Duration>,
) -> Vec<BackgroundTaskOutput> {
    let registry = BackgroundTaskRegistry::global();
    let poll_until_terminal = async {
        loop {
            let running_ids: BTreeSet<String> = registry
                .running_tasks(session_id)
                .into_iter()
                .map(|task| task.task_id)
                .collect();
            if !running
                .iter()
                .any(|task| running_ids.contains(&task.task_id))
            {
                return;
            }
            tokio::time::sleep(BACKGROUND_TASK_POLL_INTERVAL).await;
        }
    };
    match bound {
        Some(bound) => {
            let _ = tokio::time::timeout(bound, poll_until_terminal).await;
        }
        None => poll_until_terminal.await,
    }

    let mut report = Vec::with_capacity(running.len());
    for task in running {
        let status = registry
            .get_task_output(session_id, &task.task_id, 0, 1)
            .await
            .map(|result| result.status)
            .unwrap_or_else(|_| task.status.clone());
        report.push(BackgroundTaskOutput::terminal(
            task.task_id.clone(),
            &status,
        ));
    }
    report
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

async fn run_with_io(
    host: &HostSession,
    prompt: String,
    format: OutputFormat,
    brokers: HeadlessBrokers<'_>,
    background_exit: BackgroundExit,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<Outcome> {
    let HeadlessBrokers {
        approval,
        interaction,
        rotation,
        credential_pool,
        cache_price,
        cache_miss_notices,
    } = brokers;
    if let Some(restored) = host.restored_interaction() {
        let required = interaction
            .and_then(HeadlessInteraction::required)
            .filter(|required| required.request_id == restored.request_id().as_str())
            .unwrap_or_else(|| InteractionRequired {
                request_id: restored.request_id().as_str().to_owned(),
                question_count: restored.question_count(),
            });
        return write_restored_interaction_required(host, format, required, stdout, stderr).await;
    }

    let session = host.session();
    let mut events = session.subscribe();
    let mut cache_projection = CacheProjection::default();
    if let Ok(history) = host.timeline_events().await {
        cache_projection.replay(history);
    }
    let initial_activation = activation_output(session);
    let history_start = session.history().len();
    let turn = match session.send(UserInput::text(prompt)) {
        Ok(turn) => turn,
        Err(error) => {
            return write_submission_failure(host, format, stdout, stderr, error.to_string()).await;
        }
    };
    let turn_id = turn.id().clone();
    let mut finish = None;
    let mut turn_usage = UsageDelta::new();
    let mut cache: Option<CacheOutput> = None;
    let mut last_error = None;
    let mut last_sequence = None;
    let mut sequence_error = None;
    let mut pending_interaction: Option<InteractionRequired> = None;
    let mut event_interaction_required: Option<InteractionRequired> = None;
    let mut lifecycle = LifecycleOutput {
        activation: initial_activation,
        ..LifecycleOutput::default()
    };
    let mut goal_continuation_turns = 0_u32;
    let mut active_goal_turns = BTreeSet::new();

    while let Some(event) = events.next().await {
        cache_projection.apply(&event);
        observe_sequence(&mut last_sequence, event.seq, &mut sequence_error);
        if matches!(
            &event.payload,
            RuntimeEvent::InternalTurnStarted { source } if source.kind == "goal"
        ) {
            goal_continuation_turns = goal_continuation_turns.saturating_add(1);
            if let Some(turn) = &event.turn {
                active_goal_turns.insert(turn.as_str().to_owned());
            }
        }
        let belongs_to_turn = event.turn.as_ref() == Some(&turn_id);
        let belongs_to_goal_turn = event
            .turn
            .as_ref()
            .is_some_and(|turn| active_goal_turns.contains(turn.as_str()));
        if belongs_to_turn {
            match &event.payload {
                RuntimeEvent::Usage { record } => turn_usage.merge(&record.delta),
                RuntimeEvent::CachePlanChanged {
                    preserved_prefix_tokens,
                    invalidated_prefix_tokens,
                    provider_cache_supported,
                    ..
                } => {
                    cache = Some(CacheOutput {
                        provider_cache_supported: *provider_cache_supported,
                        preserved_prefix_tokens: *preserved_prefix_tokens,
                        invalidated_prefix_tokens: *invalidated_prefix_tokens,
                        state: None,
                        expected_read_tokens: None,
                        observed_read_tokens: None,
                        observed_write_tokens: None,
                        missed_tokens: None,
                        confidence: None,
                        cache_read_percent: None,
                        miss_count: None,
                        rebilled_tokens: None,
                        idle_minutes: None,
                        extra_cost_micro_usd: None,
                        notice: None,
                    });
                }
                RuntimeEvent::Error { error } => last_error = Some(error.to_string()),
                RuntimeEvent::ProviderAttemptOutputCommitted { .. } => {
                    lifecycle.attempts_committed = lifecycle.attempts_committed.saturating_add(1);
                }
                RuntimeEvent::ProviderAttemptOutputDiscarded { .. } => {
                    lifecycle.attempts_discarded = lifecycle.attempts_discarded.saturating_add(1);
                }
                RuntimeEvent::CapabilitiesActivated { epoch, activation } => {
                    lifecycle.activation = Some(ActivationOutput {
                        epoch: u64::from(*epoch),
                        capabilities: activation
                            .iter()
                            .map(|capability| capability.id.to_string())
                            .collect(),
                    });
                }
                RuntimeEvent::PlanUpdated {
                    revision,
                    sensitivity,
                    counts,
                    items,
                } => {
                    lifecycle.plan = Some(plan_output(
                        *revision,
                        *sensitivity,
                        counts.clone(),
                        items.clone(),
                    ));
                }
                RuntimeEvent::TurnCompleted {
                    finish: completed, ..
                } => {
                    if let TurnFinish::NeedsInput { request } = completed {
                        let required = pending_interaction
                            .take()
                            .filter(|pending| pending.request_id == request.as_str())
                            .unwrap_or_else(|| InteractionRequired {
                                request_id: request.as_str().to_owned(),
                                question_count: 0,
                            });
                        event_interaction_required.get_or_insert(required);
                    }
                    finish = Some(completed.clone());
                }
                RuntimeEvent::InteractionRequested {
                    request,
                    question_count,
                    ..
                } => {
                    pending_interaction = Some(InteractionRequired {
                        request_id: request.as_str().to_owned(),
                        question_count: usize::from(*question_count),
                    });
                }
                RuntimeEvent::InteractionResolved {
                    request,
                    outcome: InteractionOutcomeKind::Unavailable,
                    ..
                } => {
                    if let Some(required) = pending_interaction.take()
                        && required.request_id == request.as_str()
                    {
                        event_interaction_required.get_or_insert(required);
                    }
                }
                _ => {}
            }
        }
        if belongs_to_goal_turn {
            match &event.payload {
                RuntimeEvent::Error { error } => last_error = Some(error.to_string()),
                RuntimeEvent::InteractionRequested {
                    request,
                    question_count,
                    ..
                } => {
                    pending_interaction = Some(InteractionRequired {
                        request_id: request.as_str().to_owned(),
                        question_count: usize::from(*question_count),
                    });
                }
                RuntimeEvent::InteractionResolved {
                    request,
                    outcome: InteractionOutcomeKind::Unavailable,
                    ..
                } => {
                    if let Some(required) = pending_interaction.take()
                        && required.request_id == request.as_str()
                    {
                        event_interaction_required.get_or_insert(required);
                    }
                }
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::NeedsInput { request },
                    ..
                } => {
                    let required = pending_interaction
                        .take()
                        .filter(|pending| pending.request_id == request.as_str())
                        .unwrap_or_else(|| InteractionRequired {
                            request_id: request.as_str().to_owned(),
                            question_count: 0,
                        });
                    event_interaction_required.get_or_insert(required);
                }
                _ => {}
            }
        }

        if format == OutputFormat::StreamJson
            && let Err(error) = write_json(
                stdout,
                &StreamEnvelope {
                    schema_version: OUTPUT_SCHEMA_VERSION,
                    kind: "runtime_event",
                    event: &event,
                },
            )
        {
            let _ = host.shutdown().await;
            return Err(error);
        }
        if belongs_to_goal_turn
            && matches!(event.payload, RuntimeEvent::TurnCompleted { .. })
            && let Some(turn) = &event.turn
        {
            active_goal_turns.remove(turn.as_str());
        }
        if finish.is_some() {
            match host.goal() {
                Ok(Some(goal)) if goal.status == GoalStatus::Active => {}
                Ok(_) if active_goal_turns.is_empty() => break,
                Ok(_) => {}
                Err(error) => {
                    sequence_error.get_or_insert_with(|| {
                        format!("persistent goal state became unavailable: {error}")
                    });
                    break;
                }
            }
        }
    }

    let stream_error = finish
        .is_none()
        .then(|| "the runtime event stream ended before the turn completed".to_owned());

    lifecycle.children = child_session_outputs(host);

    let final_goal = match host.goal() {
        Ok(goal) => goal,
        Err(error) => {
            sequence_error
                .get_or_insert_with(|| format!("persistent goal state unavailable: {error}"));
            None
        }
    };
    let goal_continuation_turns = final_goal.as_ref().map(|_| goal_continuation_turns);

    let (background_exit_error, background_exit_output) =
        apply_background_exit_policy(session.id(), background_exit).await;

    let shutdown_error = host.shutdown().await.err().map(|error| error.to_string());

    // SessionShutdown is queued before shutdown returns. Include it in the
    // stream without risking an indefinite wait if a future runtime changes
    // that lifecycle detail.
    if format == OutputFormat::StreamJson {
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(100), events.next()).await
        {
            observe_sequence(&mut last_sequence, event.seq, &mut sequence_error);
            let terminal = matches!(event.payload, RuntimeEvent::SessionShutdown);
            write_json(
                stdout,
                &StreamEnvelope {
                    schema_version: OUTPUT_SCHEMA_VERSION,
                    kind: "runtime_event",
                    event: &event,
                },
            )?;
            if terminal {
                break;
            }
        }
    }

    let snapshot = session.snapshot();
    let artifacts = session.artifacts_for_turn(&turn_id);
    let output = snapshot
        .history
        .iter()
        .skip(history_start)
        .rev()
        .find(|message| message.role == Role::Assistant && !message.joined_text().is_empty())
        .map(|message| message.joined_text())
        .unwrap_or_default();
    let session_usage = snapshot.usage.total();
    if let Some(summary) = cache_projection.completed_turn(turn_id.as_str()) {
        let summary = cache_price
            .map(|price| cache_projection.with_price(summary, price))
            .unwrap_or_else(|| summary.clone());
        cache = Some(CacheOutput::from_summary(&summary, cache));
    }
    let approval_required = approval.and_then(HeadlessApproval::required);
    let interaction_required = interaction
        .and_then(HeadlessInteraction::required)
        .or(event_interaction_required);
    let lifecycle_error = shutdown_error.or(stream_error).or(sequence_error);
    let error = background_exit_error.or(lifecycle_error).or_else(|| {
        matches!(finish, Some(TurnFinish::Failed))
            .then_some(last_error)
            .flatten()
    });
    let (status, exit_code) = outcome(
        finish.as_ref(),
        final_goal.as_ref(),
        approval_required.as_ref(),
        interaction_required.as_ref(),
        error.as_ref(),
    );

    let result = ResultEnvelope {
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status,
        session_id: session.id().as_str().to_owned(),
        turn_id: turn_id.as_str().to_owned(),
        provider: host.runtime().policy().provider_name.clone(),
        model: host.runtime().policy().model.as_str().to_owned(),
        output: output.clone(),
        usage: UsageOutput {
            current_turn_provenance: UsageProvenance::of(&turn_usage),
            session_provenance: UsageProvenance::of(&session_usage),
            current_turn: turn_usage,
            session: session_usage,
        },
        lifecycle,
        goal: final_goal,
        goal_continuation_turns,
        artifacts,
        account: account_output(credential_pool, rotation),
        approval_required: approval_required.map(Into::into),
        interaction_required: interaction_required.map(Into::into),
        recovery: host.recovered_ephemeral_work().map(Into::into),
        background_exit: background_exit_output,
        reasoning: Some(ReasoningOutput::of(&host.runtime().policy().reasoning)),
        cache,
        error: error.clone(),
    };

    match format {
        OutputFormat::Text if exit_code == 0 => {
            write_text(stdout, &output)?;
            write_text_projection(stderr, &result)?;
            if cache_miss_notices
                && let Some(notice) = result
                    .cache
                    .as_ref()
                    .and_then(|cache| cache.notice.as_ref())
            {
                writeln!(stderr, "smith: {notice}").context("writing cache notice")?;
                stderr.flush().context("flushing cache notice")?;
            }
        }
        OutputFormat::Text => {
            let diagnostic = match (
                &result.approval_required,
                &result.interaction_required,
                error,
            ) {
                (Some(required), _, _) => approval_diagnostic(required),
                (_, Some(required), _) => format!(
                    "interaction required for request `{}` ({} question(s)); \
                     rerun in an interactive terminal",
                    required.request_id, required.question_count
                ),
                (_, _, Some(error)) => error,
                _ => format!("turn ended with status {:?}", result.status),
            };
            write_text_projection(stderr, &result)?;
            if cache_miss_notices
                && let Some(notice) = result
                    .cache
                    .as_ref()
                    .and_then(|cache| cache.notice.as_ref())
            {
                writeln!(stderr, "smith: {notice}").context("writing cache notice")?;
            }
            writeln!(stderr, "smith: {diagnostic}").context("writing diagnostic to stderr")?;
            stderr.flush().context("flushing diagnostic stderr")?;
        }
        OutputFormat::Json | OutputFormat::StreamJson => write_json(stdout, &result)?,
    }

    Ok(Outcome { exit_code })
}

async fn write_restored_interaction_required(
    host: &HostSession,
    format: OutputFormat,
    required: InteractionRequired,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<Outcome> {
    let restored = host
        .restored_interaction()
        .expect("restored interaction checked before rendering");
    let session = host.session();
    let session_id = session.id().as_str().to_owned();
    let turn_id = restored.turn_id().as_str().to_owned();
    let session_usage = session.snapshot().usage.total();
    let final_goal = host.goal().ok().flatten();
    let goal_continuation_turns = final_goal.as_ref().map(|_| 0);
    let shutdown_error = host.shutdown().await.err().map(|error| error.to_string());
    let result = ResultEnvelope {
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status: ResultStatus::InteractionRequired,
        session_id,
        turn_id,
        provider: host.runtime().policy().provider_name.clone(),
        model: host.runtime().policy().model.as_str().to_owned(),
        output: String::new(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session_provenance: UsageProvenance::of(&session_usage),
            current_turn_provenance: UsageProvenance::Unknown,
            session: session_usage,
        },
        lifecycle: LifecycleOutput {
            activation: activation_output(session),
            children: child_session_outputs(host),
            ..LifecycleOutput::default()
        },
        goal: final_goal,
        goal_continuation_turns,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: Some(required.into()),
        recovery: host.recovered_ephemeral_work().map(Into::into),
        account: None,
        background_exit: None,
        reasoning: Some(ReasoningOutput::of(&host.runtime().policy().reasoning)),
        cache: None,
        error: shutdown_error,
    };

    match format {
        OutputFormat::Text => {
            write_text_projection(stderr, &result)?;
            let required = result
                .interaction_required
                .as_ref()
                .expect("interaction-required result metadata");
            writeln!(
                stderr,
                "smith: interaction required for request `{}` ({} question(s)); \
                 rerun in an interactive terminal",
                required.request_id, required.question_count
            )
            .context("writing diagnostic to stderr")?;
            stderr.flush().context("flushing diagnostic stderr")?;
        }
        OutputFormat::Json | OutputFormat::StreamJson => write_json(stdout, &result)?,
    }

    Ok(Outcome {
        exit_code: INTERACTION_REQUIRED_EXIT,
    })
}

async fn write_submission_failure(
    host: &HostSession,
    format: OutputFormat,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    submission_error: String,
) -> Result<Outcome> {
    let session = host.session();
    let snapshot = session.snapshot();
    let session_usage = snapshot.usage.total();
    let final_goal = host.goal().ok().flatten();
    let goal_continuation_turns = final_goal.as_ref().map(|_| 0);
    let error = match host.shutdown().await {
        Ok(_) => submission_error,
        Err(shutdown) => format!("{submission_error}; shutdown also failed: {shutdown}"),
    };
    let result = ResultEnvelope {
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status: ResultStatus::Failed,
        session_id: session.id().as_str().to_owned(),
        // Submission was rejected before the runtime minted an accepted turn
        // handle. The versioned schema retains its required string field and
        // uses the empty value to state that no turn exists.
        turn_id: String::new(),
        provider: host.runtime().policy().provider_name.clone(),
        model: host.runtime().policy().model.as_str().to_owned(),
        output: String::new(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session_provenance: UsageProvenance::of(&session_usage),
            current_turn_provenance: UsageProvenance::Unknown,
            session: session_usage,
        },
        lifecycle: LifecycleOutput {
            activation: activation_output(session),
            children: child_session_outputs(host),
            ..LifecycleOutput::default()
        },
        goal: final_goal,
        goal_continuation_turns,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: None,
        recovery: host.recovered_ephemeral_work().map(Into::into),
        account: None,
        background_exit: None,
        reasoning: Some(ReasoningOutput::of(&host.runtime().policy().reasoning)),
        cache: None,
        error: Some(error.clone()),
    };

    match format {
        OutputFormat::Text => {
            write_text_projection(stderr, &result)?;
            writeln!(stderr, "smith: {error}").context("writing diagnostic to stderr")?;
            stderr.flush().context("flushing diagnostic stderr")?;
        }
        OutputFormat::Json | OutputFormat::StreamJson => write_json(stdout, &result)?,
    }
    Ok(Outcome { exit_code: 1 })
}

fn observe_sequence(last: &mut Option<u64>, current: u64, error: &mut Option<String>) {
    if let Some(previous) = *last
        && current != previous.saturating_add(1)
        && error.is_none()
    {
        *error = Some(format!(
            "runtime event stream lost events between sequence {previous} and {current}"
        ));
    }
    *last = Some(current);
}

fn activation_output(session: &smith_runtime::SessionHandle) -> Option<ActivationOutput> {
    session.activation_epoch().map(|epoch| ActivationOutput {
        epoch: epoch.index(),
        capabilities: epoch
            .activated()
            .iter()
            .map(|(id, _)| id.to_string())
            .collect(),
    })
}

fn plan_output(
    revision: u64,
    sensitivity: PlanSensitivity,
    counts: BTreeMap<String, u32>,
    items: Option<Vec<PlanItemProjection>>,
) -> PlanOutput {
    PlanOutput {
        revision,
        sensitivity,
        counts,
        items: if sensitivity == PlanSensitivity::Public {
            items
        } else {
            None
        },
    }
}

fn outcome(
    finish: Option<&TurnFinish>,
    goal: Option<&GoalProjection>,
    approval: Option<&ApprovalRequired>,
    interaction: Option<&InteractionRequired>,
    error: Option<&String>,
) -> (ResultStatus, u8) {
    if interaction.is_some() {
        return (ResultStatus::InteractionRequired, INTERACTION_REQUIRED_EXIT);
    }
    if approval.is_some() {
        return (ResultStatus::ApprovalRequired, APPROVAL_REQUIRED_EXIT);
    }
    if error.is_some() {
        return (ResultStatus::Failed, 1);
    }
    if let Some(goal) = goal {
        match goal.status {
            GoalStatus::Active => return (ResultStatus::Failed, 1),
            GoalStatus::Paused => return (ResultStatus::Cancelled, 1),
            GoalStatus::Blocked => return (ResultStatus::Failed, 1),
            GoalStatus::UsageLimited | GoalStatus::BudgetLimited => {
                return (ResultStatus::LimitReached, 1);
            }
            GoalStatus::Complete => {}
        }
    }
    match finish {
        Some(TurnFinish::Completed) => (ResultStatus::Ok, 0),
        Some(TurnFinish::Cancelled { .. }) => (ResultStatus::Cancelled, 1),
        Some(TurnFinish::LimitReached { .. }) => (ResultStatus::LimitReached, 1),
        Some(TurnFinish::NeedsInput { .. }) => {
            (ResultStatus::InteractionRequired, INTERACTION_REQUIRED_EXIT)
        }
        Some(TurnFinish::Failed) | None => (ResultStatus::Failed, 1),
    }
}

fn write_json(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *writer, value).context("writing JSON to stdout")?;
    writer
        .write_all(b"\n")
        .context("terminating JSON output line")?;
    writer.flush().context("flushing JSON stdout")
}

fn write_text(writer: &mut impl Write, text: &str) -> Result<()> {
    writer
        .write_all(text.as_bytes())
        .context("writing assistant output")?;
    if !text.ends_with('\n') {
        writer.write_all(b"\n").context("terminating output")?;
    }
    writer.flush().context("flushing stdout")
}

fn write_text_projection(writer: &mut impl Write, result: &ResultEnvelope) -> Result<()> {
    let mut lines = Vec::new();
    if let Some(goal) = &result.goal {
        let status = goal.status.as_str();
        let used = goal
            .usage
            .charged_tokens
            .map_or_else(|| "unknown".to_owned(), |tokens| tokens.to_string());
        let budget = goal
            .token_budget
            .map_or_else(|| "none".to_owned(), |tokens| tokens.to_string());
        lines.push(format!(
            "goal: {status} · {used} tokens · budget {budget} · {} continuation turn(s)",
            result.goal_continuation_turns.unwrap_or_default()
        ));
        if let Some(reason) = &goal.stopped_reason {
            lines.push(reason.detail.as_ref().map_or_else(
                || format!("goal reason: {}", reason.code),
                |detail| format!("goal reason: {} · {detail}", reason.code),
            ));
        }
    }
    if result.lifecycle.attempts_committed > 0 || result.lifecycle.attempts_discarded > 0 {
        lines.push(format!(
            "provider attempts: {} committed · {} discarded",
            result.lifecycle.attempts_committed, result.lifecycle.attempts_discarded
        ));
    }
    if let Some(activation) = &result.lifecycle.activation {
        lines.push(if activation.capabilities.is_empty() {
            format!(
                "activation epoch {} · no optional capabilities",
                activation.epoch
            )
        } else {
            format!(
                "activation epoch {} · {}",
                activation.epoch,
                activation.capabilities.join(", ")
            )
        });
    }
    if let Some(plan) = &result.lifecycle.plan {
        let counts = plan
            .counts
            .iter()
            .map(|(status, count)| format!("{status}={count}"))
            .collect::<Vec<_>>()
            .join(" · ");
        lines.push(format!("todo plan revision {} · {counts}", plan.revision));
    }
    if let Some(cache) = &result.cache {
        let state = cache
            .state
            .map_or_else(|| "unknown".to_owned(), |state| state.as_str().to_owned());
        let ch = cache
            .cache_read_percent
            .map_or_else(|| "?".to_owned(), |percent| format!("{percent}%"));
        let confidence = cache.confidence.map_or_else(
            || "?".to_owned(),
            |confidence| match confidence {
                EstimationConfidence::Exact => "exact".to_owned(),
                EstimationConfidence::Estimated => "estimated".to_owned(),
            },
        );
        let mut line = format!("cache: {state} · CH {ch} · confidence {confidence}");
        if let Some(expected) = cache.expected_read_tokens {
            line.push_str(&format!(" · expected {expected}"));
        }
        if let Some(observed) = cache.observed_read_tokens {
            line.push_str(&format!(" · observed {observed}"));
        }
        if let Some(missed) = cache.missed_tokens {
            line.push_str(&format!(" · missed {missed}"));
        }
        if let Some(rebilled) = cache.rebilled_tokens {
            line.push_str(&format!(" · re-billed {rebilled}"));
        }
        lines.push(line);
    }
    for artifact in &result.artifacts {
        lines.push(format!(
            "artifact {} · {} bytes · {}",
            artifact.id, artifact.byte_length, artifact.media_type
        ));
    }
    if let Some(recovery) = &result.recovery {
        lines.push(format!(
            "recovery {} · {} child(ren) interrupted · {} monitor(s) interrupted · not restarted",
            recovery.reason,
            recovery.interrupted_children.len(),
            recovery.interrupted_monitors.len()
        ));
    }

    for line in lines {
        writeln!(writer, "smith: {line}").context("writing text projection to stderr")?;
    }
    writer.flush().context("flushing text projection stderr")
}

fn approval_diagnostic(required: &ApprovalOutput) -> String {
    let resource = match &required.resource {
        SecurityResource::Filesystem { mount, segments } => {
            let relative = segments.join("/");
            if relative.is_empty() {
                mount.clone()
            } else if mount.ends_with('/') {
                format!("{mount}{relative}")
            } else {
                format!("{mount}/{relative}")
            }
        }
        SecurityResource::Network {
            origin,
            method,
            segments,
        } => {
            let path = segments.join("/");
            if path.is_empty() {
                format!("{method} {origin}")
            } else {
                format!("{method} {origin}/{path}")
            }
        }
        SecurityResource::Credential { reference } => format!("credential:{reference}"),
        SecurityResource::Other { kind, id } => format!("{kind}:{id}"),
    };
    let permissions = if required.permissions.is_empty() {
        "none".to_owned()
    } else {
        required.permissions.join(", ")
    };
    let mut diagnostic = format!(
        "approval required for tool `{}` · resource `{resource}` · permissions {permissions} · \
         fingerprint {}",
        required.tool, required.preparation_fingerprint
    );
    if let Some(deadline) = required.deadline_at_ms {
        diagnostic.push_str(&format!(" · deadline_ms {deadline}"));
    }
    if !required.authority_warnings.is_empty() {
        diagnostic.push_str(&format!(
            " · warnings {}",
            required.authority_warnings.join(", ")
        ));
    }
    diagnostic.push_str(" · argument values protected");
    diagnostic
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_runtime::provider::fake::{
        FakeProvider, ScriptedStream, tool_call_fragments, usage_event,
    };
    use agent_runtime_core::approval::{AllowAll, DenyAll};
    use agent_runtime_core::artifact::{
        ArtifactDigest, ArtifactId, ArtifactProvenance, ArtifactRead, ArtifactRef,
        ArtifactRetention, ArtifactSensitivity, MAX_ARTIFACT_READ_BYTES,
    };
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::clock::Timestamp;
    use agent_runtime_core::event::CacheState;
    use agent_runtime_core::goal::{GoalTokenUsage, GoalUsageProvenance};
    use agent_runtime_core::ids::{AttemptId, EventId, GoalId, RequestId, TurnId};
    use agent_runtime_core::provider::{
        Capabilities, FinishReason, Provider, ProviderError, ProviderErrorKind, ProviderStreamEvent,
    };
    use agent_runtime_core::usage::{CounterKind, Provenance, UsageRecord, UsageSource};
    use smith_config::resolve::{ResolveRequest, resolve};
    use smith_host::{InteractionNotice, InteractiveInteraction, ProjectWorkspace};
    use smith_runtime::checkpoint::{
        CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError,
    };
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

    #[test]
    fn approval_has_a_distinct_stable_exit_status() {
        let required = ApprovalRequired {
            call_id: "call-1".into(),
            tool: "edit".into(),
            argument_keys: vec!["path".into()],
            mutates: true,
            requires_authorization: true,
            permissions: vec!["fs.write".into()],
            resource: SecurityResource::filesystem("/repo", vec!["target.txt".into()]),
            authority_warnings: Vec::new(),
            deadline_at_ms: None,
            preparation_fingerprint: "0123456789abcdef0123456789abcdef".into(),
        };
        assert!(matches!(
            outcome(
                Some(&TurnFinish::Completed),
                None,
                Some(&required),
                None,
                None,
            ),
            (ResultStatus::ApprovalRequired, APPROVAL_REQUIRED_EXIT)
        ));
    }

    #[test]
    fn returned_child_input_has_the_interaction_required_exit_status() {
        assert!(matches!(
            outcome(
                Some(&TurnFinish::NeedsInput {
                    request: agent_runtime_core::ids::InteractionRequestId::new("child-question"),
                }),
                None,
                None,
                None,
                None,
            ),
            (ResultStatus::InteractionRequired, INTERACTION_REQUIRED_EXIT)
        ));
    }

    #[test]
    fn empty_usage_is_labelled_unknown_instead_of_reported_zero() {
        assert!(matches!(
            UsageProvenance::of(&UsageDelta::new()),
            UsageProvenance::Unknown
        ));
    }

    fn goal_projection(status: GoalStatus) -> GoalProjection {
        GoalProjection {
            id: GoalId::new("goal-fixture"),
            generation: 4,
            objective: "Finish the fixture".into(),
            status,
            token_budget: Some(100),
            usage: GoalTokenUsage {
                charged_tokens: Some(120),
                provenance: GoalUsageProvenance::ProviderReported,
                active_elapsed_ms: 25,
            },
            created_at: Timestamp(10),
            updated_at: Timestamp(20),
            stopped_reason: None,
        }
    }

    #[test]
    fn every_goal_terminal_maps_to_a_stable_headless_outcome() {
        for (status, expected) in [
            (GoalStatus::Active, ResultStatus::Failed),
            (GoalStatus::Paused, ResultStatus::Cancelled),
            (GoalStatus::Blocked, ResultStatus::Failed),
            (GoalStatus::UsageLimited, ResultStatus::LimitReached),
            (GoalStatus::BudgetLimited, ResultStatus::LimitReached),
            (GoalStatus::Complete, ResultStatus::Ok),
        ] {
            let goal = goal_projection(status);
            assert_eq!(
                outcome(Some(&TurnFinish::Completed), Some(&goal), None, None, None,).0,
                expected
            );
        }
    }

    #[test]
    fn machine_lifecycle_projects_durable_child_continuation_without_content() {
        let lifecycle = LifecycleOutput {
            children: vec![ChildSessionOutput {
                child_id: "child-3".to_owned(),
                child_session_id: "child-session-3".to_owned(),
                durability: "durable",
                state: "interrupted",
                resumable: true,
                turns_used: 1,
                max_turns: None,
                tokens_used: 42,
                incompatibility: None,
            }],
            ..LifecycleOutput::default()
        };
        let value = serde_json::to_value(lifecycle).expect("machine lifecycle serializes");
        assert_eq!(value["children"][0]["child_id"], "child-3");
        assert_eq!(value["children"][0]["child_session_id"], "child-session-3");
        assert_eq!(value["children"][0]["durability"], "durable");
        assert_eq!(value["children"][0]["resumable"], true);
        // An unbounded child reports no cap at all rather than a sentinel.
        assert!(value["children"][0].get("max_turns").is_none());
        assert!(
            !value.to_string().contains("task"),
            "child task content entered the machine status projection"
        );
    }

    #[test]
    fn a_sequence_gap_is_a_failure_instead_of_silent_stream_loss() {
        let mut last = None;
        let mut error = None;
        observe_sequence(&mut last, 4, &mut error);
        observe_sequence(&mut last, 5, &mut error);
        assert!(error.is_none());

        observe_sequence(&mut last, 8, &mut error);
        assert!(error.expect("a gap").contains("between sequence 5 and 8"));
    }

    #[test]
    fn machine_result_v2_compatibility_fixture_is_stable() {
        let usage = UsageDelta::new()
            .with(agent_runtime_core::usage::CounterKind::InputUncached, 12)
            .with(agent_runtime_core::usage::CounterKind::Output, 2);
        let result = ResultEnvelope {
            schema_version: 2,
            kind: "result",
            status: ResultStatus::Ok,
            session_id: "session-fixture".into(),
            turn_id: "turn-fixture".into(),
            provider: "fixture-provider".into(),
            model: "fixture-model".into(),
            output: "fixture answer".into(),
            usage: UsageOutput {
                current_turn: usage.clone(),
                session: usage,
                current_turn_provenance: UsageProvenance::ProviderReported,
                session_provenance: UsageProvenance::ProviderReported,
            },
            lifecycle: LifecycleOutput {
                attempts_committed: 1,
                attempts_discarded: 0,
                activation: Some(ActivationOutput {
                    epoch: 1,
                    capabilities: vec!["tool:read".into()],
                }),
                plan: None,
                children: Vec::new(),
            },
            goal: None,
            goal_continuation_turns: None,
            artifacts: Vec::new(),
            approval_required: None,
            interaction_required: None,
            recovery: None,
            account: None,
            background_exit: None,
            reasoning: None,
            cache: None,
            error: None,
        };

        let actual = serde_json::to_value(result).expect("serializable result");
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/machine-result-v2.json"))
                .expect("valid fixture");
        assert_eq!(actual, expected);
    }

    #[test]
    fn approval_required_v2_compatibility_fixture_is_stable_and_redacted() {
        let result = ResultEnvelope {
            schema_version: 2,
            kind: "result",
            status: ResultStatus::ApprovalRequired,
            session_id: "session-fixture".into(),
            turn_id: "turn-fixture".into(),
            provider: "fixture-provider".into(),
            model: "fixture-model".into(),
            output: String::new(),
            usage: UsageOutput {
                current_turn: UsageDelta::new(),
                session: UsageDelta::new(),
                current_turn_provenance: UsageProvenance::Unknown,
                session_provenance: UsageProvenance::Unknown,
            },
            lifecycle: LifecycleOutput::default(),
            goal: None,
            goal_continuation_turns: None,
            artifacts: Vec::new(),
            approval_required: Some(ApprovalOutput {
                call_id: "call-fixture".into(),
                tool: "edit".into(),
                argument_keys: vec!["new_string".into(), "old_string".into(), "path".into()],
                mutates: true,
                requires_authorization: true,
                permissions: vec!["fs.write".into()],
                resource: SecurityResource::filesystem(
                    "/repo",
                    vec!["src".into(), "lib.rs".into()],
                ),
                authority_warnings: Vec::new(),
                deadline_at_ms: Some(1_750_000_000_000),
                preparation_fingerprint: "0123456789abcdef0123456789abcdef".into(),
            }),
            interaction_required: None,
            recovery: None,
            account: None,
            background_exit: None,
            reasoning: None,
            cache: None,
            error: None,
        };

        let actual = serde_json::to_value(result).expect("serializable result");
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/approval-required-v2.json"))
                .expect("valid fixture");
        assert_eq!(actual, expected);
        assert_eq!(
            actual["approval_required"]["requires_authorization"], true,
            "the v2 fixture records authoritative permission-bearing work"
        );
        assert!(
            !actual.to_string().contains("replacement contents"),
            "machine approval fixture exposed an argument value"
        );
    }

    #[test]
    fn interaction_required_v2_fixture_is_stable_and_content_free() {
        let result = ResultEnvelope {
            schema_version: 2,
            kind: "result",
            status: ResultStatus::InteractionRequired,
            session_id: "session-fixture".into(),
            turn_id: "turn-fixture".into(),
            provider: "fixture-provider".into(),
            model: "fixture-model".into(),
            output: String::new(),
            usage: UsageOutput {
                current_turn: UsageDelta::new(),
                session: UsageDelta::new(),
                current_turn_provenance: UsageProvenance::Unknown,
                session_provenance: UsageProvenance::Unknown,
            },
            lifecycle: LifecycleOutput::default(),
            goal: None,
            goal_continuation_turns: None,
            artifacts: Vec::new(),
            approval_required: None,
            interaction_required: Some(InteractionOutput {
                request_id: "interaction-fixture".into(),
                question_count: 2,
            }),
            recovery: None,
            account: None,
            background_exit: None,
            reasoning: None,
            cache: None,
            error: None,
        };
        let actual = serde_json::to_value(result).expect("serializable result");
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/interaction-required-v2.json"
        ))
        .expect("valid fixture");
        assert_eq!(actual, expected);
        assert!(!actual.to_string().contains("question prompt"));
    }

    #[test]
    fn recovery_projection_is_metadata_only_and_keeps_the_monitor_seam_explicit() {
        let interruption = EphemeralWorkInterruption::process_exit(
            [agent_runtime_core::ids::ChildId::new("child-2")],
            std::iter::empty::<String>(),
            std::iter::empty::<String>(),
        );
        let actual =
            serde_json::to_value(RecoveryOutput::from(&interruption)).expect("serializable");
        assert_eq!(actual["reason"], "process_exit");
        assert_eq!(actual["interrupted_children"][0], "child-2");
        assert_eq!(
            actual["interrupted_monitors"],
            serde_json::json!([]),
            "this fixture contains no running monitor marker to reconcile"
        );
    }

    #[test]
    fn sensitive_plan_event_content_is_removed_before_machine_projection() {
        let protected_item = "PROTECTED PLAN CONTENT";
        let projected = plan_output(
            7,
            PlanSensitivity::Sensitive,
            BTreeMap::from([("pending".to_owned(), 1)]),
            Some(vec![PlanItemProjection {
                id: "protected".to_owned(),
                text: protected_item.to_owned(),
                status: agent_runtime_core::event::PlanItemStatus::Pending,
                reason: None,
            }]),
        );

        let serialized = serde_json::to_string(&projected).expect("machine plan projection");
        assert!(!serialized.contains(protected_item));
        assert!(!serialized.contains("\"items\""));
    }

    #[test]
    fn text_projection_reports_lifecycle_without_exposing_todo_or_argument_content() {
        let protected_item = "PROTECTED TODO CONTENT";
        let result = ResultEnvelope {
            account: None,
            schema_version: OUTPUT_SCHEMA_VERSION,
            kind: "result",
            status: ResultStatus::Ok,
            session_id: "session-fixture".into(),
            turn_id: "turn-fixture".into(),
            provider: "fixture-provider".into(),
            model: "fixture-model".into(),
            output: "answer".into(),
            usage: UsageOutput {
                current_turn: UsageDelta::new(),
                session: UsageDelta::new(),
                current_turn_provenance: UsageProvenance::Unknown,
                session_provenance: UsageProvenance::Unknown,
            },
            lifecycle: LifecycleOutput {
                attempts_committed: 2,
                attempts_discarded: 1,
                activation: Some(ActivationOutput {
                    epoch: 3,
                    capabilities: vec!["tool:read".into(), "tool:write_todos".into()],
                }),
                plan: Some(PlanOutput {
                    revision: 4,
                    sensitivity: PlanSensitivity::Sensitive,
                    counts: BTreeMap::from([
                        ("in_progress".to_owned(), 1),
                        ("pending".to_owned(), 2),
                    ]),
                    items: Some(vec![PlanItemProjection {
                        id: "protected".to_owned(),
                        text: protected_item.to_owned(),
                        status: agent_runtime_core::event::PlanItemStatus::InProgress,
                        reason: None,
                    }]),
                }),
                children: Vec::new(),
            },
            goal: None,
            goal_continuation_turns: None,
            artifacts: vec![ArtifactRef {
                id: ArtifactId::new("artifact-fixture").expect("valid artifact id"),
                digest: ArtifactDigest::new("sha256", "ab12").expect("valid digest"),
                media_type: "text/plain".into(),
                byte_length: 262_144,
                sensitivity: ArtifactSensitivity::Sensitive,
                retention: ArtifactRetention::Session,
                provenance: ArtifactProvenance::new(
                    agent_runtime_core::ids::SessionId::new("session-fixture"),
                    "tool-output",
                ),
            }],
            approval_required: None,
            interaction_required: None,
            recovery: Some(RecoveryOutput {
                reason: "process_exit",
                interrupted_children: vec!["child-1".into()],
                interrupted_monitors: vec!["monitor-1".into()],
                interrupted_tasks: vec!["task-1".into()],
            }),
            background_exit: None,
            reasoning: None,
            cache: None,
            error: None,
        };
        let mut stderr = Vec::new();

        write_text_projection(&mut stderr, &result).expect("text projection");

        let rendered = String::from_utf8(stderr).expect("UTF-8 projection");
        assert!(rendered.contains("2 committed · 1 discarded"));
        assert!(rendered.contains("activation epoch 3 · tool:read, tool:write_todos"));
        assert!(rendered.contains("todo plan revision 4 · in_progress=1 · pending=2"));
        assert!(rendered.contains("artifact artifact-fixture · 262144 bytes · text/plain"));
        assert!(rendered.contains("1 child(ren) interrupted · 1 monitor(s) interrupted"));
        assert!(!rendered.contains(protected_item));
    }

    #[test]
    fn canonical_cache_fixture_matches_tui_final_stream_and_text_surfaces() {
        let turn = TurnId::new("turn-cache");
        let event = |seq: u64, payload: RuntimeEvent| {
            EventEnvelope::new(
                seq,
                EventId::new(format!("cache-event-{seq}")),
                SessionId::new("session-cache"),
                Some(turn.clone()),
                Timestamp(seq.saturating_mul(60_000)),
                payload,
            )
        };
        let observation: RuntimeEvent = serde_json::from_value(serde_json::json!({
            "event": "cache_observation",
            "request": "request-cache",
            "attempt": "attempt-cache",
            "cache_plan": "plan-cache",
            "read_tokens": 0
        }))
        .expect("cache observation fixture");
        let state: RuntimeEvent = serde_json::from_value(serde_json::json!({
            "event": "cache_state_changed",
            "request": "request-cache",
            "attempt": "attempt-cache",
            "cache_plan": "plan-cache",
            "state": "miss_observed",
            "expected_read_tokens": 20_000,
            "observed_read_tokens": 0,
            "missed_tokens": 20_000,
            "confidence": "exact"
        }))
        .expect("cache state fixture");
        assert!(matches!(
            &state,
            RuntimeEvent::CacheStateChanged {
                state: CacheState::MissObserved,
                ..
            }
        ));
        let usage = UsageDelta::new().with(CounterKind::InputUncached, 20_000);
        let events = vec![
            event(
                1,
                RuntimeEvent::ProviderAttemptStarted {
                    request: RequestId::new("request-cache"),
                    attempt: AttemptId::new("attempt-cache"),
                    index: 0,
                    model: "fixture-model".to_owned(),
                },
            ),
            event(
                2,
                RuntimeEvent::Usage {
                    record: UsageRecord {
                        source: UsageSource::ProviderAttempt,
                        provenance: Provenance {
                            request: Some(RequestId::new("request-cache")),
                            attempt: Some(AttemptId::new("attempt-cache")),
                            ..Provenance::default()
                        },
                        delta: usage.clone(),
                    },
                },
            ),
            event(3, observation),
            event(4, state),
            event(
                5,
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ),
        ];

        let mut tui_status = smith_tui::status::Status::new("fixture-model", "/fixture");
        for envelope in &events {
            tui_status.record_cache_event(envelope);
        }
        let tui_summary = tui_status.cache_summary().expect("TUI cache summary");

        let mut projection = CacheProjection::default();
        projection.replay(events.clone());
        let headless_summary = projection
            .latest_completed()
            .expect("headless cache summary")
            .clone();
        assert_eq!(tui_summary, headless_summary);
        assert_eq!(headless_summary.missed_tokens, Some(20_000));
        assert_eq!(headless_summary.rebilled_tokens, 20_000);

        let result = ResultEnvelope {
            schema_version: OUTPUT_SCHEMA_VERSION,
            kind: "result",
            status: ResultStatus::Ok,
            session_id: "session-cache".to_owned(),
            turn_id: "turn-cache".to_owned(),
            provider: "fixture-provider".to_owned(),
            model: "fixture-model".to_owned(),
            output: "fixture answer".to_owned(),
            usage: UsageOutput {
                current_turn: usage.clone(),
                session: usage,
                current_turn_provenance: UsageProvenance::ProviderReported,
                session_provenance: UsageProvenance::ProviderReported,
            },
            lifecycle: LifecycleOutput::default(),
            goal: None,
            goal_continuation_turns: None,
            artifacts: Vec::new(),
            approval_required: None,
            interaction_required: None,
            recovery: None,
            account: None,
            background_exit: None,
            reasoning: None,
            cache: Some(CacheOutput::from_summary(&headless_summary, None)),
            error: None,
        };
        let final_json = serde_json::to_value(&result).expect("final JSON");
        assert_eq!(final_json["cache"]["state"], "miss_observed");
        assert_eq!(final_json["cache"]["cache_read_percent"], 0);
        assert_eq!(final_json["cache"]["missed_tokens"], 20_000);
        assert_eq!(final_json["cache"]["rebilled_tokens"], 20_000);

        let stream_lines: Vec<String> = events
            .iter()
            .map(|envelope| {
                serde_json::to_string(&StreamEnvelope {
                    schema_version: OUTPUT_SCHEMA_VERSION,
                    kind: "event",
                    event: envelope,
                })
                .expect("stream JSON")
            })
            .collect();
        let observation_at = stream_lines
            .iter()
            .position(|line| line.contains("cache_observation"))
            .expect("observation line");
        let state_at = stream_lines
            .iter()
            .position(|line| line.contains("cache_state_changed"))
            .expect("state line");
        assert!(observation_at < state_at);
        assert!(stream_lines[state_at].contains("\"missed_tokens\":20000"));

        let mut stderr = Vec::new();
        write_text_projection(&mut stderr, &result).expect("text cache projection");
        let text = String::from_utf8(stderr).expect("UTF-8 projection");
        assert!(text.contains("cache: miss_observed · CH 0% · confidence exact"));
        assert!(text.contains("missed 20000 · re-billed 20000"));
    }

    #[test]
    fn text_approval_diagnostic_identifies_exact_authority_without_argument_values() {
        let diagnostic = approval_diagnostic(&ApprovalOutput {
            call_id: "call-fixture".into(),
            tool: "edit".into(),
            argument_keys: vec!["new_string".into(), "path".into()],
            mutates: true,
            requires_authorization: true,
            permissions: vec!["fs.read".into(), "fs.write".into()],
            resource: SecurityResource::filesystem("/repo", vec!["src".into(), "lib.rs".into()]),
            authority_warnings: vec!["workspace_root_mutation".into()],
            deadline_at_ms: Some(1_750_000_000_000),
            preparation_fingerprint: "0123456789abcdef0123456789abcdef".into(),
        });

        assert!(diagnostic.contains("resource `/repo/src/lib.rs`"));
        assert!(diagnostic.contains("permissions fs.read, fs.write"));
        assert!(diagnostic.contains("workspace_root_mutation"));
        assert!(diagnostic.contains("deadline_ms 1750000000000"));
        assert!(diagnostic.contains("0123456789abcdef0123456789abcdef"));
        assert!(diagnostic.contains("argument values protected"));
        assert!(!diagnostic.contains("new_string"));
    }

    #[test]
    fn legacy_v1_result_shapes_remain_frozen_as_migration_fixtures() {
        for fixture in [
            include_str!("../tests/fixtures/machine-result-v1.json"),
            include_str!("../tests/fixtures/approval-required-v1.json"),
        ] {
            let value: serde_json::Value =
                serde_json::from_str(fixture).expect("valid legacy fixture");
            assert_eq!(value["schema_version"], 1);
            assert!(value.get("interaction_required").is_none());
        }
    }

    #[tokio::test]
    async fn rejected_headless_submission_is_a_structured_machine_result() {
        const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
        let home = tempfile::tempdir().expect("a home");
        let project = tempfile::tempdir().expect("a project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved config")
            .config;
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(Arc::new(HeadlessApproval::new())),
            provider: Some(Arc::new(FakeProvider::text_reply("unused")) as Arc<dyn Provider>),
            ..RuntimeRequest::new(config, HostSurface::Headless)
        };
        let host = smith_runtime::host::start(host_request(runtime, project.path()))
            .await
            .expect("a host");
        let initial_activation = host
            .session()
            .activation_epoch()
            .expect("the protected bootstrap activation");
        host.session().cancel_session(CancelReason::Shutdown);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let outcome = run_with_io(
            &host,
            "must be rejected".into(),
            OutputFormat::Json,
            HeadlessBrokers::default(),
            BackgroundExit::Error,
            &mut stdout,
            &mut stderr,
        )
        .await
        .expect("a structured result");

        assert_eq!(outcome.exit_code, 1);
        assert!(stderr.is_empty());
        let result: serde_json::Value = serde_json::from_slice(&stdout).expect("a result envelope");
        assert_eq!(result["status"], "failed");
        assert_eq!(result["turn_id"], "");
        assert_eq!(
            result["lifecycle"]["activation"]["epoch"],
            initial_activation.index()
        );
        assert_eq!(
            result["lifecycle"]["activation"]["capabilities"],
            serde_json::Value::Array(
                initial_activation
                    .activated()
                    .iter()
                    .map(|(id, _)| serde_json::Value::String(id.to_string()))
                    .collect()
            )
        );
        assert!(
            result["error"]
                .as_str()
                .is_some_and(|error| error.contains("no longer accepts turns"))
        );
    }

    #[tokio::test]
    async fn headless_result_never_reuses_an_older_session_answer() {
        const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
        let home = tempfile::tempdir().expect("a home");
        let project = tempfile::tempdir().expect("a project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved config")
            .config;
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "answer from an older turn".into(),
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::ReasoningDelta {
                        text: "the current turn has no visible answer".into(),
                        redacted: false,
                        signature: None,
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
            ],
        ));
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(Arc::new(HeadlessApproval::new())),
            provider: Some(provider as Arc<dyn Provider>),
            ..RuntimeRequest::new(config, HostSurface::Headless)
        };
        let host = smith_runtime::host::start(host_request(runtime, project.path()))
            .await
            .expect("a host");
        host.session()
            .run(UserInput::text("first turn"))
            .await
            .expect("the older turn runs");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let outcome = run_with_io(
            &host,
            "current turn".into(),
            OutputFormat::Json,
            HeadlessBrokers::default(),
            BackgroundExit::Error,
            &mut stdout,
            &mut stderr,
        )
        .await
        .expect("a structured result");

        assert_eq!(outcome.exit_code, 0);
        assert!(stderr.is_empty());
        let result: serde_json::Value = serde_json::from_slice(&stdout).expect("a result envelope");
        assert_eq!(result["status"], "ok");
        assert_eq!(result["output"], "");
        assert_ne!(result["turn_id"], "");
        assert!(
            !String::from_utf8(stdout)
                .expect("UTF-8 output")
                .contains("answer from an older turn")
        );
    }

    #[tokio::test]
    async fn headless_follows_an_explicit_goal_until_its_internal_turn_completes() {
        const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
        let home = tempfile::tempdir().expect("a home");
        let project = tempfile::tempdir().expect("a project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");

        let mut create = tool_call_fragments(
            0,
            "call-create",
            "create_goal",
            r#"{"objective":"finish the explicit goal"}"#,
        );
        create.push(usage_event(10, 2));
        create.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        let mut complete = tool_call_fragments(
            0,
            "call-complete",
            "update_goal",
            r#"{"id":"goal-call-create","generation":2,"status":"complete"}"#,
        );
        complete.push(usage_event(8, 2));
        complete.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(create),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "goal accepted".into(),
                    },
                    usage_event(4, 1),
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
                ScriptedStream::new(complete),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "goal complete".into(),
                    },
                    usage_event(3, 1),
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
            ],
        ));
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved config")
            .config;
        assert!(config.persistence.enabled.value);
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(Arc::new(HeadlessApproval::new())),
            provider: Some(provider.clone() as Arc<dyn Provider>),
            ..RuntimeRequest::new(config, HostSurface::Headless)
        };
        let host = smith_runtime::host::start(host_request(runtime, project.path()))
            .await
            .expect("a host");
        assert!(host.runtime().goal_component().is_some());
        let session = host.session().clone();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let execution = tokio::time::timeout(
            HEADLESS_TEST_WATCHDOG,
            run_with_io(
                &host,
                "Use create_goal to create an explicit persistent multi-turn goal, then continue it until complete".into(),
                OutputFormat::Json,
                HeadlessBrokers::default(),
                BackgroundExit::Error,
                &mut stdout,
                &mut stderr,
            ),
        )
        .await;
        let outcome = match execution {
            Ok(result) => result.expect("goal result"),
            Err(_) => {
                let goal = host.goal();
                let requests = provider.requests();
                let request_tools = requests
                    .iter()
                    .take(4)
                    .map(|request| {
                        request
                            .tools
                            .iter()
                            .map(|tool| tool.name.clone())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                let complete_result = host
                    .tool_result_text(&agent_runtime_core::ids::ToolCallId::new("call-complete"));
                let timeline = host.timeline_events().await.unwrap_or_default();
                let errors = timeline
                    .iter()
                    .filter_map(|event| match &event.payload {
                        RuntimeEvent::Error { error } => Some(error.to_string()),
                        _ => None,
                    })
                    .take(3)
                    .collect::<Vec<_>>();
                let _ = host.shutdown().await;
                panic!(
                    "goal execution did not stop; goal={goal:?}; requests={}; request_tools={request_tools:?}; complete_result={complete_result:?}; errors={errors:?}",
                    requests.len(),
                );
            }
        };

        assert_eq!(outcome.exit_code, 0);
        assert!(stderr.is_empty());
        let result: serde_json::Value = serde_json::from_slice(&stdout).expect("goal result JSON");
        assert_eq!(result["goal"]["status"], "complete", "{result:#}");
        assert_eq!(result["goal"]["usage"]["charged_tokens"], 19);
        assert_eq!(result["goal"]["usage"]["provenance"], "provider_reported");
        assert_eq!(result["goal_continuation_turns"], 1);
        assert_eq!(result["output"], "goal complete");
        assert_eq!(provider.requests().len(), 4);
        assert_eq!(
            session
                .history()
                .iter()
                .filter(|message| message.role == Role::User)
                .count(),
            1,
            "the internal continuation created a synthetic user message"
        );
    }

    #[tokio::test]
    async fn a_headless_edit_without_authority_is_structured_denied_and_redacted() {
        const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

# A turn carries no wall-clock ceiling by default. This test covers the
# deadline reaching the approval envelope end to end, so it opts back in to
# one; the absent case is covered where the envelope is built directly.
[limits]
turn_time_limit_ms = 600000
"#;
        const PROTECTED: &str = "TOP-SECRET-REPLACEMENT";

        let home = tempfile::tempdir().expect("a home");
        let project = tempfile::tempdir().expect("a project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");
        let target = project.path().join("target.txt");
        std::fs::write(&target, "safe\n").expect("a target");

        let mut tool = tool_call_fragments(
            0,
            "call-edit",
            "edit",
            &serde_json::json!({
                "path": "target.txt",
                "old_string": "safe",
                "new_string": PROTECTED,
            })
            .to_string(),
        );
        tool.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        let final_answer = vec![
            ProviderStreamEvent::TextDelta {
                text: "The edit was not authorized.".into(),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ];
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![ScriptedStream::new(tool), ScriptedStream::new(final_answer)],
        ));

        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved config")
            .config;
        let approval = Arc::new(HeadlessApproval::new());
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(approval.clone()),
            provider: Some(provider as Arc<dyn Provider>),
            ..RuntimeRequest::new(config, HostSurface::Headless)
        };
        let host = smith_runtime::host::start(host_request(runtime, project.path()))
            .await
            .expect("a host");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let outcome = tokio::time::timeout(
            HEADLESS_TEST_WATCHDOG,
            run_with_io(
                &host,
                "edit the file".into(),
                OutputFormat::Json,
                HeadlessBrokers {
                    approval: Some(approval.as_ref()),
                    ..HeadlessBrokers::default()
                },
                BackgroundExit::Error,
                &mut stdout,
                &mut stderr,
            ),
        )
        .await
        .expect("headless approval must not wait for stdin")
        .expect("a presented result");

        assert_eq!(outcome.exit_code, APPROVAL_REQUIRED_EXIT);
        assert_eq!(
            std::fs::read_to_string(target).expect("target contents"),
            "safe\n"
        );
        assert!(stderr.is_empty(), "JSON diagnostics leaked to stderr");
        let rendered = String::from_utf8(stdout).expect("UTF-8 JSON");
        assert!(!rendered.contains(PROTECTED), "{rendered}");
        let result: serde_json::Value =
            serde_json::from_str(rendered.trim()).expect("a result envelope");
        assert_eq!(result["status"], "approval_required");
        assert_eq!(result["approval_required"]["tool"], "edit");
        assert_eq!(
            result["approval_required"]["argument_keys"],
            // `operation` is normalized in by `edit::prepare`, so the key the
            // permission set was derived from is visible to the approver.
            serde_json::json!([
                "new_string",
                "old_string",
                "operation",
                "path",
                "replace_all"
            ])
        );
        assert_eq!(
            result["approval_required"]["permissions"],
            serde_json::json!(["fs.read", "fs.write"])
        );
        assert_eq!(
            result["approval_required"]["resource"]["resource_kind"],
            "filesystem"
        );
        assert_eq!(
            result["approval_required"]["resource"]["segments"],
            serde_json::json!(["target.txt"])
        );
        assert_eq!(
            result["approval_required"]["preparation_fingerprint"]
                .as_str()
                .map(str::len),
            Some(32)
        );
        // The configured ceiling above must survive the whole path into the
        // approval envelope: an approver that cannot see when its window
        // closes has to guess.
        assert!(
            result["approval_required"]["deadline_at_ms"]
                .as_u64()
                .is_some(),
            "{result}"
        );
    }

    #[tokio::test]
    async fn stream_json_projects_attempts_activation_todos_and_recoverable_artifacts() {
        const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
        const DISCARDED: &str = "FAILED ATTEMPT MUST NOT BECOME FINAL OUTPUT";
        let home = tempfile::tempdir().expect("a home");
        let project = tempfile::tempdir().expect("a project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");

        let mut todos = tool_call_fragments(
            0,
            "call-plan",
            "write_todos",
            &serde_json::json!({
                "items": [
                    {
                        "id": "inspect",
                        "text": "Inspect the retry evidence",
                        "status": "completed"
                    },
                    {
                        "id": "capture",
                        "text": "Capture the large diagnostic",
                        "status": "in_progress"
                    }
                ]
            })
            .to_string(),
        );
        todos.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        let command = "yes 'headless artifact line' | head -c 262144";
        let mut shell = tool_call_fragments(
            0,
            "call-shell",
            "shell",
            &serde_json::json!({ "command": command }).to_string(),
        );
        shell.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: DISCARDED.into(),
                    },
                    ProviderStreamEvent::Error {
                        error: ProviderError::new(
                            ProviderErrorKind::Server,
                            "retry the deterministic fixture",
                        )
                        .retryable(),
                    },
                ]),
                ScriptedStream::new(todos),
                ScriptedStream::new(shell),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "final committed answer".into(),
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
            ],
        ));
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved config")
            .config;
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(Arc::new(AllowAll)),
            provider: Some(provider as Arc<dyn Provider>),
            ..RuntimeRequest::new(config, HostSurface::Headless)
        };
        let host = smith_runtime::host::start(host_request(runtime, project.path()))
            .await
            .expect("a host");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let outcome = run_with_io(
            &host,
            "Use write_todos, then shell, for this multi-step diagnostic.".into(),
            OutputFormat::StreamJson,
            HeadlessBrokers::default(),
            BackgroundExit::Error,
            &mut stdout,
            &mut stderr,
        )
        .await
        .expect("a stream result");

        assert_eq!(outcome.exit_code, 0);
        assert!(stderr.is_empty());
        let lines = String::from_utf8(stdout)
            .expect("UTF-8 JSONL")
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("one JSON value"))
            .collect::<Vec<_>>();
        let events = &lines[..lines.len() - 1];
        assert!(events.iter().any(|line| {
            line["event"]["payload"]["event"] == "provider_attempt_output_discarded"
        }));
        assert!(events.iter().any(|line| {
            line["event"]["payload"]["event"] == "provider_attempt_output_committed"
        }));
        assert!(
            events
                .iter()
                .any(|line| line["event"]["payload"]["event"] == "capabilities_activated")
        );
        assert!(
            events
                .iter()
                .any(|line| line["event"]["payload"]["event"] == "plan_updated")
        );

        let result = lines.last().expect("a terminal result");
        assert_eq!(result["status"], "ok");
        assert_eq!(result["output"], "final committed answer");
        assert_eq!(result["lifecycle"]["attempts_discarded"], 1);
        assert_eq!(result["lifecycle"]["attempts_committed"], 3);
        assert_eq!(result["lifecycle"]["plan"]["revision"], 2);
        assert_eq!(result["lifecycle"]["plan"]["counts"]["in_progress"], 0);
        assert_eq!(result["lifecycle"]["plan"]["counts"]["cancelled"], 1);
        assert!(
            result["lifecycle"]["activation"]["capabilities"]
                .as_array()
                .is_some_and(|capabilities| capabilities.iter().any(|id| {
                    id.as_str()
                        .is_some_and(|id| id.contains("write_todos") || id.contains("shell"))
                }))
        );
        assert_eq!(result["artifacts"].as_array().map(Vec::len), Some(1));
        assert!(!result.to_string().contains(DISCARDED));

        let reference: ArtifactRef =
            serde_json::from_value(result["artifacts"][0].clone()).expect("typed artifact");
        let page = host
            .runtime()
            .artifact_store()
            .expect("protected artifact store")
            .read(ArtifactRead {
                session: host.session().id().clone(),
                id: reference.id,
                offset: 0,
                limit: MAX_ARTIFACT_READ_BYTES,
            })
            .await
            .expect("the reported artifact is readable by its session");
        assert!(String::from_utf8_lossy(&page.bytes).contains("headless artifact line"));
    }

    #[tokio::test]
    async fn a_forced_headless_question_is_structured_and_never_reads_stdin() {
        const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
        const SENSITIVE_PROMPT: &str = "Which unreleased codename?";
        let home = tempfile::tempdir().expect("a home");
        let project = tempfile::tempdir().expect("a project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");

        let arguments = serde_json::json!({
            "questions": [{
                "id": "codename",
                "header": "Codename",
                "prompt": SENSITIVE_PROMPT,
                "choices": [{"id": "alpha", "label": "Alpha"}],
                "allow_free_form": true
            }],
            "sensitivity": "sensitive"
        })
        .to_string();
        let mut question = tool_call_fragments(0, "call-question", "ask_user", &arguments);
        question.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(question),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "Input was unavailable.".into(),
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
            ],
        ));
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved config")
            .config;
        let interaction = Arc::new(HeadlessInteraction::new());
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(Arc::new(HeadlessApproval::new())),
            interaction: Some(interaction.clone()),
            provider: Some(provider.clone() as Arc<dyn Provider>),
            ..RuntimeRequest::new(config, HostSurface::Headless)
        };
        let host = smith_runtime::host::start(host_request(runtime, project.path()))
            .await
            .expect("a host");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        // Runtime construction and the scripted follow-up can be delayed when
        // the full binary test suite contends for CI workers. The broker is
        // still required to resolve without stdin; this bound only avoids
        // treating scheduler contention as an interaction regression.
        let outcome = tokio::time::timeout(
            HEADLESS_TEST_WATCHDOG,
            run_with_io(
                &host,
                "ask me for the codename".into(),
                OutputFormat::Json,
                HeadlessBrokers {
                    interaction: Some(interaction.as_ref()),
                    ..HeadlessBrokers::default()
                },
                BackgroundExit::Error,
                &mut stdout,
                &mut stderr,
            ),
        )
        .await
        .expect("headless interaction must not wait for stdin")
        .expect("a structured outcome");

        assert_eq!(outcome.exit_code, INTERACTION_REQUIRED_EXIT);
        assert!(stderr.is_empty(), "JSON diagnostics leaked to stderr");
        let rendered = String::from_utf8(stdout).expect("UTF-8 result");
        assert!(!rendered.contains(SENSITIVE_PROMPT), "{rendered}");
        let result: serde_json::Value = serde_json::from_str(rendered.trim()).expect("result JSON");
        assert_eq!(result["schema_version"], OUTPUT_SCHEMA_VERSION);
        assert_eq!(result["status"], "interaction_required");
        assert_eq!(result["interaction_required"]["question_count"], 1);
        assert!(
            provider.requests()[0]
                .tools
                .iter()
                .all(|tool| tool.name != "ask_user"),
            "ordinary headless planning advertised the questionnaire ability"
        );
    }

    #[tokio::test]
    async fn a_restored_question_returns_the_same_request_without_submitting_a_new_turn() {
        const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
        const SENSITIVE_PROMPT: &str = "Which private recovery branch?";
        const NEW_PROMPT: &str = "THIS MUST NOT BECOME A SECOND USER TURN";
        let home = tempfile::tempdir().expect("a home");
        let project = tempfile::tempdir().expect("a project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");

        let arguments = serde_json::json!({
            "questions": [{
                "id": "branch",
                "header": "Branch",
                "prompt": SENSITIVE_PROMPT,
                "choices": [{"id": "safe", "label": "Safe"}]
            }],
            "sensitivity": "sensitive"
        })
        .to_string();
        let mut question = tool_call_fragments(0, "recovery-question", "ask_user", &arguments);
        question.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        let first_provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(question),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "first host closed the question".into(),
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
            ],
        ));
        let (interactive, mut requests) = InteractiveInteraction::new();
        let first_config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved config")
            .config;
        let first_runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(Arc::new(DenyAll)),
            interaction: Some(Arc::new(interactive)),
            provider: Some(first_provider as Arc<dyn Provider>),
            ..RuntimeRequest::new(first_config, HostSurface::Terminal)
        };
        let first = smith_runtime::host::start(host_request(first_runtime, project.path()))
            .await
            .expect("interactive host");
        let session_id = first.session().id().clone();
        let checkpoint_path = first
            .paths()
            .expect("persistent paths")
            .checkpoint(&session_id)
            .expect("checkpoint path");
        let turn = first
            .session()
            .send(UserInput::text("ask the recovery question"))
            .expect("accepted first turn");
        let turn_id = turn.id().clone();
        let InteractionNotice::Present(prompt) =
            requests.recv().await.expect("questionnaire presentation")
        else {
            panic!("expected a questionnaire presentation");
        };
        let request_id = prompt.request().id().clone();
        let pending_checkpoint =
            std::fs::read(&checkpoint_path).expect("protected pending checkpoint");
        assert!(
            !pending_checkpoint
                .windows(SENSITIVE_PROMPT.len())
                .any(|window| window == SENSITIVE_PROMPT.as_bytes()),
            "the protected envelope exposed plaintext questionnaire content"
        );

        prompt.cancel().expect("close the first presentation");
        turn.completed().await;
        first.shutdown().await.expect("first host shutdown");

        // Model an abrupt process loss at the captured AwaitingInteraction
        // boundary after the orderly test owner has released the lifecycle
        // lease. The later journal tail is intentionally left in place so
        // startup also exercises checkpoint-watermark reconciliation.
        std::fs::write(&checkpoint_path, &pending_checkpoint)
            .expect("restore the pending crash boundary");

        let recovery_provider = Arc::new(FakeProvider::text_reply(
            "recovered with unavailable interaction",
        ));
        let headless_interaction = Arc::new(HeadlessInteraction::new());
        let recovery_config =
            resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
                .expect("resolved recovery config")
                .config;
        let recovery_runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(Arc::new(DenyAll)),
            interaction: Some(headless_interaction.clone()),
            provider: Some(recovery_provider.clone() as Arc<dyn Provider>),
            ..RuntimeRequest::new(recovery_config, HostSurface::Headless)
        };
        let recovered = smith_runtime::host::start(
            host_request(recovery_runtime, project.path()).resume(session_id.clone()),
        )
        .await
        .expect("headless recovery host");
        let restored = recovered
            .restored_interaction()
            .expect("pending interaction metadata");
        assert_eq!(restored.request_id(), &request_id);
        assert_eq!(restored.turn_id(), &turn_id);
        assert_eq!(restored.question_count(), 1);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let outcome = tokio::time::timeout(
            HEADLESS_TEST_WATCHDOG,
            run_with_io(
                &recovered,
                NEW_PROMPT.into(),
                OutputFormat::Json,
                HeadlessBrokers {
                    interaction: Some(headless_interaction.as_ref()),
                    ..HeadlessBrokers::default()
                },
                BackgroundExit::Error,
                &mut stdout,
                &mut stderr,
            ),
        )
        .await
        .expect("restored headless interaction never waits for stdin")
        .expect("structured restored result");

        assert_eq!(outcome.exit_code, INTERACTION_REQUIRED_EXIT);
        assert!(stderr.is_empty(), "JSON diagnostics leaked to stderr");
        let rendered = String::from_utf8(stdout).expect("UTF-8 result");
        assert!(!rendered.contains(SENSITIVE_PROMPT), "{rendered}");
        assert!(!rendered.contains(NEW_PROMPT), "{rendered}");
        let result: serde_json::Value = serde_json::from_str(rendered.trim()).expect("result JSON");
        assert_eq!(result["schema_version"], OUTPUT_SCHEMA_VERSION);
        assert_eq!(result["status"], "interaction_required");
        assert_eq!(result["session_id"], session_id.as_str());
        assert_eq!(result["turn_id"], turn_id.as_str());
        assert_eq!(
            result["interaction_required"]["request_id"],
            request_id.as_str()
        );
        assert_eq!(result["interaction_required"]["question_count"], 1);
        assert!(
            headless_interaction.required().is_none(),
            "headless inspection consumed the restored interaction through its broker"
        );
        assert_eq!(
            std::fs::read(&checkpoint_path).expect("preserved pending checkpoint"),
            pending_checkpoint,
            "reporting interaction_required advanced the exact pending checkpoint"
        );
        assert!(
            recovery_provider.requests().iter().all(|request| {
                !serde_json::to_string(&request.messages)
                    .expect("serializable provider messages")
                    .contains(NEW_PROMPT)
            }),
            "the command-line prompt was submitted while an older interaction was being recovered"
        );

        let interactive_provider = Arc::new(FakeProvider::text_reply(
            "resumed after the exact restored answer",
        ));
        let (interactive, mut requests) = InteractiveInteraction::new();
        let interactive_config =
            resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
                .expect("resolved interactive config")
                .config;
        let interactive_runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(Arc::new(DenyAll)),
            interaction: Some(Arc::new(interactive)),
            provider: Some(interactive_provider.clone() as Arc<dyn Provider>),
            ..RuntimeRequest::new(interactive_config, HostSurface::Terminal)
        };
        let interactive_host = smith_runtime::host::start(
            host_request(interactive_runtime, project.path()).resume(session_id),
        )
        .await
        .expect("interactive recovery host");
        let InteractionNotice::Present(prompt) =
            tokio::time::timeout(HEADLESS_TEST_WATCHDOG, requests.recv())
                .await
                .expect("interactive recovery presents without hanging")
                .expect("restored questionnaire presentation")
        else {
            panic!("expected the restored questionnaire presentation");
        };
        assert_eq!(prompt.request().id(), &request_id);
        assert_eq!(prompt.request().origin().turn(), &turn_id);
        prompt
            .answer(vec![
                agent_runtime_core::interaction::QuestionAnswer::choice(
                    agent_runtime_core::ids::QuestionId::new("branch"),
                    agent_runtime_core::ids::ChoiceId::new("safe"),
                ),
            ])
            .expect("the exact restored request accepts one answer");

        tokio::time::timeout(HEADLESS_TEST_WATCHDOG, async {
            loop {
                if interactive_host.session().history().iter().any(|message| {
                    message.joined_text() == "resumed after the exact restored answer"
                }) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the original turn resumes after its restored answer");
        assert_eq!(
            interactive_provider.requests().len(),
            1,
            "restored answer repeated provider work or created another user turn"
        );
        interactive_host
            .shutdown()
            .await
            .expect("interactive recovery shutdown");
    }

    fn pooled(active: usize) -> SharedPool {
        let mut pool = smith_runtime::pool::CredentialPool::new(
            "acme",
            [
                "keychain:smith/personal".to_owned(),
                "keychain:smith/work".to_owned(),
            ],
            None,
        );
        pool.set_active(active);
        SharedPool::new(pool)
    }

    #[test]
    fn a_single_credential_provider_projects_no_account() {
        // Nothing to disambiguate, so the field is absent rather than a row
        // naming the only credential there is.
        assert!(account_output(None, None).is_none());
    }

    #[test]
    fn a_headless_run_projects_the_account_it_used() {
        let pool = pooled(1);
        let account = account_output(Some(&pool), None).expect("an account");

        assert_eq!(account.position, 1);
        assert_eq!(account.reference, "keychain:smith/work");
        // Never measured, so no percentage is invented.
        assert_eq!(account.used_percent, None);
        assert!(!account.exhausted);
        assert_eq!(account.resets_at_ms, None);
        // One other account existed and was deliberately not used.
        assert_eq!(account.unused_members, 1);

        let value = serde_json::to_value(&account).expect("the account serializes");
        assert_eq!(value["reference"], "keychain:smith/work");
        assert_eq!(value["exhausted"], false);
        // Absent rather than null: a consumer must not read "unmeasured" as a
        // number.
        assert!(value.get("used_percent").is_none());
    }

    #[tokio::test]
    async fn an_exhausted_headless_run_reports_the_reset_it_stopped_on() {
        let pool = pooled(0);
        let rotation = HeadlessRotation::new();
        let request = smith_host::rotation::RotationRequest {
            provider: "acme".to_owned(),
            trigger: smith_host::rotation::RotationTrigger::Exhausted,
            outgoing: smith_host::rotation::RotationMember {
                position: 0,
                label: "keychain:smith/personal".to_owned(),
                used_percent: Some(100.0),
                cooling_until_ms: None,
            },
            eligible: vec![smith_host::rotation::RotationMember {
                position: 1,
                label: "keychain:smith/work".to_owned(),
                used_percent: None,
                cooling_until_ms: None,
            }],
            outgoing_resets_at_ms: Some(1_785_866_400_000),
        };
        // The policy declines and records, which is what a headless run does.
        {
            use smith_host::rotation::RotationPolicy;
            rotation.decide(&request).await;
        }

        let account = account_output(Some(&pool), Some(&rotation)).expect("an account");
        assert!(account.exhausted);
        assert_eq!(account.resets_at_ms, Some(1_785_866_400_000));
        // The run stayed put: the account is still the one it started on.
        assert_eq!(account.reference, "keychain:smith/personal");
    }

    fn sample_running_task(task_id: &str, command: &str) -> BackgroundTaskInfo {
        BackgroundTaskInfo {
            task_id: task_id.to_owned(),
            command: command.to_owned(),
            cwd: std::env::temp_dir(),
            spool_path: std::env::temp_dir().join("fixture.log"),
            status: TaskStatus::Running,
            timeout_ms: None,
        }
    }

    /// A fresh session ID every call, so tests that spawn real background
    /// tasks through the process-wide registry never collide with each other.
    fn unique_background_test_session(label: &str) -> SessionId {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        SessionId::new(format!(
            "headless-bg-exit-{label}-{}-{n}",
            std::process::id()
        ))
    }

    #[test]
    fn no_running_tasks_never_needs_a_background_exit_decision() {
        for policy in [
            BackgroundExit::Error,
            BackgroundExit::Wait,
            BackgroundExit::Stop,
        ] {
            assert_eq!(
                decide_background_exit(policy, &[]),
                BackgroundExitDecision::Clear
            );
        }
    }

    #[test]
    fn the_default_policy_is_error_and_names_the_task_by_id_and_command() {
        let running = [sample_running_task("task:7", "cargo test --workspace")];
        let decision = decide_background_exit(BackgroundExit::default(), &running);
        let BackgroundExitDecision::Error(message) = decision else {
            panic!("the default policy must fail closed instead of orphaning: {decision:?}");
        };
        assert!(message.contains("task:7"), "{message}");
        assert!(message.contains("cargo test --workspace"), "{message}");
    }

    #[test]
    fn wait_and_stop_policies_defer_to_the_async_poll_instead_of_deciding_synchronously() {
        let running = [sample_running_task("task:8", "sleep 30")];
        assert_eq!(
            decide_background_exit(BackgroundExit::Wait, &running),
            BackgroundExitDecision::Wait
        );
        assert_eq!(
            decide_background_exit(BackgroundExit::Stop, &running),
            BackgroundExitDecision::Stop
        );
    }

    #[test]
    fn multiple_running_tasks_are_all_named_in_the_error_policy_message() {
        let running = [
            sample_running_task("task:1", "make build"),
            sample_running_task("task:2", "npm test"),
        ];
        let decision = decide_background_exit(BackgroundExit::Error, &running);
        let BackgroundExitDecision::Error(message) = decision else {
            panic!("expected an error decision: {decision:?}");
        };
        assert!(
            message.starts_with("2 background shell task(s)"),
            "{message}"
        );
        assert!(
            message.contains("task:1") && message.contains("make build"),
            "{message}"
        );
        assert!(
            message.contains("task:2") && message.contains("npm test"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn no_running_tasks_leaves_no_error_and_no_report_under_every_policy() {
        let session_id = unique_background_test_session("clear");
        for policy in [
            BackgroundExit::Error,
            BackgroundExit::Wait,
            BackgroundExit::Stop,
        ] {
            let (error, output) = apply_background_exit_policy(&session_id, policy).await;
            assert!(error.is_none());
            assert!(output.is_none());
        }
    }

    #[tokio::test]
    async fn error_policy_reports_a_running_task_without_waiting_for_it() {
        let session_id = unique_background_test_session("error");
        let registry = BackgroundTaskRegistry::global();
        let (task_id, _spool) = registry
            .spawn_background_task(&session_id, "sleep 2".into(), std::env::temp_dir(), None)
            .await
            .expect("a spawned background task");

        let (error, output) =
            apply_background_exit_policy(&session_id, BackgroundExit::Error).await;

        let error = error.expect("the default policy fails closed");
        assert!(error.contains(&task_id), "{error}");
        assert!(error.contains("sleep 2"), "{error}");
        let output = output.expect("a background-exit report");
        assert_eq!(output.policy, "error");
        assert_eq!(output.tasks.len(), 1);
        assert_eq!(output.tasks[0].task_id, task_id);
        assert_eq!(output.tasks[0].status, "running");
        assert_eq!(output.tasks[0].exit_code, None);

        // The policy only reports; it never waits or stops on its own.
        assert_eq!(registry.running_tasks(&session_id).len(), 1);
        let _ = registry.stop_task(&session_id, &task_id).await;
    }

    #[tokio::test]
    async fn wait_policy_blocks_until_the_task_exits_and_reports_its_exit_code() {
        let session_id = unique_background_test_session("wait");
        let registry = BackgroundTaskRegistry::global();
        let (task_id, _spool) = registry
            .spawn_background_task(&session_id, "sleep 0.2".into(), std::env::temp_dir(), None)
            .await
            .expect("a spawned background task");

        let (error, output) = apply_background_exit_policy(&session_id, BackgroundExit::Wait).await;

        assert!(error.is_none());
        let output = output.expect("a background-exit report");
        assert_eq!(output.policy, "wait");
        assert_eq!(output.tasks.len(), 1);
        assert_eq!(output.tasks[0].task_id, task_id);
        assert_eq!(output.tasks[0].status, "exited");
        assert_eq!(output.tasks[0].exit_code, Some(0));
        assert!(registry.running_tasks(&session_id).is_empty());
    }

    #[tokio::test]
    async fn stop_policy_ends_a_long_running_task_well_before_its_own_deadline() {
        let session_id = unique_background_test_session("stop");
        let registry = BackgroundTaskRegistry::global();
        let (task_id, _spool) = registry
            .spawn_background_task(&session_id, "sleep 30".into(), std::env::temp_dir(), None)
            .await
            .expect("a spawned background task");

        let (error, output) = tokio::time::timeout(
            HEADLESS_TEST_WATCHDOG,
            apply_background_exit_policy(&session_id, BackgroundExit::Stop),
        )
        .await
        .expect("stop should not wait for the 30s command to finish on its own");

        assert!(error.is_none());
        let output = output.expect("a background-exit report");
        assert_eq!(output.policy, "stop");
        assert_eq!(output.tasks.len(), 1);
        assert_eq!(output.tasks[0].task_id, task_id);
        assert_eq!(output.tasks[0].status, "stopped");
        assert_eq!(output.tasks[0].exit_code, None);
        assert!(registry.running_tasks(&session_id).is_empty());
    }

    const BACKGROUND_EXIT_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;

    #[tokio::test]
    async fn default_error_policy_fails_a_headless_run_with_a_running_background_task() {
        let home = tempfile::tempdir().expect("a home");
        let project = tempfile::tempdir().expect("a project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), BACKGROUND_EXIT_CONFIG).expect("a config");
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved config")
            .config;
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(Arc::new(HeadlessApproval::new())),
            provider: Some(Arc::new(FakeProvider::text_reply("the answer")) as Arc<dyn Provider>),
            ..RuntimeRequest::new(config, HostSurface::Headless)
        };
        let host = smith_runtime::host::start(host_request(runtime, project.path()))
            .await
            .expect("a host");
        let registry = BackgroundTaskRegistry::global();
        let (task_id, _spool) = registry
            .spawn_background_task(
                host.session().id(),
                // Keep the task alive well beyond a contended headless turn.
                // A one-second sleep made this assertion depend on CI timing:
                // the correct error policy sees no running work after it exits.
                "sleep 30".into(),
                std::env::temp_dir(),
                None,
            )
            .await
            .expect("a spawned background task");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let outcome = run_with_io(
            &host,
            "hello".into(),
            OutputFormat::Json,
            HeadlessBrokers::default(),
            BackgroundExit::Error,
            &mut stdout,
            &mut stderr,
        )
        .await
        .expect("a structured result");

        assert_eq!(outcome.exit_code, 1);
        let result: serde_json::Value = serde_json::from_slice(&stdout).expect("a result envelope");
        assert_eq!(result["status"], "failed");
        assert_eq!(result["background_exit"]["policy"], "error");
        assert_eq!(result["background_exit"]["tasks"][0]["task_id"], task_id);
        assert_eq!(result["background_exit"]["tasks"][0]["status"], "running");
        assert!(
            result["error"]
                .as_str()
                .is_some_and(|error| error.contains(&task_id)),
            "{result:#}"
        );

        let _ = registry.stop_task(host.session().id(), &task_id).await;
    }

    #[tokio::test]
    async fn wait_policy_lets_a_headless_run_finish_after_its_background_task_exits() {
        let home = tempfile::tempdir().expect("a home");
        let project = tempfile::tempdir().expect("a project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), BACKGROUND_EXIT_CONFIG).expect("a config");
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved config")
            .config;
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a workspace"),
            )),
            approval: Some(Arc::new(HeadlessApproval::new())),
            provider: Some(Arc::new(FakeProvider::text_reply("the answer")) as Arc<dyn Provider>),
            ..RuntimeRequest::new(config, HostSurface::Headless)
        };
        let host = smith_runtime::host::start(host_request(runtime, project.path()))
            .await
            .expect("a host");
        let registry = BackgroundTaskRegistry::global();
        let (task_id, _spool) = registry
            .spawn_background_task(
                host.session().id(),
                // Long enough to still be running when the policy check
                // happens (after host startup and the turn itself), short
                // enough that `wait` polling it to completion stays fast.
                "sleep 1".into(),
                std::env::temp_dir(),
                None,
            )
            .await
            .expect("a spawned background task");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let outcome = run_with_io(
            &host,
            "hello".into(),
            OutputFormat::Json,
            HeadlessBrokers::default(),
            BackgroundExit::Wait,
            &mut stdout,
            &mut stderr,
        )
        .await
        .expect("a structured result");

        assert_eq!(outcome.exit_code, 0);
        let result: serde_json::Value = serde_json::from_slice(&stdout).expect("a result envelope");
        assert_eq!(result["status"], "ok");
        assert_eq!(result["background_exit"]["policy"], "wait");
        assert_eq!(result["background_exit"]["tasks"][0]["task_id"], task_id);
        assert_eq!(result["background_exit"]["tasks"][0]["status"], "exited");
        assert_eq!(result["background_exit"]["tasks"][0]["exit_code"], 0);
    }
}
