//! Layer discovery, loading, flattening, environment parsing, and resolution entry points.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::model::{
    AgentModeSection, AgentPosture, ApprovalMode, ApprovalSection, BackgroundExit,
    BackgroundSection, CacheSection, ChildAgentSection, ConfigFile, ContextCacheSection,
    ContextSection, KIND_COMMAND_JSONL, LimitsSection, PersistenceSection,
};
use agent_runtime_core::store::Secret;

use super::agent::*;
use super::provenance::*;
use super::provider::*;
use super::types::*;

/// Resolves one run's configuration.
///
/// Discovery and layering finish before validation or provider construction.
pub fn resolve(request: &ResolveRequest) -> Result<Resolution, ConfigError> {
    let layout = discover(request)?;

    let mut declared = Declarations::default();
    let mut file_layers: Vec<Vec<Contribution>> = Vec::new();
    let defaults = built_in_defaults(&layout.user_dir);
    declared.absorb(&defaults, Layer::BuiltIn, None);
    file_layers.push(contributions_of(&defaults, Layer::BuiltIn, None)?);
    for file in &layout.files {
        let loaded = load(&file.path, file.layer)?;
        declared.absorb(&loaded.file, file.layer, Some(&file.path));
        file_layers.push(loaded.contributions);
    }

    let env = env_contributions(&request.env)?;
    let mut cli = request.cli.contributions(Layer::CommandLine);
    normalize_idle_compaction_aliases(&mut cli)?;
    let mut session = request.session.contributions(Layer::SessionOverride);
    normalize_idle_compaction_aliases(&mut session)?;

    let agent_profiles = resolve_agent_profiles(&file_layers, &declared)?;
    let selected = select_profile(&file_layers, &env, &cli, &session, &declared)?;
    let profile_layer = selected
        .as_ref()
        .map(|profile| profile_contributions(&file_layers, &profile.value, &declared))
        .transpose()?
        .unwrap_or_default();

    let mut provenance = Provenance::default();
    for layer in &file_layers {
        provenance.extend(layer.clone());
    }
    provenance.extend(profile_layer);
    provenance.extend(env);
    provenance.extend(cli);
    provenance.extend(session);
    apply_product_model_defaults(&mut provenance);

    let config = extract(
        &provenance,
        selected,
        &declared,
        agent_profiles,
        request.profile_use,
        request.synthetic_cache_spend,
    )?;
    let cache_miss_notices = required_flag(&provenance, "cache.miss_notices")?;
    Ok(Resolution {
        layout,
        config,
        provenance,
        cache_miss_notices,
    })
}

pub(super) fn apply_product_model_defaults(provenance: &mut Provenance) {
    if provenance.winner("max_output_tokens").is_some() {
        return;
    }
    let (Some(provider), Some(model)) = (provenance.winner("provider"), provenance.winner("model"))
    else {
        return;
    };
    let (SettingValue::Text(provider), SettingValue::Text(model)) = (&provider.value, &model.value)
    else {
        return;
    };
    let Some(record) = crate::setup::trusted_model(provider, model) else {
        return;
    };
    if record.request_output_tokens == 0 {
        return;
    }
    provenance.extend(vec![Contribution {
        key: "max_output_tokens".to_owned(),
        value: SettingValue::Integer(i64::from(record.request_output_tokens)),
        source: Source::built_in(format!(
            "trusted catalog {}@{} models.\"{}/{}\".request_output_tokens",
            record.catalog, record.revision, record.provider, record.model
        )),
    }]);
}

