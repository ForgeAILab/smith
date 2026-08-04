//! Smith's one command registry.
//!
//! Slash completion, `Ctrl+P`, `/help`, and execution all consume this table.
//! A command cannot be advertised through one surface while being absent from
//! another because there is no second list to drift.

/// What the host must do for a command after the TUI parses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    /// Start a fresh session.
    NewSession,
    /// Resume a saved session.
    Resume(Option<String>),
    /// Select a configured profile.
    Profile(Option<String>),
    /// Select a provider.
    Provider(Option<String>),
    /// Connect or reconnect a provider/backend.
    Connect(Option<String>),
    /// Disconnect a provider/backend.
    Disconnect(Option<String>),
    /// Select a model.
    Model(Option<String>),
    /// Select an explicit thinking state or the provider default.
    Think(Option<String>),
    /// Select an advertised reasoning effort or the provider default.
    Effort(Option<String>),
    /// Inspect the provider's credential pool, or switch to one account.
    Account(Option<String>),
    /// Render resolved local status.
    Status,
    /// Inspect or mutate the persistent session goal.
    Goal(GoalAction),
    /// Visualize the latest model-facing context plan.
    Context,
    /// Toggle bounded active-work detail.
    Details,
    /// Render the bounded local root/child/recovery timeline.
    Timeline,
    /// List children or inspect one child.
    Agent(Option<String>),
    /// Explicitly continue one interrupted child's exact checkpoint.
    AgentResume(String),
    /// Inspect a Git change scope.
    Diff(Option<String>),
    /// Review a change scope.
    Review(Option<String>),
    /// Undo the newest attributable turn.
    Undo,
    /// Reapply the newest exact successfully undone turn.
    Redo,
    /// Revert a selected file or hunk.
    Revert(Option<String>),
    /// Exit under the active-work policy.
    Quit,
    /// Render command help locally.
    Help,
}

/// Typed local persistent-goal control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalAction {
    /// Render the current goal, if any.
    Show,
    /// Create a goal with the supplied bounded objective.
    Create(String),
    /// Replace the unfinished goal objective.
    Edit(String),
    /// Set a positive token budget or remove it with `None`.
    Budget(Option<u64>),
    /// Pause active automatic work.
    Pause,
    /// Resume eligible stopped automatic work.
    Resume,
    /// Clear the current goal without marking it complete.
    Clear,
}

/// One discoverable command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    /// Name without the leading slash.
    pub name: &'static str,
    /// Optional argument syntax.
    pub argument_hint: &'static str,
    /// One-line description.
    pub description: &'static str,
    /// Commands that require a safe idle boundary.
    pub requires_idle: bool,
    /// Less-frequent commands shown in the advanced help group.
    pub advanced: bool,
}

/// The complete implemented command set.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        argument_hint: "",
        description: "list available commands",
        requires_idle: false,
        advanced: false,
    },
    CommandSpec {
        name: "status",
        argument_hint: "",
        description: "show runtime and workspace status",
        requires_idle: false,
        advanced: false,
    },
    CommandSpec {
        name: "goal",
        argument_hint: "[OBJECTIVE|edit …|budget N|pause|resume|clear]",
        description: "inspect or control a persistent multi-turn goal",
        requires_idle: false,
        advanced: false,
    },
    CommandSpec {
        name: "context",
        argument_hint: "",
        description: "show current context usage",
        requires_idle: false,
        advanced: false,
    },
    CommandSpec {
        name: "details",
        argument_hint: "",
        description: "toggle bounded live tool detail",
        requires_idle: false,
        advanced: false,
    },
    CommandSpec {
        name: "timeline",
        argument_hint: "",
        description: "show local turn, child, and recovery history",
        requires_idle: false,
        advanced: false,
    },
    CommandSpec {
        name: "new",
        argument_hint: "",
        description: "start a fresh session",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "resume",
        argument_hint: "[ID]",
        description: "resume a saved session",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "connect",
        argument_hint: "[PROVIDER]",
        description: "connect or reconnect a provider",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "disconnect",
        argument_hint: "[PROVIDER]",
        description: "disconnect a provider",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "model",
        argument_hint: "[PROVIDER/MODEL]",
        description: "switch model",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "think",
        argument_hint: "[on|off|default]",
        description: "set thinking for the next turn",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "effort",
        argument_hint: "[LEVEL|default]",
        description: "set reasoning effort for the next turn",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "account",
        argument_hint: "[N]",
        description: "show provider accounts and their usage",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "agent",
        argument_hint: "[ID|resume ID]",
        description: "list, inspect, or resume an existing agent",
        requires_idle: false,
        advanced: false,
    },
    CommandSpec {
        name: "diff",
        argument_hint: "[SCOPE]",
        description: "inspect workspace changes",
        requires_idle: false,
        advanced: false,
    },
    CommandSpec {
        name: "review",
        argument_hint: "[SCOPE]",
        description: "run a read-only change review",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "undo",
        argument_hint: "",
        description: "undo the last attributable turn",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "redo",
        argument_hint: "",
        description: "reapply the newest exact undone turn",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "revert",
        argument_hint: "[FILE]",
        description: "selectively revert a file or hunk",
        requires_idle: true,
        advanced: false,
    },
    CommandSpec {
        name: "profile",
        argument_hint: "[NAME]",
        description: "switch configured profile",
        requires_idle: true,
        advanced: true,
    },
    CommandSpec {
        name: "provider",
        argument_hint: "[NAME]",
        description: "switch provider",
        requires_idle: true,
        advanced: true,
    },
    CommandSpec {
        name: "quit",
        argument_hint: "",
        description: "exit Smith",
        requires_idle: false,
        advanced: true,
    },
];

