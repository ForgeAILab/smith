use super::choices::provider_action_entries;
use super::model_limits::configured_provider_section;
use super::{
    AVAILABLE_ADAPTER_KINDS, AgentPosture, ApplyOutcome, Arc, CheckpointSetupContext, ConfigFile,
    ConfigReadiness, CredentialEnroller, ExistingCheckpointSource, GLM_PROFILE, GOOGLE_PROFILE,
    GOOGLE_PROVIDER, KIND_ANTHROPIC_MESSAGES, KIND_GEMINI_INTERACTIONS, KIND_OPENAI_RESPONSES,
    PathBuf, ProfileUse, ResolveModelLimits, ResolveRequest, Result, Selection, SelectionInventory,
    SetupApp, SetupContext, SetupCredential, SetupEffect, SetupFlow, SetupMode, SetupModelLimits,
    SetupProviderKind, SetupSubmission, XAI_ENDPOINT, XAI_PROFILE, XAI_PROVIDER,
    apply_submission_with, catalog_model_entries, configured_probe_target, glm_quick_start,
    inspect, protected_checkpoint_exists, provider_descriptors, provider_setup_flow,
    refuse_unsafe_checkpoint_rotation, resolve_model_limits, safe_profile_name,
    setup_action_entries, setup_plan, setup_prompts,
};
use std::sync::Mutex;

use agent_runtime_core::store::Secret;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use smith_config::credential::{CredentialEnrollmentBackend, KeychainError};

fn setup_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn catalog_model_choices_keep_ids_first_and_use_compact_limits() {
    let context = setup_context_for(PathBuf::from("<HOME>"), PathBuf::from("<PROJECT>"));
    let (entries, limits) =
        catalog_model_entries(&context, "openrouter", "OpenRouter").expect("catalog models");
    let entry = entries
        .iter()
        .find(|entry| entry.id == "aion-labs/aion-2.0")
        .expect("Aion model");
    assert_eq!(
        entry.description,
        "aion-labs/aion-2.0 · 131k context · 131k input · 32.7k output · reasoning"
    );
    let model_limits = limits.get(&entry.id).expect("model limits");
    assert_eq!(model_limits.context_tokens, 131_072);
    assert_eq!(model_limits.max_input_tokens, 131_072);
    assert_eq!(model_limits.max_output_tokens, 32_768);
}

#[test]
fn every_offered_setup_action_has_a_handler() {
    let actions = provider_action_entries();
    assert!(!actions.is_empty());
    assert_eq!(
        actions.len(),
        provider_descriptors(AVAILABLE_ADAPTER_KINDS).len()
    );
    for (descriptor, entry) in provider_descriptors(AVAILABLE_ADAPTER_KINDS)
        .iter()
        .zip(&actions)
    {
        assert_eq!(entry.id, descriptor.setup_id);
        assert_eq!(entry.label, descriptor.label);
        assert_eq!(entry.detail, descriptor.description);
        assert_eq!(entry.flow, provider_setup_flow(*descriptor, false));
    }
    for mode in [SetupMode::FirstRun, SetupMode::Menu] {
        for (index, entry) in setup_action_entries(&mode).iter().enumerate() {
            let mut app = SetupApp::new(
                mode.clone(),
                Vec::new(),
                Vec::new(),
                glm_quick_start(),
                setup_action_entries(&mode),
                setup_prompts(),
            );
            for _ in 0..index {
                app.on_key(setup_key(KeyCode::Down));
            }
            let effect = app.on_key(setup_key(KeyCode::Enter));
            assert!(
                !app.is_choosing_action(),
                "offered setup action `{}` has no handler in {mode:?}",
                entry.id
            );
            if matches!(entry.flow, SetupFlow::OAuth { .. }) {
                assert!(matches!(effect, SetupEffect::ConnectChatGpt));
            } else {
                assert!(matches!(effect, SetupEffect::None));
                assert!(!app.is_busy(), "{} did not reach an input step", entry.id);
            }
        }
    }
}

