//! Smith's direct-child delegation wiring (harness tasks 7.1–7.3).
//!
//! The shared runtime owns the delegation mechanism — lifecycle, depth-one
//! authorization, scoped child views, budgets, and attributed child events.
//! Smith owns the product policy on top of it, all of which lives here:
//!
//! - [`DelegationAuthority`]: the authoritative security check covering the
//!   `agent.delegate` permission. It answers `RequireApproval`, so delegation
//!   flows through the same approval surface as Smith's mutating tools —
//!   interactive modal in the TUI, configured policy headless.
//! - [`SmithChildFactory`]: builds child runtimes through the same policy the
//!   parent was built with — same provider, model profile, context policy,
//!   loop limits, approval surface, and clock — so a child cannot drift from
//!   the one composition path.
//! - [`AgentTool`]: the model-facing `agent` tool (spawn / list / wait /
//!   result / follow_up / stop). The tool name is product policy; the neutral
//!   runtime never registers it, and the coordinator strips it from every
//!   child view so a child can never manage children.
//! - [`wire_delegation`]: installs the coordinator once the parent session
//!   exists and routes child results into the parent's safe-boundary inbox
//!   ([`SessionHandle::inject`]) so they reach the model only at a
//!   provider/tool boundary.

use std::sync::{Arc, OnceLock};

use agent_runtime::agent::config::LoopConfig;
use agent_runtime::context::ContextPolicy;
use agent_runtime::delegation::{
    ChildRuntimeFactory, ChildState, ChildStatus, DELEGATION_PERMISSION, DelegationConfig,
    DelegationCoordinator, DelegationLimits, SpawnOutcome,
};
use agent_runtime::registry::{Permission, RegistryRevision};
use agent_runtime::runtime::{InjectedContent, RuntimeBuilder, SessionHandle};
use agent_runtime_core::approval::ApprovalPolicy;
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::catalog::ResolvedModelProfile;
use agent_runtime_core::clock::Clock;
use agent_runtime_core::content::UserInput;
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::event::RuntimeEvent;
use agent_runtime_core::grant::{
    GrantConstraints, SecurityCheck, SecurityCheckId, SecurityCheckOutcome, SecurityCheckRevision,
};
use agent_runtime_core::provider::{ModelId, Provider};
use agent_runtime_core::security::{AuthorizationRequest, PermissionSet};
use agent_runtime_core::tool::{InvocationContext, Tool, ToolEffects, ToolOutcome};
use agent_runtime_core::workspace::Workspace;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use smith_host::ProjectWorkspace;

use crate::factory::CACHE_CAPABILITY_REVISION;

/// The model-facing delegation tool's name — Smith product policy.
pub const AGENT_TOOL_NAME: &str = "agent";

/// The default cap on tasks (spawn plus follow-ups) per child.
pub const DEFAULT_CHILD_MAX_TURNS: u32 = 4;

/// The default cap on concurrently alive children per root session.
pub const DEFAULT_MAX_RUNNING_CHILDREN: usize = 4;

/// Smith's authoritative coverage for the shared runtime's
/// [`DELEGATION_PERMISSION`].
///
/// Mirrors [`LegacyApprovalAuthority`](agent_runtime_core::compat::LegacyApprovalAuthority):
/// it expresses no policy of its own beyond routing delegation operations
/// through the approval surface Smith already exposes for write, process, and
/// network effects.
#[derive(Debug)]
pub struct DelegationAuthority {
    id: SecurityCheckId,
    revision: SecurityCheckRevision,
    coverage: PermissionSet,
}

impl DelegationAuthority {
    /// The authority with its fixed coverage.
    pub fn new() -> Self {
        Self {
            id: SecurityCheckId::new("smith-delegation-authority"),
            revision: SecurityCheckRevision::new("v1"),
            coverage: PermissionSet::single(Permission::other(DELEGATION_PERMISSION.to_owned())),
        }
    }

    /// The fixed coverage, for the registration call site.
    pub fn coverage(&self) -> &PermissionSet {
        &self.coverage
    }
}

impl Default for DelegationAuthority {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecurityCheck for DelegationAuthority {
    fn id(&self) -> &SecurityCheckId {
        &self.id
    }

    fn revision(&self) -> &SecurityCheckRevision {
        &self.revision
    }

    fn declared_coverage(&self) -> Option<PermissionSet> {
        Some(self.coverage.clone())
    }

