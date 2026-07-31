//! Non-interactive execution and versioned stdout contracts.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::time::Duration;

use agent_runtime_core::artifact::ArtifactRef;
use agent_runtime_core::content::{Role, UserInput};
use agent_runtime_core::event::{
    EventEnvelope, PlanItemProjection, PlanSensitivity, RuntimeEvent, TurnFinish,
};
use agent_runtime_core::interaction::InteractionOutcomeKind;
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::usage::UsageDelta;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use smith_host::{ApprovalRequired, HeadlessApproval, HeadlessInteraction, InteractionRequired};
use smith_runtime::host::HostSession;
use smith_runtime::journal::{EphemeralInterruptionReason, EphemeralWorkInterruption};

use crate::cli::OutputFormat;

/// Version of Smith's result/event wrappers, independent of runtime events.
const OUTPUT_SCHEMA_VERSION: u32 = 2;

/// Stable process status used when an unattended call needs authorization.
pub(crate) const APPROVAL_REQUIRED_EXIT: u8 = 4;
/// Stable process status used when an unattended run needs task input.
pub(crate) const INTERACTION_REQUIRED_EXIT: u8 = 5;

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    artifacts: Vec<ArtifactRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_required: Option<ApprovalOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interaction_required: Option<InteractionOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
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

/// Runs one turn, preserving canonical event order for stream JSON.
pub(crate) async fn run(
    host: &HostSession,
    prompt: String,
    format: OutputFormat,
    approval: Option<&HeadlessApproval>,
    interaction: Option<&HeadlessInteraction>,
) -> Result<Outcome> {
    let stdout = io::stdout();
    let stderr = io::stderr();
    run_with_io(
        host,
        prompt,
        format,
        approval,
        interaction,
        &mut stdout.lock(),
        &mut stderr.lock(),
    )
    .await
}

async fn run_with_io(
    host: &HostSession,
    prompt: String,
    format: OutputFormat,
    approval: Option<&HeadlessApproval>,
    interaction: Option<&HeadlessInteraction>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<Outcome> {
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
    let mut last_error = None;
    let mut last_sequence = None;
    let mut sequence_error = None;
    let mut pending_interaction: Option<InteractionRequired> = None;
    let mut event_interaction_required: Option<InteractionRequired> = None;
    let mut lifecycle = LifecycleOutput {
        activation: initial_activation,
        ..LifecycleOutput::default()
    };

    while let Some(event) = events.next().await {
        observe_sequence(&mut last_sequence, event.seq, &mut sequence_error);
        let belongs_to_turn = event.turn.as_ref() == Some(&turn_id);
        if belongs_to_turn {
            match &event.payload {
                RuntimeEvent::Usage { record } => turn_usage.merge(&record.delta),
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
        if finish.is_some() {
            break;
        }
    }

    let stream_error = finish
        .is_none()
        .then(|| "the runtime event stream ended before the turn completed".to_owned());

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
    let approval_required = approval.and_then(HeadlessApproval::required);
    let interaction_required = interaction
        .and_then(HeadlessInteraction::required)
        .or(event_interaction_required);
    let lifecycle_error = shutdown_error.or(stream_error).or(sequence_error);
    let error = lifecycle_error.or_else(|| {
        matches!(finish, Some(TurnFinish::Failed))
            .then_some(last_error)
            .flatten()
    });
    let (status, exit_code) = outcome(
        finish.as_ref(),
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
        artifacts,
        approval_required: approval_required.map(Into::into),
        interaction_required: interaction_required.map(Into::into),
        recovery: host.recovered_ephemeral_work().map(Into::into),
        error: error.clone(),
    };

    match format {
        OutputFormat::Text if exit_code == 0 => {
            write_text(stdout, &output)?;
            write_text_projection(stderr, &result)?;
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
            ..LifecycleOutput::default()
        },
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: Some(required.into()),
        recovery: host.recovered_ephemeral_work().map(Into::into),
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
            ..LifecycleOutput::default()
        },
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: None,
        recovery: host.recovered_ephemeral_work().map(Into::into),
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

    use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
    use agent_runtime_core::approval::{AllowAll, DenyAll};
    use agent_runtime_core::artifact::{
        ArtifactDigest, ArtifactId, ArtifactProvenance, ArtifactRead, ArtifactRef,
        ArtifactRetention, ArtifactSensitivity, MAX_ARTIFACT_READ_BYTES,
    };
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::provider::{
        Capabilities, FinishReason, Provider, ProviderError, ProviderErrorKind, ProviderStreamEvent,
    };
    use smith_config::resolve::{ResolveRequest, resolve};
    use smith_host::{InteractionNotice, InteractiveInteraction, ProjectWorkspace};
    use smith_runtime::checkpoint::{
        CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError,
    };
    use smith_runtime::factory::{HostSurface, RuntimeRequest};
    use smith_runtime::host::HostSessionRequest;

    use super::*;

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
            outcome(Some(&TurnFinish::Completed), Some(&required), None, None,),
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
            schema_version: OUTPUT_SCHEMA_VERSION,
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
            },
            artifacts: Vec::new(),
            approval_required: None,
            interaction_required: None,
            recovery: None,
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
            schema_version: OUTPUT_SCHEMA_VERSION,
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
            schema_version: OUTPUT_SCHEMA_VERSION,
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
            artifacts: Vec::new(),
            approval_required: None,
            interaction_required: Some(InteractionOutput {
                request_id: "interaction-fixture".into(),
                question_count: 2,
            }),
            recovery: None,
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
                    }]),
                }),
            },
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
            }),
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
            None,
            None,
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
            None,
            None,
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
            Duration::from_secs(2),
            run_with_io(
                &host,
                "edit the file".into(),
                OutputFormat::Json,
                Some(approval.as_ref()),
                None,
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
            serde_json::json!(["new_string", "old_string", "path", "replace_all"])
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
        assert!(
            result["approval_required"]["deadline_at_ms"]
                .as_u64()
                .is_some()
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
            None,
            None,
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
        assert_eq!(result["lifecycle"]["plan"]["revision"], 1);
        assert_eq!(result["lifecycle"]["plan"]["counts"]["in_progress"], 1);
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

        let outcome = tokio::time::timeout(
            Duration::from_secs(2),
            run_with_io(
                &host,
                "ask me for the codename".into(),
                OutputFormat::Json,
                None,
                Some(interaction.as_ref()),
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
        assert_eq!(result["schema_version"], 2);
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
            Duration::from_secs(2),
            run_with_io(
                &recovered,
                NEW_PROMPT.into(),
                OutputFormat::Json,
                None,
                Some(headless_interaction.as_ref()),
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
        assert_eq!(result["schema_version"], 2);
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
            tokio::time::timeout(Duration::from_secs(2), requests.recv())
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

        tokio::time::timeout(Duration::from_secs(2), async {
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
}
