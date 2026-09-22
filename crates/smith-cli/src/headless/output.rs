//! Versioned headless result projections and text/JSON rendering.

use super::background::BackgroundExitOutput;
use super::*;

#[derive(Debug, Serialize)]
pub(super) struct StreamEnvelope<'a> {
    pub(super) schema_version: u32,
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) event: &'a EventEnvelope,
}

#[derive(Debug, Serialize)]
pub(super) struct CacheControllerEnvelope<'a> {
    pub(super) schema_version: u32,
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) controller: &'a CacheControllerSnapshot,
}

#[derive(Debug, Serialize)]
pub(super) struct ResultEnvelope {
    pub(super) schema_version: u32,
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) status: ResultStatus,
    pub(super) session_id: String,
    pub(super) turn_id: String,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) output: String,
    pub(super) usage: UsageOutput,
    pub(super) lifecycle: LifecycleOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) goal: Option<GoalProjection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) goal_continuation_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) artifacts: Vec<ArtifactRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) approval_required: Option<ApprovalOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) interaction_required: Option<InteractionOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) recovery: Option<RecoveryOutput>,
    /// The credential-pool account this run used, when the provider declares
    /// a pool. A headless run keeps one account start to finish, so this names
    /// what the whole run was billed to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) account: Option<AccountOutput>,
    /// What the background-exit policy did about background shell tasks that
    /// were still running when the final answer was ready. Absent when none
    /// were running, regardless of policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) background_exit: Option<BackgroundExitOutput>,
    /// The effective reasoning selection: what this run actually asked of the
    /// provider, so a caller reading token counts can tell "max was
    /// requested" apart from "the provider default applied".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reasoning: Option<ReasoningOutput>,
    /// The final turn's last cache plan, summarizing provider prefix reuse.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache: Option<CacheOutput>,
    /// Redaction-safe cold-continuation metadata; protected summary text and
    /// canonical history are deliberately absent from this projection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) resume_capsule: Option<smith_runtime::resume_capsule::RedactedResumeCapsule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

/// Redaction-safe projection of the run's reasoning selection.
#[derive(Debug, Serialize)]
pub(super) struct ReasoningOutput {
    /// "on", "off", or "provider default".
    pub(super) state: &'static str,
    /// Selected or documented effort, or "provider default".
    pub(super) effort: String,
    /// Bounded description of where the selection came from.
    pub(super) source: String,
}

impl ReasoningOutput {
    pub(super) fn of(policy: &smith_runtime::reasoning::ReasoningRuntimePolicy) -> Self {
        Self {
            state: policy.effective_state(),
            effort: policy.effective_effort().to_owned(),
            source: policy.selection_source.clone(),
        }
    }
}

/// The last cache plan the runtime emitted for the reported turn.
#[derive(Debug, Serialize)]
pub(super) struct CacheOutput {
    /// Whether the provider can reuse the plan's stable prefix.
    pub(super) provider_cache_supported: bool,
    /// Tokens of prefix carried over from the previous attempt's plan.
    pub(super) preserved_prefix_tokens: u32,
    /// Tokens at or after the first changed segment.
    pub(super) invalidated_prefix_tokens: u32,
    /// Aggregate canonical state for the final root turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) state: Option<CacheVisibilityState>,
    /// Exact redaction-safe Runtime cache-identity digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_identity: Option<String>,
    /// Canonical expectation and provider observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) expected_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) observed_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) observed_write_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) missed_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) confidence: Option<EstimationConfidence>,
    /// Latest completed root-turn provider cache-read share.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_read_percent: Option<u8>,
    /// Derived retry diagnostics, separate from `usage`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) miss_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) rebilled_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) idle_minutes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) extra_cost_micro_usd: Option<u128>,
    /// Canonical Runtime operation/evidence lifecycle for the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) lifecycle: Option<CacheLifecycleSummary>,
    /// Smith's bounded scheduler/lease projection captured before shutdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) controller: Option<CacheControllerSnapshot>,
    #[serde(skip)]
    pub(super) notice: Option<String>,
}

