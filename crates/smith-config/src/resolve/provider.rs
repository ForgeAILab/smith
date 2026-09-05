//! Provider, model, reasoning, context, and policy validation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use crate::model::{
    ApprovalMode, AutoApprovalRuleSection, BackgroundExit, CacheMaintenanceMode,
    KIND_ANTHROPIC_MESSAGES, KIND_CHATGPT_RESPONSES, KIND_COMMAND_JSONL, KIND_FAKE,
    KIND_GEMINI_INTERACTIONS, KIND_OPENAI_COMPATIBLE, KIND_OPENAI_RESPONSES, KIND_XAI_RESPONSES,
    ReasoningDialect, ReasoningOnlyBehavior,
};
use agent_runtime_core::store::Secret;

use super::load::join_key;
use super::provenance::*;
use super::types::*;

pub(super) fn resolve_provider(
    provenance: &Provenance,
    name: Sourced<String>,
) -> Result<ResolvedProvider, ConfigError> {
    let scope = join_key(&["providers", name.value.as_str()]);
    let kind =
        text(provenance, &format!("{scope}.kind"))?.ok_or_else(|| ConfigError::MissingSetting {
            key: format!("{scope}.kind"),
            message: format!(
                "provider `{}` must name the shared adapter it uses",
                name.value
            ),
        })?;
    let base_url = text(provenance, &format!("{scope}.base_url"))?;
    let credentials = resolve_credential_pool(provenance, &scope)?;
    let rotate_at_percent = resolve_rotate_at_percent(provenance, &scope)?;
    let api_key = secret(provenance, &format!("{scope}.api_key"))?;
    let command = resolve_command_provider(provenance, &scope)?;
    let reasoning_only = text(provenance, &format!("{scope}.response.reasoning_only"))?
        .map(|value| {
            ReasoningOnlyBehavior::parse(&value.value)
                .map(|behavior| Sourced::new(behavior, value.source.clone()))
                .ok_or_else(|| ConfigError::InvalidValue {
                    source: value.source,
                    message: "`reasoning_only` must be `reasoning` or `text`".to_owned(),
                })
        })
        .transpose()?;

    let header_prefix = format!("{scope}.headers.");
    let header_keys: Vec<String> = provenance
        .keys()
        .filter(|key| key.starts_with(&header_prefix))
        .map(str::to_owned)
        .collect();
    let mut headers = BTreeMap::new();
    for key in header_keys {
        let header = unquote_segment(&key[header_prefix.len()..]);
        if let Some(value) = text(provenance, &key)? {
            headers.insert(header, value);
        }
    }

    let provider = ResolvedProvider {
        name,
        kind,
        base_url,
        credentials,
        rotate_at_percent,
        api_key,
        headers,
        response: ResolvedProviderResponse { reasoning_only },
        command,
    };
    validate_provider(&provider)?;
    Ok(provider)
}

const MAX_COMMAND_ARGUMENTS: usize = 256;
const MAX_COMMAND_ARGUMENT_BYTES: usize = 64 * 1024;
const MAX_COMMAND_ENVIRONMENT_ENTRIES: usize = 256;
const MAX_COMMAND_ENVIRONMENT_BYTES: usize = 1024 * 1024;

fn resolve_command_provider(
    provenance: &Provenance,
    scope: &str,
) -> Result<Option<ResolvedCommandProvider>, ConfigError> {
    let executable_key = format!("{scope}.command.executable");
    let Some(executable) = text(provenance, &executable_key)? else {
        return Ok(None);
    };
    require_user_process_source(&executable.source)?;
    if executable.value.is_empty() || executable.value.contains('\0') {
        return Err(ConfigError::InvalidValue {
            source: executable.source,
            message: "a command provider executable must be a non-empty path without NUL"
                .to_owned(),
        });
    }
    let executable_path = PathBuf::from(&executable.value);
    if !executable_path.is_absolute() {
        return Err(ConfigError::InvalidValue {
            source: executable.source,
            message:
                "a command provider executable must be an absolute path; Smith does not search PATH"
                    .to_owned(),
        });
    }
    let executable = Sourced::new(executable_path, executable.source);

    let args = list(provenance, &format!("{scope}.command.args"))?;
    if let Some(args) = &args {
        require_user_process_source(&args.source)?;
        if args.value.len() > MAX_COMMAND_ARGUMENTS {
            return Err(ConfigError::InvalidValue {
                source: args.source.clone(),
                message: format!(
                    "a command provider accepts at most {MAX_COMMAND_ARGUMENTS} fixed arguments"
                ),
            });
        }
        if args
            .value
            .iter()
            .any(|argument| argument.len() > MAX_COMMAND_ARGUMENT_BYTES || argument.contains('\0'))
        {
            return Err(ConfigError::InvalidValue {
                source: args.source.clone(),
                message: format!(
                    "each command argument must be at most {MAX_COMMAND_ARGUMENT_BYTES} bytes and contain no NUL"
                ),
            });
        }
    }

    let cwd = text(provenance, &format!("{scope}.command.cwd"))?
        .map(|cwd| {
            require_user_process_source(&cwd.source)?;
            let value = if cwd.value == "workspace" {
                CommandWorkingDirectory::Workspace
            } else {
                if cwd.value.is_empty() || cwd.value.contains('\0') {
                    return Err(ConfigError::InvalidValue {
                        source: cwd.source,
                        message: "a command provider cwd must be `workspace` or an absolute path without NUL"
                            .to_owned(),
                    });
                }
                let path = PathBuf::from(&cwd.value);
                if !path.is_absolute() {
                    return Err(ConfigError::InvalidValue {
                        source: cwd.source,
                        message: "a command provider cwd must be exactly `workspace` or an absolute path"
                            .to_owned(),
                    });
                }
                CommandWorkingDirectory::Absolute(path)
            };
            Ok(Sourced::new(value, cwd.source))
        })
        .transpose()?;

    let env_prefix = format!("{scope}.command.env.");
    let env_keys: Vec<String> = provenance
        .keys()
        .filter(|key| key.starts_with(&env_prefix))
        .map(str::to_owned)
        .collect();
    if env_keys.len() > MAX_COMMAND_ENVIRONMENT_ENTRIES {
        let source = provenance
            .winner(&env_keys[0])
            .expect("a discovered environment key has a winner")
            .source
            .clone();
        return Err(ConfigError::InvalidValue {
            source,
            message: format!(
                "a command provider accepts at most {MAX_COMMAND_ENVIRONMENT_ENTRIES} environment entries"
            ),
        });
    }
    let mut env = BTreeMap::new();
    let mut environment_bytes = 0usize;
    for key in env_keys {
        let name = unquote_segment(&key[env_prefix.len()..]);
        let entry = provenance
            .winner(&key)
            .expect("a discovered environment key has a winner");
        require_user_process_source(&entry.source)?;
        if name.is_empty() || name.contains(['=', '\0']) {
            return Err(ConfigError::InvalidValue {
                source: entry.source.clone(),
                message:
                    "a command environment name must be non-empty and contain neither `=` nor NUL"
                        .to_owned(),
            });
        }
        let value = match &entry.value {
            SettingValue::Text(reference) => {
                let sourced = Sourced::new(reference.clone(), entry.source.clone());
                validate_credential(&sourced)?;
                McpValue::Credential(reference.clone())
            }
            SettingValue::Secret(literal) => McpValue::Literal(literal.clone()),
            other => return Err(wrong_kind(entry, other, "a string")),
        };
        let value_len = match &value {
            McpValue::Credential(reference) => reference.len(),
            McpValue::Literal(literal) => literal.expose().len(),
        };
        if match &value {
            McpValue::Credential(reference) => reference.contains('\0'),
            McpValue::Literal(literal) => literal.expose().contains('\0'),
        } {
            return Err(ConfigError::InvalidValue {
                source: entry.source.clone(),
                message: "a command environment value cannot contain NUL".to_owned(),
            });
        }
        environment_bytes = environment_bytes
            .saturating_add(name.len())
            .saturating_add(value_len);
        if environment_bytes > MAX_COMMAND_ENVIRONMENT_BYTES {
            return Err(ConfigError::InvalidValue {
                source: entry.source.clone(),
                message: format!(
                    "a command provider environment may contain at most {MAX_COMMAND_ENVIRONMENT_BYTES} bytes"
                ),
            });
        }
        env.insert(name, Sourced::new(value, entry.source.clone()));
    }

    Ok(Some(ResolvedCommandProvider {
        executable,
        args,
        cwd,
        env,
    }))
}

