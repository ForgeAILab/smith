//! Guided setup orchestration and guarded terminal lifecycle.
//!
//! `smith-tui` owns pure state/rendering. This module performs the reviewed
//! user-config and credential effects, rolls both back on failed preflight,
//! and never starts a session or sends a provider request.

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use crossterm::event::{Event as TermEvent, EventStream};
use futures_util::StreamExt;
use smith_config::credential::{
    CredentialEnroller, CredentialEnrollmentError, CredentialRef, EnrollmentReceipt,
    setup_environment_reference, setup_keychain_reference,
};
use smith_config::inventory::{SelectionInventory, local_inventory};
use smith_config::model::{
    ConfigFile, ConfigSecret, ContextSection, KIND_OPENAI_COMPATIBLE, ModelSection, ProfileSection,
    ProviderResponseSection, ProviderSection, ReasoningOnlyBehavior,
};
use smith_config::resolve::{ConfigReadiness, ResolveRequest, inspect};
use smith_config::setup::{GLM_4_7, GLM_ENDPOINT, GLM_PROFILE, GLM_PROVIDER, provider_descriptors};
use smith_config::user_config::prepare_user_config_edit;
use smith_host::ProjectWorkspace;
use smith_runtime::factory::{
    self, AVAILABLE_ADAPTER_KINDS, FactoryError, HostSurface, RuntimeRequest,
};
use smith_tui::picker::ResourceEntry;
use smith_tui::setup::{
    SetupApp, SetupCredential, SetupEffect, SetupMode, SetupModelLimits, SetupSubmission,
    draw_setup,
};
use smith_tui::theme::Theme;

use crate::cli::{Selection, SetupAction, SetupArgs};
use crate::terminal;

/// Result of running a setup surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetupOutcome {
    /// Reviewed effects committed and preflight passed.
    Completed,
    /// User cancelled; nothing was committed.
    Cancelled,
}

struct SetupContext {
    selection: Selection,
    user_dir: PathBuf,
    project: PathBuf,
    inventory: SelectionInventory,
}

/// Runs an explicit reusable setup command.
pub(crate) async fn run_explicit(args: SetupArgs) -> Result<SetupOutcome> {
    require_interactive_terminal()?;
    let selection = Selection {
        project: args.project,
        ..Selection::default()
    };
    let mode = match args.action {
        SetupAction::Menu => SetupMode::Menu,
        SetupAction::AddProvider => SetupMode::AddProvider,
        SetupAction::AddModel { provider } => SetupMode::AddModel { provider },
        SetupAction::Credential { provider } => SetupMode::Credential { provider },
    };
    run_surface(selection, mode, args.no_color, args.no_motion).await
}