    async fn evaluate(
        &self,
        request: &AuthorizationRequest,
        _cancel: &Cancellation,
    ) -> SecurityCheckOutcome {
        let applies = request
            .requested
            .iter()
            .any(|permission| self.coverage.contains(permission));
        if applies {
            SecurityCheckOutcome::RequireApproval {
                constraints: GrantConstraints::unconstrained(),
            }
        } else {
            SecurityCheckOutcome::NotApplicable
        }
    }
}

/// Builds child runtimes through the parent's own resolved policy.
///
/// Captured at parent composition time by the factory, so a child is always a
/// narrowing of the run the user configured: same provider instance, same
/// model profile, same context policy and loop limits, same approval surface
/// and clock. The coordinator applies the spec's tool-view scope and strips
/// the [`AGENT_TOOL_NAME`] tool after this returns.
#[derive(Debug)]
pub struct SmithChildFactory {
    pub(crate) provider: Arc<dyn Provider>,
    pub(crate) provider_name: String,
    pub(crate) provider_kind: String,
    pub(crate) model: ModelId,
    pub(crate) profile: ResolvedModelProfile,
    pub(crate) context_policy: ContextPolicy,
    pub(crate) loop_config: LoopConfig,
    pub(crate) approval: Arc<dyn ApprovalPolicy>,
    pub(crate) workspace: Arc<dyn Workspace>,
    pub(crate) clock: Arc<dyn Clock>,
}

impl ChildRuntimeFactory for SmithChildFactory {
    fn child_builder(&self, spec: &ChildSpec) -> Result<RuntimeBuilder, RuntimeError> {
        // The parent chose this run's provider and model through the one
        // factory path. A child may inherit them or restate them; routing a
        // child to a *different* provider/model needs its own credential and
        // profile resolution and is a coordinated follow-up, so it is refused
        // rather than half-built.
        if let ChildModelSelection::Explicit { provider, model } = &spec.model {
            let same_provider = provider
                .as_ref()
                .is_none_or(|name| name == &self.provider_name);
            if !same_provider || model != &self.model {
                return Err(RuntimeError::new(
                    ErrorKind::Config,
                    "child sessions currently run on the parent's provider and model; \
                     explicit child model routing is a coordinated follow-up",
                ));
            }
        }

        let workspace: Arc<dyn Workspace> = match &spec.workspace {
            WorkspacePolicy::SharedProject | WorkspacePolicy::ReadOnlyView => {
                self.workspace.clone()
            }
            WorkspacePolicy::ExplicitDirectory { path } => Arc::new(ProjectWorkspace::new(path)?),
            WorkspacePolicy::IsolatedWorktree => {
                return Err(RuntimeError::new(
                    ErrorKind::Config,
                    "isolated-worktree children are not available yet; use a shared, \
                     read-only, or explicit-directory workspace policy",
                ));
            }
        };

        let mut loop_config = self.loop_config.clone();
        loop_config.model = self.model.clone();

        Ok(RuntimeBuilder::new(self.model.clone())
            .provider(self.provider.clone())
            .provider_name(self.provider_name.clone())
            .model_profile(self.profile.clone())
            .loop_config(loop_config)
            .context_policy(self.context_policy.clone())
            .cache_capability(agent_runtime::context::ProviderCacheCapability::none(
                RegistryRevision::new(CACHE_CAPABILITY_REVISION),
                self.provider_kind.clone(),
            ))
            .legacy_approval_authority()
            .approval(self.approval.clone())
            .workspace(workspace)
            .tools(smith_tools::all())
            .clock(self.clock.clone()))
    }
}

/// The delegation surface a built Smith runtime carries until its session
/// exists: the child factory and the slot the coordinator is installed into.
#[derive(Debug, Clone)]
pub struct SmithDelegation {
    pub(crate) factory: Arc<SmithChildFactory>,
    pub(crate) slot: Arc<OnceLock<DelegationCoordinator>>,
}

impl SmithDelegation {
    /// The coordinator, once [`wire_delegation`] has run.
    pub fn coordinator(&self) -> Option<&DelegationCoordinator> {
        self.slot.get()
    }
}

/// Installs the delegation coordinator for a freshly started root session and
/// routes child results into its safe-boundary inbox.
///
/// Presentation is untouched: hosts render the attributed child lifecycle
/// events from the parent stream themselves. What this wires is model-facing:
/// a completed child's final result is injected must-deliver, so the parent
/// model receives it at the next provider/tool boundary and never mid-stream.
pub fn wire_delegation(
    session: &SessionHandle,
    delegation: &SmithDelegation,
) -> Result<(), RuntimeError> {
    let coordinator = DelegationCoordinator::new(
        session,
        delegation.factory.clone(),
        DelegationConfig {
            limits: DelegationLimits {
                max_running_children: DEFAULT_MAX_RUNNING_CHILDREN,
            },
            delegation_tool_names: vec![AGENT_TOOL_NAME.to_owned()],
            ..DelegationConfig::default()
        },
    )?;
    delegation
        .slot
        .set(coordinator)
        .map_err(|_| RuntimeError::new(ErrorKind::Conflict, "delegation is already wired"))?;

    let session = session.clone();
    let mut events = session.subscribe();
    tokio::spawn(async move {
        while let Some(envelope) = events.next().await {
            match envelope.payload {
                RuntimeEvent::ChildCompleted { child, result } => {
                    let text = if result.is_empty() {
                        format!("Sub-agent {child} completed with no visible answer.")
                    } else {
                        format!("Sub-agent {child} completed:\n{result}")
                    };
                    let _ = session.inject(InjectedContent::text(text).must_deliver());
                }
                RuntimeEvent::ChildFailed { child, error } => {
                    let _ = session.inject(InjectedContent::text(format!(
                        "Sub-agent {child} failed: {error}"
                    )));
                }
                RuntimeEvent::SessionShutdown => break,
                _ => {}
            }
        }
    });
    Ok(())
}

/// The model-facing delegation tool.
///
/// Declares no effects: the authority-bearing decision happens inside the
/// coordinator through the composed authorization path (covered by
/// [`DelegationAuthority`]), so declaring effects here would route one spawn
/// through approval twice.
#[derive(Debug)]
pub struct AgentTool {
    slot: Arc<OnceLock<DelegationCoordinator>>,
}

impl AgentTool {
    /// A tool over the coordinator `slot` the host fills after session start.
    pub fn new(slot: Arc<OnceLock<DelegationCoordinator>>) -> Self {
        Self { slot }
    }