fn require_user_process_source(source: &Source) -> Result<(), ConfigError> {
    if source.layer == Layer::UserFile {
        return Ok(());
    }
    Err(ConfigError::InvalidValue {
        source: source.clone(),
        message: "command-provider process settings are user-scoped; project configuration may select an existing provider but cannot define or override its process"
            .to_owned(),
    })
}

/// Resolves the ordered credential pool from either spelling.
///
/// `credential` and `credentials` are the same declaration for one account and
/// several, so naming both is a contradiction rather than a merge: there is no
/// defensible order to splice a single entry into a list, and guessing one
/// would silently decide which account a user's turns are billed to.
fn resolve_credential_pool(
    provenance: &Provenance,
    scope: &str,
) -> Result<Vec<Sourced<String>>, ConfigError> {
    let single = text(provenance, &format!("{scope}.credential"))?;
    let pool = list(provenance, &format!("{scope}.credentials"))?;

    let (entries, source) = match (single, pool) {
        (Some(single), None) => return Ok(vec![single]),
        (None, Some(pool)) => (pool.value, pool.source),
        (None, None) => return Ok(Vec::new()),
        (Some(_), Some(pool)) => {
            return Err(ConfigError::InvalidValue {
                source: pool.source,
                message:
                    "choose one spelling: `credential` for a single account, or `credentials` for an ordered pool"
                        .to_owned(),
            });
        }
    };

    // An empty list reads as no declaration rather than as an error: the
    // config round-trips through serde before provenance sees it, and an empty
    // vector is skipped there, so `credentials = []` and an omitted key are
    // already the same input by the time resolution runs. Reporting an error
    // here would be reporting one this code cannot actually distinguish.
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    // Rejected rather than deduplicated: a repeated entry means the user
    // believes they declared two accounts, and silently collapsing it would
    // leave them with a pool that cannot rotate anywhere.
    let mut seen = BTreeSet::new();
    for entry in &entries {
        if !seen.insert(entry.as_str()) {
            return Err(ConfigError::InvalidValue {
                source,
                message: format!("`credentials` lists `{entry}` more than once"),
            });
        }
    }

    Ok(entries
        .into_iter()
        .map(|entry| Sourced::new(entry, source.clone()))
        .collect())
}

/// Resolves the proactive rotation threshold, which is a percentage.
fn resolve_rotate_at_percent(
    provenance: &Provenance,
    scope: &str,
) -> Result<Option<Sourced<u8>>, ConfigError> {
    let Some(value) = integer(provenance, &format!("{scope}.rotate_at_percent"))? else {
        return Ok(None);
    };
    // A threshold outside 1–100 cannot describe a usage window: 0 would rotate
    // before the first turn, and above 100 can never be reached.
    let percent = u8::try_from(value.value)
        .ok()
        .filter(|p| (1..=100).contains(p));
    match percent {
        Some(percent) => Ok(Some(Sourced::new(percent, value.source))),
        None => Err(ConfigError::InvalidValue {
            source: value.source,
            message: "`rotate_at_percent` is a usage percentage between 1 and 100".to_owned(),
        }),
    }
}

