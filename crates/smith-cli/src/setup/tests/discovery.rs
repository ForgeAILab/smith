use super::*;
use smith_config::catalog::{
    CatalogLimits, CatalogProvider, EndpointCatalog, PartialCatalogLimits,
};
use std::collections::BTreeMap;

fn endpoint_context(user_dir: PathBuf, project: PathBuf, explicit: &str) -> SetupContext {
    std::fs::write(user_dir.join("config.toml"), format!(
        "[providers.xiaoge]\nkind = \"openai-compatible\"\nbase_url = \"http://127.0.0.1:9/v1\"\napi_key = \"test-only\"\n{explicit}"
    )).expect("provider config");
    let mut context = setup_context_for(user_dir, project);
    let mut model = context
        .catalog
        .providers
        .values()
        .flat_map(|provider| provider.models.values())
        .next()
        .expect("seed model")
        .clone();
    model.id = "selected-large".into();
    model.name = "Selected Large".into();
    model.limits = Some(CatalogLimits {
        context_tokens: 1_048_576,
        max_input_tokens: 1_048_576,
        max_output_tokens: 256_000,
    });
    model.disabled_reason = None;
    Arc::make_mut(&mut context.catalog)
        .endpoint_catalogs
        .insert(
            "xiaoge".into(),
            EndpointCatalog {
                binding_digest: "fixture-binding".into(),
                kind: "openai-compatible".into(),
                base_url: "http://127.0.0.1:9/v1".into(),
                source: "endpoint /models listing".into(),
                revision: "test-revision".into(),
                retrieved_at_ms: 123,
                catalog: CatalogProvider {
                    id: "xiaoge".into(),
                    name: "Xiaoge".into(),
                    models: BTreeMap::from([("selected-large".into(), model)]),
                },
                partial_limits: BTreeMap::new(),
                unadvertised_capabilities: Default::default(),
            },
        );
    context.discovery_attempted.insert("xiaoge".into());
    context
}

#[tokio::test]
async fn discovered_limits_are_frozen_and_existing_output_wins_over_advertisement() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let mut context = endpoint_context(
        home.path().into(),
        project.path().into(),
        "[models.\"xiaoge/selected-large\"]\nmax_output_tokens = 200000\n",
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let endpoint = format!("http://{}/v1", listener.local_addr().expect("address"));
    let path = home.path().join("config.toml");
    let text = std::fs::read_to_string(&path).expect("config");
    std::fs::write(path, text.replace("http://127.0.0.1:9/v1", &endpoint)).expect("endpoint");
    Arc::make_mut(&mut context.catalog)
        .endpoint_catalogs
        .get_mut("xiaoge")
        .expect("cached endpoint")
        .base_url = endpoint;
    let request = ResolveModelLimits {
        use_endpoint_listing: true,
        endpoint: None,
        bearer: None,
        environment_variable: None,
        provider: Some("xiaoge".into()),
        model: "selected-large".into(),
    };
    // A second listing request would make the listener ready before there is
    // any response. Manual-ID recovery must use exactly the prepared snapshot.
    let limits = tokio::select! {
        limits = resolve_model_limits(&context, &request) => limits.expect("cached limits"),
        _ = listener.accept() => panic!("listing was fetched twice"),
    };
    assert_eq!(limits.context_tokens, 1_048_576);
    assert_eq!(limits.max_output_tokens, 200_000);
    assert!(limits.source.contains("test-revision"));
    assert!(limits.source.contains("existing explicit fields preserved"));
    let (entries, values) = super::super::provider_model_entries(&context, "xiaoge");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].detail.contains("200k output ceiling"));
    assert_eq!(values["selected-large"].max_output_tokens, 200_000);
}

#[test]
fn missing_context_preserves_published_partial_ceilings_for_manual_recovery() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let mut context = endpoint_context(home.path().into(), project.path().into(), "");
    let endpoint = Arc::make_mut(&mut context.catalog)
        .endpoint_catalogs
        .get_mut("xiaoge")
        .expect("endpoint");
    let model = endpoint
        .catalog
        .models
        .get_mut("selected-large")
        .expect("model");
    model.limits = None;
    model.disabled_reason =
        Some("endpoint model has no complete context/output limit declaration".into());
    endpoint.partial_limits.insert(
        "selected-large".into(),
        PartialCatalogLimits {
            context_tokens: None,
            max_input_tokens: Some(1_000_000),
            max_output_tokens: Some(256_000),
        },
    );
    assert_eq!(
        super::super::model_limits::known_model_ceilings(&context, "xiaoge", "selected-large"),
        (Some(1_000_000), Some(256_000))
    );
    let (entries, values) = super::super::provider_model_entries(&context, "xiaoge");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].disabled_reason.is_none());
    assert!(values.is_empty());
}

