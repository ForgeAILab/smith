//! Web configuration, provenance, repository authority, and posture policy.

use smith_config::credential::CredentialRef;
use smith_config::model::{ConfigFile, ProfileUse, WebSearchBackend, WebSearchMode};
use smith_config::resolve::{
    ConfigError, KnownModule, Layer, Overrides, Resolution, ResolveRequest, SettingValue, resolve,
};
use tempfile::TempDir;

const BASE: &str = r#"
default_profile = "chat"
[profiles.chat]
provider = "chatgpt"
model = "main"
[profiles.glm]
provider = "glm"
model = "text"
[profiles.child]
provider = "glm"
model = "text"
use = ["child"]
[providers.chatgpt]
kind = "chatgpt-responses"
base_url = "https://chatgpt.com/backend-api/codex"
credential = "authfile:chatgpt"
[providers.glm]
kind = "openai-compatible"
base_url = "https://example.test/v1"
credential = "env:GLM_API_KEY"
"#;

struct Fixture {
    home: TempDir,
    project: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            home: tempfile::tempdir().unwrap(),
            project: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir(fixture.home.path().join(".smith")).unwrap();
        std::fs::create_dir(fixture.project.path().join(".smith")).unwrap();
        fixture.project("config.toml", BASE);
        fixture
    }

    fn user(&self, config: &str) {
        std::fs::write(self.home.path().join(".smith/config.toml"), config).unwrap();
    }

    fn project(&self, name: &str, config: &str) {
        std::fs::write(self.project.path().join(".smith").join(name), config).unwrap();
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path()).with_home_dir(self.home.path())
    }

    fn resolve(&self) -> Result<Resolution, ConfigError> {
        resolve(&self.request())
    }
}

#[test]
fn web_defaults_are_safe_keyless_and_explainable() {
    let resolved = Fixture::new().resolve().unwrap();
    let config = &resolved.config;
    assert!(!config.web.fetch.allow_private_networks.value);
    assert_eq!(config.web.search.mode.value, WebSearchMode::Auto);
    assert_eq!(
        config.web.search.backend.value,
        WebSearchBackend::DuckDuckGo
    );
    assert!(config.web.search.api_key.is_none());
    assert!(config.web.search.searxng_url.is_none());
    assert!(config.web.search.diagnostic().is_none());
    assert_eq!(
        config.agent.read_only_network_modules.value,
        ["web", "vision"]
    );
    for key in [
        "tools.web_fetch.allow_private_networks",
        "tools.web_search.mode",
        "tools.web_search.backend",
        "agent.read_only_network_modules",
    ] {
        assert_eq!(
            resolved.provenance.explain(key).unwrap().source.layer,
            Layer::BuiltIn
        );
    }
}

#[test]
fn repository_cannot_open_private_networks_or_set_backend_authority() {
    for setting in [
        "[tools.web_fetch]\nallow_private_networks = true",
        "[tools.web_fetch]\nallow_private_networks = false",
        "[tools.web_search]\nbackend = \"duckduckgo\"",
        "[tools.web_search]\napi_key = \"env:SEARCH_KEY\"",
        "[tools.web_search]\nsearxng_url = \"https://search.example.test\"",
        "[agent]\nread_only_network_modules = []",
    ] {
        for name in ["config.toml", "config.local.toml"] {
            let fixture = Fixture::new();
            fixture.project(name, &format!("{BASE}\n{setting}"));
            match fixture.resolve().unwrap_err() {
                ConfigError::InvalidValue { source, message } => {
                    assert_eq!(source.file.unwrap().file_name().unwrap(), name);
                    assert!(message.contains("user-scoped"), "{message}");
                    assert!(source.key.starts_with("tools.") || source.key.starts_with("agent."));
                }
                error => panic!("expected repository refusal: {error}"),
            }
        }
    }
}

#[test]
fn repository_profile_cannot_smuggle_backend_settings_through_inheritance() {
    for setting in [
        "[profiles.base.tools.web_fetch]\nallow_private_networks = true",
        "[profiles.base.tools.web_search]\nbackend = \"brave\"",
        "[profiles.base.tools.web_search]\napi_key = \"env:SEARCH_KEY\"",
        "[profiles.base.tools.web_search]\nsearxng_url = \"https://search.example.test\"",
    ] {
        let fixture = Fixture::new();
        fixture.project(
            "config.local.toml",
            &format!("[profiles.chat]\nextends = \"base\"\n{setting}"),
        );
        match fixture.resolve().unwrap_err() {
            ConfigError::InvalidValue { source, .. } => {
                assert!(source.key.starts_with("profiles.base.tools."))
            }
            error => panic!("expected profile authority refusal: {error}"),
        }
    }
}

