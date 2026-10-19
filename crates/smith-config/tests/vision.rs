//! Vision selection, image-input metadata, and the request image window.

use std::collections::BTreeMap;
use std::sync::Arc;

use smith_config::catalog::{
    CATALOG_SCHEMA_REVISION, CatalogModality, CatalogModel, CatalogProvider, CatalogSnapshot,
    EndpointCatalog,
};
use smith_config::model::{ConfigFile, KIND_COMMAND_JSONL, ProfileUse, VisionSelection};
use smith_config::resolve::{
    ConfigError, ImageInput, Layer, Overrides, ReferenceKind, Resolution, ResolveRequest,
    SettingValue, VisionTarget, image_input, resolve,
};
use tempfile::TempDir;

struct Fixture {
    home: TempDir,
    project: TempDir,
}

impl Fixture {
    fn new(default: &str, code: &str, extra: &str) -> Self {
        let fixture = Self {
            home: tempfile::tempdir().unwrap(),
            project: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir(fixture.home.path().join(".smith")).unwrap();
        std::fs::create_dir(fixture.project.path().join(".smith")).unwrap();
        fixture.write_project(
            "config.toml",
            &format!(
                r#"
default_profile = "code"
{default}
[profiles.code]
provider = "acme"
model = "working-model"
{code}
[profiles.sol]
provider = "google"
model = "gemini-3-flash"
[providers.acme]
kind = "openai-compatible"
base_url = "https://custom.example.test/v1"
credential = "file:must-not-be-read"
[providers.google]
kind = "gemini-interactions"
credential = "file:must-not-be-read"
{extra}
"#
            ),
        );
        fixture
    }

    fn write_project(&self, name: &str, config: &str) {
        std::fs::write(self.project.path().join(".smith").join(name), config).unwrap();
    }

    fn write_user(&self, config: &str) {
        std::fs::write(self.home.path().join(".smith/config.toml"), config).unwrap();
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path())
            .with_home_dir(self.home.path())
            .with_catalog(Arc::new(catalog()))
    }

    fn resolve(&self) -> Result<Resolution, ConfigError> {
        resolve(&self.request())
    }
}

fn catalog_model(modalities: &[CatalogModality]) -> CatalogModel {
    CatalogModel {
        id: "gemini-3-flash".to_owned(),
        name: "Gemini 3 Flash".to_owned(),
        limits: None,
        input_modalities: modalities.to_vec(),
        output_modalities: vec![CatalogModality::Text],
        tool_call: true,
        reasoning: false,
        reasoning_controls: None,
        structured_output: false,
        cost: None,
        disabled_reason: None,
    }
}

fn catalog() -> CatalogSnapshot {
    CatalogSnapshot {
        schema_revision: CATALOG_SCHEMA_REVISION,
        source_url: "https://example.test/catalog".to_owned(),
        source_digest: "digest".to_owned(),
        content_digest: "content".to_owned(),
        source_revision: "vision-test".to_owned(),
        retrieved_at_ms: 0,
        endpoint_catalogs: BTreeMap::new(),
        providers: BTreeMap::from([(
            "google".to_owned(),
            CatalogProvider {
                id: "google".to_owned(),
                name: "Google".to_owned(),
                models: BTreeMap::from([(
                    "gemini-3-flash".to_owned(),
                    catalog_model(&[CatalogModality::Text, CatalogModality::Image]),
                )]),
            },
        )]),
    }
}

fn endpoint_catalog() -> CatalogSnapshot {
    let mut snapshot = catalog();
    for (provider, model) in [("openai", "gpt-5.6-luna"), ("xai", "grok-4")] {
        let mut entry = catalog_model(&[
            CatalogModality::Text,
            CatalogModality::Image,
            CatalogModality::Document,
        ]);
        entry.id = model.to_owned();
        snapshot.providers.insert(
            provider.to_owned(),
            CatalogProvider {
                id: provider.to_owned(),
                name: provider.to_owned(),
                models: BTreeMap::from([(model.to_owned(), entry)]),
            },
        );
    }
    snapshot
}

