//! Smith's client-neutral command registry and argument grammar.
//!
//! Slash completion, `Ctrl+P`, `/help`, and parsing consume the same table.
//! Each parsed value carries its entry and encodes the executor in its type.

use crate::help_report::{HelpCommand, HelpKey, HelpReport};

/// A command routed to the executor that can handle it.
///
/// The local host accepts only [`HostCommand`], so a UI or session command
/// cannot be passed to that executor.
///
/// ```compile_fail
/// use smith_client::commands::{HostCommand, UiCommand};
/// fn dispatch_host(_: HostCommand) {}
/// dispatch_host(UiCommand::Help);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// A local application transition.
    Ui(UiCommand),
    /// A command executed against the hosted runtime.
    Host(HostCommand),
    /// A resolved session or live-account control.
    Session(SessionControl),
    /// A transition requiring application confirmation.
    Confirm(ConfirmCommand),
}

/// A resolved control after resource selection and validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionControl {
    /// Rebuild or replace the session with a resolved selection.
    Reconfigure(SelectionCommand),
    /// Open the reviewed connection ceremony for a provider or backend.
    Connect(String),
    /// Remove one provider or backend authentication source.
    Disconnect(String),
    /// Switch the active provider credential to a pool position.
    Account(usize),
}

/// A resolved selection that rebuilds or replaces the hosted session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionCommand {
    /// Create a fresh session with the current selection.
    NewSession,
    /// Resume an existing session identity.
    Resume(String),
    /// Select a configured profile and clear narrower provider/model flags.
    Profile(String),
    /// Select a model, with the provider that serves it when there is one.
    Model {
        /// Serving provider, absent for an installed CLI agent: nothing is
        /// called through a provider to run its turn.
        provider: Option<String>,
        /// Model ID.
        model: String,
    },
    /// Select a deprecated legacy root mode at a safe session boundary.
    Agent(String),
    /// Select an explicit thinking state; `None` restores provider behavior.
    Think(Option<bool>),
    /// Select an advertised effort; `None` restores provider behavior.
    Effort(Option<String>),
    /// Choose the session's advisor in place of the configured one.
    Advisor(AdvisorChoice),
    /// Deny one capability pattern for the rest of this session.
    CapabilityDeny(String),
    /// Lift a denial this session added.
    CapabilityAllow(String),
    /// Select a model context window; `None` restores the model default.
    ContextWindow(Option<String>),
}

/// A session's advisor choice, already checked against the offered targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvisorChoice {
    /// Use what configuration resolves.
    Default,
    /// Consult no advisor.
    Off,
    /// Consult this profile or `provider/model`.
    Target(String),
}

/// Typed capability listing and session-limit control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitiesAction {
    /// List active, available, and denied capabilities.
    List,
    /// Deny a `<domain>:<name>` pattern for this session.
    Deny(String),
    /// Lift a denial this session added.
    Allow(String),
}

/// Typed local MCP server control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpAction {
    /// List every declared server with its state.
    List,
    /// Show the named server's resolved invocation and content identity, and
    /// ask whether it may be run.
    Trust(String),
}

/// Typed local skill-catalog control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsAction {
    /// List every indexed skill and every file discovery refused.
    List,
    /// Show the named project skill's path and content identity, and ask
    /// whether its instructions may be activated.
    Trust(String),
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

/// An existing-child selection parsed by the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentAction {
    /// List retained children.
    List,
    /// Return to the root timeline.
    Parent,
    /// Inspect the next retained child.
    Next,
    /// Inspect the previous retained child.
    Previous,
    /// Inspect one exact child identity.
    Inspect(String),
}

/// A diff source parsed before host dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffScope {
    /// Inspect the newest attributable Smith turn.
    LastTurn,
    /// Inspect a Git scope or path; `None` selects all uncommitted changes.
    Git(Option<String>),
}

/// The argument shape and constructor for one routed command.
#[derive(Debug, Clone, Copy)]
pub enum ArgumentGrammar {
    /// A command with no value.
    NoValue(fn() -> Command),
    /// At most one whitespace-delimited value.
    OptionalValue(fn(Option<String>) -> Result<Command, String>),
    /// At most two words, interpreted as a subcommand and its value.
    Subcommand(fn(Option<String>, Option<String>) -> Result<Command, String>),
    /// The complete remaining text, including internal whitespace.
    WholeValue(fn(&str) -> Result<Command, String>),
}

/// One discoverable command, including its grammar and routed constructor.
#[derive(Debug, Clone, Copy)]
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
    /// Argument grammar that constructs the typed route.
    pub grammar: ArgumentGrammar,
    /// One valid invocation exercising this entry's grammar.
    pub usage_example: &'static str,
    /// Whether completion is a complete bare command despite optional values.
    pub complete_without_value: bool,
}

