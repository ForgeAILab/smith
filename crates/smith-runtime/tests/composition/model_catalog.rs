use super::*;

/// A Z.AI Coding Plan profile whose selected model exists only in the catalog.
const ZAI_CATALOG_CONFIG: &str = r#"
default_profile = "glm"

[profiles.glm]
provider = "zai"
model = "glm-5-turbo"

[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "env:ZAI_API_KEY"

[approval]
mode = "allow-all"
"#;

#[tokio::test]
async fn catalog_only_selection_resolves_frozen_limits_before_any_provider_request() {
    let fixture = Fixture::new(ZAI_CATALOG_CONFIG);
    let snapshot: CatalogSnapshot =
        serde_json::from_str(EMBEDDED_MODELS_DEV_SEED).expect("embedded snapshot");
    let source = runtime_catalog_source(
        &snapshot,
        "zai",
        "openai-compatible",
        Some("https://api.z.ai/api/coding/paas/v4"),
    )
    .expect("Z.AI catalog source");
    let provider = Arc::new(FakeProvider::text_reply("unused"));
    let request = RuntimeRequest {
        credentials: Some(resolver(Some(TOKEN))),
        provider: Some(provider.clone() as Arc<dyn Provider>),
        catalog_sources: vec![source],
        ..request(&fixture, HostSurface::Headless)
    };

    let smith = factory::build_request(request)
        .await
        .expect("catalog-backed runtime");
    assert_eq!(
        smith.policy().model_profile.limits,
        ModelLimits::new(200_000, 200_000, 131_072)
    );
    let provenance = smith
        .policy()
        .model_profile
        .provenance_of(ProfileField::ContextTokens)
        .expect("catalog provenance");
    assert_eq!(provenance.source, CatalogSource::CachedRemote);
    assert_eq!(
        provenance.source_revision.as_deref(),
        Some(snapshot.source_revision.as_str())
    );
    assert_eq!(
        provenance.retrieved,
        Some(agent_runtime_core::clock::Timestamp(
            snapshot.retrieved_at_ms
        ))
    );
    assert!(
        provider.requests().is_empty(),
        "catalog-backed preflight must not send a provider request"
    );
}

#[tokio::test]
async fn catalog_ceiling_equal_to_context_gets_one_frozen_automatic_budget() {
    let fixture = Fixture::new(NO_LIMITS_CONFIG);
    let source = StaticSource::new("models.dev", CatalogSource::CachedRemote).with_model(
        "example-model",
        ModelRecord::new()
            .with_limits(ModelLimits::new(500_000, 500_000, 500_000))
            .with_revision("models-dev-equal-ceiling"),
    );
    let provider = Arc::new(FakeProvider::text_reply("unused"));
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        catalog_sources: vec![Arc::new(source)],
        ..request(&fixture, HostSurface::Headless)
    };

    let smith = factory::build_request(request)
        .await
        .expect("an automatically budgeted runtime");
    assert_eq!(
        smith.policy().model_profile.limits,
        ModelLimits::new(500_000, 500_000, 500_000),
        "automatic policy must not rewrite the catalog ceiling"
    );
    assert_eq!(smith.policy().max_output_tokens, Some(32_768));
    assert_eq!(smith.policy().context_policy.output_reserve, 32_768);
    assert!(
        smith
            .harness_report()
            .entries
            .iter()
            .any(|entry| entry == "output=request:32768:automatic reserve:32768:automatic")
    );
    assert!(
        provider.requests().is_empty(),
        "composition must not spend a provider request"
    );
}

#[tokio::test]
async fn explicit_reserve_conflict_fails_before_credential_resolution() {
    const CONFLICTING_RESERVE_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "remote"
model = "example-model"

[providers.remote]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "env:MUST_NOT_BE_READ"

[models."remote/example-model"]
context_tokens = 8000
max_input_tokens = 8000
max_output_tokens = 4000

[context]
output_reserve = 6000
reasoning_reserve = 2000

[approval]
mode = "allow-all"
"#;

    let fixture = Fixture::new(CONFLICTING_RESERVE_CONFIG);
    let credentials = CredentialResolver::new("/nonexistent-user-state")
        .with_environment(Arc::new(PanicsIfCredentialResolved));
    let request = RuntimeRequest {
        credentials: Some(credentials),
        ..request(&fixture, HostSurface::Headless)
    };

    let error = factory::build_request(request)
        .await
        .expect_err("explicit reserves consume the context");
    assert!(matches!(error, FactoryError::ContextReserve { .. }));
}

#[tokio::test]
async fn explicit_limits_beat_catalog_metadata_and_both_provenances_survive() {
    const PARTIAL_LIMITS_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
max_output_tokens = 2048

[approval]
mode = "allow-all"
"#;

    let fixture = Fixture::new(PARTIAL_LIMITS_CONFIG);
    let cached = StaticSource::new("models.dev", CatalogSource::CachedRemote).with_model(
        "example-model",
        ModelRecord::new()
            .with_limits(ModelLimits::new(128_000, 124_000, 8_192))
            .with_revision("models-dev-r7"),
    );
    let request = RuntimeRequest {
        catalog_sources: vec![Arc::new(cached)],
        ..request(&fixture, HostSurface::Terminal)
    };

    let smith = factory::build_request(request).await.expect("a runtime");
    let resolution = smith.profile();

    // The configured value wins; the ones configuration was silent about fall
    // through to the catalog.
    assert_eq!(resolution.profile.limits.max_output_tokens, 2_048);
    assert_eq!(resolution.profile.limits.context_tokens, 128_000);
    assert_eq!(
        resolution
            .profile
            .provenance_of(ProfileField::MaxOutputTokens)
            .expect("catalog provenance")
            .source,
        CatalogSource::Explicit
    );
    assert_eq!(
        resolution
            .profile
            .provenance_of(ProfileField::ContextTokens)
            .expect("catalog provenance")
            .source_revision
            .as_deref(),
        Some("models-dev-r7")
    );

    // Both halves of the provenance survive: the shared catalog layer, and the
    // Smith file and key that supplied the winner.
    let offers = resolution.contributions_for(ProfileField::MaxOutputTokens);
    assert_eq!(offers.len(), 2);
    assert_eq!(
        (offers[0].layer, offers[0].value),
        (CatalogSource::Explicit, 2_048)
    );
    assert_eq!(
        (offers[1].layer, offers[1].value),
        (CatalogSource::CachedRemote, 8_192)
    );
    let configured = resolution
        .configured_source(ProfileField::MaxOutputTokens)
        .expect("a Smith configuration source");
    assert_eq!(configured.layer, Layer::ProjectFile);
    assert!(
        configured
            .file
            .as_ref()
            .expect("a file")
            .ends_with(".smith/config.toml"),
        "{configured}"
    );
    assert!(
        resolution
            .configured_source(ProfileField::ContextTokens)
            .is_none()
    );
}
