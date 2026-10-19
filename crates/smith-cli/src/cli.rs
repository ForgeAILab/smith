//! Stable command-line parsing without coupling Smith's product types to a
//! parser framework.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::PathBuf;

use smith_config::model::{ApprovalMode, BackgroundExit};
use smith_config::resolve::Overrides;

mod plugin;
pub(crate) use plugin::{MarketplaceAction, PluginAction};

/// The command selected by process arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    /// Start an interactive or one-prompt agent session.
    Run(RunArgs),
    /// Explain one resolved configuration key.
    ConfigExplain {
        /// The dotted key to explain.
        key: String,
        /// Selection flags that participate in resolution.
        selection: Selection,
    },
    /// List known module choices and composition outcomes without a session.
    ConfigModules {
        /// Selection flags that participate in resolution.
        selection: Selection,
    },
    /// List persisted sessions for a project.
    SessionsList {
        /// The project whose session partition to inspect.
        selection: Selection,
    },
    /// Synchronize configured provider model metadata without inference.
    ModelsSync {
        provider: Option<String>,
        force: bool,
    },
    /// Manage user-owned plugin bundles without starting a session.
    Plugin(PluginAction),
    /// Enter guided user-scoped provider/model setup.
    Setup(SetupArgs),
    /// Print command help.
    Help,
    /// Print the binary version.
    Version,
    /// Hidden component-engine diagnostic used by the dependency-cost gate.
    WasmEngineDiagnostic,
    /// Hidden byte relay for a turn's private MCP socket.
    McpRelay {
        /// Absolute path of the turn's Unix socket.
        socket: PathBuf,
    },
    /// Hidden lifetime supervisor for an installed harness.
    HarnessSupervise {
        program: OsString,
        args: Vec<OsString>,
    },
    /// Hidden lifetime watchdog for a stdio MCP server.
    McpWatchdog {
        parent: u32,
        program: OsString,
        args: Vec<OsString>,
    },
}

/// Which guided setup path was requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SetupAction {
    /// Show the reusable action menu.
    Menu,
    /// Add a provider and its first model.
    AddProvider,
    /// Add a model beneath an existing provider.
    AddModel {
        /// Preselected provider; absence opens the provider picker.
        provider: Option<String>,
    },
    /// Change only one existing provider's credential source.
    Credential {
        /// Provider to migrate.
        provider: String,
    },
    /// Configure the authenticated-encryption key for exact checkpoints.
    CheckpointKey,
}

/// Presentation and project inputs for guided setup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SetupArgs {
    /// Requested setup path.
    pub action: SetupAction,
    /// Project used only for post-commit preflight.
    pub project: Option<PathBuf>,
    /// Disable terminal color.
    pub no_color: bool,
    /// Disable animation.
    pub no_motion: bool,
}

/// Provider and policy inputs shared by all commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Selection {
    /// Where project discovery starts.
    pub project: Option<PathBuf>,
    /// Deprecated root-mode compatibility override; prefer `profile`.
    pub agent: Option<String>,
    /// A named profile override.
    pub profile: Option<String>,
    /// A named provider override.
    pub provider: Option<String>,
    /// A model override.
    pub model: Option<String>,
    /// Session-local explicit thinking state.
    pub reasoning_enabled: Option<bool>,
    /// An explicit `/think default` clears a persisted override.
    pub reasoning_enabled_reset: bool,
    /// Session-local provider-advertised effort.
    pub reasoning_effort: Option<String>,
    /// An explicit `/effort default` clears a persisted override.
    pub reasoning_effort_reset: bool,
    /// `--effort <NAME>`: one provider-advertised effort for this invocation.
    ///
    /// Deliberately separate from `reasoning_effort` above. That field carries
    /// an in-session `/effort` choice and becomes the session layer; this one
    /// was typed on a command line and becomes the command-line layer, so the
    /// two never have to pretend to be each other in diagnostics or in what a
    /// resumed session saves.
    pub effort: Option<String>,
    /// `--context-window <NAME>`: context selection for this invocation.
    pub context_window_flag: Option<String>,
    /// Session-local context-window choice from `/context`.
    pub context_window: Option<String>,
    /// An explicit `/context default` clears a persisted selection.
    pub context_window_reset: bool,
    /// Session-local `/advisor` choice; absent uses the configured advisor.
    pub advisor: Option<smith_config::resolve::AdvisorOverride>,
    /// Capability patterns `/capabilities deny` added for this session.
    pub capability_denials: Vec<String>,
    /// An approval policy override.
    pub approval: Option<ApprovalMode>,
    /// A background-exit policy override.
    pub background_exit: Option<BackgroundExit>,
    /// Explicit trusted-host authority for bounded synthetic cache spend.
    pub allow_synthetic_cache_spend: bool,
}

impl Selection {
    /// Converts parsed flags into the config resolver's typed CLI layer.
    pub(crate) fn overrides(&self) -> Overrides {
        Overrides {
            agent: self.agent.clone(),
            profile: self.profile.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            reasoning_effort: self.effort.clone(),
            context_window: self.context_window_flag.clone(),
            approval_mode: self.approval,
            background_exit: self.background_exit,
            ..Overrides::default()
        }
    }

    /// Converts interactive controls into the highest-precedence session
    /// layer rather than pretending they were command-line flags.
    pub(crate) fn session_overrides(&self) -> Overrides {
        Overrides {
            reasoning_enabled: self.reasoning_enabled,
            reasoning_effort: self.reasoning_effort.clone(),
            context_window: self.context_window.clone(),
            ..Overrides::default()
        }
    }
}

