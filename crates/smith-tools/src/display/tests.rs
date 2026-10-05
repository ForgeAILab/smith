use super::*;
use serde_json::json;

fn invocation(name: &str, arguments: Value) -> String {
    project_tool_call_display(name, &arguments)
        .expect("the call should have a reviewed projection")
        .invocation()
}

fn agent_invocation(name: &str, detail: Value) -> String {
    project_external_tool_call_display(name, &detail)
        .expect("the agent call should have a reviewed projection")
        .invocation()
}

/// The details here are verbatim shapes taken from a real
/// `claude --output-format stream-json` run and a real `codex exec --json`
/// run, so a vendor change shows up as a failing test rather than as a
/// silently value-free row.
#[test]
fn an_installed_agent_s_own_tools_project_their_reviewed_fields() {
    assert_eq!(
        agent_invocation("Read", json!({"file_path": "/repo/README.md"})),
        "Read(/repo/README.md)"
    );
    assert_eq!(
        agent_invocation(
            "Bash",
            json!({"command": "echo hi", "description": "Print hi"})
        ),
        "Bash(echo hi)"
    );
    assert_eq!(
        agent_invocation(
            "Edit",
            json!({"file_path": "src/lib.rs", "old_string": "a", "new_string": "b", "replace_all": true})
        ),
        "Update(src/lib.rs · replace all)"
    );
    assert_eq!(
        agent_invocation(
            "Grep",
            json!({"pattern": "fn main", "path": "src", "output_mode": "content"})
        ),
        "Grep(fn main · src · content)"
    );
    assert_eq!(
        agent_invocation("TodoWrite", json!({"todos": [{}, {}, {}]})),
        "Todo Write(3 items)"
    );
    assert_eq!(
        agent_invocation(
            "command_execution",
            json!({
                "id": "item_7",
                "type": "command_execution",
                "command": "/bin/zsh -lc 'echo hi'",
                "aggregated_output": "hi\n",
                "exit_code": 0,
                "status": "completed"
            })
        ),
        "Command(/bin/zsh -lc 'echo hi')"
    );
    assert_eq!(
        agent_invocation(
            "file_change",
            json!({
                "id": "item_8",
                "type": "file_change",
                "changes": [
                    {"path": "/repo/note.txt", "kind": "add"},
                    {"path": "/repo/other.txt", "kind": "update"}
                ],
                "status": "completed"
            })
        ),
        "File Change(/repo/note.txt · add · +1 more)"
    );
}

#[test]
fn an_agent_tool_or_shape_without_a_reviewed_projection_falls_back() {
    // A tool this build has never heard of.
    assert!(project_external_tool_call_display("Sorcery", &json!({"spell": "x"})).is_none());
    // The right tool, an ill-typed field.
    assert!(project_external_tool_call_display("Read", &json!({"file_path": 42})).is_none());
    // The right tool, the field missing.
    assert!(project_external_tool_call_display("Bash", &json!({"description": "hi"})).is_none());
    // A detail that is not an object at all.
    assert!(project_external_tool_call_display("Read", &json!("src/lib.rs")).is_none());
}

#[test]
fn agent_tool_call_values_are_bounded_and_control_stripped_like_built_ins() {
    let command = format!("echo {}", "x".repeat(400));
    let projected = agent_invocation("Bash", json!({"command": command}));
    assert!(projected.chars().count() < 200, "{projected}");
    assert!(projected.ends_with("…)"), "{projected}");
    assert_eq!(
        agent_invocation("Read", json!({"file_path": "src/\u{202e}gnp.rs"})),
        "Read(src/ gnp.rs)"
    );
}

#[test]
fn an_agent_tool_result_is_read_from_the_shapes_the_clis_report() {
    // Claude Code reports a string…
    assert_eq!(
        external_tool_result_text(&json!("1\thello\n2\t")).as_deref(),
        Some("1 hello\n2 ")
    );
    // …or a list of content blocks.
    assert_eq!(
        external_tool_result_text(&json!([
            {"type": "text", "text": "first"},
            {"type": "image", "source": {}},
            {"type": "text", "text": "second"}
        ]))
        .as_deref(),
        Some("first\nsecond")
    );
    // Codex reports its aggregated output, and reports nothing at all for
    // an item with no output.
    assert_eq!(
        external_tool_result_text(&json!("hi\n")).as_deref(),
        Some("hi\n")
    );
    assert!(external_tool_result_text(&Value::Null).is_none());
    assert!(external_tool_result_text(&json!("   \n ")).is_none());
    assert!(external_tool_result_text(&json!([{"type": "image", "source": {}}])).is_none());
}