/// Inspects whether an invocation is ready, genuinely unconfigured, or
/// invalid.
///
/// This is intentionally a wrapper around [`resolve`], rather than a second
/// resolver. A ready result therefore cannot drift from normal startup. When
/// resolution fails, Smith performs only enough declarative inspection to
/// decide whether setup is safe to offer; malformed and partial user intent
/// remains the original error.
pub fn inspect(request: &ResolveRequest) -> ConfigReadiness {
    match resolve(request) {
        Ok(resolution) => ConfigReadiness::Ready(Box::new(resolution)),
        Err(error) => {
            let missing_selection = matches!(
                &error,
                ConfigError::MissingSetting { key, .. }
                    if matches!(key.as_str(), "provider" | "model")
            );
            if missing_selection {
                match setup_intent(request) {
                    Ok((layout, false)) => ConfigReadiness::Unconfigured(SetupContext { layout }),
                    Ok((_, true)) | Err(_) => ConfigReadiness::Invalid(error),
                }
            } else {
                ConfigReadiness::Invalid(error)
            }
        }
    }
}

/// Returns the discovered layout and whether any layer expresses
/// provider/model setup intent.
pub(super) fn setup_intent(request: &ResolveRequest) -> Result<(Layout, bool), ConfigError> {
    let layout = discover(request)?;
    let mut intent = false;
    for loaded in &layout.files {
        let file = load(&loaded.path, loaded.layer)?.file;
        intent |= file.default_profile.is_some()
            || !file.profiles.is_empty()
            || !file.providers.is_empty()
            || !file.models.is_empty();
    }

    intent |= request.env.keys().any(|name| {
        matches!(
            name.to_ascii_uppercase().as_str(),
            "SMITH_PROFILE" | "SMITH_PROVIDER" | "SMITH_MODEL"
        )
    });
    intent |= request.cli.profile.is_some()
        || request.cli.provider.is_some()
        || request.cli.model.is_some()
        || request.session.profile.is_some()
        || request.session.provider.is_some()
        || request.session.model.is_some();
    Ok((layout, intent))
}

/// Finds the project's `.smith` directory and the user's.
///
/// The project root is the nearest ancestor of the start directory that
/// contains a `.smith` directory. The user root is excluded from that walk: a
/// project opened inside the home directory must not adopt `~/.smith` as its
/// project layer, or user state would silently become project configuration.
pub(super) fn discover(request: &ResolveRequest) -> Result<Layout, ConfigError> {
    let home = match &request.home_dir {
        Some(home) => home.clone(),
        None => dirs::home_dir().ok_or(ConfigError::NoHomeDirectory)?,
    };
    let home = canonical_or_given(&home);
    let user_dir = home.join(SMITH_DIR);

    let start = canonical_or_given(&request.start_dir);
    let project_root = start
        .ancestors()
        .find(|dir| **dir != *home.as_path() && dir.join(SMITH_DIR).is_dir())
        .map(Path::to_path_buf);
    let project_dir = project_root.as_ref().map(|root| root.join(SMITH_DIR));

    let mut files = Vec::new();
    let user_config = user_dir.join(CONFIG_FILE);
    if user_config.is_file() {
        files.push(LoadedFile {
            layer: Layer::UserFile,
            path: user_config,
        });
    }
    if let Some(dir) = &project_dir {
        let project_config = dir.join(CONFIG_FILE);
        if project_config.is_file() {
            files.push(LoadedFile {
                layer: Layer::ProjectFile,
                path: project_config,
            });
        }
        let local_config = dir.join(LOCAL_CONFIG_FILE);
        if local_config.is_file() {
            files.push(LoadedFile {
                layer: Layer::ProjectLocalFile,
                path: local_config,
            });
        }
    }

    Ok(Layout {
        project_root,
        project_dir,
        user_dir,
        files,
    })
}