/// One run's presentation-specific arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunArgs {
    /// Shared project/provider/policy selection.
    pub selection: Selection,
    /// A prompt makes the run headless; absence opens the TUI.
    pub prompt: Option<Prompt>,
    /// Caller-authorized image paths for a headless prompt.
    pub images: Vec<PathBuf>,
    /// An existing identity to resume.
    pub resume: Option<String>,
    /// Whether `--resume`/`--session` was supplied, including picker form.
    pub resume_requested: bool,
    /// Human or machine output encoding.
    pub output: OutputFormat,
    /// Disable terminal color attributes.
    pub no_color: bool,
    /// Disable animation.
    pub no_motion: bool,
}

impl Default for RunArgs {
    fn default() -> Self {
        Self {
            selection: Selection::default(),
            prompt: None,
            images: Vec::new(),
            resume: None,
            resume_requested: false,
            output: OutputFormat::Text,
            no_color: false,
            no_motion: false,
        }
    }
}

/// Where a non-interactive prompt comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Prompt {
    /// Text supplied directly after `-p`.
    Argument(String),
    /// All UTF-8 bytes from standard input.
    Stdin,
}

/// A non-interactive output contract.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    /// The final assistant text only.
    #[default]
    Text,
    /// One versioned final result object.
    Json,
    /// Versioned JSON Lines events followed by a final result object.
    StreamJson,
}

impl OutputFormat {
    fn parse(value: &str) -> Result<Self, ParseError> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "stream-json" => Ok(Self::StreamJson),
            _ => Err(ParseError::new(format!(
                "invalid output format `{value}`; expected `text`, `json`, or `stream-json`"
            ))),
        }
    }
}

/// A user-facing argument diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub(crate) struct ParseError {
    message: String,
}

impl ParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Parses process arguments after the executable name.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, ParseError> {
    let args = VecDeque::from_iter(args);
    match args.front().and_then(|arg| arg.to_str()) {
        Some("mcp-relay") => parse_mcp_relay(args),
        Some("harness-supervise") => parse_harness_supervise(args),
        Some("mcp-watchdog") => parse_mcp_watchdog(args),
        Some("plugin") => plugin::parse(args),
        Some("config") => parse_config(args),
        Some("sessions") => parse_sessions(args),
        Some("setup") => parse_setup(args),
        Some("models") => parse_models(args),
        _ => parse_run(args),
    }
}

fn parse_mcp_relay(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    args.pop_front();
    let socket = args
        .pop_front()
        .map(PathBuf::from)
        .filter(|socket| socket.is_absolute() && args.is_empty())
        .ok_or_else(|| ParseError::new("usage: smith mcp-relay <absolute-socket-path>"))?;
    Ok(Command::McpRelay { socket })
}

fn parse_harness_supervise(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    args.pop_front();
    let usage = || ParseError::new("usage: smith harness-supervise -- <program> [args...]");
    if args.pop_front().as_deref() != Some(std::ffi::OsStr::new("--")) {
        return Err(usage());
    }
    let program = args
        .pop_front()
        .filter(|arg| !arg.is_empty())
        .ok_or_else(usage)?;
    Ok(Command::HarnessSupervise {
        program,
        args: args.into(),
    })
}

fn parse_mcp_watchdog(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    args.pop_front();
    let usage =
        || ParseError::new("usage: smith mcp-watchdog --parent <pid> -- <program> [args...]");
    if args.pop_front().as_deref() != Some(std::ffi::OsStr::new("--parent")) {
        return Err(usage());
    }
    let parent = args
        .pop_front()
        .and_then(|value| value.to_str()?.parse::<u32>().ok())
        .filter(|pid| *pid > 0 && *pid <= i32::MAX as u32)
        .ok_or_else(usage)?;
    if args.pop_front().as_deref() != Some(std::ffi::OsStr::new("--")) {
        return Err(usage());
    }
    let program = args
        .pop_front()
        .filter(|arg| !arg.is_empty())
        .ok_or_else(usage)?;
    Ok(Command::McpWatchdog {
        parent,
        program,
        args: args.into(),
    })
}

fn parse_models(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    args.pop_front();
    if args.front().is_some_and(|arg| is_help(arg.to_str())) {
        return Ok(Command::Help);
    }
    let action = required_text(args.pop_front(), "`sync` after `models`")?;
    if action != "sync" {
        return Err(ParseError::new(
            "expected `smith models sync [--provider NAME] [--force]`",
        ));
    }
    let mut provider = None;
    let mut force = false;
    while let Some(raw) = args.pop_front() {
        let (flag, inline) = flag(raw)?;
        match flag.as_str() {
            "help" | "--help" | "-h" => return Ok(Command::Help),
            "--provider" => {
                let parsed = text_value(&flag, inline, &mut args)?;
                set_once(&mut provider, parsed, &flag)?;
            }
            "--force" if inline.is_none() && !force => force = true,
            "--force" => {
                return Err(ParseError::new(
                    "`--force` takes no value and may be supplied once",
                ));
            }
            _ => {
                return Err(ParseError::new(format!(
                    "unknown models sync option `{flag}`"
                )));
            }
        }
    }
    Ok(Command::ModelsSync { provider, force })
}