#[test]
fn every_built_in_projects_only_its_reviewed_target_and_qualifiers() {
    assert_eq!(
        invocation(
            "read",
            json!({"path": "src/lib.rs", "offset": 10, "limit": 5})
        ),
        "Read(src/lib.rs · offset 10 · limit 5)"
    );
    assert_eq!(
        invocation(
            "list",
            json!({"path": "src", "recursive": true, "all": true, "limit": 25})
        ),
        "List(src · recursive · all · limit 25)"
    );
    assert_eq!(
        invocation(
            "search",
            json!({
                "pattern": "TOP_SECRET_PATTERN",
                "path": "crates",
                "extension": "TOP_SECRET_EXTENSION",
                "case_sensitive": true,
                "limit": 8
            })
        ),
        "Search(\"TOP_SECRET_PATTERN\" · crates · extension TOP_SECRET_EXTENSION · case sensitive · limit 8)"
    );
    assert_eq!(
        invocation(
            "edit",
            json!({
                "path": "src/config.rs",
                "old_string": "TOP_SECRET_OLD",
                "new_string": "TOP_SECRET_NEW",
                "replace_all": true
            })
        ),
        "Update(src/config.rs · replace all)"
    );
    assert_eq!(
        invocation(
            "shell",
            json!({
                "command": "printf TOP_SECRET_COMMAND",
                "cwd": "crates/smith-cli",
                "timeout_ms": 3000
            })
        ),
        "Bash(printf TOP_SECRET_COMMAND · cwd crates/smith-cli · timeout 3000ms)"
    );
}

#[test]
fn advisor_has_a_label_and_accepts_only_its_empty_object_schema() {
    let display = project_tool_call_display("advisor", &json!({})).expect("advisor display");
    assert_eq!(display.label(), "Advisor");
    assert_eq!(display.invocation(), "Advisor()");
    assert!(display.target().is_empty());
    assert!(display.qualifiers().is_empty());
    assert!(has_tool_call_display_schema("advisor"));
    assert!(project_tool_call_display("advisor", &Value::Null).is_none());
    assert!(project_tool_call_display("advisor", &json!({"secret": "hidden"})).is_none());
}

#[test]
fn registry_search_projects_its_reviewed_query_and_bound() {
    assert_eq!(
        invocation(
            "registry.search",
            json!({"query": "browser automation", "max_results": 5})
        ),
        "Registry Search(\"browser automation\" · max 5)"
    );
    assert_eq!(
        invocation("registry.search", json!({"query": "sql"})),
        "Registry Search(\"sql\")"
    );
    assert!(has_tool_call_display_schema("registry.search"));
    assert!(project_tool_call_display("registry.search", &json!({"max_results": 3})).is_none());
}

#[test]
fn optional_root_targets_have_a_stable_default() {
    assert_eq!(
        invocation("list", json!({"recursive": true})),
        "List(. · recursive)"
    );
    assert_eq!(
        invocation("search", json!({"pattern": "hidden"})),
        "Search(\"hidden\" · .)"
    );
    assert_eq!(
        invocation("shell", json!({"command": "hidden"})),
        "Bash(hidden)"
    );
}

#[test]
fn ordinary_operation_values_enter_but_bulk_and_unknown_values_do_not() {
    let search = invocation(
        "search",
        json!({
            "pattern": "NEEDLE",
            "extension": "rs",
            "result": "TOP_SECRET_RESULT",
            "unknown": "TOP_SECRET_UNKNOWN"
        }),
    );
    assert!(search.contains("NEEDLE"));
    assert!(search.contains("extension rs"));
    assert!(!search.contains("TOP_SECRET_RESULT"));
    assert!(!search.contains("TOP_SECRET_UNKNOWN"));

    let edit = invocation(
        "edit",
        json!({
            "path": "safe.rs",
            "old_string": "TOP_SECRET_OLD",
            "new_string": "TOP_SECRET_NEW",
            "result": "TOP_SECRET_RESULT",
            "unknown": "TOP_SECRET_UNKNOWN"
        }),
    );
    assert!(!edit.contains("TOP_SECRET_OLD"));
    assert!(!edit.contains("TOP_SECRET_NEW"));
    assert!(!edit.contains("TOP_SECRET_RESULT"));
    assert!(!edit.contains("TOP_SECRET_UNKNOWN"));

    let shell = invocation(
        "shell",
        json!({
            "command": "printf ordinary",
            "result": "TOP_SECRET_RESULT",
            "unknown": "TOP_SECRET_UNKNOWN"
        }),
    );
    assert!(shell.contains("printf ordinary"));
    assert!(!shell.contains("TOP_SECRET_RESULT"));
    assert!(!shell.contains("TOP_SECRET_UNKNOWN"));
}

