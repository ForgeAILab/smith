//! Safe-boundary provider connection orchestration.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use agent_runtime_core::store::Secret;
use anyhow::{Context, Result};
use smith_config::credential::{CredentialEnroller, CredentialRef, EnrollmentReceipt};
use smith_config::inventory::local_inventory;
use smith_config::model::{
    ConfigFile, KIND_CHATGPT_RESPONSES, KIND_XAI_RESPONSES, ModelReasoningSection, ModelSection,
    ProviderSection, ReasoningDialect,
};
use smith_config::setup::{
    CHATGPT_CREDENTIAL, CHATGPT_ENDPOINT, CHATGPT_PROVIDER, CHATGPT_TERRA, GOOGLE_PROVIDER,
    XAI_CREDENTIAL, XAI_DEFAULT_MODEL, XAI_ENDPOINT, XAI_PROVIDER,
};
use smith_config::user_config::{
    CommittedConfigEdit, prepare_provider_credential_removal, prepare_user_config_edit,
};
use smith_host::ProjectWorkspace;
use smith_runtime::factory::{self, HostSurface, RuntimeRequest};
use smith_tui::ResourceEntry;
use smith_tui::setup::SetupMode;

use crate::cli::Selection;
use crate::{AVAILABLE_ADAPTER_KINDS, chatgpt, prepare, setup};

/// How `/connect` proceeds for a login-kind provider.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectMode {
    /// First connection, or an explicit replacement of whatever is stored.
    Replace,
    /// Keep the stored login(s) and add another account to the pool.
    Add {
        /// The references already declared, in pool order.
        existing: Vec<String>,
    },
}

/// The user-layer login references already declared for `provider`, when its
/// section carries the expected login kind.
///
/// Read from the user file directly rather than the resolution: only the user
/// layer may hold these credentials, and an add must extend exactly what that
/// file says rather than a merged view of it.
fn existing_login_references(user_dir: &Path, provider: &str, kind: &str) -> Option<Vec<String>> {
    let text = fs::read_to_string(user_dir.join("config.toml")).ok()?;
    let file = ConfigFile::parse(&text).ok()?;
    let section = file.providers.get(provider)?;
    if section.kind.as_deref() != Some(kind) {
        return None;
    }
    let references: Vec<String> = if section.credentials.is_empty() {
        section.credential.clone().into_iter().collect()
    } else {
        section.credentials.clone()
    };
    (!references.is_empty()).then_some(references)
}

/// Asks whether to replace the stored login or add another account.
async fn choose_connect_mode(
    display: &str,
    existing: &[String],
    no_color: bool,
    no_motion: bool,
) -> Result<Option<ConnectMode>> {
    let accounts = existing.len();
    let picked = crate::resources::pick_one(
        &format!("Connect {display} · already connected"),
        vec![
            ResourceEntry::new(
                "add",
                "Add another account",
                "usage-aware pool · /account switches between them",
            ),
            ResourceEntry::new(
                "replace",
                "Replace the stored login",
                if accounts > 1 {
                    "sign in again as a single account, discarding the pool"
                } else {
                    "sign in again"
                },
            ),
        ],
        "No connection choices",
        no_color,
        no_motion,
    )
    .await?;
    Ok(picked.and_then(|choice| match choice.as_str() {
        "add" => Some(ConnectMode::Add {
            existing: existing.to_vec(),
        }),
        "replace" => Some(ConnectMode::Replace),
        _ => None,
    }))
}

/// The first free numbered auth-file entry (`chatgpt` → `chatgpt-2`, …).
///
/// Numbered from 2 because entry 1 is the unnumbered original. Only declared
/// references count as taken: an orphaned entry left behind by an earlier
/// replacement is reused rather than skipped forever.
fn next_pool_entry(prefix: &str, existing: &[String]) -> String {
    let taken: BTreeSet<String> = existing
        .iter()
        .filter_map(|reference| CredentialRef::parse(reference).ok())
        .filter_map(|reference| match reference {
            CredentialRef::AuthFile { entry } => Some(entry),
            _ => None,
        })
        .collect();
    (2_u32..)
        .map(|number| format!("{prefix}-{number}"))
        .find(|candidate| !taken.contains(candidate))
        .expect("an unbounded counter finds a free entry")
}

