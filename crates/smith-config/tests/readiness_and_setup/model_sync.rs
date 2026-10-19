use super::*;
use smith_config::resolve::{
    Source, Sourced, endpoint_discovery_authorized, resolve_model_sync_request,
    resolve_user_model_sync, resolve_user_provider,
};

#[test]
fn model_sync_defaults_and_user_overrides_have_explainable_sources() {
    let fixture = Fixture::new();
    fixture.write_user(READY);
    let resolution = resolve(&fixture.request()).expect("ready defaults");
    assert!(resolution.config.model_sync.enabled.value);
    assert_eq!(resolution.config.model_sync.interval_hours.value, 24);
    assert_eq!(
        resolution.config.model_sync.enabled.source.layer,
        Layer::BuiltIn
    );
    fixture.write_user(&format!(
        "{READY}\n[model_sync]\nenabled = false\ninterval_hours = 168\n"
    ));
    let resolution = resolve(&fixture.request()).expect("user policy");
    assert!(!resolution.config.model_sync.enabled.value);
    assert_eq!(resolution.config.model_sync.interval_hours.value, 168);
    let explanation = resolution
        .provenance
        .explain("model_sync.interval_hours")
        .unwrap();
    assert_eq!(explanation.source.layer, Layer::UserFile);
    assert_eq!(explanation.source.key, "model_sync.interval_hours");
    assert_eq!(
        explanation.overridden[0].value,
        smith_config::resolve::SettingValue::Integer(24)
    );
}

#[test]
fn model_sync_refuses_invalid_intervals_and_project_authority() {
    for invalid in ["0", "169", "-1", "1.5", "\"24\""] {
        let fixture = Fixture::new();
        fixture.write_user(&format!(
            "{READY}\n[model_sync]\ninterval_hours = {invalid}\n"
        ));
        let error = resolve(&fixture.request()).expect_err("invalid refresh interval");
        assert!(error.to_string().contains("interval_hours"), "{error}");
    }
    for policy in ["enabled = false", "interval_hours = 24", ""] {
        let fixture = Fixture::new();
        fixture.write_user(READY);
        fixture.write_project(&format!("[model_sync]\n{policy}\n"));
        let error = resolve(&fixture.request()).expect_err("project policy is forbidden");
        assert!(matches!(error, ConfigError::InvalidValue { source, .. }
            if source.layer == Layer::ProjectFile && source.key.starts_with("model_sync")));
    }
}

#[test]
fn model_sync_environment_cannot_override_user_policy() {
    let fixture = Fixture::new();
    fixture.write_user(READY);
    let mut request = fixture.request();
    request
        .env
        .insert("SMITH_MODEL_SYNC_ENABLED".to_owned(), "true".to_owned());
    assert!(
        matches!(resolve(&request), Err(ConfigError::InvalidValue { source, .. })
        if source.layer == Layer::Environment && source.key == "SMITH_MODEL_SYNC_ENABLED")
    );
}

#[test]
fn user_discovery_resolution_needs_no_selected_model_or_credential_read() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
[model_sync]
enabled = false
interval_hours = 12
[providers.xiaoge]
kind = "openai-compatible"
base_url = "https://user.example/v1"
credential = "file:does-not-exist"
"#,
    );
    fixture.write_project(
        r#"
[providers.xiaoge]
base_url = "https://project.example/v1"
"#,
    );
    let user_dir = fixture.home.path().join(".smith");
    let policy = resolve_user_model_sync(&user_dir).expect("policy without model selection");
    assert!(!policy.enabled.value);
    assert_eq!(policy.interval_hours.value, 12);
    let provider = resolve_user_provider(&user_dir, "xiaoge").expect("user provider");
    assert_eq!(provider.base_url.unwrap().value, "https://user.example/v1");
    assert_eq!(provider.credentials[0].value, "file:does-not-exist");
    assert!(resolve_user_provider(&user_dir, "missing").is_err());
}

#[test]
fn synchronization_authority_validation_does_not_require_a_resolvable_model_selection() {
    let fixture = Fixture::new();
    fixture.write_user("[model_sync]\ninterval_hours = 12\n");
    let request = fixture.request();
    assert!(resolve(&request).is_err(), "no active provider or model");
    assert_eq!(
        resolve_model_sync_request(&request)
            .unwrap()
            .interval_hours
            .value,
        12
    );
    fixture.write_project("[model_sync]\nenabled = true\n");
    assert!(matches!(resolve_model_sync_request(&request),
        Err(ConfigError::InvalidValue{source,..}) if source.layer == Layer::ProjectFile));
}

