//! Stable command-line parsing without coupling Smith's product types to a
//! parser framework.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::PathBuf;

use smith_config::model::{ApprovalMode, BackgroundExit};
use smith_config::resolve::Overrides;

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
    /// List persisted sessions for a project.
    SessionsList {
        /// The project whose session partition to inspect.
        selection: Selection,
    },
    /// Enter guided user-scoped provider/model setup.
    Setup(SetupArgs),
    /// Print command help.
    Help,
    /// Print the binary version.
    Version,
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
    /// An approval policy override.
    pub approval: Option<ApprovalMode>,
    /// A background-exit policy override.
    pub background_exit: Option<BackgroundExit>,
}

impl Selection {
    /// Converts parsed flags into the config resolver's typed CLI layer.
    pub(crate) fn overrides(&self) -> Overrides {
        Overrides {
            agent: self.agent.clone(),
            profile: self.profile.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
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
        Some("config") => parse_config(args),
        Some("sessions") => parse_sessions(args),
        Some("setup") => parse_setup(args),
        _ => parse_run(args),
    }
}

fn parse_setup(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    args.pop_front();
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
                return Err(ParseError::new(format!(
                    "unknown setup option `{flag}`; run `smith --help`"
                )));
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
    let action = required_text(args.pop_front(), "an action after `config`")?;
    if action != "explain" {
        return Err(ParseError::new(format!(
            "unknown config action `{action}`; expected `smith config explain <key>`"
        )));
    }
    let key = required_text(args.pop_front(), "a dotted key after `config explain`")?;
    let selection = parse_selection_only(args)?;
    Ok(Command::ConfigExplain { key, selection })
}

fn parse_sessions(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    args.pop_front();
    let action = required_text(args.pop_front(), "an action after `sessions`")?;
    if action != "list" {
        return Err(ParseError::new(format!(
            "unknown sessions action `{action}`; expected `smith sessions list`"
        )));
    }
    Ok(Command::SessionsList {
        selection: parse_selection_only(args)?,
    })
}

fn parse_selection_only(mut args: VecDeque<OsString>) -> Result<Selection, ParseError> {
    let mut selection = Selection::default();
    while let Some(raw) = args.pop_front() {
        let (flag, inline) = flag(raw)?;
        if flag == "--help" || flag == "-h" {
            return Err(ParseError::new(
                "`--help` applies to `smith`; run `smith --help`",
            ));
        }
        parse_selection_flag(&flag, inline, &mut args, &mut selection)?;
    }
    Ok(selection)
}

fn parse_run(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    let mut run = RunArgs::default();
    let mut output_set = false;
    while let Some(raw) = args.pop_front() {
        let (flag, inline) = flag(raw)?;
        match flag.as_str() {
            "--help" | "-h" => return Ok(Command::Help),
            "--version" | "-V" => return Ok(Command::Version),
            "-p" | "--prompt" => {
                let value = value(&flag, inline, &mut args)?;
                let prompt = required_text(Some(value), "a UTF-8 prompt after `-p`")?;
                set_once(&mut run.prompt, prompt_source(prompt), "--prompt")?;
            }
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
        _ if flag.starts_with('-') => Err(ParseError::new(format!(
            "unknown option `{flag}`; run `smith --help`"
        ))),
        _ => Err(ParseError::new(format!(
            "unexpected argument `{flag}`; pass a prompt as `smith -p <prompt>`"
        ))),
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
  smith config explain <KEY> [SELECTION OPTIONS]
  smith sessions list [SELECTION OPTIONS]
  smith setup [--project <PATH>]
  smith setup add-provider [--project <PATH>]
  smith setup add-model [--provider <NAME>] [--project <PATH>]
  smith setup credential --provider <NAME> [--project <PATH>]
  smith setup checkpoint-key [--project <PATH>]

RUN OPTIONS:
  -p, --prompt <PROMPT|->       Run once; `-` reads the prompt from stdin
      --project <PATH>          Start project discovery at PATH
      --resume [SESSION_ID]     Resume by ID, or choose from project sessions
      --session [SESSION_ID]    Alias for --resume
      --profile <NAME>          Select a main-enabled agent profile
      --agent <MODE>            Deprecated legacy root-mode override
      --provider <NAME>         Select a configured provider
      --model <MODEL>           Select a configured model
      --approval <POLICY>       ask | deny | allow-all
      --yolo                    Alias for --approval allow-all
      --background-exit <MODE>  error | wait | stop
      --output-format <FORMAT>  text | json | stream-json
      --no-color                Disable terminal colors
      --no-motion               Disable terminal animation
  -h, --help                    Print help
  -V, --version                 Print version

INTERACTIVE COMPOSER:
  Tab                           Cycle profile_order while empty and idle
  @FILE                         Prepare an exact file attachment read
  @PROFILE TASK                 Confirm a child-enabled read-only profile
  !COMMAND                      Run the canonical prepared local shell path
  @@ / !!                       Send a literal leading @ / !
  /connect /disconnect         Manage provider auth, Google Gemini, or ChatGPT OAuth
  /details /timeline /redo      Inspect work, history, or exact recovery

Headless runs fail closed at approval boundaries unless an explicit policy or
auto-approve rule authorizes the tool. Machine output is written only to stdout;
diagnostics are written to stderr.
";

#[cfg(test)]
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
        // Absence is meaningful: the headless runner applies the
        // `BackgroundExit` default itself rather than this layer guessing it.
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
        assert!(invalid.to_string().contains("invalid background-exit policy"));

        let duplicate = parse(
            ["--background-exit", "wait", "--background-exit", "stop"].map(OsString::from),
        )
        .expect_err("background-exit was supplied twice");
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
    fn interactive_machine_output_is_rejected() {
        let error = parse(["--output-format", "json"].map(OsString::from))
            .expect_err("machine output needs a prompt");
        assert!(error.to_string().contains("requires non-interactive"));
    }

    #[test]
    fn positional_prompts_are_rejected_with_the_supported_form() {
        let error = parse(["hello"].map(OsString::from)).expect_err("a positional prompt");
        assert!(error.to_string().contains("smith -p <prompt>"));
    }
}
