//! First-run readiness and setup-data contracts.

use std::collections::BTreeMap;
use std::fs;

use smith_config::catalog::{
    CATALOG_SCHEMA_REVISION, CatalogLimits, CatalogModality, CatalogModel, CatalogProvider,
    CatalogSnapshot, MODELS_DEV_SOURCE_URL,
};
use smith_config::inventory::{
    ModelLimitOrigin, ModelSelectionError, local_inventory, local_inventory_with_catalog,
};
use smith_config::model::{
    ConfigFile, ConfigSecret, ContextSection, ModelSection, ProfileSection, ProviderSection,
    ReasoningOnlyBehavior,
};
use smith_config::resolve::{
    ConfigError, ConfigReadiness, Layer, Overrides, ResolveRequest, inspect, resolve,
};
use smith_config::user_config::{UserConfigEditError, prepare_user_config_edit};

const READY: &str = r#"
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
"#;

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().expect("a home"),
            project: tempfile::tempdir().expect("a project"),
        }
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path()).with_home_dir(self.home.path())
    }

    fn write_user(&self, text: &str) {
        let directory = self.home.path().join(".smith");
        fs::create_dir_all(&directory).expect("a user config directory");
        fs::write(directory.join("config.toml"), text).expect("a user config");
    }
}

fn glm_patch(credential: &str) -> ConfigFile {
    ConfigFile {
        default_profile: Some("glm".into()),
        profiles: BTreeMap::from([(
            "glm".into(),
            ProfileSection {
                provider: Some("zai".into()),
                model: Some("glm-4.7".into()),
                max_output_tokens: Some(8_192),
                ..ProfileSection::default()
            },
        )]),
        providers: BTreeMap::from([(
            "zai".into(),
            ProviderSection {
                kind: Some("openai-compatible".into()),
                base_url: Some("https://api.z.ai/api/coding/paas/v4".into()),
                credential: Some(credential.into()),
                ..ProviderSection::default()
            },
        )]),
        models: BTreeMap::from([(
            "zai/glm-4.7".into(),
            ModelSection {
                context_tokens: Some(200_000),
                max_input_tokens: Some(196_000),
                max_output_tokens: Some(131_072),
            },
        )]),
        context: Some(ContextSection {
            output_reserve: Some(8_192),
            ..ContextSection::default()
        }),
        ..ConfigFile::default()
    }
}

fn catalog_snapshot() -> CatalogSnapshot {
    let valid = |id: &str, name: &str, context: u32, input: u32, output: u32| CatalogModel {
        id: id.to_owned(),
        name: name.to_owned(),
        limits: Some(CatalogLimits {
            context_tokens: context,
            max_input_tokens: input,
            max_output_tokens: output,
        }),
        input_modalities: vec![CatalogModality::Text],
        output_modalities: vec![CatalogModality::Text],
        tool_call: true,
        reasoning: true,
        structured_output: true,
        disabled_reason: None,
    };
    let mut openrouter_models = BTreeMap::from([
        (
            "vendor/model".to_owned(),
            valid("vendor/model", "Vendor Model", 128_000, 100_000, 16_000),
        ),
        (
            "no-tools".to_owned(),
            CatalogModel {
                tool_call: false,
                ..valid("no-tools", "No Tools", 32_000, 32_000, 4_000)
            },
        ),
    ]);
    openrouter_models.insert(
        "invalid-limits".to_owned(),
        CatalogModel {
            id: "invalid-limits".to_owned(),
            name: "Invalid Limits".to_owned(),
            limits: None,
            input_modalities: vec![CatalogModality::Text],
            output_modalities: vec![CatalogModality::Text],
            tool_call: true,
            reasoning: false,
            structured_output: false,
            disabled_reason: Some("catalog output limit exceeds its context window".to_owned()),
        },
    );
    CatalogSnapshot {
        schema_revision: CATALOG_SCHEMA_REVISION,
        source_url: MODELS_DEV_SOURCE_URL.to_owned(),
        source_digest: format!("sha256:{}", "1".repeat(64)),
        content_digest: format!("sha256:{}", "2".repeat(64)),
        source_revision: "fixture-r1".to_owned(),
        retrieved_at_ms: 1_000,
        providers: BTreeMap::from([
            (
                "openrouter".to_owned(),
                CatalogProvider {
                    id: "openrouter".to_owned(),
                    name: "OpenRouter".to_owned(),
                    models: openrouter_models,
                },
            ),
            (
                "zai-coding-plan".to_owned(),
                CatalogProvider {
                    id: "zai-coding-plan".to_owned(),
                    name: "Z.AI Coding Plan".to_owned(),
                    models: BTreeMap::from([(
                        "glm-next".to_owned(),
                        valid("glm-next", "GLM Next", 200_000, 180_000, 64_000),
                    )]),
                },
            ),
        ]),
    }
}