#[test]
fn exact_endpoint_modalities_resolve_with_endpoint_revision_and_explicit_precedence() {
    let fixture = Fixture::new("", "", "");
    let mut snapshot = catalog();
    let mut entry = catalog_model(&[CatalogModality::Text, CatalogModality::Image]);
    entry.id = "working-model".to_owned();
    snapshot.endpoint_catalogs.insert(
        "acme".to_owned(),
        EndpointCatalog {
            binding_digest: String::new(),
            unadvertised_capabilities: Default::default(),
            kind: "openai-compatible".to_owned(),
            base_url: "https://custom.example.test/v1/".to_owned(),
            source: "endpoint/acme".to_owned(),
            revision: "acme-r7".to_owned(),
            retrieved_at_ms: 7_000,
            partial_limits: BTreeMap::new(),
            catalog: CatalogProvider {
                id: "acme".to_owned(),
                name: "Acme".to_owned(),
                models: BTreeMap::from([("working-model".to_owned(), entry)]),
            },
        },
    );
    let request = fixture.request().with_catalog(Arc::new(snapshot.clone()));
    let resolution = resolve(&request).expect("endpoint metadata");
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::Accepts
    );
    assert!(
        resolution
            .config
            .model_image_input
            .source
            .key
            .contains("acme@acme-r7 source=endpoint/acme")
    );
    assert!(
        resolution
            .config
            .model_image_input
            .source
            .key
            .contains("retrieved_at_ms=7000")
    );
    fixture.write_user("[models.\"acme/working-model\"]\ninput_modalities = [\"text\"]\n");
    let resolution = resolve(&request).expect("explicit modality wins");
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::TextOnly
    );
    assert_eq!(
        resolution.config.model_image_input.source.layer,
        Layer::UserFile
    );
    snapshot.endpoint_catalogs.get_mut("acme").unwrap().base_url =
        "https://another.example/v1".to_owned();
    fixture.write_user("");
    let resolution = resolve(&fixture.request().with_catalog(Arc::new(snapshot))).unwrap();
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::Unknown
    );
}

fn model(provider: &str, model: &str) -> VisionTarget {
    VisionTarget::Model {
        provider: provider.to_owned(),
        model: model.to_owned(),
    }
}

#[test]
fn image_input_uses_endpoint_catalog_bindings_and_preserves_local_identity() {
    for (provider, kind, endpoint, model, catalog_provider, credential) in [
        (
            "chatgpt",
            "chatgpt-responses",
            Some(smith_config::setup::CHATGPT_ENDPOINT),
            "gpt-5.6-luna",
            "openai",
            "authfile:chatgpt",
        ),
        (
            "platform",
            "openai-compatible",
            Some(smith_config::catalog::OPENAI_ENDPOINT),
            "gpt-5.6-luna",
            "openai",
            "file:must-not-be-read",
        ),
        (
            "responses",
            "openai-responses",
            Some(smith_config::catalog::OPENAI_ENDPOINT),
            "gpt-5.6-luna",
            "openai",
            "file:must-not-be-read",
        ),
        (
            "grok",
            "xai-responses",
            Some(smith_config::setup::XAI_ENDPOINT),
            "grok-4",
            "xai",
            "file:must-not-be-read",
        ),
        (
            "gemini",
            "gemini-interactions",
            None,
            "gemini-3-flash",
            "google",
            "file:must-not-be-read",
        ),
    ] {
        let fixture = Fixture::new("", "", "");
        let endpoint = endpoint.map_or_else(String::new, |url| format!("base_url = {url:?}\n"));
        fixture.write_user(&format!(
            "[providers.{provider}]\nkind = {kind:?}\n{endpoint}credential = {credential:?}\n"
        ));
        let resolution = resolve(
            &fixture
                .request()
                .with_catalog(Arc::new(endpoint_catalog()))
                .with_cli(Overrides {
                    provider: Some(provider.to_owned()),
                    model: Some(model.to_owned()),
                    ..Overrides::default()
                }),
        )
        .unwrap();
        assert_eq!(resolution.config.provider.name.value, provider);
        assert_eq!(resolution.config.model.value, model);
        assert_eq!(
            resolution.config.model_image_input.value,
            ImageInput::Accepts,
            "{provider}/{model}"
        );
        let source = &resolution.config.model_image_input.source;
        assert!(
            source
                .key
                .contains(&format!("{catalog_provider}@vision-test"))
        );
        assert_eq!(
            &resolution
                .provenance
                .explain("input_modalities")
                .unwrap()
                .source,
            source
        );
    }
}

