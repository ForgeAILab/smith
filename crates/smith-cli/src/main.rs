//! The `smith` composition root.
//!
//! Both terminal and one-prompt runs resolve the same configuration, inject
//! the same project/credential policy, and start through
//! [`smith_runtime::host`]. Presentation begins only after that preflight and
//! the session restore have succeeded.

#![warn(clippy::wildcard_imports)]

mod browser;
mod chatgpt;
mod cli;
mod config_command;
mod connection;
mod connection_review;
mod headless;
mod interaction;
mod local_command;
mod logging;
mod login_progress;
mod mcp;
mod resources;
mod runtime_host;
mod screen_runner;
mod setup;
mod skills;
mod submission;
mod terminal;
mod tui_driver;
mod xai;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use cli::{Command, Prompt, RunArgs};
use smith_config::resolve::ConfigReadiness;
use smith_runtime::factory::HostSurface;

use config_command::{explain_config, inspect_selection};
use resources::{choose_resume_session, list_sessions};
use runtime_host::{read_prompt, run_interactive_command, start_host};

/// The frame budget: `DESIGN.md` §6 caps redraws at 30 fps.
const FRAME: Duration = Duration::from_millis(33);

/// The spinner advances every 100 ms, independently of the frame rate.
const SPINNER_TICK: Duration = Duration::from_millis(100);

/// A piped prompt is bounded before it can consume process memory. The runtime
/// applies the model-specific token budget later.
const MAX_STDIN_PROMPT_BYTES: usize = 1024 * 1024;

#[tokio::main]
async fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("smith: {error}");
            eprintln!("Try `smith --help` for usage.");
            return ExitCode::from(2);
        }
    };

    match execute(command).await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("smith: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn execute(command: Command) -> Result<u8> {
    match command {
        Command::Help => {
            print!("{}", cli::HELP);
            Ok(0)
        }
        Command::Version => {
            println!("smith {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Command::ConfigExplain { key, selection } => {
            explain_config(&key, &selection)?;
            Ok(0)
        }
        Command::SessionsList { selection } => {
            list_sessions(&selection).await?;
            Ok(0)
        }
        Command::Setup(args) => {
            let outcome = setup::run_explicit(args).await?;
            print_setup_outcome(outcome, &mut std::io::stdout().lock())?;
            Ok(0)
        }
        Command::Run(args) => run_command(args).await,
    }
}

async fn run_command(mut args: RunArgs) -> Result<u8> {
    let configured_background_exit = match inspect_selection(&args.selection)? {
        ConfigReadiness::Ready(resolution) => Some(resolution.config.background.exit_policy.value),
        ConfigReadiness::Invalid(error) => {
            return Err(anyhow::anyhow!("{error}")).context("resolving Smith configuration");
        }
        ConfigReadiness::Unconfigured(_) => {
            let interactive = args.prompt.is_none() && is_interactive_terminal();
            if !interactive {
                anyhow::bail!(
                    "Smith has no configured provider/model. Run `smith setup` in an interactive \
                     terminal, or supply a complete provider, model, and limits through config"
                );
            }
            match setup::run_first_run(args.selection.clone(), args.no_color, args.no_motion)
                .await?
            {
                setup::SetupOutcome::Cancelled => {
                    print_setup_outcome(
                        setup::SetupOutcome::Cancelled,
                        &mut std::io::stdout().lock(),
                    )?;
                    return Ok(0);
                }
                setup::SetupOutcome::Completed => {}
            }
            None
        }
    };

    if args.resume_requested && args.resume.is_none() {
        if args.prompt.is_some() {
            anyhow::bail!(
                "bare `--resume` needs an interactive terminal; use `smith sessions list` and \
                 pass `--resume <SESSION_ID>` for a headless run"
            );
        }
        if !is_interactive_terminal() {
            anyhow::bail!(
                "bare `--resume` needs an interactive terminal; use `smith sessions list` or \
                 pass `--resume <SESSION_ID>`"
            );
        }
        args.resume =
            match choose_resume_session(&args.selection, args.no_color, args.no_motion).await? {
                Some(session) => Some(session),
                None => return Ok(0),
            };
    }

    if args.prompt.is_none() && !is_interactive_terminal() {
        anyhow::bail!(
            "the interactive surface needs a terminal on stdin, stdout, and stderr; \
             use `smith -p -` to read the prompt from standard input, or `smith -p <PROMPT>`"
        );
    }

    let prompt = match args.prompt.take() {
        Some(Prompt::Argument(prompt)) => Some(prompt),
        Some(Prompt::Stdin) => Some(read_prompt(std::io::stdin().lock())?),
        None => None,
    };

    match prompt {
        Some(prompt) => {
            let started = start_host(
                &args.selection,
                args.resume.as_deref(),
                HostSurface::Headless,
                None,
                None,
            )
            .await?;
            logging::init(started.host.session().id()).await;
            let cache_price =
                tui_driver::resolve_price(started.host.runtime().policy(), &started.catalog)
                    .map(|price| smith_client::cache::CachePrice::from(&price.table));
            headless::run(
                &started.host,
                prompt,
                args.output,
                headless::HeadlessBrokers {
                    approval: started.headless_approval.as_deref(),
                    interaction: started.headless_interaction.as_deref(),
                    rotation: started.headless_rotation.as_deref(),
                    credential_pool: started.credential_pool.as_ref(),
                    cache_price,
                    cache_miss_notices: started.cache_miss_notices,
                },
                headless::background_exit_policy(
                    args.selection.background_exit,
                    configured_background_exit,
                ),
            )
            .await
            .map(|outcome| outcome.exit_code)
        }
        None => run_interactive_command(args).await,
    }
}

/// Reports cancellation only after standalone callers have restored the terminal.
fn print_setup_outcome(
    outcome: setup::SetupOutcome,
    output: &mut impl std::io::Write,
) -> std::io::Result<()> {
    if outcome == setup::SetupOutcome::Cancelled {
        writeln!(output, "Setup cancelled · nothing was written")?;
    }
    Ok(())
}

fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
}

#[cfg(test)]
#[allow(clippy::wildcard_imports)]
#[path = "main_tests/mod.rs"]
mod tests;
