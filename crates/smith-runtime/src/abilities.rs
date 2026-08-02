//! Smith's descriptor-first catalog for coding tools.
//!
//! The live provider tool surface remains owned by Agent Runtime. This module
//! gives the same concrete tools a sealed, bounded ability view so retrieval
//! can reason about semantic affordances, authority, risk, readiness, cost,
//! provenance, and revision without invoking or even preparing a call.

use std::sync::Arc;

use agent_runtime::ability::activation::{Activated, ActivationError, ActivationHandle};
use agent_runtime::ability::descriptor::{
    AbilityDescriptor, ContextCost, DependencyRequirement, ReadinessRequirement, RiskLevel,
};
use agent_runtime::ability::{Ability, AbilityKind, AbilityRegistry, SealedAbilities};
use agent_runtime::harness::{
    ARTIFACT_READ_TOOL_NAME, CREATE_GOAL_TOOL_NAME, GET_GOAL_TOOL_NAME, QUESTIONNAIRE_TOOL_NAME,
    UPDATE_GOAL_TOOL_NAME, WRITE_TODOS_TOOL_NAME,
};
use agent_runtime::registry::{
    EntryProvenance, NameConflict, Named, Permission, RegistryId, RegistryRevision, RegistrySource,
};
use agent_runtime_core::tool::{Tool, ToolSpec, canonicalize_json};

use crate::delegation::AGENT_TOOL_NAME;

/// Named readiness fact required before `ask_user` may activate.
pub const INTERACTION_READY_CONFIG: &str = "host.interaction";

/// Seals one deterministic ability entry for every registered tool.
///
/// Smith's six product tools have built-in provenance and explicit semantic
/// affordances. Extra injected tools remain visible, but correctly carry host
/// provenance and conservative permission-derived affordances.
pub fn seal_tool_abilities(
    tools: impl IntoIterator<Item = (Arc<dyn Tool>, RegistrySource)>,
) -> Result<SealedAbilities, NameConflict> {
    let mut registry = AbilityRegistry::new();
    for (tool, provenance) in tools {
        registry.register(Arc::new(SmithToolAbility::new(tool, provenance)))?;
    }
    Ok(registry.seal())
}

#[derive(Debug, Clone)]
struct SmithToolAbility {
    spec: ToolSpec,
    provenance: RegistrySource,
}

impl SmithToolAbility {
    fn new(tool: Arc<dyn Tool>, provenance: RegistrySource) -> Self {
        let spec = tool.spec();
        Self { spec, provenance }
    }
}

impl Named for SmithToolAbility {
    fn name(&self) -> &str {
        &self.spec.name
    }
}

impl Ability for SmithToolAbility {
    fn description(&self) -> &str {
        &self.spec.description
    }

    fn kind(&self) -> AbilityKind {
        AbilityKind::Tool
    }

    fn descriptor(&self) -> AbilityDescriptor {
        let mut canonical_spec = self.spec.clone();
        canonical_spec.input_schema = canonicalize_json(canonical_spec.input_schema);
        let spec_text =
            serde_json::to_string(&canonical_spec).expect("ToolSpec serialization is infallible");
        let revision = RegistryRevision::from_content(&spec_text);
        let permissions = self
            .spec
            .permission_upper_bound
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let schema_text = self.spec.input_schema.to_string();
        let mut descriptor = AbilityDescriptor::new(
            AbilityKind::Tool,
            self.spec.name.clone(),
            EntryProvenance::new(self.provenance, revision.clone()),
            self.spec.name.clone(),
            self.spec.description.clone(),
            revision,
        )
        .with_keywords(keywords(&self.spec.name))
        .with_affordances(affordances(&self.spec.name, &permissions))
        .with_permissions(permissions.clone())
        .with_risk(permission_risk(&permissions))
        .with_readiness(readiness(&self.spec.name))
        .with_context_cost(ContextCost::estimate(&schema_text, &self.spec.description))
        .with_input_modalities(["json"])
        .with_output_modalities(["text", "json"]);
        if self.spec.name == CREATE_GOAL_TOOL_NAME {
            descriptor = descriptor.with_dependency(DependencyRequirement::single(
                RegistryId::tool(UPDATE_GOAL_TOOL_NAME),
            ));
        }
        descriptor
    }
}