#[test]
fn no_setup_intent_is_unconfigured_with_discovered_locations() {
    let fixture = Fixture::new();
    let ConfigReadiness::Unconfigured(context) = inspect(&fixture.request()) else {
        panic!("a fresh user should be unconfigured");
    };
    assert_eq!(
        context.layout.user_dir,
        fixture
            .home
            .path()
            .canonicalize()
            .expect("a canonical home")
            .join(".smith")
    );
    assert!(context.layout.project_root.is_none());

    fixture.write_user(
        r#"
        [approval]
        mode = "deny"
        "#,
    );
    assert!(
        matches!(
            inspect(&fixture.request()),
            ConfigReadiness::Unconfigured(_)
        ),
        "policy without provider/model intent remains first-run state"
    );
}

#[test]
fn ready_inspection_is_the_ordinary_resolution() {
    let fixture = Fixture::new();
    fixture.write_user(READY);
    let expected = resolve(&fixture.request()).expect("ordinary resolution");
    assert_eq!(
        inspect(&fixture.request()),
        ConfigReadiness::Ready(Box::new(expected))
    );
}

#[test]
fn partial_or_malformed_setup_intent_is_invalid() {
    let cases = [
        (
            "provider declaration",
            r#"
            [providers.local]
            kind = "fake"
            "#,
        ),
        (
            "selected provider without model",
            r#"
            default_profile = "dev"
            [profiles.dev]
            provider = "local"
            [providers.local]
            kind = "fake"
            "#,
        ),
        (
            "model without complete limits",
            r#"
            default_profile = "dev"
            [profiles.dev]
            provider = "local"
            model = "example-model"
            [providers.local]
            kind = "fake"
            [models."local/example-model"]
            context_tokens = "many"
            "#,
        ),
        ("malformed file", "default_profile = ["),
    ];

    for (name, text) in cases {
        let fixture = Fixture::new();
        fixture.write_user(text);
        assert!(
            matches!(inspect(&fixture.request()), ConfigReadiness::Invalid(_)),
            "{name} must not be rewritten as first-run setup"
        );
    }
}

#[test]
fn environment_cli_and_session_selection_count_as_setup_intent() {
    let fixture = Fixture::new();
    let cases = [
        fixture.request().with_env([("SMITH_PROVIDER", "missing")]),
        fixture.request().with_cli(Overrides {
            model: Some("example-model".into()),
            ..Overrides::default()
        }),
        fixture.request().with_session(Overrides {
            profile: Some("missing".into()),
            ..Overrides::default()
        }),
    ];

    for request in cases {
        assert!(
            matches!(inspect(&request), ConfigReadiness::Invalid(_)),
            "an explicit selection must not trigger automatic setup"
        );
    }
}

#[test]
fn removing_the_only_ready_configuration_derives_unconfigured_again() {
    let fixture = Fixture::new();
    fixture.write_user(READY);
    assert!(matches!(
        inspect(&fixture.request()),
        ConfigReadiness::Ready(_)
    ));

    fs::remove_file(fixture.home.path().join(".smith/config.toml"))
        .expect("the prior setup was removed");
    assert!(matches!(
        inspect(&fixture.request()),
        ConfigReadiness::Unconfigured(_)
    ));
}

#[test]
fn invalid_state_keeps_the_actionable_resolver_error() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
        default_profile = "dev"
        [profiles.dev]
        provider = "missing"
        model = "m"
        "#,
    );
    let ConfigReadiness::Invalid(error) = inspect(&fixture.request()) else {
        panic!("an unknown selected provider is invalid");
    };
    assert!(matches!(
        error,
        ConfigError::UnusableReference { name, .. } if name == "missing"
    ));
}

#[test]
fn user_config_edits_preserve_comments_and_unrelated_tables() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"# keep this explanation
[limits]
max_retries = 7 # and this inline note

[providers.unrelated]
kind = "fake"
"#,
    );
    let prepared = prepare_user_config_edit(
        fixture.home.path().join(".smith"),
        &glm_patch("env:ZAI_API_KEY"),
    )
    .expect("a prepared edit");
    assert!(prepared.collisions().is_empty());
    assert!(prepared.preview().contains("providers.zai.credential"));
    assert!(!prepared.preview().contains("ZAI_API_KEY="));
    prepared.commit(false).expect("an atomic commit").accept();

    let text = fs::read_to_string(fixture.home.path().join(".smith/config.toml"))
        .expect("the merged config");
    assert!(text.contains("# keep this explanation"), "{text}");
    assert!(text.contains("# and this inline note"), "{text}");
    assert!(text.contains("[providers.unrelated]"), "{text}");
    assert!(text.contains("[providers.zai]"), "{text}");
    assert!(text.contains("[models.\"zai/glm-4.7\"]"), "{text}");
}

