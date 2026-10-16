//! Whether an installed coding agent runs its own tools, and who may say so.

use smith_config::resolve::{ConfigError, ResolveRequest, resolve};
use tempfile::TempDir;

struct Fixture {
    home: TempDir,
    project: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            home: tempfile::tempdir().expect("a home root"),
            project: tempfile::tempdir().expect("a project root"),
        };
        std::fs::create_dir_all(fixture.home.path().join(".smith")).expect("a user config dir");
        std::fs::create_dir_all(fixture.project.path().join(".smith"))
            .expect("a project config dir");
        fixture
    }

    fn write_user(&self, text: &str) {
        std::fs::write(self.home.path().join(".smith/config.toml"), text).expect("a user config");
    }

    fn write_project(&self, text: &str) {
        std::fs::write(self.project.path().join(".smith/config.toml"), text)
            .expect("a project config");
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path()).with_home_dir(self.home.path())
    }
}

const SELECTION: &str = r#"
default_profile = "cc"

[profiles.cc]
provider = "local"
model = "cli/claude-code/opus"

[providers.local]
kind = "fake"
"#;

#[test]
fn an_installed_agent_runs_its_own_tools_by_default() {
    let fixture = Fixture::new();
    fixture.write_user(SELECTION);
    let resolution = resolve(&fixture.request()).expect("a ready config");
    let harness = resolution.config.harness.expect("an installed agent");
    assert!(harness.allow_own_tools.value);
    assert_eq!(
        harness.allow_own_tools.source.to_string(),
        "built-in default `harness.claude-code.allow_own_tools`"
    );
}

#[test]
fn the_owner_can_withhold_the_cli_s_own_tools() {
    let fixture = Fixture::new();
    fixture.write_user(&format!(
        "{SELECTION}\n[harness.claude-code]\nallow_own_tools = false\n"
    ));
    let resolution = resolve(&fixture.request()).expect("a ready config");
    let harness = resolution.config.harness.expect("an installed agent");
    assert!(!harness.allow_own_tools.value);
}

#[test]
fn a_project_cannot_set_the_cli_s_own_tools_either_way() {
    for value in [true, false] {
        let fixture = Fixture::new();
        fixture.write_user(SELECTION);
        fixture.write_project(&format!(
            "[harness.claude-code]\nallow_own_tools = {value}\n"
        ));
        let error = resolve(&fixture.request()).expect_err("a project-layer harness setting");
        assert!(matches!(error, ConfigError::InvalidValue { .. }), "{value}");
        let diagnostic = error.to_string();
        assert!(
            diagnostic.contains("harness.claude-code.allow_own_tools"),
            "{value}: {diagnostic}"
        );
    }
}
