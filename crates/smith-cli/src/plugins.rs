//! User-owned plugin admission and the inputs the ordinary catalogs consume.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use sha2::{Digest, Sha256};
use smith_client::file_commands::CommandCatalog;
use smith_config::resolve::{
    KnownPlugin, Layer, ResolveRequest, ResolvedConfig, ResolvedMcpServer, Source,
};
use smith_config::trust::{
    ContentDigest, Executable, ExecutableKind, TrustDecision, TrustStatus, TrustStore,
};
use smith_plugin::{InstalledPlugin, InstalledState, PathVariables, PluginInventory, PluginStore};
use smith_runtime::harness::{
    CapabilitySet, Contribution, ModuleId, ModuleProvenance, ModuleRevision, ModuleSpec,
    ModuleTrust,
};
use smith_runtime::skills::SmithSkillSources;

/// Status kept for the plugin management surface; only `On` contributes content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginStatus {
    On,
    Off,
    Changed,
    Untrusted(TrustStatus),
    Failed(String),
}

/// One installed record and its inspected content, when inspection succeeded.
#[derive(Debug, Clone)]
pub(crate) struct PluginEntry {
    pub(crate) installed: InstalledPlugin,
    pub(crate) inventory: Option<PluginInventory>,
    pub(crate) status: PluginStatus,
}

/// Read-only catalog; discovering it never installs, updates, or trusts anything.
#[derive(Debug, Clone)]
pub(crate) struct PluginCatalog {
    user_root: PathBuf,
    entries: Vec<PluginEntry>,
}

/// Opens the plugin store at the same state root as configuration and trust.
pub(crate) fn open_store(user_root: &Path) -> PluginStore {
    PluginStore::new(user_root)
}

/// The install decision is user-level, keyed by the plugin store and bundle name.
pub(crate) fn trust_status(
    user_root: &Path,
    name: &str,
    digest: &ContentDigest,
) -> Result<TrustStatus> {
    let trust = TrustStore::open(user_root)?;
    Ok(trust.status(&user_root.join("plugins"), &executable(name, digest))?)
}

/// Persists an explicitly confirmed decision, refusing a stale review digest.
/// The task-5 command must call this only after confirmation or explicit `--yes`.
pub(crate) fn record_trust(
    user_root: &Path,
    name: &str,
    digest: &ContentDigest,
    decision: TrustDecision,
) -> Result<()> {
    let InstalledState::Unchanged(inventory) = open_store(user_root).load(name)? else {
        anyhow::bail!("plugin `{name}` changed; inspect and confirm its content again");
    };
    if &inventory.digest != digest {
        anyhow::bail!("plugin `{name}` changed during confirmation");
    }
    TrustStore::open(user_root)?.record(
        &user_root.join("plugins"),
        &executable(name, digest),
        decision,
    )?;
    Ok(())
}

fn executable(name: &str, digest: &ContentDigest) -> Executable {
    Executable::new(ExecutableKind::Extension, name, digest.clone())
}

/// Supplies frozen model metadata and every installed plugin to a CLI request.
pub(crate) fn with_known_plugins(request: ResolveRequest) -> Result<ResolveRequest> {
    let user_root = match &request.home_dir {
        Some(home) => home.join(".smith"),
        None => smith_config::credential::user_state_root()?,
    };
    let catalog = PluginCatalog::discover(&user_root)?;
    let mut request = request;
    if request.catalog.is_none() {
        request = request.with_catalog(
            smith_runtime::model_catalog::CatalogLoader::production(&user_root)?.snapshot()?,
        );
    }
    request.known_modules.extend(
        catalog
            .entries
            .iter()
            .filter_map(|entry| entry.inventory.as_ref())
            .flat_map(|inventory| inventory.modules.iter())
            .map(|module| smith_config::resolve::KnownModule {
                id: module.id.clone(),
                default_enabled: true,
                compiled_in: cfg!(feature = "wasm-modules"),
                legacy_enabled_key: None,
            }),
    );
    Ok(request.with_known_plugins(catalog.known_plugins()))
}