#[test]
fn chatgpt_vision_profile_validates_through_the_openai_catalog() {
    let fixture = Fixture::new(
        "vision = \"luna\"",
        "",
        "[profiles.luna]\nprovider = \"chatgpt\"\nmodel = \"gpt-5.6-luna\"",
    );
    fixture.write_user(&format!(
        "[providers.chatgpt]\nkind = \"chatgpt-responses\"\nbase_url = {:?}\ncredential = \"authfile:chatgpt\"\n",
        smith_config::setup::CHATGPT_ENDPOINT,
    ));
    let resolution = resolve(&fixture.request().with_catalog(Arc::new(endpoint_catalog())))
        .expect("ChatGPT vision profile without credential access");
    assert_eq!(
        resolution.config.agent.profile.vision.unwrap().value,
        VisionTarget::Profile("luna".to_owned())
    );
}

#[test]
fn image_input_stays_unknown_for_unmapped_endpoints_and_unknown_chatgpt_models() {
    let fixture = Fixture::new("", "", "");
    fixture.write_user(&format!(
        "[providers.chatgpt]\nkind = \"chatgpt-responses\"\nbase_url = {:?}\ncredential = \"authfile:chatgpt\"\n",
        smith_config::setup::CHATGPT_ENDPOINT,
    ));
    for (provider, model) in [("acme", "gpt-5.6-luna"), ("chatgpt", "unknown-model")] {
        let resolution = resolve(
            &fixture
                .request()
                .with_catalog(Arc::new(endpoint_catalog()))
                .with_cli(Overrides {
                    provider: Some(provider.to_owned()),
                    model: Some(model.to_owned()),
                    ..Overrides::default()
                }),
        )
        .unwrap();
        assert_eq!(
            resolution.config.model_image_input.value,
            ImageInput::Unknown
        );
        assert!(resolution.config.model_input_modalities.is_none());
    }
}

#[test]
fn catalog_image_input_is_recorded_before_runtime_layering() {
    let fixture = Fixture::new("", "", "");
    let resolution = resolve(&fixture.request().with_cli(Overrides {
        profile: Some("sol".to_owned()),
        ..Overrides::default()
    }))
    .unwrap();
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::Accepts
    );
    let source = &resolution.config.model_image_input.source;
    assert!(source.key.contains("google@vision-test"));
    let explained = resolution.provenance.explain("input_modalities").unwrap();
    assert_eq!(&explained.source, source);
    assert_eq!(
        explained.value,
        SettingValue::List(vec!["text".to_owned(), "image".to_owned()])
    );
}

#[test]
fn uncatalogued_override_preserves_the_user_file_source_and_other_model_fields() {
    let fixture = Fixture::new("", "", "");
    fixture.write_user(
        "[models.\"acme/working-model\"]\ninput_modalities = [\"text\", \"image\"]\ncontext_tokens = 32768\n",
    );
    fixture.write_project(
        "config.local.toml",
        "[models.\"acme/working-model\"]\nmax_output_tokens = 4096\n",
    );
    let resolution = fixture.resolve().unwrap();
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::Accepts
    );
    assert_eq!(
        resolution.config.model_image_input.source.layer,
        Layer::UserFile
    );
    assert_eq!(
        resolution.config.model_limits.context_tokens.unwrap().value,
        32768
    );
    let explained = resolution.provenance.explain("input_modalities").unwrap();
    assert_eq!(explained.source, resolution.config.model_image_input.source);
    assert!(
        explained
            .source
            .file
            .unwrap()
            .ends_with(".smith/config.toml")
    );
}

