//! Selectable model context-window behavior through the production factory.

use std::sync::Arc;

use agent_runtime_core::catalog::ModelLimits;
use agent_runtime_core::store::Secret;
use agent_runtime_testkit::MemoryWorkspace;
use smith_config::credential::{CredentialResolver, Environment};
use smith_config::resolve::{ResolveRequest, resolve};
use smith_runtime::factory::{self, FactoryError, HostSurface, RuntimeRequest};

const BASE_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"
context_window = "small"

[providers.local]
kind = "fake"

[models."local/example-model"]
max_output_tokens = 4096
default_context_window = "large"

[models."local/example-model".context_windows."small"]
context_tokens = 32768

[models."local/example-model".context_windows."large"]
context_tokens = 131072
"#;

const CHATGPT_CONFIG: &str = r#"
default_profile = "sol"

[profiles.sol]
provider = "chatgpt"
model = "gpt-6.1-sol"
use = ["main", "child"]

[providers.chatgpt]
kind = "fake"
"#;

const CHATGPT_FLAT_LIMITS: &str = r#"
[models."chatgpt/gpt-6.1-sol"]
context_tokens = 872000
max_input_tokens = 828400
max_output_tokens = 128000
"#;

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new(config: &str) -> Self {
        let home = tempfile::tempdir().expect("home directory");
        let project = tempfile::tempdir().expect("project directory");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("configuration directory");
        std::fs::write(config_dir.join("config.toml"), config).expect("configuration");
        Self { home, project }
    }

    fn request(&self) -> RuntimeRequest {
        let config =
            resolve(&ResolveRequest::new(self.project.path()).with_home_dir(self.home.path()))
                .expect("resolved config")
                .config;
        RuntimeRequest {
            workspace: Some(Arc::new(MemoryWorkspace::new("/repo"))),
            ..RuntimeRequest::new(config, HostSurface::Headless)
        }
    }
}

#[tokio::test]
async fn profile_selects_its_named_window_and_derives_the_input_limit() {
    let fixture = Fixture::new(BASE_CONFIG);
    let request = fixture.request();

    let preflight = factory::preflight(&request)
        .await
        .expect("selected model window resolves before provider construction");

    assert_eq!(
        preflight.model_profile.limits,
        ModelLimits::new(32_768, 28_672, 4_096)
    );
}

#[tokio::test]
async fn flat_model_limits_override_the_trusted_default_window() {
    let fixture = Fixture::new(&format!("{CHATGPT_CONFIG}{CHATGPT_FLAT_LIMITS}"));
    let request = fixture.request();

    assert!(request.config.context_window.is_none());
    let preflight = factory::preflight(&request)
        .await
        .expect("flat model limits override the trusted default window");

    assert_eq!(
        preflight.model_profile.limits,
        ModelLimits::new(872_000, 828_400, 128_000)
    );
}

#[tokio::test]
async fn an_explicit_trusted_window_still_fails_when_pinned_by_flat_model_limits() {
    let config = format!("{CHATGPT_CONFIG}{CHATGPT_FLAT_LIMITS}").replace(
        "model = \"gpt-6.1-sol\"",
        "model = \"gpt-6.1-sol\"\ncontext_window = \"872k\"",
    );
    let fixture = Fixture::new(&config);
    let request = fixture.request();

    let error = factory::preflight(&request)
        .await
        .expect_err("explicit named windows cannot override flat model limits");

    assert!(
        matches!(error, FactoryError::ContextWindow { .. }),
        "{error}"
    );
    assert!(
        error.to_string().contains(
            "window `872k` is pinned by flat model limit `models.\"chatgpt/gpt-6.1-sol\".max_input_tokens`; remove that limit to select a named window"
        ),
        "{error}"
    );
}

#[tokio::test]
async fn the_trusted_default_window_is_selected_without_flat_model_limits() {
    let fixture = Fixture::new(CHATGPT_CONFIG);
    let request = fixture.request();

    let preflight = factory::preflight(&request)
        .await
        .expect("the trusted default window remains active without flat model limits");

    assert_eq!(preflight.model_profile.limits.context_tokens, 272_000);
}

#[derive(Debug)]
struct PanicsIfRead;

impl Environment for PanicsIfRead {
    fn value(&self, _name: &str) -> Option<Secret> {
        panic!("unknown context-window validation must precede credential lookup")
    }
}

#[tokio::test]
async fn an_unknown_window_fails_before_credential_lookup() {
    let config = BASE_CONFIG
        .replace("context_window = \"small\"", "context_window = \"missing\"")
        .replace(
            "kind = \"fake\"",
            "kind = \"openai-compatible\"\nbase_url = \"https://api.example.test/v1\"\ncredential = \"env:SMITH_TEST_KEY\"",
        );
    let fixture = Fixture::new(&config);
    let mut request = fixture.request();
    request.credentials = Some(
        CredentialResolver::new("/nonexistent-state").with_environment(Arc::new(PanicsIfRead)),
    );

    let error = factory::preflight(&request)
        .await
        .expect_err("unknown named windows are rejected locally");

    assert!(
        matches!(error, FactoryError::ContextWindow { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("large, small"), "{error}");
}
