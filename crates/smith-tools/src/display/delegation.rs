use super::*;
/// Reviewed decision evidence for a coordinator-prepared child operation.
/// Numeric limits stay typed so clients can use their shared count/duration
/// formatters without reading raw material fields themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationApprovalDisplay {
    /// Action title, shown once at the top of the approval.
    pub title: &'static str,
    /// Bounded task, tool/workspace posture, or addressed child.
    pub lines: Vec<String>,
    /// A finite turn ceiling; the runtime's unlimited sentinel is omitted.
    pub turn_limit: Option<u32>,
    /// An explicitly set total token budget.
    pub token_limit: Option<u64>,
    /// An explicitly set lifetime in milliseconds, rather than a prompt deadline.
    pub time_limit_ms: Option<u64>,
    /// The consequence of granting the reviewed delegation permission.
    pub warning: &'static str,
}

/// Projects the material the delegation coordinator prepared, not the
/// model-facing `agent` arguments. Unknown operations/material shapes
/// retain the caller's ordinary approval fallback.
pub fn project_delegation_approval_display(
    prepared: &PreparedToolCall,
) -> Option<DelegationApprovalDisplay> {
    if !matches!(prepared.resource(), SecurityResource::Other { kind, .. } if kind == "child-agent")
    {
        return None;
    }
    let fields = prepared.arguments().as_object()?;
    let mut display = DelegationApprovalDisplay {
        title: "",
        lines: Vec::new(),
        turn_limit: None,
        token_limit: None,
        time_limit_ms: None,
        warning: "the child acts on its own with these tools",
    };
    match prepared.tool() {
        "delegation.spawn" => {
            if fields.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "task" | "tools" | "workspace" | "max_turns" | "max_tokens" | "deadline_ms"
                )
            }) {
                return None;
            }
            let tools: ToolViewScope = serde_json::from_value(fields.get("tools")?.clone()).ok()?;
            let workspace: WorkspacePolicy =
                serde_json::from_value(fields.get("workspace")?.clone()).ok()?;
            // Serde accepts extra enum fields; retain the material fallback
            // rather than silently hiding a future authority-bearing field.
            if serde_json::to_value(&tools).ok()? != *fields.get("tools")?
                || serde_json::to_value(&workspace).ok()? != *fields.get("workspace")?
            {
                return None;
            }
            display.title = "Start a child agent";
            display.lines = vec![
                approval_task(fields)?,
                format!("tools {}", agent_tool_scope_display(&tools)?),
                format!("workspace {}", agent_workspace_display(&workspace)?),
            ];
            let turns = u32::try_from(fields.get("max_turns")?.as_u64()?).ok()?;
            if turns == 0 {
                return None;
            }
            display.turn_limit = (turns != u32::MAX).then_some(turns);
            display.token_limit = nullable_positive_integer(fields, "max_tokens")?;
            display.time_limit_ms = nullable_positive_integer(fields, "deadline_ms")?;
        }
        "delegation.follow_up" | "delegation.resume" | "delegation.stop" => {
            let follow_up = prepared.tool() == "delegation.follow_up";
            if fields
                .keys()
                .any(|key| key != "child_id" && !(follow_up && key == "task"))
            {
                return None;
            }
            let child = required_target(fields, "child_id")?;
            display.title = match prepared.tool() {
                "delegation.follow_up" => "Send a child agent a follow-up",
                "delegation.resume" => "Resume a child agent",
                _ => "Stop a child agent",
            };
            if follow_up {
                display.lines.push(approval_task(fields)?);
            }
            display.lines.push(child);
            if prepared.tool() == "delegation.resume" {
                display
                    .lines
                    .push("continue the exact saved checkpoint".to_owned());
            }
            if prepared.tool() == "delegation.stop" {
                display.warning = "stops the child's current work";
            }
        }
        _ => return None,
    }
    Some(display)
}

fn approval_task(fields: &Map<String, Value>) -> Option<String> {
    // The coordinator clips approval task text to 200 characters plus an
    // ellipsis. Keep that bound while removing terminal and bidi controls.
    normalize_value_with_limit(require_string_field(fields, "task")?, 201)
}

fn nullable_positive_integer(fields: &Map<String, Value>, key: &str) -> Option<Option<u64>> {
    match fields.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(value) => value.as_u64().filter(|value| *value > 0).map(Some),
    }
}
