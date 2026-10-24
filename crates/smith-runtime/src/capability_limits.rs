//! The always-active core tools and the capability limits a session runs under.

use agent_runtime::ability::SealedAbilities;
use agent_runtime::hub::ScopeInputs;
use agent_runtime::registry::RegistryId;
use agent_runtime_core::error::RuntimeError;
use smith_config::resolve::ResolvedCapabilityLimits;

/// Built-in tools every session holds from its first request.
///
/// A model that has to discover its own terminal can conclude it has none, so
/// these never depend on retrieval. Posture and limits still narrow the set:
/// a tool the session does not register or may not see is simply not pinned.
pub const CORE_TOOL_NAMES: [&str; 7] = [
    "read",
    "list",
    "search",
    "edit",
    "shell",
    "task_output",
    "task_stop",
];

/// Core tools and image inspection, when mounted, in the runtime's pinned set.
pub(crate) fn pinned_core(abilities: &SealedAbilities) -> Vec<RegistryId> {
    let registered = abilities.names();
    CORE_TOOL_NAMES
        .into_iter()
        .chain(["view_image"])
        .filter(|name| registered.contains(name))
        .map(RegistryId::tool)
        .collect()
}

/// Names the registered tools a session may activate but does not start with.
///
/// Discovery by search can miss: a model that searched the wrong words or
/// domain concludes the tool does not exist. One line naming what is there
/// makes `registry.activate` reachable without a search. Built from the tools
/// actually registered, minus the pinned core and anything the session limits
/// deny, so it never names a tool the session could not activate.
pub(crate) fn activatable_tools(
    tools: &[std::sync::Arc<dyn agent_runtime_core::tool::Tool>],
    abilities: &SealedAbilities,
    limits: &CapabilityLimits,
) -> Option<ActivatableTools> {
    let pinned = pinned_core(abilities);
    let names = tools
        .iter()
        .map(|tool| tool.spec().name)
        .filter(|name| {
            let id = RegistryId::tool(name);
            !pinned.contains(&id) && limits.admits(&id)
        })
        .collect::<Vec<_>>();
    if names.is_empty() {
        return None;
    }
    let body = format!(
        "Tools you can activate with registry.activate (ids `tool:<name>`): {}.",
        names.join(", ")
    );
    use sha2::{Digest, Sha256};
    let revision = agent_runtime::registry::RegistryRevision::new(format!(
        "{:x}",
        Sha256::digest(body.as_bytes())
    ));
    Some(ActivatableTools { body, revision })
}

/// The one developer fragment [`activatable_tools`] contributes.
#[derive(Debug)]
pub(crate) struct ActivatableTools {
    body: String,
    revision: agent_runtime::registry::RegistryRevision,
}

#[async_trait::async_trait]
impl agent_runtime::harness::ContextContributor for ActivatableTools {
    fn descriptor(&self) -> agent_runtime::harness::ComponentDescriptor {
        agent_runtime::harness::ComponentDescriptor::new(
            "smith.activatable-tools",
            self.revision.clone(),
        )
    }

    async fn contribute(
        &self,
        _: &agent_runtime::harness::ContextView,
    ) -> Result<agent_runtime::harness::ContextPatch, RuntimeError> {
        use agent_runtime::context::{
            ContextFragment, FragmentContent, FragmentKind, FragmentSource,
        };
        Ok(agent_runtime::harness::ContextPatch::new(vec![
            ContextFragment::new(
                "smith.activatable-tools",
                FragmentKind::DeveloperInstruction,
                FragmentSource::Host,
                self.revision.clone(),
                FragmentContent::Text(self.body.clone()),
            ),
        ]))
    }
}

/// Validated `<domain>:<name>` patterns limiting what a session may use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityLimits {
    /// When non-empty, only capabilities matching one of these are usable.
    pub allow: Vec<String>,
    /// Capabilities matching any of these are never usable.
    pub deny: Vec<String>,
}

impl CapabilityLimits {
    /// Whether the declared session limits admit this exact registry id.
    pub(crate) fn admits(&self, id: &RegistryId) -> bool {
        let matches = |pattern: &String| {
            agent_runtime::hub::CapabilityPattern::parse(pattern)
                .is_ok_and(|pattern| pattern.matches(id))
        };
        !self.deny.iter().any(matches) && (self.allow.is_empty() || self.allow.iter().any(matches))
    }

    /// A profile's limits plus the denials the session added on top.
    pub fn for_session(profile: &ResolvedCapabilityLimits, session_denials: &[String]) -> Self {
        let mut limits = Self::from_profile(profile);
        limits.deny.extend(session_denials.iter().cloned());
        limits
    }