#[test]
fn quick_start_display_matches_the_written_model_and_trusted_revision() {
    let data = glm_quick_start();
    let entry = provider_action_entries()
        .into_iter()
        .find(|entry| entry.id == "glm")
        .expect("GLM action");
    assert!(entry.detail.contains(&data.model_label));
    let mut app = SetupApp::new(
        SetupMode::FirstRun,
        Vec::new(),
        Vec::new(),
        data.clone(),
        provider_action_entries(),
        setup_prompts(),
    );
    app.on_key(setup_key(KeyCode::Enter));
    app.on_key(setup_key(KeyCode::Down));
    app.on_key(setup_key(KeyCode::Enter));
    let review = app.review_lines().join("\n");
    let SetupEffect::Submit { submission, .. } = app.on_key(setup_key(KeyCode::Enter)) else {
        panic!("review submits the GLM quick start");
    };
    let plan = setup_plan(submission).expect("GLM plan");
    let profile = &plan.patch.profiles[&data.profile];
    assert_eq!(profile.provider.as_deref(), Some(data.provider.as_str()));
    assert_eq!(profile.model.as_deref(), Some(data.model.as_str()));
    let model = &plan.patch.models[&format!("{}/{}", data.provider, data.model)];
    assert_eq!(model.context_tokens, Some(data.limits.context_tokens));
    assert_eq!(model.max_input_tokens, Some(data.limits.max_input_tokens));
    assert_eq!(model.max_output_tokens, Some(data.limits.max_output_tokens));
    let trusted = smith_config::setup::trusted_model(&data.provider, &data.model)
        .expect("written binding's trusted metadata");
    assert_eq!(data.catalog_revision, trusted.revision);
    assert_eq!(
        data.catalog_revision,
        smith_config::setup::TRUSTED_MODEL_CATALOG_REVISION
    );
    assert!(
        review
            .lines()
            .any(|line| line.starts_with("Model") && line.contains(&data.model)),
        "{review}"
    );
    assert!(
        review.contains(&format!("trusted catalog v{}", trusted.revision)),
        "{review}"
    );
}

