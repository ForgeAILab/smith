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
//!   exists and lets Agent Runtime admit protected child completion batches as
//!   attributed internal turns only at an idle boundary.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use agent_runtime::ability::Ability;
use agent_runtime::ability::activation::{ActivationContext, FailClosedPolicy};
use agent_runtime::agent::config::LoopConfig;
use agent_runtime::capability::{ActivationBudget, CapabilityResolver};
use agent_runtime::context::{ContextBudget, ContextPolicy};
use agent_runtime::delegation::{
    ChildCompletionAdmission, ChildCompletionAdmissionRequest, ChildDurability,
    ChildRuntimeFactory, ChildState, ChildStatus, ChildTaskOutcome, DELEGATION_PERMISSION,
    DelegationConfig, DelegationCoordinator, DelegationLimits, DurableChildSpec, SpawnOutcome,
};
use agent_runtime::harness::{
    MemoryContributor, QuestionnaireTool, SemanticSummaryCoordinator, TodoComponent, WriteTodosTool,
};
use agent_runtime::hub::{ScopeIdentity, ScopeInputs};
use agent_runtime::registry::{Fingerprint, Permission, RegistryRevision, RegistrySource};
use agent_runtime::runtime::{RuntimeBuilder, SessionHandle};
use agent_runtime_core::approval::ApprovalPolicy;
use agent_runtime_core::artifact::ArtifactStore;
use agent_runtime_core::cancel::{CancelReason, Cancellation};
use agent_runtime_core::catalog::ResolvedModelProfile;
use agent_runtime_core::check_set::ActionClass;
use agent_runtime_core::checkpoint::CheckpointStore;
use agent_runtime_core::clock::Clock;
use agent_runtime_core::content::UserInput;
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::event::RuntimeEvent;
use agent_runtime_core::grant::{
    GrantConstraints, SecurityCheck, SecurityCheckId, SecurityCheckMode, SecurityCheckOutcome,
    SecurityCheckRevision,
};
use agent_runtime_core::provider::{CacheEndpointIdentity, ModelId, Provider};
use agent_runtime_core::security::{AuthorizationRequest, PermissionSet, SecurityResource};
use agent_runtime_core::store::SessionStore;
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolCallDisplay, ToolEffects,
    ToolOutcome, ToolSpec,
};
use agent_runtime_core::workspace::Workspace;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use smith_config::model::AgentPosture;
use smith_host::ProjectWorkspace;

use crate::abilities::{INTERACTION_READY_CONFIG, seal_tool_abilities};
use crate::authority::SmithToolAuthority;
use crate::cli_agent::TurnExecution;
use crate::factory::CACHE_CAPABILITY_REVISION;
use crate::prompt::SmithPromptContributor;

#[path = "delegation_parking.rs"]
pub mod delegation_parking;

pub use self::delegation_parking::{
    DelegationParking, DelegationWaitPolicy, ParentParkingState, ParkingSnapshot, TerminalBatch,
    TerminalOutcomeKey,
};

/// The model-facing delegation tool's name — Smith product policy.
pub const AGENT_TOOL_NAME: &str = "agent";

/// Reviewed workspace wording exposed to clients through the runtime boundary.
pub use smith_tools::display::agent_workspace_display;

/// The limits a spawned child runs under: none. Smith deliberately spawns
/// children unbounded — the coordinator's concurrency cap is the only brake.
/// `ChildLimits::max_turns` is a required count in the shared runtime, so
/// "no limit" is expressed as the counter's full range.
pub const UNLIMITED_CHILD_LIMITS: ChildLimits = ChildLimits {
    max_turns: u32::MAX,
    max_tokens: None,
    deadline_ms: None,
};

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
    pub(crate) default_route: SmithChildRoute,
    pub(crate) profile_routes: BTreeMap<String, SmithChildRoute>,
    /// The parent session's limits, which every child inherits.
    pub(crate) capability_limits: crate::capability_limits::CapabilityLimits,
    pub(crate) approval: Arc<dyn ApprovalPolicy>,
    pub(crate) workspace: Arc<dyn Workspace>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) artifact_store: Option<Arc<dyn ArtifactStore>>,
    pub(crate) session_store: Option<Arc<dyn SessionStore>>,
    pub(crate) checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    pub(crate) skills: Vec<Arc<dyn Ability>>,
    pub(crate) memory: Option<MemoryContributor>,
    pub(crate) semantic_summary: Option<Arc<SemanticSummaryCoordinator>>,
}