impl ActivationHandle for SmithToolAbility {
    fn activate(&self) -> Result<Activated, ActivationError> {
        Ok(Activated::ToolSchema(self.spec.to_schema()))
    }
}

fn keywords(name: &str) -> Vec<&'static str> {
    match name {
        "read" => vec![
            "read",
            "inspect",
            "examine",
            "open",
            "file",
            "source",
            "repository",
            "review",
        ],
        "list" => vec![
            "list",
            "browse",
            "tree",
            "directory",
            "files",
            "repository",
            "inspect",
            "review",
        ],
        "search" => vec![
            "search",
            "find",
            "locate",
            "grep",
            "symbol",
            "usages",
            "text",
            "repository",
            "inspect",
            "review",
        ],
        "edit" => vec![
            "edit",
            "fix",
            "implement",
            "replace",
            "create",
            "change",
            "update",
            "refactor",
            "modify",
            "patch",
            "write",
        ],
        "shell" => vec![
            "shell",
            "command",
            "run",
            "test",
            "tests",
            "cargo",
            "compile",
            "check",
            "verify",
            "validation",
            "benchmark",
            "build",
        ],
        AGENT_TOOL_NAME => vec![
            "agent",
            "delegate",
            "delegation",
            "sub-agent",
            "subagent",
            "reviewer",
            "review",
            "parallel",
            "investigate",
        ],
        QUESTIONNAIRE_TOOL_NAME => vec!["ask", "question", "clarify", "choice", "user"],
        WRITE_TODOS_TOOL_NAME => vec![
            "plan",
            "todo",
            "todos",
            "steps",
            "multi-step",
            "workflow",
            "track",
            "review",
        ],
        ARTIFACT_READ_TOOL_NAME => vec!["artifact", "read", "output", "page"],
        GET_GOAL_TOOL_NAME => vec!["goal", "persistent", "status", "objective", "budget"],
        CREATE_GOAL_TOOL_NAME => vec!["goal", "persistent", "create", "objective", "multi-turn"],
        UPDATE_GOAL_TOOL_NAME => vec![
            "goal",
            "persistent",
            "create",
            "objective",
            "multi-turn",
            "complete",
            "blocked",
            "finish",
        ],
        _ => Vec::new(),
    }
}

fn affordances(name: &str, permissions: &[Permission]) -> Vec<&'static str> {
    let mut values = match name {
        "read" => vec!["file-content-read", "file-read"],
        "list" => vec!["directory-list", "file-read"],
        "search" => vec!["file-search", "file-read"],
        "edit" => vec!["file-edit", "file-read", "file-write", "file-create"],
        "shell" => vec!["shell-command"],
        AGENT_TOOL_NAME => vec!["agent-delegation"],
        QUESTIONNAIRE_TOOL_NAME => vec!["user-interaction", "task-question"],
        WRITE_TODOS_TOOL_NAME => vec!["plan-management"],
        ARTIFACT_READ_TOOL_NAME => vec!["artifact-read"],
        GET_GOAL_TOOL_NAME => vec!["goal-read"],
        CREATE_GOAL_TOOL_NAME | UPDATE_GOAL_TOOL_NAME => vec!["goal-management"],
        _ => Vec::new(),
    };
    values.extend(permissions.iter().map(permission_affordance));
    values
}

fn readiness(name: &str) -> ReadinessRequirement {
    if name == QUESTIONNAIRE_TOOL_NAME {
        ReadinessRequirement::none().with_config_keys([INTERACTION_READY_CONFIG])
    } else {
        ReadinessRequirement::none()
    }
}

fn permission_affordance(permission: &Permission) -> &'static str {
    match permission {
        Permission::FsRead => "file-read",
        Permission::FsWrite => "file-write",
        Permission::FsCreate => "file-create",
        Permission::FsDelete => "file-delete",
        Permission::NetHttp => "network-http",
        Permission::DataEgress => "data-egress",
        Permission::CredentialUse => "credential-use",
        Permission::ProcessSpawn => "process-spawn",
        Permission::StdioRead => "stdio-read",
        Permission::StdioWrite => "stdio-write",
        Permission::ClockRead => "clock-read",
        Permission::RandomRead => "random-read",
        Permission::Other(_) => "host-defined-authority",
    }
}