/// Resolves symlinks where possible; a path that does not exist yet is used as
/// written so discovery can report what was looked for.
pub(super) fn canonical_or_given(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Smith's own defaults.
///
/// Deliberately absent: every model limit, and every reserve that depends on
/// one. A default context window would be a claim about a real model that
/// Smith is in no position to make.
pub(super) fn built_in_defaults(user_dir: &Path) -> ConfigFile {
    ConfigFile {
        default_agent: Some("build".to_owned()),
        agent_order: Some(vec![
            "build".to_owned(),
            "plan".to_owned(),
            "review".to_owned(),
        ]),
        agent_modes: BTreeMap::from([
            (
                "build".to_owned(),
                AgentModeSection {
                    posture: Some(AgentPosture::Build),
                    description: Some("coding workflow with policy-bounded mutation".to_owned()),
                },
            ),
            (
                "plan".to_owned(),
                AgentModeSection {
                    posture: Some(AgentPosture::Plan),
                    description: Some("read-only inspection and planning".to_owned()),
                },
            ),
            (
                "review".to_owned(),
                AgentModeSection {
                    posture: Some(AgentPosture::Review),
                    description: Some("read-only change review and findings".to_owned()),
                },
            ),
        ]),
        child_agents: BTreeMap::from([
            (
                "explore".to_owned(),
                ChildAgentSection {
                    posture: Some(AgentPosture::Plan),
                    description: Some("read-only repository exploration".to_owned()),
                },
            ),
            (
                "review".to_owned(),
                ChildAgentSection {
                    posture: Some(AgentPosture::Review),
                    description: Some("read-only independent review".to_owned()),
                },
            ),
        ]),
        context: Some(ContextSection {
            reasoning_reserve: Some(0),
            compaction_high_watermark_percent: Some(85),
            compaction_low_watermark_percent: Some(60),
            cache: Some(ContextCacheSection {
                // One hour of meaningful inactivity, per the context-lifetime
                // policy.
                inactivity_limit_ms: Some(60 * 60 * 1000),
                maintenance: Some(crate::model::CacheMaintenanceMode::Off),
                max_hold_while_child_ms: Some(60 * 60 * 1000),
                max_maintenance_calls: Some(1),
                max_maintenance_input_tokens: Some(0),
                max_maintenance_output_tokens: Some(256),
                maintenance_deadline_ms: Some(30_000),
                keepalive_margin_ms: Some(120_000),
                keepalive_jitter_percent: Some(10),
                handoff_checkpoint: Some(true),
                idle_compaction: Some(true),
                resume_capsule: Some(true),
            }),
            ..ContextSection::default()
        }),
        limits: Some(LimitsSection {
            max_retries: Some(2),
            max_tool_steps: Some(0),
            turn_time_limit_ms: Some(0),
            tool_output_limit_bytes: Some(64 * 1024),
        }),
        persistence: Some(PersistenceSection {
            enabled: Some(true),
            sessions_dir: Some(user_dir.join("sessions").to_string_lossy().into_owned()),
            journal_events: Some(true),
            checkpoint_key: None,
            checkpoint_key_credential: None,
        }),
        // Approval and background-work defaults fail closed: ask before acting,
        // and refuse to exit while work is still running.
        approval: Some(ApprovalSection {
            mode: Some(ApprovalMode::Ask),
            auto_approve: None,
            auto: Vec::new(),
        }),
        background: Some(BackgroundSection {
            exit_policy: Some(BackgroundExit::Error),
            max_children: Some(4),
            max_monitors: Some(8),
        }),
        cache: Some(CacheSection {
            miss_notices: Some(false),
        }),
        ..ConfigFile::default()
    }
}

/// One file's parsed contents plus the contributions it makes.
pub(super) struct Loaded {
    pub(super) file: ConfigFile,
    pub(super) contributions: Vec<Contribution>,
}

/// The named tables a file declares, and where each was declared.
#[derive(Debug, Default)]
pub(super) struct Declarations {
    pub(super) agent_modes: BTreeMap<String, Source>,
    pub(super) child_agents: BTreeMap<String, Source>,
    pub(super) profiles: BTreeMap<String, Source>,
    pub(super) providers: BTreeMap<String, Source>,
    pub(super) mcp_servers: BTreeMap<String, Source>,
}

impl Declarations {
    fn absorb(&mut self, file: &ConfigFile, layer: Layer, path: Option<&Path>) {
        for name in file.agent_modes.keys() {
            let key = join_key(&["agent_modes", name]);
            self.agent_modes
                .insert(name.clone(), source_for(layer, path, key));
        }
        for name in file.child_agents.keys() {
            let key = join_key(&["child_agents", name]);
            self.child_agents
                .insert(name.clone(), source_for(layer, path, key));
        }
        for name in file.profiles.keys() {
            let key = join_key(&["profiles", name]);
            self.profiles
                .insert(name.clone(), source_for(layer, path, key));
        }
        for name in file.providers.keys() {
            let key = join_key(&["providers", name]);
            self.providers
                .insert(name.clone(), source_for(layer, path, key));
        }
        for name in file.mcp.iter().flat_map(|mcp| mcp.servers.keys()) {
            let key = join_key(&["mcp", "servers", name]);
            self.mcp_servers
                .insert(name.clone(), source_for(layer, path, key));
        }
    }
}

/// Reads and parses one file.
pub(super) fn load(path: &Path, layer: Layer) -> Result<Loaded, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|err| ConfigError::Unreadable {
        path: path.to_path_buf(),
        message: err.to_string(),
    })?;
    let file = ConfigFile::parse(&text).map_err(|err| parse_error(path, layer, &text, &err))?;
    validate_inline_secret_file(path, layer, &file)?;
    let contributions = contributions_of(&file, layer, Some(path))?;
    Ok(Loaded {
        file,
        contributions,
    })
}

