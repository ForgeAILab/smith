//! Safe-boundary provider connection orchestration.

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use anyhow::{Context, Result};
use smith_config::credential::{CredentialEnroller, CredentialRef};
use smith_config::inventory::local_inventory;
use smith_config::model::{
    ConfigFile, KIND_CHATGPT_RESPONSES, ModelReasoningSection, ModelSection, ProviderSection,
    ReasoningDialect,
};
use smith_config::setup::{CHATGPT_CREDENTIAL, CHATGPT_ENDPOINT, CHATGPT_PROVIDER, CHATGPT_TERRA};
use smith_config::user_config::{prepare_provider_credential_removal, prepare_user_config_edit};
use smith_host::ProjectWorkspace;
use smith_runtime::factory::{self, HostSurface, RuntimeRequest};
use smith_tui::setup::SetupMode;

use crate::cli::Selection;
use crate::{AVAILABLE_ADAPTER_KINDS, chatgpt, prepare, setup};

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
    let credential = section
        .credential
        .as_deref()
        .map(CredentialRef::parse)
        .transpose()
        .map_err(|error| anyhow::anyhow!(error))?;
    if credential.is_none() && section.api_key.is_none() {
        anyhow::bail!("provider `{provider}` has no user-scoped credential to disconnect");
    }

    let edit = prepare_provider_credential_removal(&prepared.resolution.layout.user_dir, provider)
        .context("preparing the provider disconnect transaction")?;
    let committed = edit
        .commit(true)
        .context("publishing the provider disconnect transaction")?;
    if let Some(reference) = credential {
        let cleanup = match &reference {
            CredentialRef::Keychain { .. } | CredentialRef::AuthFile { .. } => {
                CredentialEnroller::new().cleanup(&reference)
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

async fn connect_chatgpt(selection: Selection, no_color: bool, no_motion: bool) -> Result<bool> {
    let before = prepare(&selection)?;
    let inventory = local_inventory(&before.resolution, AVAILABLE_ADAPTER_KINDS)
        .map_err(|error| anyhow::anyhow!(error))
        .context("building the provider connection inventory")?;
    let model_configured = inventory
        .models
        .iter()
        .any(|model| model.provider == CHATGPT_PROVIDER && model.model == CHATGPT_TERRA.model);
    let Some(bundle) = chatgpt::login(no_color, no_motion).await? else {
        return Ok(false);
    };
    let reference = CredentialRef::parse(CHATGPT_CREDENTIAL)
        .expect("the fixed ChatGPT credential reference is valid");
    let secret = bundle
        .to_secret()
        .context("encoding the protected ChatGPT credential bundle")?;
    let mut patch = ConfigFile {
        providers: BTreeMap::from([(
            CHATGPT_PROVIDER.to_owned(),
            ProviderSection {
                kind: Some(KIND_CHATGPT_RESPONSES.to_owned()),
                base_url: Some(CHATGPT_ENDPOINT.to_owned()),
                credential: Some(CHATGPT_CREDENTIAL.to_owned()),
                ..ProviderSection::default()
            },
        )]),
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
    let prepared_config = prepare_user_config_edit(&before.resolution.layout.user_dir, &patch)
        .context("preparing the ChatGPT connection configuration")?;
    println!("{}", prepared_config.preview());

    let enroller = CredentialEnroller::new();
    let enrollment_enroller = enroller.clone();
    let enrollment_reference = reference.clone();
    let receipt = tokio::task::spawn_blocking(move || {
        enrollment_enroller.enroll(&enrollment_reference, &secret)
    })
    .await
    .context("the protected ChatGPT credential task stopped")?
    .context("storing the protected ChatGPT credential bundle")?;

    let committed = match prepared_config.commit(true) {
        Ok(committed) => committed,
        Err(error) => {
            let restore = tokio::task::spawn_blocking(move || enroller.restore(receipt)).await;
            if !matches!(restore, Ok(Ok(()))) {
                anyhow::bail!(
                    "publishing ChatGPT configuration failed and protected credential rollback also failed"
                );
            }
            return Err(anyhow::Error::new(error))
                .context("publishing the ChatGPT connection configuration");
        }
    };

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
    println!(
        "Connected ChatGPT directly in Smith (experimental). Select `{CHATGPT_PROVIDER}/{}` with /model.",
        CHATGPT_TERRA.model
    );
    Ok(true)
}