    fn coordinator(&self) -> Result<&DelegationCoordinator, RuntimeError> {
        self.slot.get().ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::Config,
                "delegation is not wired for this session",
            )
        })
    }
}

/// One parsed `agent` tool call.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum AgentAction {
    /// Start a child with a task.
    Spawn {
        task: String,
        #[serde(default)]
        tools: ToolScopeArg,
        #[serde(default)]
        workspace: Option<WorkspaceArg>,
        #[serde(default)]
        max_turns: Option<u32>,
        #[serde(default)]
        max_tokens: Option<u64>,
        #[serde(default)]
        deadline_ms: Option<u64>,
    },
    /// List every child and its status.
    List,
    /// Block until a child is idle or stopped, then report it.
    Wait { child_id: String },
    /// Report a child's latest completed result.
    Result { child_id: String },
    /// Send a follow-up task to an idle child.
    FollowUp { child_id: String, task: String },
    /// Stop a child.
    Stop { child_id: String },
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolScopeArg {
    /// Read, list, and search only (the default).
    #[default]
    ReadOnly,
    /// Every built-in tool, including edit and shell.
    All,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkspaceArg {
    Shared,
    ReadOnly,
    Directory { path: String },
}

fn status_json(status: &ChildStatus) -> Value {
    let state = match &status.state {
        ChildState::Running => "running".to_owned(),
        ChildState::Idle => "idle".to_owned(),
        ChildState::Stopped { reason } => format!("stopped ({reason:?})"),
        ChildState::Failed => "failed".to_owned(),
    };
    json!({
        "child_id": status.child.as_str(),
        "state": state,
        "turns_used": status.turns_used,
        "max_turns": status.max_turns,
        "result": status.last_result,
    })
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        AGENT_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Delegate a task to a sub-agent. Actions: spawn (start a child with a task; \
         read-only tools unless tools=\"all\"), list, wait (block until a child finishes), \
         result, follow_up (send another task to a child), stop. A completed child's \
         result is also delivered to you automatically at the next safe point."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "list", "wait", "result", "follow_up", "stop"],
                    "description": "The delegation operation."
                },
                "task": {
                    "type": "string",
                    "description": "The task text (spawn and follow_up)."
                },
                "child_id": {
                    "type": "string",
                    "description": "The child to address (wait, result, follow_up, stop)."
                },
                "tools": {
                    "type": "string",
                    "enum": ["read_only", "all"],
                    "description": "The child's tool scope (spawn). Defaults to read_only."
                },
                "workspace": {
                    "description": "The child's workspace policy (spawn): \"shared\", \
                                    \"read_only\", or {\"directory\": {\"path\": \"…\"}}. \
                                    Defaults to read_only."
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Tasks the child may run in total. Defaults to 4."
                },
                "max_tokens": {
                    "type": "integer",
                    "description": "Optional total token budget for the child."
                },
                "deadline_ms": {
                    "type": "integer",
                    "description": "Optional lifetime deadline in milliseconds."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn effects(&self) -> ToolEffects {
        ToolEffects::read_only()
    }

    async fn invoke(
        &self,
        arguments: Value,
        _ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let action: AgentAction = serde_json::from_value(arguments).map_err(|err| {
            RuntimeError::new(ErrorKind::Tool, format!("unusable agent arguments: {err}"))
        })?;
        let coordinator = self.coordinator()?;

        match action {
            AgentAction::Spawn {
                task,
                tools,
                workspace,
                max_turns,
                max_tokens,
                deadline_ms,
            } => {
                let workspace = match workspace {
                    None | Some(WorkspaceArg::ReadOnly) => WorkspacePolicy::ReadOnlyView,
                    Some(WorkspaceArg::Shared) => WorkspacePolicy::SharedProject,
                    Some(WorkspaceArg::Directory { path }) => {
                        WorkspacePolicy::ExplicitDirectory { path }
                    }
                };
                let spec = ChildSpec {
                    task: UserInput::text(task),
                    model: ChildModelSelection::Inherit,
                    limits: ChildLimits {
                        max_turns: max_turns.unwrap_or(DEFAULT_CHILD_MAX_TURNS),
                        max_tokens,
                        deadline_ms,
                    },
                    tools: match tools {
                        ToolScopeArg::ReadOnly => ToolViewScope::ReadOnly,
                        ToolScopeArg::All => ToolViewScope::All,
                    },
                    workspace,
                };
                match coordinator.spawn(spec).await {
                    Ok(SpawnOutcome::Spawned { child, .. }) => Ok(ToolOutcome::json(json!({
                        "spawned": child.as_str(),
                        "note": "the result will be delivered when the child completes",
                    }))),
                    Ok(SpawnOutcome::Queued { child }) => Ok(ToolOutcome::json(json!({
                        "queued": child.as_str(),
                    }))),
                    Ok(SpawnOutcome::AtCapacity { running, limit }) => {
                        Ok(ToolOutcome::json(json!({
                            "at_capacity": { "running": running, "limit": limit },
                            "note": "stop or wait for a child before spawning another",
                        })))
                    }
                    Err(err) => Ok(ToolOutcome::error(err.message)),
                }
            }
            AgentAction::List => {
                let children: Vec<Value> = coordinator.list().iter().map(status_json).collect();
                Ok(ToolOutcome::json(json!({ "children": children })))
            }
            AgentAction::Wait { child_id } => {
                let status = coordinator
                    .wait(&agent_runtime_core::ids::ChildId::new(child_id))
                    .await;
                match status {
                    Ok(status) => Ok(ToolOutcome::json(status_json(&status))),
                    Err(err) => Ok(ToolOutcome::error(err.message)),
                }
            }
            AgentAction::Result { child_id } => {
                let status = coordinator.status(&agent_runtime_core::ids::ChildId::new(child_id));
                match status {
                    Ok(status) => Ok(ToolOutcome::json(status_json(&status))),
                    Err(err) => Ok(ToolOutcome::error(err.message)),
                }
            }
            AgentAction::FollowUp { child_id, task } => {
                let child = agent_runtime_core::ids::ChildId::new(child_id);
                match coordinator.follow_up(&child, UserInput::text(task)).await {
                    Ok(()) => Ok(ToolOutcome::json(json!({
                        "follow_up_sent": child.as_str(),
                    }))),
                    Err(err) => Ok(ToolOutcome::error(err.message)),
                }
            }
            AgentAction::Stop { child_id } => {
                let child = agent_runtime_core::ids::ChildId::new(child_id);
                match coordinator.stop(&child).await {
                    Ok(status) => Ok(ToolOutcome::json(status_json(&status))),
                    Err(err) => Ok(ToolOutcome::error(err.message)),
                }
            }
        }
    }
}
