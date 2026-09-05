//! Resolves the installed coding agent a profile runs its turns on.
//!
//! An agent is selected by model id -- `cli/claude-code/sonnet` -- so trying
//! one needs no configuration at all. `[harness.<kind>]` remains available for
//! what genuinely varies per machine: a non-standard executable path, extra
//! arguments, the environment, and whether the CLI may run its own tools.
//!
//! A harness replaces how a turn is *executed*, not how the model is
//! identified: the profile still resolves a provider, because the runtime
//! plans against real limits before any work runs.

use std::collections::BTreeMap;

use crate::cli_agents::parse_cli_model_id;

use super::load::join_key;
use super::provenance::*;
use super::provider::{flag, list, text};
use super::types::*;

/// Reads the installed agent the selected model names, if it names one.
pub(super) fn resolve_harness(
    provenance: &Provenance,
) -> Result<Option<ResolvedHarness>, ConfigError> {
    let Some(model) = text(provenance, "model")? else {
        return Ok(None);
    };
    let Some((entry, cli_model)) = parse_cli_model_id(&model.value) else {
        return Ok(None);
    };

    // Optional per-machine overrides. Absent is the ordinary case: the
    // executable is found on PATH and the CLI runs without its own tools.
    let scope = join_key(&["harness", entry.kind]);

    let executable = match text(provenance, &format!("{scope}.executable"))? {
        Some(executable) => {
            owner_only(&executable.source, &format!("{scope}.executable"))?;
            if !std::path::Path::new(&executable.value).is_absolute() {
                return Err(ConfigError::InvalidValue {
                    source: executable.source.clone(),
                    message: format!(
                        "`{scope}.executable` must be an absolute path, invoked without a shell"
                    ),
                });
            }
            Some(executable)
        }
        None => None,
    };

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
        kind: Sourced::new(entry.kind.to_owned(), model.source.clone()),
        program: entry.program.to_owned(),
        model: Sourced::new(cli_model, model.source.clone()),
        executable,
        args,
        allow_own_tools,
        env,
    }))
}

/// Refuses a process-bearing value written by a project layer.
///
/// A project may select an agent, because that only chooses which installed
/// program Smith asks to work. It may not say what gets executed, with which
/// arguments, or whether that program may write to the machine -- the same
/// boundary command providers already enforce.
fn owner_only(source: &Source, key: &str) -> Result<(), ConfigError> {
    if matches!(source.layer, Layer::ProjectFile | Layer::ProjectLocalFile) {
        return Err(ConfigError::InvalidValue {
            source: source.clone(),
            message: format!(
                "`{key}` is owner-controlled: a project may select an installed agent but \
                 cannot declare what Smith executes; move it to `~/.smith/config.toml`"
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