fn parse_setup(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    args.pop_front();
    if args.front().is_some_and(|arg| is_help(arg.to_str())) {
        return Ok(Command::Help);
    }
    let action = match args.front().and_then(|value| value.to_str()) {
        Some("add-provider") => {
            args.pop_front();
            SetupAction::AddProvider
        }
        Some("add-model") => {
            args.pop_front();
            SetupAction::AddModel { provider: None }
        }
        Some("credential") => {
            args.pop_front();
            SetupAction::Credential {
                provider: String::new(),
            }
        }
        Some("checkpoint-key") => {
            args.pop_front();
            SetupAction::CheckpointKey
        }
        Some(value) if !value.starts_with('-') => {
            return Err(ParseError::new(format!(
                "unknown setup action `{value}`; expected `add-provider`, `add-model`, `credential`, or `checkpoint-key`"
            )));
        }
        _ => SetupAction::Menu,
    };
    let mut project = None;
    let mut provider = None;
    let mut no_color = false;
    let mut no_motion = false;
    while let Some(raw) = args.pop_front() {
        let (flag, inline) = flag(raw)?;
        match flag.as_str() {
            "help" | "--help" | "-h" => return Ok(Command::Help),
            "--project" => {
                let value = value(&flag, inline, &mut args)?;
                set_once(&mut project, PathBuf::from(value), &flag)?;
            }
            "--provider" => {
                let value = text_value(&flag, inline, &mut args)?;
                set_once(&mut provider, value, &flag)?;
            }
            "--no-color" => {
                if inline.is_some() || no_color {
                    return Err(ParseError::new(
                        "`--no-color` does not take a value and may be supplied once",
                    ));
                }
                no_color = true;
            }
            "--no-motion" => {
                if inline.is_some() || no_motion {
                    return Err(ParseError::new(
                        "`--no-motion` does not take a value and may be supplied once",
                    ));
                }
                no_motion = true;
            }
            _ => {
                return Err(ParseError::new(format!("unknown setup option `{flag}`")));
            }
        }
    }
    let action = match action {
        SetupAction::AddModel { .. } => SetupAction::AddModel { provider },
        SetupAction::Credential { .. } => SetupAction::Credential {
            provider: provider.ok_or_else(|| {
                ParseError::new("`smith setup credential` requires `--provider <NAME>`")
            })?,
        },
        _ if provider.is_some() => {
            return Err(ParseError::new(
                "`--provider` applies only to `smith setup add-model` or `smith setup credential`",
            ));
        }
        other => other,
    };
    Ok(Command::Setup(SetupArgs {
        action,
        project,
        no_color,
        no_motion,
    }))
}

fn parse_config(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    args.pop_front();
    if args.front().is_some_and(|arg| is_help(arg.to_str())) {
        return Ok(Command::Help);
    }
    let action = required_text(args.pop_front(), "an action after `config`")?;
    if action == "modules" {
        return match parse_selection_only(args)? {
            Some(selection) => Ok(Command::ConfigModules { selection }),
            None => Ok(Command::Help),
        };
    }
    if action != "explain" {
        return Err(ParseError::new(format!(
            "unknown config action `{action}`; expected `smith config explain <key>` or `smith config modules`"
        )));
    }
    if args.front().is_some_and(|arg| is_help(arg.to_str())) {
        return Ok(Command::Help);
    }
    let key = required_text(args.pop_front(), "a dotted key after `config explain`")?;
    match parse_selection_only(args)? {
        Some(selection) => Ok(Command::ConfigExplain { key, selection }),
        None => Ok(Command::Help),
    }
}

fn parse_sessions(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    args.pop_front();
    if args.front().is_some_and(|arg| is_help(arg.to_str())) {
        return Ok(Command::Help);
    }
    let action = required_text(args.pop_front(), "an action after `sessions`")?;
    if action != "list" {
        return Err(ParseError::new(format!(
            "unknown sessions action `{action}`; expected `smith sessions list`"
        )));
    }
    match parse_selection_only(args)? {
        Some(selection) => Ok(Command::SessionsList { selection }),
        None => Ok(Command::Help),
    }
}

fn is_help(arg: Option<&str>) -> bool {
    matches!(arg, Some("help" | "--help" | "-h"))
}

fn parse_selection_only(mut args: VecDeque<OsString>) -> Result<Option<Selection>, ParseError> {
    let mut selection = Selection::default();
    while let Some(raw) = args.pop_front() {
        let (flag, inline) = flag(raw)?;
        if is_help(Some(&flag)) {
            return Ok(None);
        }
        parse_selection_flag(&flag, inline, &mut args, &mut selection)?;
    }
    Ok(Some(selection))
}