#[test]
fn explicit_field_wins_over_catalog_including_an_empty_override() {
    for modalities in ["[\"text\"]", "[]"] {
        let fixture = Fixture::new("", "", "");
        fixture.write_user(&format!(
            "[models.\"google/gemini-3-flash\"]\ninput_modalities = {modalities}\n"
        ));
        let resolution = resolve(&fixture.request().with_cli(Overrides {
            profile: Some("sol".to_owned()),
            ..Overrides::default()
        }))
        .unwrap();
        assert_eq!(
            resolution.config.model_image_input.value,
            ImageInput::TextOnly
        );
        assert_eq!(
            resolution.config.model_image_input.source.layer,
            Layer::UserFile
        );
        assert!(resolution.config.model_input_modalities.is_some());
        let explained = resolution.provenance.explain("input_modalities").unwrap();
        assert_eq!(explained.overridden.len(), 1);
        assert!(
            explained.overridden[0]
                .source
                .key
                .contains("google@vision-test")
        );
    }
}

#[test]
fn no_metadata_remains_unknown_and_can_be_explained() {
    let fixture = Fixture::new("", "", "");
    let resolution = fixture.resolve().unwrap();
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::Unknown
    );
    assert!(resolution.config.model_input_modalities.is_none());
    let explanation = resolution.provenance.explain("input_modalities").unwrap();
    assert!(explanation.value.to_string().contains("unknown"));
    assert!(
        explanation
            .source
            .key
            .contains("models.\"acme/working-model\".input_modalities")
    );
    assert!(explanation.source.key.contains("no image-input metadata"));
}

#[test]
fn an_empty_catalog_field_is_unknown_and_a_custom_endpoint_inherits_nothing() {
    let fixture = Fixture::new("", "", "");
    let mut empty = catalog();
    empty
        .providers
        .get_mut("google")
        .unwrap()
        .models
        .get_mut("gemini-3-flash")
        .unwrap()
        .input_modalities
        .clear();
    let resolution = resolve(&fixture.request().with_catalog(Arc::new(empty)).with_cli(
        Overrides {
            profile: Some("sol".to_owned()),
            ..Overrides::default()
        },
    ))
    .unwrap();
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::Unknown
    );
    assert!(resolution.config.model_input_modalities.is_none());

    // The custom endpoint must not inherit Google's metadata for the same model id.
    let resolution = resolve(&fixture.request().with_cli(Overrides {
        provider: Some("acme".to_owned()),
        model: Some("gemini-3-flash".to_owned()),
        ..Overrides::default()
    }))
    .unwrap();
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::Unknown
    );
    assert!(resolution.config.model_input_modalities.is_none());
}

#[test]
fn pure_image_input_rule_preserves_unknown_and_forces_command_text() {
    let empty = catalog_model(&[]);
    let images = catalog_model(&[CatalogModality::Image]);
    assert_eq!(image_input(None, None, "fake"), ImageInput::Unknown);
    assert_eq!(image_input(None, Some(&empty), "fake"), ImageInput::Unknown);
    assert_eq!(
        image_input(None, Some(&images), "fake"),
        ImageInput::Accepts
    );
    assert_eq!(
        image_input(Some(&[]), Some(&images), "fake"),
        ImageInput::TextOnly
    );
    assert_eq!(
        image_input(
            Some(&[CatalogModality::Image]),
            Some(&images),
            KIND_COMMAND_JSONL
        ),
        ImageInput::TextOnly
    );
}

#[test]
fn command_provider_ignores_image_override_and_explains_the_forced_source() {
    let fixture = Fixture::new("", "", "");
    fixture.write_user(
        r#"
[providers.bridge]
kind = "command-jsonl"
[providers.bridge.command]
executable = "/opt/smith/model-bridge"
[models."bridge/working-model"]
input_modalities = ["text", "image"]
context_tokens = 32768
max_input_tokens = 28672
max_output_tokens = 4096
"#,
    );
    let resolution = resolve(&fixture.request().with_cli(Overrides {
        provider: Some("bridge".to_owned()),
        ..Overrides::default()
    }))
    .unwrap();
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::TextOnly
    );
    let explained = resolution.provenance.explain("input_modalities").unwrap();
    assert_eq!(explained.value, SettingValue::List(vec!["text".to_owned()]));
    assert_eq!(explained.source.key, "providers.bridge.kind");
    assert_eq!(
        explained.overridden[0].value,
        SettingValue::List(vec!["text".to_owned(), "image".to_owned()])
    );
}

