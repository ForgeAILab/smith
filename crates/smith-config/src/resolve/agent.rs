//! Unified profile, agent, and child-agent resolution.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use crate::model::{AgentPosture, ProfileUse};

use super::load::{Declarations, join_key, quote_segment};
use super::provenance::*;
use super::provider::*;
use super::types::*;

pub(super) fn select_profile(
    file_layers: &[Vec<Contribution>],
    env: &[Contribution],
    cli: &[Contribution],
    session: &[Contribution],
    declared: &Declarations,
) -> Result<Option<Sourced<String>>, ConfigError> {
    let mut winner: Option<&Contribution> = None;
    for layer in file_layers
        .iter()
        .map(Vec::as_slice)
        .chain([env, cli, session])
    {
        if let Some(found) = layer
            .iter()
            .rev()
            .find(|contribution| contribution.key == "profile")
        {
            winner = Some(found);
        }
    }

    let Some(winner) = winner else {
        return Ok(None);
    };
    let SettingValue::Text(name) = &winner.value else {
        return Err(ConfigError::InvalidValue {
            source: winner.source.clone(),
            message: "a profile is named by a string".to_owned(),
        });
    };
    if !declared.profiles.contains_key(name) {
        return Err(ConfigError::UnusableReference {
            source: winner.source.clone(),
            what: ReferenceKind::Profile,
            name: name.clone(),
            suggestions: nearest(name, declared.profiles.keys().map(String::as_str)),
        });
    }
    Ok(Some(Sourced::new(name.clone(), winner.source.clone())))
}

/// Lifts the selected profile's settings into the layer above the files.
///
/// The contribution keeps pointing at the file and key the value was written
/// in — provenance should name where a user can go and change it — while the
/// layer records why it beat the plain file settings.
pub(super) fn profile_contributions(
    file_layers: &[Vec<Contribution>],
    profile: &str,
    declared: &Declarations,
) -> Result<Vec<Contribution>, ConfigError> {
    const MAX_PROFILE_INHERITANCE_DEPTH: usize = 16;

    fn collect(
        file_layers: &[Vec<Contribution>],
        profile: &str,
        declared: &Declarations,
        stack: &mut Vec<String>,
        depth: usize,
    ) -> Result<BTreeMap<String, Contribution>, ConfigError> {
        let declaration =
            declared
                .profiles
                .get(profile)
                .ok_or_else(|| ConfigError::UnusableReference {
                    source: stack
                        .last()
                        .and_then(|child| declared.profiles.get(child))
                        .cloned()
                        .unwrap_or_else(|| Source::built_in("profile")),
                    what: ReferenceKind::Profile,
                    name: profile.to_owned(),
                    suggestions: nearest(profile, declared.profiles.keys().map(String::as_str)),
                })?;
        if depth >= MAX_PROFILE_INHERITANCE_DEPTH {
            return Err(ConfigError::InvalidValue {
                source: declaration.clone(),
                message: format!(
                    "profile inheritance exceeds {MAX_PROFILE_INHERITANCE_DEPTH} levels"
                ),
            });
        }
        if let Some(start) = stack.iter().position(|name| name == profile) {
            let mut cycle = stack[start..].to_vec();
            cycle.push(profile.to_owned());
            return Err(ConfigError::InvalidValue {
                source: declaration.clone(),
                message: format!("profile inheritance cycle: {}", cycle.join(" -> ")),
            });
        }

        stack.push(profile.to_owned());
        let prefix = format!("{}.", join_key(&["profiles", profile]));
        let mut own = BTreeMap::<String, Contribution>::new();
        for layer in file_layers {
            for contribution in layer {
                let Some(rest) = contribution.key.strip_prefix(&prefix) else {
                    continue;
                };
                own.insert(rest.to_owned(), contribution.clone());
            }
        }

        let parent = match own.remove("extends") {
            Some(contribution) => match contribution.value {
                SettingValue::Text(parent) => Some((parent, contribution.source)),
                other => {
                    stack.pop();
                    return Err(ConfigError::InvalidValue {
                        source: contribution.source,
                        message: format!(
                            "profile `extends` must be a string, found {}",
                            describe(other.kind())
                        ),
                    });
                }
            },
            None => None,
        };

        let mut winners = if let Some((parent, source)) = parent {
            if !declared.profiles.contains_key(&parent) {
                stack.pop();
                return Err(ConfigError::UnusableReference {
                    source,
                    what: ReferenceKind::Profile,
                    name: parent.clone(),
                    suggestions: nearest(&parent, declared.profiles.keys().map(String::as_str)),
                });
            }
            collect(file_layers, &parent, declared, stack, depth + 1)?
        } else {
            BTreeMap::new()
        };
        stack.pop();

        for (key, contribution) in own {
            winners.insert(key, contribution);
        }
        Ok(winners)
    }

    Ok(collect(file_layers, profile, declared, &mut Vec::new(), 0)?
        .into_iter()
        .map(|(key, contribution)| Contribution {
            key,
            value: contribution.value,
            source: Source {
                layer: Layer::Profile,
                file: contribution.source.file,
                key: contribution.source.key,
            },
        })
        .collect())
}