#[test]
fn credential_redaction_markers_remain_explicit() {
    assert_eq!(
        invocation("search", json!({"pattern": "[redacted]"})),
        "Search(\"[redacted]\" · .)"
    );
    assert_eq!(
        invocation("shell", json!({"command": "curl -H [redacted]"})),
        "Bash(curl -H [redacted])"
    );
}

#[test]
fn targets_are_one_line_control_free_and_bounded() {
    let raw = format!(
        "{}\nnext\t\u{1b}[31m\u{202e}tail",
        "a".repeat(MAX_VALUE_CHARS * 2)
    );
    let display =
        project_tool_call_display("read", &json!({"path": raw})).expect("path should project");

    assert!(display.target().chars().count() <= MAX_VALUE_CHARS);
    assert!(display.target().ends_with('…'));
    assert!(
        !display
            .target()
            .chars()
            .any(|character| character.is_control())
    );
    assert!(!display.target().contains('\u{202e}'));
    assert!(!display.invocation().contains('\n'));
}

#[test]
fn controls_inside_a_short_target_are_collapsed_to_spaces() {
    assert_eq!(
        invocation(
            "read",
            json!({"path": "src/\nsecret\t\u{1b}[31m.rs\u{202e}"})
        ),
        "Read(src/ secret [31m.rs)"
    );
}

#[test]
fn malformed_or_unknown_calls_keep_the_caller_on_its_fallback_path() {
    assert!(has_tool_call_display_schema("read"));
    assert!(!has_tool_call_display_schema("third_party"));
    assert!(project_tool_call_display("third_party", &json!({"path": "safe"})).is_none());
    assert!(project_tool_call_display("read", &json!({"path": 42})).is_none());
    assert!(project_tool_call_display("list", &json!({"recursive": "yes"})).is_none());
    assert!(
        project_tool_call_display("shell", &json!({"command": "ok", "timeout_ms": 0})).is_none()
    );
    assert!(project_tool_call_display("search", &json!({"path": "."})).is_none());
}

#[test]
fn task_output_and_task_stop_project_only_the_task_id_and_reviewed_numbers() {
    assert_eq!(
        invocation(
            "task_output",
            json!({"task_id": "task:1", "offset": 128, "limit": 4096})
        ),
        "Task Output(task:1 · offset 128 · limit 4096)"
    );
    // Offset 0 is the ordinary default, not a signal to fall back — unlike
    // `read`'s 1-based offset, it must still project.
    assert_eq!(
        invocation("task_output", json!({"task_id": "task:1", "offset": 0})),
        "Task Output(task:1)"
    );
    assert_eq!(
        invocation("task_output", json!({"task_id": "task:1"})),
        "Task Output(task:1)"
    );
    assert_eq!(
        invocation(
            "task_stop",
            json!({"task_id": "task:2", "result": "TOP_SECRET_RESULT"})
        ),
        "Task Stop(task:2)"
    );
    assert!(has_tool_call_display_schema("task_output"));
    assert!(has_tool_call_display_schema("task_stop"));
    assert!(project_tool_call_display("task_output", &json!({"offset": 1})).is_none());
    assert!(project_tool_call_display("task_stop", &json!({})).is_none());
}

#[test]
fn agent_spawn_renders_every_reviewed_field() {
    assert_eq!(
        invocation(
            "agent",
            json!({
                "action": "spawn",
                "task": "explore the autoloads and data layer",
                "tools": "all",
                "workspace": "shared",
                "profile": "explore"
            })
        ),
        "Agent(spawn · \"explore the autoloads and data layer\" · tools all · workspace shared · profile explore)"
    );
    assert!(has_tool_call_display_schema("agent"));
}

