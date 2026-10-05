use super::*;

fn live_findings_agent_snapshot(
    resumable: bool,
    result: Option<&str>,
) -> smith_client::agent_report::AgentSnapshot {
    use smith_client::agent_report::{AgentSnapshot, AgentSummary, ChildDurability, ChildState};
    AgentSnapshot {
        summary: AgentSummary {
            child: "child-1".to_owned(),
            durability: ChildDurability::Durable,
            state: ChildState::Idle,
            resumable,
            turns_used: 1,
            max_turns: None,
            tokens_used: 3_100,
        },
        session: "child-session-1".to_owned(),
        workspace: "read only".to_owned(),
        incompatibility: None,
        last_result: result.map(str::to_owned),
    }
}

#[test]
fn live_findings_agent_list_names_the_command_and_labels_compact_counts() {
    use smith_client::agent_report::AgentReport;
    for turns in [1, 2] {
        let mut snapshot = live_findings_agent_snapshot(false, None);
        snapshot.summary.turns_used = turns;
        let mut app = App::new("model", "project");
        app.show_local_report(LocalResult::Agent(Box::new(AgentReport::List(vec![
            snapshot.summary,
        ]))));
        let rows = transcript_lines(&app, Theme::new().without_color(), 100);
        assert_eq!(rows[0].to_string(), "/agent");
        let text = rows
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains(if turns == 1 {
                "1 turn · 3.1k tokens"
            } else {
                "2 turns · 3.1k tokens"
            }),
            "{text}"
        );
        assert!(!text.contains("1 turns"), "{text}");
    }
}

#[test]
fn live_findings_inspector_states_facts_once_and_renders_result_markdown() {
    use smith_client::agent_report::exact_resume_label;
    for resumable in [false, true] {
        for width in [42, 100] {
            for theme in [Theme::new(), Theme::new().without_color()] {
                let snapshot = live_findings_agent_snapshot(
                    resumable,
                    Some(
                        "# Answer\n\n**4 entries** in `src/lib.rs` are ready for review after checking the child output and preserving the inspector's indentation on every wrapped row.",
                    ),
                );
                let mut app = App::new("model", "project");
                app.restore_child(
                    "child-1",
                    crate::app::ChildState::Idle,
                    Some("durable · session child-session-1 · 1 turns · 3100 tokens".to_owned()),
                );
                app.inspect_child("child-1");
                app.set_inspected_detail("child-1", Some(snapshot));
                let rows = transcript_lines(&app, theme, width);
                let text = rows
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
                for fact in [
                    "session child-session-1",
                    "durable",
                    "idle",
                    "1 turn",
                    "3.1k tokens",
                    "read only",
                    exact_resume_label(resumable),
                    "continue: type a follow-up below",
                    "result",
                    "Answer",
                    "4 entries",
                    "src/lib.rs",
                ] {
                    assert_eq!(words.matches(fact).count(), 1, "{fact}: {text}");
                }
                assert_eq!(
                    words.contains("exact recovery: /agent resume child-1"),
                    resumable,
                    "{text}"
                );
                assert!(!text.contains("no activity"), "{text}");
                assert!(
                    !text.contains("**") && !text.contains('`') && !text.contains("# Answer"),
                    "{text}"
                );
                let result_rows = text.split_once("  result\n").expect("result label").1;
                assert!(!result_rows.contains('●'), "{text}");
                assert!(
                    result_rows
                        .lines()
                        .filter(|row| !row.is_empty())
                        .all(|row| row.starts_with("  ")),
                    "{text}"
                );
                assert!(
                    result_rows.lines().count() > 3,
                    "result should wrap: {text}"
                );
                assert!(rows.iter().all(|row| row.width() <= usize::from(width)));
                assert!(
                    rows.iter()
                        .flat_map(|row| &row.spans)
                        .any(|span| span.content == "4 entries"
                            && span.style.add_modifier.contains(Modifier::BOLD)),
                    "result emphasis should retain Markdown styling"
                );
                if theme != Theme::new().without_color() {
                    assert!(
                        rows.iter()
                            .flat_map(|row| &row.spans)
                            .any(|span| span.content == "src/lib.rs"
                                && span.style.fg == theme.style(crate::theme::Tone::Code).fg)
                    );
                }
            }
        }
    }
}