#[test]
fn top_level_vision_model_reference_resolves_with_catalog_metadata() {
    let fixture = Fixture::new("vision = \"google/gemini-3-flash\"", "", "");
    let resolution = fixture.resolve().unwrap();
    let selected = resolution.config.agent.profile.vision.unwrap();
    assert_eq!(selected.value, model("google", "gemini-3-flash"));
    assert_eq!(selected.source.key, "vision");
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::Unknown
    );
}

#[test]
fn explicit_vision_input_loads_without_a_catalog() {
    let fixture = Fixture::new(
        "vision = \"acme/uncatalogued-vl\"",
        "",
        "[models.\"acme/uncatalogued-vl\"]\ninput_modalities = [\"text\", \"image\"]",
    );
    let mut request = fixture.request();
    request.catalog = None;
    assert_eq!(
        resolve(&request)
            .unwrap()
            .config
            .agent
            .profile
            .vision
            .unwrap()
            .value,
        model("acme", "uncatalogued-vl")
    );
}

#[test]
fn unusable_vision_is_rejected_before_credentials_whatever_the_main_model_is() {
    for main in [
        "",
        "[models.\"acme/working-model\"]\ninput_modalities = [\"image\"]",
    ] {
        let fixture = Fixture::new("vision = \"acme/text-only\"", "", main);
        // A malformed credential would fail provider validation if it ran first.
        fixture.write_project(
            "config.local.toml",
            "[providers.acme]\ncredential = \"invalid\"\n",
        );
        match fixture.resolve().unwrap_err() {
            ConfigError::InvalidValue { source, message } => {
                assert_eq!(source.key, "vision");
                assert!(message.contains("does not accept image input"), "{message}");
            }
            error => panic!("vision validation must precede credential handling: {error}"),
        }
    }
}

#[test]
fn valid_vision_loads_when_the_main_model_already_accepts_images() {
    let fixture = Fixture::new(
        "vision = \"google/gemini-3-flash\"",
        "",
        "[models.\"acme/working-model\"]\ninput_modalities = [\"text\", \"image\"]",
    );
    let resolution = fixture.resolve().unwrap();
    assert_eq!(
        resolution.config.model_image_input.value,
        ImageInput::Accepts
    );
    assert!(resolution.config.agent.profile.vision.is_some());
}

#[test]
fn command_vision_is_rejected_even_with_an_explicit_image_override() {
    let fixture = Fixture::new("vision = \"bridge/vl\"", "", "");
    fixture.write_user(
        r#"
[providers.bridge]
kind = "command-jsonl"
[providers.bridge.command]
executable = "/opt/smith/model-bridge"
[models."bridge/vl"]
input_modalities = ["image"]
"#,
    );
    match fixture.resolve().unwrap_err() {
        ConfigError::InvalidValue { source, message } => {
            assert_eq!(source.key, "vision");
            assert!(message.contains("does not accept image input"));
        }
        error => panic!("expected command vision rejection: {error}"),
    }
}

#[test]
fn inherited_profile_selection_overrides_a_disabled_default_and_changes_revision() {
    let fixture = Fixture::new(
        "vision = false",
        "extends = \"base\"",
        "[profiles.base]\nvision = \"sol\"",
    );
    let with_vision = fixture.resolve().unwrap();
    let selected = with_vision.config.agent.profile.vision.as_ref().unwrap();
    assert_eq!(selected.value, VisionTarget::Profile("sol".to_owned()));
    assert_eq!(selected.source.key, "profiles.base.vision");
    let explained = with_vision.provenance.explain("vision").unwrap();
    assert_eq!(explained.source, selected.source);
    assert_eq!(explained.overridden[0].value, SettingValue::Flag(false));
    fixture.write_project("config.local.toml", "[profiles.code]\nvision = false");
    let without_vision = fixture.resolve().unwrap();
    assert!(without_vision.config.agent.profile.vision.is_none());
    assert_ne!(
        with_vision.config.agent.profile.revision,
        without_vision.config.agent.profile.revision
    );
}