/// Registry entries matching a composer draft or palette query.
pub fn matches(input: &str) -> Vec<&'static CommandSpec> {
    let query = input.trim().trim_start_matches('/');
    let name = query.split_whitespace().next().unwrap_or_default();
    COMMANDS
        .iter()
        .filter(|command| command.name.starts_with(name))
        .collect()
}

/// Completes the selected command without executing it.
pub fn completion(command: &CommandSpec) -> String {
    if command.argument_hint.is_empty() {
        format!("/{}", command.name)
    } else {
        format!("/{} ", command.name)
    }
}

/// Parses one command using the same registry shown by discovery and help.
pub fn parse(input: &str) -> Result<CommandAction, String> {
    let trimmed = input.trim().trim_start_matches('/');
    let mut words = trimmed.split_whitespace();
    let Some(name) = words.next() else {
        return Err("select or enter a command".to_owned());
    };
    let Some(spec) = COMMANDS.iter().find(|command| command.name == name) else {
        return Err(format!("unknown command `/{name}` — type /help"));
    };

    if name == "goal" {
        return parse_goal(trimmed.strip_prefix(name).unwrap_or_default().trim());
    }

    let argument = words.next().map(str::to_owned);
    let second = words.next().map(str::to_owned);
    if words.next().is_some() {
        return Err(format!("`/{name}` accepts at most one value"));
    }
    if spec.argument_hint.is_empty() && argument.is_some() {
        return Err(format!("`/{name}` takes no value"));
    }

    if name == "agent" && argument.as_deref() == Some("resume") {
        let child = second.ok_or_else(|| "`/agent resume` requires a child ID".to_owned())?;
        return Ok(CommandAction::AgentResume(child));
    }
    if second.is_some() {
        return Err(format!("`/{name}` accepts at most one value"));
    }

    Ok(match name {
        "help" => CommandAction::Help,
        "status" => CommandAction::Status,
        "context" => CommandAction::Context,
        "details" => CommandAction::Details,
        "timeline" => CommandAction::Timeline,
        "new" => CommandAction::NewSession,
        "resume" => CommandAction::Resume(argument),
        "profile" => CommandAction::Profile(argument),
        "provider" => CommandAction::Provider(argument),
        "connect" => CommandAction::Connect(argument),
        "disconnect" => CommandAction::Disconnect(argument),
        "model" => CommandAction::Model(argument),
        "think" => CommandAction::Think(argument),
        "effort" => CommandAction::Effort(argument),
        "account" | "accounts" => CommandAction::Account(argument),
        "agent" => CommandAction::Agent(argument),
        "diff" => CommandAction::Diff(argument),
        "review" => CommandAction::Review(argument),
        "undo" => CommandAction::Undo,
        "redo" => CommandAction::Redo,
        "revert" => CommandAction::Revert(argument),
        "quit" => CommandAction::Quit,
        _ => unreachable!("every registered command is parsed above"),
    })
}

fn parse_goal(argument: &str) -> Result<CommandAction, String> {
    let action = if argument.is_empty() {
        GoalAction::Show
    } else if let Some(objective) = argument.strip_prefix("edit ") {
        let objective = objective.trim();
        if objective.is_empty() {
            return Err("`/goal edit` requires an objective".to_owned());
        }
        GoalAction::Edit(objective.to_owned())
    } else if argument == "edit" {
        return Err("`/goal edit` requires an objective".to_owned());
    } else if let Some(value) = argument.strip_prefix("budget ") {
        let value = value.trim();
        if value == "none" {
            GoalAction::Budget(None)
        } else {
            let budget = value
                .parse::<u64>()
                .map_err(|_| "`/goal budget` requires a positive integer or `none`".to_owned())?;
            if budget == 0 {
                return Err("`/goal budget` requires a positive integer or `none`".to_owned());
            }
            GoalAction::Budget(Some(budget))
        }
    } else if argument == "budget" {
        return Err("`/goal budget` requires a positive integer or `none`".to_owned());
    } else {
        match argument {
            "pause" => GoalAction::Pause,
            "resume" => GoalAction::Resume,
            "clear" => GoalAction::Clear,
            objective => GoalAction::Create(objective.to_owned()),
        }
    };
    Ok(CommandAction::Goal(action))
}

