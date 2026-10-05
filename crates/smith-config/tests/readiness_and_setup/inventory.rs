use super::*;

#[test]
fn local_inventory_keeps_models_provider_qualified_and_deterministic() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "zai"

[profiles.zai]
provider = "zai"
model = "shared"

[profiles.router]
provider = "openrouter"
model = "shared"

[providers.zai]
kind = "openai-compatible"
base_url = "https://zai.example/v1"
credential = "env:ZAI_API_KEY"

[providers.openrouter]
kind = "openai-compatible"
base_url = "https://router.example/v1"
credential = "env:OPENROUTER_API_KEY"

[models."zai/shared"]
context_tokens = 200
max_input_tokens = 180
max_output_tokens = 20

[models."openrouter/shared"]
context_tokens = 100
max_input_tokens = 90
max_output_tokens = 10

[models."openrouter/incomplete"]
context_tokens = 100
"#,
    );
    let resolution = resolve(&fixture.request()).expect("ready config");
    let inventory = local_inventory(&resolution, &["openai-compatible"]).expect("local inventory");
    assert_eq!(
        inventory
            .models
            .iter()
            .map(|entry| entry.id())
            .collect::<Vec<_>>(),
        ["openrouter/shared", "zai/shared"]
    );
    assert!(
        inventory
            .models
            .iter()
            .find(|entry| entry.id() == "zai/shared")
            .is_some_and(|entry| entry.active)
    );
    assert_eq!(
        inventory
            .providers
            .iter()
            .map(|entry| (&entry.name, entry.model_count))
            .collect::<Vec<_>>(),
        [(&"openrouter".to_owned(), 1), (&"zai".to_owned(), 1)]
    );
    assert!(matches!(
        inventory.resolve_model("shared", None),
        Err(ModelSelectionError::Ambiguous { choices, .. })
            if choices == ["openrouter/shared", "zai/shared"]
    ));
    assert_eq!(
        inventory
            .resolve_model("shared", Some("zai"))
            .expect("active-provider match")
            .id(),
        "zai/shared"
    );
}

#[test]
fn a_valid_provider_without_models_is_available_for_add_model_but_not_runtime_selection() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[providers.empty]
kind = "openai-compatible"
base_url = "https://empty.example/v1"
credential = "env:EMPTY_API_KEY"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#,
    );
    let resolution = resolve(&fixture.request()).expect("ready config");
    let inventory =
        local_inventory(&resolution, &["fake", "openai-compatible"]).expect("local inventory");
    let provider = inventory
        .providers
        .iter()
        .find(|entry| entry.name == "empty")
        .expect("empty provider");
    assert!(provider.adapter_available);
    assert!(!provider.selectable);
    assert_eq!(provider.model_count, 0);
}

#[test]
fn trusted_glm_is_selectable_without_copying_limits_into_user_toml() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "glm"
[profiles.glm]
provider = "zai"
model = "glm-4.7"
max_output_tokens = 8192
[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "env:ZAI_API_KEY"
[context]
output_reserve = 8192
"#,
    );
    let resolution = resolve(&fixture.request()).expect("ready selection");
    let inventory =
        local_inventory(&resolution, &["openai-compatible"]).expect("trusted inventory");
    let [glm] = inventory.models.as_slice() else {
        panic!("expected one trusted GLM model: {:?}", inventory.models);
    };
    assert_eq!(glm.id(), "zai/glm-4.7");
    assert_eq!(glm.context_tokens.as_ref().unwrap().value, 200_000);
    assert_eq!(glm.max_input_tokens.as_ref().unwrap().value, 196_000);
    assert_eq!(glm.max_output_tokens.as_ref().unwrap().value, 131_072);
}

#[test]
fn inventory_filters_adapters_this_runtime_does_not_ship() {
    let fixture = Fixture::new();
    fixture.write_user(READY);
    let resolution = resolve(&fixture.request()).expect("ready config");
    let inventory = local_inventory(&resolution, &[]).expect("inventory");
    assert!(inventory.models.is_empty());
    assert_eq!(inventory.providers.len(), 1);
    assert!(!inventory.providers[0].selectable);
    assert!(!inventory.profiles[0].selectable);
}

