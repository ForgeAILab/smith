//! Guided setup orchestration and guarded terminal lifecycle.
//!
//! `smith-tui` owns pure state/rendering. This module performs the reviewed
//! user-config and credential effects, rolls both back on failed preflight,
//! and never starts a session or sends a provider request.

use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chacha20poly1305::aead::Generate;
use smith_client::{compact_tokens, plural};
use smith_config::credential::{
    CredentialEnroller, CredentialEnrollmentError, CredentialRef, EnrollmentReceipt,
    setup_environment_reference, setup_keychain_reference,
};
use smith_config::inventory::SelectionInventory;
use smith_config::model::{
    AgentPosture, ConfigFile, ConfigSecret, ContextSection, KIND_ANTHROPIC_MESSAGES,
    KIND_GEMINI_INTERACTIONS, KIND_OPENAI_COMPATIBLE, KIND_OPENAI_RESPONSES, ModelSection,
    PersistenceSection, ProfileSection, ProfileUse, ProviderResponseSection, ProviderSection,
    ReasoningOnlyBehavior,
};
use smith_config::resolve::{ConfigReadiness, ResolveRequest, inspect};
use smith_config::setup::{
    CHATGPT_PROVIDER, CHATGPT_TERRA, GLM_5_3, GLM_ENDPOINT, GLM_PROFILE, GLM_PROVIDER,
    GOOGLE_PROFILE, GOOGLE_PROVIDER, ProviderSetupDescriptor, ProviderSetupFlow, QuickKeySetup,
    SETUP_ENVIRONMENT_VARIABLE_ERROR, SETUP_PROVIDER_NAME_HELP, XAI_ENDPOINT, XAI_PROFILE,
    XAI_PROVIDER, connectable_provider_descriptors, provider_descriptors, setup_endpoint_help,
};
use smith_config::user_config::{prepare_checkpoint_key_source_removal, prepare_user_config_edit};
use smith_runtime::checkpoint::{CheckpointKeyProvider, ConfiguredCheckpointKeyProvider};
use smith_runtime::factory::{self, AVAILABLE_ADAPTER_KINDS, FactoryError, HostSurface};
use smith_runtime::model_catalog::CatalogLoader;
use smith_tui::picker::ResourceEntry;
use smith_tui::setup::{
    ResolveModelLimits, ResolvedModelLimits, SetupApp, SetupCredential, SetupEffect, SetupEntry,
    SetupFlow, SetupKeyReview, SetupMode, SetupModelLimits, SetupPrompts, SetupProviderKind,
    SetupQuickKey, SetupQuickStart, SetupSubmission,
};
use zeroize::Zeroizing;

use crate::cli::{Selection, SetupAction, SetupArgs};
use crate::screen_runner::{EffectCancellation, ScreenContext, ScreenResult, ScreenSession};

/// Result of running a setup surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetupOutcome {
    /// Reviewed effects committed and preflight passed.
    Completed,
    /// User cancelled; nothing was committed.
    Cancelled,
}

/// Messages travel to the owner, which can restore stdout or retain session notices.
pub(crate) struct SurfaceOutcome {
    pub(crate) outcome: SetupOutcome,
    pub(crate) messages: Vec<String>,
    /// What a committed submission changed, for owners with no other confirmation.
    pub(crate) summary: Option<String>,
}

impl SurfaceOutcome {
    /// Earlier recoverable errors belong to their step, not the cancellation report.
    fn cancelled() -> Self {
        Self {
            outcome: SetupOutcome::Cancelled,
            messages: Vec::new(),
            summary: None,
        }
    }
}

/// Names what a submission commits, without any credential material.
fn completion_summary(submission: &SetupSubmission, destination: &Path) -> String {
    let added = |provider: &str, model: &str, make_default: bool| {
        format!(
            "added {provider}/{model}{}",
            if make_default { " as the default" } else { "" }
        )
    };
    let change = match submission {
        SetupSubmission::QuickGlm { .. } => added(GLM_PROVIDER, GLM_5_3.model, true),
        SetupSubmission::QuickXai { model, .. } => added(XAI_PROVIDER, model, true),
        SetupSubmission::QuickGoogle { model, .. } => added(GOOGLE_PROVIDER, model, true),
        SetupSubmission::AddProvider {
            provider,
            model,
            make_default,
            ..
        }
        | SetupSubmission::AddModel {
            provider,
            model,
            make_default,
            ..
        } => added(provider, model, *make_default),
        SetupSubmission::ChangeDefault { provider, model } => {
            format!("default is now {provider}/{model}")
        }
        SetupSubmission::ChangeCredential { provider, .. } => {
            format!("updated the credential for {provider}")
        }
    };
    format!(
        "Setup complete · {change} · saved to {}",
        destination.display()
    )
}