#[tokio::test]
async fn anthropic_setup_publishes_a_native_provider_and_rolls_back_on_failure() {
    let root = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let user_dir = root.path().join(".smith");
    let context = setup_context_for(user_dir.clone(), project.path().to_owned());
    let actions = provider_action_entries();
    let index = actions
        .iter()
        .position(|entry| entry.id == KIND_ANTHROPIC_MESSAGES)
        .expect("Anthropic action");
    let mut app = SetupApp::new(
        SetupMode::FirstRun,
        Vec::new(),
        Vec::new(),
        glm_quick_start(),
        actions,
        setup_prompts(),
    );
    for _ in 0..index {
        app.on_key(setup_key(KeyCode::Down));
    }
    app.on_key(setup_key(KeyCode::Enter));
    for _ in 0..3 {
        app.on_key(setup_key(KeyCode::Down));
    }
    app.on_key(setup_key(KeyCode::Enter));
    app.on_paste("ANTHROPIC_API_KEY");
    app.on_key(setup_key(KeyCode::Enter));
    app.on_paste("claude-test-only");
    let SetupEffect::ResolveModelLimits { request } = app.on_key(setup_key(KeyCode::Enter)) else {
        panic!("native provider reaches shared limit resolution");
    };
    assert!(!request.use_endpoint_listing);
    assert!(resolve_model_limits(&context, &request).await.is_none());
    app.apply_resolved_limits(None);
    app.on_paste("64000");
    app.on_key(setup_key(KeyCode::Enter));
    app.on_key(setup_key(KeyCode::Enter));
    let SetupEffect::Submit {
        submission,
        allow_collisions,
    } = app.on_key(setup_key(KeyCode::Enter))
    else {
        panic!("Anthropic review submits");
    };
    let enroller = CredentialEnroller::with_backend(Arc::new(FakeEnrollmentBackend::default()));
    assert!(matches!(
        apply_submission_with(
            &context,
            submission.clone(),
            allow_collisions,
            &enroller,
            || async { Err(("synthetic failure".into(), false)) }
        )
        .await,
        ApplyOutcome::Failed { .. }
    ));
    assert!(!user_dir.join("config.toml").exists());
    assert!(matches!(
        apply_submission_with(
            &context,
            submission,
            allow_collisions,
            &enroller,
            || async { Ok(()) }
        )
        .await,
        ApplyOutcome::Completed
    ));
    let written = ConfigFile::parse(
        &std::fs::read_to_string(user_dir.join("config.toml")).expect("published config"),
    )
    .expect("valid TOML");
    let provider = &written.providers["anthropic"];
    assert_eq!(provider.kind.as_deref(), Some(KIND_ANTHROPIC_MESSAGES));
    assert_eq!(
        provider.base_url.as_deref(),
        Some(smith_config::model::ANTHROPIC_DEFAULT_ENDPOINT)
    );
    assert_eq!(
        provider.credential.as_deref(),
        Some("env:ANTHROPIC_API_KEY")
    );
    assert!(provider.response.is_none());
    assert!(written.default_profile.is_some());
    assert_eq!(
        written.models["anthropic/claude-test-only"].context_tokens,
        Some(64_000)
    );
    let request = ResolveRequest::new(project.path()).with_home_dir(root.path());
    let ConfigReadiness::Ready(resolution) = inspect(&request) else {
        panic!("the published native provider must be locally runnable");
    };
    assert_eq!(
        resolution.config.provider.kind.value,
        KIND_ANTHROPIC_MESSAGES
    );
    assert_eq!(resolution.config.model.value, "claude-test-only");
}

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

    fn store(&self, _service: &str, _account: &str, secret: &Secret) -> Result<(), KeychainError> {
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

#[tokio::test]
async fn an_unreachable_endpoint_falls_back_to_a_same_name_catalog_match() {
    let context = setup_context_for(
        tempfile::tempdir().expect("home").path().to_path_buf(),
        tempfile::tempdir().expect("project").path().to_path_buf(),
    );
    // Any catalog id works and survives seed regeneration, so pick one
    // the embedded snapshot actually carries.
    let (provider, model) = context
        .catalog
        .providers
        .iter()
        .find_map(|(provider, catalog)| {
            catalog
                .models
                .values()
                .find(|model| model.limits.is_some())
                .map(|model| (provider.clone(), model.id.clone()))
        })
        .expect("the embedded catalog carries a limited model");
    let request = ResolveModelLimits {
        use_endpoint_listing: true,
        // Port 9 refuses connections immediately, so the test stays
        // offline and the probe fails fast.
        endpoint: Some("http://127.0.0.1:9/v1".to_owned()),
        bearer: None,
        environment_variable: None,
        provider: None,
        model: model.clone(),
    };
    let resolved = resolve_model_limits(&context, &request)
        .await
        .expect("the catalog fallback resolves");
    let limits = context
        .catalog
        .resolve_model_by_id(&model)
        .expect("the same match again")
        .1
        .limits
        .expect("complete limits");
    assert_eq!(resolved.context_tokens, limits.context_tokens);
    assert_eq!(resolved.max_input_tokens, limits.max_input_tokens);
    assert_eq!(resolved.max_output_tokens, limits.max_output_tokens);
    assert_eq!(
        resolved.source,
        format!("trusted catalog match {provider}/{model}")
    );
}

#[tokio::test]
async fn an_unknown_model_on_an_unreachable_endpoint_resolves_nothing() {
    let context = setup_context_for(
        tempfile::tempdir().expect("home").path().to_path_buf(),
        tempfile::tempdir().expect("project").path().to_path_buf(),
    );
    let request = ResolveModelLimits {
        use_endpoint_listing: true,
        endpoint: Some("http://127.0.0.1:9/v1".to_owned()),
        bearer: None,
        environment_variable: None,
        provider: None,
        model: "no-such-model-anywhere".to_owned(),
    };
    assert!(resolve_model_limits(&context, &request).await.is_none());
}

#[tokio::test]
async fn a_gateway_spelled_claude_fable_alias_resolves_from_the_embedded_catalog() {
    let context = setup_context_for(
        tempfile::tempdir().expect("home").path().to_path_buf(),
        tempfile::tempdir().expect("project").path().to_path_buf(),
    );
    let request = ResolveModelLimits {
        use_endpoint_listing: true,
        endpoint: Some("http://127.0.0.1:9/v1".to_owned()),
        bearer: None,
        environment_variable: None,
        provider: None,
        model: "claude-fable-5-1".to_owned(),
    };
    let resolved = resolve_model_limits(&context, &request)
        .await
        .expect("the separator-normalized catalog alias resolves");
    assert_eq!(resolved.context_tokens, 1_000_000);
    assert_eq!(resolved.max_input_tokens, 1_000_000);
    assert_eq!(resolved.max_output_tokens, 128_000);
    assert!(
        resolved
            .source
            .contains("openrouter/anthropic/claude-fable-5.1"),
        "{}",
        resolved.source
    );
}

#[tokio::test]
async fn a_configured_non_openai_compatible_provider_is_not_probed() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    std::fs::create_dir_all(project.path().join(".smith")).expect("project config dir");
    std::fs::write(
            project.path().join(".smith/config.toml"),
            "[providers.native]\nkind = \"gemini-interactions\"\nbase_url = \"https://example.test/v1\"\n",
        )
        .expect("config");
    let context = setup_context_for(home.path().to_path_buf(), project.path().to_path_buf());
    // The section exists, but its adapter is not the one whose listing
    // protocol Smith reviewed, so no probe target is derived.
    assert!(configured_provider_section(&context, "native").is_some());
    let (endpoint, bearer) = configured_probe_target(&context, "native").await;
    assert_eq!(endpoint, None);
    assert_eq!(bearer, None);
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
        catalog: Arc::new(
            serde_json::from_str(smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED)
                .expect("embedded catalog"),
        ),
        unconfigured: true,
    }
}

