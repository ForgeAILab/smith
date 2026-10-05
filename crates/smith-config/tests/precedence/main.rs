//! Layering, provenance, discovery, and diagnostics, from the outside.
//!
//! Precedence is the one rule every other configuration behavior rests on, so
//! it is tested exhaustively rather than by sample: every ordered pair of
//! layers is built as a real fixture and the higher one is required to win and
//! to say that it won.
//!
//! Every fixture uses its own temporary project and its own temporary user
//! root, and the environment is passed in rather than set, so the suite never
//! reads a developer's real `~/.smith` and never mutates process-wide state.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smith_config::inventory::local_inventory;
use smith_config::model::{
    AgentPosture, ApprovalMode, AutoApprovalMount, AutoApprovalOperation, AutoApprovalPermission,
    AutoApprovalRisk, BackgroundExit, ProfileUse,
};
use smith_config::resolve::{
    ConfigError, Layer, Overrides, ReferenceKind, Resolution, ResolveRequest, SettingValue, resolve,
};
use tempfile::TempDir;

/// The provider, profile, and model every scenario starts from.
const BASE_PROJECT_CONFIG: &str = r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"
"#;

/// A project and a user root that exist only for one test.
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
        std::fs::create_dir_all(fixture.home.path().join(".smith")).expect("a user dir");
        std::fs::create_dir_all(fixture.project.path().join(".smith")).expect("a project dir");
        fixture
    }

    fn write_user(&self, text: &str) {
        std::fs::write(self.home.path().join(".smith/config.toml"), text).expect("a user config");
    }

    #[cfg(unix)]
    fn write_private_user(&self, text: &str) {
        use std::os::unix::fs::PermissionsExt;

        let path = self.home.path().join(".smith/config.toml");
        std::fs::write(&path, text).expect("a user config");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only user config");
    }

    fn write_project(&self, text: &str) {
        std::fs::write(self.project.path().join(".smith/config.toml"), text)
            .expect("a project config");
    }

    fn write_project_local(&self, text: &str) {
        std::fs::write(self.project.path().join(".smith/config.local.toml"), text)
            .expect("a project-local config");
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path()).with_home_dir(self.home.path())
    }

    fn project_root(&self) -> PathBuf {
        self.project
            .path()
            .canonicalize()
            .expect("a canonical project root")
    }
}

/// One configured run, assembled layer by layer.
#[derive(Default)]
struct Scenario {
    user: Vec<String>,
    project: Vec<String>,
    project_local: Vec<String>,
    profile: Vec<String>,
    env: BTreeMap<String, String>,
    cli: Overrides,
    session: Overrides,
}

impl Scenario {
    /// Sets `context.reasoning_reserve` in `layer`.
    ///
    /// The setting is the one every layer can address, which is what makes an
    /// exhaustive pair table possible. The built-in layer is set by *not*
    /// setting it anywhere, so it is a deliberate no-op here.
    fn set_reserve(&mut self, layer: Layer, value: u32) {
        let table = format!("[context]\nreasoning_reserve = {value}\n");
        match layer {
            Layer::BuiltIn => {}
            Layer::UserFile => self.user.push(table),
            Layer::ProjectFile => self.project.push(table),
            Layer::ProjectLocalFile => self.project_local.push(table),
            Layer::Profile => self.profile.push(format!(
                "[profiles.work.context]\nreasoning_reserve = {value}\n"
            )),
            Layer::Environment => {
                self.env.insert(
                    "SMITH_CONTEXT_REASONING_RESERVE".to_owned(),
                    value.to_string(),
                );
            }
            Layer::CommandLine => self.cli.reasoning_reserve = Some(value),
            Layer::SessionOverride => self.session.reasoning_reserve = Some(value),
        }
    }

    fn resolve(&self, fixture: &Fixture) -> Result<Resolution, ConfigError> {
        if !self.user.is_empty() {
            fixture.write_user(&self.user.join("\n"));
        }
        let mut project = vec![BASE_PROJECT_CONFIG.to_owned()];
        project.extend(self.project.clone());
        project.extend(self.profile.clone());
        fixture.write_project(&project.join("\n"));
        if !self.project_local.is_empty() {
            fixture.write_project_local(&self.project_local.join("\n"));
        }

        resolve(
            &fixture
                .request()
                .with_env(self.env.clone())
                .with_cli(self.cli.clone())
                .with_session(self.session.clone()),
        )
    }
}

/// A project whose config is exactly `text`, resolved with no other input.
fn resolve_project(text: &str) -> Result<Resolution, ConfigError> {
    let fixture = Fixture::new();
    fixture.write_project(text);
    resolve(&fixture.request())
}

mod approval;
mod credentials;
mod diagnostics;
mod layering;
mod mcp;
mod profiles;
