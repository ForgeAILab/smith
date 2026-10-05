use super::*;

#[test]
fn project_layers_cannot_redirect_the_trusted_chatgpt_endpoint_or_storage() {
    let fixture = Fixture::new();
    for (base_url, credential) in [
        ("https://proxy.example/codex", "keychain:smith/chatgpt"),
        (
            "https://chatgpt.com/backend-api/codex",
            "env:CHATGPT_ACCESS_TOKEN",
        ),
    ] {
        fixture.write_project(&format!(
            r#"
default_profile = "chatgpt"
[profiles.chatgpt]
provider = "chatgpt"
model = "gpt-5.6-terra"
[providers.chatgpt]
kind = "chatgpt-responses"
base_url = "{base_url}"
credential = "{credential}"
[models."chatgpt/gpt-5.6-terra"]
context_tokens = 272000
max_input_tokens = 255616
max_output_tokens = 16384
"#,
        ));
        let error = resolve(&fixture.request()).expect_err("trusted constants cannot be changed");
        assert!(
            error.to_string().contains("fixed trusted endpoint")
                || error.to_string().contains("requires Smith OAuth")
        );
    }
}

/// Multi-account ChatGPT: a `credentials` pool of Smith-owned auth-file
/// entries resolves. `/connect` writes `chatgpt` and numbered additions, but a
/// hand-renamed `chatgpt-<label>` entry keeps working too.
#[test]
fn a_chatgpt_pool_of_smith_authfile_entries_resolves() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "chatgpt"
[profiles.chatgpt]
provider = "chatgpt"
model = "gpt-5.6-terra"
[providers.chatgpt]
kind = "chatgpt-responses"
base_url = "https://chatgpt.com/backend-api/codex"
credentials = ["authfile:chatgpt", "authfile:chatgpt-2", "authfile:chatgpt-work"]
[models."chatgpt/gpt-5.6-terra"]
context_tokens = 272000
max_input_tokens = 255616
max_output_tokens = 16384
"#,
    );
    resolve(&fixture.request()).expect("a pooled ChatGPT provider resolves");
}

/// Every pool member must be a Smith-owned `authfile:chatgpt*` entry: a pool
/// is not a side door for references some other product owns and rotates.
#[test]
fn a_chatgpt_pool_member_outside_smith_entries_is_rejected() {
    let fixture = Fixture::new();
    for member in [
        "authfile:codex",
        "keychain:smith/chatgpt-2",
        "env:CHATGPT_TOKEN_2",
    ] {
        fixture.write_user(&format!(
            r#"
default_profile = "chatgpt"
[profiles.chatgpt]
provider = "chatgpt"
model = "gpt-5.6-terra"
[providers.chatgpt]
kind = "chatgpt-responses"
base_url = "https://chatgpt.com/backend-api/codex"
credentials = ["authfile:chatgpt", "{member}"]
[models."chatgpt/gpt-5.6-terra"]
context_tokens = 272000
max_input_tokens = 255616
max_output_tokens = 16384
"#,
        ));
        let error = resolve(&fixture.request()).expect_err("a foreign pool member must fail");
        assert!(
            error.to_string().contains("requires Smith OAuth"),
            "{member}: {error}"
        );
    }
}

/// Multi-account xAI: each pool member is a stored login of its own.
#[test]
fn an_xai_pool_of_stored_logins_resolves() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "grok"
[profiles.grok]
provider = "xai"
model = "grok-4.3"
[providers.xai]
kind = "xai-responses"
base_url = "https://api.x.ai/v1"
credentials = ["authfile:xai", "authfile:xai-2"]
[models."xai/grok-4.3"]
context_tokens = 1000000
max_input_tokens = 970000
max_output_tokens = 30000
"#,
    );
    resolve(&fixture.request()).expect("a pooled xAI provider resolves");
}

