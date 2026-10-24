use super::*;

const CONFIG: &str = r#"
default_profile = "work"
[providers.first]
kind = "fake"
[providers.second]
kind = "fake"
[profiles.work]
provider = "first"
model = "primary"
posture = "plan"
instructions = "Keep the same task and permissions."
max_output_tokens = 1024
[profiles.work.reasoning]
enabled = true
[profiles.work.fallback]
first_response_timeout_ms = 30000
models = [
  { provider = "second", model = "backup", first_response_timeout_ms = 45000, max_output_tokens = 512, reasoning = { enabled = false } },
  { provider = "first", model = "another-model" },
]
"#;

#[test]
fn fallback_timeout_is_validated_without_an_alternative_list() {
    let invalid =
        format!("{BASE_PROJECT_CONFIG}\n[profiles.work.fallback]\nfirst_response_timeout_ms = 0\n");
    assert!(
        resolve_project(&invalid)
            .unwrap_err()
            .to_string()
            .contains("positive")
    );
    let inactive = resolve_project(&invalid.replace(" = 0", " = 1000")).unwrap();
    assert!(inactive.config.fallback.models.is_empty());
    assert_eq!(
        inactive
            .provenance
            .explain("fallback.active")
            .unwrap()
            .value,
        SettingValue::Flag(false)
    );
}

#[test]
fn fallback_resolves_order_controls_policy_and_provenance() {
    let result = resolve_project(CONFIG).unwrap();
    let fallback = &result.config.fallback;
    assert_eq!(fallback.models.len(), 2);
    assert_eq!(
        fallback.first_response_timeout_ms.as_ref().unwrap().value,
        30000
    );
    let backup = &fallback.models[0];
    assert_eq!(backup.config.provider.name.value, "second");
    assert_eq!(backup.config.model.value, "backup");
    assert_eq!(
        backup.first_response_timeout_ms.as_ref().unwrap().value,
        45000
    );
    assert_eq!(backup.config.max_output_tokens.as_ref().unwrap().value, 512);
    assert!(!backup.config.reasoning.enabled.as_ref().unwrap().value);
    assert_eq!(backup.config.agent.active_posture(), AgentPosture::Plan);
    assert_eq!(
        backup.config.agent.profile.instructions,
        result.config.agent.profile.instructions
    );
    assert!(backup.config.fallback.models.is_empty());
    assert!(backup.source.key.ends_with("fallback.models[0]"));
    assert_eq!(
        fallback.models[1]
            .first_response_timeout_ms
            .as_ref()
            .unwrap()
            .value,
        30000
    );
    assert!(
        result
            .provenance
            .explain("fallback.context_disclosure")
            .unwrap()
            .value
            .to_string()
            .contains("conversation context")
    );
}

#[test]
fn fallback_inherits_and_empty_list_disables_timeout() {
    let fixture = Fixture::new();
    fixture.write_project(&format!("{CONFIG}\n[profiles.plan]\nextends = \"work\"\n"));
    let inherited = resolve(&fixture.request().with_cli(Overrides {
        profile: Some("plan".into()),
        ..Overrides::default()
    }))
    .unwrap();
    assert_eq!(inherited.config.fallback.models.len(), 2);
    fixture.write_project_local("[profiles.plan.fallback]\nmodels = []\n");
    let disabled = resolve(&fixture.request().with_cli(Overrides {
        profile: Some("plan".into()),
        ..Overrides::default()
    }))
    .unwrap();
    assert!(disabled.config.fallback.models.is_empty());
    assert!(disabled.config.fallback.first_response_timeout_ms.is_none());
    let explain = disabled.provenance.explain("fallback.models").unwrap();
    assert_eq!(explain.source.key, "profiles.plan.fallback.models");
    assert!(!explain.overridden.is_empty());
}