/// Runs automatic first-install setup, returning only after commit or cancel.
pub(crate) async fn run_first_run(
    selection: Selection,
    no_color: bool,
    no_motion: bool,
) -> Result<SetupOutcome> {
    require_interactive_terminal()?;
    run_surface(selection, SetupMode::FirstRun, no_color, no_motion).await
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

async fn run_surface(
    selection: Selection,
    mode: SetupMode,
    no_color: bool,
    no_motion: bool,
) -> Result<SetupOutcome> {
    let context = setup_context(selection, &mode)?;
    let providers = provider_entries(&context.inventory);
    let models = model_entries(&context.inventory);
    let provider_actions = provider_action_entries();
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

    let mut app = SetupApp::new(mode, providers, models)
        .with_provider_actions(provider_actions)
        .with_destination(context.user_dir.join("config.toml").display().to_string());
    let mut terminal = terminal::enter().context("entering guided setup")?;
    let mut theme = Theme::from_env();
    if no_color {
        theme = theme.without_color();
    }
    if no_motion {
        theme = theme.without_motion();
    }
    let mut events = EventStream::new();

    loop {
        terminal
            .draw(|frame| draw_setup(frame, &app, theme))
            .context("drawing guided setup")?;
        let Some(event) = events.next().await else {
            terminal.restore().context("restoring the terminal")?;
            return Ok(SetupOutcome::Cancelled);
        };
        match event.context("reading a guided-setup terminal event")? {
            TermEvent::Key(key) => match app.on_key(key) {
                SetupEffect::None => {}
                SetupEffect::Cancel => {
                    terminal.restore().context("restoring the terminal")?;
                    return Ok(SetupOutcome::Cancelled);
                }
                SetupEffect::Submit {
                    submission,
                    allow_collisions,
                } => {
                    terminal
                        .draw(|frame| draw_setup(frame, &app, theme))
                        .context("drawing setup preflight")?;
                    match apply_submission(&context, submission, allow_collisions).await {
                        ApplyOutcome::Completed => {
                            terminal.restore().context("restoring the terminal")?;
                            return Ok(SetupOutcome::Completed);
                        }
                        ApplyOutcome::Collision(preview) => app.review_collisions(preview),
                        ApplyOutcome::Failed {
                            message,
                            authentication,
                        } => app.fail(message, authentication),
                    }
                }
            },
            TermEvent::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn provider_action_entries() -> Vec<ResourceEntry> {
    provider_descriptors(AVAILABLE_ADAPTER_KINDS)
        .into_iter()
        .map(|descriptor| {
            let id = if descriptor.id == "openai-compatible" {
                "add-provider"
            } else {
                descriptor.id
            };
            ResourceEntry::new(id, descriptor.label, descriptor.description)
        })
        .collect()
}

fn setup_context(mut selection: Selection, mode: &SetupMode) -> Result<SetupContext> {
    let start = canonical_start(selection.project.as_deref())?;
    let request = ResolveRequest::new(&start)
        .with_env(std::env::vars())
        .with_cli(selection.overrides());
    let (layout, resolution) = match inspect(&request) {
        ConfigReadiness::Ready(resolution) => (resolution.layout.clone(), Some(*resolution)),
        ConfigReadiness::Unconfigured(context) => (context.layout, None),
        ConfigReadiness::Invalid(error) => {
            return Err(anyhow::anyhow!("{error}"))
                .context("configuration is invalid; guided setup will not overwrite it");
        }
    };
    if matches!(mode, SetupMode::FirstRun) && resolution.is_some() {
        anyhow::bail!("Smith is already configured; run `smith setup` to add or change a choice");
    }
    let inventory = resolution
        .as_ref()
        .map(|resolution| local_inventory(resolution, AVAILABLE_ADAPTER_KINDS))
        .transpose()
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .unwrap_or_default();
    if let SetupMode::Credential { provider } = mode {
        let provider_entry = inventory
            .providers
            .iter()
            .find(|entry| entry.name == *provider)
            .ok_or_else(|| anyhow::anyhow!("provider `{provider}` is not configured"))?;
        if !provider_entry.selectable {
            anyhow::bail!(
                "provider `{provider}` has no selectable model to preflight; finish its endpoint \
                 and model setup first"
            );
        }
        let model = inventory
            .models
            .iter()
            .find(|entry| entry.provider == *provider && entry.active)
            .or_else(|| {
                inventory
                    .models
                    .iter()
                    .find(|entry| entry.provider == *provider)
            })
            .expect("a selectable provider has at least one selectable model");
        selection.provider = Some(provider.clone());
        selection.model = Some(model.model.clone());
    }
    let project = layout.project_root.clone().unwrap_or(start);
    Ok(SetupContext {
        selection,
        user_dir: layout.user_dir,
        project,
        inventory,
    })
}

fn canonical_start(project: Option<&Path>) -> Result<PathBuf> {
    let start = match project {
        Some(project) => project.to_path_buf(),
        None => std::env::current_dir().context("reading the current directory")?,
    };
    let start = start
        .canonicalize()
        .with_context(|| format!("resolving project path `{}`", start.display()))?;
    if !start.is_dir() {
        anyhow::bail!("project path `{}` is not a directory", start.display());
    }
    Ok(start)
}

fn provider_entries(inventory: &SelectionInventory) -> Vec<ResourceEntry> {
    inventory
        .providers
        .iter()
        .map(|provider| {
            let entry = ResourceEntry::new(
                provider.name.clone(),
                provider.name.clone(),
                format!(
                    "{} · {} model(s)",
                    provider.kind.as_deref().unwrap_or("unknown adapter"),
                    provider.model_count
                ),
            )
            .active(provider.active);
            if provider.adapter_available {
                entry
            } else {
                entry.disabled("provider adapter or declaration is unavailable")
            }
        })
        .collect()
}

fn model_entries(inventory: &SelectionInventory) -> Vec<ResourceEntry> {
    inventory
        .models
        .iter()
        .map(|model| {
            let profiles = if model.profiles.is_empty() {
                "no profile".to_owned()
            } else {
                format!("profiles: {}", model.profiles.join(", "))
            };
            ResourceEntry::new(model.id(), model.id(), profiles).active(model.active)
        })
        .collect()
}

enum ApplyOutcome {
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

async fn apply_submission(
    context: &SetupContext,
    submission: SetupSubmission,
    allow_collisions: bool,
) -> ApplyOutcome {
    let enroller = CredentialEnroller::new();
    apply_submission_with(context, submission, allow_collisions, &enroller, || {
        preflight(context)
    })
    .await
}

async fn apply_submission_with<F, Fut>(
    context: &SetupContext,
    submission: SetupSubmission,
    allow_collisions: bool,
    enroller: &CredentialEnroller,
    run_preflight: F,
) -> ApplyOutcome
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), (String, bool)>>,
{
    let plan = match setup_plan(submission) {
        Ok(plan) => plan,
        Err(error) => {
            return ApplyOutcome::Failed {
                message: error.to_string(),
                authentication: false,
            };
        }
    };
    let prepared = match prepare_user_config_edit(&context.user_dir, &plan.patch) {
        Ok(prepared) => prepared,
        Err(error) => {
            return ApplyOutcome::Failed {
                message: error.to_string(),
                authentication: false,
            };
        }
    };
    if !allow_collisions && !prepared.collisions().is_empty() {
        return ApplyOutcome::Collision(prepared.preview());
    }

    let receipt = match enroll_if_needed(enroller, plan.credential_reference.as_ref(), plan.secret)
        .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            return ApplyOutcome::Failed {
                authentication: error.can_use_environment_instead(),
                message: format!(
                    "{error}. Choose the environment-variable option if protected storage is unavailable."
                ),
            };
        }
    };

    let committed = match prepared.commit(allow_collisions) {
        Ok(committed) => committed,
        Err(error) => {
            let cleanup = restore_enrollment(enroller, receipt);
            return ApplyOutcome::Failed {
                message: append_cleanup(error.to_string(), cleanup.err()),
                authentication: false,
            };
        }
    };

    match run_preflight().await {
        Ok(()) => {
            committed.accept();
            ApplyOutcome::Completed
        }
        Err((message, authentication)) => {
            let rollback = committed.rollback();
            let cleanup = restore_enrollment(enroller, receipt);
            let message = append_cleanup(append_cleanup(message, rollback.err()), cleanup.err());
            ApplyOutcome::Failed {
                message,
                authentication,
            }
        }
    }
}

async fn enroll_if_needed(
    enroller: &CredentialEnroller,
    reference: Option<&CredentialRef>,
    secret: Option<agent_runtime_core::store::Secret>,
) -> Result<Option<EnrollmentReceipt>, CredentialEnrollmentError> {
    let (Some(reference), Some(secret)) = (reference, secret) else {
        return Ok(None);
    };
    let enroller = enroller.clone();
    let reference = reference.clone();
    tokio::task::spawn_blocking(move || enroller.enroll(&reference, &secret))
        .await
        .map_err(|_| CredentialEnrollmentError::Backend {
            reference: reference_for_task_failure(),
            operation: smith_config::credential::EnrollmentOperation::Store,
            cause: smith_config::credential::KeychainError::Unavailable(
                "the credential task did not complete".into(),
            ),
        })?
        .map(Some)
}

fn reference_for_task_failure() -> CredentialRef {
    setup_keychain_reference("unknown").expect("the fixed reference is valid")
}

fn restore_enrollment(
    enroller: &CredentialEnroller,
    receipt: Option<EnrollmentReceipt>,
) -> Result<(), CredentialEnrollmentError> {
    match receipt {
        Some(receipt) => enroller.restore(receipt),
        None => Ok(()),
    }
}

fn append_cleanup(mut message: String, cleanup: Option<impl std::fmt::Display>) -> String {
    if let Some(cleanup) = cleanup {
        message.push_str(&format!("; rollback also reported: {cleanup}"));
    }
    message
}

async fn preflight(context: &SetupContext) -> Result<(), (String, bool)> {
    let start = canonical_start(context.selection.project.as_deref())
        .map_err(|error| (error.to_string(), false))?;
    let request = ResolveRequest::new(&start)
        .with_env(std::env::vars())
        .with_cli(context.selection.overrides());
    let resolution = match inspect(&request) {
        ConfigReadiness::Ready(resolution) => *resolution,
        ConfigReadiness::Unconfigured(_) => {
            return Err((
                "the reviewed edit still leaves Smith unconfigured; make the new pair the default"
                    .into(),
                false,
            ));
        }
        ConfigReadiness::Invalid(error) => return Err((error.to_string(), false)),
    };
    if let Err(error) =
        smith_runtime::host::validate_host_policy(&resolution.config, &context.project)
    {
        return Err((error.to_string(), false));
    }
    let workspace =
        ProjectWorkspace::new(&context.project).map_err(|error| (error.to_string(), false))?;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(workspace)),
        credentials: Some(smith_config::credential::CredentialResolver::new(
            &resolution.layout.user_dir,
        )),
        ..RuntimeRequest::new(resolution.config, HostSurface::Terminal)
    };
    factory::preflight(&runtime)
        .await
        .map(|_| ())
        .map_err(|error| {
            let authentication = matches!(
                error,
                FactoryError::Credential(_)
                    | FactoryError::CredentialReference { .. }
                    | FactoryError::CredentialTask
                    | FactoryError::CredentialTimeout { .. }
            );
            (error.to_string(), authentication)
        })
}