impl PluginCatalog {
    pub(crate) fn discover(user_root: &Path) -> Result<Self> {
        let store = open_store(user_root);
        let mut entries = Vec::new();
        for installed in store.list()? {
            let (inventory, status) = match store.load(&installed.name)? {
                InstalledState::Unchanged(inventory) => {
                    let status = match trust_status(user_root, &installed.name, &installed.digest) {
                        Ok(TrustStatus::Trusted) => PluginStatus::On,
                        Ok(TrustStatus::Changed) => PluginStatus::Changed,
                        Ok(status) => PluginStatus::Untrusted(status),
                        Err(error) => PluginStatus::Failed(error.to_string()),
                    };
                    (Some(inventory), status)
                }
                InstalledState::Changed { inventory, .. } => {
                    (Some(inventory), PluginStatus::Changed)
                }
                InstalledState::Failed { reason, .. } => (None, PluginStatus::Failed(reason)),
            };
            entries.push(PluginEntry {
                installed,
                inventory,
                status,
            });
        }
        Ok(Self {
            user_root: user_root.into(),
            entries,
        })
    }

    pub(crate) fn known_plugins(&self) -> Vec<KnownPlugin> {
        self.entries
            .iter()
            .map(|entry| KnownPlugin {
                name: entry.installed.name.clone(),
                default_enabled: entry
                    .inventory
                    .as_ref()
                    .is_none_or(|inventory| inventory.manifest.default_enabled),
            })
            .collect()
    }

    /// Resolves enablement only after digest verification and the trust decision.
    pub(crate) fn resolve(
        mut self,
        config: &ResolvedConfig,
        project: &Path,
    ) -> Result<ActivePlugins> {
        let mut active = Vec::new();
        let mut registry_names = config.mcp.servers.keys().cloned().collect::<BTreeSet<_>>();
        for entry in &mut self.entries {
            let enabled = config.plugins.get(&entry.installed.name).map_or_else(
                || {
                    entry
                        .inventory
                        .as_ref()
                        .is_none_or(|inventory| inventory.manifest.default_enabled)
                },
                |plugin| plugin.enabled.value,
            );
            if entry.status == PluginStatus::On {
                if enabled {
                    let mut inventory = entry
                        .inventory
                        .clone()
                        .expect("admitted content was inspected");
                    let data = open_store(&self.user_root).data_directory(&entry.installed.name)?;
                    let mut plugin = ActivePlugin {
                        inventory: inventory.clone(),
                        data,
                        project: project.into(),
                        servers: BTreeMap::new(),
                    };
                    for declaration in inventory.resolved_mcp(plugin.paths()) {
                        match resolved_server(config, &inventory, &declaration) {
                            Ok(server) => {
                                if registry_names
                                    .insert(smith_config::mcp::registry_server_name(&server.name))
                                {
                                    plugin.servers.insert(server.name.clone(), server);
                                } else {
                                    inventory.not_loaded.push(smith_plugin::NotLoadedComponent {
                                        kind: "mcpServers".into(),
                                        name: declaration.name,
                                        path: None,
                                        reason:
                                            "MCP registry identity conflicts with another server"
                                                .into(),
                                    });
                                }
                            }
                            Err(error) => {
                                inventory.not_loaded.push(smith_plugin::NotLoadedComponent {
                                    kind: "mcpServers".into(),
                                    name: declaration.name,
                                    path: None,
                                    reason: error.to_string(),
                                });
                            }
                        }
                    }
                    let commands = CommandCatalog::empty().with_plugin(&inventory, plugin.paths());
                    for problem in commands.problems() {
                        if !inventory.not_loaded.iter().any(|item| {
                            item.kind == "commands"
                                && item.name == problem.name
                                && item.reason == problem.reason
                        }) {
                            inventory.not_loaded.push(smith_plugin::NotLoadedComponent {
                                kind: "commands".into(),
                                name: problem.name.clone(),
                                path: None,
                                reason: problem.reason.clone(),
                            });
                        }
                    }
                    inventory
                        .commands
                        .retain(|command| commands.resolve(&command.name).is_some());
                    inventory
                        .mcp_servers
                        .retain(|server| plugin.servers.contains_key(&server.name));
                    plugin.inventory = inventory.clone();
                    entry.inventory = Some(inventory);
                    active.push(plugin);
                } else {
                    entry.status = PluginStatus::Off;
                }
            }
        }
        Ok(ActivePlugins {
            entries: self.entries,
            user_root: self.user_root,
            module_switches: config.modules.clone(),
            active,
        })
    }
}

