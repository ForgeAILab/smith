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
    ArtifactOffloader, ArtifactReadTool, MemoryContributor, QuestionnaireTool,
    SemanticSummaryCoordinator, TodoComponent, WriteTodosTool,
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
    pub(crate) provider: Arc<dyn Provider>,
    pub(crate) provider_name: String,
    pub(crate) provider_kind: String,
    pub(crate) cache_endpoint_identity: Option<CacheEndpointIdentity>,
    pub(crate) model: ModelId,
    pub(crate) model_profile: ResolvedModelProfile,
    pub(crate) context_policy: ContextPolicy,
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
        let mut tools = if write_capable {
            smith_tools::all()
        } else {
            smith_tools::read_only()
        };
        tools.push(Arc::new(QuestionnaireTool::new()));
        tools.push(Arc::new(WriteTodosTool::new()));
        if let Some(store) = self.artifact_store.clone() {
            tools.push(Arc::new(ArtifactReadTool::new(store)));
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
        let activation_budget = ActivationBudget::new(
            ContextBudget::from_limits(&route.model_profile.limits, &route.context_policy)
                .capability_budget,
            8,
        );

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
            builder = builder.tool_output_processor(Arc::new(ArtifactOffloader::new(store)));
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
        Ok(builder)
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

/// Process-owned workers that project parking and request conditional child
/// completion admission. Exact payloads and cursor authority remain Runtime-
/// owned; this handle exists so Smith can freeze admission before shutdown.
#[derive(Debug)]
pub struct DelegationLifecycle {
    parking: Arc<std::sync::Mutex<DelegationParking>>,
    cancel: Cancellation,
    signal: Arc<tokio::sync::Notify>,
    tasks: std::sync::Mutex<Option<Vec<tokio::task::JoinHandle<()>>>>,
}

/// Cloneable notification seam used by the host cache controller. It exposes
/// only the identity-only parking projection, never child result content.
#[derive(Debug, Clone)]
pub(crate) struct DelegationParkingMonitor {
    parking: Arc<std::sync::Mutex<DelegationParking>>,
    signal: Arc<tokio::sync::Notify>,
}

impl DelegationParkingMonitor {
    pub(crate) fn snapshot(&self) -> ParkingSnapshot {
        self.parking
            .lock()
            .expect("delegation parking state poisoned")
            .snapshot()
    }

    pub(crate) async fn changed(&self) {
        self.signal.notified().await;
    }
}

impl DelegationLifecycle {
    /// Current identity-only parking projection for status and tests.
    pub fn snapshot(&self) -> ParkingSnapshot {
        self.parking
            .lock()
            .expect("delegation parking state poisoned")
            .snapshot()
    }

    pub(crate) fn monitor(&self) -> DelegationParkingMonitor {
        DelegationParkingMonitor {
            parking: self.parking.clone(),
            signal: self.signal.clone(),
        }
    }

    /// Freezes new admission first, then cancels and boundedly drains both
    /// process-owned workers.
    pub async fn shutdown(&self) {
        self.parking
            .lock()
            .expect("delegation parking state poisoned")
            .shutdown();
        self.cancel.cancel(CancelReason::Shutdown);
        self.signal.notify_waiters();
        let tasks = self
            .tasks
            .lock()
            .expect("delegation lifecycle tasks poisoned")
            .take()
            .unwrap_or_default();
        for mut task in tasks {
            if tokio::time::timeout(std::time::Duration::from_millis(250), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

impl Drop for DelegationLifecycle {
    fn drop(&mut self) {
        self.cancel.cancel(CancelReason::Shutdown);
        self.signal.notify_waiters();
        for task in self
            .tasks
            .lock()
            .expect("delegation lifecycle tasks poisoned")
            .take()
            .unwrap_or_default()
        {
            task.abort();
        }
    }
}

/// Starts the local parking projection and Runtime-backed admission worker.
///
/// Runtime's coordinator owns exact outcome payloads, protected cursor
/// advancement, and conditional idle admission. Smith only projects
/// lifecycle state and never fabricates a user-role message or local
/// canonical event for a child result.
fn start_delegation_lifecycle_tasks(
    session: &SessionHandle,
    coordinator: DelegationCoordinator,
) -> DelegationLifecycle {
    let parking = Arc::new(std::sync::Mutex::new(DelegationParking::new()));
    let signal = Arc::new(tokio::sync::Notify::new());
    let cancel = Cancellation::new();
    let handle_parking = parking.clone();
    let handle_signal = signal.clone();

    let event_parking = parking.clone();
    let event_signal = signal.clone();
    let event_coordinator = coordinator.clone();
    let event_session = session.clone();
    let event_cancel = cancel.clone();
    let event_task = tokio::spawn(async move {
        let mut events = event_session.subscribe();
        loop {
            let envelope = tokio::select! {
                _ = event_cancel.cancelled() => break,
                envelope = events.next() => match envelope {
                    Some(envelope) => envelope,
                    None => break,
                },
            };
            let mut state = event_parking
                .lock()
                .expect("delegation parking state poisoned");
            match envelope.payload {
                RuntimeEvent::TurnStarted | RuntimeEvent::InternalTurnStarted { .. } => {
                    state.parent_turn_started();
                }
                RuntimeEvent::TurnCompleted { .. } => {
                    let pending = event_coordinator
                        .list()
                        .into_iter()
                        .filter(|status| matches!(status.state, ChildState::Running))
                        .map(|status| status.child.as_str().to_owned())
                        .collect::<Vec<_>>();
                    state.parent_turn_completed(pending);
                }
                RuntimeEvent::ChildSpawned { child, .. } => {
                    state.child_spawned(child.as_str());
                }
                RuntimeEvent::ChildNeedsInput { child, .. }
                | RuntimeEvent::ChildCompleted { child, .. }
                | RuntimeEvent::ChildStopped { child, .. }
                | RuntimeEvent::ChildFailed { child, .. } => {
                    state.child_terminal(child.as_str());
                }
                RuntimeEvent::SessionShutdown => state.shutdown(),
                _ => {}
            }
            drop(state);
            event_signal.notify_waiters();
        }
        event_parking
            .lock()
            .expect("delegation parking state poisoned")
            .shutdown();
        event_signal.notify_waiters();
    });

    let admission_parking = parking;
    let admission_signal = signal;
    let admission_cancel = cancel.clone();
    let admission_task = tokio::spawn(async move {
        let mut initial_snapshot = true;
        loop {
            if admission_cancel.is_cancelled() {
                break;
            }
            let notified = admission_signal.notified();
            tokio::pin!(notified);
            // Register before reading the protected snapshot. `notify_waiters`
            // does not retain a permit for a future that has not been polled,
            // so omitting this creates a lost-wakeup window between the
            // snapshot and the select below.
            notified.as_mut().enable();
            let mut progressed = false;

            // This is an idempotent protected snapshot, not an acknowledgement.
            // Runtime's child-completion admission remains the only operation
            // that advances the canonical cursor.
            progressed |= reconcile_ready_outcome_snapshot(
                &admission_parking,
                coordinator.take_ready_task_outcomes(),
                initial_snapshot,
            );
            initial_snapshot = false;

            let should_admit = admission_parking
                .lock()
                .expect("delegation parking state poisoned")
                .begin_child_completion_admission();
            if should_admit {
                let cursor = coordinator.child_outcome_cursor();
                let request = ChildCompletionAdmissionRequest::new(cursor.parent().clone(), cursor);
                match coordinator
                    .try_admit_child_completion_if_idle(request)
                    .await
                {
                    Ok(ChildCompletionAdmission::Accepted { turn, cursor }) => {
                        progressed = true;
                        admission_parking
                            .lock()
                            .expect("delegation parking state poisoned")
                            .admission_accepted(cursor.revision());
                        // Waiting for the ordinary turn boundary ensures a
                        // second attributed continuation cannot be started
                        // concurrently by this Smith worker.
                        turn.completed().await;
                        progressed |= reconcile_ready_outcome_snapshot(
                            &admission_parking,
                            coordinator.take_ready_task_outcomes(),
                            false,
                        );
                    }
                    Ok(ChildCompletionAdmission::Busy) => admission_parking
                        .lock()
                        .expect("delegation parking state poisoned")
                        .admission_busy(),
                    Ok(ChildCompletionAdmission::Stale) => {
                        progressed = true;
                        let revision = coordinator.child_outcome_cursor().revision();
                        admission_parking
                            .lock()
                            .expect("delegation parking state poisoned")
                            .admission_stale(revision);
                        progressed |= reconcile_ready_outcome_snapshot(
                            &admission_parking,
                            coordinator.take_ready_task_outcomes(),
                            false,
                        );
                    }
                    Ok(ChildCompletionAdmission::Shutdown) => {
                        admission_parking
                            .lock()
                            .expect("delegation parking state poisoned")
                            .shutdown();
                        break;
                    }
                    Ok(ChildCompletionAdmission::Conflict { .. }) | Err(_) => admission_parking
                        .lock()
                        .expect("delegation parking state poisoned")
                        .admission_conflict(),
                }
            }

            if admission_parking
                .lock()
                .expect("delegation parking state poisoned")
                .is_shutdown_frozen()
            {
                break;
            }
            if !progressed {
                tokio::select! {
                    _ = &mut notified => {}
                    _ = admission_cancel.cancelled() => break,
                }
            }
        }
        admission_parking
            .lock()
            .expect("delegation parking state poisoned")
            .shutdown();
    });
    DelegationLifecycle {
        parking: handle_parking,
        cancel,
        signal: handle_signal,
        tasks: std::sync::Mutex::new(Some(vec![event_task, admission_task])),
    }
}

fn reconcile_ready_outcome_snapshot(
    parking: &Arc<std::sync::Mutex<DelegationParking>>,
    outcomes: Vec<ChildTaskOutcome>,
    recovered: bool,
) -> bool {
    let keys = outcomes
        .iter()
        .map(terminal_outcome_key)
        .collect::<Vec<_>>();
    let mut state = parking.lock().expect("delegation parking state poisoned");
    for key in &keys {
        // The lossless Runtime snapshot closes the local live-child
        // projection even if a bounded presentation subscriber lagged.
        state.child_terminal(&key.child_id);
    }
    let changed = state.reconcile_ready_outcomes(keys);
    if recovered {
        state.enable_idle_wakeup_for_recovered_outcomes();
    }
    changed
}

fn terminal_outcome_key(outcome: &ChildTaskOutcome) -> TerminalOutcomeKey {
    match outcome {
        ChildTaskOutcome::Completed { child, result } => {
            TerminalOutcomeKey::new(child.as_str(), result.turn.as_str())
        }
        ChildTaskOutcome::NeedsInput { child, request } => {
            TerminalOutcomeKey::new(child.as_str(), request.id().as_str())
        }
    }
}

/// One child-enabled agent profile a `spawn` call may name.
///
/// Built once at construction directly from the same preflighted routes that
/// populate [`SmithChildFactory::profile_routes`] (see
/// `factory::prepare_child_profile_routes`), so the tool's advertised schema
/// and its resolution path can never name a profile the factory cannot route.
#[derive(Debug, Clone)]
pub struct AgentToolProfile {
    /// Stable profile name, as the model names it.
    pub name: String,
    /// Deterministic agent-profile revision, part of the route key.
    pub revision: String,
    /// The profile's serving provider name, for display only.
    pub provider: String,
    /// The profile's preflighted model.
    pub model: ModelId,
}

/// The model-facing delegation tool.
///
/// Declares no invocation effects because the authority-bearing decision
/// happens inside the coordinator through the composed authorization path
/// (covered by [`DelegationAuthority`]), so declaring effects here would route
/// one spawn through approval twice. Its specification still advertises the
/// conservative delegation permission upper bound so capability routing never
/// mistakes this host-defined authority for a risk-free tool.
#[derive(Debug)]
pub struct AgentTool {
    slot: Arc<OnceLock<DelegationCoordinator>>,
    profiles: Vec<AgentToolProfile>,
    wait_policy: DelegationWaitPolicy,
}

impl AgentTool {
    /// A tool over the coordinator `slot` the host fills after session start.
    /// Offers no selectable child profile until [`Self::with_profiles`] adds
    /// some.
    pub fn new(slot: Arc<OnceLock<DelegationCoordinator>>) -> Self {
        Self {
            slot,
            profiles: Vec::new(),
            wait_policy: DelegationWaitPolicy::default(),
        }
    }

    /// Offers `profiles` on `spawn`'s `profile` argument, exactly the
    /// child-enabled profiles [`SmithChildFactory`] preflighted a route for.
    pub fn with_profiles(mut self, profiles: Vec<AgentToolProfile>) -> Self {
        self.profiles = profiles;
        self
    }

    /// Applies the same resolved wait bounds installed on the coordinator.
    #[must_use]
    pub fn with_wait_policy(mut self, wait_policy: DelegationWaitPolicy) -> Self {
        self.wait_policy = wait_policy;
        self
    }

    fn coordinator(&self) -> Result<&DelegationCoordinator, RuntimeError> {
        self.slot.get().ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::Config,
                "delegation is not wired for this session",
            )
        })
    }

    /// Resolves a model-named profile against the registered directory.
    fn find_profile(&self, name: &str) -> Option<&AgentToolProfile> {
        self.profiles.iter().find(|profile| profile.name == name)
    }

    /// A stable, human-readable list of the available profile names, named in
    /// the refusal a spawn gets when it asks for one that is not registered.
    fn available_profiles_description(&self) -> String {
        if self.profiles.is_empty() {
            "none are registered".to_owned()
        } else {
            self.profiles
                .iter()
                .map(|profile| profile.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
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
        /// A registered child-enabled agent profile to run the spawn on.
        /// Absent inherits the parent's profile exactly as before profile
        /// selection existed.
        #[serde(default)]
        profile: Option<String>,
    },
    /// List every child and its status.
    List,
    /// Wait for a bounded interval, then report the child status.
    Wait {
        child_id: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Report a child's latest completed result.
    Result { child_id: String },
    /// Send a follow-up task to an idle child.
    FollowUp { child_id: String, task: String },
    /// Resume the exact checkpoint of an interrupted durable child.
    Resume { child_id: String },
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

/// Waits in the foreground for at most `timeout`, keeping each shared-runtime
/// wait call within its own hard maximum. A running result at the overall
/// boundary is a soft handoff: the child is deliberately left untouched so
/// the parent can finish its turn and park while the child continues.
async fn wait_for_child_foreground(
    coordinator: &DelegationCoordinator,
    child: &agent_runtime_core::ids::ChildId,
    timeout: Duration,
    runtime_slice: Duration,
) -> Result<(ChildStatus, bool), RuntimeError> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let slice = remaining.min(runtime_slice);
        let status = if slice.is_zero() {
            coordinator
                .wait_with_options(
                    child,
                    agent_runtime::delegation::DelegationWaitOptions {
                        timeout: Some(slice),
                    },
                )
                .await?
        } else {
            // Runtime normally enforces the same bound through its injected
            // clock. The host timer is a final safety net so a custom/frozen
            // clock cannot keep the parent call open beyond its soft boundary.
            match tokio::time::timeout(
                slice,
                coordinator.wait_with_options(
                    child,
                    agent_runtime::delegation::DelegationWaitOptions {
                        timeout: Some(slice),
                    },
                ),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => {
                    coordinator
                        .wait_with_options(
                            child,
                            agent_runtime::delegation::DelegationWaitOptions {
                                timeout: Some(Duration::ZERO),
                            },
                        )
                        .await?
                }
            }
        };
        if status.state != ChildState::Running {
            return Ok((status, false));
        }
        // `slice == remaining` means the shared wait consumed the last
        // foreground interval. Returning here does not stop the child; it
        // merely releases the parent tool call.
        if slice.is_zero() || slice >= remaining {
            return Ok((status, true));
        }
    }
}

fn status_json(status: &ChildStatus) -> Value {
    let state = match &status.state {
        ChildState::Running => "running".to_owned(),
        ChildState::Idle => "idle".to_owned(),
        ChildState::Interrupted { .. } => "interrupted".to_owned(),
        ChildState::Stopped { reason } => format!("stopped ({reason:?})"),
        ChildState::Failed => "failed".to_owned(),
        ChildState::Expired => "expired".to_owned(),
    };
    json!({
        "child_id": status.child.as_str(),
        "child_session_id": status.session.as_str(),
        "durability": match status.durability {
            ChildDurability::Ephemeral => "ephemeral",
            ChildDurability::Durable => "durable",
        },
        "state": state,
        "resumable": status.resumable(),
        "turns_used": status.turns_used,
        // Unlimited reads as null, not as the sentinel's absurd number.
        "max_turns": (status.max_turns != u32::MAX).then_some(status.max_turns),
        "tokens_used": status.tokens_used,
        "incompatibility": status.incompatibility,
        "result": status.last_result,
    })
}

fn wait_status_json(status: &ChildStatus, timed_out: bool) -> Value {
    let mut value = status_json(status);
    if let Value::Object(object) = &mut value {
        object.insert("timed_out".to_owned(), Value::Bool(timed_out));
        if timed_out {
            object.insert(
                "note".to_owned(),
                Value::String(
                    "foreground wait expired; the child continues running in the background and its terminal result will be delivered automatically"
                        .to_owned(),
                ),
            );
        }
    }
    value
}

fn task_outcome_json(outcome: &ChildTaskOutcome) -> Value {
    match outcome {
        ChildTaskOutcome::Completed { child, result } => json!({
            "child_id": child.as_str(),
            "state": "idle",
            "result": {
                "text": result.text,
                "artifacts": result.artifacts,
            },
        }),
        ChildTaskOutcome::NeedsInput { child, .. } => json!({
            "child_id": child.as_str(),
            "state": "needs_input",
            "informational": true,
            "needs_input": outcome
                .model_projection()
                .expect("needs-input outcome has a model projection"),
            "next_action": {
                "ask": "decide whether to invoke root ask_user",
                "return": "send the answer with agent follow_up"
            },
        }),
    }
}

#[async_trait]
impl Tool for AgentTool {
    fn spec(&self) -> ToolSpec {
        let description = if self.profiles.is_empty() {
            "Delegate a task to a sub-agent. Actions: spawn (start a child with a task; \
             read-only tools unless tools=\"all\"), list, wait (foreground for up to five minutes by default; timeout_ms may request a shorter bound; zero \
             is an immediate status check; an expired wait leaves the child running in the background and terminal results are delivered automatically), \
             result, follow_up (start a new task on an idle child), resume (continue an exact \
             interrupted checkpoint), stop. A completed child's \
             result is also delivered to you automatically at the next safe point. A child's \
             needs_input result is informational and does not open user interface; decide \
             whether to call root ask_user, then send the answer with an explicit follow_up."
                .to_owned()
        } else {
            format!(
                "Delegate a task to a sub-agent. Actions: spawn (start a child with a task; \
                 read-only tools unless tools=\"all\"), list, wait (foreground for up to five minutes by default; timeout_ms may request a shorter bound; zero \
                 is an immediate status check; an expired wait leaves the child running in the background and terminal results are delivered automatically), \
                 result, follow_up (start a new task on an idle child), resume \
                 (continue an exact interrupted checkpoint), stop. spawn may name a registered \
                 child-enabled profile ({}) to run the child on that profile's own preflighted \
                 provider, model, and posture instead of inheriting the parent's; omitting it \
                 inherits the parent's profile. A profile whose posture can write still needs \
                 tools=\"all\" to receive write-capable tools, and a read-only (the default) or \
                 otherwise declared read-only workspace keeps the child read-only no matter what \
                 posture or tool scope it asked for. A completed child's result is also \
                 delivered to you automatically at the next safe point. A child's needs_input \
                 result is informational and does not open user interface; decide whether to \
                 call root ask_user, then send the answer with an explicit follow_up.",
                self.available_profiles_description()
            )
        };
        let mut properties = serde_json::Map::new();
        properties.insert(
            "action".to_owned(),
            json!({
                "type": "string",
                // Keep the valid values as provider guidance rather than a
                // schema constraint. The shared runtime classifies completed
                // calls that fail their advertised schema as malformed
                // provider output, which aborts the turn before the tool can
                // report the mistake to the model. AgentAction remains the
                // authoritative parser, and its preparation error becomes a
                // canonical tool-error result that the model can correct on
                // the next loop step.
                "description": "The delegation operation. Must be one of: spawn, list, wait, result, follow_up, resume, stop."
            }),
        );
        properties.insert(
            "task".to_owned(),
            json!({
                "type": "string",
                "description": "The task text (spawn and follow_up)."
            }),
        );
        properties.insert(
            "child_id".to_owned(),
            json!({
                "type": "string",
                "description": "The child to address (wait, result, follow_up, resume, stop)."
            }),
        );
        properties.insert(
            "timeout_ms".to_owned(),
            json!({
                "type": "integer",
                "minimum": 0,
                "description": "Optional bounded wait in milliseconds (0 checks immediately; values above the configured host maximum are rejected)."
            }),
        );
        properties.insert(
            "tools".to_owned(),
            json!({
                "type": "string",
                "enum": ["read_only", "all"],
                "description": "The child's tool scope (spawn). Defaults to read_only. A \
                                 write-posture profile still needs \"all\" to receive \
                                 write-capable tools."
            }),
        );
        properties.insert(
            "workspace".to_owned(),
            json!({
                "description": "The child's workspace policy (spawn): \"shared\", \
                                \"read_only\", or {\"directory\": {\"path\": \"…\"}}. \
                                Defaults to read_only, which keeps the child read-only \
                                regardless of tool scope or profile posture."
            }),
        );
        if !self.profiles.is_empty() {
            let names: Vec<Value> = self
                .profiles
                .iter()
                .map(|profile| Value::String(profile.name.clone()))
                .collect();
            properties.insert(
                "profile".to_owned(),
                json!({
                    "type": "string",
                    "enum": names,
                    "description": "A registered child-enabled agent profile to run the spawn \
                                     on, resolved through its own preflighted provider/model \
                                     route (spawn). Absent inherits the parent's profile."
                }),
            );
        }
        ToolSpec::new(
            AGENT_TOOL_NAME,
            description,
            json!({
                "type": "object",
                "properties": Value::Object(properties),
                "required": ["action"],
                "additionalProperties": false
            }),
            ToolEffects::new(Vec::new()),
        )
        .with_permission_upper_bound(PermissionSet::single(Permission::other(
            DELEGATION_PERMISSION.to_owned(),
        )))
    }

    async fn prepare(
        &self,
        mut arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let action: AgentAction = serde_json::from_value(arguments.clone()).map_err(|err| {
            RuntimeError::new(ErrorKind::Tool, format!("unusable agent arguments: {err}"))
        })?;
        let (resource_id, title, detail) = match &action {
            AgentAction::Spawn {
                task,
                workspace: Some(WorkspaceArg::Directory { path }),
                ..
            } => {
                let canonical = ctx.workspace.resolve(path)?;
                let value = arguments
                    .pointer_mut("/workspace/directory/path")
                    .ok_or_else(|| {
                        RuntimeError::new(
                            ErrorKind::Tool,
                            "agent workspace directory could not be canonicalized",
                        )
                    })?;
                *value = Value::String(canonical);
                ("spawn".to_owned(), "Spawn sub-agent", Some(task.clone()))
            }
            AgentAction::Spawn { task, .. } => {
                ("spawn".to_owned(), "Spawn sub-agent", Some(task.clone()))
            }
            AgentAction::List => ("list".to_owned(), "List sub-agents", None),
            AgentAction::Wait {
                child_id,
                timeout_ms,
            } => (
                child_id.clone(),
                "Wait for sub-agent",
                Some(match timeout_ms {
                    Some(timeout_ms) => format!("{child_id} for {timeout_ms} ms"),
                    None => child_id.clone(),
                }),
            ),
            AgentAction::Result { child_id } => (
                child_id.clone(),
                "Read sub-agent result",
                Some(child_id.clone()),
            ),
            AgentAction::FollowUp { child_id, task } => (
                child_id.clone(),
                "Send sub-agent follow-up",
                Some(format!("{child_id}: {task}")),
            ),
            AgentAction::Resume { child_id } => (
                child_id.clone(),
                "Resume interrupted sub-agent",
                Some(format!("{child_id}: continue the exact saved checkpoint")),
            ),
            AgentAction::Stop { child_id } => {
                (child_id.clone(), "Stop sub-agent", Some(child_id.clone()))
            }
        };
        let mut display = ToolCallDisplay::new(title);
        if let Some(detail) = detail {
            display = display.with_detail(detail);
        }
        Ok(PreparedToolCall::new(
            ctx.call_id.clone(),
            AGENT_TOOL_NAME,
            arguments,
            PermissionSet::new(),
            SecurityResource::other("delegation", resource_id),
            ToolEffects::new(Vec::new()),
            display,
        ))
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        _ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let arguments = prepared.into_arguments();
        let action: AgentAction = serde_json::from_value(arguments).map_err(|err| {
            RuntimeError::new(ErrorKind::Tool, format!("unusable agent arguments: {err}"))
        })?;
        let coordinator = self.coordinator()?;

        match action {
            AgentAction::Spawn {
                task,
                tools,
                workspace,
                profile,
            } => {
                // Resolved before any lifecycle-creating call: an unknown,
                // non-child-enabled, or unrouted profile must fail without
                // creating a child or a lifecycle event, so this has to
                // short-circuit ahead of `coordinator.spawn` below.
                let model = match profile {
                    Some(name) => match self.find_profile(&name) {
                        Some(option) => ChildModelSelection::Explicit {
                            provider: Some(profile_route_key(&option.name, &option.revision)),
                            model: option.model.clone(),
                        },
                        None => {
                            return Ok(ToolOutcome::error(format!(
                                "child profile `{name}` is not registered for direct-child use; \
                                 available profiles: {}",
                                self.available_profiles_description()
                            )));
                        }
                    },
                    None => ChildModelSelection::Inherit,
                };
                let workspace = match workspace {
                    None | Some(WorkspaceArg::ReadOnly) => WorkspacePolicy::ReadOnlyView,
                    Some(WorkspaceArg::Shared) => WorkspacePolicy::SharedProject,
                    Some(WorkspaceArg::Directory { path }) => {
                        WorkspacePolicy::ExplicitDirectory { path }
                    }
                };
                let spec = ChildSpec {
                    task: UserInput::text(task),
                    model,
                    limits: UNLIMITED_CHILD_LIMITS,
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
            AgentAction::Wait {
                child_id,
                timeout_ms,
            } => {
                let child = agent_runtime_core::ids::ChildId::new(child_id);
                let timeout = match self.wait_policy.resolve_timeout(timeout_ms) {
                    Ok(timeout) => timeout,
                    Err(err) => return Ok(ToolOutcome::error(err.message)),
                };
                let (status, timed_out) = match wait_for_child_foreground(
                    coordinator,
                    &child,
                    timeout,
                    self.wait_policy.runtime_slice(),
                )
                .await
                {
                    Ok(result) => result,
                    Err(err) => return Ok(ToolOutcome::error(err.message)),
                };
                Ok(ToolOutcome::json(wait_status_json(&status, timed_out)))
            }
            AgentAction::Result { child_id } => {
                let outcome = coordinator
                    .task_outcome(&agent_runtime_core::ids::ChildId::new(child_id.clone()));
                match outcome {
                    Ok(Some(outcome)) => Ok(ToolOutcome::json(task_outcome_json(&outcome))),
                    Ok(None) => Ok(ToolOutcome::json(json!({
                        "child_id": child_id,
                        "state": "running",
                    }))),
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
            AgentAction::Resume { child_id } => {
                let child = agent_runtime_core::ids::ChildId::new(child_id);
                match coordinator.resume(&child).await {
                    Ok(()) => Ok(ToolOutcome::json(json!({
                        "resumed": child.as_str(),
                        "mode": "exact_checkpoint",
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