    fn from_profile(profile: &ResolvedCapabilityLimits) -> Self {
        Self {
            allow: profile
                .allow
                .as_ref()
                .map(|patterns| patterns.value.clone())
                .unwrap_or_default(),
            deny: profile
                .deny
                .as_ref()
                .map(|patterns| patterns.value.clone())
                .unwrap_or_default(),
        }
    }

    /// The limits a delegated child runs under.
    ///
    /// A child is the parent's run narrowed, so every parent denial carries
    /// over: otherwise a session denied `tool:shell` could reach a shell by
    /// delegating. The child profile's own allow list replaces the parent's
    /// when it declares one.
    pub fn for_child(&self, child: &ResolvedCapabilityLimits) -> Self {
        let own = Self::from_profile(child);
        let mut deny = self.deny.clone();
        deny.extend(own.deny);
        Self {
            allow: if own.allow.is_empty() {
                self.allow.clone()
            } else {
                own.allow
            },
            deny,
        }
    }

    /// Narrows a session scope to these limits.
    pub(crate) fn apply(&self, mut inputs: ScopeInputs) -> Result<ScopeInputs, RuntimeError> {
        for pattern in &self.allow {
            inputs = inputs
                .allow_pattern(pattern)
                .map_err(RuntimeError::config)?;
        }
        for pattern in &self.deny {
            inputs = inputs.deny_pattern(pattern).map_err(RuntimeError::config)?;
        }
        Ok(inputs)
    }
}

/// Where one capability stands for a session, as a host shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityStanding {
    /// In the current activation epoch.
    Active,
    /// Authorized, and activatable by the agent.
    Available,
    /// Withheld by a `/capabilities deny` in this session.
    DeniedBySession,
    /// Withheld by the profile's limits or posture.
    DeniedByProfile,
}

/// One row of a session's capability catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRow {
    /// Qualified registry id, such as `tool:shell`.
    pub id: String,
    /// The capability's one-line summary.
    pub summary: String,
    /// Whether the session holds, may activate, or is denied it.
    pub standing: CapabilityStanding,
}

/// The session's capabilities, without the discovery bootstraps the agent
/// uses to reach them.
pub fn catalog(
    session: &agent_runtime::runtime::SessionHandle,
    session_denials: &[String],
) -> Vec<CapabilityRow> {
    use agent_runtime::capability::CapabilityState;
    let session_patterns = session_denials
        .iter()
        .filter_map(|pattern| agent_runtime::hub::CapabilityPattern::parse(pattern).ok())
        .collect::<Vec<_>>();
    session
        .capability_catalog()
        .into_iter()
        .filter(|entry| !entry.id.name.starts_with("registry."))
        .map(|entry| CapabilityRow {
            standing: match entry.state {
                CapabilityState::Active => CapabilityStanding::Active,
                CapabilityState::Available => CapabilityStanding::Available,
                CapabilityState::Denied
                    if session_patterns
                        .iter()
                        .any(|pattern| pattern.matches(&entry.id)) =>
                {
                    CapabilityStanding::DeniedBySession
                }
                CapabilityState::Denied => CapabilityStanding::DeniedByProfile,
            },
            id: entry.id.qualified(),
            summary: entry.summary,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use agent_runtime::registry::RegistrySource;

    use super::*;

    #[test]
    fn the_activatable_index_names_registered_tools_outside_the_core_and_limits() {
        let mut tools = smith_tools::all();
        tools.push(Arc::new(crate::delegation::AgentTool::new(Arc::new(
            OnceLock::new(),
        ))));
        tools.push(Arc::new(agent_runtime::harness::WriteTodosTool::new()));
        let abilities = crate::abilities::seal_tool_abilities(
            tools
                .iter()
                .cloned()
                .map(|tool| (tool, RegistrySource::BuiltIn)),
        )
        .unwrap();
        let open = activatable_tools(&tools, &abilities, &CapabilityLimits::default()).unwrap();
        assert!(open.body.contains("registry.activate"), "{}", open.body);
        assert!(open.body.contains("agent"), "{}", open.body);
        for core in CORE_TOOL_NAMES {
            assert!(
                !open.body.contains(&format!(" {core},"))
                    && !open.body.contains(&format!(" {core}.")),
                "{core}: {}",
                open.body
            );
        }
        let denied = CapabilityLimits {
            allow: Vec::new(),
            deny: vec!["tool:agent".into()],
        };
        let narrowed = activatable_tools(&tools, &abilities, &denied).unwrap();
        assert!(!narrowed.body.contains("agent"), "{}", narrowed.body);
        assert_ne!(open.revision, narrowed.revision);
    }
}
