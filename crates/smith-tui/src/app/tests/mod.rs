#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::approval::{
        ApprovalDecision, ApprovalOrigin, ApprovalPolicy, ApprovalRequest,
    };
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::clock::{Deadline, SystemClock, Timestamp};
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::error::RuntimeError;
    use agent_runtime_core::event::{
        ChildPhase, ChildRecoveryState, EstimationConfidence, EventEnvelope, GoalUpdateCause,
        PlanItemStatus, RuntimeEvent, TurnFinish,
    };
    use agent_runtime_core::goal::{
        GoalProjection, GoalStatus, GoalTokenUsage, GoalUsageProvenance,
    };
    use agent_runtime_core::ids::{
        AttemptId, ChildId, EventId, GoalId, InteractionRequestId, QuestionId, RequestId,
        SessionId, SteerId, ToolCallId, TurnId,
    };
    use agent_runtime_core::interaction::InteractionSensitivity;
    use agent_runtime_core::manifest::{ActivatedCapability, SegmentKind};
    use agent_runtime_core::provider::ModelId;
    use agent_runtime_core::tool::{PreparedToolCall, ToolCallDisplay, ToolEffects};
    use agent_runtime_core::usage::{
        CounterKind, Provenance, UsageDelta, UsageRecord, UsageSource,
    };
    use crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use smith_host::approval::InteractiveApproval;

    use crate::app::MouseOutcome;
    use crate::commands;
    use crate::questionnaire::{QuestionnaireChoice, QuestionnaireForm, QuestionnaireQuestion};
    use crate::transcript::Block;
    use ratatui::layout::Rect;

    fn fingerprint(seed: &str) -> agent_runtime_registry::Fingerprint {
        agent_runtime_registry::Fingerprint::of(seed)
    }

    fn app() -> App {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.set_resources(RuntimeResources {
            models: vec![ResourceEntry::new(
                "local/model-2",
                "local/model-2",
                "configured limits",
            )],
            providers: vec![ResourceEntry::new(
                "local",
                "local",
                "openai-compatible · 1 model",
            )],
            connections: vec![
                ResourceEntry::new("local", "local", "API key · connected"),
                ResourceEntry::new("openrouter", "OpenRouter", "API key · available"),
                ResourceEntry::new("chatgpt", "ChatGPT", "Smith OAuth · experimental"),
            ],
            disconnections: vec![ResourceEntry::new(
                "local",
                "local",
                "API key · connected",
            )],
            profiles: vec![ResourceEntry::new("work", "work", "local/model-2")],
            sessions: vec![ResourceEntry::new(
                "session-7",
                "session-7 · recent work",
                "2 turns · local/model-2",
            )],
            thinking: vec![
                ResourceEntry::new("default", "provider default", "clear override"),
                ResourceEntry::new("on", "on", "enable thinking"),
                ResourceEntry::new("off", "off", "disable thinking"),
            ],
            efforts: vec![
                ResourceEntry::new("default", "provider default", "clear override"),
                ResourceEntry::new("low", "low", "advertised effort"),
                ResourceEntry::new("high", "high", "advertised effort"),
            ],
            current_session: Some("current-session".into()),
            ..RuntimeResources::default()
        });
        app
    }

    fn expect_whole_submission(action: Option<Action>) -> PreparedSubmission {
        match action {
            Some(Action::Submit {
                submission,
                target: SubmissionTarget::WholeTurn,
            }) => submission,
            other => panic!("expected a prepared whole-turn submission, got {other:?}"),
        }
    }

    fn event(payload: RuntimeEvent) -> EventEnvelope {
        event_at(Timestamp::ZERO, payload)
    }

    fn event_at(timestamp: Timestamp, payload: RuntimeEvent) -> EventEnvelope {
        EventEnvelope::new(
            0,
            EventId::new("e"),
            SessionId::new("s"),
            None,
            timestamp,
            payload,
        )
    }

    fn turn_event(turn: &str, payload: RuntimeEvent) -> EventEnvelope {
        EventEnvelope::new(
            0,
            EventId::new(format!("event-{turn}")),
            SessionId::new("s"),
            Some(TurnId::new(turn)),
            Timestamp::ZERO,
            payload,
        )
    }

    fn text_delta(text: &str) -> RuntimeEvent {
        RuntimeEvent::TextDelta {
            request: RequestId::new("request-fixture"),
            attempt: AttemptId::new("attempt-fixture"),
            text: text.to_owned(),
        }
    }

    fn goal_projection(status: GoalStatus, charged_tokens: Option<u64>) -> GoalProjection {
        GoalProjection {
            id: GoalId::new("goal-1"),
            generation: 3,
            objective: "Ship persistent goals".into(),
            status,
            token_budget: Some(100),
            usage: GoalTokenUsage {
                charged_tokens,
                provenance: if charged_tokens.is_some() {
                    GoalUsageProvenance::ProviderReported
                } else {
                    GoalUsageProvenance::Unknown
                },
                active_elapsed_ms: 1_250,
            },
            created_at: Timestamp(10),
            updated_at: Timestamp(20),
            stopped_reason: None,
        }
    }

    fn commit_output() -> RuntimeEvent {
        RuntimeEvent::ProviderAttemptOutputCommitted {
            request: RequestId::new("request-fixture"),
            attempt: AttemptId::new("attempt-fixture"),
        }
    }

    fn discard_output() -> RuntimeEvent {
        RuntimeEvent::ProviderAttemptOutputDiscarded {
            request: RequestId::new("request-fixture"),
            attempt: AttemptId::new("attempt-fixture"),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    fn questionnaire_form(request_id: &str, deadline: Deadline) -> QuestionnaireForm {
        QuestionnaireForm::new(
            request_id,
            vec![QuestionnaireQuestion::new(
                "direction",
                "Direction",
                "Which direction should Smith take?",
                vec![
                    QuestionnaireChoice::new("minimal", "Minimal"),
                    QuestionnaireChoice::new("complete", "Complete"),
                ],
            )],
            deadline,
        )
        .expect("valid questionnaire fixture")
    }

    fn type_text(app: &mut App, text: &str) {
        for ch in text.chars() {
            app.on_key(key(KeyCode::Char(ch)));
        }
    }

    async fn prompt(tool: &str) -> ApprovalPrompt {
        prompt_with(tool, serde_json::json!({"command": "rm -rf build"})).await
    }

    async fn prompt_with(tool: &str, arguments: serde_json::Value) -> ApprovalPrompt {
        pending_prompt_with(tool, arguments).await.0
    }

    async fn pending_prompt_with(
        tool: &str,
        arguments: serde_json::Value,
    ) -> (ApprovalPrompt, tokio::task::JoinHandle<ApprovalDecision>) {
        pending_prompt_with_deadline(tool, arguments, Deadline::never()).await
    }

    async fn pending_prompt_with_deadline(
        tool: &str,
        arguments: serde_json::Value,
        deadline: Deadline,
    ) -> (ApprovalPrompt, tokio::task::JoinHandle<ApprovalDecision>) {
        // The simplest way to obtain a real prompt is to drive the policy the
        // runtime would call.
        let (policy, mut requests) = InteractiveApproval::new(1);
        let tool = tool.to_owned();
        let decision = tokio::spawn(async move {
            let effects = ToolEffects::read_only().with_write("/repo");
            let (permissions, resource) = effects.authorization_request(&tool, "/repo");
            let request = ApprovalRequest::new(
                PreparedToolCall::new(
                    ToolCallId::new("c1"),
                    &tool,
                    arguments,
                    permissions,
                    resource,
                    effects,
                    ToolCallDisplay::new(format!("Run {tool}")),
                ),
                deadline,
                ApprovalOrigin::new(
                    agent_runtime_core::ids::SessionId::new("session-1"),
                    RequestId::new("request-1"),
                ),
            );
            policy.decide(&request).await
        });
        let prompt = requests.recv().await.expect("an approval prompt");
        (prompt, decision)
    }

    fn agent_first_app() -> App {
        let mut app = app();
        app.status.switch_model(Some("zai".to_owned()), "glm-5.2");
        app.status.set_agent("build");
        app.set_resources(RuntimeResources {
            files: vec![ResourceEntry::new(
                "file:src/lib.rs",
                "src/lib.rs",
                "file · 42 bytes",
            )],
            child_agents: vec![ResourceEntry::new(
                "agent:review",
                "review",
                "child profile · review · zai/glm-5.2 · ctx 131072 · custom instructions configured",
            )],
            main_profiles: vec![
                ResourceEntry::new("build", "build", "coding").active(true),
                ResourceEntry::new("plan", "plan", "read-only planning"),
                ResourceEntry::new("review", "review", "read-only review"),
            ],
            ..app.resources.clone()
        });
        app
    }

    include!("reducer.rs");
    include!("pending_input.rs");
    include!("input.rs");
    include!("prompts.rs");
    include!("resources.rs");
    include!("child_lifecycle.rs");
}