/// Finished `/help` output, derived from the registry.
pub fn help() -> String {
    let mut output = String::from("Primary\n");
    for command in COMMANDS.iter().filter(|command| !command.advanced) {
        push_help_line(&mut output, command);
    }
    output.push_str("\nAdvanced\n");
    for command in COMMANDS.iter().filter(|command| command.advanced) {
        push_help_line(&mut output, command);
    }
    output.push_str(
        "\nComposer\n\
         ? or /help shows this local guide without contacting the model.\n\
         Tab cycles the configured profile order only while empty and idle.\n\
         While work is serving, Enter steers an ordinary prompt and Tab queues it.\n\
         Alt+Up restores the newest explicit queued turn for editing.\n\
         Esc interrupts; uncommitted steers are resent only after cancellation discards them.\n\
         Ctrl+B moves a running foreground shell command to the background without killing it.\n\
         @ completes exact files and read-only agents; @@ sends a literal @.\n\
         ! runs a prepared local shell action; !! sends a literal !.\n\
         PageUp/PageDown/Home/End or the mouse wheel scrolls the transcript.\n\
         Up/Down browse accepted and Ctrl+C-stashed input without losing your draft.\n\
         Ctrl+R searches composer history; Enter restores a match and Esc cancels.\n\
         Ctrl+C twice within 1s exits; the first press stashes and clears the draft.\n\
         Start a message with // to send a literal leading slash.",
    );
    output
}

fn push_help_line(output: &mut String, command: &CommandSpec) {
    output.push('/');
    output.push_str(command.name);
    if !command.argument_hint.is_empty() {
        output.push(' ');
        output.push_str(command.argument_hint);
    }
    output.push_str(" — ");
    output.push_str(command.description);
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_and_completion_share_the_complete_registry() {
        let help = help();
        for command in COMMANDS {
            assert!(help.contains(&format!("/{}", command.name)), "{help}");
        }
        assert!(help.contains("Ctrl+R searches composer history"), "{help}");
        assert!(help.contains("Up/Down browse accepted"), "{help}");
        assert_eq!(
            matches("/rev")
                .into_iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["review", "revert"]
        );
    }

    #[test]
    fn parser_returns_typed_actions_and_actionable_errors() {
        assert_eq!(
            parse("/model zai").expect("model"),
            CommandAction::Model(Some("zai".into()))
        );
        assert_eq!(
            parse("/diff staged").expect("diff"),
            CommandAction::Diff(Some("staged".into()))
        );
        assert_eq!(parse("/context").expect("context"), CommandAction::Context);
        assert_eq!(parse("/model").expect("picker"), CommandAction::Model(None));
        assert_eq!(
            parse("/connect openrouter").expect("connection"),
            CommandAction::Connect(Some("openrouter".into()))
        );
        assert_eq!(
            parse("/disconnect").expect("disconnect picker"),
            CommandAction::Disconnect(None)
        );
        assert_eq!(
            parse("/think off").expect("thinking state"),
            CommandAction::Think(Some("off".into()))
        );
        assert_eq!(
            parse("/effort high").expect("effort"),
            CommandAction::Effort(Some("high".into()))
        );
        assert_eq!(parse("/think").expect("picker"), CommandAction::Think(None));
        assert_eq!(
            parse("/effort").expect("picker"),
            CommandAction::Effort(None)
        );
        assert_eq!(
            parse("/agent resume child-7").expect("child resume"),
            CommandAction::AgentResume("child-7".into())
        );
        assert!(
            parse("/agent resume")
                .unwrap_err()
                .contains("requires a child ID")
        );
        assert!(parse("/missing").unwrap_err().contains("/help"));
    }

    #[test]
    fn goal_parser_preserves_objectives_and_validates_controls() {
        assert_eq!(
            parse("/goal").unwrap(),
            CommandAction::Goal(GoalAction::Show)
        );
        assert_eq!(
            parse("/goal ship the persistent goal system").unwrap(),
            CommandAction::Goal(GoalAction::Create("ship the persistent goal system".into()))
        );
        assert_eq!(
            parse("/goal edit ship it safely").unwrap(),
            CommandAction::Goal(GoalAction::Edit("ship it safely".into()))
        );
        assert_eq!(
            parse("/goal budget 12000").unwrap(),
            CommandAction::Goal(GoalAction::Budget(Some(12_000)))
        );
        assert_eq!(
            parse("/goal budget none").unwrap(),
            CommandAction::Goal(GoalAction::Budget(None))
        );
        assert_eq!(
            parse("/goal pause").unwrap(),
            CommandAction::Goal(GoalAction::Pause)
        );
        assert!(parse("/goal edit").unwrap_err().contains("objective"));
        assert!(parse("/goal budget 0").unwrap_err().contains("positive"));
    }
}