#[test]
fn agent_spawn_defaults_to_read_only_scope_and_workspace() {
    assert_eq!(
        invocation("agent", json!({"action": "spawn", "task": "look around"})),
        "Agent(spawn · \"look around\" · tools read only · workspace read only)"
    );
}

fn delegation_approval(operation: &str, material: Value) -> PreparedToolCall {
    PreparedToolCall::new(
        agent_runtime_core::ids::ToolCallId::new("child-approval"),
        operation,
        material,
        agent_runtime_core::security::PermissionSet::single(
            agent_runtime_registry::Permission::other("agent.delegate"),
        ),
        SecurityResource::other("child-agent", "session-parent"),
        agent_runtime_core::tool::ToolEffects::new(Vec::new()),
        agent_runtime_core::tool::ToolCallDisplay::new("Authorize child-agent operation"),
    )
}

fn spawn_approval_material() -> Value {
    json!({
        "task": "review the change",
        "tools": {"scope": "all"},
        "workspace": {"policy": "shared_project"},
        "max_turns": u32::MAX,
        "max_tokens": null,
        "deadline_ms": null,
    })
}

#[test]
fn child_agent_approval_projection_reuses_spawn_words_and_preserves_identity() {
    for (scope, workspace) in [("all", "shared"), ("read_only", "read_only")] {
        let row = project_tool_call_display("agent", &json!({
            "action": "spawn", "task": "review the change", "tools": scope, "workspace": workspace,
        })).expect("spawn row");
        let mut material = spawn_approval_material();
        material["tools"] = json!({"scope": scope});
        material["workspace"] =
            json!({"policy": if workspace == "shared" {"shared_project"} else {"read_only_view"}});
        let prepared = delegation_approval("delegation.spawn", material);
        let before = prepared.clone();
        let projection = project_delegation_approval_display(&prepared).expect("reviewed spawn");
        assert_eq!(projection.title, "Start a child agent");
        assert_eq!(&projection.lines[1..3], &row.qualifiers()[1..3]);
        assert_eq!(projection.lines[0], "review the change");
        assert_eq!(projection.lines.len(), 3);
        assert_eq!(
            (
                projection.turn_limit,
                projection.token_limit,
                projection.time_limit_ms
            ),
            (None, None, None)
        );
        assert_eq!(
            prepared, before,
            "projection must not change the immutable approval"
        );
    }
}

#[test]
fn child_agent_approval_projection_covers_named_tools_directory_and_isolated_workspace() {
    let mut material = spawn_approval_material();
    material["tools"] = json!({"scope": "named", "names": ["read", "search"]});
    for (workspace, expected) in [
        (
            json!({"policy": "explicit_directory", "path": "/repo/crates"}),
            "workspace /repo/crates",
        ),
        (
            json!({"policy": "isolated_worktree"}),
            "workspace isolated worktree",
        ),
    ] {
        material["workspace"] = workspace;
        let projection = project_delegation_approval_display(&delegation_approval(
            "delegation.spawn",
            material.clone(),
        ))
        .expect("reviewed workspace");
        assert_eq!(projection.lines[1], "tools read, search");
        assert_eq!(projection.lines[2], expected);
    }
}

#[test]
fn child_agent_approval_projection_bounds_and_normalizes_task() {
    let mut material = spawn_approval_material();
    material["task"] = json!(format!("Review\n\u{1b}\u{202e}{}", "x".repeat(300)));
    let projection =
        project_delegation_approval_display(&delegation_approval("delegation.spawn", material))
            .expect("bounded approval");
    assert!(projection.lines[0].chars().count() <= 201);
    assert!(projection.lines[0].ends_with('…'));
    for line in &projection.lines {
        assert!(
            !line.contains(['\n', '\r', '\u{1b}', '\u{202e}']),
            "{line:?}"
        );
    }
}