#[test]
fn missing_backend_key_is_a_diagnostic_and_does_not_prevent_startup() {
    for backend in ["brave", "tavily"] {
        let fixture = Fixture::new();
        fixture.user(&format!("[tools.web_search]\nbackend = \"{backend}\""));
        let resolved = fixture.resolve().unwrap();
        let (key, message) = resolved.config.web.search.diagnostic().unwrap();
        assert_eq!(key, "tools.web_search.api_key");
        assert!(message.contains("missing credential"));
        assert!(message.contains("unavailable"));
        let explanation = resolved
            .provenance
            .explain("tools.web_search.backend")
            .unwrap();
        assert_eq!(explanation.value, SettingValue::Text(backend.to_owned()));
        assert_eq!(explanation.source.layer, Layer::UserFile);
    }
}

#[test]
fn searxng_without_a_url_is_a_nonfatal_key_diagnostic() {
    let fixture = Fixture::new();
    fixture.user("[tools.web_search]\nbackend = \"searxng\"");
    let resolved = fixture.resolve().unwrap();
    let (key, message) = resolved.config.web.search.diagnostic().unwrap();
    assert_eq!(key, "tools.web_search.searxng_url");
    assert!(message.contains("missing SearXNG URL"));
}

#[test]
fn search_credentials_remain_references_without_reading_secrets() {
    for reference in [
        "env:SMITH_WEB_TEST_MISSING_KEY",
        "authfile:web-search-not-stored",
        "keychain:smith/search-not-stored",
        "file:does-not-exist",
        "session-json:/does-not-exist#/token",
    ] {
        let fixture = Fixture::new();
        fixture.user(&format!(
            "[tools.web_search]\nbackend = \"brave\"\napi_key = \"{reference}\""
        ));
        let resolved = fixture.resolve().unwrap();
        let credential = resolved.config.web.search.api_key.unwrap();
        assert_eq!(credential.value, CredentialRef::parse(reference).unwrap());
        assert_eq!(credential.source.layer, Layer::UserFile);
        assert_eq!(
            resolved
                .provenance
                .explain("tools.web_search.api_key")
                .unwrap()
                .value,
            SettingValue::Text(reference.to_owned())
        );
    }
}

#[test]
fn inline_search_keys_are_refused_without_exposing_the_value() {
    const TOKEN: &str = "sk-secret-must-not-be-printed";
    let fixture = Fixture::new();
    fixture.user(&format!("[tools.web_search]\napi_key = \"{TOKEN}\""));
    let error = fixture.resolve().unwrap_err();
    assert!(!format!("{error} {error:?}").contains(TOKEN));
    let error = resolve(
        &Fixture::new()
            .request()
            .with_env([("SMITH_TOOLS_WEB_SEARCH_API_KEY", TOKEN)]),
    )
    .unwrap_err();
    assert!(!format!("{error} {error:?}").contains(TOKEN));
}

#[test]
fn searxng_accepts_absolute_http_urls_and_rejects_invalid_or_credentialed_urls() {
    for url in [
        "https://search.example.test/",
        "http://localhost:8080/search",
    ] {
        let fixture = Fixture::new();
        fixture.user(&format!(
            "[tools.web_search]\nbackend = \"searxng\"\nsearxng_url = \"{url}\""
        ));
        let resolved = fixture.resolve().unwrap();
        assert!(resolved.config.web.search.diagnostic().is_none());
        assert_eq!(resolved.config.web.search.searxng_url.unwrap().value, url);
    }
    for url in [
        "",
        "/search",
        "search.example.test",
        "ftp://search.example.test",
        "https://user:secret@search.example.test",
    ] {
        let fixture = Fixture::new();
        fixture.user(&format!("[tools.web_search]\nsearxng_url = \"{url}\""));
        match fixture.resolve().unwrap_err() {
            ConfigError::InvalidValue { source, message } => {
                assert_eq!(source.key, "tools.web_search.searxng_url");
                assert!(!message.contains(url) || url.is_empty());
            }
            error => panic!("expected URL validation: {error}"),
        }
    }
}

