//! The explicit build catalog and resolved module composition.

#[cfg(any(
    feature = "module-image-generation",
    feature = "module-budget-notice",
    feature = "wasm-modules"
))]
use std::sync::Arc;

use smith_config::resolve::{KnownModule, ResolvedConfig};
#[cfg(any(
    not(feature = "module-image-generation"),
    not(feature = "module-budget-notice")
))]
use smith_module::ModuleDescriptor;
use smith_module::{CompiledModule, ModuleComposition, ModuleOrigin, SettingValue};

/// The single compiled-in list, with one feature gate per first-party module.
fn compiled_modules() -> Vec<CompiledModule> {
    Vec::from([
        #[cfg(feature = "module-image-generation")]
        CompiledModule {
            module: Arc::new(smith_module_image_generation::ImageGenerationModule),
            origin: ModuleOrigin::FirstParty,
        },
        #[cfg(feature = "module-budget-notice")]
        CompiledModule {
            module: Arc::new(smith_module_budget_notice::BudgetNoticeModule),
            origin: ModuleOrigin::FirstParty,
        },
    ])
}

pub(super) fn composition() -> ModuleComposition {
    let known = Vec::from([
        #[cfg(not(feature = "module-image-generation"))]
        ModuleDescriptor {
            id: "image-generation".into(),
            description: "Generate and edit images through the active provider".into(),
            default_enabled: true,
            origin: ModuleOrigin::FirstParty,
            compiled_in: false,
        },
        #[cfg(not(feature = "module-budget-notice"))]
        ModuleDescriptor {
            id: "budget-notice".into(),
            description: "Warn when conversation context nears the summary boundary".into(),
            default_enabled: true,
            origin: ModuleOrigin::FirstParty,
            compiled_in: false,
        },
    ]);
    ModuleComposition::with_defaults(compiled_modules(), known)
}

/// Configuration metadata comes from the same catalog used for mounting.
pub(super) fn known_modules() -> Vec<KnownModule> {
    let modules = composition();
    modules
        .compiled
        .iter()
        .map(CompiledModule::descriptor)
        .chain(modules.known)
        .map(|descriptor| KnownModule {
            legacy_enabled_key: (descriptor.id == "image-generation")
                .then(|| "tools.image_generation.enabled".into()),
            id: descriptor.id,
            default_enabled: descriptor.default_enabled,
            compiled_in: descriptor.compiled_in,
        })
        .collect()
}

/// Project effective switches and feature settings into the mount input.
pub(super) fn composition_from_resolved_config(config: &ResolvedConfig) -> ModuleComposition {
    let mut modules = composition();
    modules.enabled = config
        .modules
        .iter()
        .filter(|(_, module)| module.enabled.value)
        .map(|(id, _)| id.clone())
        .collect();
    modules.settings.insert(
        "image-generation".into(),
        [
            (
                "model".into(),
                SettingValue::String(config.image_generation.model.value.clone()),
            ),
            (
                "quality".into(),
                SettingValue::String(config.image_generation.quality.value.clone()),
            ),
            (
                "size".into(),
                SettingValue::String(config.image_generation.size.value.clone()),
            ),
        ]
        .into(),
    );
    modules
}

mod switch;
pub(crate) use switch::{commit_switch, prepare_switch, switch_outcome};

#[cfg(test)]
pub(crate) mod tests;

#[cfg(feature = "wasm-modules")]
type SessionHealthMap = std::collections::BTreeMap<String, Arc<smith_module::ModuleHealth>>;

/// Session failure counters retained across safe-boundary rebuilds.
#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleSessions {
    #[cfg(feature = "wasm-modules")]
    health: Arc<std::sync::Mutex<SessionHealthMap>>,
}

pub(crate) struct PluginModules {
    pub(crate) composition: ModuleComposition,
    pub(crate) reports: Vec<smith_module::ModuleReport>,
}

