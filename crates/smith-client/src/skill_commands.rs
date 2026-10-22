//! Descriptor-only slash shortcuts for the session's resolved skills.

/// One effective skill exposed by slash completion and help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCommand {
    /// Resolved skill name, without a slash.
    pub name: String,
    /// Authored description from the bounded index.
    pub description: String,
    /// Source layer or owning plugin.
    pub source: String,
    /// Reason this session cannot activate the skill.
    pub disabled_reason: Option<String>,
    /// Command spelling, possibly `skills use NAME` after a name collision.
    pub alias: String,
}

/// An explicit skill selection, optionally followed by an ordinary task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInvocation {
    /// Exact resolved skill name.
    pub name: String,
    /// Remaining task text, without the separating whitespace.
    pub arguments: String,
}

/// Builds effective shortcuts from descriptors and the session's capability view.
/// No skill instruction file is opened by this projection.
pub fn for_session(
    runtime: &smith_runtime::factory::SmithRuntime,
    session: &smith_runtime::SessionHandle,
) -> Vec<SkillCommand> {
    use smith_runtime::capability_limits::{CapabilityStanding, catalog};
    let capabilities = catalog(session, &[]);
    let mut winners = std::collections::BTreeMap::new();
    for entry in runtime.skill_index() {
        let previous = winners.entry(entry.name()).or_insert(entry);
        if (entry.activatable, entry.layer) > (previous.activatable, previous.layer) {
            *previous = entry;
        }
    }
    winners.into_values().map(|entry| {
        let id = entry.descriptor.id().qualified();
        let disabled_reason = if runtime.policy().harness.is_some() {
            Some("direct activation is unavailable on installed CLI backends; use their skill MCP bridge".to_owned())
        } else if !entry.activatable {
            let reason = match entry.trust_status {
                smith_config::trust::TrustStatus::Changed => "content changed",
                smith_config::trust::TrustStatus::Denied => "trust denied",
                _ => "awaiting trust",
            };
            Some(format!("{reason}; inspect `/skills` or use `/skills trust {}`", entry.name()))
        } else if !capabilities.iter().any(|row| row.id == id && matches!(row.standing,
            CapabilityStanding::Active | CapabilityStanding::Available)) {
            Some("denied by the active profile or session".to_owned())
        } else {
            None
        };
        SkillCommand {
            name: entry.name().to_owned(),
            alias: entry.name().to_owned(),
            description: entry.description().to_owned(),
            source: format!("skill · {}", entry.layer.as_str()),
            disabled_reason,
        }
    }).collect()
}