struct SetupContext {
    selection: Selection,
    user_dir: PathBuf,
    project: PathBuf,
    inventory: SelectionInventory,
    catalog: Arc<smith_config::catalog::CatalogSnapshot>,
    unconfigured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExistingCheckpointSource {
    Platform,
    Inline,
    GeneratedInline,
    Credential(String),
}

struct CheckpointSetupContext {
    user_dir: PathBuf,
    sessions_dir: PathBuf,
    source: ExistingCheckpointSource,
}

/// Runs an explicit reusable setup command.
pub(crate) async fn run_explicit(args: SetupArgs) -> Result<SetupOutcome> {
    require_interactive_terminal()?;
    if args.action == SetupAction::CheckpointKey {
        return run_checkpoint_key_setup(args.project).await;
    }
    let selection = Selection {
        project: args.project,
        ..Selection::default()
    };
    let mode = match args.action {
        SetupAction::Menu => SetupMode::Menu,
        SetupAction::AddProvider => SetupMode::AddProvider,
        SetupAction::AddModel { provider } => SetupMode::AddModel { provider },
        SetupAction::Credential { provider } => SetupMode::Credential { provider },
        SetupAction::CheckpointKey => unreachable!("handled before the provider setup surface"),
    };
    run_standalone_surface(selection, mode, args.no_color, args.no_motion).await
}

/// Runs automatic first-install setup, returning only after commit or cancel.
pub(crate) async fn run_first_run(
    selection: Selection,
    no_color: bool,
    no_motion: bool,
) -> Result<(SetupOutcome, Option<String>)> {
    require_interactive_terminal()?;
    let result =
        run_standalone_messages(selection, SetupMode::FirstRun, no_color, no_motion).await?;
    Ok((result.outcome, result.summary))
}

fn require_interactive_terminal() -> Result<()> {
    if std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
    {
        Ok(())
    } else {
        anyhow::bail!(
            "guided setup needs an interactive terminal on stdin, stdout, and stderr; \
             run `smith setup` directly in a terminal"
        )
    }
}

async fn run_standalone_surface(
    selection: Selection,
    mode: SetupMode,
    no_color: bool,
    no_motion: bool,
) -> Result<SetupOutcome> {
    let result = run_standalone_messages(selection, mode, no_color, no_motion).await?;
    if let Some(summary) = result.summary {
        println!("{summary}");
    }
    Ok(result.outcome)
}

/// Leaves the summary to the caller: a first run opens the session over stdout.
async fn run_standalone_messages(
    selection: Selection,
    mode: SetupMode,
    no_color: bool,
    no_motion: bool,
) -> Result<SurfaceOutcome> {
    let mut session = ScreenSession::enter(no_color, no_motion).context("entering guided setup")?;
    let result = run_surface(selection, mode, &mut session, no_motion, None).await;
    let mut result = session.finish(result, "restoring the terminal")?;
    for message in result.messages.drain(..) {
        println!("{message}");
    }
    Ok(result)
}

pub(crate) async fn run_surface(
    selection: Selection,
    mode: SetupMode,
    session: &mut ScreenSession<'_>,
    no_motion: bool,
    title: Option<&str>,
) -> Result<SurfaceOutcome> {
    let context = setup_context(selection, &mode).await?;
    let providers = provider_entries(&context.inventory);
    let catalog_connection = match &mode {
        SetupMode::Provider {
            flow: SetupFlow::QuickKey { provider, .. },
        } => connectable_provider_descriptors(AVAILABLE_ADAPTER_KINDS)
            .into_iter()
            .find(|descriptor| descriptor.provider == Some(provider.as_str())),
        SetupMode::Provider {
            flow:
                SetupFlow::CustomEndpoint {
                    provider: Some(provider),
                    ..
                },
        } => connectable_provider_descriptors(AVAILABLE_ADAPTER_KINDS)
            .into_iter()
            .find(|descriptor| descriptor.provider == Some(provider.as_str())),
        _ => None,
    }
    .and_then(|descriptor| descriptor.connection)
    .and_then(|connection| {
        connection
            .catalog_provider
            .map(|catalog| (catalog, connection.label))
    });
    let (models, catalog_model_limits) = match catalog_connection {
        Some((catalog, label)) => catalog_model_entries(&context, catalog, label)?,
        None => (model_entries(&context.inventory), BTreeMap::new()),
    };
    let provider_actions = setup_action_entries(&mode);
    if matches!(mode, SetupMode::AddProvider)
        && !provider_actions
            .iter()
            .any(|entry| entry.id == "add-provider")
    {
        anyhow::bail!("this Smith build has no setup descriptor for an available provider adapter");
    }
    if let SetupMode::AddModel {
        provider: Some(provider),
    } = &mode
        && !context
            .inventory
            .providers
            .iter()
            .any(|entry| entry.name == *provider && entry.adapter_available)
    {
        anyhow::bail!(
            "provider `{provider}` is not locally selectable; run `smith setup add-provider` \
             or omit `--provider` to choose from the configured list"
        );
    }

    let mut app = SetupApp::new(
        mode,
        providers,
        models,
        glm_quick_start(),
        provider_actions,
        setup_prompts(),
    )
    .with_catalog_model_limits(catalog_model_limits)
    .with_destination(context.user_dir.join("config.toml").display().to_string());
    if let Some(title) = title {
        app = app.with_title(title);
    }
    let mut messages = Vec::new();
    loop {
        let effect = match session
            .run(
                &mut app,
                ScreenContext {
                    draw: Some("drawing guided setup"),
                    input: "reading a guided-setup terminal event",
                },
            )
            .await?
        {
            ScreenResult::Outcome(()) | ScreenResult::InputEnded => {
                return Ok(SurfaceOutcome::cancelled());
            }
            ScreenResult::Effect(effect) => effect,
            ScreenResult::Completed(never) => match never {},
        };
        match effect {
            SetupEffect::None => {}
            SetupEffect::Cancel => return Ok(SurfaceOutcome::cancelled()),
            SetupEffect::ConnectChatGpt => {
                let outcome = crate::connection::connect_chatgpt_from_setup(
                    context.selection.clone(),
                    context.user_dir.clone(),
                    context.inventory.models.iter().any(|model| {
                        model.provider == CHATGPT_PROVIDER && model.model == CHATGPT_TERRA.model
                    }),
                    context.unconfigured,
                    session,
                    no_motion,
                    true,
                )
                .await
                .context("ChatGPT setup could not complete")?;
                match outcome {
                    smith_tui::FlowOutcome::Completed(login_messages) => {
                        messages.extend(login_messages);
                        return Ok(SurfaceOutcome {
                            outcome: SetupOutcome::Completed,
                            messages,
                            summary: None,
                        });
                    }
                    smith_tui::FlowOutcome::Cancelled => {
                        return Ok(SurfaceOutcome::cancelled());
                    }
                    smith_tui::FlowOutcome::Back => app.back_from_chatgpt(),
                }
            }
            SetupEffect::ResolveModelLimits { request } => {
                // One bounded, non-inference read; the surface stays busy
                // until it completes or a recorded cancellation can return safely.
                session
                    .draw(&app)
                    .context("drawing setup limit resolution")?;
                let cancellation = EffectCancellation::default();
                let resolved = session
                    .wait_effect(
                        &app,
                        resolve_model_limits(&context, &request),
                        &cancellation,
                        ScreenContext {
                            draw: Some("drawing setup limit resolution"),
                            input: "reading setup limit resolution input",
                        },
                    )
                    .await?;
                if cancellation.requested() {
                    return Ok(SurfaceOutcome::cancelled());
                }
                app.apply_resolved_limits(resolved);
            }
            SetupEffect::Submit {
                submission,
                allow_collisions,
            } => {
                session.draw(&app).context("drawing setup preflight")?;
                let summary =
                    completion_summary(&submission, &context.user_dir.join("config.toml"));
                let cancellation = EffectCancellation::default();
                match session
                    .wait_effect(
                        &app,
                        apply_submission(&context, submission, allow_collisions, &cancellation),
                        &cancellation,
                        ScreenContext {
                            draw: Some("drawing setup preflight"),
                            input: "reading setup preflight input",
                        },
                    )
                    .await?
                {
                    ApplyOutcome::Cancelled => return Ok(SurfaceOutcome::cancelled()),
                    ApplyOutcome::Completed => {
                        return Ok(SurfaceOutcome {
                            outcome: SetupOutcome::Completed,
                            messages,
                            summary: Some(summary),
                        });
                    }
                    ApplyOutcome::Collision(preview) => app.review_collisions(preview),
                    ApplyOutcome::Failed {
                        message,
                        authentication,
                    } => {
                        messages.push(message.clone());
                        if cancellation.requested() {
                            return Ok(SurfaceOutcome::cancelled());
                        }
                        app.fail(message, authentication);
                    }
                }
            }
        }
    }
}

enum ApplyOutcome {
    Cancelled,
    Completed,
    Collision(String),
    Failed {
        message: String,
        authentication: bool,
    },
}

struct SetupPlan {
    patch: ConfigFile,
    credential_reference: Option<CredentialRef>,
    secret: Option<agent_runtime_core::store::Secret>,
}

struct PlannedCredential {
    reference: Option<CredentialRef>,
    api_key: Option<ConfigSecret>,
    enrollment_secret: Option<agent_runtime_core::store::Secret>,
}

mod checkpoint;
mod choices;
mod context;
mod model_limits;
mod plan;
mod transaction;

use checkpoint::run_checkpoint_key_setup;
#[cfg(test)]
use checkpoint::{protected_checkpoint_exists, refuse_unsafe_checkpoint_rotation};
use choices::{catalog_model_entries, model_entries, provider_entries};
use context::{canonical_start, setup_context};
#[cfg(test)]
use model_limits::configured_probe_target;
use model_limits::resolve_model_limits;
#[cfg(test)]
use plan::safe_profile_name;
use plan::setup_plan;
use transaction::apply_submission;
#[cfg(test)]
use transaction::apply_submission_with;

pub(super) use choices::{
    glm_quick_start, provider_setup_flow, setup_action_entries, setup_prompts,
};
pub(super) use plan::select_default;

#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod tests;
