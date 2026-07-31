//! Non-interactive execution and versioned stdout contracts.

use std::io::{self, Write};
use std::time::Duration;

use agent_runtime_core::content::{Role, UserInput};
use agent_runtime_core::event::{EventEnvelope, RuntimeEvent, TurnFinish};
use agent_runtime_core::usage::UsageDelta;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use smith_host::{ApprovalRequired, HeadlessApproval};
use smith_runtime::host::HostSession;

use crate::cli::OutputFormat;

/// Version of Smith's result/event wrappers, independent of runtime events.
const OUTPUT_SCHEMA_VERSION: u32 = 1;

/// Stable process status used when an unattended call needs authorization.
pub(crate) const APPROVAL_REQUIRED_EXIT: u8 = 4;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_required: Option<ApprovalOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ResultStatus {
    Ok,
    ApprovalRequired,
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
}

impl From<ApprovalRequired> for ApprovalOutput {
    fn from(required: ApprovalRequired) -> Self {
        Self {
            call_id: required.call_id,
            tool: required.tool,
            argument_keys: required.argument_keys,
            mutates: required.mutates,
            requires_authorization: required.requires_authorization,
        }
    }
}

/// Runs one turn, preserving canonical event order for stream JSON.
pub(crate) async fn run(
    host: &HostSession,
    prompt: String,
    format: OutputFormat,
    approval: Option<&HeadlessApproval>,
) -> Result<Outcome> {
    let stdout = io::stdout();
    let stderr = io::stderr();
    run_with_io(
        host,
        prompt,
        format,
        approval,
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
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<Outcome> {
    let session = host.session();
    let mut events = session.subscribe();
    let turn_id = session.send(UserInput::text(prompt));
    let mut finish = None;
    let mut turn_usage = UsageDelta::new();
    let mut last_error = None;
    let mut last_sequence = None;
    let mut sequence_error = None;

    while let Some(event) = events.next().await {
        observe_sequence(&mut last_sequence, event.seq, &mut sequence_error);
        let belongs_to_turn = event.turn.as_ref() == Some(&turn_id);
        if belongs_to_turn {
            match &event.payload {
                RuntimeEvent::Usage { record } => turn_usage.merge(&record.delta),
                RuntimeEvent::Error { error } => last_error = Some(error.to_string()),
                RuntimeEvent::TurnCompleted {
                    finish: completed, ..
                } => {
                    finish = Some(completed.clone());
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
    let output = snapshot
        .history
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant && !message.joined_text().is_empty())
        .map(|message| message.joined_text())
        .unwrap_or_default();
    let session_usage = snapshot.usage.total();
    let approval_required = approval.and_then(HeadlessApproval::required);
    let lifecycle_error = shutdown_error.or(stream_error).or(sequence_error);
    let error = lifecycle_error.or_else(|| {
        matches!(finish, Some(TurnFinish::Failed))
            .then_some(last_error)
            .flatten()
    });
    let (status, exit_code) = outcome(finish.as_ref(), approval_required.as_ref(), error.as_ref());

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
        approval_required: approval_required.map(Into::into),
        error: error.clone(),
    };

    match format {
        OutputFormat::Text if exit_code == 0 => write_text(stdout, &output)?,
        OutputFormat::Text => {
            let diagnostic = match (&result.approval_required, error) {
                (Some(required), _) => format!(
                    "approval required for tool `{}` (argument values protected)",
                    required.tool
                ),
                (_, Some(error)) => error,
                _ => format!("turn ended with status {:?}", result.status),
            };
            writeln!(stderr, "smith: {diagnostic}").context("writing diagnostic to stderr")?;
            stderr.flush().context("flushing diagnostic stderr")?;
        }
        OutputFormat::Json | OutputFormat::StreamJson => write_json(stdout, &result)?,
    }

    Ok(Outcome { exit_code })
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

fn outcome(
    finish: Option<&TurnFinish>,
    approval: Option<&ApprovalRequired>,
    error: Option<&String>,
) -> (ResultStatus, u8) {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
    use agent_runtime_core::provider::{Capabilities, FinishReason, Provider, ProviderStreamEvent};
    use smith_config::resolve::{ResolveRequest, resolve};
    use smith_host::ProjectWorkspace;
    use smith_runtime::factory::{HostSurface, RuntimeRequest};
    use smith_runtime::host::HostSessionRequest;

    use super::*;

    #[test]
    fn approval_has_a_distinct_stable_exit_status() {
        let required = ApprovalRequired {
            call_id: "call-1".into(),
            tool: "edit".into(),
            argument_keys: vec!["path".into()],
            mutates: true,
            requires_authorization: true,
        };
        assert!(matches!(
            outcome(Some(&TurnFinish::Completed), Some(&required), None),
            (ResultStatus::ApprovalRequired, APPROVAL_REQUIRED_EXIT)
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
        let host = smith_runtime::host::start(HostSessionRequest::new(runtime, project.path()))
            .await
            .expect("a host");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let outcome = run_with_io(
            &host,
            "edit the file".into(),
            OutputFormat::Json,
            Some(approval.as_ref()),
            &mut stdout,
            &mut stderr,
        )
        .await
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
            serde_json::json!(["new_string", "old_string", "path"])
        );
    }
}