/// One fully preflighted provider/model/profile route available to children.
#[derive(Debug, Clone)]
pub struct SmithChildRoute {
    pub(crate) modules: smith_module::ModuleComposition,
    pub(crate) module_context: smith_module::ModuleContext,
    pub(crate) provider: Arc<dyn Provider>,
    pub(crate) provider_name: String,
    pub(crate) provider_kind: String,
    pub(crate) cache_endpoint_identity: Option<CacheEndpointIdentity>,
    pub(crate) model: ModelId,
    pub(crate) model_profile: ResolvedModelProfile,
    pub(crate) context_policy: ContextPolicy,
    pub(crate) tool_output_context: crate::tool_output::ToolOutputContextPolicy,
    pub(crate) loop_config: LoopConfig,
    pub(crate) prompt_contributor: SmithPromptContributor,
    pub(crate) agent_profile_name: String,
    pub(crate) agent_profile_revision: String,
    pub(crate) agent_profile_posture: AgentPosture,
    /// Whether this route's agent-profile posture is read-only
    /// (`agent_profile_posture.is_read_only()`). A child can reach
    /// write-capable tools only when this is `false` *and* its spawn asked
    /// for a full tool scope and a non-read-only workspace policy — see
    /// [`SmithChildFactory::child_builder`].
    pub(crate) read_only: bool,
    /// The child profile's own capability limits, applied with the parent's.
    pub(crate) capabilities: smith_config::resolve::ResolvedCapabilityLimits,
    /// What executes this route's turns: the provider above, or the installed
    /// agent the profile's model id named.
    pub(crate) execution: TurnExecution,
}

/// Opaque Smith host route persisted in the existing child model-selection slot.
pub fn profile_route_key(name: &str, revision: &str) -> String {
    let short_revision = revision.get(..16).unwrap_or(revision);
    format!("smith-profile:{name}@{short_revision}")
}

impl SmithChildFactory {
    fn route_for(&self, selection: &ChildModelSelection) -> Result<&SmithChildRoute, RuntimeError> {
        match selection {
            ChildModelSelection::Inherit => Ok(&self.default_route),
            ChildModelSelection::Explicit { provider, model } => {
                if let Some(route_key) = provider
                    && let Some(route) = self.profile_routes.get(route_key)
                {
                    if model == &route.model {
                        return Ok(route);
                    }
                    return Err(RuntimeError::config(format!(
                        "child profile route `{route_key}` resolves model `{}` rather than `{model}`",
                        route.model
                    )));
                }
                let same_provider = provider
                    .as_ref()
                    .is_none_or(|name| name == &self.default_route.provider_name);
                if same_provider && model == &self.default_route.model {
                    Ok(&self.default_route)
                } else {
                    Err(RuntimeError::config(
                        "the requested child provider/model has no preflighted agent-profile route",
                    ))
                }
            }
        }
    }
}

impl ChildRuntimeFactory for SmithChildFactory {
    fn artifact_store(&self) -> Option<Arc<dyn ArtifactStore>> {
        self.artifact_store.clone()
    }

    fn session_store(&self) -> Option<Arc<dyn SessionStore>> {
        self.session_store.clone()
    }

    fn checkpoint_store(&self) -> Option<Arc<dyn CheckpointStore>> {
        self.checkpoint_store.clone()
    }

    fn policy_fingerprint(&self, spec: &DurableChildSpec) -> Result<Fingerprint, RuntimeError> {
        let route = self.route_for(&spec.model)?;
        let prompt_revisions = route
            .prompt_contributor
            .fragments()
            .iter()
            .map(|fragment| format!("{}@{}", fragment.id, fragment.revision))
            .collect::<Vec<_>>();
        let skill_names = self
            .skills
            .iter()
            .map(|ability| ability.name().to_owned())
            .collect::<Vec<_>>();
        let encoded = serde_json::to_vec(&json!({
            "schema_version": 1,
            "spec": spec,
            "provider_name": route.provider_name,
            "provider_kind": route.provider_kind,
            "model": route.model,
            "model_profile": route.model_profile,
            "agent_profile_name": route.agent_profile_name,
            "agent_profile_revision": route.agent_profile_revision,
            "agent_profile_placement": "child",
            "agent_profile_posture": route.agent_profile_posture.as_str(),
            "context_policy_revision": route.context_policy.revision,
            "prompt_revisions": prompt_revisions,
            "skill_names": skill_names,
            "workspace_root": self.workspace.root(),
            "read_only": route.read_only,
            "execution": route.execution.label(),
            "modules": route.modules.enabled,
            "module_revisions": route.modules.compiled.iter()
                .filter(|module| route.modules.enabled.contains(module.module.id()))
                .map(|module| (module.module.id(), module.module.revision()))
                .collect::<Vec<_>>(),
            "module_settings": format!("{:?}", route.modules.settings),
        }))
        .map_err(|error| {
            RuntimeError::new(
                ErrorKind::Serialization,
                format!("Smith child policy could not be fingerprinted: {error}"),
            )
        })?;
        Ok(Fingerprint::of(encoded))
    }

