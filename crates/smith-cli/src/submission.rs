//! Prepared-input materialization, dispatch, and agent/review actions.

use std::sync::Arc;

use agent_runtime_core::content::{ContentPart, ToolResultBlock, UserInput};
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::provider::ModelId;
use agent_runtime_core::steer::SteerRejectionReason;
use agent_runtime_core::workspace::Workspace;
use anyhow::Result;
use smith_client::NoticeKind;
use smith_client::agent_report::{AgentReport, AgentResumeReport, AgentSummary};
use smith_client::review_report::{ReviewReport, ReviewStartReport};
use smith_config::resolve::ResolvedAgent;
use smith_host::{ApprovalPrompt, GitChanges, ProjectWorkspace};
use smith_runtime::host::HostSession;
use smith_runtime::{ChildStatus, SpawnOutcome};
use smith_tui::app::{App, ChildState, PreparedSubmission, SubmissionTarget};

use crate::local_command::LocalOutcome;

#[derive(Clone, Default)]
pub(super) struct LocalShellApprovals {
    pending: Arc<std::sync::Mutex<Option<LocalShellAuthorization>>>,
}

#[derive(Clone, PartialEq, Eq)]
struct LocalShellAuthorization {
    session: SessionId,
    turn: agent_runtime_core::ids::TurnId,
}

struct LocalShellApprovalGuard {
    approvals: LocalShellApprovals,
    authorization: LocalShellAuthorization,
}

impl LocalShellApprovals {
    /// Consumes the submission's authorization, or returns a prompt to present.
    pub(super) fn resolve(&self, prompt: ApprovalPrompt) -> Option<ApprovalPrompt> {
        let mut pending = self.pending.lock().expect("local shell approval poisoned");
        let matches = pending.as_ref().is_some_and(|authorization| {
            prompt.tool() == "shell"
                && prompt.origin().session() == &authorization.session
                && prompt.origin().turn() == Some(&authorization.turn)
        });
        if !matches {
            return Some(prompt);
        }
        // A local-action turn contains exactly one call. Its runtime identity
        // binds the immutable prepared arguments without reconstructing cwd or
        // timeout normalization here, or granting authority to a model turn.
        pending.take();
        drop(pending);
        prompt.allow(smith_host::approval::PromptScope::Once);
        None
    }
}

impl Drop for LocalShellApprovalGuard {
    fn drop(&mut self) {
        let mut pending = self
            .approvals
            .pending
            .lock()
            .expect("local shell approval poisoned");
        if pending.as_ref() == Some(&self.authorization) {
            pending.take();
        }
    }
}

pub(super) enum LocalShellIdentity {
    Turn(agent_runtime_core::ids::TurnId),
    Call(agent_runtime_core::ids::ToolCallId),
}

pub(super) async fn start_local_shell(
    echo: u64,
    session: smith_runtime::SessionHandle,
    command: String,
    timeout_ms: u64,
    approvals: LocalShellApprovals,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) -> Option<LocalShellIdentity> {
    use std::future::{Future, poll_fn};
    use std::task::Poll;

    let execution_session = session.clone();
    let mut local = Box::pin(async move {
        execution_session
            .run_local_tool(
                "shell",
                serde_json::json!({
                    "command": command,
                    "cwd": ".",
                    "timeout_ms": timeout_ms,
                }),
                timeout_ms,
            )
            .await
    });
    // The pinned runtime reserves idle admission before its first await.
    // A rejection completes without arming anything; Pending proves this
    // future owns the local action, even if a model turn queues behind it.
    let (first, authorization) = poll_fn(|context| {
        // A prompt can be published during this poll. Keep its receiver
        // from resolving it before the owning identity has been bound.
        let mut pending = approvals
            .pending
            .lock()
            .expect("local shell approval poisoned");
        let first = local.as_mut().poll(context);
        // There is no public local-call handle. Empty steering cannot
        // enqueue input, and a local turn rejects it as NonSteerable
        // with the exact owning identity. Fail closed on any other
        // response instead of guessing an ID or matching command text.
        let authorization = if first.is_pending()
            && let Err(rejection) = session.steer_current_turn(None, UserInput::text(""))
            && let SteerRejectionReason::NonSteerable { active_turn } = rejection.reason
        {
            let authorization = LocalShellAuthorization {
                session: session.id().clone(),
                turn: active_turn,
            };
            *pending = Some(authorization.clone());
            Some(LocalShellApprovalGuard {
                approvals: approvals.clone(),
                authorization,
            })
        } else {
            None
        };
        Poll::Ready((first, authorization))
    })
    .await;
    let identity = match &first {
        Poll::Ready(Ok(block)) => Some(LocalShellIdentity::Call(block.call_id.clone())),
        _ => authorization
            .as_ref()
            .map(|guard| LocalShellIdentity::Turn(guard.authorization.turn.clone())),
    };
    tokio::spawn(async move {
        let outcome = match first {
            Poll::Ready(outcome) => outcome,
            Poll::Pending => local.await,
        };
        // Discard an unconsumed token before publishing completion.
        // The guard also covers task cancellation and unwinding.
        drop(authorization);
        let result = match outcome {
            Ok(block) => LocalOutcome::Shell {
                echo,
                call: Some(block.call_id.clone()),
                content: tool_result_text(&block),
                is_error: block.is_error,
            },
            Err(error) => LocalOutcome::Shell {
                echo,
                call: None,
                content: format!("shell action failed: {error}"),
                is_error: true,
            },
        };
        let _ = outcomes.send(result);
    });
    identity
}