/// Checks the options against what the adapter kind can use.
///
/// Kinds Smith does not know are not rejected here: which adapters exist is a
/// property of the pinned runtime's registry, and reporting an unavailable
/// adapter belongs to the step that consults it. The secret rules apply to
/// every kind, because they protect the file rather than the adapter.
pub(super) fn validate_provider(provider: &ResolvedProvider) -> Result<(), ConfigError> {
    if provider.kind.value == KIND_COMMAND_JSONL {
        require_user_process_source(&provider.kind.source)?;
        if provider.command.is_none() {
            return Err(ConfigError::MissingSetting {
                key: join_key(&["providers", &provider.name.value, "command", "executable"]),
                message: format!(
                    "a `{KIND_COMMAND_JSONL}` provider needs an absolute executable declaration"
                ),
            });
        }
        if let Some(base_url) = &provider.base_url {
            return Err(command_option_error(base_url, "base_url"));
        }
        if let Some(credential) = provider.credential() {
            return Err(command_option_error(credential, "credential"));
        }
        if let Some(rotation) = &provider.rotate_at_percent {
            return Err(command_option_error(rotation, "rotate_at_percent"));
        }
        if let Some(api_key) = &provider.api_key {
            return Err(command_option_error(api_key, "api_key"));
        }
        if let Some(header) = provider.headers.values().next() {
            return Err(command_option_error(header, "headers"));
        }
        if let Some(response) = &provider.response.reasoning_only {
            return Err(command_option_error(response, "response"));
        }
    } else if let Some(command) = &provider.command {
        return Err(ConfigError::IncompatibleOption {
            source: command.executable.source.clone(),
            kind: provider.kind.value.clone(),
            message: "a `command` table is accepted only by the `command-jsonl` adapter".to_owned(),
        });
    }

    if !provider.credentials.is_empty() && provider.api_key.is_some() {
        let source = provider
            .api_key
            .as_ref()
            .map(|value| value.source.clone())
            .expect("the inline key was checked as present");
        return Err(ConfigError::InvalidValue {
            source,
            message: "choose exactly one credential source: `credential` or `api_key`".to_owned(),
        });
    }
    // Every member is checked, not just the active one: a pool whose second
    // entry is unparseable must fail now, not an hour from now when the first
    // one is spent and rotation reaches for it.
    let pooled = provider.credentials.len() > 1;
    for (position, credential) in provider.credentials.iter().enumerate() {
        validate_credential(credential).map_err(|error| {
            if pooled {
                locate_pool_entry(error, position)
            } else {
                error
            }
        })?;
    }
    if let Some(threshold) = &provider.rotate_at_percent
        && provider.credentials.len() < 2
    {
        return Err(ConfigError::InvalidValue {
            source: threshold.source.clone(),
            message:
                "`rotate_at_percent` needs a `credentials` pool with another member to rotate to"
                    .to_owned(),
        });
    }
    for (name, value) in &provider.headers {
        if AUTH_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(ConfigError::PlaintextSecret {
                source: value.source.clone(),
                message: format!(
                    "set the provider's `credential` reference instead of writing an `{name}` header"
                ),
            });
        }
    }
    if !matches!(
        provider.kind.value.as_str(),
        KIND_OPENAI_COMPATIBLE
            | KIND_ANTHROPIC_MESSAGES
            | KIND_CHATGPT_RESPONSES
            | KIND_XAI_RESPONSES
            | KIND_GEMINI_INTERACTIONS
    ) && let Some(policy) = &provider.response.reasoning_only
    {
        return Err(ConfigError::IncompatibleOption {
            source: policy.source.clone(),
            kind: provider.kind.value.clone(),
            message: "`response.reasoning_only` requires an adapter that exposes reasoning events"
                .to_owned(),
        });
    }

    match provider.kind.value.as_str() {
        KIND_OPENAI_COMPATIBLE => {
            if provider.base_url.is_none() {
                return Err(ConfigError::MissingSetting {
                    key: join_key(&["providers", &provider.name.value, "base_url"]),
                    message: format!(
                        "an `{KIND_OPENAI_COMPATIBLE}` provider needs the endpoint it talks to"
                    ),
                });
            }
        }
        // The Messages API has one well-known endpoint, so `base_url` is
        // optional and defaults at provider construction; everything else a
        // provider table carries (credential, headers) applies unchanged.
        KIND_ANTHROPIC_MESSAGES => {}
        KIND_CHATGPT_RESPONSES => {
            let expected_endpoint = crate::setup::CHATGPT_ENDPOINT;
            if provider.base_url.as_ref().map(|value| value.value.as_str())
                != Some(expected_endpoint)
            {
                let source = provider.base_url.as_ref().map_or_else(
                    || provider.kind.source.clone(),
                    |value| value.source.clone(),
                );
                return Err(ConfigError::InvalidValue {
                    source,
                    message:
                        "the experimental ChatGPT provider uses Smith's fixed trusted endpoint"
                            .to_owned(),
                });
            }
            if provider.credentials.is_empty()
                || provider.api_key.is_some()
                || !provider.headers.is_empty()
            {
                let source = provider.credential().map_or_else(
                    || provider.kind.source.clone(),
                    |value| value.source.clone(),
                );
                return Err(ConfigError::InvalidValue {
                    source,
                    message: "the experimental ChatGPT provider requires Smith OAuth at `authfile:chatgpt`"
                        .to_owned(),
                });
            }
            // Every member — one account or a pool — is a Smith-owned OAuth
            // login in the owner-only auth file. Entries beyond the first take
            // a `chatgpt-` prefix, so a pool cannot smuggle in a reference
            // some other product owns and rotates underneath Smith.
            for credential in &provider.credentials {
                let entry = crate::credential::CredentialRef::parse(&credential.value)
                    .ok()
                    .and_then(|reference| match reference {
                        crate::credential::CredentialRef::AuthFile { entry } => Some(entry),
                        _ => None,
                    });
                if !entry
                    .as_deref()
                    .is_some_and(crate::setup::is_chatgpt_auth_entry)
                {
                    return Err(ConfigError::InvalidValue {
                        source: credential.source.clone(),
                        message: "the experimental ChatGPT provider requires Smith OAuth at \
                                  `authfile:chatgpt` entries (`chatgpt` or `chatgpt-<label>`)"
                            .to_owned(),
                    });
                }
            }
        }
        // A browser login, not a pasted key. An `api_key` here would be a user
        // trying to use a console key on the login adapter, which sends a
        // renewable bundle and would ignore it. A `credentials` pool is
        // allowed: each member is a stored login of its own.
        KIND_XAI_RESPONSES => {
            if provider.credential().is_none() || provider.api_key.is_some() {
                let source = provider.api_key.as_ref().map_or_else(
                    || provider.kind.source.clone(),
                    |value| value.source.clone(),
                );
                return Err(ConfigError::InvalidValue {
                    source,
                    message: format!(
                        "an `{KIND_XAI_RESPONSES}` provider carries a stored xAI login in \
                         `credential`; for a console API key use `{KIND_OPENAI_RESPONSES}`"
                    ),
                });
            }
            // Pinned, because the session this kind carries is issued by xAI's
            // own authorization server and is not a bearer any other
            // deployment would accept. A gateway takes an API key over
            // `openai-responses` instead.
            if provider.base_url.as_ref().map(|value| value.value.as_str())
                != Some(crate::setup::XAI_ENDPOINT)
            {
                let source = provider.base_url.as_ref().map_or_else(
                    || provider.kind.source.clone(),
                    |value| value.source.clone(),
                );
                return Err(ConfigError::InvalidValue {
                    source,
                    message: format!(
                        "an `{KIND_XAI_RESPONSES}` provider talks to xAI's own endpoint, \
                         which is where its login is valid"
                    ),
                });
            }
        }
        KIND_GEMINI_INTERACTIONS => {
            if let Some(base_url) = &provider.base_url {
                return Err(ConfigError::IncompatibleOption {
                    source: base_url.source.clone(),
                    kind: KIND_GEMINI_INTERACTIONS.to_owned(),
                    message:
                        "native Gemini uses Smith's fixed endpoint and does not accept `base_url`"
                            .to_owned(),
                });
            }
            if !provider.headers.is_empty() {
                let source = provider
                    .headers
                    .values()
                    .next()
                    .expect("non-empty headers have a first value")
                    .source
                    .clone();
                return Err(ConfigError::IncompatibleOption {
                    source,
                    kind: KIND_GEMINI_INTERACTIONS.to_owned(),
                    message:
                        "native Gemini uses its API-key header internally and does not accept custom headers"
                            .to_owned(),
                });
            }
            if let Some(policy) = &provider.response.reasoning_only {
                return Err(ConfigError::IncompatibleOption {
                    source: policy.source.clone(),
                    kind: KIND_GEMINI_INTERACTIONS.to_owned(),
                    message:
                        "native Gemini does not accept `response.reasoning_only`; reasoning events are preserved by the adapter"
                            .to_owned(),
                });
            }
        }
        KIND_FAKE => {
            for (source, option) in [
                (provider.base_url.as_ref(), "base_url"),
                (provider.credential(), "credential"),
            ] {
                if let Some(sourced) = source {
                    return Err(ConfigError::IncompatibleOption {
                        source: sourced.source.clone(),
                        kind: KIND_FAKE.to_owned(),
                        message: format!(
                            "the deterministic provider sends nothing, so `{option}` would never be used"
                        ),
                    });
                }
            }
            if let Some(api_key) = &provider.api_key {
                return Err(ConfigError::IncompatibleOption {
                    source: api_key.source.clone(),
                    kind: KIND_FAKE.to_owned(),
                    message:
                        "the deterministic provider sends nothing, so `api_key` would never be used"
                            .to_owned(),
                });
            }
            if let Some(value) = provider.headers.values().next() {
                return Err(ConfigError::IncompatibleOption {
                    source: value.source.clone(),
                    kind: KIND_FAKE.to_owned(),
                    message: "the deterministic provider sends no requests to add headers to"
                        .to_owned(),
                });
            }
        }
        _ => {}
    }
    Ok(())
}