pub(crate) fn plugin_composition(
    entries: &[crate::plugins::PluginEntry],
    user_root: &std::path::Path,
    switches: &std::collections::BTreeMap<String, smith_config::resolve::ResolvedPlugin>,
    module_switches: &std::collections::BTreeMap<String, smith_config::resolve::ResolvedModule>,
    sessions: &ModuleSessions,
) -> anyhow::Result<PluginModules> {
    let mut composition = ModuleComposition::default();
    let mut reports = Vec::new();
    #[cfg(feature = "wasm-modules")]
    let mut pending = Vec::new();
    #[cfg(feature = "wasm-modules")]
    let engine = std::sync::OnceLock::new();
    for entry in entries {
        let Some(inventory) = &entry.inventory else {
            continue;
        };
        for component in &inventory.modules {
            let restriction = switches
                .get(&entry.installed.name)
                .and_then(|plugin| plugin.modules.get(&component.id));
            // Only the digest-verified install record supplies grants. User
            // settings intersect that authority; no other layer can add to it.
            let confirmed = entry
                .installed
                .module_grants
                .get(&component.id)
                .cloned()
                .unwrap_or_default();
            let granted = crate::plugins::wasm::narrow(confirmed, restriction);
            let requested_labels = component.capabilities.labels();
            let labels = granted.labels();
            let origin = ModuleOrigin::Sandboxed {
                plugin: entry.installed.name.clone(),
                requested_capabilities: capability_categories(&component.capabilities),
                granted_capabilities: capability_categories(&granted),
                capability_details: labels.clone(),
                withheld_capabilities: requested_labels
                    .into_iter()
                    .filter(|label| !labels.contains(label))
                    .collect(),
            };
            let selected = module_switches
                .get(&component.id)
                .is_none_or(|module| module.enabled.value);
            if selected {
                composition.enabled.insert(component.id.clone());
            }
            #[cfg(not(feature = "wasm-modules"))]
            {
                let _ = (user_root, sessions);
                let descriptor = smith_module::ModuleDescriptor {
                    id: component.id.clone(),
                    description: format!("{} ({})", component.wasm, entry.installed.name),
                    default_enabled: true,
                    origin,
                    compiled_in: false,
                };
                reports.push(smith_module::ModuleReport {
                    descriptor: descriptor.clone(),
                    state: smith_module::ModuleState::NotBuilt,
                    health: None,
                });
                composition.known.push(descriptor);
            }
            #[cfg(feature = "wasm-modules")]
            {
                let key = format!("{}/{}", component.id, inventory.digest);
                let health = sessions
                    .health
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .entry(key)
                    .or_default()
                    .clone();
                let loaded = (|| -> Result<Arc<smith_wasm_host::WasmModule>, String> {
                    if !selected {
                        return Err("module is off".into());
                    }
                    if entry.status != crate::plugins::PluginStatus::On {
                        return Err(format!("plugin is {:?}", entry.status));
                    }
                    let smith_plugin::InstalledState::Unchanged(current) =
                        crate::plugins::open_store(user_root)
                            .load(&entry.installed.name)
                            .map_err(|error| error.to_string())?
                    else {
                        return Err("plugin content changed; confirm the plugin again".into());
                    };
                    if current.digest != inventory.digest {
                        return Err("plugin changed during discovery".into());
                    }
                    let bytes = std::fs::read(&component.path)
                        .map_err(|_| "module bytes unavailable".to_owned())?;
                    if smith_config::trust::ContentDigest::of(&bytes) != component.digest {
                        return Err("module bytes changed since confirmation".into());
                    }
                    // Re-inspect after reading executable bytes to detect a concurrent
                    // replacement of the manifest or component during discovery.
                    if smith_plugin::inspect(&inventory.root, &entry.installed.name)
                        .map_err(|error| error.to_string())?
                        .digest
                        != inventory.digest
                    {
                        return Err("plugin changed while reading module bytes".into());
                    }
                    smith_wasm_host::WasmModule::load(
                        engine
                            .get_or_init(smith_wasm_host::WasmEngine::new)
                            .as_ref()
                            .map_err(ToString::to_string)?
                            .clone(),
                        bytes,
                        smith_wasm_host::WasmModuleConfig {
                            plugin_name: entry.installed.name.clone(),
                            requested_capabilities: host_grants(&component.capabilities),
                            granted_capabilities: host_grants(&granted),
                            host_state_dir: user_root.into(),
                            data_dir: crate::plugins::open_store(user_root)
                                .data_directory(&entry.installed.name)
                                .map_err(|error| error.to_string())?,
                            cache_dir: user_root.join(smith_wasm_host::COMPONENT_CACHE_DIRECTORY),
                            limits: smith_wasm_host::WasmLimits::default(),
                            session_health: health.clone(),
                            reserved_tool_names: smith_tools::all()
                                .iter()
                                .map(|tool| tool.spec().name)
                                .collect(),
                        },
                    )
                    .map_err(|error| error.to_string())
                })();
                let plugin_off = entry.status == crate::plugins::PluginStatus::Off;
                if plugin_off {
                    composition.enabled.remove(&component.id);
                }
                let (inner, failure) = match loaded {
                    Ok(module) => (Some(module), None),
                    Err(reason) => (None, Some(reason)),
                };
                let declared_default = inner
                    .as_ref()
                    .is_none_or(|module| smith_module::Module::default_enabled(module.as_ref()));
                let uses_default = module_switches.get(&component.id).is_none_or(|module| {
                    module.enabled.source.layer == smith_config::resolve::Layer::BuiltIn
                });
                if uses_default && !declared_default {
                    composition.enabled.remove(&component.id);
                }
                let selected = composition.enabled.contains(&component.id);
                let state = if plugin_off || !selected {
                    smith_module::ModuleState::Off
                } else if let Some(reason) = &failure {
                    smith_module::ModuleState::Failed {
                        reason: reason.clone(),
                    }
                } else {
                    smith_module::ModuleState::Inactive {
                        reason: "available; mounted when the session starts".into(),
                    }
                };
                let requirements = inner.as_ref().map_or_else(Vec::new, |module| {
                    smith_module::Module::requirement_ids(module.as_ref())
                });
                let guest_id = inner
                    .as_ref()
                    .map(|module| smith_module::Module::id(module.as_ref()).to_owned());
                pending.push((
                    origin,
                    PluginWasmModule {
                        id: component.id.clone(),
                        description: format!("{} ({})", component.wasm, entry.installed.name),
                        revision: inventory.digest.as_hex().into(),
                        inner,
                        failure,
                        health,
                        requirements,
                        guest_id,
                        plugin: entry.installed.name.clone(),
                        default_enabled: declared_default,
                    },
                    state,
                ));
            }
        }
    }
    #[cfg(feature = "wasm-modules")]
    {
        let mut aliases = std::collections::BTreeMap::new();
        let mut duplicates = std::collections::BTreeSet::new();
        for (_, module, _) in &pending {
            if let Some(guest_id) = &module.guest_id
                && aliases
                    .insert((module.plugin.clone(), guest_id.clone()), module.id.clone())
                    .is_some()
            {
                duplicates.insert((module.plugin.clone(), guest_id.clone()));
            }
        }
        for (origin, mut module, mut state) in pending {
            module.requirements = module
                .requirements
                .into_iter()
                .map(|id| {
                    aliases
                        .get(&(module.plugin.clone(), id.clone()))
                        .cloned()
                        .unwrap_or(id)
                })
                .collect();
            if module
                .guest_id
                .as_ref()
                .is_some_and(|id| duplicates.contains(&(module.plugin.clone(), id.clone())))
            {
                let reason = "duplicate guest module id in plugin".to_owned();
                module.failure = Some(reason.clone());
                state = smith_module::ModuleState::Failed { reason };
            }
            let health = module.health.clone();
            let compiled = CompiledModule {
                origin,
                module: Arc::new(module),
            };
            reports.push(smith_module::ModuleReport {
                descriptor: compiled.descriptor(),
                state,
                health: Some(health),
            });
            composition.compiled.push(compiled);
        }
    }
    Ok(PluginModules {
        composition,
        reports,
    })
}