#[test]
fn differing_existing_values_require_confirmation() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"# owned by the user
default_profile = "existing"

[profiles.existing]
provider = "local"
model = "m"

[providers.local]
kind = "fake"

[models."local/m"]
context_tokens = 100
max_input_tokens = 90
max_output_tokens = 10
"#,
    );
    let path = fixture.home.path().join(".smith/config.toml");
    let before = fs::read(&path).expect("prior bytes");
    let prepared = prepare_user_config_edit(
        fixture.home.path().join(".smith"),
        &glm_patch("env:ZAI_API_KEY"),
    )
    .expect("a prepared edit");
    assert_eq!(prepared.collisions().len(), 1);
    assert_eq!(prepared.collisions()[0].key, "default_profile");
    assert!(matches!(
        prepared.commit(false),
        Err(UserConfigEditError::UnconfirmedCollisions { count: 1 })
    ));
    assert_eq!(fs::read(&path).expect("unchanged bytes"), before);

    prepared
        .commit(true)
        .expect("the reviewed collision was accepted")
        .accept();
    let after = fs::read_to_string(path).expect("the committed config");
    assert!(after.contains("default_profile = \"glm\""), "{after}");
    assert!(after.contains("# owned by the user"), "{after}");
    assert!(after.contains("[profiles.existing]"), "{after}");
}

#[test]
fn committed_edits_can_restore_exact_prior_bytes() {
    let fixture = Fixture::new();
    fixture.write_user("# exact prior bytes\n[approval]\nmode = \"deny\"\n");
    let path = fixture.home.path().join(".smith/config.toml");
    let before = fs::read(&path).expect("prior bytes");
    let committed = prepare_user_config_edit(
        fixture.home.path().join(".smith"),
        &glm_patch("env:ZAI_API_KEY"),
    )
    .expect("a prepared edit")
    .commit(false)
    .expect("a commit");
    assert_ne!(fs::read(&path).expect("candidate bytes"), before);
    committed.rollback().expect("rollback");
    assert_eq!(fs::read(path).expect("restored bytes"), before);
}