fn command_option_error<T>(value: &Sourced<T>, option: &str) -> ConfigError {
    ConfigError::IncompatibleOption {
        source: value.source.clone(),
        kind: KIND_COMMAND_JSONL.to_owned(),
        message: format!(
            "`{option}` is an HTTP-provider option; pass bridge secrets only through `command.env`"
        ),
    }
}

/// Checks that a credential is a reference rather than the secret itself.
/// Adds a pool position to a credential error.
///
/// The source key names the whole `credentials` list, which is not enough to
/// find the offending entry, and the entry's *value* can never be quoted back
/// — a rejected reference is usually rejected precisely because it looks like
/// a pasted secret. The position identifies it and reveals nothing.
fn locate_pool_entry(error: ConfigError, position: usize) -> ConfigError {
    let locate = |message: String| format!("entry {} of `credentials`: {message}", position + 1);
    match error {
        ConfigError::PlaintextSecret { source, message } => ConfigError::PlaintextSecret {
            source,
            message: locate(message),
        },
        ConfigError::InvalidValue { source, message } => ConfigError::InvalidValue {
            source,
            message: locate(message),
        },
        other => other,
    }
}

pub(super) fn validate_credential(credential: &Sourced<String>) -> Result<(), ConfigError> {
    if is_credential_reference(&credential.value) {
        return Ok(());
    }
    Err(ConfigError::PlaintextSecret {
        source: credential.source.clone(),
        message: format!(
            "write a reference such as `keychain:smith/<provider>`; the schemes are {}",
            credential_schemes()
        ),
    })
}

/// Whether a value names a place a secret can be fetched from rather than
/// being the secret itself.
pub(super) fn is_credential_reference(value: &str) -> bool {
    value.split_once(':').is_some_and(|(scheme, rest)| {
        CREDENTIAL_SCHEMES.contains(&scheme) && !rest.trim().is_empty()
    })
}

/// Whether a header name carries authorization.
pub(super) fn names_an_auth_header(name: &str) -> bool {
    AUTH_HEADERS.contains(&name.to_ascii_lowercase().as_str())
}

/// Whether a variable name says it carries a credential.
pub(super) fn names_a_credential(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    CREDENTIAL_ENV_MARKERS
        .iter()
        .any(|marker| upper.contains(marker))
}