#[test]
fn checkpoint_source_rotation_refuses_existing_encrypted_state_without_modification() {
    let root = tempfile::tempdir().expect("root");
    let user_dir = root.path().join(".smith");
    let sessions_dir = user_dir.join("sessions");
    let project_dir = sessions_dir.join("project-id");
    std::fs::create_dir_all(&project_dir).expect("session directory");
    std::fs::write(project_dir.join("session.checkpoint.bin"), b"protected").expect("checkpoint");
    let context = CheckpointSetupContext {
        user_dir: user_dir.clone(),
        sessions_dir,
        source: ExistingCheckpointSource::Credential("env:SMITH_CHECKPOINT_SECRET".to_owned()),
    };

    let error =
        refuse_unsafe_checkpoint_rotation(&context, &ExistingCheckpointSource::GeneratedInline)
            .expect_err("rotation must refuse");
    assert!(error.to_string().contains("configuration was not modified"));
    assert!(!user_dir.join("config.toml").exists());

    assert!(
        refuse_unsafe_checkpoint_rotation(
            &context,
            &ExistingCheckpointSource::Credential("env:SMITH_CHECKPOINT_SECRET".to_owned())
        )
        .is_ok(),
        "reselecting the identical source does not rotate a key"
    );
}

#[test]
fn checkpoint_inventory_is_bounded_and_does_not_follow_symlinks() {
    let root = tempfile::tempdir().expect("root");
    let sessions = root.path().join("sessions");
    std::fs::create_dir_all(sessions.join("project")).expect("sessions");
    std::fs::write(sessions.join("project/not-a-checkpoint.bin.txt"), b"x").expect("ordinary file");
    assert!(!protected_checkpoint_exists(&sessions).expect("scan"));
    std::fs::write(sessions.join("project/s.checkpoint.bin"), b"x").expect("checkpoint");
    assert!(protected_checkpoint_exists(&sessions).expect("scan"));
}

#[test]
fn xai_plan_writes_the_endpoint_the_catalog_binds_to() {
    let secret = "sk-xai-do-not-print";
    let plan = setup_plan(SetupSubmission::QuickXai {
        credential: SetupCredential::StoreInKeychain(agent_runtime_core::store::Secret::new(
            secret,
        )),
        model: "grok-4.5".into(),
    })
    .expect("a plan");

    let serialized = toml::to_string(&plan.patch).expect("TOML");
    assert!(!serialized.contains(secret), "{serialized}");
    assert!(serialized.contains("keychain:smith/xai"));
    assert_eq!(
        plan.patch.providers[XAI_PROVIDER].kind.as_deref(),
        Some(KIND_OPENAI_RESPONSES)
    );
    // The endpoint is what binds this provider to the catalog entry that
    // supplies its limits, so a plan omitting it would need a hand-written
    // `[models]` table to run at all.
    assert_eq!(
        plan.patch.providers[XAI_PROVIDER].base_url.as_deref(),
        Some(XAI_ENDPOINT)
    );
    assert!(plan.patch.models.is_empty());
    assert_eq!(plan.patch.profiles[XAI_PROFILE].max_output_tokens, None);
    assert_eq!(
        plan.patch.profiles[XAI_PROFILE].model.as_deref(),
        Some("grok-4.5")
    );
}

