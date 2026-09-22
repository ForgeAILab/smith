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