#[test]
fn unusable_profile_selection_reports_the_profile_vision_key() {
    let fixture = Fixture::new(
        "",
        "vision = \"blind\"",
        "[profiles.blind]\nextends = \"code\"\nvision = false",
    );
    match fixture.resolve().unwrap_err() {
        ConfigError::InvalidValue { source, message } => {
            assert_eq!(source.key, "profiles.code.vision");
            assert!(message.contains("does not accept image input"));
        }
        error => panic!("expected profile vision rejection: {error}"),
    }
}

#[test]
fn profile_can_disable_top_level_vision() {
    let fixture = Fixture::new("vision = \"sol\"", "vision = false", "");
    let resolution = fixture.resolve().unwrap();
    assert!(resolution.config.agent.profile.vision.is_none());
    let explained = resolution.provenance.explain("vision").unwrap();
    assert_eq!(explained.value, SettingValue::Flag(false));
    assert_eq!(explained.source.key, "profiles.code.vision");
    assert_eq!(
        explained.overridden[0].value,
        SettingValue::Text("sol".to_owned())
    );
}

#[test]
fn inherited_vision_retains_every_file_and_inheritance_source() {
    let fixture = Fixture::new(
        "vision = \"sol\"",
        "extends = \"base\"\nvision = \"sol\"",
        "[profiles.base]\nvision = \"sol\"",
    );
    fixture.write_user(
        "vision = false\n[profiles.base]\nvision = false\n[profiles.code]\nvision = false\n",
    );
    fixture.write_project(
        "config.local.toml",
        "vision = false\n[profiles.base]\nvision = false\n[profiles.code]\nvision = false\n",
    );
    let resolution = fixture.resolve().unwrap();
    let explained = resolution.provenance.explain("vision").unwrap();
    assert_eq!(explained.source.key, "profiles.code.vision");
    assert_eq!(explained.source.layer, Layer::Profile);
    assert_eq!(
        explained
            .overridden
            .iter()
            .map(|entry| (entry.source.key.as_str(), entry.source.layer))
            .collect::<Vec<_>>(),
        vec![
            ("profiles.code.vision", Layer::Profile),
            ("profiles.code.vision", Layer::Profile),
            ("profiles.base.vision", Layer::Profile),
            ("profiles.base.vision", Layer::Profile),
            ("profiles.base.vision", Layer::Profile),
            ("vision", Layer::ProjectLocalFile),
            ("vision", Layer::ProjectFile),
            ("vision", Layer::UserFile),
        ]
    );
}

#[test]
fn child_profile_resolves_its_own_inherited_vision() {
    let fixture = Fixture::new(
        "vision = false",
        "",
        "[profiles.helper]\nextends = \"code\"\nuse = [\"child\"]\nvision = \"sol\"",
    );
    let resolution = resolve(
        &fixture
            .request()
            .with_cli(Overrides {
                profile: Some("helper".to_owned()),
                ..Overrides::default()
            })
            .with_profile_use(ProfileUse::Child),
    )
    .unwrap();
    assert_eq!(
        resolution.config.agent.profile.vision.unwrap().value,
        VisionTarget::Profile("sol".to_owned())
    );
}

#[test]
fn unknown_vision_targets_use_advisor_reference_errors() {
    for (target, what, name) in [
        ("missing", ReferenceKind::Profile, "missing"),
        ("missing/model", ReferenceKind::Provider, "missing"),
    ] {
        let fixture = Fixture::new(&format!("vision = \"{target}\""), "", "");
        match fixture.resolve().unwrap_err() {
            ConfigError::UnusableReference {
                source,
                what: actual,
                name: actual_name,
                ..
            } => {
                assert_eq!(source.key, "vision");
                assert_eq!(actual, what);
                assert_eq!(actual_name, name);
            }
            error => panic!("expected a reference error: {error}"),
        }
    }
}