/// Largest PNG accepted from the clipboard, after encoding.
const MAX_CLIPBOARD_IMAGE_BYTES: usize = 5 * 1024 * 1024;

pub(super) enum ClipboardContent {
    Image {
        data_uri: String,
        width: u32,
        height: u32,
    },
    Text(String),
    Empty,
}

/// Reads the platform clipboard once and attaches whatever it holds.
///
/// An image becomes a composer attachment; text falls back to the ordinary
/// paste path (covering terminals whose `Ctrl+V` never reaches bracketed
/// paste); an unreadable clipboard reports instead of failing silently.
pub(super) fn attach_from_clipboard(app: &mut App) {
    apply_clipboard_content(app, read_clipboard());
}

/// Applies one clipboard keypress result without platform I/O.
pub(super) fn apply_clipboard_content(app: &mut App, content: Result<ClipboardContent, String>) {
    app.clear_feedback();
    match content {
        Ok(ClipboardContent::Image {
            data_uri,
            width,
            height,
        }) => {
            if app.can_attach_image() {
                app.attach_image(data_uri, width, height);
            } else {
                app.push_notice(
                    NoticeKind::Clipboard,
                    "close the current panel before attaching an image",
                );
            }
        }
        Ok(ClipboardContent::Text(text)) => app.on_paste(&text),
        Ok(ClipboardContent::Empty) => {
            app.push_notice(NoticeKind::Clipboard, "nothing to attach");
        }
        Err(error) => app.push_notice(NoticeKind::Clipboard, error),
    }
}

/// Puts pointer-selected text on the platform clipboard.
///
/// Success is deliberately silent: the highlight stays painted over exactly
/// what was copied, which is the same feedback the terminal's own selection
/// gives. A transcript line per copy would be noise, and worse, appending one
/// would shift the very cells the highlight addresses.
pub(super) fn copy_selection_to_clipboard(app: &mut App, text: &str) {
    if let Err(error) = write_clipboard(text) {
        // The error path may move content and drop the highlight; a silent
        // failure that leaves the user believing they copied is worse.
        app.transcript.push_error(error);
        app.selection = None;
    }
}

fn write_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("clipboard unavailable: {error}"))?;
    clipboard
        .set_text(text.to_owned())
        .map_err(|error| format!("clipboard write failed: {error}"))
}

pub(super) fn read_clipboard() -> Result<ClipboardContent, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("clipboard unavailable: {error}"))?;
    if let Ok(image) = clipboard.get_image() {
        let (width, height) = (
            u32::try_from(image.width).map_err(|_| "clipboard image is too wide".to_owned())?,
            u32::try_from(image.height).map_err(|_| "clipboard image is too tall".to_owned())?,
        );
        let png = encode_png(width, height, &image.bytes)?;
        if png.len() > MAX_CLIPBOARD_IMAGE_BYTES {
            return Err(format!(
                "clipboard image is {} after PNG encoding; the bound is {}",
                render_byte_size(png.len()),
                render_byte_size(MAX_CLIPBOARD_IMAGE_BYTES),
            ));
        }
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
        return Ok(ClipboardContent::Image {
            data_uri: format!("data:image/png;base64,{encoded}"),
            width,
            height,
        });
    }
    match clipboard.get_text() {
        Ok(text) if !text.is_empty() => Ok(ClipboardContent::Text(text)),
        _ => Ok(ClipboardContent::Empty),
    }
}

