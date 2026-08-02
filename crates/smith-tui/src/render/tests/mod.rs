#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::registry::Permission;
    use agent_runtime_core::approval::{ApprovalOrigin, ApprovalPolicy, ApprovalRequest};
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::clock::{Deadline, SystemClock, Timestamp};
    use agent_runtime_core::content::{ContentPart, Message, ToolCall, ToolResultBlock};
    use agent_runtime_core::event::{
        EstimationConfidence, EventEnvelope, PlanItemProjection, PlanItemStatus, PlanSensitivity,
        RuntimeEvent, TurnFinish,
    };
    use agent_runtime_core::goal::{
        GoalProjection, GoalStatus, GoalTokenUsage, GoalUsageProvenance,
    };
    use agent_runtime_core::ids::{
        AttemptId, EventId, GoalId, RequestId, SessionId, SteerId, ToolCallId, TurnId,
    };
    use agent_runtime_core::manifest::SegmentKind;
    use agent_runtime_core::security::{PermissionSet, SecurityResource};
    use agent_runtime_core::steer::SteerReceipt;
    use agent_runtime_core::tool::{PreparedToolCall, ToolCallDisplay, ToolEffects};
    use agent_runtime_core::usage::{
        CounterKind, Provenance, UsageDelta, UsageRecord, UsageSource,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Modifier};
    use unicode_width::UnicodeWidthStr;

    use crate::questionnaire::{QuestionnaireChoice, QuestionnaireForm, QuestionnaireQuestion};
    use crate::transcript::ToolStatus;

    fn render(app: &App, width: u16, height: u16, theme: Theme) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| draw(frame, app, theme))
            .expect("a frame");
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render_synced(app: &mut App, width: u16, height: u16, theme: Theme) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| draw_synced(frame, app, theme))
            .expect("a frame");
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
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

    fn conversation() -> App {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.transcript.push_user("explain the retry policy");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::TextDelta {
            request: RequestId::new("request-1"),
            attempt: AttemptId::new("attempt-1"),
            text: "The retry policy classifies failures.".into(),
        }));
        app.apply(&event(RuntimeEvent::ProviderAttemptOutputCommitted {
            request: RequestId::new("request-1"),
            attempt: AttemptId::new("attempt-1"),
        }));
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("c1"),
            name: "read".into(),
            argument_keys: vec!["path".into()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.set_tool_display(
            "c1",
            smith_tools::project_tool_call_display(
                "read",
                &serde_json::json!({"path": "src/retry.rs"}),
            )
            .expect("reviewed read projection"),
        );
        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("c1"),
            name: "read".into(),
            is_error: false,
        }));
        app.apply(&event(RuntimeEvent::ContextPlanned {
            context: agent_runtime_registry::Fingerprint::of("context"),
            cache_plan: agent_runtime_registry::Fingerprint::of("cache"),
            segment_count: 2,
            totals: std::collections::BTreeMap::from([
                (SegmentKind::new("history"), 10_000),
                (SegmentKind::new("tool_schema"), 2_400),
            ]),
            input_tokens: 12_400,
            input_budget_tokens: 100_000,
            reserved_tokens: 28_000,
            confidence: EstimationConfidence::Exact,
        }));
        app.apply(&event(RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance::default(),
                delta: UsageDelta::new().with(CounterKind::InputUncached, 12_400),
            },
        }));
        app
    }

    /// Drives the real approval policy to obtain a real prompt.
    async fn prompt(
        tool: &str,
        arguments: serde_json::Value,
    ) -> smith_host::approval::ApprovalPrompt {
        use smith_host::approval::InteractiveApproval;

        let (policy, mut requests) = InteractiveApproval::new(1);
        let tool = tool.to_owned();
        tokio::spawn(async move {
            let effects = if tool == "shell" {
                ToolEffects::read_only()
                    .with_write("/repo")
                    .with_spawn()
                    .with_network()
            } else {
                ToolEffects::read_only().with_write("/repo")
            };
            let (permissions, _) = effects.authorization_request(&tool, "/repo");
            let segments = arguments
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(|path| {
                    path.split('/')
                        .filter(|segment| !segment.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let request = ApprovalRequest::new(
                PreparedToolCall::new(
                    ToolCallId::new("c1"),
                    &tool,
                    arguments,
                    permissions,
                    SecurityResource::filesystem("/repo", segments),
                    effects,
                    ToolCallDisplay::new(format!("Run {tool}")),
                ),
                Deadline::after(&SystemClock, 60_000),
                ApprovalOrigin::new(
                    agent_runtime_core::ids::SessionId::new("session-1"),
                    agent_runtime_core::ids::RequestId::new("request-1"),
                ),
            );
            let _ = policy.decide(&request).await;
        });
        requests.recv().await.expect("a prompt")
    }

    /// An approval waiting on an `edit` of `src/retry.rs`.
    async fn edit_approval(old: &str, new: &str) -> App {
        let mut app = conversation();
        app.present_approval(
            prompt(
                "edit",
                serde_json::json!({
                    "path": "src/retry.rs",
                    "old_string": old,
                    "new_string": new,
                }),
            )
            .await,
        );
        app
    }

    /// Asserts every expected fragment appears, reporting the whole screen on
    /// failure so a broken layout is readable rather than a boolean.
    fn insta_like(screen: &str, expected: &[&str]) {
        for fragment in expected {
            assert!(
                screen.contains(fragment),
                "expected to find `{fragment}` in:\n{screen}"
            );
        }
    }
    include!("layout.rs");
    include!("transcript.rs");
    include!("composer.rs");
    include!("modal.rs");
}
