//! Model Context Protocol server declarations, resolved and validated.
//!
//! A declaration says where a server is and what it would be given. It does
//! not say that Smith may run it: spawning a declared command is an
//! executable-trust decision over the *resolved* invocation, asked once per
//! content digest, and it lives in [`crate::trust`]. Nothing here spawns a
//! process, opens a socket, or reads a secret — an environment value naming a
//! credential is carried on as a reference and resolved behind the trust
//! boundary, exactly like a provider's.

use std::collections::BTreeMap;

use super::load::{Declarations, join_key};
use super::provenance::*;
use super::provider::{
    credential_schemes, flag, list, names_an_auth_header, text, unquote_segment,
    validate_credential, wrong_kind,
};
use super::types::*;

/// The widest server name that leaves room for a tool name inside the
/// `mcp__<server>__<tool>` form providers accept.
const MAX_SERVER_NAME_CHARS: usize = 48;

/// The URL schemes the streamable HTTP transport can actually reach.
const HTTP_SCHEMES: &[&str] = &["http://", "https://"];

/// Resolves every declared server through the layered ledger.
///
/// Each server's fields resolve key by key, so a project layer that overrides
/// one argument list does not silently discard the user layer's environment,
/// and every field reports the layer that supplied it.
pub(super) fn resolve_mcp(
    provenance: &Provenance,
    declared: &Declarations,
) -> Result<ResolvedMcp, ConfigError> {
    let mut servers = BTreeMap::new();
    for (name, source) in &declared.mcp_servers {
        let server = resolve_server(provenance, name, source)?;
        servers.insert(name.clone(), server);
    }
    Ok(ResolvedMcp { servers })
}

fn resolve_server(
    provenance: &Provenance,
    name: &str,
    declaration: &Source,
) -> Result<ResolvedMcpServer, ConfigError> {
    validate_name(name, declaration)?;
    let scope = join_key(&["mcp", "servers", name]);

    let transport = resolve_transport(provenance, name, &scope, declaration)?;
    let env = resolve_env(provenance, &scope)?;
    let enabled = flag(provenance, &format!("{scope}.enabled"))?
        .unwrap_or_else(|| Sourced::new(true, Source::built_in(format!("{scope}.enabled"))));

    Ok(ResolvedMcpServer {
        name: name.to_owned(),
        source: declaration.clone(),
        transport,
        env,
        enabled,
    })
}

/// Checks the name against the grammar a provider will accept once it is
/// embedded in `mcp__<server>__<tool>`.
///
/// The rule is shared with `agent_runtime_mcp::naming`, which enforces it again
/// on the tool half. It is repeated here so a bad name is a configuration
/// diagnostic naming the file it was written in, rather than a connection that
/// fails later for a reason the user cannot place.
fn validate_name(name: &str, declaration: &Source) -> Result<(), ConfigError> {
    let refused = |message: String| ConfigError::InvalidValue {
        source: declaration.clone(),
        message,
    };
    if name.is_empty() {
        return Err(refused("a server name cannot be empty".to_owned()));
    }
    if name.chars().count() > MAX_SERVER_NAME_CHARS {
        return Err(refused(format!(
            "a server name is at most {MAX_SERVER_NAME_CHARS} characters, because it is \
             part of every one of its tools' names"
        )));
    }
    if let Some(character) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
    {
        return Err(refused(format!(
            "a server name may hold ASCII letters, digits, `-`, and `_`; `{character}` is \
             not accepted in a tool name"
        )));
    }
    if name.contains("__") {
        return Err(refused(
            "a server name cannot contain `__`, which separates the server from the tool \
             in `mcp__<server>__<tool>`"
                .to_owned(),
        ));
    }
    Ok(())
}