#[test]
fn top_level_self_default_is_skipped_but_profile_self_reference_is_rejected() {
    let fixture = Fixture::new("vision = \"sol\"", "", "");
    let resolution = fixture.resolve().unwrap();
    assert!(resolution.config.agent.profiles["sol"].vision.is_none());
    for (own, extra, key) in [
        ("vision = \"code\"", "", "profiles.code.vision"),
        (
            "extends = \"base\"",
            "[profiles.base]\nvision = \"code\"",
            "profiles.base.vision",
        ),
    ] {
        let fixture = Fixture::new("", own, extra);
        match fixture.resolve().unwrap_err() {
            ConfigError::InvalidValue { source, message } => {
                assert_eq!(source.key, key);
                assert!(message.contains("cannot select itself as its `vision`"));
            }
            error => panic!("expected self-reference rejection: {error}"),
        }
    }
}

#[test]
fn vision_and_modalities_round_trip_and_reject_wrong_vocabulary_or_types() {
    let file = ConfigFile::parse("vision = \"google/gemini-3-flash\"\n[profiles.code]\nvision = false\n[models.\"local/vl\"]\ninput_modalities = [\"text\", \"image\", \"audio\", \"video\", \"document\"]\n").unwrap();
    assert_eq!(
        file.vision,
        Some(VisionSelection::Target("google/gemini-3-flash".to_owned()))
    );
    assert_eq!(
        ConfigFile::parse(&toml::to_string(&file).unwrap()).unwrap(),
        file
    );
    for prefix in ["", "[profiles.code]\n"] {
        for value in ["true", "7", "[\"sol\"]"] {
            assert!(ConfigFile::parse(&format!("{prefix}vision = {value}")).is_err());
        }
    }
    for value in ["\"\"", "\"/model\"", "\"provider/\""] {
        assert!(
            Fixture::new(&format!("vision = {value}"), "", "")
                .resolve()
                .is_err()
        );
    }
    assert!(ConfigFile::parse("[models.\"p/m\"]\ninput_modalities = [\"picture\"]").is_err());
}

#[test]
fn inline_images_defaults_bounds_and_environment_provenance() {
    let fixture = Fixture::new("", "", "");
    let defaults = fixture.resolve().unwrap();
    assert_eq!(defaults.config.context.inline_images.value, 10);
    assert_eq!(
        defaults.config.context.inline_images.source.layer,
        Layer::BuiltIn
    );
    for count in [1, 100] {
        fixture.write_project(
            "config.local.toml",
            &format!("[context]\ninline_images = {count}"),
        );
        assert_eq!(
            fixture
                .resolve()
                .unwrap()
                .config
                .context
                .inline_images
                .value,
            count
        );
    }
    let resolution = resolve(
        &fixture
            .request()
            .with_env([("SMITH_CONTEXT_INLINE_IMAGES", "7")]),
    )
    .unwrap();
    assert_eq!(resolution.config.context.inline_images.value, 7);
    let explained = resolution
        .provenance
        .explain("context.inline_images")
        .unwrap();
    assert_eq!(explained.source.key, "SMITH_CONTEXT_INLINE_IMAGES");
    assert_eq!(explained.overridden[0].value, SettingValue::Integer(100));
    for invalid in [0, 101] {
        fixture.write_project(
            "config.local.toml",
            &format!("[context]\ninline_images = {invalid}"),
        );
        let error = fixture.resolve().unwrap_err().to_string();
        assert!(error.contains("context.inline_images"), "{error}");
        assert!(error.contains("between 1 and 100"), "{error}");
    }
}

#[test]
fn inline_images_profile_override_has_provenance() {
    let fixture = Fixture::new(
        "",
        "[profiles.code.context]\ninline_images = 3",
        "[context]\ninline_images = 12",
    );
    let resolution = fixture.resolve().unwrap();
    assert_eq!(resolution.config.context.inline_images.value, 3);
    let explained = resolution
        .provenance
        .explain("context.inline_images")
        .unwrap();
    assert_eq!(explained.source.key, "profiles.code.context.inline_images");
    assert_eq!(explained.source.layer, Layer::Profile);
    assert_eq!(explained.overridden[0].value, SettingValue::Integer(12));
}