#[tokio::test]
async fn cached_model_selection_creates_no_toml_metadata_and_readd_has_no_collision() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let context = endpoint_context(home.path().into(), project.path().into(), "");
    let submission = SetupSubmission::AddModel {
        provider: "xiaoge".into(),
        model: "selected-large".into(),
        limits: SetupModelLimits {
            context_tokens: 1_048_576,
            max_input_tokens: 1_048_576,
            max_output_tokens: 256_000,
        },
        make_default: false,
    };
    let enroller = CredentialEnroller::with_backend(Arc::new(FakeEnrollmentBackend::default()));
    for _ in 0..2 {
        assert!(matches!(
            apply_submission_with(&context, submission.clone(), false, &enroller, || async {
                Ok(())
            })
            .await,
            ApplyOutcome::Completed
        ));
    }
    let file = ConfigFile::parse(
        &std::fs::read_to_string(home.path().join("config.toml")).expect("config"),
    )
    .expect("valid config");
    assert!(
        file.models.is_empty(),
        "discovery must keep generated metadata in the cache"
    );
}

#[tokio::test]
async fn unchanged_explicit_output_has_no_collision_but_deliberate_override_is_reviewed() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let context = endpoint_context(
        home.path().into(),
        project.path().into(),
        "[models.\"xiaoge/selected-large\"]\ncontext_tokens = 1048576\nmax_input_tokens = 1048576\nmax_output_tokens = 256000\n",
    );
    let mut limits = SetupModelLimits {
        context_tokens: 1_048_576,
        max_input_tokens: 1_048_576,
        max_output_tokens: 256_000,
    };
    let enroller = CredentialEnroller::with_backend(Arc::new(FakeEnrollmentBackend::default()));
    let submit = |limits| SetupSubmission::AddModel {
        provider: "xiaoge".into(),
        model: "selected-large".into(),
        limits,
        make_default: false,
    };
    assert!(matches!(
        apply_submission_with(&context, submit(limits), false, &enroller, || async {
            Ok(())
        })
        .await,
        ApplyOutcome::Completed
    ));
    limits.max_output_tokens = 32_768;
    let result = apply_submission_with(&context, submit(limits), false, &enroller, || async {
        panic!("a differing override must be reviewed first")
    })
    .await;
    let ApplyOutcome::Collision(preview) = result else {
        panic!("override collision");
    };
    assert!(preview.contains("256000"), "{preview}");
    assert!(preview.contains("32768"), "{preview}");
}

#[tokio::test]
async fn stale_account_binding_is_rejected_before_setup_writes() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let context = endpoint_context(home.path().into(), project.path().into(), "");
    let path = home.path().join("config.toml");
    let before = std::fs::read_to_string(&path).expect("config");
    // The fixture's frozen digest belongs to an earlier account. Validation
    // must happen before creating model/default records from its metadata.
    let result = super::super::transaction::apply_submission(
        &context,
        SetupSubmission::AddModel {
            provider: "xiaoge".into(),
            model: "selected-large".into(),
            limits: SetupModelLimits {
                context_tokens: 1_048_576,
                max_input_tokens: 1_048_576,
                max_output_tokens: 256_000,
            },
            make_default: true,
        },
        false,
        &crate::screen_runner::EffectCancellation::default(),
    )
    .await;
    let ApplyOutcome::Failed { message, .. } = result else {
        panic!("stale binding accepted");
    };
    assert!(!message.contains("test-only"), "{message}");
    assert_eq!(
        std::fs::read_to_string(path).expect("unchanged config"),
        before
    );
}

#[test]
fn explicit_limit_and_provider_leaves_follow_runtime_project_local_precedence() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let user_dir = home.path().join(".smith");
    std::fs::create_dir_all(&user_dir).expect("user directory");
    std::fs::create_dir_all(project.path().join(".smith")).expect("project directory");
    let mut context = endpoint_context(
        user_dir.clone(),
        project.path().into(),
        "[models.\"xiaoge/selected-large\"]\ncontext_tokens = 1048576\nmax_input_tokens = 1000000\nmax_output_tokens = 256000\n",
    );
    let user_path = user_dir.join("config.toml");
    let user = std::fs::read_to_string(&user_path).expect("user config");
    std::fs::write(
        &user_path,
        format!(
            "default_profile = \"code\"\n[profiles.code]\nprovider = \"xiaoge\"\nmodel = \"selected-large\"\n{}",
            user.replace(
                "api_key = \"test-only\"",
                "credential = \"env:SMITH_SETUP_TEST_KEY\""
            )
        ),
    )
    .expect("selection");
    std::fs::write(project.path().join(".smith/config.toml"),
        "[providers.xiaoge.headers]\nX-Project = \"project\"\n[models.\"xiaoge/selected-large\"]\nmax_output_tokens = 128000\n")
        .expect("project config");
    std::fs::write(project.path().join(".smith/config.local.toml"),
        "[providers.xiaoge.headers]\nX-Project = \"local\"\n[models.\"xiaoge/selected-large\"]\nmax_output_tokens = 64000\n")
        .expect("local config");
    let request = ResolveRequest::new(project.path())
        .with_home_dir(home.path())
        .with_known_modules(crate::modules::known_modules())
        .with_catalog(context.catalog.clone());
    let ConfigReadiness::Ready(resolution) = inspect(&request) else {
        panic!("layered provider must resolve");
    };
    context.inventory = smith_config::inventory::local_inventory_with_catalog(
        &resolution,
        AVAILABLE_ADAPTER_KINDS,
        Some(&context.catalog),
    )
    .expect("effective inventory");
    let effective =
        super::super::model_limits::configured_model_section(&context, "xiaoge", "selected-large");
    assert_eq!(effective.context_tokens, Some(1_048_576));
    assert_eq!(effective.max_input_tokens, Some(1_000_000));
    assert_eq!(effective.max_output_tokens, Some(64_000));
    assert_eq!(
        super::super::model_limits::prepared_model_limits(&context, "xiaoge", "selected-large")
            .expect("detected limits")
            .max_output_tokens,
        64_000
    );
    let provider = super::super::model_limits::configured_provider_section(&context, "xiaoge")
        .expect("provider");
    assert_eq!(provider.kind.as_deref(), Some("openai-compatible"));
    assert_eq!(provider.base_url.as_deref(), Some("http://127.0.0.1:9/v1"));
    assert_eq!(
        provider.credential.as_deref(),
        Some("env:SMITH_SETUP_TEST_KEY")
    );
    assert_eq!(
        provider.headers.get("X-Project").map(String::as_str),
        Some("local")
    );
    // Recovery contexts without an existing inventory use the same ordered
    // field-level overlay rather than dropping project-local leaves.
    context.inventory = SelectionInventory::default();
    assert_eq!(
        super::super::model_limits::configured_model_section(&context, "xiaoge", "selected-large")
            .max_output_tokens,
        Some(64_000)
    );
}