fn resolve_transport(
    provenance: &Provenance,
    name: &str,
    scope: &str,
    declaration: &Source,
) -> Result<ResolvedMcpTransport, ConfigError> {
    let command = text(provenance, &format!("{scope}.command"))?;
    let url = text(provenance, &format!("{scope}.url"))?;

    match (command, url) {
        (Some(command), None) => {
            if command.value.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    source: command.source,
                    message: "a server's `command` cannot be empty".to_owned(),
                });
            }
            // A local child process has no endpoint to authenticate to. An
            // option that could never be used is refused rather than ignored:
            // silently dropping a declared credential is how a user comes to
            // believe a server is authenticated when it is not.
            reject_unusable(provenance, scope, "credential", "a local server")?;
            reject_unusable_map(provenance, scope, "headers", "a local server")?;
            Ok(ResolvedMcpTransport::Stdio {
                command,
                args: list(provenance, &format!("{scope}.args"))?,
            })
        }
        (None, Some(url)) => {
            if !HTTP_SCHEMES
                .iter()
                .any(|scheme| url.value.starts_with(scheme))
            {
                return Err(ConfigError::InvalidValue {
                    source: url.source,
                    message: "a server `url` names the streamable HTTP transport, so it \
                              starts with `http://` or `https://`"
                        .to_owned(),
                });
            }
            reject_unusable(provenance, scope, "args", "a remote server")?;
            reject_unusable_map(provenance, scope, "env", "a remote server")?;
            let credential = text(provenance, &format!("{scope}.credential"))?;
            if let Some(credential) = &credential {
                validate_credential(credential)?;
            }
            Ok(ResolvedMcpTransport::StreamableHttp {
                url,
                credential,
                headers: resolve_headers(provenance, scope)?,
            })
        }
        (Some(_), Some(url)) => Err(ConfigError::InvalidValue {
            source: url.source,
            message: format!(
                "server `{name}` names two transports: choose `command` for a local \
                 server or `url` for a remote one"
            ),
        }),
        (None, None) => Err(ConfigError::MissingSetting {
            key: format!("{scope}.command"),
            message: format!(
                "server `{name}` (declared at {declaration}) must say how to reach it: \
                 `command` for a local server, or `url` for a remote one"
            ),
        }),
    }
}

/// Refuses a scalar option the chosen transport could never use.
fn reject_unusable(
    provenance: &Provenance,
    scope: &str,
    option: &str,
    transport: &str,
) -> Result<(), ConfigError> {
    match provenance.winner(&format!("{scope}.{option}")) {
        None => Ok(()),
        Some(entry) => Err(ConfigError::InvalidValue {
            source: entry.source.clone(),
            message: format!("{transport} has no use for `{option}`"),
        }),
    }
}

/// Refuses a table of options the chosen transport could never use.
fn reject_unusable_map(
    provenance: &Provenance,
    scope: &str,
    option: &str,
    transport: &str,
) -> Result<(), ConfigError> {
    let prefix = format!("{scope}.{option}.");
    match provenance.keys().find(|key| key.starts_with(&prefix)) {
        None => Ok(()),
        Some(key) => {
            let source = provenance
                .winner(key)
                .map(|entry| entry.source.clone())
                .expect("a key found in the ledger has a winner");
            Err(ConfigError::InvalidValue {
                source,
                message: format!("{transport} has no use for `{option}`"),
            })
        }
    }
}

/// Reads a remote server's headers, refusing an authorization value written
/// where a reference belongs.
///
/// The rule is the provider table's, applied to the same class of value: a
/// header that carries authorization must name where its secret comes from,
/// because a token written in a file is a token in a backup, a diff, and a
/// screen share.
fn resolve_headers(
    provenance: &Provenance,
    scope: &str,
) -> Result<BTreeMap<String, Sourced<McpValue>>, ConfigError> {
    let headers = resolve_values(provenance, &format!("{scope}.headers."))?;
    for (name, value) in &headers {
        if names_an_auth_header(name) && value.value.credential().is_none() {
            return Err(ConfigError::PlaintextSecret {
                source: value.source.clone(),
                message: format!(
                    "write a reference for the `{name}` header, or use `credential` \
                     to send a bearer token; the schemes are {}",
                    credential_schemes()
                ),
            });
        }
    }
    Ok(headers)
}

/// Reads every variable declared under one server's `env` table.
///
/// The classification was made when the layer was flattened: a reference stays
/// text and a literal is already secret-bearing. Reading it back that way keeps
/// the two apart without this function ever seeing a value it would have to
/// decide about.
fn resolve_env(
    provenance: &Provenance,
    scope: &str,
) -> Result<BTreeMap<String, Sourced<McpValue>>, ConfigError> {
    resolve_values(provenance, &format!("{scope}.env."))
}

/// Reads every value written under one dotted prefix.
///
/// The classification was made when the layer was flattened: a reference stays
/// text and a literal is already secret-bearing. Reading it back that way keeps
/// the two apart without this function ever seeing a value it would have to
/// decide about.
fn resolve_values(
    provenance: &Provenance,
    prefix: &str,
) -> Result<BTreeMap<String, Sourced<McpValue>>, ConfigError> {
    let keys: Vec<String> = provenance
        .keys()
        .filter(|key| key.starts_with(prefix))
        .map(str::to_owned)
        .collect();

    let mut values = BTreeMap::new();
    for key in keys {
        let name = unquote_segment(&key[prefix.len()..]);
        let Some(entry) = provenance.winner(&key) else {
            continue;
        };
        let value = match &entry.value {
            SettingValue::Text(reference) => McpValue::Credential(reference.clone()),
            SettingValue::Secret(literal) => McpValue::Literal(literal.clone()),
            other => {
                return Err(wrong_kind(entry, other, "a string"));
            }
        };
        values.insert(name, Sourced::new(value, entry.source.clone()));
    }
    Ok(values)
}