/// A routed command with the registry entry that supplied its policy.
#[derive(Debug, Clone)]
pub struct ParsedCommand {
    /// Entry used for parsing, discovery, and the idle-boundary notice.
    pub spec: &'static CommandSpec,
    /// Typed executor and arguments.
    pub command: Command,
}

// Collect route variants and entries in one declaration. A new host command
// needs one entry below and one exhaustive handler arm, with no separate enum
// or name-to-parser list to update. Entries retain their discovery order.
macro_rules! command_registry {
    (@collect [$($ui:tt)*] [$($host:tt)*] [$($confirm:tt)*] [$($spec:expr,)*];) => {
        /// Commands handled by the application reducer.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum UiCommand { $($ui)* }
        /// Commands accepted by the local host executor.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum HostCommand { $($host)* }
        /// Commands for which the application opens a confirmation.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum ConfirmCommand { $($confirm)* }
        /// The complete implemented command set in discovery order.
        pub static COMMANDS: &[CommandSpec] = &[$($spec,)*];
    };
    (@collect [$($ui:tt)*] [$($host:tt)*] [$($confirm:tt)*] [$($spec:expr,)*];
        $(#[$doc:meta])* Ui $variant:ident $(($value:ty))? => $entry:expr, $($rest:tt)*) => {
        command_registry!(@collect
            [$($ui)* $(#[$doc])* $variant $(($value))?,]
            [$($host)*] [$($confirm)*] [$($spec,)* $entry,]; $($rest)*);
    };
    (@collect [$($ui:tt)*] [$($host:tt)*] [$($confirm:tt)*] [$($spec:expr,)*];
        $(#[$doc:meta])* Host $variant:ident $(($value:ty))? => $entry:expr, $($rest:tt)*) => {
        command_registry!(@collect
            [$($ui)*] [$($host)* $(#[$doc])* $variant $(($value))?,]
            [$($confirm)*] [$($spec,)* $entry,]; $($rest)*);
    };
    (@collect [$($ui:tt)*] [$($host:tt)*] [$($confirm:tt)*] [$($spec:expr,)*];
        $(#[$doc:meta])* Confirm $variant:ident $(($value:ty))? => $entry:expr, $($rest:tt)*) => {
        command_registry!(@collect
            [$($ui)*] [$($host)*] [$($confirm)* $(#[$doc])* $variant $(($value))?,]
            [$($spec,)* $entry,]; $($rest)*);
    };
    (@collect [$($ui:tt)*] [$($host:tt)*] [$($confirm:tt)*] [$($spec:expr,)*];
        Session => $entry:expr, $($rest:tt)*) => {
        command_registry!(@collect
            [$($ui)*] [$($host)*] [$($confirm)*] [$($spec,)* $entry,]; $($rest)*);
    };
    // /context and /agent have argument-dependent routes. Their additional
    // typed variants belong to those same entries, without advertising a row.
    (@collect [$($ui:tt)*] [$($host:tt)*] [$($confirm:tt)*] [$($spec:expr,)*];
        $(#[$doc:meta])* Host $variant:ident; $($rest:tt)*) => {
        command_registry!(@collect
            [$($ui)*] [$($host)* $(#[$doc])* $variant,]
            [$($confirm)*] [$($spec,)*]; $($rest)*);
    };
    (@collect [$($ui:tt)*] [$($host:tt)*] [$($confirm:tt)*] [$($spec:expr,)*];
        $(#[$doc:meta])* Confirm $variant:ident($value:ty); $($rest:tt)*) => {
        command_registry!(@collect
            [$($ui)*] [$($host)*] [$($confirm)* $(#[$doc])* $variant($value),]
            [$($spec,)*]; $($rest)*);
    };
    ($($entries:tt)*) => { command_registry!(@collect [] [] [] []; $($entries)*); };
}

command_registry! {
    /// List available commands.
    Ui Help => CommandSpec {
        name: "help",
        argument_hint: "",
        description: "List commands and keys",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::NoValue(|| Command::Ui(UiCommand::Help)),
        usage_example: "/help",
        complete_without_value: true,
    },
    /// Inspect or control a persistent multi-turn goal.
    Host Goal(GoalAction) => CommandSpec {
        name: "goal",
        argument_hint: "[OBJECTIVE|edit …|budget N|pause|resume|clear]",
        description: "Inspect or control a multi-turn goal",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::WholeValue(parse_goal),
        usage_example: "/goal ship the persistent goal system",
        complete_without_value: false,
    },
    /// Show context usage or select a named window.
    Ui Context(String) => CommandSpec {
        name: "context",
        argument_hint: "[NAME|default]",
        description: "Show context usage or choose a window",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(parse_context),
        usage_example: "/context 272k",
        complete_without_value: false,
    },
    /// Show session, usage, and workspace status.
    Host Status => CommandSpec {
        name: "status",
        argument_hint: "[--verbose]",
        description: "Show session, usage, and workspace status",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(parse_status),
        usage_example: "/status --verbose",
        complete_without_value: true,
    },
    /// Switch model.
    Ui Model(Option<String>) => CommandSpec {
        name: "model",
        argument_hint: "[PROVIDER/MODEL]",
        description: "Switch model",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Ui(UiCommand::Model(value)))),
        usage_example: "/model zai",
        complete_without_value: false,
    },
    /// Visualize the latest model-facing context plan.
    Host Context;
    /// Toggle bounded live tool detail.
    Ui Details => CommandSpec {
        name: "details",
        argument_hint: "",
        description: "Toggle bounded live tool detail",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::NoValue(|| Command::Ui(UiCommand::Details)),
        usage_example: "/details",
        complete_without_value: true,
    },
    /// Show local turn, child, and recovery history.
    Host Timeline => CommandSpec {
        name: "timeline",
        argument_hint: "",
        description: "Show local turn, child, and recovery history",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::NoValue(|| Command::Host(HostCommand::Timeline)),
        usage_example: "/timeline",
        complete_without_value: true,
    },
    Session => CommandSpec {
        name: "new",
        argument_hint: "",
        description: "Start a fresh session",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::NoValue(|| Command::Session(SessionControl::Reconfigure(SelectionCommand::NewSession))),
        usage_example: "/new",
        complete_without_value: true,
    },
    /// Resume a saved session.
    Ui Resume(Option<String>) => CommandSpec {
        name: "resume",
        argument_hint: "[ID]",
        description: "Resume a saved session",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Ui(UiCommand::Resume(value)))),
        usage_example: "/resume session-7",
        complete_without_value: false,
    },
    /// Connect or reconnect a provider.
    Ui Connect(Option<String>) => CommandSpec {
        name: "connect",
        argument_hint: "[PROVIDER]",
        description: "Connect or reconnect a provider",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Ui(UiCommand::Connect(value)))),
        usage_example: "/connect openrouter",
        complete_without_value: false,
    },
    /// Disconnect a provider.
    Ui Disconnect(Option<String>) => CommandSpec {
        name: "disconnect",
        argument_hint: "[PROVIDER]",
        description: "Disconnect a provider",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Ui(UiCommand::Disconnect(value)))),
        usage_example: "/disconnect openrouter",
        complete_without_value: false,
    },
    /// Set thinking for the next turn.
    Ui Think(Option<String>) => CommandSpec {
        name: "think",
        argument_hint: "[on|off|default]",
        description: "Set thinking for the next turn",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Ui(UiCommand::Think(value)))),
        usage_example: "/think off",
        complete_without_value: false,
    },
    /// Set reasoning effort for the next turn.
    Ui Effort(Option<String>) => CommandSpec {
        name: "effort",
        argument_hint: "[LEVEL|default]",
        description: "Set reasoning effort for the next turn",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Ui(UiCommand::Effort(value)))),
        usage_example: "/effort high",
        complete_without_value: false,
    },
    /// Turn the advisor on or off, or choose who advises.
    Ui Advisor(Option<String>) => CommandSpec {
        name: "advisor",
        argument_hint: "[on|off|default|TARGET]",
        description: "Turn the advisor on or off, or choose it",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Ui(UiCommand::Advisor(value)))),
        usage_example: "/advisor off",
        complete_without_value: false,
    },
    /// Show provider accounts and their usage.
    Ui Account(Option<String>) => CommandSpec {
        name: "account",
        argument_hint: "[N]",
        description: "Show provider accounts and their usage",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Ui(UiCommand::Account(value)))),
        usage_example: "/account 2",
        complete_without_value: false,
    },
    /// List, inspect, or resume an existing agent.
    Host Agent(AgentAction) => CommandSpec {
        name: "agent",
        argument_hint: "[ID|resume ID]",
        description: "List, inspect, or resume an existing agent",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::Subcommand(parse_agent),
        usage_example: "/agent child-7",
        complete_without_value: false,
    },
    /// Explicitly continue one interrupted child's exact checkpoint.
    Confirm AgentResume(String);
    /// Show MCP servers, or trust one so it may run.
    Host Mcp(McpAction) => CommandSpec {
        name: "mcp",
        argument_hint: "[trust NAME]",
        description: "Show MCP servers, or trust one so it may run",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::Subcommand(parse_mcp),
        usage_example: "/mcp trust github",
        complete_without_value: false,
    },
    /// Show what the session can use, or narrow it for this session.
    Host Capabilities(CapabilitiesAction) => CommandSpec {
        name: "capabilities",
        argument_hint: "[deny ID|allow ID]",
        description: "Show capabilities, or deny one for this session",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::Subcommand(parse_capabilities),
        usage_example: "/capabilities deny tool:shell",
        complete_without_value: true,
    },
    /// Show indexed skills, or trust one this project ships.
    Host Skills(SkillsAction) => CommandSpec {
        name: "skills",
        argument_hint: "[trust NAME]",
        description: "Show indexed skills, or trust one this project ships",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::Subcommand(parse_skills),
        usage_example: "/skills trust deploy",
        complete_without_value: false,
    },
    /// Inspect workspace changes.
    Host Diff(DiffScope) => CommandSpec {
        name: "diff",
        argument_hint: "[SCOPE]",
        description: "Inspect workspace changes",
        requires_idle: false,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(parse_diff),
        usage_example: "/diff staged",
        complete_without_value: false,
    },
    /// Run a read-only change review.
    Host Review(Option<String>) => CommandSpec {
        name: "review",
        argument_hint: "[SCOPE]",
        description: "Run a read-only change review",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Host(HostCommand::Review(value)))),
        usage_example: "/review all",
        complete_without_value: false,
    },
    /// Undo the last attributable turn.
    Host Undo => CommandSpec {
        name: "undo",
        argument_hint: "",
        description: "Undo the last attributable turn",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::NoValue(|| Command::Host(HostCommand::Undo)),
        usage_example: "/undo",
        complete_without_value: true,
    },
    /// Reapply the newest exact undone turn.
    Host Redo => CommandSpec {
        name: "redo",
        argument_hint: "",
        description: "Reapply the newest exact undone turn",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::NoValue(|| Command::Host(HostCommand::Redo)),
        usage_example: "/redo",
        complete_without_value: true,
    },
    /// Selectively revert a file or hunk.
    Host Revert(Option<String>) => CommandSpec {
        name: "revert",
        argument_hint: "[FILE]",
        description: "Selectively revert a file or hunk",
        requires_idle: true,
        advanced: false,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Host(HostCommand::Revert(value)))),
        usage_example: "/revert tracked.txt",
        complete_without_value: false,
    },
    /// Show detailed cache and recovery diagnostics.
    Host Diagnostics => CommandSpec {
        name: "diagnostics",
        argument_hint: "",
        description: "Show detailed cache and recovery diagnostics",
        requires_idle: false,
        advanced: true,
        grammar: ArgumentGrammar::NoValue(|| Command::Host(HostCommand::Diagnostics)),
        usage_example: "/diagnostics",
        complete_without_value: true,
    },
    /// Switch configured profile.
    Ui Profile(Option<String>) => CommandSpec {
        name: "profile",
        argument_hint: "[NAME]",
        description: "Switch configured profile",
        requires_idle: true,
        advanced: true,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Ui(UiCommand::Profile(value)))),
        usage_example: "/profile work",
        complete_without_value: false,
    },
    /// Switch provider.
    Ui Provider(Option<String>) => CommandSpec {
        name: "provider",
        argument_hint: "[NAME]",
        description: "Switch provider",
        requires_idle: true,
        advanced: true,
        grammar: ArgumentGrammar::OptionalValue(|value| Ok(Command::Ui(UiCommand::Provider(value)))),
        usage_example: "/provider local",
        complete_without_value: false,
    },
    /// Exit Smith.
    Ui Quit => CommandSpec {
        name: "quit",
        argument_hint: "",
        description: "Exit Smith",
        requires_idle: false,
        advanced: true,
        grammar: ArgumentGrammar::NoValue(|| Command::Ui(UiCommand::Quit)),
        usage_example: "/quit",
        complete_without_value: true,
    },
}