#[test]
fn user_can_allow_private_fetch_and_restore_the_strict_posture_filter() {
    let fixture = Fixture::new();
    fixture.user(
        "[tools.web_fetch]\nallow_private_networks = true\n[agent]\nread_only_network_modules = []",
    );
    let resolved = fixture.resolve().unwrap();
    assert!(resolved.config.web.fetch.allow_private_networks.value);
    assert!(
        resolved
            .config
            .agent
            .read_only_network_modules
            .value
            .is_empty()
    );
    assert_eq!(
        resolved
            .provenance
            .explain("agent.read_only_network_modules")
            .unwrap()
            .source
            .layer,
        Layer::UserFile
    );
}

#[test]
fn read_only_module_ids_are_validated_and_errors_list_known_ids() {
    let fixture = Fixture::new();
    fixture.user("[agent]\nread_only_network_modules = [\"budget-notice\", \"image-generation\", \"web\", \"vision\"]");
    fixture.resolve().unwrap();
    fixture.user("[agent]\nread_only_network_modules = [\"veb\"]");
    match fixture.resolve().unwrap_err() {
        ConfigError::UnknownModule {
            id,
            source,
            known_ids,
        } => {
            assert_eq!(id, "veb");
            assert_eq!(source.key, "agent.read_only_network_modules");
            assert_eq!(
                known_ids,
                ["budget-notice", "image-generation", "vision", "web"]
            );
        }
        error => panic!("expected module validation: {error}"),
    }
    fixture.user("[agent]\nread_only_network_modules = [\"host-module\"]");
    resolve(&fixture.request().with_known_modules(vec![KnownModule {
        id: "host-module".to_owned(),
        default_enabled: true,
        compiled_in: true,
        legacy_enabled_key: None,
    }]))
    .unwrap();
}

#[test]
fn mode_layers_keep_precedence_and_provenance() {
    let fixture = Fixture::new();
    fixture.user("[tools.web_search]\nmode = \"hosted\"");
    fixture.project(
        "config.toml",
        &format!("{BASE}\n[tools.web_search]\nmode = \"backend\""),
    );
    fixture.project(
        "config.local.toml",
        "[tools.web_search]\nmode = \"off\"\n[profiles.chat.tools.web_search]\nmode = \"auto\"",
    );
    let mut request = fixture.request();
    assert_eq!(
        resolve(&request)
            .unwrap()
            .config
            .web
            .search
            .mode
            .source
            .layer,
        Layer::Profile
    );
    request.env.insert(
        "SMITH_TOOLS_WEB_SEARCH_MODE".to_owned(),
        "hosted".to_owned(),
    );
    assert_eq!(
        resolve(&request)
            .unwrap()
            .config
            .web
            .search
            .mode
            .source
            .layer,
        Layer::Environment
    );
    request.cli.web_search_mode = Some(WebSearchMode::Backend);
    assert_eq!(
        resolve(&request)
            .unwrap()
            .config
            .web
            .search
            .mode
            .source
            .layer,
        Layer::CommandLine
    );
    request.session.web_search_mode = Some(WebSearchMode::Off);
    let resolved = resolve(&request).unwrap();
    assert_eq!(resolved.config.web.search.mode.value, WebSearchMode::Off);
    let explanation = resolved
        .provenance
        .explain("tools.web_search.mode")
        .unwrap();
    assert_eq!(explanation.source.layer, Layer::SessionOverride);
    assert_eq!(
        explanation
            .overridden
            .iter()
            .map(|entry| entry.source.layer)
            .collect::<Vec<_>>(),
        [
            Layer::CommandLine,
            Layer::Environment,
            Layer::Profile,
            Layer::ProjectLocalFile,
            Layer::ProjectFile,
            Layer::UserFile,
            Layer::BuiltIn
        ]
    );
}

#[test]
fn user_profile_can_supply_backend_settings_with_inherited_provenance() {
    let fixture = Fixture::new();
    fixture.user("[profiles.base.tools.web_search]\nbackend = \"tavily\"\napi_key = \"env:SEARCH_KEY\"\n[profiles.chat]\nextends = \"base\"");
    let resolved = fixture.resolve().unwrap();
    let search = resolved.config.web.search;
    assert_eq!(search.backend.value, WebSearchBackend::Tavily);
    assert_eq!(
        search.backend.source.key,
        "profiles.base.tools.web_search.backend"
    );
    assert_eq!(search.backend.source.layer, Layer::Profile);
    assert!(search.diagnostic().is_none());
}