impl CacheOutput {
    pub(super) fn from_summary(summary: &CacheTurnSummary, prior: Option<Self>) -> Self {
        let prior = prior.unwrap_or(Self {
            provider_cache_supported: summary.state != CacheVisibilityState::Unsupported,
            preserved_prefix_tokens: 0,
            invalidated_prefix_tokens: 0,
            state: None,
            cache_identity: None,
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
            lifecycle: None,
            controller: None,
            notice: None,
        });
        Self {
            state: Some(summary.state),
            cache_identity: summary.cache_identity.clone(),
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

    pub(super) fn from_lifecycle(lifecycle: CacheLifecycleSummary, prior: Option<Self>) -> Self {
        let prior = prior.unwrap_or(Self {
            provider_cache_supported: true,
            preserved_prefix_tokens: 0,
            invalidated_prefix_tokens: 0,
            state: None,
            cache_identity: lifecycle.cache_identity.clone(),
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
            lifecycle: None,
            controller: None,
            notice: None,
        });
        Self {
            lifecycle: Some(lifecycle),
            ..prior
        }
    }

    pub(super) fn from_controller(
        controller: CacheControllerSnapshot,
        prior: Option<Self>,
    ) -> Self {
        let prior = prior.unwrap_or(Self {
            provider_cache_supported: controller.provider_contract.behavior
                != agent_runtime_core::provider::ProviderCacheBehavior::Unsupported,
            preserved_prefix_tokens: controller
                .lifecycle
                .current()
                .map_or(0, |lease| lease.structurally_preserved_prefix_tokens),
            invalidated_prefix_tokens: 0,
            state: None,
            cache_identity: controller
                .lifecycle
                .current()
                .map(|lease| lease.identity().digest().to_string()),
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
            lifecycle: None,
            controller: None,
            notice: None,
        });
        Self {
            controller: Some(controller),
            ..prior
        }
    }
}
/// The account a headless run used, and whether its window ran out.
#[derive(Debug, Serialize)]
pub(super) struct AccountOutput {
    /// Zero-based position in the declared pool.
    pub(super) position: usize,
    /// The credential reference, never its value.
    pub(super) reference: String,
    /// Server-reported consumption, absent when nothing measured it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) used_percent: Option<f64>,
    /// Whether the run ended because this account's window was spent.
    pub(super) exhausted: bool,
    /// When the spent window reopens, in Unix milliseconds, if reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) resets_at_ms: Option<u64>,
    /// How many other accounts were declared but deliberately not used.
    ///
    /// A headless run never rotates, so this is the number of accounts a
    /// script could have fallen back to had it been interactive.
    pub(super) unused_members: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ResultStatus {
    Ok,
    ApprovalRequired,
    InteractionRequired,
    Failed,
    Cancelled,
    LimitReached,
}

#[derive(Debug, Serialize)]
pub(super) struct UsageOutput {
    pub(super) current_turn: UsageDelta,
    pub(super) session: UsageDelta,
    /// Provider/session usage from typed cache-maintenance and idle-compaction
    /// attempts. It remains excluded from `current_turn`.
    #[serde(default, skip_serializing_if = "SyntheticUsageOutput::is_empty")]
    pub(super) synthetic_cache: SyntheticUsageOutput,
    pub(super) current_turn_provenance: UsageProvenance,
    pub(super) session_provenance: UsageProvenance,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct SyntheticUsageOutput {
    pub(super) total: UsageDelta,
    pub(super) by_purpose: BTreeMap<String, UsageDelta>,
}

impl SyntheticUsageOutput {
    pub(super) fn is_empty(&self) -> bool {
        self.total.is_empty() && self.by_purpose.is_empty()
    }

    pub(super) fn from_records(records: &[UsageRecord]) -> Self {
        let mut output = Self::default();
        for record in records {
            let Some(purpose) = record.provenance.attempt_purpose else {
                continue;
            };
            if !purpose.is_synthetic_cache() {
                continue;
            }
            output.total.merge(&record.delta);
            output
                .by_purpose
                .entry(purpose.as_str().to_owned())
                .or_insert_with(UsageDelta::new)
                .merge(&record.delta);
        }
        output
    }
}

pub(super) fn is_synthetic_usage(record: &UsageRecord) -> bool {
    record
        .provenance
        .attempt_purpose
        .is_some_and(ProviderAttemptPurpose::is_synthetic_cache)
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum UsageProvenance {
    ProviderReported,
    Unknown,
}

impl UsageProvenance {
    pub(super) fn of(delta: &UsageDelta) -> Self {
        if delta.is_empty() {
            Self::Unknown
        } else {
            Self::ProviderReported
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct ApprovalOutput {
    pub(super) call_id: String,
    pub(super) tool: String,
    pub(super) argument_keys: Vec<String>,
    pub(super) mutates: bool,
    pub(super) requires_authorization: bool,
    pub(super) permissions: Vec<String>,
    pub(super) resource: SecurityResource,
    pub(super) authority_warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) deadline_at_ms: Option<u64>,
    pub(super) preparation_fingerprint: String,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct LifecycleOutput {
    pub(super) attempts_committed: u32,
    pub(super) attempts_discarded: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) activation: Option<ActivationOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) plan: Option<PlanOutput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) children: Vec<ChildSessionOutput>,
    /// Whether the parent is idle, serving, or parked without provider I/O.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) parent_state: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub(super) struct ChildSessionOutput {
    pub(super) child_id: String,
    pub(super) child_session_id: String,
    pub(super) durability: &'static str,
    pub(super) state: &'static str,
    pub(super) resumable: bool,
    pub(super) turns_used: u32,
    /// Absent for an unbounded child.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_turns: Option<u32>,
    pub(super) tokens_used: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) incompatibility: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ActivationOutput {
    pub(super) epoch: u64,
    pub(super) capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct PlanOutput {
    pub(super) revision: u64,
    pub(super) sensitivity: PlanSensitivity,
    pub(super) counts: BTreeMap<String, u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) items: Option<Vec<PlanItemProjection>>,
}

#[derive(Debug, Serialize)]
pub(super) struct InteractionOutput {
    pub(super) request_id: String,
    pub(super) question_count: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct RecoveryOutput {
    pub(super) reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) interrupted_turn: Option<String>,
    pub(super) interrupted_children: Vec<String>,
    pub(super) interrupted_monitors: Vec<String>,
    pub(super) interrupted_tasks: Vec<String>,
}

impl From<&EphemeralWorkInterruption> for RecoveryOutput {
    fn from(interruption: &EphemeralWorkInterruption) -> Self {
        let reason = match interruption.reason {
            EphemeralInterruptionReason::ProcessExit => "process_exit",
        };
        Self {
            reason,
            interrupted_turn: None,
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

pub(super) fn recovery_output(host: &HostSession) -> Option<RecoveryOutput> {
    let mut recovery = host.recovered_ephemeral_work().map(RecoveryOutput::from);
    if let Some(turn) = host.session().interrupted_on_resume() {
        let record = recovery.get_or_insert_with(|| RecoveryOutput {
            reason: "activation_changed",
            interrupted_turn: None,
            interrupted_children: Vec::new(),
            interrupted_monitors: Vec::new(),
            interrupted_tasks: Vec::new(),
        });
        record.interrupted_turn = Some(turn.as_str().to_owned());
    }
    recovery
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

pub(super) fn child_session_outputs(host: &HostSession) -> Vec<ChildSessionOutput> {
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
pub(super) fn account_output(
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
pub(super) fn activation_output(
    session: &smith_runtime::SessionHandle,
) -> Option<ActivationOutput> {
    session.activation_epoch().map(|epoch| ActivationOutput {
        epoch: epoch.index(),
        capabilities: epoch
            .activated()
            .iter()
            .map(|(id, _)| id.to_string())
            .collect(),
    })
}

pub(super) fn plan_output(
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

/// The turn's own account of why it ended without an answer.
///
/// A failed turn reports the runtime's terminal error, or failing that the
/// last error an attempt reported about itself. A turn stopped by a limit
/// reports an attempt error only: the limit terminal carries no cause, and a
/// turn that exhausted its provider attempts (or its deadline while retrying)
/// failed for a reason the attempts already named. Budget limits such as
/// output length and tool steps end attempts without errors, so they keep the
/// causeless `limit_reached` status.
pub(super) fn terminal_error(
    finish: Option<&TurnFinish>,
    last_error: Option<String>,
    last_attempt_error: Option<String>,
) -> Option<String> {
    match finish {
        Some(TurnFinish::Failed) => last_error.or(last_attempt_error),
        Some(TurnFinish::LimitReached { .. }) => last_attempt_error,
        _ => None,
    }
}

pub(super) fn outcome(
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

pub(super) fn write_json(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *writer, value).context("writing JSON to stdout")?;
    writer
        .write_all(b"\n")
        .context("terminating JSON output line")?;
    writer.flush().context("flushing JSON stdout")
}

pub(super) fn write_text(writer: &mut impl Write, text: &str) -> Result<()> {
    writer
        .write_all(text.as_bytes())
        .context("writing assistant output")?;
    if !text.ends_with('\n') {
        writer.write_all(b"\n").context("terminating output")?;
    }
    writer.flush().context("flushing stdout")
}

pub(super) fn write_text_projection(
    writer: &mut impl Write,
    result: &ResultEnvelope,
) -> Result<()> {
    let mut lines = Vec::new();
    if let Some(parent_state) = result.lifecycle.parent_state {
        lines.push(format!("parent: {parent_state}"));
    }
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
        if let Some(line) = smith_tui::cache::render_cache_read_usage(
            cache.cache_read_percent,
            cache.observed_read_tokens,
        ) {
            lines.push(line);
        }
        if let Some(controller) = &cache.controller
            && !controller.synthetic_attempts.is_empty()
        {
            lines.push(crate::local_command::render_cache_controller_summary(
                controller,
            ));
        }
    }
    if !result.usage.synthetic_cache.is_empty() {
        let purposes = result
            .usage
            .synthetic_cache
            .by_purpose
            .iter()
            .map(|(purpose, usage)| format!("{purpose} ({})", render_usage_delta(usage)))
            .collect::<Vec<_>>()
            .join("; ");
        lines.push(format!("cache synthetic usage: {purposes}"));
    }
    if let Some(capsule) = &result.resume_capsule {
        lines.push(crate::local_command::render_resume_summary(capsule));
    }
    for artifact in &result.artifacts {
        lines.push(format!(
            "artifact {} · {} bytes · {}",
            artifact.id, artifact.byte_length, artifact.media_type
        ));
    }
    if let Some(recovery) = &result.recovery {
        if let Some(turn) = &recovery.interrupted_turn {
            lines.push(format!("session restored: tools changed since turn {turn}; unfinished action not retried; check previous changes before continuing"));
        }
        if !recovery.interrupted_children.is_empty()
            || !recovery.interrupted_monitors.is_empty()
            || !recovery.interrupted_tasks.is_empty()
        {
            lines.push(format!(
            "recovery {} · {} child(ren) interrupted · {} monitor(s) interrupted · not restarted",
            recovery.reason,
            recovery.interrupted_children.len(),
            recovery.interrupted_monitors.len()
        ));
        }
    }

    for line in lines {
        writeln!(writer, "smith: {line}").context("writing text projection to stderr")?;
    }
    writer.flush().context("flushing text projection stderr")
}

pub(super) fn render_usage_delta(delta: &UsageDelta) -> String {
    if delta.is_empty() {
        return "none".to_owned();
    }
    delta
        .iter()
        .map(|(kind, value)| format!("{} {value}", smith_tui::status::counter_label(kind)))
        .collect::<Vec<_>>()
        .join(" · ")
}

pub(super) fn approval_diagnostic(required: &ApprovalOutput) -> String {
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
        diagnostic.push_str(&format!(
            " · deadline {}",
            smith_tui::time_display::local_timestamp(deadline)
        ));
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