/// The credential schemes, spelled for a diagnostic.
pub(super) fn credential_schemes() -> String {
    CREDENTIAL_SCHEMES
        .iter()
        .map(|scheme| format!("`{scheme}:`"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn resolve_model_limits(
    provenance: &Provenance,
    provider: &str,
    model: &str,
) -> Result<ResolvedModelLimits, ConfigError> {
    let scope = join_key(&["models", &format!("{provider}/{model}")]);
    let declared = ResolvedModelLimits {
        context_tokens: optional_u32(provenance, &format!("{scope}.context_tokens"))?,
        max_input_tokens: optional_u32(provenance, &format!("{scope}.max_input_tokens"))?,
        max_output_tokens: optional_u32(provenance, &format!("{scope}.max_output_tokens"))?,
    };
    if crate::cli_agents::parse_cli_model_id(model).is_none() {
        return Ok(declared);
    }
    // An installed agent owns its own context, so Smith has no limits to
    // discover and nothing to enforce with them. Supply defaults rather than
    // demand a `[models]` table for a number that changes nothing, while
    // still letting an owner override them.
    let built_in = |key: &str, fallback: u32| {
        Sourced::new(fallback, Source::built_in(format!("{scope}.{key}")))
    };
    Ok(ResolvedModelLimits {
        context_tokens: declared.context_tokens.or_else(|| {
            Some(built_in(
                "context_tokens",
                crate::cli_agents::CLI_AGENT_CONTEXT_TOKENS,
            ))
        }),
        max_input_tokens: declared.max_input_tokens.or_else(|| {
            Some(built_in(
                "max_input_tokens",
                crate::cli_agents::CLI_AGENT_MAX_INPUT_TOKENS,
            ))
        }),
        max_output_tokens: declared.max_output_tokens.or_else(|| {
            Some(built_in(
                "max_output_tokens",
                crate::cli_agents::CLI_AGENT_MAX_OUTPUT_TOKENS,
            ))
        }),
    })
}

pub(super) fn resolve_model_reasoning(
    provenance: &Provenance,
    provider: &str,
    model: &str,
) -> Result<ResolvedModelReasoning, ConfigError> {
    let scope = format!(
        "{}.reasoning",
        join_key(&["models", &format!("{provider}/{model}")])
    );
    let dialect = text(provenance, &format!("{scope}.dialect"))?
        .map(|raw| {
            let Some(value) = ReasoningDialect::ALL
                .into_iter()
                .find(|dialect| dialect.as_str() == raw.value)
            else {
                let supported = ReasoningDialect::ALL
                    .map(|dialect| format!("`{}`", dialect.as_str()))
                    .join(", ");
                return Err(ConfigError::InvalidValue {
                    source: raw.source,
                    message: format!(
                        "`{}` is not a reasoning dialect; use {supported}",
                        raw.value
                    ),
                });
            };
            Ok(Sourced::new(value, raw.source))
        })
        .transpose()?;
    let efforts = list(provenance, &format!("{scope}.efforts"))?;
    if let Some(efforts) = &efforts {
        if efforts.value.is_empty() {
            return Err(ConfigError::InvalidValue {
                source: efforts.source.clone(),
                message: "`efforts` must contain at least one advertised value".to_owned(),
            });
        }
        let mut seen = BTreeSet::new();
        for effort in &efforts.value {
            if effort.is_empty()
                || effort.len() > 32
                || !effort
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(ConfigError::InvalidValue {
                    source: efforts.source.clone(),
                    message: format!(
                        "reasoning effort `{effort}` must contain 1 to 32 ASCII letters, digits, `-`, or `_`"
                    ),
                });
            }
            if !seen.insert(effort) {
                return Err(ConfigError::InvalidValue {
                    source: efforts.source.clone(),
                    message: format!("`efforts` contains duplicate value `{effort}`"),
                });
            }
        }
    }
    Ok(ResolvedModelReasoning {
        toggle: flag(provenance, &format!("{scope}.toggle"))?,
        mandatory: flag(provenance, &format!("{scope}.mandatory"))?,
        efforts,
        default_enabled: flag(provenance, &format!("{scope}.default_enabled"))?,
        default_effort: text(provenance, &format!("{scope}.default_effort"))?,
        dialect,
    })
}

pub(super) fn resolve_context(
    provenance: &Provenance,
    model_limits: &ResolvedModelLimits,
    synthetic_cache_spend: SyntheticCacheSpendAuthority,
) -> Result<ResolvedContext, ConfigError> {
    let high = required_percent(provenance, "context.compaction_high_watermark_percent")?;
    let low = required_percent(provenance, "context.compaction_low_watermark_percent")?;
    if low.value >= high.value {
        return Err(ConfigError::InvalidValue {
            source: low.source,
            message: format!(
                "compaction must leave room below the watermark it triggers at ({}%)",
                high.value
            ),
        });
    }
    let cache = resolve_cache_policy(provenance, model_limits, synthetic_cache_spend)?;
    // Keep the old resolved field as a compatibility projection.  It points
    // at the exact same sourced winner as the replacement field, so no second
    // inactivity timer can be constructed downstream.
    let idle_compaction_ms = cache.inactivity_limit_ms.clone();
    Ok(ResolvedContext {
        output_reserve: optional_u32(provenance, "context.output_reserve")?,
        reasoning_reserve: required_u32(provenance, "context.reasoning_reserve")?,
        capability_budget: optional_u32(provenance, "context.capability_budget")?,
        max_estimated_slack: optional_u32(provenance, "context.max_estimated_slack")?,
        compaction_high_watermark_percent: high,
        compaction_low_watermark_percent: low,
        idle_compaction_ms,
        cache,
    })
}

/// Resolves the typed cache lifecycle policy and applies host-only authority
/// narrowing.  The repository/project layers can request adaptive behavior,
/// but they cannot manufacture `SyntheticCacheSpendAuthority::Allow`.
pub(super) fn resolve_cache_policy(
    provenance: &Provenance,
    model_limits: &ResolvedModelLimits,
    synthetic_cache_spend: SyntheticCacheSpendAuthority,
) -> Result<ResolvedCachePolicy, ConfigError> {
    let requested_raw = text(provenance, "context.cache.maintenance")?.unwrap_or_else(|| {
        Sourced::new(
            "off".to_owned(),
            Source::built_in("context.cache.maintenance"),
        )
    });
    let requested = CacheMaintenanceMode::parse(&requested_raw.value).ok_or_else(|| {
        ConfigError::InvalidValue {
            source: requested_raw.source.clone(),
            message: format!(
                "`{}` is not a cache maintenance mode; the modes are {}",
                requested_raw.value,
                list_spellings(CacheMaintenanceMode::spellings())
            ),
        }
    })?;
    let requested_maintenance = Sourced::new(requested, requested_raw.source.clone());
    let (effective, narrowing_reason) = match (requested, synthetic_cache_spend) {
        (CacheMaintenanceMode::Adaptive, SyntheticCacheSpendAuthority::Deny) => (
            CacheMaintenanceMode::Observe,
            Some(
                "requested adaptive maintenance narrowed to observe: host synthetic_cache_spend authority is deny"
                    .to_owned(),
            ),
        ),
        (mode, _) => (mode, None),
    };
    let effective_maintenance = Sourced::new(effective, requested_raw.source.clone());

    let inactivity_limit_ms = bounded_u64(
        provenance,
        "context.cache.inactivity_limit_ms",
        1_000,
        86_400_000,
    )?;
    let max_hold_while_child_ms = bounded_u64(
        provenance,
        "context.cache.max_hold_while_child_ms",
        0,
        86_400_000,
    )?;
    let max_maintenance_calls =
        bounded_u8(provenance, "context.cache.max_maintenance_calls", 0, 8)?;
    let max_maintenance_input_tokens =
        required_u32(provenance, "context.cache.max_maintenance_input_tokens")?;
    if max_maintenance_input_tokens.value != 0 {
        let Some(limit) = model_limits
            .max_input_tokens
            .as_ref()
            .map(|limit| limit.value)
        else {
            return Err(ConfigError::InvalidValue {
                source: max_maintenance_input_tokens.source,
                message: "context.cache.max_maintenance_input_tokens requires a resolved model input limit when nonzero".to_owned(),
            });
        };
        if max_maintenance_input_tokens.value > limit {
            return Err(ConfigError::InvalidValue {
                source: max_maintenance_input_tokens.source,
                message: format!(
                    "context.cache.max_maintenance_input_tokens must be 0 or no greater than the resolved model input limit ({limit})"
                ),
            });
        }
    }
    let max_maintenance_output_tokens = bounded_u32(
        provenance,
        "context.cache.max_maintenance_output_tokens",
        1,
        4_096,
    )?;
    let maintenance_deadline_ms = bounded_u64(
        provenance,
        "context.cache.maintenance_deadline_ms",
        1,
        120_000,
    )?;
    let keepalive_margin_ms = bounded_u64(
        provenance,
        "context.cache.keepalive_margin_ms",
        0,
        inactivity_limit_ms.value,
    )?;
    let keepalive_jitter_percent =
        bounded_u8(provenance, "context.cache.keepalive_jitter_percent", 0, 50)?;
    let handoff_checkpoint = required_flag(provenance, "context.cache.handoff_checkpoint")?;
    let idle_compaction = required_flag(provenance, "context.cache.idle_compaction")?;
    let resume_capsule = required_flag(provenance, "context.cache.resume_capsule")?;

    Ok(ResolvedCachePolicy {
        requested_maintenance,
        effective_maintenance,
        narrowing_reason,
        inactivity_limit_ms,
        max_hold_while_child_ms,
        max_maintenance_calls,
        max_maintenance_input_tokens,
        max_maintenance_output_tokens,
        maintenance_deadline_ms,
        keepalive_margin_ms,
        keepalive_jitter_percent,
        handoff_checkpoint,
        idle_compaction,
        resume_capsule,
    })
}

/// Resolves the bounded parent wait policy. Missing values are deliberately
/// filled with built-in sources so a profile that predates the new table stays
/// usable during the migration window.
pub(super) fn resolve_child_agents(
    provenance: &Provenance,
) -> Result<ResolvedChildAgents, ConfigError> {
    let default_timeout = bounded_u64_or_default(
        provenance,
        "child_agents.wait_default_timeout_ms",
        0,
        300_000,
        300_000,
    )?;
    let max_timeout = bounded_u64_or_default(
        provenance,
        "child_agents.wait_max_timeout_ms",
        1,
        300_000,
        300_000,
    )?;
    if default_timeout.value > max_timeout.value {
        return Err(ConfigError::InvalidValue {
            source: default_timeout.source,
            message: format!(
                "child_agents.wait_default_timeout_ms ({}) must not exceed wait_max_timeout_ms ({})",
                default_timeout.value, max_timeout.value
            ),
        });
    }
    Ok(ResolvedChildAgents {
        wait_default_timeout_ms: default_timeout,
        wait_max_timeout_ms: max_timeout,
    })
}

pub(super) fn resolve_limits(provenance: &Provenance) -> Result<ResolvedLimits, ConfigError> {
    Ok(ResolvedLimits {
        max_retries: required_u32(provenance, "limits.max_retries")?,
        max_tool_steps: required_u32(provenance, "limits.max_tool_steps")?,
        turn_time_limit_ms: required_u64(provenance, "limits.turn_time_limit_ms")?,
        tool_output_limit_bytes: required_u64(provenance, "limits.tool_output_limit_bytes")?,
    })
}

pub(super) fn resolve_persistence(
    provenance: &Provenance,
) -> Result<ResolvedPersistence, ConfigError> {
    let sessions_dir = required_text(provenance, "persistence.sessions_dir")?;
    let checkpoint_key = secret(provenance, "persistence.checkpoint_key")?;
    let checkpoint_key_credential = text(provenance, "persistence.checkpoint_key_credential")?;
    if let (Some(key), Some(_credential)) = (&checkpoint_key, &checkpoint_key_credential) {
        return Err(ConfigError::InvalidValue {
            source: key.source.clone(),
            message: "choose exactly one checkpoint key source: `checkpoint_key` or `checkpoint_key_credential`"
                .to_owned(),
        });
    }
    if let Some(key) = &checkpoint_key {
        let exposed = key.value.expose();
        if exposed.len() != 64 || !exposed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ConfigError::InvalidValue {
                source: key.source.clone(),
                message:
                    "`checkpoint_key` must encode exactly 32 bytes as 64 hexadecimal characters"
                        .to_owned(),
            });
        }
    }
    if let Some(credential) = &checkpoint_key_credential {
        validate_credential(credential)?;
    }
    Ok(ResolvedPersistence {
        enabled: required_flag(provenance, "persistence.enabled")?,
        sessions_dir: Sourced::new(PathBuf::from(sessions_dir.value), sessions_dir.source),
        journal_events: required_flag(provenance, "persistence.journal_events")?,
        checkpoint_key,
        checkpoint_key_credential,
    })
}

