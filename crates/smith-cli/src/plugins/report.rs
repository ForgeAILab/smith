//! Shared bundle inventory and installed-state reports.

use std::collections::BTreeMap;

use smith_client::plugins_report::{
    PluginComponent, PluginRow, PluginState, PluginsReport, UnknownPluginRow,
};
use smith_config::resolve::{ResolvedPlugin, UnknownPlugin};
use smith_config::trust::TrustStatus;
use smith_plugin::{McpDeclaration, PathVariables, PluginInventory, PluginSource};

use super::{ActivePlugins, PluginCatalog, PluginStatus, open_store};

pub(crate) fn source_label(source: &PluginSource) -> String {
    match source {
        PluginSource::Local { path } => path.display().to_string(),
        PluginSource::Git {
            url,
            reference,
            subdirectory,
        } => format!(
            "{url}{}{}",
            reference
                .as_ref()
                .map_or_else(String::new, |reference| format!("#{reference}")),
            subdirectory
                .as_ref()
                .map_or_else(String::new, |path| format!(" ({path})"))
        ),
        PluginSource::Marketplace {
            plugin,
            marketplace,
        } => format!("{plugin}@{marketplace}"),
    }
}

pub(crate) fn mcp_target(server: &McpDeclaration) -> String {
    server.url.clone().unwrap_or_else(|| {
        std::iter::once(server.command.as_deref().unwrap_or(""))
            .chain(server.args.iter().map(String::as_str))
            .map(|part| serde_json::to_string(part).expect("string is JSON encodable"))
            .collect::<Vec<_>>()
            .join(" ")
    })
}

pub(crate) fn inventory_components(
    inventory: &PluginInventory,
    paths: PathVariables<'_>,
) -> (Vec<PluginComponent>, Vec<PluginComponent>) {
    let commands = smith_client::file_commands::CommandCatalog::empty().with_plugin(
        inventory,
        PathVariables {
            plugin_root: &inventory.root,
            ..paths
        },
    );
    let mut loaded = inventory
        .skills
        .iter()
        .map(|skill| PluginComponent {
            kind: "skill".into(),
            name: skill.name.clone(),
            detail: None,
        })
        .chain(
            inventory
                .commands
                .iter()
                .filter(|command| commands.resolve(&command.name).is_some())
                .map(|command| PluginComponent {
                    kind: "command".into(),
                    name: command.name.clone(),
                    detail: None,
                }),
        )
        .collect::<Vec<_>>();
    let mut not_loaded = inventory
        .not_loaded
        .iter()
        .map(|component| PluginComponent {
            kind: component.kind.clone(),
            name: component.name.clone(),
            detail: Some(if component.kind == "hooks" {
                format!("will not run · {}", component.reason)
            } else {
                component.reason.clone()
            }),
        })
        .collect::<Vec<_>>();
    for problem in commands.problems() {
        if inventory
            .commands
            .iter()
            .any(|command| command.name == problem.name)
        {
            not_loaded.push(PluginComponent {
                kind: "command".into(),
                name: problem.name.clone(),
                detail: Some(problem.reason.clone()),
            });
        }
    }
    for server in inventory.resolved_mcp(paths) {
        let section = smith_config::model::McpServerSection {
            command: server.command.clone(),
            args: server.args.clone(),
            env: server.env.clone(),
            url: server.url.clone(),
            headers: server.headers.clone(),
            ..smith_config::model::McpServerSection::default()
        };
        let source =
            smith_config::resolve::Source::built_in(format!("plugin {}", inventory.manifest.name));
        match smith_config::mcp::resolve_plugin_server(&server.name, &section, source) {
            Ok(_) => loaded.push(PluginComponent {
                kind: "MCP server".into(),
                name: server.name.clone(),
                detail: Some(mcp_target(&server)),
            }),
            Err(error) => not_loaded.push(PluginComponent {
                kind: "MCP server".into(),
                name: server.name,
                detail: Some(error.to_string()),
            }),
        }
    }
    (loaded, not_loaded)
}

impl PluginCatalog {
    pub(crate) fn selection_report(
        mut self,
        switches: &BTreeMap<String, ResolvedPlugin>,
        unknown: &[UnknownPlugin],
        project: &std::path::Path,
    ) -> anyhow::Result<PluginsReport> {
        for entry in &mut self.entries {
            if entry.status == PluginStatus::On
                && switches
                    .get(&entry.installed.name)
                    .is_some_and(|plugin| !plugin.enabled.value)
            {
                entry.status = PluginStatus::Off;
            }
        }
        ActivePlugins {
            entries: self.entries,
            active: Vec::new(),
        }
        .report(&self.user_root, switches, unknown, project)
    }
}

impl ActivePlugins {
    pub(crate) fn report(
        &self,
        user_root: &std::path::Path,
        switches: &BTreeMap<String, ResolvedPlugin>,
        unknown: &[UnknownPlugin],
        project: &std::path::Path,
    ) -> anyhow::Result<PluginsReport> {
        let mut rows = Vec::new();
        for entry in &self.entries {
            let state = match &entry.status {
                PluginStatus::On => PluginState::On,
                PluginStatus::Off => PluginState::Off,
                PluginStatus::Changed => PluginState::Changed,
                PluginStatus::Untrusted(TrustStatus::Denied) => {
                    PluginState::Untrusted("approval denied".into())
                }
                PluginStatus::Untrusted(_) => {
                    PluginState::Untrusted("needs approval; run `smith plugin add`".into())
                }
                PluginStatus::Failed(reason) => PluginState::Failed(reason.clone()),
            };
            let (mut loaded, mut not_loaded) = match &entry.inventory {
                Some(inventory) => {
                    let data = open_store(user_root).data_directory(&entry.installed.name)?;
                    inventory_components(
                        inventory,
                        PathVariables {
                            plugin_root: &entry.installed.install_directory,
                            plugin_data: &data,
                            project_dir: project,
                        },
                    )
                }
                None => (Vec::new(), Vec::new()),
            };
            if state != PluginState::On {
                not_loaded.extend(loaded.drain(..).map(|mut component| {
                    component.detail = Some(state.label());
                    component
                }));
            }
            rows.push(PluginRow {
                name: entry.installed.name.clone(),
                state,
                source: source_label(&entry.installed.source),
                version: entry.installed.version.clone(),
                resolved_commit: entry.installed.resolved_commit.clone(),
                digest: entry.installed.digest.to_string(),
                enabled_source: switches.get(&entry.installed.name).map_or_else(
                    || "manifest default".into(),
                    |plugin| plugin.enabled.source.to_string(),
                ),
                loaded,
                not_loaded,
            });
        }
        Ok(PluginsReport {
            rows,
            unknown: unknown
                .iter()
                .map(|unknown| UnknownPluginRow {
                    name: unknown.name.clone(),
                    source: unknown.source.to_string(),
                })
                .collect(),
        })
    }
}