#[test]
fn exact_openrouter_binding_augments_inventory_and_keeps_incompatible_models_visible() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "router"
[profiles.router]
provider = "router"
model = "local"
[providers.router]
kind = "openai-compatible"
base_url = "https://OPENROUTER.ai/api/v1/"
credential = "env:OPENROUTER_API_KEY"
[models."router/local"]
context_tokens = 64000
max_input_tokens = 60000
max_output_tokens = 4000
[context]
output_reserve = 4000
"#,
    );
    let resolution = resolve(&fixture.request()).expect("ready OpenRouter selection");
    let snapshot = catalog_snapshot();
    let inventory =
        local_inventory_with_catalog(&resolution, &["openai-compatible"], Some(&snapshot))
            .expect("catalog inventory");

    assert_eq!(
        inventory
            .models
            .iter()
            .map(|model| model.id())
            .collect::<Vec<_>>(),
        [
            "router/invalid-limits",
            "router/local",
            "router/no-tools",
            "router/vendor/model"
        ]
    );
    assert_eq!(
        inventory.providers[0].model_count, 2,
        "only local and the compatible catalog model count"
    );
    let nested = inventory
        .models
        .iter()
        .find(|model| model.model == "vendor/model")
        .unwrap();
    assert!(nested.selectable);
    assert_eq!(nested.label, "Vendor Model");
    assert_eq!(nested.catalog_provider.as_deref(), Some("openrouter"));
    assert!(matches!(
        nested.context_tokens.as_ref().unwrap().origin,
        ModelLimitOrigin::Catalog { .. }
    ));
    assert!(
        inventory
            .models
            .iter()
            .find(|model| model.model == "no-tools")
            .is_some_and(|model| {
                !model.selectable
                    && model
                        .disabled_reason
                        .as_deref()
                        .is_some_and(|reason| reason.contains("tool"))
            })
    );
    assert!(
        inventory
            .models
            .iter()
            .find(|model| model.model == "invalid-limits")
            .is_some_and(|model| {
                !model.selectable
                    && model
                        .disabled_reason
                        .as_deref()
                        .is_some_and(|reason| reason.contains("output limit"))
            })
    );
    assert_eq!(
        inventory
            .resolve_model("router/vendor/model", None)
            .unwrap()
            .model,
        "vendor/model"
    );
    assert!(inventory.resolve_model("router/no-tools", None).is_err());
}

#[test]
fn explicit_limit_fields_win_over_catalog_fields_independently() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "router"
[profiles.router]
provider = "router"
model = "vendor/model"
[providers.router]
kind = "openai-compatible"
base_url = "https://openrouter.ai/api/v1"
credential = "env:OPENROUTER_API_KEY"
[models."router/vendor/model"]
context_tokens = 256000
[context]
output_reserve = 4000
"#,
    );
    let resolution = resolve(&fixture.request()).expect("ready mixed-limit selection");
    let snapshot = catalog_snapshot();
    let inventory =
        local_inventory_with_catalog(&resolution, &["openai-compatible"], Some(&snapshot))
            .expect("catalog inventory");
    let model = inventory
        .models
        .iter()
        .find(|model| model.model == "vendor/model")
        .unwrap();

    assert_eq!(model.context_tokens.as_ref().unwrap().value, 256_000);
    assert!(matches!(
        model.context_tokens.as_ref().unwrap().origin,
        ModelLimitOrigin::Configured(_)
    ));
    assert_eq!(model.max_input_tokens.as_ref().unwrap().value, 100_000);
    assert!(matches!(
        model.max_input_tokens.as_ref().unwrap().origin,
        ModelLimitOrigin::Catalog { .. }
    ));
    assert_eq!(model.max_output_tokens.as_ref().unwrap().value, 16_000);
}

#[test]
fn familiar_provider_name_at_an_unbound_endpoint_gets_no_catalog_models() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "router"
[profiles.router]
provider = "openrouter"
model = "local"
[providers.openrouter]
kind = "openai-compatible"
base_url = "https://proxy.example/v1"
credential = "env:OPENROUTER_API_KEY"
[models."openrouter/local"]
context_tokens = 64000
max_input_tokens = 60000
max_output_tokens = 4000
"#,
    );
    let resolution = resolve(&fixture.request()).expect("ready custom endpoint");
    let snapshot = catalog_snapshot();
    let inventory =
        local_inventory_with_catalog(&resolution, &["openai-compatible"], Some(&snapshot))
            .expect("catalog inventory");

    assert_eq!(
        inventory
            .models
            .iter()
            .map(|model| model.id())
            .collect::<Vec<_>>(),
        ["openrouter/local"]
    );
}

#[test]
fn effective_reserves_keep_a_catalog_model_visible_but_disabled() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "router"
[profiles.router]
provider = "router"
model = "local"
[providers.router]
kind = "openai-compatible"
base_url = "https://openrouter.ai/api/v1"
credential = "env:OPENROUTER_API_KEY"
[models."router/local"]
context_tokens = 200000
max_input_tokens = 190000
max_output_tokens = 4000
[context]
output_reserve = 128000
"#,
    );
    let resolution = resolve(&fixture.request()).expect("ready local selection");
    let snapshot = catalog_snapshot();
    let inventory =
        local_inventory_with_catalog(&resolution, &["openai-compatible"], Some(&snapshot))
            .expect("catalog inventory");
    let model = inventory
        .models
        .iter()
        .find(|model| model.model == "vendor/model")
        .unwrap();

    assert!(!model.selectable);
    assert!(
        model
            .disabled_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("leaves no input budget"))
    );
}