/// Registry entries matching a composer draft or palette query.
pub fn matches(input: &str) -> Vec<&'static CommandSpec> {
    let query = input.trim().trim_start_matches('/');
    let name = query.split_whitespace().next().unwrap_or_default();
    let name_matches = COMMANDS
        .iter()
        .filter(|command| command.name.starts_with(name))
        .collect::<Vec<_>>();
    if !name_matches.is_empty() || name.is_empty() {
        return name_matches;
    }

    // A command's description is part of its discoverable surface. Keep name
    // prefixes authoritative so `/mod` never turns into a description search,
    // then let intent words such as `switch` find the same registered rows.
    let query = query.to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|command| command.description.to_ascii_lowercase().contains(&query))
        .collect()
}

/// Whether the first token names a registered command exactly.
///
/// The palette uses this distinction to decide whether Enter should preserve
/// parser errors for an explicitly named command or activate its highlighted
/// completion for a partial/intent query.
pub fn has_exact_name(input: &str) -> bool {
    let query = input.trim().trim_start_matches('/');
    let Some(name) = query.split_whitespace().next() else {
        return false;
    };
    COMMANDS.iter().any(|command| command.name == name)
}

/// Completes the selected command without executing it.
pub fn completion(command: &CommandSpec) -> String {
    // /status is complete by itself; its diagnostic flag is optional.
    if command.complete_without_value {
        format!("/{}", command.name)
    } else {
        format!("/{} ", command.name)
    }
}