#[test]
fn live_findings_inspector_reports_no_activity_only_without_activity_or_a_result() {
    let mut app = App::new("model", "project");
    app.restore_child("child-1", crate::app::ChildState::Idle, None);
    app.inspect_child("child-1");
    for result in [None, Some("An answer.")] {
        app.set_inspected_detail("child-1", Some(live_findings_agent_snapshot(false, result)));
        let text = transcript_lines(&app, Theme::new(), 100)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            text.matches("no activity recorded in this session").count(),
            usize::from(result.is_none()),
            "{text}"
        );
    }
    app.set_inspected_detail("child-1", Some(live_findings_agent_snapshot(false, None)));
    app.apply_child(
        "child-1",
        &event(RuntimeEvent::ExternalText {
            text: "Recorded activity.".to_owned(),
        }),
    );
    let text = transcript_lines(&app, Theme::new(), 100)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("no activity"), "{text}");
}

#[test]
fn child_inspector_and_agents_agree_with_plain_workspace_and_resume_words() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;
    use smith_client::agent_report::{AgentReport, AgentSnapshot, render_plain};
    use smith_runtime::{ChildDurability, ChildState, ChildStatus};

    for (workspace, expected_workspace) in [
        (WorkspacePolicy::SharedProject, "shared"),
        (
            WorkspacePolicy::ExplicitDirectory {
                path: "/repo/child".to_owned(),
            },
            "/repo/child",
        ),
        (WorkspacePolicy::IsolatedWorktree, "isolated worktree"),
        (WorkspacePolicy::ReadOnlyView, "read only"),
    ] {
        for (resumable, expected_resume) in [
            (true, "exact resume available"),
            (false, "no exact checkpoint"),
        ] {
            let status = ChildStatus {
                child: ChildId::new("child"),
                parent: SessionId::new("parent"),
                session: SessionId::new("session"),
                durability: ChildDurability::Durable,
                state: ChildState::Interrupted { resumable },
                workspace: workspace.clone(),
                turns_used: 1,
                max_turns: 5,
                tokens_used: 2,
                last_result: None,
                last_artifacts: Vec::new(),
                updated_at: Timestamp(0),
                incompatibility: None,
                last_error: None,
            };
            let snapshot = AgentSnapshot::from(&status);
            assert_eq!(snapshot.workspace, expected_workspace);
            for theme in [Theme::new(), Theme::new().without_color()] {
                let mut app = App::new("model", "project");
                app.apply(&event(RuntimeEvent::ChildSpawned {
                    child: status.child.clone(),
                    workspace: workspace.clone(),
                    max_turns: 5,
                    max_tokens: None,
                    deadline_ms: None,
                }));
                app.inspect_child("child");
                app.set_inspected_detail("child", Some(snapshot.clone()));
                let inspector = transcript_lines(&app, theme, 240)
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                let plain = render_plain(&AgentReport::Inspector(snapshot.clone()));
                let card = plain
                    .lines()
                    .map(|line| format!("  {line}"))
                    .collect::<Vec<_>>();
                assert!(
                    inspector
                        .windows(card.len())
                        .any(|rows| rows == card.as_slice())
                );
                assert!(inspector.iter().any(|line| line.trim() == expected_resume));

                for report in [
                    AgentReport::Inspector(snapshot.clone()),
                    AgentReport::List(vec![snapshot.summary.clone()]),
                ] {
                    let plain = render_plain(&report);
                    let mut app = App::new("model", "project");
                    app.show_local_report(LocalResult::Agent(Box::new(report)));
                    let rows = transcript_lines(&app, theme, 240);
                    let rendered = rows
                        .iter()
                        .skip(1)
                        .map(|row| row.to_string().trim_start().to_owned())
                        .collect::<Vec<_>>()
                        .join("\n");
                    assert_eq!(rendered, plain);
                    assert!(!rendered.contains("resumable true"));
                    assert!(!rendered.contains("resumable false"));
                }
            }
        }
    }
}

// -- The agent row adopts its child's identity (tool-call-display group 3) --

