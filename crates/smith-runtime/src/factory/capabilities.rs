//! Built-in, trusted-native, MCP, and delegation-tool capability stage.

use super::*;

pub(super) fn prepare(
    request: &RuntimeRequest,
    agent_tool_profiles: Vec<AgentToolProfile>,
    image_backend: Option<Arc<dyn smith_tools::ImageGenerationBackend>>,
    image_history: Arc<crate::image_history::SessionImageHistory>,
) -> Result<CapabilityStage, FactoryError> {
    let mut tools = tools(request, image_backend, image_history);
    // No tool, no projected plan state: a posture that cannot write a plan
    // should not carry one in its context either.
    let todo = todo_planning_eligible(request).then(|| Arc::new(TodoComponent::public()));
    let goal = goal_component_eligible(request).then(|| Arc::new(GoalComponent::public()));
    if goal.is_some() {
        tools.extend([
            Arc::new(GetGoalTool::new()) as Arc<dyn Tool>,
            Arc::new(CreateGoalTool::new()) as Arc<dyn Tool>,
            Arc::new(UpdateGoalTool::new()) as Arc<dyn Tool>,
        ]);
    }
    let host_tool_names = request
        .trusted_native
        .tools()
        .iter()
        .map(|tool| tool.spec().name)
        .collect::<BTreeSet<_>>();
    let mut ability_sources = tools
        .iter()
        .map(|tool| {
            if host_tool_names.contains(&tool.spec().name) {
                agent_runtime::registry::RegistrySource::Host
            } else {
                agent_runtime::registry::RegistrySource::BuiltIn
            }
        })
        .collect::<Vec<_>>();

    // Remote tools are appended after every local one, and a name already
    // spoken for is left alone. The namespacing in `mcp__<server>__<tool>`
    // makes a collision with a built-in impossible by construction; the check
    // stays because "impossible by construction" is a property of a naming
    // scheme, and a shadowed `shell` would be the worst possible way to
    // discover that the scheme had changed.
    if let Some(supervisor) = &request.mcp {
        let mut taken = tools
            .iter()
            .map(|tool| tool.spec().name)
            .collect::<BTreeSet<_>>();
        for tool in supervisor.tools() {
            let name = tool.spec().name;
            if !taken.insert(name.clone()) {
                tracing::warn!(
                    tool = %name,
                    "a remote tool was not registered because that name is already in use"
                );
                continue;
            }
            tools.push(tool);
            ability_sources.push(agent_runtime::registry::RegistrySource::Provider);
        }
    }

    let delegation_slot = if !delegation_eligible(request) {
        None
    } else {
        let slot = Arc::new(std::sync::OnceLock::new());
        let wait_policy = DelegationWaitPolicy::new(
            request.config.child_agents.wait_default_timeout_ms.value,
            request.config.child_agents.wait_max_timeout_ms.value,
        )
        .map_err(FactoryError::Runtime)?;
        tools.push(Arc::new(
            AgentTool::new(slot.clone())
                .with_profiles(agent_tool_profiles)
                .with_wait_policy(wait_policy),
        ) as Arc<dyn Tool>);
        ability_sources.push(agent_runtime::registry::RegistrySource::BuiltIn);
        Some(slot)
    };
    let abilities = seal_tool_abilities(tools.iter().cloned().zip(ability_sources))
        .map_err(FactoryError::AbilityRegistry)?;

    Ok(CapabilityStage {
        tools,
        abilities,
        todo,
        goal,
        delegation_slot,
    })
}

/// The tools this run registers.
pub(super) fn tools(
    request: &RuntimeRequest,
    image_backend: Option<Arc<dyn smith_tools::ImageGenerationBackend>>,
    image_history: Arc<crate::image_history::SessionImageHistory>,
) -> Vec<Arc<dyn Tool>> {
    let read_only = request.config.agent.active_posture().is_read_only();
    let background = request
        .background_services
        .as_ref()
        .map(BackgroundServices::host)
        .unwrap_or_else(smith_tools::background::unavailable);
    let mut tools = if request.built_in_tools {
        request.change_recorder.as_ref().map_or_else(
            || smith_tools::all_with_background(background.clone()),
            |recorder| {
                smith_tools::observed_tools_with_background(recorder.clone(), background.clone())
            },
        )
    } else {
        Vec::new()
    };
    if read_only {
        tools.retain(|tool| tool.spec().effects.is_read_only());
    }
    if !read_only
        && request.config.image_generation.enabled.value
        && let Some(backend) = image_backend
    {
        tools.push(Arc::new(smith_tools::GenerateImageTool::new(
            backend,
            image_history,
            request.config.user_dir.join("generated_images"),
            request.config.image_generation.model.value.clone(),
            request.config.image_generation.quality.value.clone(),
            request.config.image_generation.size.value.clone(),
        )));
    }
    if questionnaire_eligible(request) {
        tools.push(Arc::new(QuestionnaireTool::new()));
    }
    if let Some(store) = request.artifact_store.clone() {
        tools.push(Arc::new(
            ToolOutputContextPolicy::from_config(&request.config).reader(store),
        ));
    }
    if todo_planning_eligible(request) {
        tools.push(Arc::new(WriteTodosTool::new()));
    }
    tools.extend(
        request
            .trusted_native
            .tools()
            .iter()
            .filter(|tool| !read_only || read_only_extension(tool.spec()))
            .map(Arc::clone),
    );
    tools
}

pub(super) fn goal_component_eligible(request: &RuntimeRequest) -> bool {
    request.config.persistence.enabled.value && !matches!(request.surface, HostSurface::Child)
}

// The three predicates below decide both whether a tool is registered and
// whether its instruction section is contributed. They exist as named
// functions precisely so those two decisions cannot drift apart: a run whose
// prompt describes a capability it did not register is a run that will try to
// call a tool that is not there.
/// Whether this run registers the root questionnaire tool.
pub(super) fn questionnaire_eligible(request: &RuntimeRequest) -> bool {
    !matches!(request.surface, HostSurface::Child)
}

/// Whether this run registers the child-delegation `agent` tool.
pub(super) fn delegation_eligible(request: &RuntimeRequest) -> bool {
    !matches!(request.surface, HostSurface::Child) && request.config.agent.profile.delegation.value
}

/// Whether this run registers the todo-planning tool.
///
/// A read-only posture's deliverable *is* a plan or a review, so a second
/// parallel plan in tool state is redundant with the answer itself.
pub(super) fn todo_planning_eligible(request: &RuntimeRequest) -> bool {
    !request.config.agent.active_posture().is_read_only()
}

pub(super) fn read_only_extension(spec: agent_runtime_core::tool::ToolSpec) -> bool {
    spec.effects.is_read_only()
        && spec
            .permission_upper_bound
            .iter()
            .all(|permission| matches!(permission, Permission::FsRead | Permission::ClockRead))
}