/// Parses one command using the same entry shown by discovery and help.
pub fn parse(input: &str) -> Result<ParsedCommand, String> {
    let trimmed = input.trim().trim_start_matches('/');
    let Some(name) = trimmed.split_whitespace().next() else {
        return Err("select or enter a command".to_owned());
    };
    let Some(spec) = COMMANDS.iter().find(|command| command.name == name) else {
        return Err(format!("unknown command `/{name}` — type /help"));
    };
    let argument = trimmed.strip_prefix(name).unwrap_or_default().trim();
    let command = match spec.grammar {
        ArgumentGrammar::WholeValue(parse) => parse(argument)?,
        grammar => {
            let mut words = argument.split_whitespace();
            let first = words.next().map(str::to_owned);
            let second = words.next().map(str::to_owned);
            if words.next().is_some() {
                return Err(format!("`/{name}` accepts at most one value"));
            }
            match grammar {
                ArgumentGrammar::NoValue(command) => {
                    if first.is_some() {
                        return Err(format!("`/{name}` takes no value"));
                    }
                    command()
                }
                ArgumentGrammar::OptionalValue(parse) => {
                    if second.is_some() {
                        return Err(format!("`/{name}` accepts at most one value"));
                    }
                    parse(first)?
                }
                ArgumentGrammar::Subcommand(parse) => parse(first, second)?,
                ArgumentGrammar::WholeValue(parse) => parse(argument)?,
            }
        }
    };
    Ok(ParsedCommand { spec, command })
}

