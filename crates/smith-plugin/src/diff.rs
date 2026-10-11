//! Stable inventory comparisons before an update is committed.

use std::collections::BTreeMap;

use crate::{CommandSource, McpDeclaration, NotLoadedComponent, PluginInventory};

/// One changed component, preserving both values for a confirmation surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentChange<T> {
    /// Namespaced component name.
    pub name: String,
    /// Previously installed value.
    pub old: T,
    /// Prepared replacement value.
    pub new: T,
}

/// Added, removed, and changed components in a prepared update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InventoryDifference {
    /// Namespaced skills added.
    pub added_skills: Vec<String>,
    /// Namespaced skills removed.
    pub removed_skills: Vec<String>,
    /// Namespaced skills whose body or descriptor changed.
    pub changed_skills: Vec<String>,
    /// Namespaced commands added.
    pub added_commands: Vec<String>,
    /// Namespaced commands removed.
    pub removed_commands: Vec<String>,
    /// Namespaced commands whose body or descriptor changed.
    pub changed_commands: Vec<String>,
    /// New MCP declarations, before path substitution.
    pub added_mcp_servers: Vec<McpDeclaration>,
    /// Removed MCP declarations, before path substitution.
    pub removed_mcp_servers: Vec<McpDeclaration>,
    /// Old and new declarations, including command lines, environment and headers.
    pub changed_mcp_servers: Vec<ComponentChange<McpDeclaration>>,
    /// Added sandboxed components, with all requested capabilities.
    pub added_modules: Vec<crate::WasmModuleComponent>,
    /// Removed sandboxed components.
    pub removed_modules: Vec<crate::WasmModuleComponent>,
    /// Changed paths or capability requests, retaining both inventories.
    pub changed_modules: Vec<ComponentChange<crate::WasmModuleComponent>>,
    /// Newly present components that will not load.
    pub added_not_loaded: Vec<NotLoadedComponent>,
    /// Components no longer listed as not loaded.
    pub removed_not_loaded: Vec<NotLoadedComponent>,
}

impl InventoryDifference {
    /// Compares content identities rather than absolute installation paths.
    pub fn between(old: &PluginInventory, new: &PluginInventory) -> Self {
        let old_skills: BTreeMap<_, _> = old
            .skills
            .iter()
            .map(|s| {
                (
                    s.name.clone(),
                    (
                        &s.digest,
                        &s.description,
                        crate::manifest::relative(&old.root, &s.path).ok(),
                    ),
                )
            })
            .collect();
        let new_skills: BTreeMap<_, _> = new
            .skills
            .iter()
            .map(|s| {
                (
                    s.name.clone(),
                    (
                        &s.digest,
                        &s.description,
                        crate::manifest::relative(&new.root, &s.path).ok(),
                    ),
                )
            })
            .collect();
        let old_commands: BTreeMap<_, _> = old
            .commands
            .iter()
            .map(|s| {
                (
                    s.name.clone(),
                    (
                        &s.digest,
                        &s.description,
                        &s.argument_hint,
                        command_path(old, &s.source),
                    ),
                )
            })
            .collect();
        let new_commands: BTreeMap<_, _> = new
            .commands
            .iter()
            .map(|s| {
                (
                    s.name.clone(),
                    (
                        &s.digest,
                        &s.description,
                        &s.argument_hint,
                        command_path(new, &s.source),
                    ),
                )
            })
            .collect();
        let (added_skills, removed_skills, changed_skills) = names(&old_skills, &new_skills);
        let (added_commands, removed_commands, changed_commands) =
            names(&old_commands, &new_commands);
        let old_mcp: BTreeMap<_, _> = old
            .mcp_servers
            .iter()
            .map(|s| (s.name.clone(), s))
            .collect();
        let new_mcp: BTreeMap<_, _> = new
            .mcp_servers
            .iter()
            .map(|s| (s.name.clone(), s))
            .collect();
        let (added, removed, changed) = names(&old_mcp, &new_mcp);
        let old_modules: BTreeMap<_, _> = old.modules.iter().map(|m| (m.id.clone(), m)).collect();
        let new_modules: BTreeMap<_, _> = new.modules.iter().map(|m| (m.id.clone(), m)).collect();
        // Installation paths differ between snapshots; compare portable declarations.
        let changed_modules = new_modules
            .iter()
            .filter_map(|(id, module)| {
                let prior = old_modules.get(id)?;
                (prior.capabilities != module.capabilities
                    || prior.wasm != module.wasm
                    || prior.digest != module.digest)
                    .then(|| ComponentChange {
                        name: id.clone(),
                        old: (*prior).clone(),
                        new: (*module).clone(),
                    })
            })
            .collect();
        Self {
            added_modules: new_modules
                .iter()
                .filter(|(id, _)| !old_modules.contains_key(*id))
                .map(|(_, m)| (*m).clone())
                .collect(),
            removed_modules: old_modules
                .iter()
                .filter(|(id, _)| !new_modules.contains_key(*id))
                .map(|(_, m)| (*m).clone())
                .collect(),
            changed_modules,
            added_skills,
            removed_skills,
            changed_skills,
            added_commands,
            removed_commands,
            changed_commands,
            added_mcp_servers: added.iter().map(|name| new_mcp[name].clone()).collect(),
            removed_mcp_servers: removed.iter().map(|name| old_mcp[name].clone()).collect(),
            changed_mcp_servers: changed
                .iter()
                .map(|name| ComponentChange {
                    name: name.clone(),
                    old: old_mcp[name].clone(),
                    new: new_mcp[name].clone(),
                })
                .collect(),
            added_not_loaded: new
                .not_loaded
                .iter()
                .filter(|item| !old.not_loaded.contains(item))
                .cloned()
                .collect(),
            removed_not_loaded: old
                .not_loaded
                .iter()
                .filter(|item| !new.not_loaded.contains(item))
                .cloned()
                .collect(),
        }
    }
}

fn command_path(inventory: &PluginInventory, source: &CommandSource) -> Option<String> {
    match source {
        CommandSource::File(path) => crate::manifest::relative(&inventory.root, path).ok(),
        CommandSource::Inline(_) => None,
    }
}

fn names<T: PartialEq>(
    old: &BTreeMap<String, T>,
    new: &BTreeMap<String, T>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let added = new
        .keys()
        .filter(|key| !old.contains_key(*key))
        .cloned()
        .collect();
    let removed = old
        .keys()
        .filter(|key| !new.contains_key(*key))
        .cloned()
        .collect();
    let changed = new
        .iter()
        .filter(|(key, value)| old.get(*key).is_some_and(|prior| prior != *value))
        .map(|(key, _)| key.clone())
        .collect();
    (added, removed, changed)
}
