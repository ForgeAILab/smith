//! Safe-boundary provider connection orchestration.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use agent_runtime_core::store::Secret;
use anyhow::{Context, Result};
use smith_config::credential::{CredentialEnroller, CredentialRef, EnrollmentReceipt};
use smith_config::inventory::local_inventory;
use smith_config::model::{
    ConfigFile, KIND_CHATGPT_RESPONSES, KIND_XAI_RESPONSES, ModelReasoningSection, ModelSection,
    ProviderSection, ReasoningDialect,
};
use smith_config::setup::{
    CHATGPT_CREDENTIAL, CHATGPT_ENDPOINT, CHATGPT_PROVIDER, CHATGPT_TERRA, ProviderConnectFlow,
    XAI_CREDENTIAL, XAI_DEFAULT_MODEL, XAI_ENDPOINT, XAI_PROVIDER,
    connectable_provider_descriptors,
};
use smith_config::user_config::{
    CommittedConfigEdit, prepare_provider_credential_removal, prepare_user_config_edit,
};
use smith_runtime::factory::{self, AVAILABLE_ADAPTER_KINDS, HostSurface};
use smith_tui::setup::SetupMode;
use smith_tui::{FlowOutcome, ResourceEntry, ResourcePicker};

use crate::cli::Selection;
use crate::config_command::prepare;
use crate::screen_runner::{EffectCancellation, ScreenContext, ScreenResult, ScreenSession};
use crate::{chatgpt, setup};

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
    session: &mut ScreenSession<'_>,
) -> Result<FlowOutcome<ConnectMode>> {
    let picked = crate::resources::pick_one(
        &format!("Connect {display} · already connected"),
        connect_mode_entries(existing.len()),
        "No connection choices",
        session,
    )
    .await?;
    Ok(picked
        .and_then(|choice| connect_mode_from_choice(&choice, existing))
        .map(FlowOutcome::Completed)
        .unwrap_or(FlowOutcome::Cancelled))
}

fn connect_mode_from_choice(choice: &str, existing: &[String]) -> Option<ConnectMode> {
    match choice {
        "add" => Some(ConnectMode::Add {
            existing: existing.to_vec(),
        }),
        "replace" => Some(ConnectMode::Replace),
        _ => None,
    }
}

/// Picker entries shared by the account-choice loop and its terminal fixtures.
pub(super) fn connect_mode_entries(accounts: usize) -> Vec<ResourceEntry> {
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
    ]
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

