//! Installed plugin switches and nonfatal reports of uninstalled names.

use std::collections::BTreeMap;
use std::path::Path;

use crate::model::{ConfigFile, PluginSection};

use super::load::{env_name, join_key, source_for};
use super::provenance::{Contribution, Layer, Overrides, Provenance, SettingValue, Source};
use super::provider::required_flag;
use super::types::{ConfigError, KnownPlugin, ResolvedPlugin, UnknownPlugin};

pub(super) fn enabled_key(name: &str) -> String {
    join_key(&["plugins", name, "enabled"])
}

pub(super) fn defaults(known: &[KnownPlugin]) -> Vec<Contribution> {
    known
        .iter()
        .map(|plugin| {
            let key = enabled_key(&plugin.name);
            Contribution {
                source: Source::built_in(&key),
                key,
                value: SettingValue::Flag(plugin.default_enabled),
            }
        })
        .collect()
}

pub(super) fn report_file_plugins(
    file: &ConfigFile,
    layer: Layer,
    path: &Path,
    known: &[KnownPlugin],
    out: &mut Vec<UnknownPlugin>,
) {
    let mut report = |plugins: &BTreeMap<String, PluginSection>, prefix: &[&str]| {
        for (name, section) in plugins {
            if !known.iter().any(|plugin| plugin.name == *name) {
                let mut parts = prefix.to_vec();
                parts.extend(["plugins", name.as_str()]);
                if section.enabled.is_some() {
                    parts.push("enabled");
                }
                out.push(UnknownPlugin {
                    name: name.clone(),
                    source: source_for(layer, Some(path), join_key(&parts)),
                });
            }
        }
    };
    report(&file.plugins, &[]);
    for (name, profile) in &file.profiles {
        report(&profile.plugins, &["profiles", name]);
    }
}

pub(super) fn report_overrides(
    overrides: &Overrides,
    layer: Layer,
    known: &[KnownPlugin],
    out: &mut Vec<UnknownPlugin>,
) {
    for name in overrides.plugins.keys() {
        if !known.iter().any(|plugin| plugin.name == *name) {
            let key = enabled_key(name);
            out.push(UnknownPlugin {
                name: name.clone(),
                source: if layer == Layer::SessionOverride {
                    Source::session(key)
                } else {
                    Source::flag(key)
                },
            });
        }
    }
}

fn plugin_for_env(name: &str, known: &[KnownPlugin]) -> Option<String> {
    known
        .iter()
        .find(|plugin| env_name(&enabled_key(&plugin.name)) == name)
        .map(|plugin| plugin.name.clone())
        .or_else(|| {
            name.strip_prefix("SMITH_PLUGINS_")
                .and_then(|rest| rest.strip_suffix("_ENABLED"))
                .filter(|name| !name.is_empty())
                .map(str::to_ascii_lowercase)
        })
}

pub(super) fn setting_for_plugin_env(name: &str, known: &[KnownPlugin]) -> Option<String> {
    plugin_for_env(name, known).map(|name| enabled_key(&name))
}

pub(super) fn report_environment(
    env: &BTreeMap<String, String>,
    known: &[KnownPlugin],
    out: &mut Vec<UnknownPlugin>,
) {
    for name in env.keys() {
        if let Some(plugin_name) = plugin_for_env(&name.to_uppercase(), known)
            && !known.iter().any(|plugin| plugin.name == plugin_name)
        {
            out.push(UnknownPlugin {
                name: plugin_name,
                source: Source::environment(name),
            });
        }
    }
}