const DISCOVERY_PROVIDER: &str = r#"
[providers.xiaoge]
kind = "openai-compatible"
base_url = "https://user.example/v1"
credential = "env:USER_ACCOUNT"
[providers.xiaoge.headers]
X-Tenant = "user-tenant"
X-Region = "user-region"
"#;

#[test]
fn endpoint_discovery_checks_effective_auth_for_active_and_inactive_providers() {
    for active in [false, true] {
        let fixture = Fixture::new();
        fixture.write_user(&format!("{READY}\n{DISCOVERY_PROVIDER}"));
        let request = if active {
            fixture.request().with_cli(Overrides {
                provider: Some("xiaoge".to_owned()),
                model: Some("cached-model".to_owned()),
                ..Overrides::default()
            })
        } else {
            fixture.request()
        };
        let resolution = resolve(&request).unwrap();
        assert!(endpoint_discovery_authorized(&resolution, "xiaoge").unwrap());
        assert!(!endpoint_discovery_authorized(&resolution, "project-only").unwrap());
        for project in [
            "[providers.xiaoge]\ncredential = \"env:PROJECT_ACCOUNT\"\n",
            "[providers.xiaoge.headers]\nX-Tenant = \"project-tenant\"\n",
            "[providers.xiaoge.headers]\nX-New-Tenant = \"extra-scope\"\n",
            "[providers.xiaoge]\nbase_url = \"https://other.example/v1\"\n",
        ] {
            fixture.write_project(project);
            let resolution = resolve(&request).unwrap();
            assert!(
                !endpoint_discovery_authorized(&resolution, "xiaoge").unwrap(),
                "active={active}, project={project}"
            );
        }
    }
}

#[test]
fn endpoint_authority_uses_fieldwise_project_local_precedence_and_semantic_values() {
    let fixture = Fixture::new();
    fixture.write_user(&format!("{READY}\n{DISCOVERY_PROVIDER}"));
    fixture.write_project("[providers.xiaoge]\ncredential = \"env:PROJECT_ACCOUNT\"\n[providers.xiaoge.headers]\nX-Tenant = \"project-tenant\"\n");
    std::fs::write(fixture.project.path().join(".smith/config.local.toml"), "[providers.xiaoge]\ncredential = \"env:USER_ACCOUNT\"\nbase_url = \"https://USER.example/v1/\"\n[providers.xiaoge.headers]\nX-Tenant = \"user-tenant\"\n").unwrap();
    let resolution = resolve(&fixture.request()).unwrap();
    assert!(
        endpoint_discovery_authorized(&resolution, "xiaoge").unwrap(),
        "equivalent values may come from another layer"
    );
    assert_eq!(
        resolution
            .provenance
            .source("providers.xiaoge.credential")
            .unwrap()
            .layer,
        Layer::ProjectLocalFile
    );
    assert_eq!(
        resolution
            .provenance
            .source("providers.xiaoge.headers.X-Region")
            .unwrap()
            .layer,
        Layer::UserFile
    );
}

#[test]
fn endpoint_authority_checks_ordered_pools_and_typed_active_environment_winners() {
    let fixture = Fixture::new();
    fixture.write_user(&format!(
        "{READY}\n{}",
        DISCOVERY_PROVIDER.replace(
            "credential = \"env:USER_ACCOUNT\"",
            "credentials = [\"env:USER_ACCOUNT\", \"env:OTHER_ACCOUNT\"]"
        )
    ));
    fixture.write_project(
        "[providers.xiaoge]\ncredentials = [\"env:OTHER_ACCOUNT\", \"env:USER_ACCOUNT\"]\n",
    );
    let resolution = resolve(&fixture.request()).unwrap();
    assert!(!endpoint_discovery_authorized(&resolution, "xiaoge").unwrap());
    fixture.write_project("");
    let mut resolution = resolve(&fixture.request().with_cli(Overrides {
        provider: Some("xiaoge".to_owned()),
        model: Some("cached-model".to_owned()),
        ..Overrides::default()
    }))
    .unwrap();
    assert!(endpoint_discovery_authorized(&resolution, "xiaoge").unwrap());
    resolution.config.provider.credentials[0] = Sourced::new(
        "env:SERVING_ACCOUNT".to_owned(),
        Source::environment("SMITH_EFFECTIVE_CREDENTIAL"),
    );
    assert!(!endpoint_discovery_authorized(&resolution, "xiaoge").unwrap());
}
