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
    wait_timers: Arc<AgentWaitTimers>,
    transcript: ChildTranscriptReader,
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
            wait_timers: Arc::new(AgentWaitTimers::default()),
            transcript: ChildTranscriptReader::new(
                None,
                crate::journal::DefaultRedactor::new(),
                crate::tool_output::ToolOutputContextPolicy {
                    inline_bytes: 8192,
                    artifact_page_bytes: 2048,
                },
            ),
        }
    }

    /// Offers `profiles` on `spawn`'s `profile` argument, exactly the
    /// child-enabled profiles [`SmithChildFactory`] preflighted a route for.
    pub fn with_profiles(mut self, profiles: Vec<AgentToolProfile>) -> Self {
        self.profiles = profiles;
        self
    }

    /// Applies the resolved host wake timer bounds.
    #[must_use]
    pub fn with_wait_policy(mut self, wait_policy: DelegationWaitPolicy) -> Self {
        self.wait_policy = wait_policy;
        self
    }

    /// Shares process-local timers with the session's delegation lifecycle.
    #[must_use]
    pub fn with_wait_timers(mut self, wait_timers: Arc<AgentWaitTimers>) -> Self {
        self.wait_timers = wait_timers;
        self
    }

    /// Applies the root's output budget, artifact store, and shared credential policy.
    #[must_use]
    pub fn with_transcript_output(
        mut self,
        store: Option<Arc<dyn ArtifactStore>>,
        redactor: crate::journal::DefaultRedactor,
        policy: crate::tool_output::ToolOutputContextPolicy,
    ) -> Self {
        self.transcript = ChildTranscriptReader::new(store, redactor, policy);
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

    /// Adds a running child's current-task activity to its status object.
    async fn insert_activity(
        &self,
        coordinator: &DelegationCoordinator,
        status: &ChildStatus,
        value: &mut Value,
    ) {
        if let (Some(activity), Value::Object(object)) =
            (self.wait_timers.activity(coordinator, status).await, value)
        {
            object.insert("activity".to_owned(), activity);
        }
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
    /// Report status immediately and optionally arm a single-use wake timer.
    Wait {
        child_id: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Report a child's latest completed result.
    Result { child_id: String },
    /// Inspect committed history without acknowledging completion or executing work.
    Transcript {
        child_id: String,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        page_size: Option<usize>,
    },
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

pub(super) fn status_json(status: &ChildStatus) -> Value {
    let state = match &status.state {
        ChildState::Running => "running".to_owned(),
        ChildState::Idle => "idle".to_owned(),
        ChildState::Interrupted { .. } => "interrupted".to_owned(),
        ChildState::Stopped { reason } => format!("stopped ({reason:?})"),
        ChildState::Failed => "failed".to_owned(),
        ChildState::Expired => "expired".to_owned(),
    };
    let mut value = json!({
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
        // Only an idle child's last result answers its latest task. Any other
        // state keeps an earlier task's answer, which beside `running` reads
        // as the current task's result.
        "result": if status.state == ChildState::Idle {
            status.last_result.as_deref()
        } else {
            None
        },
    });
    // An idle child with no result may have stopped at a limit; say which,
    // so the model does not read it as an empty answer.
    if let (Some(limit), Value::Object(object)) = (child_status_limit(status), &mut value) {
        object.insert("stopped_at_limit".to_owned(), json!(limit));
    }
    value
}

/// What a started task's result tells the model to do next. A model that is
/// told only "the result will be delivered" may write the delivery itself.
const OUTCOME_PENDING_NOTE: &str = "The child's outcome will arrive as a host message in a \
    later turn. Do any independent work, then end your response. Do not predict, announce, or \
    write the outcome yourself.";

fn wait_status_json(
    status: &ChildStatus,
    monitor: Option<ChildWaitMonitor>,
    clamped: bool,
) -> Value {
    let mut value = status_json(status);
    if let Value::Object(object) = &mut value {
        object.insert("armed".to_owned(), json!(monitor.is_some()));
        object.insert("clamped".to_owned(), json!(clamped));
        object.insert(
            "wake_after_ms".to_owned(),
            json!(monitor.as_ref().map(|timer| timer.wake_after_ms)),
        );
        object.insert(
            "deadline".to_owned(),
            json!(monitor.as_ref().map(|timer| timer.deadline)),
        );
        if status.state == ChildState::Running {
            object.remove("result");
            let note = if monitor.is_some() {
                "This call returned immediately. The child's outcome, or a notice that the armed deadline expired, will arrive as a host message in a later turn; the deadline fires once. Do any independent work, then end your response. Do not predict, announce, or write the outcome yourself."
            } else {
                "This call returned immediately and no deadline is armed. The child's outcome will arrive as a host message in a later turn. Do any independent work, then end your response. Do not predict, announce, or write the outcome yourself."
            };
            object.insert("note".to_owned(), Value::String(note.to_owned()));
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
                // The stored copy a completion delivery names for a long
                // reply, so this result and that delivery cite one id.
                "reply_artifact": result.reply.as_ref().map(|reply| reply.id.as_str()),
            },
        }),
        ChildTaskOutcome::LimitReached {
            child,
            limit,
            artifacts,
            ..
        } => json!({
            "child_id": child.as_str(),
            "state": "idle",
            "stopped_at_limit": limit.as_str(),
            // Whatever the child said before the limit is a preamble or a
            // step, never the task's answer.
            "result": Value::Null,
            "artifacts": artifacts,
            "note": format!(
                "The child's task stopped at its {} before it gave a final answer. Nothing it \
                 said in that task is a result. The child is still available: send a follow_up \
                 to continue the work{}, or read its transcript to see what it did.",
                child_limit_label(*limit),
                if *limit == LimitKind::Output {
                    " in smaller steps"
                } else {
                    ""
                },
            ),
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
        const ACTIONS: &str = "Delegate a task to a sub-agent (child). Actions: spawn (start a \
            child with a task; read-only tools unless tools=\"all\"); list (every child's \
            status); wait (returns immediately; a positive timeout_ms arms one wake-up, five \
            minutes by default and at most; 0 only reports status); result (the child's latest \
            completed result); transcript (pages of the child's committed conversation, by \
            page_size and cursor, including a running task's finished steps but not a response \
            still streaming; read large entries with artifact.read; reading does not consume \
            the outcome delivery); follow_up (a new task for an idle child); \
            resume (continue an interrupted child from its exact checkpoint); stop.";
        const DELIVERY: &str = "After spawn, follow_up, or wait, do any independent work, then \
            end your response with no further tool calls. The child's outcome, or an expired \
            wait deadline, arrives as a host message that starts a later turn; none follows \
            when wait or result already returned the outcome. Never write that message \
            yourself or announce a completion you have not received. Copy artifact ids exactly \
            from tool results or host messages; never invent one. A running child's status \
            carries no result; its activity shows elapsed time, steps so far, and any response \
            still streaming. Do not call list, wait, result, or transcript repeatedly to \
            detect completion. A child's needs_input result is informational and does not \
            open user interface; decide whether to call root ask_user, then send the answer \
            with an explicit follow_up. Stop a child with agent stop and its child_id; \
            task_stop and task_output control background shell tasks.";
        let description = if self.profiles.is_empty() {
            format!("{ACTIONS} {DELIVERY}")
        } else {
            format!(
                "{ACTIONS} spawn may name a registered child-enabled profile ({}) to run the \
                 child on that profile's own preflighted provider, model, and posture instead of \
                 inheriting the parent's; omitting it inherits the parent's profile. A profile \
                 whose posture can write still needs tools=\"all\" to receive write-capable \
                 tools, and a read-only (the default) or otherwise declared read-only workspace \
                 keeps the child read-only no matter what posture or tool scope it asked for. \
                 {DELIVERY}",
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
                "description": "The delegation operation. Must be one of: spawn, list, wait, result, transcript, follow_up, resume, stop."
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
                "description": "The child to address (wait, result, transcript, follow_up, resume, stop)."
            }),
        );
        properties.insert(
            "cursor".to_owned(),
            json!({
                "type": "string",
                "description": "Opaque next_cursor from a transcript page. Omit to capture new history; existing cursors retain the same snapshot."
            }),
        );
        properties.insert(
            "page_size".to_owned(),
            json!({
                "type": "integer",
                "description": "Maximum transcript entries per page, 1..=100 (default 20). The output budget can shorten a page; follow next_cursor."
            }),
        );
        properties.insert(
            "timeout_ms".to_owned(),
            json!({
                "type": "integer",
                "minimum": 0,
                "description": "Optional single-use wake timer in milliseconds. Returns immediately. 0 only checks status; positive values have no minimum and are clamped to the configured maximum (at most 300000). End your turn; you wake automatically."
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
                "Monitor sub-agent",
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
            AgentAction::Transcript { child_id, .. } => (
                child_id.clone(),
                "Read sub-agent transcript",
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
                let model = match &profile {
                    Some(name) => match self.find_profile(name) {
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
                    Ok(SpawnOutcome::Spawned { child, .. }) => {
                        self.wait_timers
                            .task_started(child.as_str(), profile.as_deref());
                        Ok(ToolOutcome::json(json!({
                            "spawned": child.as_str(),
                            "note": OUTCOME_PENDING_NOTE,
                        })))
                    }
                    Ok(SpawnOutcome::Queued { child }) => {
                        self.wait_timers
                            .task_started(child.as_str(), profile.as_deref());
                        Ok(ToolOutcome::json(json!({
                            "queued": child.as_str(),
                            "note": OUTCOME_PENDING_NOTE,
                        })))
                    }
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
                let mut children = Vec::new();
                for status in coordinator.list() {
                    let mut value = status_json(&status);
                    self.insert_activity(coordinator, &status, &mut value).await;
                    children.push(value);
                }
                Ok(ToolOutcome::json(json!({ "children": children })))
            }
            AgentAction::Wait {
                child_id,
                timeout_ms,
            } => {
                let child = agent_runtime_core::ids::ChildId::new(child_id);
                let timeout = self.wait_policy.resolve_timeout(timeout_ms);
                let status = match coordinator.status(&child) {
                    Ok(status) => status,
                    Err(err) => return Ok(ToolOutcome::error(err.message)),
                };
                let clamped = timeout_ms
                    .is_some_and(|requested| requested > self.wait_policy.max_timeout_ms());
                let monitor = if status.state != ChildState::Running {
                    self.wait_timers.cancel(child.as_str());
                    None
                } else if timeout.is_zero() {
                    // A status check arms nothing, but must not hide a timer
                    // an earlier wait left armed.
                    self.wait_timers.armed(child.as_str())
                } else {
                    self.wait_timers.arm(child.as_str(), timeout)
                };
                // The status carries the completed result text; when it is
                // the child's latest outcome, the model now has it. A
                // needs-input or limit outcome adds what the status lacks.
                if status.state != ChildState::Running
                    && let Ok(Some(outcome)) = coordinator.task_outcome(&child)
                {
                    acknowledge_delivered(coordinator, ctx, &outcome);
                    if matches!(
                        outcome,
                        ChildTaskOutcome::NeedsInput { .. } | ChildTaskOutcome::LimitReached { .. }
                    ) {
                        let mut value = wait_status_json(&status, None, clamped);
                        value.as_object_mut().expect("status object").extend(
                            task_outcome_json(&outcome)
                                .as_object()
                                .expect("outcome object")
                                .clone(),
                        );
                        return Ok(ToolOutcome::json(value));
                    }
                }
                let mut value = wait_status_json(&status, monitor, clamped);
                self.insert_activity(coordinator, &status, &mut value).await;
                Ok(ToolOutcome::json(value))
            }
            AgentAction::Result { child_id } => {
                let outcome = coordinator
                    .task_outcome(&agent_runtime_core::ids::ChildId::new(child_id.clone()));
                match outcome {
                    Ok(Some(outcome)) => {
                        if coordinator
                            .status(&agent_runtime_core::ids::ChildId::new(&child_id))
                            .is_ok_and(|status| status.state != ChildState::Running)
                        {
                            self.wait_timers.cancel(&child_id);
                        }
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
            AgentAction::Transcript {
                child_id,
                cursor,
                page_size,
            } => {
                let child = agent_runtime_core::ids::ChildId::new(child_id);
                match self
                    .transcript
                    .page(coordinator, &child, cursor.as_deref(), page_size, ctx)
                    .await
                {
                    Ok(page) => Ok(ToolOutcome::json(serde_json::to_value(page).map_err(
                        |error| {
                            RuntimeError::tool(format!(
                                "transcript page could not be serialized: {error}"
                            ))
                        },
                    )?)),
                    Err(error) => Ok(ToolOutcome::error(error.message)),
                }
            }
            AgentAction::FollowUp { child_id, task } => {
                let child = agent_runtime_core::ids::ChildId::new(child_id);
                match coordinator.follow_up(&child, UserInput::text(task)).await {
                    Ok(()) => {
                        self.wait_timers.task_restarted(child.as_str());
                        Ok(ToolOutcome::json(json!({
                            "follow_up_sent": child.as_str(),
                            "note": OUTCOME_PENDING_NOTE,
                        })))
                    }
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
                    Ok(status) => {
                        self.wait_timers.cancel(child.as_str());
                        Ok(ToolOutcome::json(status_json(&status)))
                    }
                    Err(err) => Ok(ToolOutcome::error(err.message)),
                }
            }
        }
    }
}