pub(super) fn resolve_agent_profiles(
    file_layers: &[Vec<Contribution>],
    declared: &Declarations,
) -> Result<BTreeMap<String, ResolvedAgentProfile>, ConfigError> {
    let mut global = Provenance::default();
    for layer in file_layers {
        global.extend(layer.clone());
    }

    let mut profiles = BTreeMap::new();
    for (name, declaration) in &declared.profiles {
        validate_agent_name(name, declaration, "agent profile")?;
        let mut effective = Provenance::default();
        effective.extend(profile_contributions(file_layers, name, declared)?);
        profiles.insert(
            name.clone(),
            resolved_profile(name, declaration, &effective, &global, false)?,
        );
    }

    for (name, declaration) in &declared.agent_modes {
        validate_agent_name(name, declaration, "agent mode")?;
        let scope = join_key(&["agent_modes", name]);
        let posture = resolve_agent_posture(&global, &format!("{scope}.posture"), declaration)?;
        let description = bounded_description(&global, &format!("{scope}.description"))?;
        let delegation = Sourced::new(true, declaration.clone());
        let uses = Sourced::new(vec![ProfileUse::Main], declaration.clone());
        let revision = agent_profile_revision(
            name,
            &posture,
            description.as_ref(),
            None,
            &delegation,
            &uses,
            None,
            None,
            true,
        );
        merge_legacy_profile(
            &mut profiles,
            ResolvedAgentProfile {
                name: name.clone(),
                posture,
                description,
                instructions: None,
                delegation,
                uses,
                provider: None,
                model: None,
                revision,
                legacy: true,
            },
            declaration,
        )?;
    }

    for (name, declaration) in &declared.child_agents {
        validate_agent_name(name, declaration, "child agent")?;
        let scope = join_key(&["child_agents", name]);
        let posture = resolve_agent_posture(&global, &format!("{scope}.posture"), declaration)?;
        if !posture.value.is_read_only() {
            return Err(ConfigError::InvalidValue {
                source: posture.source,
                message: format!(
                    "child agent `{name}` must use a read-only `plan` or `review` posture"
                ),
            });
        }
        let description = bounded_description(&global, &format!("{scope}.description"))?;
        let delegation = Sourced::new(true, declaration.clone());
        let uses = Sourced::new(vec![ProfileUse::Child], declaration.clone());
        let revision = agent_profile_revision(
            name,
            &posture,
            description.as_ref(),
            None,
            &delegation,
            &uses,
            None,
            None,
            true,
        );
        merge_legacy_profile(
            &mut profiles,
            ResolvedAgentProfile {
                name: name.clone(),
                posture,
                description,
                instructions: None,
                delegation,
                uses,
                provider: None,
                model: None,
                revision,
                legacy: true,
            },
            declaration,
        )?;
    }
    Ok(profiles)
}