#[test]
fn fallback_list_replaces_across_file_layers() {
    let fixture = Fixture::new();
    fixture.write_project(CONFIG);
    fixture.write_project_local(
        "[profiles.work.fallback]\nmodels = [{ provider = \"second\", model = \"replacement\" }]\n",
    );
    let result = resolve(&fixture.request()).unwrap();
    assert_eq!(result.config.fallback.models.len(), 1);
    assert_eq!(
        result.config.fallback.models[0].config.model.value,
        "replacement"
    );
    assert_eq!(
        result.config.fallback.models[0]
            .first_response_timeout_ms
            .as_ref()
            .unwrap()
            .value,
        30000
    );
}

#[test]
fn explicit_candidate_reasoning_replaces_incompatible_profile_controls() {
    let config = CONFIG.replace(
        "[profiles.work.reasoning]\nenabled = true",
        "[profiles.work.reasoning]\nenabled = true\neffort = \"high\"",
    );
    let result = resolve_project(&config).unwrap();
    assert!(
        result.config.fallback.models[0]
            .config
            .reasoning
            .effort
            .is_none()
    );
    assert_eq!(
        result.config.fallback.models[1]
            .config
            .reasoning
            .effort
            .as_ref()
            .unwrap()
            .value,
        "high"
    );
    let defaults =
        resolve_project(&config.replace("reasoning = { enabled = false }", "reasoning = {}"))
            .unwrap();
    assert!(
        defaults.config.fallback.models[0]
            .config
            .reasoning
            .enabled
            .is_none()
    );
    assert!(
        defaults.config.fallback.models[0]
            .config
            .reasoning
            .effort
            .is_none()
    );
    let fixture = Fixture::new();
    fixture.write_project(&config);
    let invocation = resolve(&fixture.request().with_cli(Overrides {
        reasoning_effort: Some("max".into()),
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(
        invocation.config.fallback.models[0]
            .config
            .reasoning
            .effort
            .as_ref()
            .unwrap()
            .value,
        "max"
    );
}

#[test]
fn exact_provider_or_model_selection_disables_fallback() {
    let fixture = Fixture::new();
    fixture.write_project(CONFIG);
    for overrides in [
        Overrides {
            model: Some("manual".into()),
            ..Overrides::default()
        },
        Overrides {
            provider: Some("second".into()),
            ..Overrides::default()
        },
    ] {
        let result = resolve(&fixture.request().with_cli(overrides)).unwrap();
        assert!(result.config.fallback.models.is_empty());
        assert!(
            result
                .config
                .fallback
                .inactive_reason
                .as_ref()
                .unwrap()
                .contains("explicit provider/model")
        );
    }
}

#[test]
fn invocation_controls_outrank_candidate_controls() {
    let fixture = Fixture::new();
    fixture.write_project(CONFIG);
    let result = resolve(&fixture.request().with_cli(Overrides {
        max_output_tokens: Some(128),
        reasoning_enabled: Some(true),
        context_window: Some("invocation-window".into()),
        ..Overrides::default()
    }))
    .unwrap();
    let candidate = &result.config.fallback.models[0].config;
    assert_eq!(candidate.max_output_tokens.as_ref().unwrap().value, 128);
    assert!(candidate.reasoning.enabled.as_ref().unwrap().value);
    assert_eq!(
        candidate.context_window.as_ref().unwrap().value,
        "invocation-window"
    );
}

#[test]
fn fallback_rejects_invalid_bindings_and_timeouts_with_entry_sources() {
    for (from, to, expected) in [
        (
            "model = \"backup\"",
            "model = \"cli/codex/example\"",
            "provider-backed",
        ),
        (
            "provider = \"second\", model = \"backup\"",
            "provider = \"missing\", model = \"backup\"",
            "not declared",
        ),
        (
            "provider = \"second\", model = \"backup\"",
            "provider = \"first\", model = \"primary\"",
            "duplicate",
        ),
        (
            "first_response_timeout_ms = 30000",
            "first_response_timeout_ms = 0",
            "positive",
        ),
        (
            "first_response_timeout_ms = 45000",
            "first_response_timeout_ms = 0",
            "positive",
        ),
    ] {
        let error = resolve_project(&CONFIG.replace(from, to))
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
        assert!(error.contains("fallback"), "{error}");
    }
}