#[test]
fn fresh_commit_is_restrictive_and_rollback_removes_only_the_config() {
    let fixture = Fixture::new();
    let directory = fixture.home.path().join(".smith");
    let path = directory.join("config.toml");
    let committed = prepare_user_config_edit(&directory, &glm_patch("env:ZAI_API_KEY"))
        .expect("a prepared fresh edit")
        .commit(false)
        .expect("a fresh commit");
    assert!(path.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            fs::metadata(&directory)
                .expect("directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path)
                .expect("file metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    committed.rollback().expect("fresh rollback");
    assert!(!path.exists());
    assert!(
        directory.is_dir(),
        "rollback must not remove broader user state"
    );
}

#[test]
fn config_edit_errors_and_debug_never_echo_secret_values() {
    let fixture = Fixture::new();
    let secret = "sk-test-do-not-print-0123456789";
    let error = prepare_user_config_edit(fixture.home.path().join(".smith"), &glm_patch(secret))
        .expect_err("a plaintext credential");
    assert!(!format!("{error:?} {error}").contains(secret));

    let mut patch = glm_patch("env:ZAI_API_KEY");
    patch
        .providers
        .get_mut("zai")
        .expect("zai")
        .headers
        .insert("x-trace".into(), secret.into());
    let prepared = prepare_user_config_edit(fixture.home.path().join(".smith"), &patch)
        .expect("a non-authorization header is allowed");
    let rendered = format!("{prepared:?}\n{}", prepared.preview());
    assert!(!rendered.contains(secret), "{rendered}");
    assert!(rendered.contains("[configured header value]"), "{rendered}");
}

#[cfg(unix)]
#[test]
fn inline_credential_replacement_is_redacted_restrictive_and_exactly_reversible() {
    use std::os::unix::fs::PermissionsExt;

    const SECRET: &str = "sk-inline-edit-must-not-render";
    let fixture = Fixture::new();
    fixture.write_user(
        r#"# exact prior credential config
[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "keychain:smith/zai"
"#,
    );
    let path = fixture.home.path().join(".smith/config.toml");
    let before = fs::read(&path).expect("prior bytes");
    let patch = ConfigFile {
        providers: BTreeMap::from([(
            "zai".to_owned(),
            ProviderSection {
                api_key: Some(ConfigSecret::new(SECRET)),
                ..ProviderSection::default()
            },
        )]),
        ..ConfigFile::default()
    };

    let prepared = prepare_user_config_edit(fixture.home.path().join(".smith"), &patch)
        .expect("an inline credential edit");
    assert_eq!(prepared.collisions().len(), 1);
    assert_eq!(prepared.collisions()[0].key, "providers.zai.credential");
    let rendered = format!("{prepared:?}\n{}", prepared.preview());
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(rendered.contains("api_key"), "{rendered}");
    assert!(rendered.contains("[redacted]"), "{rendered}");
    assert!(matches!(
        prepared.commit(false),
        Err(UserConfigEditError::UnconfirmedCollisions { count: 1 })
    ));
    assert_eq!(fs::read(&path).expect("unchanged bytes"), before);

    let committed = prepared.commit(true).expect("the reviewed replacement");
    let candidate = fs::read_to_string(&path).expect("candidate config");
    assert!(candidate.contains(&format!("api_key = \"{SECRET}\"")));
    assert!(!candidate.contains("credential ="));
    assert_eq!(
        fs::metadata(&path)
            .expect("config metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    committed.rollback().expect("exact rollback");
    assert_eq!(fs::read(path).expect("restored bytes"), before);
}

#[test]
fn replacing_one_inline_key_redacts_both_sides_of_collision_review() {
    const OLD_SECRET: &str = "sk-old-inline-must-not-render";
    const NEW_SECRET: &str = "sk-new-inline-must-not-render";
    let fixture = Fixture::new();
    fixture.write_user(&format!(
        "[providers.zai]\nkind = \"openai-compatible\"\nbase_url = \"https://example.test/v1\"\napi_key = \"{OLD_SECRET}\"\n"
    ));
    let patch = ConfigFile {
        providers: BTreeMap::from([(
            "zai".to_owned(),
            ProviderSection {
                api_key: Some(ConfigSecret::new(NEW_SECRET)),
                ..ProviderSection::default()
            },
        )]),
        ..ConfigFile::default()
    };
    let prepared = prepare_user_config_edit(fixture.home.path().join(".smith"), &patch)
        .expect("a replacement");
    let rendered = format!("{prepared:?}\n{}", prepared.preview());
    assert!(!rendered.contains(OLD_SECRET), "{rendered}");
    assert!(!rendered.contains(NEW_SECRET), "{rendered}");
    assert!(rendered.matches("[redacted]").count() >= 2, "{rendered}");
}

#[test]
fn reasoning_only_response_policy_is_typed_optional_and_provenanced() {
    for (spelling, expected) in [
        ("reasoning", ReasoningOnlyBehavior::Reasoning),
        ("text", ReasoningOnlyBehavior::Text),
    ] {
        let fixture = Fixture::new();
        fixture.write_user(&format!(
            r#"
default_profile = "dev"
[profiles.dev]
provider = "remote"
model = "m"
[providers.remote]
kind = "openai-compatible"
base_url = "https://example.test/v1"
[providers.remote.response]
reasoning_only = "{spelling}"
[models."remote/m"]
context_tokens = 100
max_input_tokens = 90
max_output_tokens = 10
"#
        ));
        let resolved = resolve(&fixture.request()).expect("a resolved response policy");
        let policy = resolved
            .config
            .provider
            .response
            .reasoning_only
            .expect("the response policy");
        assert_eq!(policy.value, expected);
        assert_eq!(policy.source.layer, Layer::UserFile);
        assert!(policy.source.key.ends_with("response.reasoning_only"));
    }

    let fixture = Fixture::new();
    fixture.write_user(READY);
    assert!(
        resolve(&fixture.request())
            .expect("an omitted policy")
            .config
            .provider
            .response
            .reasoning_only
            .is_none()
    );
}

#[test]
fn invalid_or_incompatible_response_policy_fails_during_resolution() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "dev"
[profiles.dev]
provider = "remote"
model = "m"
[providers.remote]
kind = "openai-compatible"
base_url = "https://example.test/v1"
[providers.remote.response]
reasoning_only = "visible-ish"
[models."remote/m"]
context_tokens = 100
max_input_tokens = 90
max_output_tokens = 10
"#,
    );
    assert!(matches!(
        resolve(&fixture.request()),
        Err(ConfigError::Malformed { .. })
    ));

    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "dev"
[profiles.dev]
provider = "local"
model = "m"
[providers.local]
kind = "fake"
[providers.local.response]
reasoning_only = "text"
[models."local/m"]
context_tokens = 100
max_input_tokens = 90
max_output_tokens = 10
"#,
    );
    let error = resolve(&fixture.request()).expect_err("fake has no reasoning stream");
    assert!(matches!(error, ConfigError::IncompatibleOption { .. }));
    assert!(error.to_string().contains("response.reasoning_only"));
}

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