#[derive(Debug, Clone)]
struct ActivePlugin {
    inventory: PluginInventory,
    data: PathBuf,
    project: PathBuf,
    servers: BTreeMap<String, ResolvedMcpServer>,
}

impl ActivePlugin {
    fn paths(&self) -> PathVariables<'_> {
        PathVariables {
            plugin_root: &self.inventory.root,
            plugin_data: &self.data,
            project_dir: &self.project,
        }
    }
}

fn resolved_server(
    config: &ResolvedConfig,
    inventory: &PluginInventory,
    declaration: &smith_plugin::McpDeclaration,
) -> Result<ResolvedMcpServer> {
    let registry = smith_config::mcp::registry_server_name(&declaration.name);
    if config.mcp.servers.contains_key(&registry) {
        anyhow::bail!("plugin MCP registry name `{registry}` conflicts with configured server");
    }
    let source = Source::file(
        Layer::UserFile,
        &inventory.root,
        format!(
            "plugins.{}.mcp.{}",
            inventory.manifest.name, declaration.name
        ),
    );
    let section = smith_config::model::McpServerSection {
        command: declaration.command.clone(),
        args: declaration.args.clone(),
        env: declaration.env.clone(),
        url: declaration.url.clone(),
        headers: declaration.headers.clone(),
        ..smith_config::model::McpServerSection::default()
    };
    Ok(smith_config::mcp::resolve_plugin_server(
        &declaration.name,
        &section,
        source,
    )?)
}

/// The exact active set for one resolved profile and safe-boundary composition.
#[derive(Debug, Clone)]
pub(crate) struct ActivePlugins {
    pub(crate) entries: Vec<PluginEntry>,
    active: Vec<ActivePlugin>,
    user_root: PathBuf,
    module_switches: BTreeMap<String, smith_config::resolve::ResolvedModule>,
}

impl ActivePlugins {
    pub(crate) fn skills(&self, mut sources: SmithSkillSources) -> Result<SmithSkillSources> {
        for plugin in &self.active {
            for component in &plugin.inventory.skills {
                // Verify the bytes inspected by the bundle scanner before pinning
                // the ordinary lazy skill. Activation rechecks this raw-byte pin.
                let bytes = std::fs::read(&component.path)?;
                if ContentDigest::of(&bytes) != component.digest {
                    anyhow::bail!("plugin skill `{}` changed during discovery", component.name);
                }
                let skill =
                    smith_runtime::skills::plugin_skill(component, Sha256::digest(&bytes).into());
                sources = sources.with_plugin(skill, plugin.paths());
            }
        }
        Ok(sources)
    }

    pub(crate) fn commands(&self, mut catalog: CommandCatalog) -> CommandCatalog {
        for entry in &self.entries {
            let name = &entry.installed.name;
            let reason = match &entry.status {
                PluginStatus::On => continue,
                PluginStatus::Off => format!(
                    "plugin `{name}` is off; run `/plugins {name} on` or set `plugins.{name}.enabled = true`"
                ),
                PluginStatus::Changed => format!(
                    "plugin `{name}` content changed; run `smith plugin update {name}` to review and confirm it again"
                ),
                PluginStatus::Untrusted(status) => format!(
                    "plugin `{name}` bundle trust is {status:?}; run `smith plugin update {name}` to review and confirm it"
                ),
                PluginStatus::Failed(reason) => format!(
                    "plugin `{name}` failed: {}; run `smith plugin update {name}` to repair and confirm it",
                    reason.chars().take(200).collect::<String>()
                ),
            };
            catalog = catalog.with_unavailable_plugin(name, reason);
        }
        for plugin in &self.active {
            catalog = catalog.with_plugin(&plugin.inventory, plugin.paths());
        }
        catalog
    }