    fn child_builder(&self, spec: &ChildSpec) -> Result<RuntimeBuilder, RuntimeError> {
        let route = self.route_for(&spec.model)?;

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

        let mut loop_config = route.loop_config.clone();
        loop_config.model = route.model.clone();
        let tool_authority = Arc::new(SmithToolAuthority::new(workspace.root()));
        let tool_coverage = tool_authority.coverage().clone();

        // A child reaches write-capable tools only when three things hold at
        // once: its resolved route's agent-profile posture is not read-only,
        // its spawn declared a full tool scope, and its spawn's workspace
        // policy is not the read-only view. The workspace key is not
        // optional. Just above, `WorkspacePolicy::ReadOnlyView` is mapped to
        // this same shared `self.workspace` handle as
        // `WorkspacePolicy::SharedProject` — there is no separate read-only
        // wrapper, so within this factory nothing about the workspace object
        // itself refuses a write. The tool set chosen here is what actually
        // enforces "read-only" for that policy. `WorkspacePolicy::ReadOnlyView`
        // is also what `AgentTool` defaults an absent `workspace` argument to,
        // so without this third key a build-posture spawn that asked for
        // `tools: "all"` but named no workspace would silently receive
        // write-capable tools against the shared project.
        let write_capable = !route.read_only
            && spec.tools == ToolViewScope::All
            && !matches!(spec.workspace, WorkspacePolicy::ReadOnlyView);
        let mut module_context = route.module_context.clone();
        module_context.posture = if write_capable {
            smith_module::ModulePosture::ReadWrite
        } else {
            smith_module::ModulePosture::ReadOnly
        };
        let mut module_plan = smith_module::mount_modules(&route.modules, &module_context);
        crate::factory::modules::enforce_posture(&mut module_plan, module_context.posture);
        let mut tools = if write_capable {
            smith_tools::all()
        } else {
            smith_tools::read_only()
        };
        for contribution in module_plan
            .mounted
            .iter()
            .flat_map(|module| &module.contributions)
        {
            if let smith_module::ModuleContribution::Tool(tool) = contribution {
                tools.push(tool.clone());
            }
        }
        tools.push(Arc::new(QuestionnaireTool::new()));
        tools.push(Arc::new(WriteTodosTool::new()));
        if let Some(store) = self.artifact_store.clone() {
            tools.push(Arc::new(route.tool_output_context.reader(store)));
        }
        let todo_component = Arc::new(TodoComponent::public());
        let abilities = seal_tool_abilities(
            tools
                .iter()
                .cloned()
                .map(|tool| (tool, RegistrySource::BuiltIn)),
        )
        .map_err(|error| RuntimeError::conflict(error.to_string()))?;
        let scope_inputs = ScopeInputs::new().with_identity(
            ScopeIdentity::new()
                .with_workspace(workspace.root())
                .with_agent("child"),
        );
        let scope_inputs = self
            .capability_limits
            .for_child(&route.capabilities)
            .apply(scope_inputs)?;
        let activation_budget = ActivationBudget::new(
            ContextBudget::from_limits(&route.model_profile.limits, &route.context_policy)
                .capability_budget,
            8,
        );

        // A profile that names an installed agent runs its child turns on
        // that agent. The route's provider stays exactly where it is -- it
        // supplies model identity and the limits planned against above -- and
        // is simply never called to produce a turn.
        let external_agent = match &route.execution {
            TurnExecution::Provider => None,
            TurnExecution::InstalledAgent(plan) => Some(plan.backend(
                std::path::PathBuf::from(workspace.root()),
                // The CLI runs its own tools under its own permission policy,
                // outside everything the three keys above decided. A child
                // that was not granted write-capable tools does not get them
                // back through the agent it happens to run on.
                write_capable,
            )),
            TurnExecution::MissingProgram { kind, program, .. } => {
                return Err(RuntimeError::new(
                    ErrorKind::Config,
                    format!(
                        "child profile `{}` runs turns on the installed agent `{kind}`, but \
                         `{program}` is not on PATH; install it, or declare \
                         `[harness.{kind}]` with an absolute `executable`",
                        route.agent_profile_name
                    ),
                ));
            }
        };

        let mut builder = RuntimeBuilder::new(route.model.clone())
            .provider(route.provider.clone())
            .provider_name(route.provider_name.clone())
            .model_profile(route.model_profile.clone())
            .loop_config(loop_config)
            .context_policy(route.context_policy.clone())
            .cache_capability(
                agent_runtime::context::ProviderCacheCapability::from_control(
                    RegistryRevision::new(CACHE_CAPABILITY_REVISION),
                    route.provider_kind.clone(),
                    route
                        .provider
                        .capabilities(&route.model)
                        .map(|capabilities| capabilities.prompt_cache)
                        .unwrap_or_default(),
                ),
            )
            .security_check(
                tool_authority,
                SecurityCheckMode::Authoritative,
                tool_coverage,
                ActionClass::new("smith-built-in-tools"),
            )
            .approval(self.approval.clone())
            .workspace(workspace)
            .tools(tools)
            .live_ability_routing()
            .pinned_abilities(crate::capability_limits::pinned_core())
            .scope_inputs(scope_inputs)
            .capability_resolver(Arc::new(CapabilityResolver::new()))
            .activation_policy(Arc::new(FailClosedPolicy))
            // This readiness fact denotes the coordinator-owned
            // ReturnToParent route. It does not install or borrow the root
            // UI broker; the runtime flips the concrete disposition only
            // after applying the child's narrowed tool view.
            .activation_context(
                ActivationContext::new().with_ready_config([INTERACTION_READY_CONFIG]),
            )
            .activation_budget(activation_budget)
            .context_contributor(Arc::new(route.prompt_contributor.clone()))
            .context_contributor(todo_component.clone())
            .tool_output_processor(todo_component.clone())
            .turn_commit_hook(todo_component)
            .clock(self.clock.clone());
        if let Some(backend) = external_agent {
            builder = builder.external_agent(backend);
        }
        if let Some(identity) = route.cache_endpoint_identity.as_ref() {
            builder = builder.cache_endpoint_identity(identity.clone());
        }
        if let Some(contributor) = self.memory.clone() {
            builder = builder.context_contributor(Arc::new(contributor));
        }
        if let Some(coordinator) = self.semantic_summary.clone() {
            builder = builder
                .history_projector(coordinator.clone())
                .turn_commit_hook(coordinator);
        }
        if let Some(store) = self.artifact_store.clone() {
            builder = builder
                .tool_output_processor(Arc::new(route.tool_output_context.offloader(store)?));
        }
        if let Some(store) = self.session_store.clone() {
            builder = builder.session_store(store);
        }
        if let Some(store) = self.checkpoint_store.clone() {
            builder = builder.checkpoint_store(store);
        }
        for descriptor in abilities.descriptors() {
            builder = builder.tool_ability_descriptor(descriptor);
        }
        for skill in self.skills.iter().cloned() {
            builder = builder.ability(skill);
        }
        Ok(crate::factory::modules::apply(builder, &module_plan))
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
/// starts bounded child-completion admission with the standard wait policy.
pub async fn wire_delegation(
    session: &SessionHandle,
    delegation: &SmithDelegation,
) -> Result<DelegationLifecycle, RuntimeError> {
    wire_delegation_with_wait_policy(session, delegation, DelegationWaitPolicy::default()).await
}

/// Installs delegation with a resolved host wait policy.
///
/// The policy is passed directly to Agent Runtime's coordinator; Smith does
/// not implement a second waiting loop or silently widen the runtime maximum.
pub async fn wire_delegation_with_wait_policy(
    session: &SessionHandle,
    delegation: &SmithDelegation,
    wait_policy: DelegationWaitPolicy,
) -> Result<DelegationLifecycle, RuntimeError> {
    wait_policy.resolve_timeout(None)?;
    let coordinator = DelegationCoordinator::new(
        session,
        delegation.factory.clone(),
        DelegationConfig {
            limits: DelegationLimits {
                max_running_children: DEFAULT_MAX_RUNNING_CHILDREN,
                ..DelegationLimits::default()
            },
            delegation_tool_names: vec![AGENT_TOOL_NAME.to_owned()],
            wait_default: wait_policy.runtime_default_timeout(),
            wait_max: wait_policy.runtime_max_timeout(),
            ..DelegationConfig::default()
        },
    )?;
    coordinator.recover().await?;
    delegation
        .slot
        .set(coordinator.clone())
        .map_err(|_| RuntimeError::new(ErrorKind::Conflict, "delegation is already wired"))?;

    Ok(start_delegation_lifecycle_tasks(session, coordinator))
}

mod lifecycle;
mod tool;

pub(crate) use lifecycle::DelegationParkingMonitor;
use lifecycle::start_delegation_lifecycle_tasks;
pub use lifecycle::{DelegationLifecycle, next_admission_retry_delay};
pub use tool::{AgentTool, AgentToolProfile};