struct PublishedLogin {
    committed: CommittedConfigEdit,
    enroller: CredentialEnroller,
    receipt: EnrollmentReceipt,
    preview: String,
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
    session: &mut ScreenSession<'_>,
) -> Result<FlowOutcome<PublishedLogin>> {
    let prepared = prepare_user_config_edit(user_dir, patch)
        .with_context(|| format!("preparing the {display} connection configuration"))?;
    let preview = prepared.preview();
    let title = if display == "ChatGPT" {
        "Connect ChatGPT · experimental".to_owned()
    } else {
        format!("Connect {display}")
    };
    let mut review = crate::connection_review::ConnectionReview::new(&title, preview.clone());
    match session
        .run(
            &mut review,
            ScreenContext {
                draw: Some("drawing connection review"),
                input: "reading connection review input",
            },
        )
        .await?
    {
        ScreenResult::Outcome(FlowOutcome::Completed(())) => {}
        ScreenResult::Outcome(FlowOutcome::Back) => return Ok(FlowOutcome::Back),
        ScreenResult::Outcome(FlowOutcome::Cancelled) | ScreenResult::InputEnded => {
            return Ok(FlowOutcome::Cancelled);
        }
        ScreenResult::Effect(never) | ScreenResult::Completed(never) => match never {},
    }
    // The preview is also returned for standalone stdout after terminal restoration.
    // Inside a session it has already been reviewed and needs no transcript dump.

    review.saving();
    let cancellation = EffectCancellation::default();
    let publication = async {
        if cancellation.requested() {
            return Ok(FlowOutcome::Cancelled);
        }
        let enroller = CredentialEnroller::new();
        let enrollment_enroller = enroller.clone();
        let enrollment_reference = reference.clone();
        let receipt = tokio::task::spawn_blocking(move || {
            enrollment_enroller.enroll(&enrollment_reference, &secret)
        })
        .await
        .with_context(|| format!("the protected {display} credential task stopped"))?
        .with_context(|| format!("storing the protected {display} credential bundle"))?;

        if cancellation.requested() {
            let restore = tokio::task::spawn_blocking(move || enroller.restore(receipt)).await;
            if !matches!(restore, Ok(Ok(()))) {
                anyhow::bail!(
                    "cancelling {display} connection failed to restore its protected credential"
                );
            }
            return Ok(FlowOutcome::Cancelled);
        }

        match prepared.commit(true) {
            Ok(committed) => Ok(FlowOutcome::Completed(PublishedLogin {
                committed,
                enroller,
                receipt,
                preview,
            })),
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
    };
    session
        .wait_effect(
            &review,
            publication,
            &cancellation,
            ScreenContext {
                draw: Some("drawing connection publication"),
                input: "reading connection publication input",
            },
        )
        .await?
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DisconnectOutcome {
    Completed,
    ActiveDirectProvider,
}

/// The active-provider exception lets the terminal owner decide when to exit.
pub(super) struct DisconnectResult {
    pub(super) outcome: DisconnectOutcome,
    pub(super) messages: Vec<String>,
}

/// Local outcomes survive a same-session host rebuild without entering model history.
pub(super) fn push_notices(app: &mut smith_tui::App, result: Result<Vec<String>>) {
    let messages = match result {
        Ok(messages) => messages,
        Err(error) => vec![format!("Connection failed: {error:#}")],
    };
    if !messages.is_empty() {
        app.following = true;
        app.scroll_back = 0;
    }
    for message in messages {
        app.transcript
            .push_notice(smith_client::NoticeKind::Provider, message);
    }
}

pub(super) async fn connect(
    selection: Selection,
    provider: &str,
    session: &mut ScreenSession<'_>,
    no_motion: bool,
) -> Result<setup::SurfaceOutcome> {
    let descriptor = connectable_provider_descriptors(AVAILABLE_ADAPTER_KINDS)
        .into_iter()
        .find(|descriptor| descriptor.id == provider);
    if let Some(connection) = descriptor.and_then(|descriptor| descriptor.connection) {
        match connection.flow {
            ProviderConnectFlow::ChatGptOAuth => {
                return connect_chatgpt(selection, session, no_motion)
                    .await
                    .map(connection_outcome);
            }
            ProviderConnectFlow::XaiLogin => {
                return connect_xai(selection, session, no_motion)
                    .await
                    .map(connection_outcome);
            }
            ProviderConnectFlow::Setup => {}
        }
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
    } else if let Some(descriptor) = descriptor {
        // The generic descriptor collects its provider name; fixed built-ins
        // start at authentication and may select from the frozen catalog.
        SetupMode::Provider {
            flow: setup::provider_setup_flow(
                descriptor,
                descriptor
                    .connection
                    .is_some_and(|connection| connection.catalog_provider.is_some()),
            ),
        }
    } else {
        anyhow::bail!(
            "provider `{provider}` is not configured; connect a custom OpenAI-compatible endpoint \
             with `/connect openai-compatible`, or run `smith setup add-provider`"
        );
    };
    let label = descriptor
        .and_then(|descriptor| descriptor.connection)
        .map(|connection| connection.label)
        .unwrap_or(provider);
    let mut outcome = setup::run_surface(
        selection,
        mode,
        session,
        no_motion,
        Some(&format!("Connect {label}")),
    )
    .await?;
    if outcome.outcome == setup::SetupOutcome::Completed {
        outcome.messages.push(format!("Connected {label}"));
    }
    Ok(outcome)
}

fn connection_outcome(outcome: FlowOutcome<Vec<String>>) -> setup::SurfaceOutcome {
    match outcome {
        FlowOutcome::Completed(messages) => setup::SurfaceOutcome {
            outcome: setup::SetupOutcome::Completed,
            messages,
        },
        FlowOutcome::Back | FlowOutcome::Cancelled => setup::SurfaceOutcome {
            outcome: setup::SetupOutcome::Cancelled,
            messages: Vec::new(),
        },
    }
}

pub(super) async fn disconnect(selection: &Selection, provider: &str) -> Result<DisconnectResult> {
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
    Ok(DisconnectResult {
        outcome: if prepared.resolution.config.provider.name.value == provider {
            DisconnectOutcome::ActiveDirectProvider
        } else {
            DisconnectOutcome::Completed
        },
        messages: vec![format!(
            "Disconnected `{provider}` without changing its endpoint, models, profiles, or defaults."
        )],
    })
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
async fn connect_xai(
    selection: Selection,
    session: &mut ScreenSession<'_>,
    no_motion: bool,
) -> Result<FlowOutcome<Vec<String>>> {
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
    let existing = existing_login_references(&user_dir, XAI_PROVIDER, KIND_XAI_RESPONSES);
    let mut previous_mode = None;
    loop {
        let mode = match previous_mode.take() {
            Some(mode) => mode,
            None => match &existing {
                Some(existing) => match choose_connect_mode("xAI", existing, session).await? {
                    FlowOutcome::Completed(mode) => mode,
                    FlowOutcome::Back | FlowOutcome::Cancelled => {
                        return Ok(FlowOutcome::Cancelled);
                    }
                },
                None => ConnectMode::Replace,
            },
        };
        let bundle = match crate::xai::login(session, no_motion).await? {
            FlowOutcome::Completed(bundle) => bundle,
            FlowOutcome::Back | FlowOutcome::Cancelled => return Ok(FlowOutcome::Cancelled),
        };
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
        let PublishedLogin {
            committed,
            enroller: _enroller,
            receipt,
            preview,
        } = match publish_login(&user_dir, "xAI", &reference, secret, &patch, session).await? {
            FlowOutcome::Completed(published) => published,
            FlowOutcome::Back => {
                previous_mode = Some(mode);
                continue;
            }
            FlowOutcome::Cancelled => return Ok(FlowOutcome::Cancelled),
        };
        committed.accept();
        drop(receipt);
        let result = if matches!(mode, ConnectMode::Add { .. }) {
            "Added another xAI account. Switch or inspect accounts with `/account`.".to_owned()
        } else {
            format!(
                "Connected xAI. Select it with `smith --provider {XAI_PROVIDER} --model {model}`, or put \
             `provider = \"{XAI_PROVIDER}\"` in a profile to make it a default. Add other Grok models \
             with `smith setup add-model`.",
                model = if model_configured {
                    "<model>"
                } else {
                    XAI_DEFAULT_MODEL
                }
            )
        };
        return Ok(FlowOutcome::Completed(session.login_messages(
            preview,
            vec!["Signed in to xAI.".to_owned(), result],
        )));
    }
}

async fn connect_chatgpt(
    selection: Selection,
    session: &mut ScreenSession<'_>,
    no_motion: bool,
) -> Result<FlowOutcome<Vec<String>>> {
    let before = prepare(&selection)?;
    let inventory = local_inventory(&before.resolution, AVAILABLE_ADAPTER_KINDS)
        .map_err(|error| anyhow::anyhow!(error))
        .context("building the provider connection inventory")?;
    let model_configured = inventory
        .models
        .iter()
        .any(|model| model.provider == CHATGPT_PROVIDER && model.model == CHATGPT_TERRA.model);
    connect_chatgpt_from_setup(
        selection,
        before.resolution.layout.user_dir.clone(),
        model_configured,
        false,
        session,
        no_motion,
        false,
    )
    .await
}

/// Runs the existing ChatGPT connection ceremony from a reviewed setup context.
/// An empty install also receives a default profile in the same transaction;
/// the shared runner keeps the setup-to-login handoff under one terminal guard.
pub(super) async fn connect_chatgpt_from_setup(
    selection: Selection,
    user_dir: std::path::PathBuf,
    model_configured: bool,
    make_default: bool,
    session: &mut ScreenSession<'_>,
    no_motion: bool,
    from_setup: bool,
) -> Result<FlowOutcome<Vec<String>>> {
    let existing = existing_login_references(&user_dir, CHATGPT_PROVIDER, KIND_CHATGPT_RESPONSES);
    let mut account_picker = existing.as_ref().map(|existing| {
        ResourcePicker::choices(
            "Connect ChatGPT · already connected",
            connect_mode_entries(existing.len()),
            "No connection choices",
        )
        .with_back(from_setup)
    });
    let mut methods =
        chatgpt::login_method_picker().with_back(from_setup || account_picker.is_some());
    let mut previous_mode = None;
    loop {
        let (mode, bundle) = loop {
            let mode = if let Some(mode) = previous_mode.take() {
                mode
            } else if let (Some(existing), Some(picker)) = (&existing, &mut account_picker) {
                match session
                    .run(
                        picker,
                        ScreenContext {
                            draw: None,
                            input: "reading a terminal event",
                        },
                    )
                    .await?
                {
                    ScreenResult::Outcome(FlowOutcome::Completed(choice)) => {
                        let Some(mode) = connect_mode_from_choice(&choice, existing) else {
                            return Ok(FlowOutcome::Cancelled);
                        };
                        mode
                    }
                    ScreenResult::Outcome(FlowOutcome::Back) => {
                        return Ok(FlowOutcome::Back);
                    }
                    ScreenResult::Outcome(FlowOutcome::Cancelled) | ScreenResult::InputEnded => {
                        return Ok(FlowOutcome::Cancelled);
                    }
                    ScreenResult::Effect(never) | ScreenResult::Completed(never) => match never {},
                }
            } else {
                ConnectMode::Replace
            };
            match chatgpt::login(session, &mut methods, no_motion, from_setup).await? {
                FlowOutcome::Completed(bundle) => break (mode, bundle),
                FlowOutcome::Back if account_picker.is_some() => continue,
                FlowOutcome::Back => return Ok(FlowOutcome::Back),
                FlowOutcome::Cancelled => return Ok(FlowOutcome::Cancelled),
            }
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
        if make_default {
            setup::select_default(
                &mut patch,
                CHATGPT_PROVIDER,
                CHATGPT_PROVIDER,
                CHATGPT_TERRA.model,
                0,
            );
            if let Some(profile) = patch.profiles.get_mut(CHATGPT_PROVIDER) {
                // Trusted ChatGPT metadata supplies both budgets, as it does for
                // /connect; the first-run profile must not override them with zero.
                profile.max_output_tokens = None;
                profile.context = None;
            }
        }
        let PublishedLogin {
            committed,
            enroller,
            receipt,
            preview,
        } = match publish_login(&user_dir, "ChatGPT", &reference, secret, &patch, session).await? {
            FlowOutcome::Completed(published) => published,
            FlowOutcome::Back => {
                previous_mode = Some(mode);
                continue;
            }
            FlowOutcome::Cancelled => return Ok(FlowOutcome::Cancelled),
        };

        let mut selected = selection.clone();
        selected.profile = None;
        selected.provider = Some(CHATGPT_PROVIDER.to_owned());
        selected.model = Some(CHATGPT_TERRA.model.to_owned());
        let cancellation = EffectCancellation::default();
        let preflight = async {
            let prepared = prepare(&selected)?;
            smith_runtime::host::validate_host_policy(
                &prepared.resolution.config,
                &prepared.project,
            )
            .map_err(anyhow::Error::new)
            .context("validating Smith host policy for ChatGPT")?;
            let request = crate::runtime_host::preflight_request(
                &prepared.resolution,
                &prepared.project,
                HostSurface::Terminal,
                None,
            )
            .context("rooting the project workspace for ChatGPT preflight")?;
            factory::preflight(&request)
                .await
                .map(|_| ())
                .map_err(anyhow::Error::new)
                .context("preflighting the direct ChatGPT provider")
        };
        let checking = crate::login_progress::LoginProgress::new(
            "Connect ChatGPT · experimental",
            vec!["Checking the reviewed connection…".to_owned()],
            "Checking ChatGPT",
            no_motion,
        );
        let preflight = session
            .wait_effect(
                &checking,
                preflight,
                &cancellation,
                ScreenContext {
                    draw: Some("drawing ChatGPT connection preflight"),
                    input: "reading ChatGPT connection preflight input",
                },
            )
            .await
            .and_then(|result| result);
        let preflight = if cancellation.requested() {
            Err(anyhow::anyhow!("ChatGPT connection cancelled"))
        } else {
            preflight
        };
        if let Err(error) = preflight {
            let config_rollback = committed.rollback();
            let credential_rollback =
                tokio::task::spawn_blocking(move || enroller.restore(receipt)).await;
            if config_rollback.is_err() || !matches!(credential_rollback, Ok(Ok(()))) {
                anyhow::bail!(
                    "ChatGPT preflight failed and one or more local rollback operations also failed"
                );
            }
            if cancellation.requested() {
                return Ok(FlowOutcome::Cancelled);
            }
            return Err(error);
        }
        committed.accept();
        drop(receipt);
        let result = if matches!(mode, ConnectMode::Add { .. }) {
            "Added another ChatGPT account. Switch or inspect accounts with `/account`.".to_owned()
        } else {
            format!(
                "Connected ChatGPT directly in Smith (experimental). Select `{CHATGPT_PROVIDER}/{}` with /model.",
                CHATGPT_TERRA.model
            )
        };
        return Ok(FlowOutcome::Completed(
            session.login_messages(preview, vec![result]),
        ));
    }
}

#[cfg(test)]
mod tests {
    use smith_client::NoticeKind;
    use smith_tui::App;
    use smith_tui::transcript::Block;

    #[test]
    fn connection_and_disconnection_outcomes_are_retained_notices() {
        let mut app = App::new("example-model", "~/work/api");
        app.transcript.push_user("retained transcript");
        app.composer.insert_str("retained draft");
        app.following = false;
        app.scroll_back = 10;
        super::push_notices(&mut app, Ok(vec!["Connected OpenRouter".into()]));
        let disconnected = super::DisconnectResult {
            outcome: super::DisconnectOutcome::Completed,
            messages: vec!["Disconnected zai".into()],
        };
        super::push_notices(&mut app, Ok(disconnected.messages));
        super::push_notices(&mut app, Err(anyhow::anyhow!("device login timed out")));
        let notices = app
            .transcript
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::Notice {
                    kind: NoticeKind::Provider,
                    text,
                } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            notices,
            [
                "Connected OpenRouter",
                "Disconnected zai",
                "Connection failed: device login timed out"
            ]
        );
        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
        assert_eq!(app.composer.text(), "retained draft");
        app.rebind_host();
        assert_eq!(app.transcript.blocks().len(), 4);
        assert_eq!(app.composer.text(), "retained draft");
    }
}