#[test]
fn routing_inputs_cover_chatgpt_auto_free_preference_model_switch_and_child() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.resolve().unwrap().config.web.search.mode.value,
        WebSearchMode::Auto
    );
    fixture.user("[tools.web_search]\nmode = \"backend\"");
    let free = fixture.resolve().unwrap();
    assert_eq!(free.config.web.search.mode.value, WebSearchMode::Backend);
    assert_eq!(
        free.config.web.search.backend.value,
        WebSearchBackend::DuckDuckGo
    );
    fixture.user(
        "[tools.web_search]\nmode = \"auto\"\nbackend = \"brave\"\napi_key = \"env:BRAVE_API_KEY\"",
    );
    for (profile, placement) in [
        ("chat", ProfileUse::Main),
        ("glm", ProfileUse::Main),
        ("child", ProfileUse::Child),
    ] {
        let resolved = resolve(
            &fixture
                .request()
                .with_session(Overrides {
                    profile: Some(profile.to_owned()),
                    ..Overrides::default()
                })
                .with_profile_use(placement),
        )
        .unwrap();
        assert_eq!(resolved.config.web.search.mode.value, WebSearchMode::Auto);
        assert_eq!(
            resolved.config.web.search.backend.value,
            WebSearchBackend::Brave
        );
        assert_eq!(resolved.config.agent.profile.name, profile);
    }
}

#[test]
fn web_settings_round_trip_and_validate_vocabulary_and_types() {
    let file = ConfigFile::parse("[agent]\nread_only_network_modules = []\n[tools.web_fetch]\nallow_private_networks = true\n[tools.web_search]\nmode = \"auto\"\nbackend = \"brave\"\napi_key = \"env:SEARCH_KEY\"\nsearxng_url = \"https://search.example.test\"").unwrap();
    assert_eq!(
        ConfigFile::parse(&toml::to_string(&file).unwrap()).unwrap(),
        file
    );
    assert!(ConfigFile::default().agent.is_none());
    for config in [
        "[agent]\nread_only_network_modules = true",
        "[tools.web_fetch]\nallow_private_networks = \"true\"",
        "[tools.web_search]\nmode = \"always\"",
        "[tools.web_search]\nbackend = \"google\"",
        "[tools.web_search]\napi_key = false",
        "[tools.web_search]\nunknown = true",
    ] {
        assert!(ConfigFile::parse(config).is_err(), "{config}");
    }
    for (key, value) in [("MODE", "always"), ("BACKEND", "google")] {
        let error = resolve(
            &Fixture::new()
                .request()
                .with_env([(format!("SMITH_TOOLS_WEB_SEARCH_{key}"), value)]),
        )
        .unwrap_err();
        assert!(matches!(error, ConfigError::InvalidValue { .. }));
    }
}

#[test]
fn owner_environment_overrides_keep_their_sources_and_validate_references() {
    let fixture = Fixture::new();
    let resolved = resolve(&fixture.request().with_env([
        ("SMITH_TOOLS_WEB_FETCH_ALLOW_PRIVATE_NETWORKS", "true"),
        ("SMITH_TOOLS_WEB_SEARCH_BACKEND", "searxng"),
        ("SMITH_TOOLS_WEB_SEARCH_API_KEY", "env:SEARCH_KEY"),
        (
            "SMITH_TOOLS_WEB_SEARCH_SEARXNG_URL",
            "https://search.example.test",
        ),
        ("SMITH_AGENT_READ_ONLY_NETWORK_MODULES", "vision"),
    ]))
    .unwrap();
    assert!(resolved.config.web.fetch.allow_private_networks.value);
    assert_eq!(
        resolved.config.web.search.backend.value,
        WebSearchBackend::Searxng
    );
    assert_eq!(
        resolved.config.agent.read_only_network_modules.value,
        ["vision"]
    );
    for key in [
        "tools.web_fetch.allow_private_networks",
        "tools.web_search.backend",
        "tools.web_search.api_key",
        "tools.web_search.searxng_url",
        "agent.read_only_network_modules",
    ] {
        let source = resolved.provenance.explain(key).unwrap().source;
        assert_eq!(source.layer, Layer::Environment);
        assert!(source.key.starts_with("SMITH_"));
    }
}

#[test]
fn web_fetch_host_approval_rules_accept_exact_hosts_and_reject_wildcards() {
    let fixture = Fixture::new();
    for (host, valid) in [("docs.rs", true), ("*.rs", false), ("docs.rs/path", false)] {
        fixture.user(&format!(
            r#"
[[approval.auto]]
revision = 1
tool = "smith/web_fetch"
resource = "web:{host}"
operations = []
permissions = ["net.http", "data.egress"]
max_risk = "low"
mount = "workspace"
paths = []
"#
        ));
        assert_eq!(fixture.resolve().is_ok(), valid, "{host}");
    }
}