pub(super) fn resolved_profile(
    name: &str,
    declaration: &Source,
    effective: &Provenance,
    global: &Provenance,
    legacy: bool,
) -> Result<ResolvedAgentProfile, ConfigError> {
    let posture = match text(effective, "posture")? {
        Some(raw) => {
            let posture =
                AgentPosture::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
                    source: raw.source.clone(),
                    message: format!(
                        "`{}` is not an agent posture; the postures are {}",
                        raw.value,
                        list_spellings(AgentPosture::spellings())
                    ),
                })?;
            Sourced::new(posture, raw.source)
        }
        None => match text(effective, "agent")? {
            Some(mode) => {
                let mode_key = format!("{}.posture", join_key(&["agent_modes", &mode.value]));
                let mode_source = global.source(&mode_key).cloned().ok_or_else(|| {
                    ConfigError::UnusableReference {
                        source: mode.source.clone(),
                        what: ReferenceKind::AgentMode,
                        name: mode.value.clone(),
                        suggestions: Vec::new(),
                    }
                })?;
                resolve_agent_posture(global, &mode_key, &mode_source)?
            }
            None => match text(global, "agent")? {
                Some(mode) => {
                    let mode_key = format!("{}.posture", join_key(&["agent_modes", &mode.value]));
                    let mode_source = global.source(&mode_key).cloned().ok_or_else(|| {
                        ConfigError::UnusableReference {
                            source: mode.source.clone(),
                            what: ReferenceKind::AgentMode,
                            name: mode.value.clone(),
                            suggestions: Vec::new(),
                        }
                    })?;
                    resolve_agent_posture(global, &mode_key, &mode_source)?
                }
                None => Sourced::new(AgentPosture::Build, declaration.clone()),
            },
        },
    };
    let description = bounded_description(effective, "description")?;
    let instructions = bounded_instructions(effective, "instructions")?;
    let delegation =
        flag(effective, "delegation")?.unwrap_or_else(|| Sourced::new(true, declaration.clone()));
    let uses = profile_uses(effective, declaration)?;
    let provider = text(effective, "provider")?;
    let model = text(effective, "model")?;
    let revision = agent_profile_revision(
        name,
        &posture,
        description.as_ref(),
        instructions.as_ref(),
        &delegation,
        &uses,
        provider.as_ref(),
        model.as_ref(),
        legacy,
    );
    Ok(ResolvedAgentProfile {
        name: name.to_owned(),
        posture,
        description,
        instructions,
        delegation,
        uses,
        provider,
        model,
        revision,
        legacy,
    })
}

pub(super) fn merge_legacy_profile(
    profiles: &mut BTreeMap<String, ResolvedAgentProfile>,
    incoming: ResolvedAgentProfile,
    source: &Source,
) -> Result<(), ConfigError> {
    let Some(existing) = profiles.get_mut(&incoming.name) else {
        profiles.insert(incoming.name.clone(), incoming);
        return Ok(());
    };
    if !existing.legacy {
        if source.layer == Layer::BuiltIn {
            return Ok(());
        }
        return Err(ConfigError::Ambiguous {
            key: format!("profiles.{}", quote_segment(&incoming.name)),
            sources: vec![existing.posture.source.clone(), source.clone()],
        });
    }
    if existing.posture.value != incoming.posture.value {
        return Err(ConfigError::Ambiguous {
            key: format!("legacy agent profile `{}`", incoming.name),
            sources: vec![existing.posture.source.clone(), source.clone()],
        });
    }
    for placement in incoming.uses.value {
        if !existing.uses.value.contains(&placement) {
            existing.uses.value.push(placement);
        }
    }
    existing.uses.value.sort();
    existing.revision = agent_profile_revision(
        &existing.name,
        &existing.posture,
        existing.description.as_ref(),
        existing.instructions.as_ref(),
        &existing.delegation,
        &existing.uses,
        existing.provider.as_ref(),
        existing.model.as_ref(),
        true,
    );
    Ok(())
}

pub(super) fn profile_uses(
    provenance: &Provenance,
    declaration: &Source,
) -> Result<Sourced<Vec<ProfileUse>>, ConfigError> {
    let Some(raw) = list(provenance, "use")? else {
        return Ok(Sourced::new(vec![ProfileUse::Main], declaration.clone()));
    };
    if raw.value.is_empty() {
        return Err(ConfigError::InvalidValue {
            source: raw.source,
            message: "profile `use` must contain `main`, `child`, or both".to_owned(),
        });
    }
    let mut seen = BTreeSet::new();
    let mut uses = Vec::new();
    for value in raw.value {
        let placement = ProfileUse::parse(&value).ok_or_else(|| ConfigError::InvalidValue {
            source: raw.source.clone(),
            message: format!(
                "`{value}` is not a profile placement; expected {}",
                list_spellings(ProfileUse::spellings())
            ),
        })?;
        if !seen.insert(placement) {
            return Err(ConfigError::InvalidValue {
                source: raw.source.clone(),
                message: format!("profile `use` contains duplicate `{value}`"),
            });
        }
        uses.push(placement);
    }
    Ok(Sourced::new(uses, raw.source))
}