pub(super) fn validate_inline_secret_file(
    path: &Path,
    layer: Layer,
    file: &ConfigFile,
) -> Result<(), ConfigError> {
    if layer != Layer::UserFile
        && let Some((provider, section)) = file.providers.iter().find(|(_, provider)| {
            provider.command.is_some() || provider.kind.as_deref() == Some(KIND_COMMAND_JSONL)
        })
    {
        let key = if section.command.is_some() {
            join_key(&["providers", provider, "command"])
        } else {
            join_key(&["providers", provider, "kind"])
        };
        return Err(ConfigError::InvalidValue {
            source: Source::file(layer, path, key),
            message: "command-provider process settings are user-scoped; project configuration may select an existing provider but cannot define or override it"
                .to_owned(),
        });
    }

    let has_inline_provider_key = file
        .providers
        .values()
        .any(|provider| provider.api_key.is_some());
    let checkpoint_key = file
        .persistence
        .as_ref()
        .is_some_and(|persistence| persistence.checkpoint_key.is_some());
    let checkpoint_credential = file
        .persistence
        .as_ref()
        .is_some_and(|persistence| persistence.checkpoint_key_credential.is_some());
    if layer != Layer::UserFile && (checkpoint_key || checkpoint_credential) {
        return Err(ConfigError::InvalidValue {
            source: Source::file(layer, path, "persistence.checkpoint_key"),
            message:
                "checkpoint protection is user-scoped; project configuration cannot supply or redirect its key"
                    .to_owned(),
        });
    }

    // A credential-named MCP environment variable holding anything but a
    // reference is a secret written in a file, and is refused on the same terms
    // as an inline provider key. The key is named; the value never is.
    let mcp_env_literal = file.mcp.as_ref().and_then(|mcp| {
        mcp.servers.iter().find_map(|(server, section)| {
            section
                .env
                .iter()
                .find(|(name, value)| names_a_credential(name) && !is_credential_reference(value))
                .map(|(name, _)| join_key(&["mcp", "servers", server, "env", name]))
        })
    });
    let command_env_literal = file.providers.iter().find_map(|(provider, section)| {
        section.command.as_ref().and_then(|command| {
            command
                .env
                .iter()
                .find(|(_, value)| !is_credential_reference(value))
                .map(|(name, _)| join_key(&["providers", provider, "command", "env", name]))
        })
    });

    let has_inline = has_inline_provider_key
        || checkpoint_key
        || mcp_env_literal.is_some()
        || command_env_literal.is_some();
    if !has_inline {
        return Ok(());
    }
    let source = Source::file(
        layer,
        path,
        match (&command_env_literal, &mcp_env_literal, checkpoint_key) {
            (Some(key), _, _) => key.clone(),
            (None, Some(key), _) => key.clone(),
            (None, None, true) => "persistence.checkpoint_key".to_owned(),
            (None, None, false) => "providers.<name>.api_key".to_owned(),
        },
    );
    if layer != Layer::UserFile {
        return Err(ConfigError::PlaintextSecret {
            source,
            message: "inline keys are allowed only in `~/.smith/config.toml`; project files must use a credential reference"
                .to_owned(),
        });
    }

    #[cfg(not(unix))]
    {
        return Err(ConfigError::PlaintextSecret {
            source,
            message:
                "inline keys are unavailable because this platform cannot enforce Unix owner-only permissions"
                    .to_owned(),
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let metadata =
            std::fs::symlink_metadata(path).map_err(|error| ConfigError::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ConfigError::PlaintextSecret {
                source,
                message: "an inline key requires a regular, non-symlink user config".to_owned(),
            });
        }
        if metadata.uid() != rustix::process::getuid().as_raw() {
            return Err(ConfigError::PlaintextSecret {
                source,
                message: "an inline key requires a user config owned by the current user"
                    .to_owned(),
            });
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ConfigError::PlaintextSecret {
                source,
                message:
                    "an inline key requires owner-only config permissions; run `chmod 600 ~/.smith/config.toml` and rotate an exposed key"
                        .to_owned(),
            });
        }
    }
    Ok(())
}