fn setup_plan(submission: SetupSubmission) -> Result<SetupPlan> {
    match submission {
        SetupSubmission::QuickGlm { credential } => {
            let PlannedCredential {
                reference,
                api_key,
                enrollment_secret,
            } = credential_plan(GLM_PROVIDER, credential)?;
            let mut patch = ConfigFile {
                providers: BTreeMap::from([(
                    GLM_PROVIDER.into(),
                    ProviderSection {
                        kind: Some(KIND_OPENAI_COMPATIBLE.into()),
                        base_url: Some(GLM_ENDPOINT.into()),
                        credential: reference.as_ref().map(ToString::to_string),
                        api_key,
                        response: Some(ProviderResponseSection {
                            reasoning_only: Some(ReasoningOnlyBehavior::Text),
                        }),
                        ..ProviderSection::default()
                    },
                )]),
                models: BTreeMap::from([(
                    format!("{GLM_PROVIDER}/{}", GLM_4_7.model),
                    ModelSection {
                        context_tokens: Some(GLM_4_7.context_tokens),
                        max_input_tokens: Some(GLM_4_7.max_input_tokens),
                        max_output_tokens: Some(GLM_4_7.max_output_tokens),
                    },
                )]),
                ..ConfigFile::default()
            };
            select_default(
                &mut patch,
                GLM_PROFILE,
                GLM_PROVIDER,
                GLM_4_7.model,
                GLM_4_7.request_output_tokens,
            );
            Ok(SetupPlan {
                patch,
                credential_reference: reference,
                secret: enrollment_secret,
            })
        }
        SetupSubmission::AddProvider {
            provider,
            endpoint,
            credential,
            model,
            limits,
            reasoning_only_text,
            make_default,
        } => {
            let PlannedCredential {
                reference,
                api_key,
                enrollment_secret,
            } = credential_plan(&provider, credential)?;
            let mut patch = ConfigFile {
                providers: BTreeMap::from([(
                    provider.clone(),
                    ProviderSection {
                        kind: Some(KIND_OPENAI_COMPATIBLE.into()),
                        base_url: Some(endpoint),
                        credential: reference.as_ref().map(ToString::to_string),
                        api_key,
                        response: reasoning_only_text.then_some(ProviderResponseSection {
                            reasoning_only: Some(ReasoningOnlyBehavior::Text),
                        }),
                        ..ProviderSection::default()
                    },
                )]),
                models: BTreeMap::from([(format!("{provider}/{model}"), model_section(limits))]),
                ..ConfigFile::default()
            };
            if make_default {
                let profile = safe_profile_name(&provider, &model);
                select_default(
                    &mut patch,
                    &profile,
                    &provider,
                    &model,
                    limits.max_output_tokens.min(8_192),
                );
            }
            Ok(SetupPlan {
                patch,
                credential_reference: reference,
                secret: enrollment_secret,
            })
        }
        SetupSubmission::AddModel {
            provider,
            model,
            limits,
            make_default,
        } => {
            let mut patch = ConfigFile {
                models: BTreeMap::from([(format!("{provider}/{model}"), model_section(limits))]),
                ..ConfigFile::default()
            };
            if make_default {
                let profile = safe_profile_name(&provider, &model);
                select_default(
                    &mut patch,
                    &profile,
                    &provider,
                    &model,
                    limits.max_output_tokens.min(8_192),
                );
            }
            Ok(SetupPlan {
                patch,
                credential_reference: None,
                secret: None,
            })
        }
        SetupSubmission::ChangeDefault { provider, model } => {
            let profile = safe_profile_name(&provider, &model);
            let mut patch = ConfigFile::default();
            select_default(&mut patch, &profile, &provider, &model, 0);
            if let Some(profile) = patch.profiles.get_mut(&profile) {
                profile.max_output_tokens = None;
                profile.context = None;
            }
            Ok(SetupPlan {
                patch,
                credential_reference: None,
                secret: None,
            })
        }
        SetupSubmission::ChangeCredential {
            provider,
            credential,
        } => {
            let PlannedCredential {
                reference,
                api_key,
                enrollment_secret,
            } = credential_plan(&provider, credential)?;
            let patch = ConfigFile {
                providers: BTreeMap::from([(
                    provider,
                    ProviderSection {
                        credential: reference.as_ref().map(ToString::to_string),
                        api_key,
                        ..ProviderSection::default()
                    },
                )]),
                ..ConfigFile::default()
            };
            Ok(SetupPlan {
                patch,
                credential_reference: reference,
                secret: enrollment_secret,
            })
        }
    }
}