#[test]
fn an_empty_xai_model_is_refused_rather_than_written() {
    assert!(
        setup_plan(SetupSubmission::QuickXai {
            credential: SetupCredential::Environment("XAI_API_KEY".into()),
            model: "  ".into(),
        })
        .is_err()
    );
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
        plan.patch.models["zai/glm-5.3"].context_tokens,
        Some(1_000_000)
    );
    assert_eq!(plan.patch.profiles[GLM_PROFILE].max_output_tokens, None);
    assert_eq!(
        plan.patch.profile_order.as_ref().expect("profile order"),
        &[
            GLM_PROFILE.to_owned(),
            format!("{GLM_PROFILE}-plan"),
            format!("{GLM_PROFILE}-review"),
        ]
    );
    assert_eq!(
        plan.patch.profiles[GLM_PROFILE].posture,
        Some(AgentPosture::Build)
    );
    assert_eq!(
        plan.patch.profiles[GLM_PROFILE].uses.as_deref(),
        Some([ProfileUse::Main, ProfileUse::Child].as_slice())
    );
    let plan_profile = &plan.patch.profiles[&format!("{GLM_PROFILE}-plan")];
    assert_eq!(plan_profile.extends.as_deref(), Some(GLM_PROFILE));
    assert_eq!(plan_profile.posture, Some(AgentPosture::Plan));
    let review_profile = &plan.patch.profiles[&format!("{GLM_PROFILE}-review")];
    assert_eq!(review_profile.extends.as_deref(), Some(GLM_PROFILE));
    assert_eq!(review_profile.posture, Some(AgentPosture::Review));
}

#[test]
fn google_plan_uses_the_catalog_model_without_copying_endpoint_or_limits() {
    let secret = "sk-google-plan-do-not-print";
    let plan = setup_plan(SetupSubmission::QuickGoogle {
        credential: SetupCredential::StoreInKeychain(Secret::new(secret)),
        model: "gemini-3.6-flash".to_owned(),
    })
    .expect("a Google plan");
    let serialized = toml::to_string(&plan.patch).expect("TOML");
    assert!(!serialized.contains(secret), "{serialized}");
    assert!(serialized.contains("keychain:smith/google"));
    assert_eq!(
        plan.patch.providers[GOOGLE_PROVIDER].kind.as_deref(),
        Some(KIND_GEMINI_INTERACTIONS)
    );
    assert_eq!(plan.patch.providers[GOOGLE_PROVIDER].base_url, None);
    assert!(plan.patch.models.is_empty());
    assert_eq!(plan.patch.profiles[GOOGLE_PROFILE].max_output_tokens, None);
    assert_eq!(plan.patch.profiles[GOOGLE_PROFILE].context, None);
    assert_eq!(
        plan.patch.profiles[GOOGLE_PROFILE].model.as_deref(),
        Some("gemini-3.6-flash")
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
            kind: SetupProviderKind::OpenAiCompatible,
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
            kind: SetupProviderKind::OpenAiCompatible,
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

#[test]
fn completion_summary_names_the_change_and_destination() {
    let destination = std::path::Path::new("/home/u/.smith/config.toml");
    let limits = SetupModelLimits {
        context_tokens: 1,
        max_input_tokens: 1,
        max_output_tokens: 1,
    };
    assert_eq!(
        super::completion_summary(
            &SetupSubmission::AddModel {
                provider: "local".into(),
                model: "m".into(),
                limits,
                make_default: false,
            },
            destination,
        ),
        "Setup complete · added local/m · saved to /home/u/.smith/config.toml"
    );
    assert_eq!(
        super::completion_summary(
            &SetupSubmission::ChangeDefault {
                provider: "local".into(),
                model: "m".into(),
            },
            destination,
        ),
        "Setup complete · default is now local/m · saved to /home/u/.smith/config.toml"
    );
}
