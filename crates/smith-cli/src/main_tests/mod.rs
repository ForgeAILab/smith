use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::content::UserInput;
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::goal::GoalCommand;
use agent_runtime_core::ids::{ChildId, SessionId};
use agent_runtime_core::provider::ModelId;
use agent_runtime_core::usage::CounterKind;

use anyhow::Result;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use smith_client::commands::{AgentAction, DiffScope, HostCommand, SelectionCommand};
use smith_client::status::{ContextPlanUpdate, Status};

use smith_config::inventory::local_inventory_with_catalog;
use smith_config::model::ApprovalMode;
use smith_config::resolve::{ResolveRequest, resolve};

use smith_host::{ApprovalPrompt, ApprovalRequests, InteractiveApproval, ProjectWorkspace};

use smith_runtime::checkpoint::{CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError};
use smith_runtime::client::{
    EstimationConfidence, SmithEvent as EventEnvelope, SmithEventKind as RuntimeEvent,
};
use smith_runtime::factory::{AVAILABLE_ADAPTER_KINDS, FactoryError, HostSurface, RuntimeRequest};
use smith_runtime::host::{HostSession, HostSessionRequest};
use smith_runtime::session::{SNAPSHOT_SCHEMA_VERSION, SessionListing};

use smith_tui::app::{Action, App, LEGACY_AGENT_PROFILE_PREFIX};
use smith_tui::{Block, LocalResult, LocalResultState, ResourceEntry, RuntimeResources};

use crate::cli::Selection;
use crate::local_command::*;
use crate::resources::*;
use crate::runtime_host::*;
use crate::submission::*;
use crate::{MAX_STDIN_PROMPT_BYTES, local_command};

#[derive(Debug)]
struct TestCheckpointKeys;

impl CheckpointKeyProvider for TestCheckpointKeys {
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
        Ok(CheckpointKey::new([0x52; 32]))
    }
}

const LOCAL_COMMAND_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;

#[test]
fn setup_outcome_prints_cancel_once_and_completion_is_silent() {
    let mut output = Vec::new();
    crate::print_setup_outcome(crate::setup::SetupOutcome::Completed, &mut output).expect("output");
    assert!(output.is_empty());
    crate::print_setup_outcome(crate::setup::SetupOutcome::Cancelled, &mut output).expect("output");
    crate::print_setup_outcome(crate::setup::SetupOutcome::Completed, &mut output).expect("output");
    assert_eq!(
        String::from_utf8(output).expect("text"),
        "Setup cancelled · nothing was written\n"
    );
}

fn git(project: &std::path::Path, arguments: &[&str]) {
    let output = std::process::Command::new("git")
        .args(arguments)
        .current_dir(project)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod cross_provider;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod file_commands;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod fixtures;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod host_routing;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod interactive_rebind;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod local_commands;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod local_shell;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod mcp;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod rendering;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod resources;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod skills;
#[cfg(test)]
mod standalone_screens;
#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod submission;

pub(crate) use fixtures::fixture_support;