/// A connected xAI login must leave a model behind, or the provider reads as
/// configured while every picker shows nothing to select — which is what
/// `/connect xai` produced before it declared one.
#[test]
fn a_bare_xai_model_entry_becomes_selectable_from_the_catalog_alone() {
    let fixture = Fixture::new();
    fixture.write_user(&format!(
        r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[providers.{provider}]
kind = "{kind}"
base_url = "{endpoint}"
credential = "{credential}"

# Exactly what `/connect xai` writes: a name, and no limits of its own.
[models."{provider}/{model}"]
"#,
        provider = smith_config::setup::XAI_PROVIDER,
        kind = smith_config::model::KIND_XAI_RESPONSES,
        endpoint = smith_config::setup::XAI_ENDPOINT,
        credential = smith_config::setup::XAI_CREDENTIAL,
        model = smith_config::setup::XAI_DEFAULT_MODEL,
    ));
    let resolution = resolve(&fixture.request()).expect("a resolvable xAI connection");
    let snapshot = xai_catalog_snapshot();

    let inventory = local_inventory_with_catalog(
        &resolution,
        &["fake", smith_config::model::KIND_XAI_RESPONSES],
        Some(&snapshot),
    )
    .expect("catalog inventory");

    let grok = inventory
        .models
        .iter()
        .find(|model| model.provider == smith_config::setup::XAI_PROVIDER)
        .expect("the connected provider contributes a model");
    assert_eq!(grok.model, smith_config::setup::XAI_DEFAULT_MODEL);
    assert!(
        grok.selectable,
        "a listed model a user cannot pick is noise"
    );
    // The limits come from the catalog rather than the config, which is why
    // the written entry can be empty in the first place.
    assert!(matches!(
        grok.context_tokens
            .as_ref()
            .expect("a context window")
            .origin,
        ModelLimitOrigin::Catalog { .. }
    ));
    let provider = inventory
        .providers
        .iter()
        .find(|entry| entry.name == smith_config::setup::XAI_PROVIDER)
        .expect("the connected provider is listed");
    assert!(provider.selectable);
    assert_eq!(provider.model_count, 1);
}

fn xai_catalog_snapshot() -> CatalogSnapshot {
    CatalogSnapshot {
        schema_revision: CATALOG_SCHEMA_REVISION,
        source_url: MODELS_DEV_SOURCE_URL.to_owned(),
        source_digest: format!("sha256:{}", "1".repeat(64)),
        content_digest: format!("sha256:{}", "2".repeat(64)),
        source_revision: "fixture-r1".to_owned(),
        retrieved_at_ms: 1_000,
        providers: BTreeMap::from([(
            smith_config::catalog::XAI_CATALOG_PROVIDER.to_owned(),
            CatalogProvider {
                id: smith_config::catalog::XAI_CATALOG_PROVIDER.to_owned(),
                name: "xAI".to_owned(),
                models: BTreeMap::from([(
                    smith_config::setup::XAI_DEFAULT_MODEL.to_owned(),
                    CatalogModel {
                        id: smith_config::setup::XAI_DEFAULT_MODEL.to_owned(),
                        // The real catalog shape for this model: a large
                        // context against a much smaller output cap.
                        name: "Grok 4.3".to_owned(),
                        limits: Some(CatalogLimits {
                            context_tokens: 1_000_000,
                            max_input_tokens: 1_000_000,
                            max_output_tokens: 30_000,
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
        )]),
    }
}

/// A provider ceiling is not Smith's ordinary request size. Models.dev may
/// advertise the whole context window as the output ceiling, and the automatic
/// request policy must leave that model useful without a local limit guess.
#[test]
fn an_xai_model_whose_output_ceiling_equals_context_gets_an_automatic_budget() {
    let fixture = Fixture::new();
    fixture.write_user(&format!(
        r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[providers.{provider}]
kind = "{kind}"
base_url = "{endpoint}"
credential = "{credential}"

[models."{provider}/output-equals-context"]
"#,
        provider = smith_config::setup::XAI_PROVIDER,
        kind = smith_config::model::KIND_XAI_RESPONSES,
        endpoint = smith_config::setup::XAI_ENDPOINT,
        credential = smith_config::setup::XAI_CREDENTIAL,
    ));
    let resolution = resolve(&fixture.request()).expect("a resolvable xAI connection");
    let mut snapshot = xai_catalog_snapshot();
    let provider = snapshot
        .providers
        .get_mut(smith_config::catalog::XAI_CATALOG_PROVIDER)
        .expect("the fixture provider");
    let mut squeezed = provider
        .models
        .get(smith_config::setup::XAI_DEFAULT_MODEL)
        .expect("the default model")
        .clone();
    squeezed.id = "output-equals-context".to_owned();
    squeezed.limits = Some(CatalogLimits {
        context_tokens: 500_000,
        max_input_tokens: 500_000,
        max_output_tokens: 500_000,
    });
    provider
        .models
        .insert("output-equals-context".to_owned(), squeezed);

    let inventory = local_inventory_with_catalog(
        &resolution,
        &["fake", smith_config::model::KIND_XAI_RESPONSES],
        Some(&snapshot),
    )
    .expect("catalog inventory");

    let squeezed = inventory
        .models
        .iter()
        .find(|model| model.model == "output-equals-context")
        .expect("the model is listed");
    assert!(squeezed.selectable);
    assert_eq!(squeezed.max_output_tokens.as_ref().unwrap().value, 500_000);
    let budget = squeezed.output_budget.expect("an automatic output budget");
    assert_eq!(budget.request_tokens, 32_768);
    assert_eq!(budget.output_reserve, 32_768);
    assert_eq!(budget.request_origin, OutputBudgetOrigin::Automatic);
    assert_eq!(budget.reserve_origin, OutputBudgetOrigin::Automatic);
}