/// Turns a parser error into the most specific diagnostic it supports.
///
/// serde reports an unknown field together with the fields it expected, which
/// is exactly the candidate list a "did you mean" needs. When that shape is not
/// present the raw message is still reported, so a parser change degrades the
/// suggestion rather than the diagnostic.
pub(super) fn parse_error(
    path: &Path,
    layer: Layer,
    text: &str,
    err: &toml::de::Error,
) -> ConfigError {
    let location = err.span().map(|span| position(text, span.start));
    let message = err.message();
    if let Some((key, candidates)) = unknown_field(message) {
        return ConfigError::UnknownKey {
            suggestions: nearest(&key, candidates.iter().map(String::as_str)),
            source: Some(Source::file(layer, path, key.clone())),
            key,
            location,
        };
    }
    ConfigError::Malformed {
        path: path.to_path_buf(),
        location,
        message: message.to_owned(),
    }
}

/// Extracts the offending field and the expected fields from serde's
/// unknown-field message.
pub(super) fn unknown_field(message: &str) -> Option<(String, Vec<String>)> {
    let rest = message.strip_prefix("unknown field `")?;
    let (key, rest) = rest.split_once('`')?;
    let candidates = rest
        .split_once("expected one of ")
        .map(|(_, list)| list)
        .or_else(|| rest.split_once("expected ").map(|(_, list)| list))
        .map(|list| {
            list.split(',')
                .filter_map(|item| item.trim().strip_prefix('`'))
                .filter_map(|item| item.strip_suffix('`'))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some((key.to_owned(), candidates))
}

/// The line and column a byte offset falls on.
pub(super) fn position(text: &str, offset: usize) -> Position {
    let consumed = &text[..offset.min(text.len())];
    let line = consumed.matches('\n').count() + 1;
    let column = consumed
        .rfind('\n')
        .map_or(consumed.len(), |index| consumed.len() - index - 1)
        + 1;
    Position {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        column: u32::try_from(column).unwrap_or(u32::MAX),
    }
}

/// Flattens a file into one contribution per written value.
///
/// The file is serialized and walked rather than destructured field by field:
/// a new setting in [`crate::model`] then layers like every other setting
/// instead of waiting for someone to remember to wire it up here.
pub(super) fn contributions_of(
    file: &ConfigFile,
    layer: Layer,
    path: Option<&Path>,
) -> Result<Vec<Contribution>, ConfigError> {
    let table = toml::Table::try_from(file).map_err(|err| ConfigError::Unrepresentable {
        message: err.to_string(),
    })?;
    let mut out = Vec::new();
    flatten(&table, &mut Vec::new(), layer, path, &mut out)?;

    normalize_idle_compaction_aliases(&mut out)?;

    // `default_profile` is how a file selects the active profile, so it is also
    // that file's contribution to the `profile` setting.
    if let Some(name) = &file.default_profile {
        out.push(Contribution {
            key: "profile".to_owned(),
            value: SettingValue::Text(name.clone()),
            source: source_for(layer, path, "default_profile"),
        });
    }
    if let Some(name) = &file.default_agent {
        out.push(Contribution {
            key: "agent".to_owned(),
            value: SettingValue::Text(name.clone()),
            source: source_for(layer, path, "default_agent"),
        });
    }
    Ok(out)
}

/// Maps the one-release `context.idle_compaction_ms` spelling onto the
/// replacement cache-policy key while retaining the original source key for
/// explain output.  A file may contain both spellings, but choosing one by
/// serialization order would make a configuration change silently alter the
/// inactivity clock, so differing same-file values fail closed.
fn normalize_idle_compaction_aliases(out: &mut [Contribution]) -> Result<(), ConfigError> {
    let mut seen: BTreeMap<(Layer, Option<PathBuf>, String), (&SettingValue, Source)> =
        BTreeMap::new();
    for contribution in out.iter() {
        let Some(canonical) = idle_compaction_canonical_key(&contribution.key) else {
            continue;
        };
        let identity = (
            contribution.source.layer,
            contribution.source.file.clone(),
            canonical,
        );
        if let Some((previous, previous_source)) = seen.get(&identity) {
            if *previous != &contribution.value {
                return Err(ConfigError::Ambiguous {
                    key: identity.2.clone(),
                    sources: vec![previous_source.clone(), contribution.source.clone()],
                });
            }
        } else {
            seen.insert(identity, (&contribution.value, contribution.source.clone()));
        }
    }
    for contribution in out {
        if let Some(canonical) = idle_compaction_canonical_key(&contribution.key) {
            contribution.key = canonical;
        }
    }
    Ok(())
}

fn idle_compaction_canonical_key(key: &str) -> Option<String> {
    if key == "context.idle_compaction_ms" {
        return Some("context.cache.inactivity_limit_ms".to_owned());
    }
    if let Some(prefix) = key.strip_suffix(".context.idle_compaction_ms") {
        return Some(format!("{prefix}.context.cache.inactivity_limit_ms"));
    }
    (key == "context.cache.inactivity_limit_ms"
        || key.ends_with(".context.cache.inactivity_limit_ms"))
    .then(|| key.to_owned())
}

pub(super) fn flatten(
    table: &toml::Table,
    prefix: &mut Vec<String>,
    layer: Layer,
    path: Option<&Path>,
    out: &mut Vec<Contribution>,
) -> Result<(), ConfigError> {
    for (name, value) in table {
        prefix.push(name.clone());
        let result = match value {
            // An automatic-approval rule is one security-policy object. Keep
            // the complete ordered rule list as one layered value so a higher
            // layer replaces it rather than accidentally merging individual
            // fields from independently sourced grants.
            toml::Value::Array(items) if is_auto_approval_rules(prefix) => {
                let key = join_owned(prefix);
                let source = source_for(layer, path, key.clone());
                serde_json::to_string(items)
                    .map(|value| {
                        out.push(Contribution {
                            key,
                            value: SettingValue::Text(value),
                            source,
                        });
                    })
                    .map_err(|error| ConfigError::Unrepresentable {
                        message: format!(
                            "automatic approval rules could not be represented: {error}"
                        ),
                    })
            }
            toml::Value::Table(inner) => flatten(inner, prefix, layer, path, out),
            other
                if prefix.last().is_some_and(|segment| {
                    matches!(segment.as_str(), "api_key" | "checkpoint_key")
                }) =>
            {
                let key = join_owned(prefix);
                let source = source_for(layer, path, key.clone());
                if layer != Layer::UserFile {
                    Err(ConfigError::PlaintextSecret {
                        source,
                        message: "inline keys are allowed only in owner-only user configuration"
                            .to_owned(),
                    })
                } else if let Some(value) = other.as_str() {
                    if value.is_empty() {
                        Err(ConfigError::InvalidValue {
                            source,
                            message: "an inline key cannot be empty".to_owned(),
                        })
                    } else {
                        out.push(Contribution {
                            key,
                            value: SettingValue::Secret(Secret::new(value)),
                            source,
                        });
                        Ok(())
                    }
                } else {
                    Err(ConfigError::Unrepresentable {
                        message: format!("`{key}` must be a string"),
                    })
                }
            }
            // A server's environment and headers are the one place a value's
            // *meaning* is defined by a third party. A reference is a locator
            // and stays readable; anything else is treated as secret-bearing,
            // because Smith has no way to tell which of a server's variables
            // carry a token. Which layers may write a literal at all is decided
            // in `validate_inline_secret_file` and `resolve_headers`, on either
            // side of this.
            other if is_secret_capable_value(prefix) => {
                let key = join_owned(prefix);
                let source = source_for(layer, path, key.clone());
                match other.as_str() {
                    Some(value) if is_credential_reference(value) => {
                        out.push(Contribution {
                            key,
                            value: SettingValue::Text(value.to_owned()),
                            source,
                        });
                        Ok(())
                    }
                    Some(value) => {
                        out.push(Contribution {
                            key,
                            value: SettingValue::Secret(Secret::new(value)),
                            source,
                        });
                        Ok(())
                    }
                    None => Err(ConfigError::Unrepresentable {
                        message: format!("`{key}` must be a string"),
                    }),
                }
            }
            other => match setting_value(other) {
                Some(setting) => {
                    let key = join_owned(prefix);
                    out.push(Contribution {
                        key: key.clone(),
                        value: setting,
                        source: source_for(layer, path, key),
                    });
                    Ok(())
                }
                None => Err(ConfigError::Unrepresentable {
                    message: format!("`{}` holds an unsupported value", join_owned(prefix)),
                }),
            },
        };
        prefix.pop();
        result?;
    }
    Ok(())
}

fn is_auto_approval_rules(prefix: &[String]) -> bool {
    matches!(prefix, [approval, auto] if approval == "approval" && auto == "auto")
        || matches!(
            prefix,
            [profiles, _name, approval, auto]
                if profiles == "profiles" && approval == "approval" && auto == "auto"
        )
}

/// Whether a flattened key addresses one environment variable or header of one
/// declared server.
///
/// Both tables hold the same class of value — something a third party receives,
/// which may or may not be a secret — so both are classified the same way.
fn is_secret_capable_value(prefix: &[String]) -> bool {
    matches!(
        prefix,
        [root, servers, _name, table, _key]
            if root == "mcp" && servers == "servers" && (table == "env" || table == "headers")
    ) || matches!(
        prefix,
        [providers, _name, command, env, _key]
            if providers == "providers" && command == "command" && env == "env"
    )
}

pub(super) fn setting_value(value: &toml::Value) -> Option<SettingValue> {
    match value {
        toml::Value::String(text) => Some(SettingValue::Text(text.clone())),
        toml::Value::Integer(number) => Some(SettingValue::Integer(*number)),
        toml::Value::Boolean(flag) => Some(SettingValue::Flag(*flag)),
        toml::Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<Vec<String>>>()
            .map(SettingValue::List),
        _ => None,
    }
}

pub(super) fn source_for(layer: Layer, path: Option<&Path>, key: impl Into<String>) -> Source {
    match path {
        Some(path) => Source::file(layer, path, key),
        None => Source::built_in(key),
    }
}

/// Quotes a key segment when it holds anything but the bare-key characters
/// TOML allows, so a key round-trips into `smith config explain`.
pub(super) fn join_key(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| quote_segment(part))
        .collect::<Vec<_>>()
        .join(".")
}

pub(super) fn join_owned(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| quote_segment(part))
        .collect::<Vec<_>>()
        .join(".")
}