pub(super) fn resolve_approval(provenance: &Provenance) -> Result<ResolvedApproval, ConfigError> {
    let raw = required_text(provenance, "approval.mode")?;
    let mode = ApprovalMode::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
        source: raw.source.clone(),
        message: format!(
            "`{}` is not an approval mode; the modes are {}",
            raw.value,
            list_spellings(ApprovalMode::spellings())
        ),
    })?;
    let legacy_auto_approve = list(provenance, "approval.auto_approve")?;
    if let Some(legacy) = &legacy_auto_approve
        && !legacy.value.is_empty()
    {
        return Err(ConfigError::InvalidValue {
            source: legacy.source.clone(),
            message: "non-empty `approval.auto_approve` tool-name lists are no longer supported; migrate to versioned `[[approval.auto]]` prepared-call rules, or use user-owned `approval.mode = \"allow-all\"` only for deliberately unrestricted automation"
                .to_owned(),
        });
    }
    let auto = text(provenance, "approval.auto")?
        .map(resolve_auto_approval_rules)
        .transpose()?
        .unwrap_or_default();
    Ok(ResolvedApproval {
        mode: Sourced::new(mode, raw.source),
        auto_approve: legacy_auto_approve,
        auto,
    })
}

fn resolve_auto_approval_rules(
    encoded: Sourced<String>,
) -> Result<Vec<Sourced<AutoApprovalRule>>, ConfigError> {
    let rules: Vec<AutoApprovalRuleSection> =
        serde_json::from_str(&encoded.value).map_err(|error| ConfigError::InvalidValue {
            source: encoded.source.clone(),
            message: format!("`approval.auto` is not a valid rule list: {error}"),
        })?;
    rules
        .into_iter()
        .enumerate()
        .map(|(index, rule)| {
            let mut source = encoded.source.clone();
            source.key = format!("{}[{index}]", source.key);
            validate_auto_approval_rule(rule, source)
        })
        .collect()
}

