//! Module switches and legacy aliases over the ordinary contribution ledger.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::model::{ConfigFile, ModuleSection};

use super::load::{env_name, join_key, source_for};
use super::provenance::{Contribution, Layer, Overrides, Provenance, SettingValue, Source};
use super::provider::required_flag;
use super::types::{ConfigError, KnownModule, ResolvedModule};

pub(super) fn enabled_key(id: &str) -> String {
    join_key(&["modules", id, "enabled"])
}

pub(super) fn unknown_module(id: &str, source: Source, known: &[KnownModule]) -> ConfigError {
    let mut known_ids: Vec<String> = known.iter().map(|module| module.id.clone()).collect();
    known_ids.sort();
    known_ids.dedup();
    ConfigError::UnknownModule {
        id: id.to_owned(),
        source,
        known_ids,
    }
}

pub(super) fn validate_file_modules(
    file: &ConfigFile,
    layer: Layer,
    path: &Path,
    known: &[KnownModule],
) -> Result<(), ConfigError> {
    let validate = |modules: &BTreeMap<String, ModuleSection>, prefix: &[&str]| {
        for id in modules.keys() {
            if !known.iter().any(|module| module.id == *id) {
                let mut parts = prefix.to_vec();
                parts.extend(["modules", id.as_str()]);
                return Err(unknown_module(
                    id,
                    source_for(layer, Some(path), join_key(&parts)),
                    known,
                ));
            }
        }
        Ok(())
    };
    validate(&file.modules, &[])?;
    for (name, profile) in &file.profiles {
        validate(&profile.modules, &["profiles", name])?;
    }
    Ok(())
}

pub(super) fn validate_overrides(
    overrides: &Overrides,
    layer: Layer,
    known: &[KnownModule],
) -> Result<(), ConfigError> {
    for id in overrides.modules.keys() {
        if !known.iter().any(|module| module.id == *id) {
            let key = enabled_key(id);
            let source = if layer == Layer::SessionOverride {
                Source::session(key)
            } else {
                Source::flag(key)
            };
            return Err(unknown_module(id, source, known));
        }
    }
    Ok(())
}

pub(super) fn defaults(known: &[KnownModule]) -> Vec<Contribution> {
    known
        .iter()
        .filter(|module| module.legacy_enabled_key.is_none())
        .map(|module| {
            let key = enabled_key(&module.id);
            Contribution {
                source: Source::built_in(&key),
                key,
                value: SettingValue::Flag(module.default_enabled),
            }
        })
        .collect()
}

/// Preserve a ported switch's computed default before mirroring its history.
pub(super) fn legacy_defaults(provenance: &mut Provenance, known: &[KnownModule]) {
    for module in known {
        let Some(alias) = &module.legacy_enabled_key else {
            continue;
        };
        let key = enabled_key(&module.id);
        if provenance.explain(&key).is_ok_and(|explanation| {
            explanation.source.layer == Layer::BuiltIn
                || explanation
                    .overridden
                    .iter()
                    .any(|entry| entry.source.layer == Layer::BuiltIn)
        }) {
            continue;
        }
        let default = provenance.winner(alias).cloned();
        provenance.prepend(Contribution {
            key: key.clone(),
            value: default
                .as_ref()
                .map_or(SettingValue::Flag(module.default_enabled), |entry| {
                    entry.value.clone()
                }),
            source: default.map_or_else(|| Source::built_in(key), |entry| entry.source),
        });
    }
}

/// Canonicalize before profiles are lifted so alternate spellings in different
/// files follow file precedence, while two spellings in one file must agree.
/// Source keys stay untouched, including their profile prefix.
pub(super) fn normalize_aliases(
    out: &mut [Contribution],
    known: &[KnownModule],
) -> Result<(), ConfigError> {
    let mut seen: BTreeMap<(Layer, Option<PathBuf>, String), (&SettingValue, Source)> =
        BTreeMap::new();
    for contribution in out.iter() {
        let Some(canonical) = canonical_key(&contribution.key, known) else {
            continue;
        };
        let identity = (
            contribution.source.layer,
            contribution.source.file.clone(),
            canonical,
        );
        if let Some((previous, source)) = seen.get(&identity) {
            if *previous != &contribution.value {
                return Err(ConfigError::Ambiguous {
                    key: identity.2,
                    sources: vec![source.clone(), contribution.source.clone()],
                });
            }
        } else {
            seen.insert(identity, (&contribution.value, contribution.source.clone()));
        }
    }
    for contribution in out {
        if let Some(canonical) = canonical_key(&contribution.key, known) {
            contribution.key = canonical;
        }
    }
    Ok(())
}

fn canonical_key(key: &str, known: &[KnownModule]) -> Option<String> {
    for module in known {
        let canonical = enabled_key(&module.id);
        for spelling in
            std::iter::once(canonical.as_str()).chain(module.legacy_enabled_key.as_deref())
        {
            if key == spelling {
                return Some(canonical);
            }
            if let Some(prefix) = key.strip_suffix(&format!(".{spelling}")) {
                return Some(format!("{prefix}.{canonical}"));
            }
        }
    }
    None
}

pub(super) fn setting_for_module_env(name: &str, known: &[KnownModule]) -> Option<String> {
    known
        .iter()
        .map(|module| enabled_key(&module.id))
        .find(|key| env_name(key) == name)
}

/// Both lookups expose the same history, not merely the same final boolean.
pub(super) fn mirror_aliases(provenance: &mut Provenance, known: &[KnownModule]) {
    for module in known {
        if let Some(alias) = &module.legacy_enabled_key {
            provenance.mirror(&enabled_key(&module.id), alias);
        }
    }
}

pub(super) fn extract_modules(
    provenance: &Provenance,
    known: &[KnownModule],
) -> Result<BTreeMap<String, ResolvedModule>, ConfigError> {
    known
        .iter()
        .map(|module| {
            Ok((
                module.id.clone(),
                ResolvedModule {
                    enabled: required_flag(provenance, &enabled_key(&module.id))?,
                    compiled_in: module.compiled_in,
                },
            ))
        })
        .collect()
}