fn parse_run(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    let mut run = RunArgs::default();
    let mut output_set = false;
    while let Some(raw) = args.pop_front() {
        let (flag, inline) = flag(raw)?;
        match flag.as_str() {
            "help" | "--help" | "-h" => return Ok(Command::Help),
            "--version" | "-V" => return Ok(Command::Version),
            "--wasm-engine-diagnostic" => return Ok(Command::WasmEngineDiagnostic),
            "-p" | "--prompt" => {
                let value = value(&flag, inline, &mut args)?;
                let prompt = required_text(Some(value), "a UTF-8 prompt after `-p`")?;
                set_once(&mut run.prompt, prompt_source(prompt), "--prompt")?;
            }
            "--image" => run
                .images
                .push(PathBuf::from(value(&flag, inline, &mut args)?)),
            "--resume" | "--session" => {
                if run.resume_requested {
                    return Err(ParseError::new("`--resume/--session` was supplied twice"));
                }
                run.resume_requested = true;
                run.resume = optional_text_value(&flag, inline, &mut args)?;
            }
            "--output-format" => {
                let parsed = OutputFormat::parse(&text_value(&flag, inline, &mut args)?)?;
                if output_set {
                    return Err(ParseError::new("`--output-format` was supplied twice"));
                }
                run.output = parsed;
                output_set = true;
            }
            "--no-color" => {
                if inline.is_some() {
                    return Err(ParseError::new("`--no-color` does not take a value"));
                }
                if run.no_color {
                    return Err(ParseError::new("`--no-color` was supplied twice"));
                }
                run.no_color = true;
            }
            "--no-motion" => {
                if inline.is_some() {
                    return Err(ParseError::new("`--no-motion` does not take a value"));
                }
                if run.no_motion {
                    return Err(ParseError::new("`--no-motion` was supplied twice"));
                }
                run.no_motion = true;
            }
            _ => {
                parse_selection_flag(&flag, inline, &mut args, &mut run.selection)?;
            }
        }
    }

    if run.prompt.is_none() && !run.images.is_empty() {
        return Err(ParseError::new(
            "`--image` requires non-interactive `-p <prompt>`",
        ));
    }
    if run.prompt.is_none() && run.output != OutputFormat::Text {
        return Err(ParseError::new(
            "`--output-format` requires non-interactive `-p <prompt>`",
        ));
    }
    Ok(Command::Run(run))
}

fn parse_selection_flag(
    flag: &str,
    inline: Option<OsString>,
    args: &mut VecDeque<OsString>,
    selection: &mut Selection,
) -> Result<(), ParseError> {
    match flag {
        "--project" => {
            let path = value(flag, inline, args)?;
            set_once(&mut selection.project, PathBuf::from(path), flag)
        }
        "--profile" => {
            let parsed = text_value(flag, inline, args)?;
            set_once(&mut selection.profile, parsed, flag)
        }
        "--agent" => {
            let parsed = text_value(flag, inline, args)?;
            set_once(&mut selection.agent, parsed, flag)
        }
        "--provider" => {
            let parsed = text_value(flag, inline, args)?;
            set_once(&mut selection.provider, parsed, flag)
        }
        "--model" => {
            let parsed = text_value(flag, inline, args)?;
            set_once(&mut selection.model, parsed, flag)
        }
        // No list is spelled here: the advertised effort ladder belongs to the
        // resolved provider/model binding, so an unsupported value is refused
        // by the reasoning resolver — which knows the alternatives — rather
        // than guessed at by the parser.
        "--effort" => {
            let parsed = text_value(flag, inline, args)?;
            set_once(&mut selection.effort, parsed, flag)
        }
        "--context-window" => {
            let parsed = text_value(flag, inline, args)?;
            set_once(&mut selection.context_window_flag, parsed, flag)
        }
        "--approval" => {
            let raw = text_value(flag, inline, args)?;
            let parsed = ApprovalMode::parse(&raw).ok_or_else(|| {
                ParseError::new(format!(
                    "invalid approval policy `{raw}`; expected `ask`, `deny`, or `allow-all`"
                ))
            })?;
            set_approval_once(selection, parsed)
        }
        "--yolo" => {
            if inline.is_some() {
                return Err(ParseError::new("`--yolo` does not take a value"));
            }
            set_approval_once(selection, ApprovalMode::AllowAll)
        }
        "--background-exit" => {
            let raw = text_value(flag, inline, args)?;
            let parsed = BackgroundExit::parse(&raw).ok_or_else(|| {
                ParseError::new(format!(
                    "invalid background-exit policy `{raw}`; expected `error`, `wait`, or `stop`"
                ))
            })?;
            set_once(&mut selection.background_exit, parsed, flag)
        }
        "--allow-synthetic-cache-spend" => {
            if inline.is_some() {
                return Err(ParseError::new(
                    "`--allow-synthetic-cache-spend` does not take a value",
                ));
            }
            if selection.allow_synthetic_cache_spend {
                return Err(ParseError::new(
                    "`--allow-synthetic-cache-spend` was supplied twice",
                ));
            }
            selection.allow_synthetic_cache_spend = true;
            Ok(())
        }
        _ if flag.starts_with('-') => Err(ParseError::new(format!("unknown option `{flag}`"))),
        _ => Err(ParseError::new(format!("unexpected argument `{flag}`"))),
    }
}

fn prompt_source(value: String) -> Prompt {
    if value == "-" {
        Prompt::Stdin
    } else {
        Prompt::Argument(value)
    }
}

fn flag(raw: OsString) -> Result<(String, Option<OsString>), ParseError> {
    let text = raw
        .into_string()
        .map_err(|_| ParseError::new("option names and prompts must be valid UTF-8"))?;
    if let Some((flag, value)) = text.split_once('=') {
        Ok((flag.to_owned(), Some(OsString::from(value))))
    } else {
        Ok((text, None))
    }
}

fn value(
    flag: &str,
    inline: Option<OsString>,
    args: &mut VecDeque<OsString>,
) -> Result<OsString, ParseError> {
    inline
        .or_else(|| args.pop_front())
        .ok_or_else(|| ParseError::new(format!("`{flag}` requires a value")))
}

fn text_value(
    flag: &str,
    inline: Option<OsString>,
    args: &mut VecDeque<OsString>,
) -> Result<String, ParseError> {
    required_text(
        Some(value(flag, inline, args)?),
        &format!("a UTF-8 value for `{flag}`"),
    )
}