fn parse_status(argument: Option<String>) -> Result<Command, String> {
    match argument.as_deref() {
        None => Ok(Command::Host(HostCommand::Status)),
        Some("--verbose") => Ok(Command::Host(HostCommand::Diagnostics)),
        _ => Err("use `/status` or `/status --verbose`".to_owned()),
    }
}

fn parse_context(argument: Option<String>) -> Result<Command, String> {
    Ok(match argument {
        None => Command::Host(HostCommand::Context),
        Some(value) => Command::Ui(UiCommand::Context(value)),
    })
}

fn parse_diff(argument: Option<String>) -> Result<Command, String> {
    let scope = match argument.as_deref() {
        Some("last-turn") => DiffScope::LastTurn,
        _ => DiffScope::Git(argument),
    };
    Ok(Command::Host(HostCommand::Diff(scope)))
}

fn parse_agent(argument: Option<String>, second: Option<String>) -> Result<Command, String> {
    if argument.as_deref() == Some("resume") {
        let child = second.ok_or_else(|| "`/agent resume` requires a child ID".to_owned())?;
        return Ok(Command::Confirm(ConfirmCommand::AgentResume(child)));
    }
    if second.is_some() {
        return Err("`/agent` accepts at most one value".to_owned());
    }
    let action = match argument {
        None => AgentAction::List,
        Some(value) => match value.as_str() {
            "parent" => AgentAction::Parent,
            "next" => AgentAction::Next,
            "previous" => AgentAction::Previous,
            _ => AgentAction::Inspect(value),
        },
    };
    Ok(Command::Host(HostCommand::Agent(action)))
}

fn parse_mcp(argument: Option<String>, second: Option<String>) -> Result<Command, String> {
    match (argument.as_deref(), second) {
        (None, _) => Ok(Command::Host(HostCommand::Mcp(McpAction::List))),
        (Some("trust"), Some(server)) => {
            Ok(Command::Host(HostCommand::Mcp(McpAction::Trust(server))))
        }
        (Some("trust"), None) => {
            Err("`/mcp trust` requires a server name — run `/mcp` to list them".to_owned())
        }
        (Some(other), _) => Err(format!(
            "`/mcp` takes no value, or `trust NAME`; `{other}` is neither"
        )),
    }
}