/// Publishes one login connection: the stored credential and the published
/// configuration commit together, or the whole edit unwinds.
///
/// The returned pieces stay live so a caller with a preflight can still roll
/// both halves back; a caller without one accepts immediately.
async fn publish_login(
    user_dir: &Path,
    display: &str,
    reference: &CredentialRef,
    secret: Secret,
    patch: &ConfigFile,
) -> Result<(CommittedConfigEdit, CredentialEnroller, EnrollmentReceipt)> {
    let prepared = prepare_user_config_edit(user_dir, patch)
        .with_context(|| format!("preparing the {display} connection configuration"))?;
    println!("{}", prepared.preview());

    let enroller = CredentialEnroller::new();
    let enrollment_enroller = enroller.clone();
    let enrollment_reference = reference.clone();
    let receipt = tokio::task::spawn_blocking(move || {
        enrollment_enroller.enroll(&enrollment_reference, &secret)
    })
    .await
    .with_context(|| format!("the protected {display} credential task stopped"))?
    .with_context(|| format!("storing the protected {display} credential bundle"))?;

    match prepared.commit(true) {
        Ok(committed) => Ok((committed, enroller, receipt)),
        Err(error) => {
            let restore = tokio::task::spawn_blocking(move || enroller.restore(receipt)).await;
            if !matches!(restore, Ok(Ok(()))) {
                anyhow::bail!(
                    "publishing {display} configuration failed and protected credential rollback also failed"
                );
            }
            Err(anyhow::Error::new(error))
                .with_context(|| format!("publishing the {display} connection configuration"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DisconnectOutcome {
    Completed,
    ActiveDirectProvider,
}

pub(super) async fn connect(
    selection: Selection,
    provider: &str,
    no_color: bool,
    no_motion: bool,
) -> Result<bool> {
    if provider == CHATGPT_PROVIDER {
        return connect_chatgpt(selection, no_color, no_motion).await;
    }
    if provider == XAI_PROVIDER {
        return connect_xai(selection, no_color, no_motion).await;
    }
    let prepared = prepare(&selection)?;
    let inventory = local_inventory(&prepared.resolution, AVAILABLE_ADAPTER_KINDS)
        .map_err(|error| anyhow::anyhow!(error))
        .context("building the provider connection inventory")?;
    let configured = inventory
        .providers
        .iter()
        .any(|entry| entry.name == provider);
    let mode = if configured {
        SetupMode::Credential {
            provider: provider.to_owned(),
        }
    } else if provider == "openrouter" {
        SetupMode::OpenRouter
    } else if provider == GOOGLE_PROVIDER {
        SetupMode::Google
    } else {
        anyhow::bail!(
            "provider `{provider}` is not configured; add custom providers with `smith setup add-provider`"
        );
    };
    Ok(matches!(
        setup::run_surface(selection, mode, no_color, no_motion).await?,
        setup::SetupOutcome::Completed
    ))
}

pub(super) async fn disconnect(selection: &Selection, provider: &str) -> Result<DisconnectOutcome> {
    let prepared = prepare(selection)?;
    let user_config_path = prepared.resolution.layout.user_dir.join("config.toml");
    let user_config = fs::read_to_string(&user_config_path)
        .with_context(|| format!("reading `{}`", user_config_path.display()))?;
    let user_config = ConfigFile::parse(&user_config)
        .context("the user configuration must be valid before disconnecting a provider")?;
    let section = user_config.providers.get(provider).ok_or_else(|| {
        anyhow::anyhow!(
            "provider `{provider}` is not owned by user configuration; disconnect it in the layer that declares its credential"
        )
    })?;
    // One account or a pool: every declared reference disconnects together. A
    // partial disconnect would leave a pool whose remaining members the user
    // believed were gone.
    let references = section
        .credential
        .iter()
        .chain(section.credentials.iter())
        .map(|reference| CredentialRef::parse(reference))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| anyhow::anyhow!(error))?;
    if references.is_empty() && section.api_key.is_none() {
        anyhow::bail!("provider `{provider}` has no user-scoped credential to disconnect");
    }

    let edit = prepare_provider_credential_removal(&prepared.resolution.layout.user_dir, provider)
        .context("preparing the provider disconnect transaction")?;
    let committed = edit
        .commit(true)
        .context("publishing the provider disconnect transaction")?;
    for reference in &references {
        let cleanup = match reference {
            CredentialRef::Keychain { .. } | CredentialRef::AuthFile { .. } => {
                CredentialEnroller::new().cleanup(reference)
            }
            // Nothing of Smith's to remove: the environment, another tool's
            // session file, and an encrypted file all outlive a disconnect.
            CredentialRef::Env { .. }
            | CredentialRef::SessionJson { .. }
            | CredentialRef::File { .. } => Ok(()),
        };
        if let Err(error) = cleanup {
            let rollback = committed.rollback();
            return match rollback {
                Ok(()) => Err(anyhow::anyhow!(error)
                    .context("protected credential cleanup failed; configuration was restored")),
                Err(rollback) => Err(anyhow::anyhow!(
                    "protected credential cleanup failed and configuration rollback also failed: {rollback}"
                )),
            };
        }
    }
    committed.accept();
    println!(
        "Disconnected `{provider}` without changing its endpoint, models, profiles, or defaults."
    );

    if prepared.resolution.config.provider.name.value == provider {
        Ok(DisconnectOutcome::ActiveDirectProvider)
    } else {
        Ok(DisconnectOutcome::Completed)
    }
}

/// Builds the provider block one login connection publishes.
///
/// A replacement declares the single fixed reference; an addition extends the
/// declared pool with the next free numbered entry. Returns the reference the
/// new bundle is stored at alongside the section that uses it.
fn login_patch_section(
    mode: &ConnectMode,
    kind: &str,
    endpoint: &str,
    fixed_reference: &str,
    entry_prefix: &str,
) -> (String, ProviderSection) {
    match mode {
        ConnectMode::Replace => (
            fixed_reference.to_owned(),
            ProviderSection {
                // The login-backed kind, not the generic Responses one: what
                // is stored is a renewable bundle, and the generic adapter
                // would send it verbatim as the bearer.
                kind: Some(kind.to_owned()),
                base_url: Some(endpoint.to_owned()),
                credential: Some(fixed_reference.to_owned()),
                ..ProviderSection::default()
            },
        ),
        ConnectMode::Add { existing } => {
            let reference = format!("authfile:{}", next_pool_entry(entry_prefix, existing));
            let mut credentials = existing.clone();
            credentials.push(reference.clone());
            (
                reference,
                ProviderSection {
                    kind: Some(kind.to_owned()),
                    base_url: Some(endpoint.to_owned()),
                    credentials,
                    ..ProviderSection::default()
                },
            )
        }
    }
}

/// Signs in to xAI and writes the provider block that uses the session.
///
/// The credential and the configuration are committed together: a stored
/// session Smith is not configured to use, or a provider block pointing at a
/// credential that was never stored, are both worse than failing.
async fn connect_xai(selection: Selection, no_color: bool, no_motion: bool) -> Result<bool> {
    let before = prepare(&selection)?;
    let inventory = local_inventory(&before.resolution, AVAILABLE_ADAPTER_KINDS)
        .map_err(|error| anyhow::anyhow!(error))
        .context("building the provider connection inventory")?;
    // Whether any xAI model is already selectable, not whether one particular
    // model is: a user who already chose their own Grok should keep it rather
    // than have a second one appear beside it.
    let model_configured = inventory
        .models
        .iter()
        .any(|model| model.provider == XAI_PROVIDER);
    let user_dir = before.resolution.layout.user_dir.clone();
    let mode = match existing_login_references(&user_dir, XAI_PROVIDER, KIND_XAI_RESPONSES) {
        Some(existing) => match choose_connect_mode("xAI", &existing, no_color, no_motion).await? {
            Some(mode) => mode,
            None => return Ok(false),
        },
        None => ConnectMode::Replace,
    };
    let bundle = crate::xai::login(no_motion).await?;
    let secret = bundle
        .to_secret()
        .context("encoding the protected xAI credential bundle")?;
    // The bundle that was just earned must round-trip through protected
    // storage before anything is committed on its behalf.
    smith_runtime::xai::XaiTokenBundle::from_secret(&secret)
        .context("the completed xAI login did not produce a storable bundle")?;
    let (reference, section) = login_patch_section(
        &mode,
        KIND_XAI_RESPONSES,
        XAI_ENDPOINT,
        XAI_CREDENTIAL,
        "xai",
    );
    let reference = CredentialRef::parse(&reference).map_err(|error| anyhow::anyhow!(error))?;
    let mut patch = ConfigFile {
        providers: BTreeMap::from([(XAI_PROVIDER.to_owned(), section)]),
        ..ConfigFile::default()
    };
    if !model_configured {
        // Declared with no limits of its own. The endpoint pairs this provider
        // with its Models.dev entry, so writing limits here would freeze a copy
        // of numbers the catalog already carries and keeps current.
        patch.models.insert(
            format!("{XAI_PROVIDER}/{XAI_DEFAULT_MODEL}"),
            ModelSection::default(),
        );
    }
    let (committed, _enroller, receipt) =
        publish_login(&user_dir, "xAI", &reference, secret, &patch).await?;
    committed.accept();
    drop(receipt);
    if matches!(mode, ConnectMode::Add { .. }) {
        println!("Added another xAI account. Switch or inspect accounts with `/account`.");
    } else {
        println!(
            "Connected xAI. Select it with `smith --provider {XAI_PROVIDER} --model {model}`, or put \
             `provider = \"{XAI_PROVIDER}\"` in a profile to make it a default. Add other Grok models \
             with `smith setup add-model`.",
            model = if model_configured {
                "<model>"
            } else {
                XAI_DEFAULT_MODEL
            }
        );
    }
    Ok(true)
}

async fn connect_chatgpt(selection: Selection, no_color: bool, no_motion: bool) -> Result<bool> {
    let before = prepare(&selection)?;
    let inventory = local_inventory(&before.resolution, AVAILABLE_ADAPTER_KINDS)
        .map_err(|error| anyhow::anyhow!(error))
        .context("building the provider connection inventory")?;
    let model_configured = inventory
        .models
        .iter()
        .any(|model| model.provider == CHATGPT_PROVIDER && model.model == CHATGPT_TERRA.model);
    let user_dir = before.resolution.layout.user_dir.clone();
    let mode = match existing_login_references(&user_dir, CHATGPT_PROVIDER, KIND_CHATGPT_RESPONSES)
    {
        Some(existing) => {
            match choose_connect_mode("ChatGPT", &existing, no_color, no_motion).await? {
                Some(mode) => mode,
                None => return Ok(false),
            }
        }
        None => ConnectMode::Replace,
    };
    let Some(bundle) = chatgpt::login(no_color, no_motion).await? else {
        return Ok(false);
    };
    let secret = bundle
        .to_secret()
        .context("encoding the protected ChatGPT credential bundle")?;
    // The bundle that was just earned must round-trip through protected
    // storage before anything is committed on its behalf.
    smith_runtime::chatgpt::ChatGptTokenBundle::from_secret(&secret)
        .context("the completed ChatGPT login did not produce a storable bundle")?;
    let (reference, section) = login_patch_section(
        &mode,
        KIND_CHATGPT_RESPONSES,
        CHATGPT_ENDPOINT,
        CHATGPT_CREDENTIAL,
        "chatgpt",
    );
    let reference = CredentialRef::parse(&reference).map_err(|error| anyhow::anyhow!(error))?;
    let mut patch = ConfigFile {
        providers: BTreeMap::from([(CHATGPT_PROVIDER.to_owned(), section)]),
        ..ConfigFile::default()
    };
    if !model_configured {
        patch.models.insert(
            format!("{CHATGPT_PROVIDER}/{}", CHATGPT_TERRA.model),
            ModelSection {
                reasoning: Some(ModelReasoningSection {
                    mandatory: Some(true),
                    efforts: Some(
                        ["low", "medium", "high", "xhigh", "max", "ultra"]
                            .into_iter()
                            .map(str::to_owned)
                            .collect(),
                    ),
                    default_enabled: Some(true),
                    default_effort: Some("medium".to_owned()),
                    dialect: Some(ReasoningDialect::OpenaiEffort),
                    ..ModelReasoningSection::default()
                }),
                ..ModelSection::default()
            },
        );
    }
    let (committed, enroller, receipt) =
        publish_login(&user_dir, "ChatGPT", &reference, secret, &patch).await?;

    let mut selected = selection.clone();
    selected.profile = None;
    selected.provider = Some(CHATGPT_PROVIDER.to_owned());
    selected.model = Some(CHATGPT_TERRA.model.to_owned());
    let preflight = async {
        let prepared = prepare(&selected)?;
        smith_runtime::host::validate_host_policy(&prepared.resolution.config, &prepared.project)
            .map_err(anyhow::Error::new)
            .context("validating Smith host policy for ChatGPT")?;
        let workspace = ProjectWorkspace::new(&prepared.project)
            .map_err(|error| anyhow::anyhow!(error))
            .context("rooting the project workspace for ChatGPT preflight")?;
        let request = RuntimeRequest {
            workspace: Some(Arc::new(workspace)),
            credentials: Some(smith_config::credential::CredentialResolver::new(
                &prepared.resolution.layout.user_dir,
            )),
            ..RuntimeRequest::new(prepared.resolution.config, HostSurface::Terminal)
        };
        factory::preflight(&request)
            .await
            .map(|_| ())
            .map_err(anyhow::Error::new)
            .context("preflighting the direct ChatGPT provider")
    }
    .await;
    if let Err(error) = preflight {
        let config_rollback = committed.rollback();
        let credential_rollback =
            tokio::task::spawn_blocking(move || enroller.restore(receipt)).await;
        if config_rollback.is_err() || !matches!(credential_rollback, Ok(Ok(()))) {
            anyhow::bail!(
                "ChatGPT preflight failed and one or more local rollback operations also failed"
            );
        }
        return Err(error);
    }
    committed.accept();
    drop(receipt);
    if matches!(mode, ConnectMode::Add { .. }) {
        println!("Added another ChatGPT account. Switch or inspect accounts with `/account`.");
    } else {
        println!(
            "Connected ChatGPT directly in Smith (experimental). Select `{CHATGPT_PROVIDER}/{}` with /model.",
            CHATGPT_TERRA.model
        );
    }
    Ok(true)
}