pub(super) fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    use image::ImageEncoder as _;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| format!("clipboard image could not be encoded: {error}"))?;
    Ok(png)
}

pub(super) fn render_byte_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{}KB", bytes.div_ceil(1024))
    }
}

pub(super) async fn dispatch_prepared_with_materialization(
    app: &mut App,
    session: &smith_runtime::SessionHandle,
    project: &std::path::Path,
    submission: PreparedSubmission,
    target: SubmissionTarget,
) {
    match materialize_prepared_submission(project, &submission).await {
        Ok(input) => dispatch_prepared_submission(app, session, submission, target, input),
        Err(error) => app.restore_submission(submission, error),
    }
}

pub(super) async fn materialize_prepared_submission(
    project: &std::path::Path,
    submission: &PreparedSubmission,
) -> Result<UserInput, String> {
    const MAX_ATTACHMENT_CHARS: usize = 512 * 1024;
    const MAX_ATTACHMENT_LINES: usize = 2_000;

    let workspace = ProjectWorkspace::new(project)
        .map_err(|error| format!("attachments could not resolve the project: {error}"))?;
    let mut input = submission.input_without_files();
    let mut attached_chars = 0usize;
    for path in submission.files() {
        let canonical = workspace
            .resolve(path)
            .map_err(|error| format!("attachment `@{path}` was not sent: {error}"))?;
        let contents = smith_tools::support::read_bounded_from_workspace(
            workspace.clone(),
            std::path::Path::new(&canonical),
            smith_tools::support::MAX_READ_BYTES,
        )
        .await
        .map_err(|error| format!("attachment `@{path}` was not sent: {error}"))?;
        let all = contents.text.lines().collect::<Vec<_>>();
        if all.is_empty() {
            return Err(format!(
                "attachment `@{path}` was not sent: the file is empty"
            ));
        }
        let end = all.len().min(MAX_ATTACHMENT_LINES);
        let width = end.to_string().len();
        let mut rendered = String::new();
        for (index, line) in all[..end].iter().enumerate() {
            let number = index + 1;
            rendered.push_str(&format!("{number:>width$}  {line}\n"));
        }
        attached_chars = attached_chars.saturating_add(rendered.chars().count());
        if attached_chars > MAX_ATTACHMENT_CHARS {
            return Err(format!(
                "prepared attachments exceed the {MAX_ATTACHMENT_CHARS}-character bound"
            ));
        }
        input.parts.push(ContentPart::text(format!(
            "<smith_file_attachment path=\"{path}\" source=\"prepared_read\">\n{rendered}\n</smith_file_attachment>"
        )));
    }
    Ok(input)
}

pub(super) fn dispatch_prepared_submission(
    app: &mut App,
    session: &smith_runtime::SessionHandle,
    submission: PreparedSubmission,
    target: SubmissionTarget,
    input: UserInput,
) {
    match target {
        SubmissionTarget::WholeTurn => {
            dispatch_whole_turn(app, session, submission, input);
        }
        SubmissionTarget::Steer { expected_turn } => {
            dispatch_steer(app, session, submission, expected_turn, input);
        }
    }
}

pub(super) fn dispatch_whole_turn(
    app: &mut App,
    session: &smith_runtime::SessionHandle,
    submission: PreparedSubmission,
    input: UserInput,
) {
    match session.send(input) {
        Ok(handle) => app.whole_turn_dispatched(handle.id().clone(), &submission),
        Err(error) => {
            app.restore_submission(submission, format!("turn submission was rejected: {error}"))
        }
    }
}