fn capability_categories(
    caps: &smith_plugin::ModuleCapabilities,
) -> std::collections::BTreeSet<smith_module::ModuleCapability> {
    use smith_module::ModuleCapability as C;
    [
        (caps.workspace_read, C::WorkspaceRead),
        (caps.workspace_write, C::WorkspaceWrite),
        (caps.plugin_data_read || caps.session_history, C::HostRead),
        (caps.plugin_data_write, C::HostWrite),
        (!caps.network.is_empty(), C::Network),
        (!caps.data_egress.is_empty(), C::DataEgress),
    ]
    .into_iter()
    .filter_map(|(enabled, capability)| enabled.then_some(capability))
    .collect()
}

#[cfg(feature = "wasm-modules")]
fn host_grants(caps: &smith_plugin::ModuleCapabilities) -> Vec<smith_wasm_host::CapabilityGrant> {
    use smith_module::ModuleCapability as C;
    let mut grants = Vec::new();
    for (enabled, capability, root) in [
        (caps.workspace_read, C::WorkspaceRead, "workspace"),
        (caps.workspace_write, C::WorkspaceWrite, "workspace"),
        (caps.plugin_data_read, C::HostRead, "plugin-data"),
        (caps.plugin_data_write, C::HostWrite, "plugin-data"),
        (caps.session_history, C::HostRead, "session-history"),
    ] {
        if enabled {
            grants.push(smith_wasm_host::CapabilityGrant {
                capability,
                roots: vec![root.into()],
                hosts: vec![],
                allow_http: false,
            });
        }
    }
    for (capability, hosts) in [
        (C::Network, &caps.network),
        (C::DataEgress, &caps.data_egress),
    ] {
        for host in hosts {
            grants.push(smith_wasm_host::CapabilityGrant {
                capability,
                roots: vec![],
                hosts: vec![host.clone()],
                allow_http: host.starts_with("http://"),
            });
        }
    }
    grants
}

#[cfg(feature = "wasm-modules")]
#[derive(Debug)]
struct PluginWasmModule {
    id: String,
    description: String,
    revision: String,
    inner: Option<Arc<smith_wasm_host::WasmModule>>,
    failure: Option<String>,
    health: Arc<smith_module::ModuleHealth>,
    requirements: Vec<String>,
    guest_id: Option<String>,
    plugin: String,
    default_enabled: bool,
}
#[cfg(feature = "wasm-modules")]
impl smith_module::Module for PluginWasmModule {
    fn id(&self) -> &str {
        &self.id
    }
    fn revision(&self) -> &str {
        &self.revision
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn default_enabled(&self) -> bool {
        self.default_enabled
    }
    fn requirement_ids(&self) -> Vec<String> {
        self.requirements.clone()
    }
    fn session_health(&self) -> Option<Arc<smith_module::ModuleHealth>> {
        Some(self.health.clone())
    }
    fn mount(
        &self,
        context: &smith_module::ModuleContext,
    ) -> Result<smith_module::Mounted, smith_module::ModuleError> {
        if let Some(reason) = self
            .health
            .failure()
            .filter(|_| !context.is_child_session)
            .or_else(|| self.failure.clone())
        {
            return Err(smith_module::ModuleError(reason));
        }
        self.inner
            .as_ref()
            .ok_or_else(|| smith_module::ModuleError("module unavailable".into()))?
            .mount(context)
    }
}
