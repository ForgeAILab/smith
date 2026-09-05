//! Resolves the installed coding agent a profile runs its turns on.
//!
//! A harness replaces how a turn is *executed*, not how the model is
//! identified: the profile still resolves a provider and model, because the
//! runtime plans against real limits before any work runs. The CLI's own model
//! is `harness.<name>.model`, which is what the CLI is actually told to use.

use std::collections::BTreeMap;

use super::load::join_key;
use super::provenance::*;
use super::provider::{flag, list, text};
use super::types::*;

/// Harness names Smith knows how to drive.
const KNOWN_HARNESSES: [&str; 2] = ["claude-code", "codex"];

/// Reads the harness a profile selected, if any.
pub(super) fn resolve_harness(
    provenance: &Provenance,
) -> Result<Option<ResolvedHarness>, ConfigError> {
    let Some(name) = text(provenance, "harness")? else {
        return Ok(None);
    };
    if !KNOWN_HARNESSES.contains(&name.value.as_str()) {
        return Err(ConfigError::InvalidValue {
            source: name.source.clone(),
            message: format!(
                "unknown harness `{}`; Smith drives {}",
                name.value,
                KNOWN_HARNESSES.join(" or ")
            ),
        });
    }
    let scope = join_key(&["harness", &name.value]);

    let executable = text(provenance, &format!("{scope}.executable"))?.ok_or_else(|| {
        ConfigError::InvalidValue {
            source: name.source.clone(),
            message: format!(
                "harness `{}` needs `{scope}.executable`, an absolute path to the installed CLI",
                name.value
            ),
        }
    })?;
    owner_only(&executable.source, &format!("{scope}.executable"))?;
    if !std::path::Path::new(&executable.value).is_absolute() {
        return Err(ConfigError::InvalidValue {
            source: executable.source.clone(),
            message: format!(
                "`{scope}.executable` must be an absolute path, invoked without a shell"
            ),
        });
    }

    let model = text(provenance, &format!("{scope}.model"))?;

    let args = match list(provenance, &format!("{scope}.args"))? {
        Some(args) => {
            owner_only(&args.source, &format!("{scope}.args"))?;
            args.value
        }
        None => Vec::new(),
    };

    // Absent means off. A CLI running its own tools executes reads, writes,
    // and commands Smith never approved, never scoped to the workspace, and
    // cannot record as tool history, so it is never on by default and never
    // enabled by a project.
    let allow_own_tools = match flag(provenance, &format!("{scope}.allow_own_tools"))? {
        Some(flag) => {
            owner_only(&flag.source, &format!("{scope}.allow_own_tools"))?;
            flag
        }
        None => Sourced::new(false, Source::built_in(format!("{scope}.allow_own_tools"))),
    };

    let env = resolve_env(provenance, &scope)?;

    Ok(Some(ResolvedHarness {
        name,
        executable,
        model,
        args,
        allow_own_tools,
        env,
    }))
}

/// Refuses a process-bearing value written by a project layer.
///
/// A project may select a harness, because selecting one changes only which
/// installed program Smith asks to work. It may not say what gets executed,
/// with which arguments, or whether that program may write to the machine —
/// the same boundary command providers already enforce.
fn owner_only(source: &Source, key: &str) -> Result<(), ConfigError> {
    if matches!(source.layer, Layer::ProjectFile | Layer::ProjectLocalFile) {
        return Err(ConfigError::InvalidValue {
            source: source.clone(),
            message: format!(
                "`{key}` is owner-controlled: a project may select a harness but cannot \
                 declare what Smith executes; move it to `~/.smith/config.toml`"
            ),
        });
    }
    Ok(())
}

fn resolve_env(
    provenance: &Provenance,
    scope: &str,
) -> Result<BTreeMap<String, String>, ConfigError> {
    let prefix = format!("{scope}.env.");
    let mut env = BTreeMap::new();
    let keys: Vec<String> = provenance
        .keys()
        .filter(|key| key.starts_with(&prefix))
        .map(str::to_owned)
        .collect();
    for key in keys {
        if let Some(value) = text(provenance, &key)? {
            owner_only(&value.source, &key)?;
            env.insert(key[prefix.len()..].to_owned(), value.value);
        }
    }
    Ok(env)
}