pub(super) fn dispatch_steer(
    app: &mut App,
    session: &smith_runtime::SessionHandle,
    submission: PreparedSubmission,
    mut expected_turn: Option<agent_runtime_core::ids::TurnId>,
    mut input: UserInput,
) {
    let mut retried_stale_turn = false;
    loop {
        match session.steer_current_turn(expected_turn.as_ref(), input) {
            Ok(receipt) => {
                app.accept_steer(receipt, submission);
                return;
            }
            Err(rejection) => {
                let message = rejection.to_string();
                let reason = rejection.reason;
                input = rejection.input;
                match reason {
                    SteerRejectionReason::TurnMismatch {
                        active_turn,
                        steerable: true,
                        ..
                    } if !retried_stale_turn => {
                        retried_stale_turn = true;
                        expected_turn = Some(active_turn);
                    }
                    SteerRejectionReason::NoActiveTurn => {
                        dispatch_whole_turn(app, session, submission, input);
                        return;
                    }
                    SteerRejectionReason::TurnMismatch { active_turn, .. }
                    | SteerRejectionReason::NonSteerable { active_turn } => {
                        app.reject_steer_for_followup(Some(active_turn), submission);
                        return;
                    }
                    SteerRejectionReason::TurnClosing { turn } => {
                        app.reject_steer_for_followup(Some(turn), submission);
                        return;
                    }
                    SteerRejectionReason::EmptyInput
                    | SteerRejectionReason::InputTooLarge { .. }
                    | SteerRejectionReason::PendingLimit { .. }
                    | SteerRejectionReason::TurnByteLimit { .. }
                    | SteerRejectionReason::Shutdown => {
                        app.restore_submission(
                            submission,
                            format!("active-turn steering was rejected: {message}"),
                        );
                        return;
                    }
                }
            }
        }
    }
}

pub(super) fn tool_result_text(block: &ToolResultBlock) -> String {
    let text = block
        .content
        .iter()
        .filter_map(ContentPart::as_text)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        "tool completed without text output".to_owned()
    } else {
        text
    }
}

/// Renders a child's turn consumption. An unbounded child shows only what it
/// has used: the unlimited sentinel is an implementation detail, not a number
/// anyone should read.
pub(crate) fn turns_label(used: u32, max: u32) -> String {
    if max == u32::MAX {
        used.to_string()
    } else {
        format!("{used}/{max}")
    }
}

pub(super) fn child_summary_projection(status: &ChildStatus) -> (ChildState, String) {
    let summary = AgentSummary::from(status);
    let state = summary.state.into();
    let durability = summary.durability.label();
    let mut detail = format!(
        "{durability} · session {} · {} turns · {} tokens",
        status.session,
        turns_label(status.turns_used, status.max_turns),
        status.tokens_used
    );
    if summary.resumable {
        detail.push_str(" · resumable");
    }
    if let Some(reason) = &status.incompatibility {
        detail.push_str(" · blocked: ");
        detail.push_str(reason);
    }
    (state, detail)
}

pub(super) fn start_agent(
    host: &HostSession,
    agents: &ResolvedAgent,
    preset: String,
    task: String,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    let Some(profile) = agents.child_profile(&preset).cloned() else {
        let _ = outcomes.send(LocalOutcome::Error(format!(
            "profile `{preset}` is not available for direct-child use"
        )));
        return;
    };
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .cloned()
    else {
        let _ = outcomes.send(LocalOutcome::Error(
            "child delegation is unavailable because the coordinator is not wired".to_owned(),
        ));
        return;
    };
    let model = match (&profile.provider, &profile.model) {
        (Some(_provider), Some(model)) => ChildModelSelection::Explicit {
            provider: Some(smith_runtime::delegation::profile_route_key(
                &profile.name,
                &profile.revision,
            )),
            model: ModelId::new(model.value.clone()),
        },
        (None, None) if profile.legacy => ChildModelSelection::Inherit,
        _ => {
            let _ = outcomes.send(LocalOutcome::Error(format!(
                "profile `{preset}` does not resolve a complete provider/model pair"
            )));
            return;
        }
    };
    let profile_revision = profile.revision.clone();
    let posture = profile.posture.value.as_str();
    tokio::spawn(async move {
        let outcome = coordinator
            .spawn(ChildSpec {
                task: UserInput::text(format!(
                    "Run this bounded task under the preflighted `{preset}` agent profile (revision {profile_revision}, posture {posture}) as a read-only direct child. Do not modify the workspace.\n\nTask:\n{task}"
                )),
                model,
                limits: ChildLimits::turns(1),
                tools: ToolViewScope::ReadOnly,
                workspace: WorkspacePolicy::ReadOnlyView,
            })
            .await;
        let message = match outcome {
            Ok(SpawnOutcome::Spawned { child, .. }) => LocalOutcome::Notice {
                kind: NoticeKind::Agents,
                text: format!("{preset} child {child} started"),
            },
            Ok(SpawnOutcome::Queued { child }) => LocalOutcome::Notice {
                kind: NoticeKind::Agents,
                text: format!("{preset} child {child} queued"),
            },
            Ok(SpawnOutcome::AtCapacity { running, limit }) => LocalOutcome::Error(format!(
                "{preset} child did not start: {running} children are already running (limit {limit})"
            )),
            Err(error) => {
                LocalOutcome::Error(format!("{preset} child did not start: {}", error.message))
            }
        };
        let _ = outcomes.send(message);
    });
}