#[tokio::test]
async fn discovery_never_borrows_same_id_limits_from_another_deployment() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let mut context = endpoint_context(home.path().into(), project.path().into(), "");
    let foreign = context
        .catalog
        .providers
        .values()
        .flat_map(|provider| provider.models.values())
        .find(|model| model.limits.is_some())
        .expect("foreign catalog model")
        .clone();
    assert!(context.catalog.resolve_model_by_id(&foreign.id).is_some());
    let endpoint = Arc::make_mut(&mut context.catalog)
        .endpoint_catalogs
        .get_mut("xiaoge")
        .expect("custom snapshot");
    let mut incomplete = foreign.clone();
    incomplete.limits = None;
    incomplete.disabled_reason =
        Some("endpoint model has no complete context/output limit declaration".into());
    endpoint.catalog.models = BTreeMap::from([(foreign.id.clone(), incomplete)]);
    endpoint.partial_limits = BTreeMap::from([(
        foreign.id.clone(),
        PartialCatalogLimits {
            context_tokens: None,
            max_input_tokens: None,
            max_output_tokens: Some(256_000),
        },
    )]);
    let (entries, limits) = super::super::provider_model_entries(&context, "xiaoge");
    assert_eq!(entries.len(), 1);
    assert!(
        limits.is_empty(),
        "foreign deployment metadata must not complete this snapshot"
    );
    assert_eq!(
        super::super::model_limits::known_model_ceilings(&context, "xiaoge", &foreign.id),
        (None, Some(256_000))
    );
    let request = ResolveModelLimits {
        use_endpoint_listing: true,
        endpoint: None,
        bearer: None,
        environment_variable: None,
        provider: Some("xiaoge".into()),
        model: foreign.id,
    };
    assert!(
        resolve_model_limits(&context, &request).await.is_none(),
        "a missing custom context must enter manual recovery"
    );
}

#[tokio::test]
async fn unchanged_project_local_limits_do_not_replace_lower_user_values() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    std::fs::create_dir_all(project.path().join(".smith")).expect("project config directory");
    let mut context = endpoint_context(
        home.path().into(),
        project.path().into(),
        "[models.\"xiaoge/selected-large\"]\ncontext_tokens = 1048576\nmax_input_tokens = 1048576\nmax_output_tokens = 256000\n",
    );
    // No complete endpoint catalog is needed to recognize an unchanged
    // explicit re-add. The project-local leaf already supplies the winner.
    Arc::make_mut(&mut context.catalog)
        .endpoint_catalogs
        .clear();
    std::fs::write(
        project.path().join(".smith/config.local.toml"),
        "[models.\"xiaoge/selected-large\"]\nmax_output_tokens = 64000\n",
    )
    .expect("project-local override");
    let path = home.path().join("config.toml");
    let before = std::fs::read_to_string(&path).expect("user config");
    let enroller = CredentialEnroller::with_backend(Arc::new(FakeEnrollmentBackend::default()));
    let outcome = apply_submission_with(
        &context,
        SetupSubmission::AddModel {
            provider: "xiaoge".into(),
            model: "selected-large".into(),
            limits: SetupModelLimits {
                context_tokens: 1_048_576,
                max_input_tokens: 1_048_576,
                max_output_tokens: 64_000,
            },
            make_default: false,
        },
        false,
        &enroller,
        || async { Ok(()) },
    )
    .await;
    assert!(matches!(outcome, ApplyOutcome::Completed));
    assert_eq!(
        std::fs::read_to_string(path).expect("user config"),
        before,
        "an unchanged effective selection must preserve lower-layer values"
    );
}