pub(super) fn quote_segment(part: &str) -> String {
    let bare = !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        part.to_owned()
    } else {
        format!("\"{}\"", part.replace('"', "\\\""))
    }
}

/// Reads `SMITH_*` variables into contributions.
///
/// Variable names are matched without regard to case, because an environment
/// map can hold both `SMITH_MODEL` and `smith_model` and a run must not depend
/// on which one the map happened to iterate first. Two names for one setting is
/// therefore the same-layer ambiguity this rejects.
pub(super) fn env_contributions(
    env: &BTreeMap<String, String>,
) -> Result<Vec<Contribution>, ConfigError> {
    let mut claimed: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for (name, value) in env {
        let upper = name.to_uppercase();
        if !upper.starts_with(ENV_PREFIX) {
            continue;
        }
        match setting_for_env(&upper) {
            Some(key) => claimed
                .entry(canonical_setting_key(key))
                .or_default()
                .push((name.as_str(), value.as_str())),
            None => {
                return Err(ConfigError::UnknownKey {
                    key: name.clone(),
                    source: Some(Source::environment(name.clone())),
                    location: None,
                    suggestions: nearest_env(&upper),
                });
            }
        }
    }

    let mut out = Vec::new();
    for (key, hits) in claimed {
        if hits.len() > 1 {
            return Err(ConfigError::Ambiguous {
                key: key.to_owned(),
                sources: hits
                    .iter()
                    .map(|(name, _)| Source::environment(*name))
                    .collect(),
            });
        }
        let (name, raw) = hits[0];
        let source = Source::environment(name);
        let value = parse_text(raw, kind_of(key), &source)?;
        out.push(Contribution {
            key: key.to_owned(),
            value,
            source,
        });
    }
    Ok(out)
}