fn optional_text_value(
    flag: &str,
    inline: Option<OsString>,
    args: &mut VecDeque<OsString>,
) -> Result<Option<String>, ParseError> {
    if let Some(inline) = inline {
        return required_text(
            Some(inline),
            &format!("a non-empty UTF-8 value for `{flag}`"),
        )
        .map(Some);
    }
    let has_value = args
        .front()
        .and_then(|value| value.to_str())
        .is_some_and(|value| !value.starts_with('-'));
    if has_value {
        return required_text(
            args.pop_front(),
            &format!("a non-empty UTF-8 value for `{flag}`"),
        )
        .map(Some);
    }
    Ok(None)
}

fn required_text(raw: Option<OsString>, what: &str) -> Result<String, ParseError> {
    let value = raw
        .ok_or_else(|| ParseError::new(format!("expected {what}")))?
        .into_string()
        .map_err(|_| ParseError::new(format!("expected {what}")))?;
    if value.is_empty() {
        Err(ParseError::new(format!("expected {what}")))
    } else {
        Ok(value)
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), ParseError> {
    if slot.is_some() {
        Err(ParseError::new(format!("`{flag}` was supplied twice")))
    } else {
        *slot = Some(value);
        Ok(())
    }
}

fn set_approval_once(selection: &mut Selection, approval: ApprovalMode) -> Result<(), ParseError> {
    if selection.approval.is_some() {
        Err(ParseError::new(
            "approval policy was supplied more than once (`--approval`/`--yolo`)",
        ))
    } else {
        selection.approval = Some(approval);
        Ok(())
    }
}

/// Complete command help kept as a stable golden contract.
pub(crate) const HELP: &str = "\
Smith — a terminal coding agent

USAGE:
  smith [OPTIONS]
  smith -p <PROMPT|-> [OPTIONS]
  smith help
  smith config explain <KEY> [SELECTION OPTIONS]
  smith config modules [SELECTION OPTIONS]
  smith sessions list [SELECTION OPTIONS]
  smith models sync [--provider <NAME>] [--force]
  smith plugin add <SOURCE> [--yes]
  smith plugin list
  smith plugin update [NAME] [--yes]
  smith plugin remove <NAME> [--delete-data]
  smith plugin marketplace add <SOURCE>
  smith plugin marketplace list
  smith plugin marketplace update [NAME]
  smith plugin marketplace remove <NAME>
  smith setup [--project <PATH>]
  smith setup add-provider [--project <PATH>]
  smith setup add-model [--provider <NAME>] [--project <PATH>]
  smith setup credential --provider <NAME> [--project <PATH>]
  smith setup checkpoint-key [--project <PATH>]

RUN OPTIONS:
  -p, --prompt <PROMPT|->       Run once; `-` reads the prompt from stdin
      --image <PATH>            Attach an image to `-p` (repeatable; paths relative to cwd)
      --project <PATH>          Start project discovery at PATH
      --resume [SESSION_ID]     Resume by ID, or choose from project sessions
      --session [SESSION_ID]    Alias for --resume
      --profile <NAME>          Select a main-enabled agent profile
      --agent <MODE>            Deprecated legacy root-mode override
      --provider <NAME>         Select a configured provider
      --model <MODEL>           Select a configured model
      --effort <NAME>           Select a provider-advertised reasoning effort
      --context-window <NAME>   Select a named model context window
      --approval <POLICY>       ask | deny | allow-all
      --yolo                    Alias for --approval allow-all
      --background-exit <MODE>  error | wait | stop
      --allow-synthetic-cache-spend
                                Permit bounded, conformance-gated cache maintenance
      --output-format <FORMAT>  text | json | stream-json
      --no-color                Disable terminal colors
      --no-motion               Disable terminal animation
  -h, --help                    Print help
  -V, --version                 Print version

SELECTION OPTIONS:
  config explain, config modules and sessions list accept --project, --profile, --agent,
  --provider, --model, --effort, --context-window, --approval, --yolo,
  --background-exit, and --allow-synthetic-cache-spend as described above.
  -h, --help                    Print help at either command level

PLUGIN OPTIONS:
      --yes                     Confirm the printed inventory and trust its digest
      --delete-data             Also delete the removed plugin’s data directory
  Sources: local path, github:owner/repo[#ref], git URL, or name@marketplace.
  Installation and updates require a terminal confirmation or --yes.
  -h, --help                    Print help at any plugin command level

SETUP OPTIONS:
      --project <PATH>          Project used for post-setup preflight
      --provider <NAME>         Provider for add-model or credential
      --no-color                Disable terminal colors
      --no-motion               Disable terminal animation
  -h, --help                    Print help for setup or any setup action

INTERACTIVE COMPOSER:
  Tab                           Cycle profile_order while empty and idle
  @FILE                         Prepare an exact file attachment read
  @PROFILE TASK                 Confirm a child-enabled read-only profile
  !COMMAND                      Run the canonical prepared local shell path
  @@ / !!                       Send a literal leading @ / !
  /connect /disconnect         Manage provider auth, custom endpoints, or ChatGPT OAuth
  /details /timeline /redo      Inspect work, history, or exact recovery

Headless runs fail closed at approval boundaries unless an explicit policy or
auto-approve rule authorizes the tool. Machine output is written only to stdout;
diagnostics are written to stderr.
";

#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod tests {
    use super::*;

    fn command(args: &[&str]) -> Command {
        parse(args.iter().map(OsString::from)).expect("parsed command")
    }

    #[test]
    fn no_arguments_selects_the_interactive_surface() {
        assert_eq!(command(&[]), Command::Run(RunArgs::default()));
    }

    #[test]
    fn mcp_watchdog_requires_parent_separator_and_program() {
        assert_eq!(
            command(&["mcp-watchdog", "--parent", "123", "--", "server", "--flag"]),
            Command::McpWatchdog {
                parent: 123,
                program: "server".into(),
                args: vec!["--flag".into()],
            }
        );
        for args in [
            vec!["mcp-watchdog"],
            vec!["mcp-watchdog", "--parent", "0", "--", "server"],
            vec!["mcp-watchdog", "--parent", "-1", "--", "server"],
            vec!["mcp-watchdog", "--parent", "2147483648", "--", "server"],
            vec!["mcp-watchdog", "--parent", "bad", "--", "server"],
            vec!["mcp-watchdog", "--parent", "123", "server"],
            vec!["mcp-watchdog", "--parent", "123", "--"],
            vec!["mcp-watchdog", "--parent", "123", "--", ""],
        ] {
            assert!(parse(args.into_iter().map(OsString::from)).is_err());
        }
        assert!(!HELP.contains("mcp-watchdog"));
    }

    #[test]
    fn mcp_relay_requires_exactly_one_absolute_socket_path() {
        assert_eq!(
            command(&["mcp-relay", "/tmp/turn socket"]),
            Command::McpRelay {
                socket: PathBuf::from("/tmp/turn socket"),
            }
        );
        for args in [
            vec!["mcp-relay"],
            vec!["mcp-relay", ""],
            vec!["mcp-relay", "relative.sock"],
            vec!["mcp-relay", "/tmp/turn.sock", "extra"],
            vec!["mcp-relay", "/tmp/turn.sock", "--help"],
        ] {
            let error = parse(args.into_iter().map(OsString::from)).expect_err("relay usage");
            assert_eq!(
                error.to_string(),
                "usage: smith mcp-relay <absolute-socket-path>"
            );
        }
        assert!(!HELP.contains("mcp-relay"));
        assert!(!HELP.contains("wasm-engine-diagnostic"));
    }

    #[cfg(unix)]
    #[test]
    fn mcp_relay_preserves_non_utf8_socket_paths() {
        use std::os::unix::ffi::OsStringExt;

        let socket = OsString::from_vec(b"/tmp/turn-\xff.sock".to_vec());
        assert_eq!(
            parse([OsString::from("mcp-relay"), socket.clone()]).expect("native path"),
            Command::McpRelay {
                socket: PathBuf::from(socket),
            }
        );
    }

    #[test]
    fn help_is_accepted_at_every_command_level() {
        for path in [
            &[][..],
            &["setup"][..],
            &["setup", "add-provider"][..],
            &["setup", "add-model"][..],
            &["setup", "credential"][..],
            &["setup", "checkpoint-key"][..],
            &["config"][..],
            &["config", "explain"][..],
            &["config", "explain", "model"][..],
            &["sessions"][..],
            &["sessions", "list"][..],
        ] {
            for help in ["help", "-h", "--help"] {
                let args = path.iter().copied().chain([help]).map(OsString::from);
                assert_eq!(parse(args).expect("help"), Command::Help, "{path:?} {help}");
            }
        }
        for args in [
            vec!["setup", "credential", "--provider", "local", "--help"],
            vec!["config", "explain", "model", "--provider", "local", "-h"],
            vec!["sessions", "list", "--project", "/repo", "--help"],
        ] {
            assert_eq!(command(&args), Command::Help, "{args:?}");
        }
    }

    #[test]
    fn help_spellings_remain_values_when_consumed_by_an_option() {
        for prompt in ["help", "-h", "--help"] {
            let Command::Run(run) = command(&["-p", prompt]) else {
                panic!("expected a literal prompt");
            };
            assert_eq!(run.prompt, Some(Prompt::Argument(prompt.to_owned())));
        }
        let Command::SessionsList { selection } =
            command(&["sessions", "list", "--project", "--help"])
        else {
            panic!("expected a literal project path");
        };
        assert_eq!(selection.project, Some(PathBuf::from("--help")));
    }

    #[test]
    fn prompt_stdin_and_machine_output_are_parsed() {
        let Command::Run(run) = command(&[
            "--project",
            "/repo",
            "-p",
            "-",
            "--output-format=stream-json",
            "--approval",
            "deny",
        ]) else {
            panic!("expected a run");
        };
        assert_eq!(run.prompt, Some(Prompt::Stdin));
        assert_eq!(run.output, OutputFormat::StreamJson);
        assert_eq!(run.selection.project, Some(PathBuf::from("/repo")));
        assert_eq!(run.selection.approval, Some(ApprovalMode::Deny));
    }

    #[test]
    fn background_exit_is_parsed_and_absent_when_not_supplied() {
        let Command::Run(run) = command(&[]) else {
            panic!("expected a run");
        };
        // Absence lets the resolved configuration supply the headless policy;
        // the configuration resolver owns the default.
        assert_eq!(run.selection.background_exit, None);

        for (flag, expected) in [
            ("error", BackgroundExit::Error),
            ("wait", BackgroundExit::Wait),
            ("stop", BackgroundExit::Stop),
        ] {
            let Command::Run(run) = command(&["--background-exit", flag]) else {
                panic!("expected a run");
            };
            assert_eq!(run.selection.background_exit, Some(expected));
        }

        let invalid = parse(["--background-exit", "orphan"].map(OsString::from))
            .expect_err("an unknown background-exit spelling");
        assert!(
            invalid
                .to_string()
                .contains("invalid background-exit policy")
        );

        let duplicate =
            parse(["--background-exit", "wait", "--background-exit", "stop"].map(OsString::from))
                .expect_err("background-exit was supplied twice");
        assert!(duplicate.to_string().contains("supplied twice"));
    }

    #[test]
    fn synthetic_cache_spend_requires_one_explicit_valueless_host_flag() {
        let Command::Run(run) = command(&["--allow-synthetic-cache-spend"]) else {
            panic!("expected a run");
        };
        assert!(run.selection.allow_synthetic_cache_spend);

        let valued = parse(["--allow-synthetic-cache-spend=true"].map(OsString::from))
            .expect_err("host authority does not accept a value");
        assert!(valued.to_string().contains("does not take a value"));

        let duplicate = parse(
            [
                "--allow-synthetic-cache-spend",
                "--allow-synthetic-cache-spend",
            ]
            .map(OsString::from),
        )
        .expect_err("host authority is granted once");
        assert!(duplicate.to_string().contains("supplied twice"));
    }

    #[test]
    fn yolo_is_a_valueless_allow_all_alias_and_conflicts_fail_closed() {
        let Command::Run(run) = command(&["--yolo"]) else {
            panic!("expected a run");
        };
        assert_eq!(run.selection.approval, Some(ApprovalMode::AllowAll));

        let valued =
            parse(["--yolo=true"].map(OsString::from)).expect_err("yolo does not accept a value");
        assert!(valued.to_string().contains("does not take a value"));

        for args in [
            vec!["--yolo", "--yolo"],
            vec!["--yolo", "--approval", "deny"],
            vec!["--approval", "ask", "--yolo"],
        ] {
            let duplicate =
                parse(args.into_iter().map(OsString::from)).expect_err("approval conflict");
            assert!(
                duplicate
                    .to_string()
                    .contains("approval policy was supplied more than once"),
                "{duplicate}"
            );
        }
    }

    #[test]
    fn provider_model_and_resume_are_stable_run_flags() {
        let Command::Run(run) = command(&[
            "-p",
            "review",
            "--provider",
            "acme",
            "--model",
            "gpt-x",
            "--resume",
            "session-1",
        ]) else {
            panic!("expected a run");
        };
        assert_eq!(run.resume.as_deref(), Some("session-1"));
        assert!(run.resume_requested);
        assert_eq!(run.selection.provider.as_deref(), Some("acme"));
        assert_eq!(run.selection.model.as_deref(), Some("gpt-x"));
    }

    #[test]
    fn effort_is_an_invocation_selection_not_a_session_override() {
        for args in [
            vec!["--effort", "high"],
            vec!["--effort=high"],
            vec!["-p", "review", "--effort", "high"],
        ] {
            let Command::Run(run) = command(&args) else {
                panic!("expected a run");
            };
            assert_eq!(run.selection.effort.as_deref(), Some("high"));
            // The session field belongs to `/effort`; a flag must not be able
            // to masquerade as an in-session choice.
            assert_eq!(run.selection.reasoning_effort, None);
            assert!(!run.selection.reasoning_effort_reset);
            assert_eq!(
                run.selection.overrides().reasoning_effort.as_deref(),
                Some("high")
            );
            assert_eq!(run.selection.session_overrides().reasoning_effort, None);
        }

        let Command::Run(absent) = command(&[]) else {
            panic!("expected a run");
        };
        assert_eq!(absent.selection.effort, None);
        assert_eq!(absent.selection.overrides().reasoning_effort, None);

        let duplicate = parse(["--effort", "low", "--effort", "high"].map(OsString::from))
            .expect_err("effort is one selection");
        assert!(
            duplicate.to_string().contains("supplied twice"),
            "{duplicate}"
        );

        let missing = parse(["--effort"].map(OsString::from)).expect_err("effort requires a value");
        assert!(
            missing.to_string().contains("requires a value"),
            "{missing}"
        );
    }

    #[test]
    fn context_window_flag_is_an_invocation_selection_not_a_session_override() {
        for args in [
            vec!["--context-window", "872k"],
            vec!["--context-window=872k"],
            vec!["-p", "review", "--context-window", "872k"],
        ] {
            let Command::Run(run) = command(&args) else {
                panic!("expected a run");
            };
            assert_eq!(run.selection.context_window_flag.as_deref(), Some("872k"));
            assert_eq!(run.selection.context_window, None);
            assert_eq!(
                run.selection.overrides().context_window.as_deref(),
                Some("872k")
            );
            assert_eq!(run.selection.session_overrides().context_window, None);
        }

        let duplicate =
            parse(["--context-window", "272k", "--context-window", "872k"].map(OsString::from))
                .expect_err("context window is one selection");
        assert!(
            duplicate.to_string().contains("supplied twice"),
            "{duplicate}"
        );

        let missing = parse(["--context-window"].map(OsString::from))
            .expect_err("context window requires a value");
        assert!(
            missing.to_string().contains("requires a value"),
            "{missing}"
        );
    }

    #[test]
    fn effort_reaches_the_selection_only_commands() {
        assert_eq!(
            command(&["config", "explain", "reasoning.effort", "--effort", "low"]),
            Command::ConfigExplain {
                key: "reasoning.effort".into(),
                selection: Selection {
                    effort: Some("low".into()),
                    ..Selection::default()
                },
            }
        );
        assert_eq!(
            command(&["sessions", "list", "--effort", "low"]),
            Command::SessionsList {
                selection: Selection {
                    effort: Some("low".into()),
                    ..Selection::default()
                },
            }
        );
    }

    #[test]
    fn effort_composes_with_a_profile_without_replacing_it() {
        let Command::Run(run) = command(&["--profile", "work", "--effort", "low"]) else {
            panic!("expected a run");
        };
        assert_eq!(run.selection.profile.as_deref(), Some("work"));
        let overrides = run.selection.overrides();
        assert_eq!(overrides.profile.as_deref(), Some("work"));
        assert_eq!(overrides.reasoning_effort.as_deref(), Some("low"));
        // Everything else the profile pins stays the profile's to decide.
        assert_eq!(overrides.provider, None);
        assert_eq!(overrides.model, None);
        assert_eq!(overrides.approval_mode, None);
    }

    #[test]
    fn bare_resume_requests_the_interactive_picker_without_eating_options() {
        let Command::Run(run) = command(&["--resume", "--no-color"]) else {
            panic!("expected a run");
        };
        assert!(run.resume_requested);
        assert!(run.resume.is_none());
        assert!(run.no_color);

        let duplicate = parse(["--resume", "--session"].map(OsString::from))
            .expect_err("resume aliases are one option");
        assert!(duplicate.to_string().contains("supplied twice"));
    }

    #[test]
    fn accessibility_flags_are_explicit_booleans() {
        let Command::Run(run) = command(&["--no-color", "--no-motion"]) else {
            panic!("expected a run");
        };
        assert!(run.no_color);
        assert!(run.no_motion);
    }

    #[test]
    fn config_explain_keeps_cli_selection_provenance() {
        assert_eq!(
            command(&[
                "config",
                "explain",
                "model",
                "--model",
                "gpt-x",
                "--project",
                "/repo",
            ]),
            Command::ConfigExplain {
                key: "model".into(),
                selection: Selection {
                    project: Some("/repo".into()),
                    model: Some("gpt-x".into()),
                    ..Selection::default()
                },
            }
        );
    }

    #[test]
    fn models_sync_is_typed_and_rejects_ambiguous_flags() {
        assert_eq!(
            command(&["models", "sync"]),
            Command::ModelsSync {
                provider: None,
                force: false
            }
        );
        assert_eq!(
            command(&["models", "sync", "--provider=xiaoge", "--force"]),
            Command::ModelsSync {
                provider: Some("xiaoge".into()),
                force: true
            }
        );
        for args in [
            vec!["models"],
            vec!["models", "list"],
            vec!["models", "sync", "--force=true"],
            vec!["models", "sync", "--force", "--force"],
            vec!["models", "sync", "--provider"],
            vec!["models", "sync", "--project", "/tmp"],
        ] {
            assert!(parse(args.into_iter().map(OsString::from)).is_err());
        }
    }

    #[test]
    fn reusable_setup_commands_are_typed() {
        assert_eq!(
            command(&["setup"]),
            Command::Setup(SetupArgs {
                action: SetupAction::Menu,
                project: None,
                no_color: false,
                no_motion: false,
            })
        );
        assert_eq!(
            command(&[
                "setup",
                "add-model",
                "--provider",
                "zai",
                "--project",
                "/repo",
                "--no-color",
            ]),
            Command::Setup(SetupArgs {
                action: SetupAction::AddModel {
                    provider: Some("zai".into())
                },
                project: Some("/repo".into()),
                no_color: true,
                no_motion: false,
            })
        );
        assert!(matches!(
            command(&["setup", "add-provider"]),
            Command::Setup(SetupArgs {
                action: SetupAction::AddProvider,
                ..
            })
        ));
        assert!(matches!(
            command(&["setup", "credential", "--provider", "zai"]),
            Command::Setup(SetupArgs {
                action: SetupAction::Credential { provider },
                ..
            }) if provider == "zai"
        ));
        let missing = parse(["setup", "credential"].map(OsString::from))
            .expect_err("credential migration requires a provider");
        assert!(missing.to_string().contains("--provider"), "{missing}");
    }

    #[test]
    fn headless_images_are_repeatable_and_require_a_prompt() {
        let Command::Run(run) =
            command(&["-p", "describe", "--image", "one.png", "--image=two.jpg"])
        else {
            panic!("expected a headless run");
        };
        assert_eq!(
            run.images,
            vec![PathBuf::from("one.png"), PathBuf::from("two.jpg")]
        );
        let error = parse(["--image", "one.png"].map(OsString::from)).unwrap_err();
        assert!(error.to_string().contains("requires non-interactive `-p"));
        assert!(parse(["-p", "describe", "--image"].map(OsString::from)).is_err());
    }

    #[test]
    fn interactive_machine_output_is_rejected() {
        let error = parse(["--output-format", "json"].map(OsString::from))
            .expect_err("machine output needs a prompt");
        assert!(error.to_string().contains("requires non-interactive"));
    }

    #[test]
    fn positional_prompts_are_rejected_with_the_argument_named() {
        let error = parse(["hello"].map(OsString::from)).expect_err("a positional prompt");
        assert_eq!(error.to_string(), "unexpected argument `hello`");
    }
}
