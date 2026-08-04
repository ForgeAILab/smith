//! Prepared-input materialization, dispatch, and agent/review actions.

use super::*;

pub(super) fn start_local_shell(
    session: smith_runtime::SessionHandle,
    command: String,
    timeout_ms: u64,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    tokio::spawn(async move {
        let outcome = session
            .run_local_tool(
                "shell",
                serde_json::json!({
                    "command": command,
                    "cwd": ".",
                    "timeout_ms": timeout_ms,
                }),
                timeout_ms,
            )
            .await;
        let result = match outcome {
            Ok(block) => LocalOutcome::Shell {
                content: tool_result_text(&block),
                is_error: block.is_error,
            },
            Err(error) => LocalOutcome::Error(format!("shell action failed: {error}")),
        };
        let _ = outcomes.send(result);
    });
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
    match read_clipboard() {
        Ok(ClipboardContent::Image {
            data_uri,
            width,
            height,
        }) => {
            if app.can_attach_image() {
                app.attach_image(data_uri, width, height);
            }
        }
        Ok(ClipboardContent::Text(text)) => app.on_paste(&text),
        Ok(ClipboardContent::Empty) => {
            app.transcript.push_notice("clipboard", "nothing to attach");
        }
        Err(error) => app.transcript.push_error(error),
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
        let contents = smith_tools::support::read_bounded(
            std::path::Path::new(&canonical),
            smith_tools::support::MAX_READ_BYTES,
        )
        .await
        .map_err(|error| format!("attachment `@{path}` was not sent: {error}"))?;
        let all = contents.lines().collect::<Vec<_>>();
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

pub(super) fn child_summary_projection(status: &ChildStatus) -> (&'static str, String) {
    let state = match &status.state {
        ChildState::Running => "working",
        ChildState::Idle => "idle",
        ChildState::Interrupted { .. } => "interrupted",
        ChildState::Stopped { .. } => "stopped",
        ChildState::Failed => "failed",
        ChildState::Expired => "expired",
    };
    let durability = match status.durability {
        ChildDurability::Ephemeral => "ephemeral",
        ChildDurability::Durable => "durable",
    };
    let mut detail = format!(
        "{durability} · session {} · {}/{} turns · {} tokens",
        status.session, status.turns_used, status.max_turns, status.tokens_used
    );
    if status.resumable() {
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
                source: "agents",
                text: format!("{preset} child {child} started"),
            },
            Ok(SpawnOutcome::Queued { child }) => LocalOutcome::Notice {
                source: "agents",
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
                source: "agents",
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
        let _ = outcomes.send(LocalOutcome::Error(
            "child resume is unavailable because the coordinator is not wired".to_owned(),
        ));
        return;
    };
    tokio::spawn(async move {
        let child = agent_runtime_core::ids::ChildId::new(child_id);
        let message = match coordinator.resume(&child).await {
            Ok(()) => LocalOutcome::Notice {
                source: "agents",
                text: format!("{child} exact checkpoint resume started · no new child task"),
            },
            Err(error) => LocalOutcome::Error(format!("{child} did not resume: {}", error.message)),
        };
        let _ = outcomes.send(message);
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
        let _ = outcomes.send(LocalOutcome::Error(
            "read-only review is unavailable because delegation is not wired".to_owned(),
        ));
        return;
    };
    let view = match GitChanges::discover(project).and_then(|git| git.inspect(Some(scope.as_str())))
    {
        Ok(view) => view,
        Err(error) => {
            let _ = outcomes.send(LocalOutcome::Error(error.message));
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
        let message = match outcome {
            Ok(SpawnOutcome::Spawned { child, .. }) => LocalOutcome::Notice {
                source: "review",
                text: format!("read-only reviewer {child} started"),
            },
            Ok(SpawnOutcome::Queued { child }) => LocalOutcome::Notice {
                source: "review",
                text: format!("read-only reviewer {child} queued"),
            },
            Ok(SpawnOutcome::AtCapacity { running, limit }) => LocalOutcome::Error(format!(
                "review did not start: {running} children are already running (limit {limit})"
            )),
            Err(error) => LocalOutcome::Error(format!("review did not start: {}", error.message)),
        };
        let _ = outcomes.send(message);
    });
}
