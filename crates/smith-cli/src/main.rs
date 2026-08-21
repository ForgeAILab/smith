//! The `smith` composition root.
//!
//! Both terminal and one-prompt runs resolve the same configuration, inject
//! the same project/credential policy, and start through
//! [`smith_runtime::host`]. Presentation begins only after that preflight and
//! the session restore have succeeded.

mod browser;
mod chatgpt;
mod cli;
mod config_command;
mod connection;
mod headless;
mod interaction;
mod local_command;
mod logging;
mod mcp;
mod resources;
mod runtime_host;
mod setup;
mod skills;
mod submission;
mod terminal;
mod tui_driver;
mod xai;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::content::{ContentPart, ToolResultBlock, UserInput};
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::goal::{GoalCommand, GoalProjection};
use agent_runtime_core::ids::{ChildId, SessionId};
use agent_runtime_core::provider::{ModelId, ReasoningSupport};
use agent_runtime_core::steer::SteerRejectionReason;
use agent_runtime_core::usage::CounterKind;
use agent_runtime_core::workspace::Workspace;
use anyhow::{Context, Result};
use cli::{Command, Prompt, RunArgs, Selection};
use crossterm::event::{Event as TermEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ignore::WalkBuilder;
use ratatui::layout::Rect;
use smith_config::credential::CredentialResolver;
use smith_config::inventory::{
    InventoryLimit, ModelLimitOrigin, SelectionInventory, local_inventory_with_catalog,
};
use smith_config::model::{ApprovalMode, ProfileUse};
use smith_config::resolve::{
    ConfigReadiness, Layer, Resolution, ResolveRequest, ResolvedAgent,
    SyntheticCacheSpendAuthority, inspect, resolve,
};
use smith_host::{
    ApprovalPrompt, ApprovalRequests, GitChanges, HeadlessApproval, HeadlessInteraction,
    HeadlessRotation, InteractionRequests, InteractiveApproval, InteractiveInteraction,
    InteractiveRotation, ProjectWorkspace, RotationPrompt, RotationRequests,
};
use smith_runtime::client::{
    ChildPhase, EstimationConfidence, SmithEvent as EventEnvelope, SmithEventKind as RuntimeEvent,
    TurnFinish,
};
use smith_runtime::factory::{
    AVAILABLE_ADAPTER_KINDS, ChildProfileRequest, FactoryError, HostSurface, RuntimePolicy,
    RuntimeRequest,
};
use smith_runtime::host::{HostSession, HostSessionRequest};
use smith_runtime::journal::DefaultRedactor;
use smith_runtime::model_catalog::{CatalogLoader, runtime_catalog_source};
use smith_runtime::pool::CredentialPool;
use smith_runtime::pool_state::ActiveAccounts;
use smith_runtime::rotation::SharedPool;
use smith_runtime::session::{SNAPSHOT_SCHEMA_VERSION, SessionListing};
use smith_runtime::{ChildDurability, ChildState, ChildStatus, SpawnOutcome};
use smith_tui::app::{
    Action, App, LEGACY_AGENT_PROFILE_PREFIX, MouseOutcome, PaletteCommand, PreparedSubmission,
    SubmissionTarget,
};
use smith_tui::commands::{CommandAction, GoalAction};
#[cfg(test)]
use smith_tui::status::ContextPlanUpdate;
use smith_tui::status::{Status, TokenCount, render_elapsed};
use smith_tui::theme::{Theme, glyph};
use smith_tui::{
    PickerOutcome, ResourceEntry, ResourcePicker, RuntimeResources, draw_resource_picker,
};

use config_command::*;
use local_command::*;
use resources::*;
use runtime_host::*;
use submission::*;
use tui_driver::*;

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
            setup::run_explicit(args).await?;
            Ok(0)
        }
        Command::Run(args) => run_command(args).await,
    }
}

async fn run_command(mut args: RunArgs) -> Result<u8> {
    match inspect_selection(&args.selection)? {
        ConfigReadiness::Ready(_) => {}
        ConfigReadiness::Invalid(error) => {
            return Err(anyhow::anyhow!("{error}")).context("resolving Smith configuration");
        }
        ConfigReadiness::Unconfigured(_) => {
            let interactive = args.prompt.is_none()
                && std::io::stdin().is_terminal()
                && std::io::stdout().is_terminal()
                && std::io::stderr().is_terminal();
            if !interactive {
                anyhow::bail!(
                    "Smith has no configured provider/model. Run `smith setup` in an interactive \
                     terminal, or supply a complete provider, model, and limits through config"
                );
            }
            match setup::run_first_run(args.selection.clone(), args.no_color, args.no_motion)
                .await?
            {
                setup::SetupOutcome::Cancelled => return Ok(0),
                setup::SetupOutcome::Completed => {}
            }
        }
    }

    if args.resume_requested && args.resume.is_none() {
        if args.prompt.is_some() {
            anyhow::bail!(
                "bare `--resume` needs an interactive terminal; use `smith sessions list` and \
                 pass `--resume <SESSION_ID>` for a headless run"
            );
        }
        if !std::io::stdin().is_terminal()
            || !std::io::stdout().is_terminal()
            || !std::io::stderr().is_terminal()
        {
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
                tui_driver::resolve_price(started.host.runtime().policy(), &started.catalog).map(
                    |price| smith_tui::cache::CachePrice {
                        input: price.table.input,
                        cache_read: price.table.cache_read,
                        cache_write: price.table.cache_write,
                    },
                );
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
                args.selection.background_exit.unwrap_or_default(),
            )
            .await
            .map(|outcome| outcome.exit_code)
        }
        None => run_interactive_command(args).await,
    }
}

include!("main_tests/mod.rs");