#[test]
fn child_agent_approval_projection_falls_back_for_unknown_or_malformed_material() {
    assert!(
        project_delegation_approval_display(&delegation_approval(
            "delegation.future",
            json!({"child_id": "child-1"})
        ))
        .is_none()
    );
    for (key, value) in [
        ("tools", json!({"scope": "future"})),
        ("tools", json!({"scope": "all", "future_authority": true})),
        ("workspace", json!({"policy": "future"})),
        (
            "workspace",
            json!({"policy": "shared_project", "future_authority": true}),
        ),
        ("max_turns", json!(0)),
        ("max_turns", json!(u64::MAX)),
        ("max_tokens", json!("unlimited")),
        ("deadline_ms", json!(-1)),
        ("task", json!(null)),
        ("future_authority", json!(true)),
    ] {
        let mut material = spawn_approval_material();
        material[key] = value;
        assert!(
            project_delegation_approval_display(&delegation_approval("delegation.spawn", material))
                .is_none(),
            "must retain the fallback for {key}"
        );
    }
}

#[test]
fn agent_spawn_result_preview_uses_words_for_the_reviewed_success_shape() {
    let display =
        project_tool_call_display("agent", &json!({"action": "spawn", "task": "look around"}))
            .expect("spawn");
    let output =
        r#"{"note":"the result will be delivered when the child completes","spawned":"child-1"}"#;
    assert_eq!(
        display.result_summary(output).as_deref(),
        Some("child-1 started · its result arrives when it completes")
    );
    let list = project_tool_call_display("agent", &json!({"action": "list"})).expect("list");
    let result =
        project_tool_call_display("agent", &json!({"action": "result", "child_id": "child-1"}))
            .expect("result");
    assert!(list.result_summary(output).is_none());
    assert!(result.result_summary(output).is_none());
    for output in [
        "spawn failed",
        r#"{"error":"spawn failed"}"#,
        r#"{"spawned":"child-1"}"#,
        r#"{"spawned":1,"note":"the result will be delivered when the child completes"}"#,
        r#"{"spawned":"child-1","note":"unexpected note"}"#,
        r#"{"spawned":"child-1","note":"the result will be delivered when the child completes","error":"failed"}"#,
    ] {
        assert!(display.result_summary(output).is_none(), "{output}");
    }
}

#[test]
fn agent_spawn_result_preview_bounds_and_normalizes_the_child_name() {
    let display =
        project_tool_call_display("agent", &json!({"action": "spawn", "task": "look around"}))
            .expect("spawn");
    let output = json!({"spawned": format!("child\n{}\u{1b}", "x".repeat(200)), "note": "the result will be delivered when the child completes"}).to_string();
    let summary = display.result_summary(&output).expect("summary");
    assert!(
        !summary.contains('\n') && !summary.contains('\u{1b}'),
        "{summary:?}"
    );
    assert!(summary.contains('…'), "{summary}");
}

#[tokio::test]
async fn approval_paths_use_the_prepared_workspace_mount_without_changing_identity() {
    use agent_runtime_core::tool::Tool;
    let (dir, ctx) = crate::testing::project();
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("src/lib.rs"), "before\n").expect("file");
    let prepared = crate::EditTool
        .prepare(
            json!({"path": "src/lib.rs", "old_string": "before", "new_string": "after"}),
            &crate::testing::preparation_context(&ctx),
        )
        .await
        .expect("prepared");
    let fingerprint = prepared.fingerprint().clone();
    let canonical = prepared.arguments()["path"].as_str().expect("path");
    assert!(std::path::Path::new(canonical).is_absolute());
    assert_eq!(
        approval_path_display(&prepared).as_deref(),
        Some("src/lib.rs")
    );
    assert_eq!(prepared.fingerprint(), &fingerprint);

    let outside = PreparedToolCall::new(
        prepared.call_id().clone(),
        "external_edit",
        prepared.arguments().clone(),
        prepared.required_permissions().clone(),
        SecurityResource::filesystem("/outside", vec!["file.txt".to_owned()]),
        prepared.effects().clone(),
        prepared.display().clone(),
    );
    assert!(approval_path_display(&outside).is_none());
}

#[test]
fn agent_spawn_directory_workspace_shows_a_bounded_path() {
    assert_eq!(
        invocation(
            "agent",
            json!({
                "action": "spawn",
                "task": "build the feature",
                "workspace": {"directory": {"path": "/repo/crates/smith-tools"}}
            })
        ),
        "Agent(spawn · \"build the feature\" · tools read only · workspace /repo/crates/smith-tools)"
    );
    assert!(
        project_tool_call_display(
            "agent",
            &json!({
                "action": "spawn",
                "task": "build the feature",
                "workspace": {"directory": {}}
            })
        )
        .is_none()
    );
}