pub(super) fn extract_plugins(
    provenance: &Provenance,
    known: &[KnownPlugin],
) -> Result<BTreeMap<String, ResolvedPlugin>, ConfigError> {
    known
        .iter()
        .map(|plugin| {
            let mut modules = crate::model::PluginModuleRestrictions::new();
            let mut ignored_module_settings = Vec::new();
            for key in provenance.keys().filter(|key| key.ends_with(".modules")) {
                let expected = join_key(&["plugins", &plugin.name, "modules"]);
                if key != expected && !key.ends_with(&format!(".{expected}")) {
                    continue;
                }
                let explanation = provenance.explain(key)?;
                for entry in explanation.overridden.iter().rev().chain(std::iter::once(
                    &super::provenance::Entry {
                        value: explanation.value.clone(),
                        source: explanation.source.clone(),
                    },
                )) {
                    match (
                        &entry.value,
                        entry.source.layer == Layer::UserFile && key == expected && entry.source.key == expected,
                    ) {
                        (SettingValue::Text(text), true) => {
                            modules = serde_json::from_str(text).map_err(|error| {
                                ConfigError::InvalidValue {
                                    source: entry.source.clone(),
                                    message: error.to_string(),
                                }
                            })?;
                        }
                        _ => ignored_module_settings.push(format!(
                            "{} ignored: module capabilities may only be narrowed in the global user plugin table",
                            entry.source
                        )),
                    }
                }
            }
            ignored_module_settings.sort();
            ignored_module_settings.dedup();
            Ok((
                plugin.name.clone(),
                ResolvedPlugin {
                    enabled: required_flag(provenance, &enabled_key(&plugin.name))?,
                    modules,
                    ignored_module_settings,
                },
            ))
        })
        .collect()
}

/// Plugin settings for management commands, without requiring a provider/model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSelection {
    /// Ordinary discovered user and project files.
    pub layout: super::Layout,
    /// Installed names and their effective enablement.
    pub plugins: BTreeMap<String, ResolvedPlugin>,
    /// Settings naming plugins that are not installed.
    pub unknown_plugins: Vec<UnknownPlugin>,
}

/// Resolves only plugin selection using the ordinary loading and profile rules.
pub fn resolve_plugin_selection(
    request: &super::ResolveRequest,
) -> Result<PluginSelection, ConfigError> {
    use super::agent::{profile_contributions, select_profile};
    use super::load::{Declarations, contributions_of, discover, env_contributions, load};

    let layout = discover(request)?;
    let mut declarations = Declarations::default();
    let mut layers = vec![defaults(&request.known_plugins)];
    let mut unknown_plugins = Vec::new();
    for file in &layout.files {
        let loaded = load(&file.path, file.layer)?;
        report_file_plugins(
            &loaded.file,
            file.layer,
            &file.path,
            &request.known_plugins,
            &mut unknown_plugins,
        );
        declarations.absorb(&loaded.file, file.layer, Some(&file.path));
        layers.push(contributions_of(
            &loaded.file,
            file.layer,
            Some(&file.path),
        )?);
    }
    let env = env_contributions(&request.env, &request.known_modules, &request.known_plugins)?;
    let cli = request.cli.contributions(Layer::CommandLine);
    let session = request.session.contributions(Layer::SessionOverride);
    let selected = select_profile(&layers, &env, &cli, &session, &declarations)?;
    let profile = selected
        .as_ref()
        .map(|selected| profile_contributions(&layers, &selected.value, &declarations))
        .transpose()?
        .unwrap_or_default();
    let mut provenance = Provenance::default();
    for layer in layers {
        provenance.extend(layer);
    }
    provenance.extend(profile);
    provenance.extend(env);
    provenance.extend(cli);
    provenance.extend(session);
    report_environment(&request.env, &request.known_plugins, &mut unknown_plugins);
    report_overrides(
        &request.cli,
        Layer::CommandLine,
        &request.known_plugins,
        &mut unknown_plugins,
    );
    report_overrides(
        &request.session,
        Layer::SessionOverride,
        &request.known_plugins,
        &mut unknown_plugins,
    );
    Ok(PluginSelection {
        layout,
        plugins: extract_plugins(&provenance, &request.known_plugins)?,
        unknown_plugins,
    })
}
