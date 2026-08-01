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
    /// Select a model.
    Model(Option<String>),
    /// Render resolved local status.
    Status,
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
        name: "model",
        argument_hint: "[PROVIDER/MODEL]",
        description: "switch model",
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
    let argument = words.next().map(str::to_owned);
    let second = words.next().map(str::to_owned);
    if words.next().is_some() {
        return Err(format!("`/{name}` accepts at most one value"));
    }
    let Some(spec) = COMMANDS.iter().find(|command| command.name == name) else {
        return Err(format!("unknown command `/{name}` — type /help"));
    };

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
        "model" => CommandAction::Model(argument),
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
         Tab cycles build/plan/review only while empty and idle.\n\
         @ completes exact files and read-only agents; @@ sends a literal @.\n\
         ! runs a prepared local shell action; !! sends a literal !.\n\
         Up recalls a draft cleared by Ctrl+C; Down returns toward empty.\n\
         Ctrl+C twice within 1s exits; the first press stashes the draft.\n\
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
}