fn parse_capabilities(argument: Option<String>, second: Option<String>) -> Result<Command, String> {
    let action = match (argument.as_deref(), second) {
        (None, _) => CapabilitiesAction::List,
        (Some(verb @ ("deny" | "allow")), Some(pattern)) => {
            smith_config::resolve::validate_capability_pattern(&pattern)?;
            if verb == "deny" {
                CapabilitiesAction::Deny(pattern)
            } else {
                CapabilitiesAction::Allow(pattern)
            }
        }
        (Some(verb @ ("deny" | "allow")), None) => {
            return Err(format!(
                "`/capabilities {verb}` requires an id such as `tool:shell` — run \
                 `/capabilities` to list them"
            ));
        }
        (Some(other), _) => {
            return Err(format!(
                "`/capabilities` takes no value, `deny ID`, or `allow ID`; `{other}` is none of those"
            ));
        }
    };
    Ok(Command::Host(HostCommand::Capabilities(action)))
}

fn parse_skills(argument: Option<String>, second: Option<String>) -> Result<Command, String> {
    match (argument.as_deref(), second) {
        (None, _) => Ok(Command::Host(HostCommand::Skills(SkillsAction::List))),
        (Some("trust"), Some(skill)) => Ok(Command::Host(HostCommand::Skills(
            SkillsAction::Trust(skill),
        ))),
        (Some("trust"), None) => {
            Err("`/skills trust` requires a skill name — run `/skills` to list them".to_owned())
        }
        (Some(other), _) => Err(format!(
            "`/skills` takes no value, or `trust NAME`; `{other}` is neither"
        )),
    }
}

fn parse_goal(argument: &str) -> Result<Command, String> {
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
    Ok(Command::Host(HostCommand::Goal(action)))
}

/// The shared startup suggestions, in order, with registry descriptions.
pub fn getting_started_commands() -> impl Iterator<Item = &'static CommandSpec> {
    ["model", "connect", "help"].into_iter().map(|name| {
        COMMANDS
            .iter()
            .find(|command| command.name == name)
            .expect("getting-started command exists in the registry")
    })
}

