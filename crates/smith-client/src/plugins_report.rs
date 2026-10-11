//! Installed bundle states and component attribution, independent of a terminal.

/// Current admission state of a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    /// Enabled content is admitted to the ordinary catalogs.
    On,
    /// Configuration disabled this plugin.
    Off,
    /// Installed bytes differ from the confirmed digest.
    Changed,
    /// Content has no matching allow decision.
    Untrusted(String),
    /// Installed content could not be inspected.
    Failed(String),
}

impl PluginState {
    /// Readable state with its withholding reason.
    pub fn label(&self) -> String {
        match self {
            Self::On => "on".into(),
            Self::Off => "off".into(),
            Self::Changed => "changed · content changed; run `smith plugin update`".into(),
            Self::Untrusted(reason) => format!("untrusted · {reason}"),
            Self::Failed(reason) => format!("failed · {reason}"),
        }
    }
}

/// A component the plugin carries, with its invocation or refusal explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginComponent {
    /// Skill, command, MCP server, or unsupported category.
    pub kind: String,
    /// Namespaced name.
    pub name: String,
    /// Resolved command line/URL or reason it is not loaded.
    pub detail: Option<String>,
}

/// One installed plugin and its catalog outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    /// Bundle name.
    pub name: String,
    /// Current admission state.
    pub state: PluginState,
    /// Source path, git URL/ref, or marketplace entry.
    pub source: String,
    /// Manifest version when supplied.
    pub version: Option<String>,
    /// Resolved git revision when fetched from git.
    pub resolved_commit: Option<String>,
    /// Digest bound to the install decision.
    pub digest: String,
    /// Effective enablement provenance.
    pub enabled_source: String,
    /// Components admitted to the catalogs.
    pub loaded: Vec<PluginComponent>,
    /// Components withheld, with reasons.
    pub not_loaded: Vec<PluginComponent>,
}

/// A stale configuration key naming an uninstalled plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPluginRow {
    /// Uninstalled name.
    pub name: String,
    /// Configuration source that named it.
    pub source: String,
}

/// Typed `/plugins` and CLI list content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginsReport {
    /// Installed rows, ordered by name.
    pub rows: Vec<PluginRow>,
    /// Nonfatal stale settings.
    pub unknown: Vec<UnknownPluginRow>,
}

/// Shared management guidance; the TUI offers no install/update/remove ceremony.
pub const GUIDANCE: &str = "Install, update, or remove with `smith plugin …`; use `/plugins NAME on|off` to enable or disable.";

/// Plain rendering shared by the CLI, transcript capture, and terminal report.
pub fn render_plain(report: &PluginsReport) -> String {
    let mut lines = Vec::new();
    if report.rows.is_empty() {
        lines.push("no plugins are installed".into());
    }
    for row in &report.rows {
        lines.push(format!(
            "{} · {} · version {}",
            row.name,
            row.state.label(),
            row.version.as_deref().unwrap_or("unspecified")
        ));
        lines.push(format!("  source: {}", row.source));
        if let Some(commit) = &row.resolved_commit {
            lines.push(format!("  resolved commit: {commit}"));
        }
        for (label, components) in [("loaded", &row.loaded), ("not loaded", &row.not_loaded)] {
            for component in components
                .iter()
                .filter(|component| component.kind == "sandboxed module")
            {
                lines.push(format!(
                    "  {label}: {} {}{}",
                    component.kind,
                    component.name,
                    component
                        .detail
                        .as_ref()
                        .map_or_else(String::new, |detail| format!(" · {detail}"))
                ));
            }
        }
        lines.push(format!("  digest: {}", row.digest));
        lines.push(format!("  enablement: {}", row.enabled_source));
        for (label, components) in [("loaded", &row.loaded), ("not loaded", &row.not_loaded)] {
            for component in components
                .iter()
                .filter(|component| component.kind != "sandboxed module")
            {
                lines.push(format!(
                    "  {label}: {} {}{}",
                    component.kind,
                    component.name,
                    component
                        .detail
                        .as_ref()
                        .map_or_else(String::new, |detail| format!(" · {detail}"))
                ));
            }
        }
    }
    for unknown in &report.unknown {
        lines.push(format!(
            "{} · not installed · {}",
            unknown.name, unknown.source
        ));
    }
    lines.push(GUIDANCE.into());
    lines.join("\n")
}

/// Joins live session mount/failure state to sandboxed inventory rows.
/// Content-only component rows remain untouched.
pub fn apply_module_states(report: &mut PluginsReport, modules: &[smith_module::ModuleReport]) {
    for plugin in &mut report.rows {
        for component in plugin
            .loaded
            .iter_mut()
            .chain(&mut plugin.not_loaded)
            .filter(|component| component.kind == "sandboxed module")
        {
            if let Some(module) = modules
                .iter()
                .find(|module| module.descriptor.id == component.name)
            {
                let state = module.current_state();
                let label = if state == smith_module::ModuleState::NotBuilt {
                    "not supported in this build".into()
                } else {
                    crate::modules_report::state_label(&state)
                };
                let detail = component.detail.as_deref().unwrap_or_default();
                let base = detail
                    .rsplit_once(" · state: ")
                    .map_or(detail, |(base, _)| base);
                component.detail = Some(format!("{base} · state: {label}"));
            }
        }
        let mut withheld = Vec::new();
        plugin.loaded.retain(|component| {
            let failed = component.kind == "sandboxed module"
                && modules
                    .iter()
                    .find(|module| module.descriptor.id == component.name)
                    .is_some_and(|module| {
                        matches!(
                            module.current_state(),
                            smith_module::ModuleState::Failed { .. }
                                | smith_module::ModuleState::Off
                                | smith_module::ModuleState::NotBuilt
                                | smith_module::ModuleState::Blocked { .. }
                        )
                    });
            if failed {
                withheld.push(component.clone());
            }
            !failed
        });
        plugin.not_loaded.extend(withheld);
    }
}
