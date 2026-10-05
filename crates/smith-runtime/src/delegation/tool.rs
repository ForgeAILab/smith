use super::*;

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
        // Why the last task failed, when one did. A model reading a `failed`
        // state with no reason has to guess, and the guess is usually that it
        // should try the same thing again.
        "error": status.last_error.as_ref().map(|error| error.message.clone()),
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

/// Tells Runtime this call's result hands `outcome` to the model, so the
/// automatic completion turn does not deliver it a second time. Runtime acts
/// only once the result commits. A refusal leaves automatic delivery in place,
/// which repeats the result rather than losing it.
fn acknowledge_delivered(
    coordinator: &DelegationCoordinator,
    ctx: &InvocationContext,
    outcome: &ChildTaskOutcome,
) {
    if let Some(turn) = &ctx.turn {
        let _ = coordinator.acknowledge_task_outcome_on_tool_result(turn, &ctx.call_id, outcome);
    }
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
             result is delivered to you automatically at the next safe point unless wait or \
             result already returned it to you. A child's \
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
                 posture or tool scope it asked for. A completed child's result is delivered to \
                 you automatically at the next safe point unless wait or result already returned \
                 it to you. A child's needs_input \
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
        ctx: &InvocationContext,
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
                // The status carries the completed result text; when it is
                // the child's latest outcome, the model now has it.
                if let (Some(text), Ok(Some(outcome))) = (
                    status.last_result.as_deref(),
                    coordinator.task_outcome(&child),
                ) && matches!(&outcome, ChildTaskOutcome::Completed { result, .. } if result.text == text)
                {
                    acknowledge_delivered(coordinator, ctx, &outcome);
                }
                Ok(ToolOutcome::json(wait_status_json(&status, timed_out)))
            }
            AgentAction::Result { child_id } => {
                let outcome = coordinator
                    .task_outcome(&agent_runtime_core::ids::ChildId::new(child_id.clone()));
                match outcome {
                    Ok(Some(outcome)) => {
                        acknowledge_delivered(coordinator, ctx, &outcome);
                        Ok(ToolOutcome::json(task_outcome_json(&outcome)))
                    }
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