/// The setting an upper-cased environment variable name addresses.
pub(super) fn setting_for_env(name: &str) -> Option<&'static str> {
    SETTINGS
        .iter()
        .find(|(key, _)| env_name(key) == name)
        .map(|(key, _)| *key)
}

fn canonical_setting_key(key: &str) -> &str {
    match key {
        // The legacy environment spelling is accepted only as an alias; both
        // declarations therefore participate in one same-layer ambiguity
        // check while the original variable remains in Source for explain.
        "context.idle_compaction_ms" => "context.cache.inactivity_limit_ms",
        other => SETTINGS
            .iter()
            .find(|(candidate, _)| *candidate == other)
            .map_or(other, |(candidate, _)| *candidate),
    }
}

/// The environment variable name for a setting key.
pub fn env_name(key: &str) -> String {
    if key == "persistence.checkpoint_key" {
        return "SMITH_CHECKPOINT_KEY".to_owned();
    }
    format!("{ENV_PREFIX}{}", key.replace('.', "_").to_uppercase())
}

pub(super) fn nearest_env(name: &str) -> Vec<String> {
    let candidates: Vec<String> = SETTINGS.iter().map(|(key, _)| env_name(key)).collect();
    nearest(name, candidates.iter().map(String::as_str))
}

pub(super) fn kind_of(key: &str) -> ValueKind {
    SETTINGS
        .iter()
        .find(|(setting, _)| *setting == key)
        .map_or(ValueKind::Text, |(_, kind)| *kind)
}

/// Converts a textual value — an environment variable's — into the setting's
/// kind.
pub(super) fn parse_text(
    raw: &str,
    kind: ValueKind,
    source: &Source,
) -> Result<SettingValue, ConfigError> {
    match kind {
        ValueKind::Text => Ok(SettingValue::Text(raw.to_owned())),
        ValueKind::Secret => Ok(SettingValue::Secret(Secret::new(raw))),
        ValueKind::Integer => raw
            .trim()
            .parse::<i64>()
            .map(SettingValue::Integer)
            .map_err(|_| ConfigError::InvalidValue {
                source: source.clone(),
                message: format!("`{raw}` is not a whole number"),
            }),
        ValueKind::Flag => match raw.trim() {
            "true" => Ok(SettingValue::Flag(true)),
            "false" => Ok(SettingValue::Flag(false)),
            other => Err(ConfigError::InvalidValue {
                source: source.clone(),
                message: format!("`{other}` is not `true` or `false`"),
            }),
        },
        ValueKind::List => Ok(SettingValue::List(
            raw.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect(),
        )),
    }
}