/// The typed `/help` guide, derived from the registry.
pub fn help() -> HelpReport {
    HelpReport {
        introduction: "Type a task and press Enter.".to_owned(),
        getting_started: getting_started_commands()
            .map(|command| HelpCommand {
                name: command.name.to_owned(),
                argument_hint: String::new(),
                description: command.description.to_owned(),
            })
            .collect(),
        primary: COMMANDS
            .iter()
            .filter(|command| !command.advanced)
            .map(help_command)
            .collect(),
        advanced: COMMANDS
            .iter()
            .filter(|command| command.advanced)
            .map(help_command)
            .collect(),
        composer: [
            "? or /help shows this local guide without contacting the model.",
            "Tab cycles the configured profile order only while empty and idle.",
            "While work is serving, Enter steers an ordinary prompt and Tab queues it.",
            "Alt+Up restores the newest explicit queued turn for editing.",
            "Esc interrupts; uncommitted steers are resent only after cancellation discards them.",
            "Ctrl+B moves a running foreground shell command to the background without killing it.",
            "@ completes exact files and read-only agents; @@ sends a literal @.",
            "! runs a prepared local shell action; !! sends a literal !.",
            "PageUp/PageDown/Home/End or the mouse wheel scrolls the transcript.",
            "Up/Down browse accepted and Ctrl+C-stashed input without losing your draft.",
            "Down past the newest draft walks the delegated agents; the transcript shows",
            "that agent's log, Enter continues it, and Esc returns to the root timeline.",
            "Ctrl+R searches composer history; Enter restores a match and Esc cancels.",
            "Ctrl+C twice within 1s exits; the first press stashes and clears the draft.",
            "Start a message with // to send a literal leading slash.",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        keys: help_keys(),
    }
}

/// The shared key table for `/help` and the ephemeral shortcuts panel.
pub fn help_keys() -> Vec<HelpKey> {
    crate::keymap::KEY_BINDINGS
        .iter()
        .map(|binding| HelpKey {
            key: binding.label.to_owned(),
            description: binding.description.to_owned(),
        })
        .collect()
}

fn help_command(command: &CommandSpec) -> HelpCommand {
    HelpCommand {
        name: command.name.to_owned(),
        argument_hint: command.argument_hint.to_owned(),
        description: command.description.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Result<Command, String> {
        super::parse(input).map(|parsed| parsed.command)
    }

    #[test]
    fn status_completion_remains_a_complete_bare_command() {
        let status = COMMANDS
            .iter()
            .find(|command| command.name == "status")
            .unwrap();
        assert_eq!(completion(status), "/status");
        assert_eq!(
            parse(&completion(status)).unwrap(),
            Command::Host(HostCommand::Status)
        );
        assert_eq!(
            parse("/status --verbose").unwrap(),
            Command::Host(HostCommand::Diagnostics)
        );
        let model = COMMANDS
            .iter()
            .find(|command| command.name == "model")
            .unwrap();
        assert_eq!(completion(model), "/model ");
    }

    #[test]
    fn help_and_completion_share_the_complete_registry() {
        let report = help();
        assert_eq!(
            report
                .getting_started
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["model", "connect", "help"]
        );
        for command in &report.getting_started {
            let registered = COMMANDS
                .iter()
                .find(|registered| registered.name == command.name)
                .unwrap();
            assert_eq!(command.description, registered.description);
            assert!(command.argument_hint.is_empty());
        }
        let help = crate::help_report::render_plain(&report);
        assert!(help.starts_with("Getting started\n"));
        assert!(help.contains("/model — Switch model"));
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
    fn goal_argument_hint_uses_compact_alternatives_in_completion_and_help() {
        let goal = matches("/goal")[0];
        let expected = "[OBJECTIVE|edit …|budget N|pause|resume|clear]";
        assert_eq!(goal.argument_hint, expected);
        let guide = help();
        let help_goal = guide
            .primary
            .iter()
            .find(|command| command.name == "goal")
            .unwrap();
        assert_eq!(help_goal.argument_hint, expected);
    }

    #[test]
    fn description_search_is_used_only_after_name_prefixes() {
        assert_eq!(
            matches("switch")
                .into_iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["model", "profile", "provider"]
        );
        assert_eq!(
            matches("/pro")
                .into_iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["profile", "provider"]
        );
        assert!(has_exact_name("/status --verbose"));
        assert!(!has_exact_name("/sta"));
    }

    #[test]
    fn parser_returns_typed_actions_and_actionable_errors() {
        assert_eq!(
            parse("/model zai").expect("model"),
            Command::Ui(UiCommand::Model(Some("zai".into())))
        );
        assert_eq!(
            parse("/diff staged").expect("diff"),
            Command::Host(HostCommand::Diff(DiffScope::Git(Some("staged".into()))))
        );
        assert_eq!(
            parse("/context").expect("context"),
            Command::Host(HostCommand::Context)
        );
        assert_eq!(
            parse("/context 272k").expect("named context window"),
            Command::Ui(UiCommand::Context("272k".into()))
        );
        assert_eq!(
            parse("/context default").expect("default context window"),
            Command::Ui(UiCommand::Context("default".into()))
        );
        assert_eq!(
            parse("/model").expect("picker"),
            Command::Ui(UiCommand::Model(None))
        );
        assert_eq!(
            parse("/capabilities").expect("capability listing"),
            Command::Host(HostCommand::Capabilities(CapabilitiesAction::List))
        );
        assert_eq!(
            parse("/capabilities deny tool:shell").expect("session denial"),
            Command::Host(HostCommand::Capabilities(CapabilitiesAction::Deny(
                "tool:shell".into()
            )))
        );
        assert!(
            parse("/capabilities deny shell")
                .expect_err("not a pattern")
                .contains("<domain>:<name>")
        );
        assert_eq!(
            parse("/advisor").expect("advisor picker"),
            Command::Ui(UiCommand::Advisor(None))
        );
        assert_eq!(
            parse("/advisor acme/big").expect("advisor target"),
            Command::Ui(UiCommand::Advisor(Some("acme/big".into())))
        );
        assert_eq!(
            parse("/connect openrouter").expect("connection"),
            Command::Ui(UiCommand::Connect(Some("openrouter".into())))
        );
        assert_eq!(
            parse("/disconnect").expect("disconnect picker"),
            Command::Ui(UiCommand::Disconnect(None))
        );
        assert_eq!(
            parse("/think off").expect("thinking state"),
            Command::Ui(UiCommand::Think(Some("off".into())))
        );
        assert_eq!(
            parse("/effort high").expect("effort"),
            Command::Ui(UiCommand::Effort(Some("high".into())))
        );
        assert_eq!(
            parse("/think").expect("picker"),
            Command::Ui(UiCommand::Think(None))
        );
        assert_eq!(
            parse("/effort").expect("picker"),
            Command::Ui(UiCommand::Effort(None))
        );
        assert_eq!(
            parse("/agent resume child-7").expect("child resume"),
            Command::Confirm(ConfirmCommand::AgentResume("child-7".into()))
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
            Command::Host(HostCommand::Goal(GoalAction::Show))
        );
        assert_eq!(
            parse("/goal ship the persistent goal system").unwrap(),
            Command::Host(HostCommand::Goal(GoalAction::Create(
                "ship the persistent goal system".into()
            )))
        );
        assert_eq!(
            parse("/goal edit ship it safely").unwrap(),
            Command::Host(HostCommand::Goal(GoalAction::Edit("ship it safely".into())))
        );
        assert_eq!(
            parse("/goal budget 12000").unwrap(),
            Command::Host(HostCommand::Goal(GoalAction::Budget(Some(12_000))))
        );
        assert_eq!(
            parse("/goal budget none").unwrap(),
            Command::Host(HostCommand::Goal(GoalAction::Budget(None)))
        );
        assert_eq!(
            parse("/goal pause").unwrap(),
            Command::Host(HostCommand::Goal(GoalAction::Pause))
        );
        assert!(parse("/goal edit").unwrap_err().contains("objective"));
        assert!(parse("/goal budget 0").unwrap_err().contains("positive"));
    }

    #[test]
    fn slash_skills_parses_its_list_and_trust_forms() {
        assert_eq!(
            parse("/skills"),
            Ok(Command::Host(HostCommand::Skills(SkillsAction::List)))
        );
        assert_eq!(
            parse("/skills trust deploy"),
            Ok(Command::Host(HostCommand::Skills(SkillsAction::Trust(
                "deploy".into()
            ))))
        );
        let error = parse("/skills trust").expect_err("a name is required");
        assert!(error.contains("requires a skill name"), "{error}");
        let error = parse("/skills deploy").expect_err("trust is the only verb");
        assert!(error.contains("is neither"), "{error}");
    }

    #[test]
    fn the_mcp_command_parses_its_only_two_forms() {
        assert_eq!(
            parse("/mcp"),
            Ok(Command::Host(HostCommand::Mcp(McpAction::List)))
        );
        assert_eq!(
            parse("/mcp trust github"),
            Ok(Command::Host(HostCommand::Mcp(McpAction::Trust(
                "github".to_owned()
            ))))
        );
        assert!(parse("/mcp trust").is_err());
        assert!(parse("/mcp nonsense").is_err());
    }

    #[test]
    fn every_entry_parses_its_own_usage_example() {
        let mut names = std::collections::BTreeSet::new();
        for spec in COMMANDS {
            assert!(names.insert(spec.name), "duplicate command {}", spec.name);
            let parsed = super::parse(spec.usage_example)
                .unwrap_or_else(|error| panic!("{}: {error}", spec.usage_example));
            assert_eq!(parsed.spec.name, spec.name, "{}", spec.usage_example);
            let completed = super::parse(&completion(spec)).expect("completion parses");
            assert_eq!(completed.spec.name, spec.name);
        }
    }

    #[test]
    fn agent_and_diff_subarguments_are_routed_before_dispatch() {
        for (input, action) in [
            ("/agent", AgentAction::List),
            ("/agent parent", AgentAction::Parent),
            ("/agent next", AgentAction::Next),
            ("/agent previous", AgentAction::Previous),
            ("/agent child-7", AgentAction::Inspect("child-7".into())),
        ] {
            assert_eq!(
                parse(input).unwrap(),
                Command::Host(HostCommand::Agent(action))
            );
        }
        assert_eq!(
            parse("/diff last-turn").unwrap(),
            Command::Host(HostCommand::Diff(DiffScope::LastTurn))
        );
        for scope in [
            None,
            Some("all"),
            Some("staged"),
            Some("unstaged"),
            Some("untracked"),
            Some("commit:HEAD"),
            Some("base:main"),
            Some("file.txt"),
        ] {
            let input = scope.map_or_else(|| "/diff".to_owned(), |value| format!("/diff {value}"));
            assert_eq!(
                parse(&input).unwrap(),
                Command::Host(HostCommand::Diff(DiffScope::Git(scope.map(str::to_owned))))
            );
        }
        assert!(parse("/agent resume child-7 extra").is_err());
        assert!(parse("/agent next extra").is_err());
        assert!(parse("/diff staged extra").is_err());
        assert_eq!(
            parse("/accounts").unwrap_err(),
            "unknown command `/accounts` — type /help"
        );
    }
}

#[cfg(test)]
mod diagnostics_tests {
    use super::*;

    fn parse(input: &str) -> Result<Command, String> {
        super::parse(input).map(|parsed| parsed.command)
    }

    #[test]
    fn diagnostics_are_explicit_and_status_stays_concise() {
        assert_eq!(
            parse("/status").unwrap(),
            Command::Host(HostCommand::Status)
        );
        assert_eq!(
            parse("/status --verbose").unwrap(),
            Command::Host(HostCommand::Diagnostics)
        );
        assert_eq!(
            parse("/diagnostics").unwrap(),
            Command::Host(HostCommand::Diagnostics)
        );
        assert!(parse("/status nonsense").is_err());
    }
}