fn credential_plan(provider: &str, credential: SetupCredential) -> Result<PlannedCredential> {
    match credential {
        SetupCredential::StoreInKeychain(secret) => Ok(PlannedCredential {
            reference: Some(setup_keychain_reference(provider)?),
            api_key: None,
            enrollment_secret: Some(secret),
        }),
        SetupCredential::StoreInConfig(secret) => {
            if secret.expose().is_empty() {
                anyhow::bail!("the API key cannot be empty");
            }
            Ok(PlannedCredential {
                reference: None,
                api_key: Some(ConfigSecret::new(secret.expose())),
                enrollment_secret: None,
            })
        }
        SetupCredential::ExistingKeychain => Ok(PlannedCredential {
            reference: Some(setup_keychain_reference(provider)?),
            api_key: None,
            enrollment_secret: None,
        }),
        SetupCredential::Environment(variable) => Ok(PlannedCredential {
            reference: Some(setup_environment_reference(&variable)?),
            api_key: None,
            enrollment_secret: None,
        }),
    }
}

fn model_section(limits: SetupModelLimits) -> ModelSection {
    ModelSection {
        context_tokens: Some(limits.context_tokens),
        max_input_tokens: Some(limits.max_input_tokens),
        max_output_tokens: Some(limits.max_output_tokens),
    }
}

