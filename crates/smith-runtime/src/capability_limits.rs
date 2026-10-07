//! The always-active core tools and the capability limits a session runs under.

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

/// The core tools as registry ids for the runtime's pinned set.
pub(crate) fn pinned_core() -> Vec<RegistryId> {
    CORE_TOOL_NAMES.into_iter().map(RegistryId::tool).collect()
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
