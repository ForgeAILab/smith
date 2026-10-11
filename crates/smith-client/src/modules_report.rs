//! Module mount outcomes and the configuration choice behind each row.

use std::collections::BTreeMap;

use smith_config::resolve::ResolvedModule;
pub use smith_config::resolve::{Layer, Source};
pub use smith_module::{ModuleBlockReason, ModuleOrigin, ModuleState};

/// Every known module in deterministic id order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulesReport {
    /// Compiled modules and first-party modules omitted from the build.
    pub rows: Vec<ModuleRow>,
}

/// One module's build provenance, effective switch, and mount outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRow {
    /// Stable configuration id.
    pub id: String,
    /// One-line description.
    pub description: String,
    /// Actual composition outcome, including safe failure explanations.
    pub state: ModuleState,
    /// Key actually written and the layer that won resolution.
    pub source: Source,
    /// First-party or the named third-party crate.
    pub origin: ModuleOrigin,
}

/// Joins actual mount outcomes to the switches used to build that runtime.
pub fn module_report(
    reports: &[smith_module::ModuleReport],
    switches: &BTreeMap<String, ResolvedModule>,
) -> ModulesReport {
    let mut rows = reports
        .iter()
        .map(|report| ModuleRow {
            id: report.descriptor.id.clone(),
            description: report.descriptor.description.clone(),
            state: report.state.clone(),
            source: switches.get(&report.descriptor.id).map_or_else(
                || Source::built_in(format!("modules.{}.enabled", report.descriptor.id)),
                |module| module.enabled.source.clone(),
            ),
            origin: report.descriptor.origin.clone(),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.id.cmp(&right.id));
    ModulesReport { rows }
}

/// Readable state, preserving the module's explanation.
pub fn state_label(state: &ModuleState) -> String {
    match state {
        ModuleState::Mounted => "mounted".into(),
        ModuleState::Off => "off".into(),
        ModuleState::NotBuilt => "not built".into(),
        ModuleState::Blocked {
            reason: ModuleBlockReason::Requirement(id),
        } => {
            format!("blocked · requires {id}")
        }
        ModuleState::Blocked {
            reason: ModuleBlockReason::Cycle(ids),
        } => {
            format!("blocked · requirement cycle: {}", ids.join(", "))
        }
        ModuleState::Failed { reason } => format!("failed · {reason}"),
        ModuleState::Inactive { reason } => format!("inactive · {reason}"),
    }
}

/// Explicit origin label; native third-party code is never labeled sandboxed.
pub fn origin_label(origin: &ModuleOrigin) -> String {
    match origin {
        ModuleOrigin::FirstParty => "first-party".into(),
        ModuleOrigin::ThirdParty { crate_name } => format!("third-party ({crate_name})"),
    }
}

/// Plain rendering shared by headless listing and transcript fixtures.
pub fn render_plain(report: &ModulesReport) -> String {
    report
        .rows
        .iter()
        .map(|row| {
            format!(
                "{} · {} · {}\n  {}\n  {} ← {} · {}",
                row.id,
                state_label(&row.state),
                origin_label(&row.origin),
                row.description,
                row.source.key,
                row.source.layer.label(),
                row.source,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests;