fn select_default(
    patch: &mut ConfigFile,
    profile: &str,
    provider: &str,
    model: &str,
    request_output_tokens: u32,
) {
    patch.default_profile = Some(profile.to_owned());
    patch.profiles.insert(
        profile.to_owned(),
        ProfileSection {
            provider: Some(provider.to_owned()),
            model: Some(model.to_owned()),
            max_output_tokens: Some(request_output_tokens),
            context: Some(ContextSection {
                output_reserve: Some(request_output_tokens),
                ..ContextSection::default()
            }),
            ..ProfileSection::default()
        },
    );
}

fn safe_profile_name(provider: &str, model: &str) -> String {
    let mut name = format!("{provider}-{model}")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    name.truncate(64);
    let name = name.trim_matches('-');
    if name.is_empty() {
        "smith-model".to_owned()
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use agent_runtime_core::store::Secret;
    use smith_config::credential::{CredentialEnrollmentBackend, KeychainError};

    #[derive(Debug, Default)]
    struct FakeEnrollmentBackend {
        value: Mutex<Option<String>>,
        failure: Mutex<Option<KeychainError>>,
    }

    impl FakeEnrollmentBackend {
        fn failing(error: KeychainError) -> Self {
            Self {
                value: Mutex::new(None),
                failure: Mutex::new(Some(error)),
            }
        }

        fn fail(&self) -> Result<(), KeychainError> {
            match self.failure.lock().expect("failure").clone() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        fn exposed(&self) -> Option<String> {
            self.value.lock().expect("value").clone()
        }
    }

    impl CredentialEnrollmentBackend for FakeEnrollmentBackend {
        fn prior(&self, _service: &str, _account: &str) -> Result<Option<Secret>, KeychainError> {
            self.fail()?;
            Ok(self.exposed().map(Secret::new))
        }

        fn store(
            &self,
            _service: &str,
            _account: &str,
            secret: &Secret,
        ) -> Result<(), KeychainError> {
            self.fail()?;
            *self.value.lock().expect("value") = Some(secret.expose().to_owned());
            Ok(())
        }

        fn remove(&self, _service: &str, _account: &str) -> Result<(), KeychainError> {
            self.fail()?;
            *self.value.lock().expect("value") = None;
            Ok(())
        }
    }

    fn setup_context_for(user_dir: PathBuf, project: PathBuf) -> SetupContext {
        SetupContext {
            selection: Selection {
                project: Some(project.clone()),
                ..Selection::default()
            },
            user_dir,
            project,
            inventory: SelectionInventory::default(),
        }
    }

    #[test]
    fn glm_plan_contains_only_the_reference_and_complete_policy() {
        let secret = "sk-plan-do-not-print";
        let plan = setup_plan(SetupSubmission::QuickGlm {
            credential: SetupCredential::StoreInKeychain(agent_runtime_core::store::Secret::new(
                secret,
            )),
        })
        .expect("a plan");
        let serialized = toml::to_string(&plan.patch).expect("TOML");
        assert!(!serialized.contains(secret), "{serialized}");
        assert!(serialized.contains("keychain:smith/zai"));
        assert!(serialized.contains("reasoning_only = \"text\""));
        assert_eq!(
            plan.patch.models["zai/glm-4.7"].context_tokens,
            Some(200_000)
        );
    }

    #[test]
    fn inline_glm_plan_keeps_the_key_out_of_every_display_surface() {
        let secret = "sk-inline-plan-must-not-render";
        let plan = setup_plan(SetupSubmission::QuickGlm {
            credential: SetupCredential::StoreInConfig(Secret::new(secret)),
        })
        .expect("an inline plan");
        assert!(plan.credential_reference.is_none());
        assert!(plan.secret.is_none());
        let rendered = format!("{:?}", plan.patch);
        assert!(!rendered.contains(secret), "{rendered}");
        assert!(rendered.contains("[redacted]"), "{rendered}");

        let serialized = toml::to_string(&plan.patch).expect("user config TOML");
        assert!(serialized.contains(&format!("api_key = \"{secret}\"")));
        assert!(!serialized.contains("credential ="));
    }

    #[test]
    fn credential_migration_plan_changes_no_other_provider_or_model_field() {
        let plan = setup_plan(SetupSubmission::ChangeCredential {
            provider: "zai".into(),
            credential: SetupCredential::Environment("ZAI_API_KEY".into()),
        })
        .expect("a credential-only plan");
        assert!(plan.patch.default_profile.is_none());
        assert!(plan.patch.profiles.is_empty());
        assert!(plan.patch.models.is_empty());
        let provider = &plan.patch.providers["zai"];
        assert_eq!(provider.credential.as_deref(), Some("env:ZAI_API_KEY"));
        assert!(provider.api_key.is_none());
        assert!(provider.kind.is_none());
        assert!(provider.base_url.is_none());
        assert!(provider.response.is_none());
    }

    #[test]
    fn additive_model_plan_does_not_redeclare_or_change_a_provider() {
        let plan = setup_plan(SetupSubmission::AddModel {
            provider: "zai".into(),
            model: "glm-next".into(),
            limits: SetupModelLimits {
                context_tokens: 100,
                max_input_tokens: 90,
                max_output_tokens: 10,
            },
            make_default: false,
        })
        .expect("a plan");
        assert!(plan.patch.providers.is_empty());
        assert!(plan.patch.default_profile.is_none());
        assert!(plan.patch.models.contains_key("zai/glm-next"));
    }

    #[test]
    fn generated_profile_names_are_always_non_empty_and_bounded() {
        assert_eq!(safe_profile_name("...", "///"), "smith-model");
        assert!(safe_profile_name(&"a".repeat(80), "model").len() <= 64);
    }

    #[tokio::test]
    async fn failed_preflight_restores_config_and_credential_without_secret_artifacts() {
        let root = tempfile::tempdir().expect("root");
        let project = tempfile::tempdir().expect("project");
        let user_dir = root.path().join(".smith");
        std::fs::create_dir_all(&user_dir).expect("user dir");
        let config_path = user_dir.join("config.toml");
        let original = "# keep this comment\n[persistence]\nenabled = true\n";
        std::fs::write(&config_path, original).expect("original config");
        let context = setup_context_for(user_dir.clone(), project.path().to_owned());
        let backend = Arc::new(FakeEnrollmentBackend::default());
        let enroller = CredentialEnroller::with_backend(backend.clone());
        let secret = "sk-transaction-must-never-leak";

        let outcome = apply_submission_with(
            &context,
            SetupSubmission::QuickGlm {
                credential: SetupCredential::StoreInKeychain(Secret::new(secret)),
            },
            false,
            &enroller,
            || async { Err(("synthetic preflight failure".to_owned(), false)) },
        )
        .await;
        let message = match outcome {
            ApplyOutcome::Failed { message, .. } => message,
            _ => panic!("expected a failed transaction"),
        };
        assert!(!message.contains(secret), "{message}");
        assert_eq!(
            std::fs::read_to_string(&config_path).expect("restored config"),
            original
        );
        assert_eq!(backend.exposed(), None, "enrollment was not restored");

        for entry in std::fs::read_dir(&user_dir).expect("user artifacts") {
            let entry = entry.expect("entry");
            assert_eq!(
                entry.file_name(),
                "config.toml",
                "failed transaction left a temporary or journal artifact"
            );
            let bytes = std::fs::read(entry.path()).expect("artifact bytes");
            assert!(
                !String::from_utf8_lossy(&bytes).contains(secret),
                "secret appeared in a failed transaction artifact"
            );
        }
    }

    #[tokio::test]
    async fn denied_keychain_enrollment_writes_nothing_and_offers_environment_auth() {
        let root = tempfile::tempdir().expect("root");
        let project = tempfile::tempdir().expect("project");
        let user_dir = root.path().join(".smith");
        let context = setup_context_for(user_dir.clone(), project.path().to_owned());
        let backend = Arc::new(FakeEnrollmentBackend::failing(KeychainError::Unavailable(
            "test service absent".into(),
        )));
        let enroller = CredentialEnroller::with_backend(backend);
        let secret = "sk-unavailable-must-never-leak";

        let outcome = apply_submission_with(
            &context,
            SetupSubmission::QuickGlm {
                credential: SetupCredential::StoreInKeychain(Secret::new(secret)),
            },
            false,
            &enroller,
            || async { Ok(()) },
        )
        .await;
        let ApplyOutcome::Failed {
            message,
            authentication,
        } = outcome
        else {
            panic!("expected enrollment failure");
        };
        assert!(authentication);
        assert!(message.contains("environment-variable"), "{message}");
        assert!(!message.contains(secret), "{message}");
        assert!(!user_dir.exists(), "plaintext fallback created user state");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inline_migration_bypasses_keychain_and_failed_preflight_restores_exact_bytes() {
        let root = tempfile::tempdir().expect("root");
        let project = tempfile::tempdir().expect("project");
        let user_dir = root.path().join(".smith");
        std::fs::create_dir_all(&user_dir).expect("user dir");
        let config_path = user_dir.join("config.toml");
        let original = r#"# exact credential source
[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "keychain:smith/zai"
"#;
        std::fs::write(&config_path, original).expect("original config");
        let before = std::fs::read(&config_path).expect("prior bytes");
        let context = setup_context_for(user_dir.clone(), project.path().to_owned());
        let backend = Arc::new(FakeEnrollmentBackend::failing(KeychainError::Denied(
            "the keychain must not be consulted".into(),
        )));
        let enroller = CredentialEnroller::with_backend(backend);
        let secret = "sk-inline-rollback-must-not-render";

        let outcome = apply_submission_with(
            &context,
            SetupSubmission::ChangeCredential {
                provider: "zai".into(),
                credential: SetupCredential::StoreInConfig(Secret::new(secret)),
            },
            true,
            &enroller,
            || async { Err(("synthetic inline preflight failure".to_owned(), false)) },
        )
        .await;
        let message = match outcome {
            ApplyOutcome::Failed { message, .. } => message,
            _ => panic!("expected a failed inline transaction"),
        };
        assert!(!message.contains(secret), "{message}");
        assert_eq!(std::fs::read(&config_path).expect("restored bytes"), before);
        for entry in std::fs::read_dir(&user_dir).expect("user artifacts") {
            let entry = entry.expect("entry");
            assert_eq!(entry.file_name(), "config.toml");
            assert!(
                !String::from_utf8_lossy(&std::fs::read(entry.path()).expect("artifact"))
                    .contains(secret)
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fresh_inline_setup_commits_owner_only_without_keychain_enrollment() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("root");
        let project = tempfile::tempdir().expect("project");
        let user_dir = root.path().join(".smith");
        let context = setup_context_for(user_dir.clone(), project.path().to_owned());
        let backend = Arc::new(FakeEnrollmentBackend::failing(KeychainError::Denied(
            "the keychain must not be consulted".into(),
        )));
        let enroller = CredentialEnroller::with_backend(backend);
        let secret = "sk-fresh-inline-config";

        let outcome = apply_submission_with(
            &context,
            SetupSubmission::QuickGlm {
                credential: SetupCredential::StoreInConfig(Secret::new(secret)),
            },
            false,
            &enroller,
            || async { Ok(()) },
        )
        .await;
        assert!(matches!(outcome, ApplyOutcome::Completed));
        let path = user_dir.join("config.toml");
        let config = std::fs::read_to_string(&path).expect("inline config");
        assert!(config.contains(&format!("api_key = \"{secret}\"")));
        assert!(!config.contains("credential ="));
        assert_eq!(
            std::fs::metadata(path)
                .expect("config metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn additive_provider_and_model_transactions_preserve_existing_defaults() {
        let root = tempfile::tempdir().expect("root");
        let project = tempfile::tempdir().expect("project");
        let user_dir = root.path().join(".smith");
        let context = setup_context_for(user_dir.clone(), project.path().to_owned());
        let enroller = CredentialEnroller::with_backend(Arc::new(FakeEnrollmentBackend::default()));
        let limits = SetupModelLimits {
            context_tokens: 128_000,
            max_input_tokens: 120_000,
            max_output_tokens: 8_000,
        };

        for submission in [
            SetupSubmission::AddProvider {
                provider: "router".into(),
                endpoint: "https://router.example/v1".into(),
                credential: SetupCredential::ExistingKeychain,
                model: "primary".into(),
                limits,
                reasoning_only_text: false,
                make_default: true,
            },
            SetupSubmission::AddModel {
                provider: "router".into(),
                model: "secondary".into(),
                limits,
                make_default: false,
            },
            SetupSubmission::AddProvider {
                provider: "other".into(),
                endpoint: "https://other.example/v1".into(),
                credential: SetupCredential::ExistingKeychain,
                model: "primary".into(),
                limits,
                reasoning_only_text: false,
                make_default: false,
            },
        ] {
            assert!(matches!(
                apply_submission_with(&context, submission, false, &enroller, || async { Ok(()) },)
                    .await,
                ApplyOutcome::Completed
            ));
        }

        let config = std::fs::read_to_string(user_dir.join("config.toml")).expect("config");
        let parsed = ConfigFile::parse(&config).expect("valid merged config");
        assert_eq!(parsed.default_profile.as_deref(), Some("router-primary"));
        assert!(parsed.providers.contains_key("router"));
        assert!(parsed.providers.contains_key("other"));
        assert!(parsed.models.contains_key("router/primary"));
        assert!(parsed.models.contains_key("router/secondary"));
        assert!(parsed.models.contains_key("other/primary"));
        assert_eq!(
            parsed.profiles["router-primary"].model.as_deref(),
            Some("primary")
        );
    }
}