fn permission_risk(permissions: &[Permission]) -> RiskLevel {
    permissions
        .iter()
        .map(|permission| match permission {
            Permission::FsDelete
            | Permission::DataEgress
            | Permission::CredentialUse
            | Permission::ProcessSpawn
            | Permission::Other(_) => RiskLevel::High,
            Permission::FsWrite
            | Permission::FsCreate
            | Permission::NetHttp
            | Permission::StdioRead
            | Permission::StdioWrite => RiskLevel::Medium,
            Permission::FsRead | Permission::ClockRead | Permission::RandomRead => RiskLevel::Low,
        })
        .max()
        .unwrap_or(RiskLevel::None)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::OnceLock;

    use agent_runtime::ability::descriptor::RiskLevel;
    use agent_runtime::capability::{ActivationBudget, CapabilityResolver, RoutingQuery};
    use agent_runtime::delegation::DELEGATION_PERMISSION;
    use agent_runtime::registry::{RegistryBuilder, RegistryEntry, ViewFilter};

    use super::*;
    use crate::delegation::AgentTool;

    fn six_tools() -> Vec<Arc<dyn Tool>> {
        let mut tools = smith_tools::all();
        tools.push(Arc::new(AgentTool::new(Arc::new(OnceLock::new()))));
        tools
    }

    fn coding_view() -> agent_runtime::registry::RegistryView<AbilityDescriptor> {
        view_for(six_tools())
    }

    fn workflow_view() -> agent_runtime::registry::RegistryView<AbilityDescriptor> {
        let mut tools = six_tools();
        tools.push(Arc::new(agent_runtime::harness::WriteTodosTool::new()));
        view_for(tools)
    }

    fn view_for(
        tools: Vec<Arc<dyn Tool>>,
    ) -> agent_runtime::registry::RegistryView<AbilityDescriptor> {
        let catalog = seal_tool_abilities(
            tools
                .into_iter()
                .map(|tool| (tool, RegistrySource::BuiltIn)),
        )
        .expect("a sealed catalog");
        let mut builder = RegistryBuilder::new();
        for descriptor in catalog.descriptors() {
            builder.declare(RegistryEntry::new(descriptor.card().clone(), descriptor));
        }
        builder
            .seal()
            .expect("a descriptor snapshot")
            .view(&ViewFilter::new().agent_facing(true))
    }

    fn selected(query: &str) -> BTreeSet<agent_runtime::registry::RegistryId> {
        selected_from(&coding_view(), query)
    }

    fn selected_from(
        view: &agent_runtime::registry::RegistryView<AbilityDescriptor>,
        query: &str,
    ) -> BTreeSet<agent_runtime::registry::RegistryId> {
        CapabilityResolver::new()
            .pre_activate(
                view,
                &RoutingQuery::derive(query, Vec::<String>::new()),
                ActivationBudget::new(16_000, 6),
            )
            .result()
            .activated_ids()
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    #[test]
    fn all_six_smith_tools_have_complete_bounded_descriptors() {
        let catalog = seal_tool_abilities(
            six_tools()
                .into_iter()
                .map(|tool| (tool, RegistrySource::BuiltIn)),
        )
        .expect("a sealed catalog");
        assert_eq!(
            catalog.names(),
            ["read", "list", "search", "edit", "shell", "agent"]
        );
        let descriptors: BTreeMap<_, _> = catalog
            .descriptors()
            .into_iter()
            .map(|descriptor| (descriptor.id().name.clone(), descriptor))
            .collect();

        let expected_risk = [
            ("read", RiskLevel::Low),
            ("list", RiskLevel::Low),
            ("search", RiskLevel::Low),
            ("edit", RiskLevel::Medium),
            ("shell", RiskLevel::High),
            ("agent", RiskLevel::High),
        ];
        for (name, risk) in expected_risk {
            let descriptor = &descriptors[name];
            assert_eq!(descriptor.risk(), risk, "{name}");
            assert!(
                descriptor.context_cost().total_tokens() > 0,
                "{name} has no activation cost"
            );
            assert!(descriptor.readiness().is_empty(), "{name}");
            assert_eq!(
                descriptor.card().provenance.source,
                RegistrySource::BuiltIn,
                "{name}"
            );
            assert_eq!(
                descriptor.card().provenance.revision,
                *descriptor.content_revision(),
                "{name}"
            );
            assert!(!descriptor.affordances().is_empty(), "{name}");
        }

        let permission_sets: BTreeMap<_, BTreeSet<_>> = descriptors
            .iter()
            .map(|(name, descriptor)| {
                (
                    name.clone(),
                    descriptor.permissions().iter().cloned().collect(),
                )
            })
            .collect();
        assert_eq!(
            permission_sets["read"],
            BTreeSet::from([Permission::FsRead])
        );
        assert_eq!(
            permission_sets["list"],
            BTreeSet::from([Permission::FsRead])
        );
        assert_eq!(
            permission_sets["search"],
            BTreeSet::from([Permission::FsRead])
        );
        assert_eq!(
            permission_sets["edit"],
            BTreeSet::from([
                Permission::FsRead,
                Permission::FsWrite,
                Permission::FsCreate,
            ])
        );
        assert_eq!(
            permission_sets["shell"],
            BTreeSet::from([
                Permission::FsRead,
                Permission::FsWrite,
                Permission::FsCreate,
                Permission::FsDelete,
                Permission::ProcessSpawn,
                Permission::NetHttp,
                Permission::DataEgress,
            ])
        );
        assert_eq!(
            permission_sets["agent"],
            BTreeSet::from([Permission::other(DELEGATION_PERMISSION.to_owned())])
        );

        let shell_affordances: BTreeSet<_> = descriptors["shell"]
            .affordances()
            .iter()
            .map(|affordance| affordance.as_str())
            .collect();
        for expected in [
            "shell-command",
            "file-delete",
            "process-spawn",
            "network-http",
            "data-egress",
        ] {
            assert!(
                shell_affordances.contains(expected),
                "shell is missing `{expected}`"
            );
        }
        assert!(
            descriptors["agent"]
                .affordances()
                .iter()
                .any(|affordance| affordance.as_str() == "agent-delegation")
        );
    }

    #[test]
    fn read_only_intent_never_routes_to_mutation_or_delegation() {
        let selected = selected("inspect and explain the Rust source files in this repository");
        for expected in ["read", "list", "search"] {
            assert!(
                selected.contains(&agent_runtime::registry::RegistryId::tool(expected)),
                "inspection intent omitted `{expected}`: {selected:?}"
            );
        }
        for forbidden in ["edit", "shell", "agent"] {
            assert!(
                !selected.contains(&agent_runtime::registry::RegistryId::tool(forbidden)),
                "read-only intent activated `{forbidden}`: {selected:?}"
            );
        }
    }

    #[test]
    fn an_explicit_read_tool_request_does_not_surface_every_tool() {
        let selected = selected(
            "Use the read tool to inspect live-proof.txt, then tell me in one concise sentence what value the file contains.",
        );
        assert!(
            selected.contains(&agent_runtime::registry::RegistryId::tool("read")),
            "explicit read request omitted read: {selected:?}"
        );
        for forbidden in ["edit", "shell", "agent"] {
            assert!(
                !selected.contains(&agent_runtime::registry::RegistryId::tool(forbidden)),
                "explicit read request activated broad `{forbidden}`: {selected:?}"
            );
        }
    }

    #[test]
    fn editing_intent_selects_exact_edit_without_broad_shell_or_delegation() {
        let selected = selected("fix the incorrect Rust function");
        assert!(
            selected.contains(&agent_runtime::registry::RegistryId::tool("edit")),
            "editing intent did not activate edit: {selected:?}"
        );
        for forbidden in ["shell", "agent"] {
            assert!(
                !selected.contains(&agent_runtime::registry::RegistryId::tool(forbidden)),
                "ordinary edit intent activated broad `{forbidden}`: {selected:?}"
            );
        }
    }

    #[test]
    fn shell_requires_explicit_command_build_or_test_intent() {
        let ordinary_edit = selected("fix the scheduler race and update its state transition");
        assert!(
            !ordinary_edit.contains(&agent_runtime::registry::RegistryId::tool("shell")),
            "ordinary editing activated broad shell: {ordinary_edit:?}"
        );

        let validation = selected("run cargo tests and verify the build");
        assert!(
            validation.contains(&agent_runtime::registry::RegistryId::tool("shell")),
            "explicit validation intent omitted shell: {validation:?}"
        );
    }

    #[test]
    fn review_workflow_activates_plan_tracking_and_delegation() {
        let selected = selected_from(
            &workflow_view(),
            "review this multi-step change with an independent reviewer and track the plan",
        );
        for expected in [WRITE_TODOS_TOOL_NAME, AGENT_TOOL_NAME] {
            assert!(
                selected.contains(&agent_runtime::registry::RegistryId::tool(expected)),
                "review workflow omitted `{expected}`: {selected:?}"
            );
        }
    }

    #[derive(Debug)]
    struct HostTool(&'static str);

    #[async_trait::async_trait]
    impl Tool for HostTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                self.0,
                "An injected host tool.",
                serde_json::json!({"type": "object", "properties": {}}),
                agent_runtime_core::tool::ToolEffects::read_only(),
            )
        }

        async fn prepare(
            &self,
            _arguments: serde_json::Value,
            _ctx: &agent_runtime_core::tool::PreparationContext,
        ) -> Result<
            agent_runtime_core::tool::PreparedToolCall,
            agent_runtime_core::error::RuntimeError,
        > {
            unreachable!("descriptor tests never prepare the host tool")
        }

        async fn invoke(
            &self,
            _prepared: agent_runtime_core::tool::PreparedToolCall,
            _ctx: &agent_runtime_core::tool::InvocationContext,
        ) -> Result<agent_runtime_core::tool::ToolOutcome, agent_runtime_core::error::RuntimeError>
        {
            unreachable!("descriptor tests never invoke the host tool")
        }
    }

    #[test]
    fn injected_tools_are_not_mislabelled_as_smith_builtins() {
        let tools: Vec<(Arc<dyn Tool>, RegistrySource)> =
            vec![(Arc::new(HostTool("host-tool")), RegistrySource::Host)];
        let descriptor = seal_tool_abilities(tools)
            .expect("a catalog")
            .descriptors()
            .pop()
            .expect("a descriptor");
        assert_eq!(descriptor.card().provenance.source, RegistrySource::Host);
        assert_eq!(descriptor.risk(), RiskLevel::Low);
        assert!(
            descriptor
                .affordances()
                .iter()
                .any(|affordance| affordance.as_str() == "file-read")
        );
    }

    #[test]
    fn a_host_tool_cannot_spoof_builtin_provenance_by_reusing_its_name() {
        let tools: Vec<(Arc<dyn Tool>, RegistrySource)> =
            vec![(Arc::new(HostTool("read")), RegistrySource::Host)];
        let descriptor = seal_tool_abilities(tools)
            .expect("a catalog")
            .descriptors()
            .pop()
            .expect("a descriptor");

        assert_eq!(descriptor.id().name, "read");
        assert_eq!(descriptor.card().provenance.source, RegistrySource::Host);
    }

    #[test]
    fn questionnaire_descriptor_requires_a_ready_interactive_host() {
        let tool: Arc<dyn Tool> = Arc::new(agent_runtime::harness::QuestionnaireTool::new());
        let descriptor = seal_tool_abilities([(tool, RegistrySource::BuiltIn)])
            .expect("a catalog")
            .descriptors()
            .pop()
            .expect("a descriptor");

        assert_eq!(descriptor.id().name, QUESTIONNAIRE_TOOL_NAME);
        assert_eq!(
            descriptor.readiness().config_keys,
            [INTERACTION_READY_CONFIG]
        );
        assert!(
            descriptor
                .affordances()
                .iter()
                .any(|affordance| affordance.as_str() == "user-interaction")
        );
        assert!(descriptor.permissions().is_empty());
        assert_eq!(descriptor.risk(), RiskLevel::None);
    }
}