    pub(crate) fn mcp_servers(&self) -> BTreeMap<String, ResolvedMcpServer> {
        self.active
            .iter()
            .flat_map(|plugin| plugin.servers.clone())
            .collect()
    }

    /// Content evidence grants no capabilities. MCP evidence is supplied by
    /// the supervisor through the existing harness MCP-record path.
    pub(crate) fn modules(&self) -> Result<Vec<ModuleSpec>> {
        self.active
            .iter()
            .map(|plugin| {
                let inventory = &plugin.inventory;
                let name = &inventory.manifest.name;
                let identity = ContentDigest::of(name.as_bytes());
                Ok(ModuleSpec {
                    id: ModuleId::parse(format!("plugins/{}", &identity.as_hex()[..40]))?,
                    revision: ModuleRevision::parse(inventory.digest.as_hex())?,
                    provenance: ModuleProvenance::UserManifest(name.clone()),
                    trust: ModuleTrust::ContentOnly,
                    contributions: inventory
                        .skills
                        .iter()
                        .map(|skill| Contribution::Skill {
                            name: skill.name.clone(),
                        })
                        .chain(
                            inventory
                                .commands
                                .iter()
                                .map(|command| Contribution::Command {
                                    name: command.name.clone(),
                                }),
                        )
                        .collect(),
                    requested_capabilities: CapabilitySet::new(),
                    granted_capabilities: CapabilitySet::new(),
                })
            })
            .collect()
    }

    pub(crate) fn profile(
        &self,
        config: &smith_config::resolve::ResolvedConfig,
        user_dir: &std::path::Path,
        project: &std::path::Path,
        credentials: Option<smith_config::credential::CredentialResolver>,
        shared: Option<&Arc<crate::mcp::McpContext>>,
    ) -> Result<smith_runtime::factory::PluginProfile> {
        let servers = self.mcp_servers();
        let supervisor = match shared {
            Some(context) if context.matches(config, &servers) => Some(context.supervisor()),
            _ => {
                let mut plugin_config = config.clone();
                plugin_config.mcp.servers.clear();
                crate::mcp::McpContext::start_with_plugins(
                    &plugin_config,
                    user_dir,
                    project,
                    credentials,
                    servers.clone(),
                )?
                .map(|context| context.supervisor())
            }
        };
        Ok(smith_runtime::factory::PluginProfile {
            skills: self.skills(smith_runtime::skills::SmithSkillSources::new())?,
            mcp: supervisor,
            servers: servers.into_keys().collect(),
            modules: self.modules()?,
            wasm_modules: Default::default(),
        })
    }

    /// Headless and terminal startup share a bounded, secret-free diagnostic.
    pub(crate) fn diagnostics(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let reason = match &entry.status {
                    PluginStatus::On | PluginStatus::Off => return None,
                    PluginStatus::Changed => "content changed; confirm the plugin again".into(),
                    PluginStatus::Untrusted(status) => format!("bundle trust is {status:?}"),
                    PluginStatus::Failed(reason) => reason.chars().take(200).collect(),
                };
                Some(format!(
                    "smith: plugin `{}` withheld: {reason}",
                    entry.installed.name
                ))
            })
            .collect()
    }
}

mod report;
mod switch;
pub(crate) use report::{inventory_components, mcp_target, source_label};
pub(crate) use switch::{commit_switch, prepare_switch, switch_outcome};

pub(crate) mod wasm;
impl ActivePlugins {
    pub(crate) fn wasm_modules(
        &self,
        config: &ResolvedConfig,
        sessions: &crate::modules::ModuleSessions,
    ) -> Result<smith_module::ModuleComposition> {
        crate::modules::plugin_composition(
            &self.entries,
            &self.user_root,
            &config.plugins,
            &config.modules,
            sessions,
        )
        .map(|modules| modules.composition)
    }
}
