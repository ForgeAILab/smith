use super::*;

#[test]
fn named_windows_resolve_the_default_and_explain_overrides() {
    let fixture = Fixture::new();
    fixture.write_project(
        r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
max_output_tokens = 4096
default_context_window = "large"

[models."local/example-model".context_windows."small"]
context_tokens = 32768

[models."local/example-model".context_windows."large"]
context_tokens = 131072
"#,
    );

    let resolution = resolve(&fixture.request()).expect("named model windows resolve");
    assert_eq!(
        resolution
            .config
            .context_window
            .as_ref()
            .map(|window| window.value.as_str()),
        Some("large")
    );
    assert_eq!(resolution.config.model_limits.context_windows.len(), 2);
    let explained = resolution
        .provenance
        .explain("context_window")
        .expect("the selected model default is explainable");
    assert_eq!(
        explained.value,
        smith_config::resolve::SettingValue::Text("large".into())
    );
    assert!(explained.source.key.ends_with("default_context_window"));

    let session = resolve(
        &fixture
            .request()
            .with_cli(Overrides {
                context_window: Some("small".into()),
                ..Overrides::default()
            })
            .with_session(Overrides {
                context_window: Some("large".into()),
                ..Overrides::default()
            }),
    )
    .expect("higher-precedence session window");
    let selected = session.config.context_window.expect("selected window");
    assert_eq!(selected.value, "large");
    assert_eq!(selected.source.layer, Layer::SessionOverride);
    let explained = session
        .provenance
        .explain("context_window")
        .expect("the active window is explainable");
    assert_eq!(explained.overridden.len(), 2);
}

#[test]
fn same_layer_flat_limits_and_named_windows_are_ambiguous() {
    let fixture = Fixture::new();
    fixture.write_project(
        r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 65536
max_output_tokens = 4096
default_context_window = "small"

[models."local/example-model".context_windows."small"]
context_tokens = 32768
"#,
    );

    let error = resolve(&fixture.request()).expect_err("the two shapes are ambiguous");
    assert!(matches!(error, ConfigError::Ambiguous { .. }), "{error}");
}

#[test]
fn exact_openai_endpoint_exposes_the_two_reviewed_gpt_windows() {
    let fixture = Fixture::new();
    fixture.write_project(
        r#"
default_profile = "dev"

[profiles.dev]
provider = "openai"
model = "gpt-5.6-terra"

[providers.openai]
kind = "openai-compatible"
base_url = "https://api.openai.com/v1"
credential = "env:OPENAI_API_KEY"
"#,
    );

    let resolution = resolve(&fixture.request()).expect("exact OpenAI endpoint resolves");
    assert_eq!(
        resolution
            .config
            .context_window
            .as_ref()
            .map(|window| window.value.as_str()),
        Some("1m")
    );
    let mut snapshot = catalog_snapshot();
    snapshot.providers.insert(
        "openai".to_owned(),
        CatalogProvider {
            id: "openai".to_owned(),
            name: "OpenAI".to_owned(),
            models: BTreeMap::from([(
                "gpt-5.6-terra".to_owned(),
                CatalogModel {
                    id: "gpt-5.6-terra".to_owned(),
                    name: "GPT-5.6 Terra".to_owned(),
                    limits: Some(CatalogLimits {
                        context_tokens: 1_050_000,
                        max_input_tokens: 1_034_000,
                        max_output_tokens: 16_000,
                    }),
                    input_modalities: vec![CatalogModality::Text],
                    output_modalities: vec![CatalogModality::Text],
                    tool_call: true,
                    reasoning: true,
                    reasoning_controls: None,
                    structured_output: true,
                    cost: None,
                    disabled_reason: None,
                },
            )]),
        },
    );
    let inventory =
        local_inventory_with_catalog(&resolution, &["openai-compatible"], Some(&snapshot))
            .expect("credential-free local inventory");
    let model = inventory
        .models
        .iter()
        .find(|entry| entry.provider == "openai" && entry.model == "gpt-5.6-terra")
        .expect("profile model is listed");
    assert_eq!(model.context_windows, ["1m", "272k"]);
}

/// Two models configured only with named windows: the active one leaves its
/// input limit to the runtime, and the other is not active at all.
const WINDOWS_ONLY: &str = r#"
default_profile = "wide"

[profiles.wide]
provider = "local"
model = "window-model"

[providers.local]
kind = "fake"

[models."local/window-model"]
default_context_window = "128k"
max_output_tokens = 8192

[models."local/window-model".context_windows."128k"]
context_tokens = 128000

[models."local/window-model".context_windows."1m"]
context_tokens = 1000000

[models."local/other-window-model"]
default_context_window = "272k"
max_output_tokens = 8192

[models."local/other-window-model".context_windows."272k"]
context_tokens = 272000
max_input_tokens = 264000

[models."local/other-window-model".context_windows."872k"]
context_tokens = 872000
max_input_tokens = 864000
"#;

#[test]
fn windows_only_models_are_listed_with_the_limits_the_runtime_applies() {
    let fixture = Fixture::new();
    fixture.write_project(WINDOWS_ONLY);
    let resolution = resolve(&fixture.request()).expect("windows-only models resolve");
    let inventory = local_inventory(&resolution, &["fake"]).expect("local inventory");

    let active = inventory
        .models
        .iter()
        .find(|entry| entry.id() == "local/window-model")
        .expect("the active windows-only model is listed");
    assert!(active.active);
    assert!(active.selectable, "{:?}", active.disabled_reason);
    assert_eq!(active.context_windows, ["128k", "1m"]);
    assert_eq!(
        active.context_tokens.as_ref().map(|limit| limit.value),
        Some(128_000)
    );
    // The runtime derives an absent window input limit as the window's
    // context minus the model's output ceiling.
    assert_eq!(
        active.max_input_tokens.as_ref().map(|limit| limit.value),
        Some(128_000 - 8_192)
    );
    let input_origin = &active
        .max_input_tokens
        .as_ref()
        .expect("an input limit")
        .origin;
    assert!(
        matches!(input_origin, ModelLimitOrigin::Configured(source)
            if source.key.ends_with("context_windows.128k.context_tokens")),
        "{input_origin:?}"
    );

    let inactive = inventory
        .models
        .iter()
        .find(|entry| entry.id() == "local/other-window-model")
        .expect("an inactive windows-only model is listed");
    assert!(!inactive.active);
    assert!(inactive.selectable, "{:?}", inactive.disabled_reason);
    assert_eq!(inactive.context_windows, ["272k", "872k"]);
    assert_eq!(
        inactive.context_tokens.as_ref().map(|limit| limit.value),
        Some(272_000)
    );
    assert_eq!(
        inactive.max_input_tokens.as_ref().map(|limit| limit.value),
        Some(264_000)
    );
    assert_eq!(inventory.providers[0].model_count, 2);
}