#[test]
fn agent_spawn_excerpt_normalizes_control_terminal_and_bidi_characters() {
    let display = project_tool_call_display(
        "agent",
        &json!({
            "action": "spawn",
            "task": "line one\nline two\rcarriage \u{1b}[31mred\u{202e}reversed"
        }),
    )
    .expect("spawn should project");
    let rendered = display.invocation();
    assert!(!rendered.contains('\n'));
    assert!(!rendered.contains('\r'));
    assert!(!rendered.contains('\u{1b}'));
    assert!(!rendered.contains('\u{202e}'));
    assert!(rendered.contains("line one"));
    assert!(rendered.contains("reversed"));
}

#[test]
fn agent_spawn_excerpt_is_bounded_to_one_line() {
    let long_task = "word ".repeat(MAX_VALUE_CHARS);
    let display =
        project_tool_call_display("agent", &json!({"action": "spawn", "task": long_task}))
            .expect("spawn should project");
    let rendered = display.invocation();
    assert!(!rendered.contains('\n'));
    assert!(rendered.contains('…'));
    // The excerpt itself (inside the quotes) must not exceed the shared
    // bound; the surrounding quotes and label are not part of that bound.
    let excerpt = &display.qualifiers()[0];
    assert!(excerpt.chars().count() <= MAX_VALUE_CHARS + 2);
}

#[test]
fn agent_addressed_actions_name_their_child() {
    assert_eq!(
        invocation(
            "agent",
            json!({"action": "follow_up", "child_id": "child-1", "task": "keep going"})
        ),
        "Agent(follow_up · child-1 · \"keep going\")"
    );
    assert_eq!(
        invocation("agent", json!({"action": "stop", "child_id": "child-1"})),
        "Agent(stop · child-1)"
    );
    assert_eq!(
        invocation("agent", json!({"action": "wait", "child_id": "child-2"})),
        "Agent(wait · child-2)"
    );
    assert_eq!(
        invocation("agent", json!({"action": "result", "child_id": "child-2"})),
        "Agent(result · child-2)"
    );
    assert_eq!(
        invocation("agent", json!({"action": "resume", "child_id": "child-3"})),
        "Agent(resume · child-3)"
    );
}

#[test]
fn agent_list_names_no_child_and_no_task() {
    assert_eq!(
        invocation("agent", json!({"action": "list"})),
        "Agent(list)"
    );
}

#[test]
fn agent_rejects_ill_typed_arguments_and_unknown_actions() {
    assert!(project_tool_call_display("agent", &json!({})).is_none());
    assert!(project_tool_call_display("agent", &json!({"action": 1})).is_none());
    assert!(project_tool_call_display("agent", &json!({"action": "teleport"})).is_none());
    assert!(project_tool_call_display("agent", &json!({"action": "spawn"})).is_none());
    assert!(
        project_tool_call_display(
            "agent",
            &json!({"action": "spawn", "task": "ok", "tools": "sudo"})
        )
        .is_none()
    );
    assert!(
        project_tool_call_display(
            "agent",
            &json!({"action": "spawn", "task": "ok", "workspace": "everywhere"})
        )
        .is_none()
    );
    assert!(
        project_tool_call_display("agent", &json!({"action": "stop", "child_id": 5})).is_none()
    );
    assert!(
        project_tool_call_display(
            "agent",
            &json!({"action": "follow_up", "child_id": "child-1"})
        )
        .is_none()
    );
    assert!(project_tool_call_display("agent", &json!({"action": "wait"})).is_none());
}

#[test]
fn with_qualifier_appends_and_normalizes() {
    let display = display("Agent", "spawn".to_owned(), Vec::new())
        .with_qualifier("child-1")
        .with_qualifier("turns 12\nnext line")
        .with_qualifier("");

    assert_eq!(
        display.qualifiers(),
        &["child-1".to_owned(), "turns 12 next line".to_owned()]
    );
    assert_eq!(
        display.invocation(),
        "Agent(spawn · child-1 · turns 12 next line)"
    );
}

#[test]
fn with_qualifiers_appends_several_in_order() {
    let display = display("Agent", "spawn".to_owned(), Vec::new())
        .with_qualifiers(["child-1", "shared", "turns 12"]);

    assert_eq!(
        display.qualifiers(),
        &[
            "child-1".to_owned(),
            "shared".to_owned(),
            "turns 12".to_owned()
        ]
    );
}