pub(super) fn follow_up_agent(
    host: &HostSession,
    child_id: String,
    task: String,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .cloned()
    else {
        let _ = outcomes.send(LocalOutcome::Error(
            "child follow-up is unavailable because the coordinator is not wired".to_owned(),
        ));
        return;
    };
    tokio::spawn(async move {
        let child = agent_runtime_core::ids::ChildId::new(child_id);
        let message = match coordinator.follow_up(&child, UserInput::text(task)).await {
            Ok(()) => LocalOutcome::Notice {
                kind: NoticeKind::Agents,
                text: format!("{child} follow-up started · same child session and prior history"),
            },
            Err(error) => LocalOutcome::Error(format!(
                "{child} follow-up did not start: {}",
                error.message
            )),
        };
        let _ = outcomes.send(message);
    });
}

pub(super) fn resume_agent(
    host: &HostSession,
    child_id: String,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .cloned()
    else {
        let _ = outcomes.send(LocalOutcome::Agent(Box::new(AgentReport::Resume(
            AgentResumeReport::Unavailable,
        ))));
        return;
    };
    tokio::spawn(async move {
        let child = agent_runtime_core::ids::ChildId::new(child_id);
        let report = match coordinator.resume(&child).await {
            Ok(()) => AgentResumeReport::Started {
                child: child.to_string(),
            },
            Err(error) => AgentResumeReport::Failed {
                child: child.to_string(),
                error: error.message,
            },
        };
        let _ = outcomes.send(LocalOutcome::Agent(Box::new(AgentReport::Resume(report))));
    });
}

pub(super) fn start_review(
    host: &HostSession,
    project: &std::path::Path,
    scope: String,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .cloned()
    else {
        let _ = outcomes.send(LocalOutcome::Review(Box::new(ReviewReport::Start(
            ReviewStartReport::Unavailable,
        ))));
        return;
    };
    let view = match GitChanges::discover(project).and_then(|git| git.inspect(Some(scope.as_str())))
    {
        Ok(view) => view,
        Err(error) => {
            let _ = outcomes.send(LocalOutcome::Review(Box::new(ReviewReport::Error(
                error.message,
            ))));
            return;
        }
    };
    let task = format!(
        "Review this bounded Git diff. Do not modify the workspace. Report only actionable \
         findings, ordered by severity, with file and line evidence. If there are no findings, \
         say so explicitly.\n\nScope: {}\n\n{}",
        view.title, view.content
    );
    tokio::spawn(async move {
        let outcome = coordinator
            .spawn(ChildSpec {
                task: UserInput::text(task),
                model: ChildModelSelection::Inherit,
                limits: ChildLimits::turns(1),
                tools: ToolViewScope::ReadOnly,
                workspace: WorkspacePolicy::ReadOnlyView,
            })
            .await;
        let report = match outcome {
            Ok(SpawnOutcome::Spawned { child, .. }) => ReviewStartReport::Started {
                child: child.to_string(),
            },
            Ok(SpawnOutcome::Queued { child }) => ReviewStartReport::Queued {
                child: child.to_string(),
            },
            Ok(SpawnOutcome::AtCapacity { running, limit }) => {
                ReviewStartReport::AtCapacity { running, limit }
            }
            Err(error) => ReviewStartReport::Failed(error.message),
        };
        let _ = outcomes.send(LocalOutcome::Review(Box::new(ReviewReport::Start(report))));
    });
}