fn validate_auto_approval_rule(
    rule: AutoApprovalRuleSection,
    source: Source,
) -> Result<Sourced<AutoApprovalRule>, ConfigError> {
    let invalid = |message: String| ConfigError::InvalidValue {
        source: source.clone(),
        message,
    };
    if rule.revision != 1 {
        return Err(invalid(format!(
            "unsupported automatic approval rule revision {}; only revision 1 is defined",
            rule.revision
        )));
    }
    let Some((module, tool)) = rule.tool.split_once('/') else {
        return Err(invalid(
            "automatic approval rule `tool` must be module-qualified, for example `smith/edit`"
                .to_owned(),
        ));
    };
    if module.is_empty() || tool.is_empty() || tool.contains('/') {
        return Err(invalid(
            "automatic approval rule `tool` must contain exactly one non-empty `/` separator"
                .to_owned(),
        ));
    }
    if rule.tool != "smith/edit" {
        return Err(invalid(format!(
            "automatic approval for `{}` is not supported; revision 1 permits only `smith/edit`",
            rule.tool
        )));
    }
    if rule.operations.is_empty() {
        return Err(invalid(
            "automatic approval rule `operations` cannot be empty".to_owned(),
        ));
    }
    if rule.permissions.is_empty() {
        return Err(invalid(
            "automatic approval rule `permissions` cannot be empty".to_owned(),
        ));
    }
    if rule.paths.is_empty() {
        return Err(invalid(
            "automatic approval rule `paths` cannot be empty".to_owned(),
        ));
    }
    for pattern in &rule.paths {
        let path = Path::new(pattern);
        if pattern.is_empty()
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(invalid(format!(
                "automatic approval path pattern `{pattern}` must be project-relative and cannot contain `..`"
            )));
        }
        globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|error| {
                invalid(format!(
                    "automatic approval path pattern `{pattern}` is invalid: {error}"
                ))
            })?;
    }
    if rule.max_uses == Some(0) {
        return Err(invalid(
            "automatic approval rule `max_uses` must be greater than zero".to_owned(),
        ));
    }
    let expires_at_unix_ms = rule
        .expires_at
        .as_deref()
        .map(|value| {
            time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
                .map(|timestamp| timestamp.unix_timestamp_nanos() / 1_000_000)
                .map_err(|error| {
                    invalid(format!(
                        "automatic approval rule `expires_at` must be RFC 3339: {error}"
                    ))
                })
        })
        .transpose()?;

    Ok(Sourced::new(
        AutoApprovalRule {
            revision: rule.revision,
            tool: rule.tool,
            operations: rule.operations,
            permissions: rule.permissions,
            max_risk: rule.max_risk,
            mount: rule.mount,
            paths: rule.paths,
            expires_at_unix_ms,
            max_uses: rule.max_uses,
        },
        source,
    ))
}

pub(super) fn resolve_background(
    provenance: &Provenance,
) -> Result<ResolvedBackground, ConfigError> {
    let raw = required_text(provenance, "background.exit_policy")?;
    let policy = BackgroundExit::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
        source: raw.source.clone(),
        message: format!(
            "`{}` is not a background-exit policy; the policies are {}",
            raw.value,
            list_spellings(BackgroundExit::spellings())
        ),
    })?;
    Ok(ResolvedBackground {
        exit_policy: Sourced::new(policy, raw.source),
        max_children: required_u32(provenance, "background.max_children")?,
        max_monitors: required_u32(provenance, "background.max_monitors")?,
    })
}

pub(super) fn list_spellings(spellings: &[&str]) -> String {
    spellings
        .iter()
        .map(|spelling| format!("`{spelling}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn text(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<String>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Text(value) => {
                Ok(Some(Sourced::new(value.clone(), entry.source.clone())))
            }
            other => Err(wrong_kind(entry, other, "a string")),
        },
    }
}

pub(super) fn secret(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<Secret>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Secret(value) => {
                Ok(Some(Sourced::new(value.clone(), entry.source.clone())))
            }
            other => Err(wrong_kind(entry, other, "a secret string")),
        },
    }
}

pub(super) fn integer(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<i64>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Integer(value) => Ok(Some(Sourced::new(*value, entry.source.clone()))),
            other => Err(wrong_kind(entry, other, "a whole number")),
        },
    }
}

pub(super) fn flag(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<bool>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Flag(value) => Ok(Some(Sourced::new(*value, entry.source.clone()))),
            other => Err(wrong_kind(entry, other, "`true` or `false`")),
        },
    }
}

pub(super) fn list(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<Vec<String>>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::List(value) => {
                Ok(Some(Sourced::new(value.clone(), entry.source.clone())))
            }
            other => Err(wrong_kind(entry, other, "a list of strings")),
        },
    }
}

pub(super) fn wrong_kind(entry: &Entry, found: &SettingValue, expected: &str) -> ConfigError {
    ConfigError::InvalidValue {
        source: entry.source.clone(),
        message: format!("expected {expected}, found {}", describe(found.kind())),
    }
}

pub(super) fn describe(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Text => "a string",
        ValueKind::Secret => "a secret string",
        ValueKind::Integer => "a whole number",
        ValueKind::Flag => "a boolean",
        ValueKind::List => "a list",
    }
}