pub(super) fn bounded_instructions(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<String>>, ConfigError> {
    const MAX_PROFILE_INSTRUCTIONS_BYTES: usize = 32 * 1024;
    let instructions = text(provenance, key)?;
    if let Some(instructions) = &instructions
        && (instructions.value.trim().is_empty()
            || instructions.value.len() > MAX_PROFILE_INSTRUCTIONS_BYTES)
    {
        return Err(ConfigError::InvalidValue {
            source: instructions.source.clone(),
            message: format!(
                "profile instructions must contain 1 to {MAX_PROFILE_INSTRUCTIONS_BYTES} UTF-8 bytes"
            ),
        });
    }
    Ok(instructions)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn agent_profile_revision(
    name: &str,
    posture: &Sourced<AgentPosture>,
    description: Option<&Sourced<String>>,
    instructions: Option<&Sourced<String>>,
    delegation: &Sourced<bool>,
    uses: &Sourced<Vec<ProfileUse>>,
    provider: Option<&Sourced<String>>,
    model: Option<&Sourced<String>>,
    legacy: bool,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"smith-agent-profile-2\0");
    digest.update(name.as_bytes());
    digest.update([0]);
    digest.update(posture.value.as_str().as_bytes());
    digest.update([0]);
    digest.update(posture.source.to_string().as_bytes());
    for placement in &uses.value {
        digest.update([0]);
        digest.update(placement.as_str().as_bytes());
    }
    digest.update([0]);
    digest.update(uses.source.to_string().as_bytes());
    digest.update([0, u8::from(delegation.value)]);
    digest.update(delegation.source.to_string().as_bytes());
    for value in [description, instructions, provider, model]
        .into_iter()
        .flatten()
    {
        digest.update([0]);
        digest.update(value.value.as_bytes());
        digest.update([0]);
        digest.update(value.source.to_string().as_bytes());
    }
    digest.update([u8::from(legacy)]);
    format!("{:x}", digest.finalize())
}

/// Reads the ledger into the typed configuration and validates it.
pub(super) fn extract(
    provenance: &Provenance,
    profile: Option<Sourced<String>>,
    declared: &Declarations,
    agent_profiles: BTreeMap<String, ResolvedAgentProfile>,
    profile_use: ProfileUse,
    synthetic_cache_spend: SyntheticCacheSpendAuthority,
) -> Result<ResolvedConfig, ConfigError> {
    let provider_name =
        text(provenance, "provider")?.ok_or_else(|| ConfigError::MissingSetting {
            key: "provider".to_owned(),
            message: "select one with a profile, `SMITH_PROVIDER`, or `--provider`".to_owned(),
        })?;
    if !declared.providers.contains_key(&provider_name.value) {
        return Err(ConfigError::UnusableReference {
            suggestions: nearest(
                &provider_name.value,
                declared.providers.keys().map(String::as_str),
            ),
            source: provider_name.source,
            what: ReferenceKind::Provider,
            name: provider_name.value,
        });
    }
    let model = text(provenance, "model")?.ok_or_else(|| ConfigError::MissingSetting {
        key: "model".to_owned(),
        message: "select one with a profile, `SMITH_MODEL`, or `--model`".to_owned(),
    })?;

    let agent = resolve_agent(
        provenance,
        declared,
        profile.as_ref(),
        agent_profiles,
        profile_use,
    )?;
    let provider = resolve_provider(provenance, provider_name)?;
    let model_limits = resolve_model_limits(provenance, &provider.name.value, &model.value)?;
    let reasoning = ResolvedReasoning {
        enabled: flag(provenance, "reasoning.enabled")?,
        effort: text(provenance, "reasoning.effort")?,
    };
    let model_reasoning = resolve_model_reasoning(provenance, &provider.name.value, &model.value)?;
    let context = resolve_context(provenance, &model_limits, synthetic_cache_spend)?;
    let child_agents = resolve_child_agents(provenance)?;
    let limits = resolve_limits(provenance)?;
    let persistence = resolve_persistence(provenance)?;
    let approval = resolve_approval(provenance)?;
    let background = resolve_background(provenance)?;
    let mcp = super::mcp::resolve_mcp(provenance, declared)?;

    let harness = super::harness::resolve_harness(provenance)?;

    Ok(ResolvedConfig {
        profile,
        agent,
        provider,
        harness,
        model,
        max_output_tokens: optional_u32(provenance, "max_output_tokens")?,
        model_limits,
        reasoning,
        model_reasoning,
        context,
        synthetic_cache_spend,
        child_agents,
        limits,
        persistence,
        approval,
        background,
        mcp,
    })
}

pub(super) fn resolve_agent(
    provenance: &Provenance,
    declared: &Declarations,
    selected_profile: Option<&Sourced<String>>,
    profiles: BTreeMap<String, ResolvedAgentProfile>,
    profile_use: ProfileUse,
) -> Result<ResolvedAgent, ConfigError> {
    let active = required_text(provenance, "agent")?;
    if !declared.agent_modes.contains_key(&active.value) {
        return Err(ConfigError::UnusableReference {
            source: active.source,
            what: ReferenceKind::AgentMode,
            name: active.value.clone(),
            suggestions: nearest(
                &active.value,
                declared.agent_modes.keys().map(String::as_str),
            ),
        });
    }

    let order = list(provenance, "agent_order")?.ok_or_else(|| missing("agent_order"))?;
    if order.value.is_empty() {
        return Err(ConfigError::InvalidValue {
            source: order.source,
            message: "`agent_order` must contain at least one declared mode".to_owned(),
        });
    }
    let mut seen = BTreeSet::new();
    for name in &order.value {
        if !seen.insert(name) {
            return Err(ConfigError::InvalidValue {
                source: order.source.clone(),
                message: format!("`agent_order` contains duplicate mode `{name}`"),
            });
        }
        if !declared.agent_modes.contains_key(name) {
            return Err(ConfigError::UnusableReference {
                source: order.source.clone(),
                what: ReferenceKind::AgentMode,
                name: name.clone(),
                suggestions: nearest(name, declared.agent_modes.keys().map(String::as_str)),
            });
        }
    }
    if !seen.contains(&active.value) {
        return Err(ConfigError::InvalidValue {
            source: order.source.clone(),
            message: format!(
                "active agent mode `{}` must appear in `agent_order`",
                active.value
            ),
        });
    }

    let mut modes = BTreeMap::new();
    for (name, declaration) in &declared.agent_modes {
        validate_agent_name(name, declaration, "agent mode")?;
        let scope = join_key(&["agent_modes", name]);
        let posture = resolve_agent_posture(provenance, &format!("{scope}.posture"), declaration)?;
        let description = bounded_description(provenance, &format!("{scope}.description"))?;
        modes.insert(
            name.clone(),
            ResolvedAgentMode {
                posture,
                description,
            },
        );
    }

    let mut child_presets = BTreeMap::new();
    for (name, declaration) in &declared.child_agents {
        validate_agent_name(name, declaration, "child agent")?;
        let scope = join_key(&["child_agents", name]);
        let posture = resolve_agent_posture(provenance, &format!("{scope}.posture"), declaration)?;
        if !posture.value.is_read_only() {
            return Err(ConfigError::InvalidValue {
                source: posture.source,
                message: format!(
                    "child agent `{name}` must use a read-only `plan` or `review` posture"
                ),
            });
        }
        let description = bounded_description(provenance, &format!("{scope}.description"))?;
        child_presets.insert(
            name.clone(),
            ResolvedChildAgent {
                posture,
                description,
            },
        );
    }

    let profile = match selected_profile {
        Some(selected) => profiles
            .get(&selected.value)
            .cloned()
            .expect("selected profile was resolved from declared profiles"),
        None => profiles
            .get(&active.value)
            .cloned()
            .unwrap_or_else(|| ResolvedAgentProfile {
                name: active.value.clone(),
                posture: modes
                    .get(&active.value)
                    .expect("resolved active agent is declared")
                    .posture
                    .clone(),
                description: modes
                    .get(&active.value)
                    .and_then(|mode| mode.description.clone()),
                instructions: None,
                delegation: Sourced::new(true, active.source.clone()),
                uses: Sourced::new(vec![ProfileUse::Main], active.source.clone()),
                provider: None,
                model: None,
                revision: agent_profile_revision(
                    &active.value,
                    &modes
                        .get(&active.value)
                        .expect("resolved active agent is declared")
                        .posture,
                    modes
                        .get(&active.value)
                        .and_then(|mode| mode.description.as_ref()),
                    None,
                    &Sourced::new(true, active.source.clone()),
                    &Sourced::new(vec![ProfileUse::Main], active.source.clone()),
                    None,
                    None,
                    true,
                ),
                legacy: true,
            }),
    };
    if !profile.supports(profile_use) {
        return Err(ConfigError::InvalidValue {
            source: selected_profile.map_or_else(
                || profile.uses.source.clone(),
                |selected| selected.source.clone(),
            ),
            message: format!(
                "profile `{}` is not enabled for {}-agent use",
                profile.name,
                profile_use.as_str()
            ),
        });
    }

    let profile_order = match list(provenance, "profile_order")? {
        Some(order) => {
            if order.value.is_empty() {
                return Err(ConfigError::InvalidValue {
                    source: order.source,
                    message: "`profile_order` must contain at least one main-enabled profile"
                        .to_owned(),
                });
            }
            let mut seen = BTreeSet::new();
            for name in &order.value {
                if !seen.insert(name) {
                    return Err(ConfigError::InvalidValue {
                        source: order.source.clone(),
                        message: format!("`profile_order` contains duplicate profile `{name}`"),
                    });
                }
                let Some(candidate) = profiles.get(name) else {
                    return Err(ConfigError::UnusableReference {
                        source: order.source.clone(),
                        what: ReferenceKind::Profile,
                        name: name.clone(),
                        suggestions: nearest(name, profiles.keys().map(String::as_str)),
                    });
                };
                if !candidate.supports(ProfileUse::Main) {
                    return Err(ConfigError::InvalidValue {
                        source: order.source.clone(),
                        message: format!("`profile_order` names child-only profile `{name}`"),
                    });
                }
            }
            if profile_use == ProfileUse::Main && !seen.contains(&profile.name) {
                return Err(ConfigError::InvalidValue {
                    source: order.source.clone(),
                    message: format!(
                        "active profile `{}` must appear in `profile_order`",
                        profile.name
                    ),
                });
            }
            order
        }
        None => {
            let mut order = profiles
                .values()
                .filter(|candidate| !candidate.legacy && candidate.supports(ProfileUse::Main))
                .map(|candidate| candidate.name.clone())
                .collect::<Vec<_>>();
            if !order.contains(&profile.name) {
                order.insert(0, profile.name.clone());
            }
            Sourced::new(
                order,
                selected_profile
                    .map_or_else(|| profile.uses.source.clone(), |value| value.source.clone()),
            )
        }
    };

    Ok(ResolvedAgent {
        active,
        order,
        modes,
        child_presets,
        profile,
        profiles,
        profile_order,
    })
}

pub(super) fn validate_agent_name(
    name: &str,
    source: &Source,
    kind: &str,
) -> Result<(), ConfigError> {
    if name.is_empty()
        || name.len() > 32
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ConfigError::InvalidValue {
            source: source.clone(),
            message: format!(
                "a {kind} name must contain 1 to 32 ASCII letters, digits, `-`, or `_`"
            ),
        });
    }
    Ok(())
}

pub(super) fn resolve_agent_posture(
    provenance: &Provenance,
    key: &str,
    declaration: &Source,
) -> Result<Sourced<AgentPosture>, ConfigError> {
    let raw = text(provenance, key)?.ok_or_else(|| ConfigError::InvalidValue {
        source: declaration.clone(),
        message: "an agent definition must declare a `posture`".to_owned(),
    })?;
    let posture = AgentPosture::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
        source: raw.source.clone(),
        message: format!(
            "`{}` is not an agent posture; the postures are {}",
            raw.value,
            list_spellings(AgentPosture::spellings())
        ),
    })?;
    Ok(Sourced::new(posture, raw.source))
}

pub(super) fn bounded_description(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<String>>, ConfigError> {
    let description = text(provenance, key)?;
    if let Some(description) = &description
        && (description.value.is_empty() || description.value.chars().count() > 160)
    {
        return Err(ConfigError::InvalidValue {
            source: description.source.clone(),
            message: "an agent description must contain 1 to 160 characters".to_owned(),
        });
    }
    Ok(description)
}