#[test]
fn a_spawn_row_adopts_its_childs_identity_and_survives_the_completion_reprojection() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;

    let mut app = App::new("gpt-5.3", "~/work/api");
    app.status.set_agent("build");

    let spawn_args = serde_json::json!({
        "action": "spawn",
        "task": "explore the autoloads and data layer",
        "tools": "read_only",
        "workspace": "shared"
    });
    let display = smith_tools::project_tool_call_display("agent", &spawn_args)
        .expect("reviewed spawn projection");

    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("spawn-1"),
        name: "agent".to_owned(),
        argument_keys: vec![
            "action".into(),
            "task".into(),
            "tools".into(),
            "workspace".into(),
        ],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    // Exactly what `tui_driver::run_tui`'s root-events branch does: note
    // the pending spawn, then set the display — in that order, since the
    // queue push only ever happens once, at request time.
    app.note_pending_spawn("spawn-1", &display);
    app.set_tool_display("spawn-1", display);

    app.apply(&event(RuntimeEvent::ChildSpawned {
        child: ChildId::new("child-9"),
        workspace: WorkspacePolicy::SharedProject,
        max_turns: u32::MAX,
        max_tokens: None,
        deadline_ms: None,
    }));

    let screen = render(&app, 220, 16, Theme::new().without_color());
    // The workspace appears exactly once, from the projector, which reads
    // it off the call's own argument. Enrichment adds only what the row
    // does not already say — the child id, and a turn ceiling when the
    // child has one.
    assert!(
        screen.contains(
            "Agent(spawn · \"explore the autoloads and data layer\" · tools read only \
             · workspace shared · child-9 · profile build (inherited))"
        ),
        "{screen}"
    );
    // The match above is the complete parenthesized invocation, from
    // `Agent(` to its closing paren, so it also pins the absence of a
    // second workspace qualifier. `describe_workspace`'s own spelling
    // (`shared project workspace`) still appears elsewhere on screen —
    // the delegated-work panel row carries it, which is a different
    // surface and its own fact.
    assert!(
        !screen.contains("up to"),
        "an unbounded child must not claim a turn ceiling: {screen}"
    );
    // No second row for the same spawn.
    assert_eq!(screen.matches("Agent(spawn").count(), 1, "{screen}");

    // The trap: the host re-projects `display` from canonical arguments
    // when the tool completes. That must not drop the enrichment.
    app.apply(&event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("spawn-1"),
        name: "agent".to_owned(),
        is_error: false,
    }));
    let reprojected = smith_tools::project_tool_call_display("agent", &spawn_args)
        .expect("reviewed spawn projection");
    app.set_tool_display("spawn-1", reprojected);

    let after_completion = render(&app, 220, 16, Theme::new().without_color());
    assert!(
        after_completion.contains("child-9")
            && after_completion.contains("shared project workspace")
            && after_completion.contains("profile build (inherited)"),
        "enrichment must survive the tool-completed re-projection: {after_completion}"
    );
}

#[test]
fn a_spawn_that_selected_a_profile_is_not_double_labelled_and_keeps_its_turn_ceiling() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;

    let mut app = App::new("gpt-5.3", "~/work/api");
    let spawn_args = serde_json::json!({
        "action": "spawn",
        "task": "build the feature",
        "tools": "all",
        "workspace": "shared",
        "profile": "explore"
    });
    let display = smith_tools::project_tool_call_display("agent", &spawn_args)
        .expect("reviewed spawn projection");
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("spawn-2"),
        name: "agent".to_owned(),
        argument_keys: vec![
            "action".into(),
            "task".into(),
            "tools".into(),
            "workspace".into(),
            "profile".into(),
        ],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    app.note_pending_spawn("spawn-2", &display);
    app.set_tool_display("spawn-2", display);
    app.apply(&event(RuntimeEvent::ChildSpawned {
        child: ChildId::new("child-explore"),
        workspace: WorkspacePolicy::SharedProject,
        max_turns: 6,
        max_tokens: None,
        deadline_ms: None,
    }));

    let screen = render(&app, 220, 16, Theme::new().without_color());
    assert_eq!(
        screen.matches("profile explore").count(),
        1,
        "the projector's own profile qualifier must not be duplicated: {screen}"
    );
    assert!(
        !screen.contains("(inherited)"),
        "a selected profile is not inherited: {screen}"
    );
    assert!(screen.contains("child-explore"), "{screen}");
    assert!(screen.contains("up to 6 turns"), "{screen}");
}