pub(super) fn optional_u32(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<u32>>, ConfigError> {
    integer(provenance, key)?.map(narrow_u32).transpose()
}

pub(super) fn required_u32(
    provenance: &Provenance,
    key: &str,
) -> Result<Sourced<u32>, ConfigError> {
    narrow_u32(integer(provenance, key)?.ok_or_else(|| missing(key))?)
}

pub(super) fn required_u64(
    provenance: &Provenance,
    key: &str,
) -> Result<Sourced<u64>, ConfigError> {
    let value = integer(provenance, key)?.ok_or_else(|| missing(key))?;
    match u64::try_from(value.value) {
        Ok(narrowed) => Ok(Sourced::new(narrowed, value.source)),
        Err(_) => Err(out_of_range(value, "0 and 9223372036854775807")),
    }
}

fn bounded_u64(
    provenance: &Provenance,
    key: &str,
    minimum: u64,
    maximum: u64,
) -> Result<Sourced<u64>, ConfigError> {
    let value = required_u64(provenance, key)?;
    if (minimum..=maximum).contains(&value.value) {
        Ok(value)
    } else {
        Err(ConfigError::InvalidValue {
            source: value.source,
            message: format!("{key} must be between {minimum} and {maximum}"),
        })
    }
}

fn bounded_u64_or_default(
    provenance: &Provenance,
    key: &str,
    minimum: u64,
    maximum: u64,
    default: u64,
) -> Result<Sourced<u64>, ConfigError> {
    match provenance.winner(key) {
        Some(_) => bounded_u64(provenance, key, minimum, maximum),
        None => Ok(Sourced::new(default, Source::built_in(key))),
    }
}

fn bounded_u8(
    provenance: &Provenance,
    key: &str,
    minimum: u8,
    maximum: u8,
) -> Result<Sourced<u8>, ConfigError> {
    let value = integer(provenance, key)?.ok_or_else(|| missing(key))?;
    match u8::try_from(value.value) {
        Ok(narrowed) if (minimum..=maximum).contains(&narrowed) => {
            Ok(Sourced::new(narrowed, value.source))
        }
        _ => Err(ConfigError::InvalidValue {
            source: value.source,
            message: format!("{key} must be between {minimum} and {maximum}"),
        }),
    }
}

fn bounded_u32(
    provenance: &Provenance,
    key: &str,
    minimum: u32,
    maximum: u32,
) -> Result<Sourced<u32>, ConfigError> {
    let value = required_u32(provenance, key)?;
    if (minimum..=maximum).contains(&value.value) {
        Ok(value)
    } else {
        Err(ConfigError::InvalidValue {
            source: value.source,
            message: format!("{key} must be between {minimum} and {maximum}"),
        })
    }
}

pub(super) fn required_percent(
    provenance: &Provenance,
    key: &str,
) -> Result<Sourced<u8>, ConfigError> {
    let value = integer(provenance, key)?.ok_or_else(|| missing(key))?;
    match u8::try_from(value.value) {
        Ok(narrowed) if narrowed <= 100 => Ok(Sourced::new(narrowed, value.source)),
        _ => Err(out_of_range(value, "0 and 100")),
    }
}

pub(super) fn required_text(
    provenance: &Provenance,
    key: &str,
) -> Result<Sourced<String>, ConfigError> {
    text(provenance, key)?.ok_or_else(|| missing(key))
}

pub(super) fn required_flag(
    provenance: &Provenance,
    key: &str,
) -> Result<Sourced<bool>, ConfigError> {
    flag(provenance, key)?.ok_or_else(|| missing(key))
}

pub(super) fn narrow_u32(value: Sourced<i64>) -> Result<Sourced<u32>, ConfigError> {
    match u32::try_from(value.value) {
        Ok(narrowed) => Ok(Sourced::new(narrowed, value.source)),
        Err(_) => Err(out_of_range(value, "0 and 4294967295")),
    }
}

pub(super) fn out_of_range(value: Sourced<i64>, range: &str) -> ConfigError {
    ConfigError::InvalidValue {
        source: value.source,
        message: format!("{} is outside {range}", value.value),
    }
}

/// A setting no layer supplied. No file is named because none is at fault: the
/// value is absent everywhere, including from Smith's own defaults.
pub(super) fn missing(key: &str) -> ConfigError {
    ConfigError::MissingSetting {
        key: key.to_owned(),
        message: "no layer supplied a value".to_owned(),
    }
}

pub(super) fn unquote_segment(segment: &str) -> String {
    segment
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .map_or_else(|| segment.to_owned(), |inner| inner.replace("\\\"", "\""))
}

/// Known names close enough to `candidate` to be worth suggesting.
///
/// The threshold grows with the length of the word so that `mdel` suggests
/// `model` without a twenty-character key suggesting every other one.
pub(super) fn nearest<'a>(candidate: &str, known: impl Iterator<Item = &'a str>) -> Vec<String> {
    let budget = (candidate.chars().count() / 3).max(1);
    let mut scored: Vec<(usize, &str)> = known
        .map(|name| (distance(candidate, name), name))
        .filter(|(score, _)| *score <= budget)
        .collect();
    scored.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));
    scored
        .into_iter()
        .take(3)
        .map(|(_, name)| name.to_owned())
        .collect()
}

/// Edit distance, case-insensitive, counting a swap of two neighbours as one
/// edit.
///
/// Plain Levenshtein charges two edits for a transposition, which is the most
/// common typo of all: `modle` would then be as far from `model` as a word
/// with two unrelated mistakes, and the suggestion a user most needs would be
/// the one they never see.
pub(super) fn distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.to_lowercase().chars().collect();
    let right: Vec<char> = right.to_lowercase().chars().collect();
    let mut grid = vec![vec![0usize; right.len() + 1]; left.len() + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in grid[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=left.len() {
        for j in 1..=right.len() {
            let cost = usize::from(left[i - 1] != right[j - 1]);
            let mut best = (grid[i - 1][j - 1] + cost)
                .min(grid[i - 1][j] + 1)
                .min(grid[i][j - 1] + 1);
            if i > 1 && j > 1 && left[i - 1] == right[j - 2] && left[i - 2] == right[j - 1] {
                best = best.min(grid[i - 2][j - 2] + 1);
            }
            grid[i][j] = best;
        }
    }
    grid[left.len()][right.len()]
}
