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
use smith_config::model::{AgentPosture, ApprovalMode, BackgroundExit, ProfileUse};
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

#[test]
fn every_higher_layer_beats_every_lower_one_and_reports_itself() {
    const LOWER: u32 = 3;
    const HIGHER: u32 = 7;

    let layers = Layer::all();
    for (index, lower) in layers.iter().enumerate() {
        for higher in &layers[index + 1..] {
            let fixture = Fixture::new();
            let mut scenario = Scenario::default();
            scenario.set_reserve(*lower, LOWER);
            scenario.set_reserve(*higher, HIGHER);

            let resolution = scenario
                .resolve(&fixture)
                .unwrap_or_else(|err| panic!("{lower:?} under {higher:?}: {err}"));
            let resolved = &resolution.config.context.reasoning_reserve;

            assert_eq!(
                resolved.value, HIGHER,
                "{higher:?} should beat {lower:?}, source {}",
                resolved.source
            );
            assert_eq!(
                resolved.source.layer, *higher,
                "{higher:?} should be named as the source, got {}",
                resolved.source
            );
        }
    }
}

#[test]
fn a_command_line_model_beats_the_project_profile_and_explain_says_so() {
    let fixture = Fixture::new();
    let scenario = Scenario {
        cli: Overrides {
            model: Some("cli-model".to_owned()),
            ..Overrides::default()
        },
        ..Scenario::default()
    };

    let resolution = scenario.resolve(&fixture).expect("a resolved run");
    assert_eq!(resolution.config.model.value, "cli-model");
    assert_eq!(resolution.config.model.source.layer, Layer::CommandLine);

    let explanation = resolution.provenance.explain("model").expect("an answer");
    assert_eq!(
        explanation.value,
        SettingValue::Text("cli-model".to_owned())
    );
    assert_eq!(explanation.source.layer, Layer::CommandLine);
    assert_eq!(
        explanation.overridden[0].value,
        SettingValue::Text("example-model".to_owned())
    );
    assert_eq!(explanation.overridden[0].source.layer, Layer::Profile);
    // Provenance points at the file a user can go and edit, not just the layer.
    assert_eq!(
        explanation.overridden[0].source.file.as_deref(),
        Some(fixture.project_root().join(".smith/config.toml").as_path())
    );
    assert_eq!(explanation.overridden[0].source.key, "profiles.work.model");
}

#[test]
fn explain_lists_every_layer_that_was_overridden_highest_first() {
    let fixture = Fixture::new();
    let mut scenario = Scenario::default();
    scenario.set_reserve(Layer::UserFile, 1);
    scenario.set_reserve(Layer::ProjectFile, 2);
    scenario.set_reserve(Layer::Environment, 3);

    let resolution = scenario.resolve(&fixture).expect("a resolved run");
    let explanation = resolution
        .provenance
        .explain("context.reasoning_reserve")
        .expect("an answer");

    assert_eq!(explanation.value, SettingValue::Integer(3));
    assert_eq!(explanation.source.layer, Layer::Environment);
    assert_eq!(explanation.source.key, "SMITH_CONTEXT_REASONING_RESERVE");
    let overridden: Vec<Layer> = explanation
        .overridden
        .iter()
        .map(|entry| entry.source.layer)
        .collect();
    assert_eq!(
        overridden,
        vec![Layer::ProjectFile, Layer::UserFile, Layer::BuiltIn]
    );
}

#[test]
fn explain_refuses_an_unknown_key_and_suggests_the_near_miss() {
    let fixture = Fixture::new();
    let resolution = Scenario::default()
        .resolve(&fixture)
        .expect("a resolved run");

    match resolution.provenance.explain("context.reasoning_reserv") {
        Err(ConfigError::UnknownKey {
            key, suggestions, ..
        }) => {
            assert_eq!(key, "context.reasoning_reserv");
            assert_eq!(suggestions, vec!["context.reasoning_reserve".to_owned()]);
        }
        other => panic!("expected an unknown-key error, got {other:?}"),
    }

    // A setting nobody configured is a different answer from a typo, and the
    // user acts on it differently.
    match resolution.provenance.explain("context.output_reserve") {
        Err(ConfigError::MissingSetting { key, .. }) => {
            assert_eq!(key, "context.output_reserve");
        }
        other => panic!("expected a missing-setting error, got {other:?}"),
    }
}

#[test]
fn every_resolved_field_keeps_the_source_that_supplied_it() {
    let fixture = Fixture::new();
    let resolution = Scenario::default()
        .resolve(&fixture)
        .expect("a resolved run");
    let config = &resolution.config;
    let project_config = fixture.project_root().join(".smith/config.toml");

    assert_eq!(config.profile.as_ref().expect("a profile").value, "work");
    assert_eq!(
        config.profile.as_ref().expect("a profile").source.layer,
        Layer::ProjectFile
    );
    assert_eq!(config.provider.name.value, "acme");
    assert_eq!(config.provider.name.source.layer, Layer::Profile);
    assert_eq!(config.provider.kind.value, "openai-compatible");
    assert_eq!(
        config.provider.kind.source.file.as_deref(),
        Some(project_config.as_path())
    );
    assert_eq!(config.provider.kind.source.key, "providers.acme.kind");
    assert_eq!(
        config.provider.credential().expect("a reference").value,
        "keychain:smith/acme"
    );

    // Smith's own defaults are a layer like any other, and say so.
    assert_eq!(config.limits.max_tool_steps.source.layer, Layer::BuiltIn);
    assert_eq!(config.approval.mode.value, ApprovalMode::Ask);
    assert_eq!(config.background.exit_policy.value, BackgroundExit::Error);
    assert_eq!(
        config.persistence.sessions_dir.value,
        fixture
            .home
            .path()
            .canonicalize()
            .expect("a canonical home")
            .join(".smith/sessions")
    );
}

#[test]
fn model_limits_are_never_invented_and_carry_their_source_when_written() {
    let fixture = Fixture::new();
    let resolution = Scenario::default()
        .resolve(&fixture)
        .expect("a resolved run");
    let limits = &resolution.config.model_limits;
    assert!(limits.context_tokens.is_none());
    assert!(limits.max_input_tokens.is_none());
    assert!(limits.max_output_tokens.is_none());

    let configured = resolve_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[models.\"acme/example-model\"]\ncontext_tokens = 128000\n"
    ))
    .expect("a resolved run");
    let context_tokens = configured
        .config
        .model_limits
        .context_tokens
        .expect("a configured limit");
    assert_eq!(context_tokens.value, 128_000);
    assert_eq!(
        context_tokens.source.key,
        "models.\"acme/example-model\".context_tokens"
    );
    assert!(configured.config.model_limits.max_input_tokens.is_none());
}

#[test]
fn an_unknown_key_in_a_file_names_the_file_the_key_and_the_alternative() {
    let error = resolve_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[context]\noutput_reserv = 10\n"
    ))
    .expect_err("an unknown key");

    match error {
        ConfigError::UnknownKey {
            ref key,
            ref source,
            ref location,
            ref suggestions,
        } => {
            assert_eq!(key, "output_reserv");
            let source = source.as_ref().expect("a file source");
            assert_eq!(source.layer, Layer::ProjectFile);
            assert!(
                source
                    .file
                    .as_ref()
                    .expect("a path")
                    .ends_with(".smith/config.toml"),
                "{source}"
            );
            assert!(location.is_some(), "{error}");
            assert_eq!(suggestions, &vec!["output_reserve".to_owned()]);
        }
        other => panic!("expected an unknown-key error, got {other:?}"),
    }
    assert!(error.to_string().contains("did you mean"), "{error}");
}

#[test]
fn an_invalid_type_in_a_file_names_the_file_and_the_position() {
    let text = format!("{BASE_PROJECT_CONFIG}\n[limits]\nmax_tool_steps = \"lots\"\n");
    let offending_line = text
        .lines()
        .position(|line| line.contains("max_tool_steps"))
        .expect("the offending line")
        + 1;
    let error = resolve_project(&text).expect_err("an invalid type");

    match error {
        ConfigError::Malformed {
            ref path,
            ref location,
            ref message,
        } => {
            assert!(path.ends_with(".smith/config.toml"), "{path:?}");
            assert_eq!(
                location.expect("a position").line as usize,
                offending_line,
                "{error}"
            );
            assert!(message.contains("invalid type"), "{message}");
        }
        other => panic!("expected a malformed-file error, got {other:?}"),
    }
}

#[test]
fn an_unknown_environment_variable_is_refused_with_the_variable_it_meant() {
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    let error = resolve(&fixture.request().with_env([("SMITH_MDOEL", "other")]))
        .expect_err("an unknown variable");

    match error {
        ConfigError::UnknownKey {
            ref key,
            ref suggestions,
            ..
        } => {
            assert_eq!(key, "SMITH_MDOEL");
            assert_eq!(suggestions, &vec!["SMITH_MODEL".to_owned()]);
        }
        other => panic!("expected an unknown-key error, got {other:?}"),
    }
}

#[test]
fn one_setting_named_twice_in_one_layer_is_ambiguous_rather_than_arbitrary() {
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    let error = resolve(
        &fixture
            .request()
            .with_env([("SMITH_MODEL", "one"), ("smith_model", "two")]),
    )
    .expect_err("an ambiguous layer");

    match error {
        ConfigError::Ambiguous {
            ref key,
            ref sources,
        } => {
            assert_eq!(key, "model");
            let named: Vec<&str> = sources.iter().map(|source| source.key.as_str()).collect();
            assert_eq!(named, vec!["SMITH_MODEL", "smith_model"]);
        }
        other => panic!("expected an ambiguity error, got {other:?}"),
    }
}

#[test]
fn an_environment_value_of_the_wrong_type_names_the_variable() {
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    let error = resolve(
        &fixture
            .request()
            .with_env([("SMITH_LIMITS_MAX_TOOL_STEPS", "lots")]),
    )
    .expect_err("an invalid value");

    match error {
        ConfigError::InvalidValue { ref source, .. } => {
            assert_eq!(source.layer, Layer::Environment);
            assert_eq!(source.key, "SMITH_LIMITS_MAX_TOOL_STEPS");
        }
        other => panic!("expected an invalid-value error, got {other:?}"),
    }
}

#[test]
fn a_default_profile_that_does_not_exist_names_the_profiles_that_do() {
    let error = resolve_project(
        r#"
default_profile = "wrok"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
"#,
    )
    .expect_err("an unusable profile");

    match error {
        ConfigError::UnusableReference {
            ref source,
            what,
            ref name,
            ref suggestions,
        } => {
            assert_eq!(what, ReferenceKind::Profile);
            assert_eq!(name, "wrok");
            assert_eq!(source.key, "default_profile");
            assert_eq!(suggestions, &vec!["work".to_owned()]);
        }
        other => panic!("expected an unusable-reference error, got {other:?}"),
    }
}

#[test]
fn a_profile_naming_an_undefined_provider_is_refused_before_anything_starts() {
    let error = resolve_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme-staging"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
"#,
    )
    .expect_err("an unusable provider");

    match error {
        ConfigError::UnusableReference {
            ref source,
            what,
            ref name,
            ..
        } => {
            assert_eq!(what, ReferenceKind::Provider);
            assert_eq!(name, "acme-staging");
            // The diagnostic points at the profile that chose it, in the file
            // it was written in.
            assert_eq!(source.layer, Layer::Profile);
            assert_eq!(source.key, "profiles.work.provider");
        }
        other => panic!("expected an unusable-reference error, got {other:?}"),
    }
}

#[test]
fn a_credential_written_in_plain_text_is_refused_and_a_reference_is_not() {
    let inline = [
        "sk-not-a-real-key",
        "",
        "keychain:",
        "vault:smith/acme",
        "smith/acme",
    ];
    for value in inline {
        let error = resolve_project(&format!(
            r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "{value}"
"#
        ))
        .expect_err("a plaintext secret");
        match error {
            ConfigError::PlaintextSecret { ref source, .. } => {
                assert_eq!(source.key, "providers.acme.credential");
            }
            other => panic!("expected a plaintext-secret error for `{value}`, got {other:?}"),
        }
    }

    for value in [
        "keychain:smith/acme",
        "authfile:chatgpt",
        "env:ACME_API_KEY",
        "file:/keys/acme",
    ] {
        let resolution = resolve_project(&format!(
            r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "{value}"
"#
        ))
        .unwrap_or_else(|err| panic!("`{value}` should resolve: {err}"));
        // The reference is carried as written; nothing here reads its value.
        assert_eq!(
            resolution
                .config
                .provider
                .credential()
                .expect("a reference")
                .value,
            value
        );
    }
}

#[cfg(unix)]
#[test]
fn an_owner_only_user_api_key_resolves_and_every_public_render_is_redacted() {
    const SECRET: &str = "sk-inline-resolution-must-not-render";
    let fixture = Fixture::new();
    fixture.write_private_user(&format!(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
api_key = "{SECRET}"
"#
    ));

    let resolution = resolve(&fixture.request()).expect("an inline-key configuration");
    let api_key = resolution
        .config
        .provider
        .api_key
        .as_ref()
        .expect("a resolved inline key");
    assert_eq!(api_key.value.expose(), SECRET);
    assert!(resolution.config.provider.credential().is_none());
    assert_eq!(api_key.source.layer, Layer::UserFile);
    assert_eq!(api_key.source.key, "providers.acme.api_key");

    let explanation = resolution
        .provenance
        .explain("providers.acme.api_key")
        .expect("inline-key provenance");
    assert_eq!(explanation.value.to_string(), "[redacted]");
    for rendered in [
        format!("{resolution:?}"),
        format!("{:?}", resolution.config),
        format!("{:?}", resolution.provenance),
        format!("{explanation:?}"),
        explanation.value.to_string(),
        format!(
            "{:?}",
            local_inventory(&resolution, &["openai-compatible"]).expect("a local inventory")
        ),
    ] {
        assert!(!rendered.contains(SECRET), "{rendered}");
    }
}

#[cfg(unix)]
#[test]
fn checkpoint_key_sources_are_private_redacted_and_mutually_exclusive() {
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    fixture.write_private_user(&format!("[persistence]\ncheckpoint_key = \"{KEY}\"\n"));

    let resolution = resolve(&fixture.request()).expect("an owner-only checkpoint key");
    let key = resolution
        .config
        .persistence
        .checkpoint_key
        .as_ref()
        .expect("the configured key");
    assert_eq!(key.value.expose(), KEY);
    assert_eq!(key.source.layer, Layer::UserFile);
    let explanation = resolution
        .provenance
        .explain("persistence.checkpoint_key")
        .expect("checkpoint-key provenance");
    assert_eq!(explanation.value.to_string(), "[redacted]");
    for rendered in [
        format!("{resolution:?}"),
        format!("{:?}", resolution.config),
        format!("{explanation:?}"),
    ] {
        assert!(!rendered.contains(KEY), "{rendered}");
    }

    let mut env = BTreeMap::new();
    env.insert("SMITH_CHECKPOINT_KEY".to_owned(), KEY.to_owned());
    let env_resolution = resolve(&fixture.request().with_env(env.clone()))
        .expect("the environment key overrides the inline value");
    assert_eq!(
        env_resolution
            .config
            .persistence
            .checkpoint_key
            .as_ref()
            .expect("environment key")
            .source
            .layer,
        Layer::Environment
    );

    fixture.write_private_user(
        "[persistence]\ncheckpoint_key_credential = \"env:SMITH_CHECKPOINT_SECRET\"\n",
    );
    let error = resolve(&fixture.request().with_env(env))
        .expect_err("direct and referenced checkpoint keys are mutually exclusive");
    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(!error.to_string().contains(KEY), "{error}");
}

#[test]
fn project_checkpoint_key_sources_are_rejected_before_use() {
    const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    for setting in [
        format!("checkpoint_key = \"{KEY}\""),
        "checkpoint_key_credential = \"env:CHECKPOINT_KEY\"".to_owned(),
    ] {
        let fixture = Fixture::new();
        fixture.write_project(&format!(
            "{BASE_PROJECT_CONFIG}\n[persistence]\n{setting}\n"
        ));
        let error = resolve(&fixture.request()).expect_err("project-controlled checkpoint key");
        assert!(matches!(error, ConfigError::InvalidValue { .. }));
        assert!(!format!("{error:?} {error}").contains(KEY));
    }
}

#[test]
fn agent_modes_are_typed_ordered_and_child_presets_stay_read_only() {
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    fixture.write_user(
        r#"
default_agent = "plan"
agent_order = ["plan", "build", "review"]

[agent_modes.audit]
posture = "review"
description = "Evidence-only audit"

[child_agents.inspect]
posture = "plan"
description = "Inspect without mutation"
"#,
    );
    let resolution = resolve(&fixture.request()).expect("typed agent modes");
    assert_eq!(resolution.config.agent.active.value, "plan");
    assert_eq!(
        resolution.config.agent.order.value,
        ["plan", "build", "review"]
    );
    assert_eq!(resolution.config.agent.active_posture(), AgentPosture::Plan);
    assert_eq!(
        resolution.config.agent.modes["audit"].posture.value,
        AgentPosture::Review
    );
    assert!(
        resolution.config.agent.child_presets["inspect"]
            .posture
            .value
            .is_read_only()
    );

    fixture.write_user(
        r#"
[child_agents.unsafe]
posture = "build"
"#,
    );
    let error = resolve(&fixture.request()).expect_err("a write-capable child preset");
    assert!(
        error.to_string().contains("must use a read-only"),
        "{error}"
    );
}

#[test]
fn agent_profiles_inherit_runtime_settings_and_work_on_main_or_child() {
    let resolution = resolve_project(
        r#"
default_profile = "plan"
profile_order = ["work", "plan"]

[profiles.work]
provider = "acme"
model = "example-model"
posture = "build"
use = ["main", "child"]
description = "Implementation profile"
instructions = "Implement and verify the requested change."

[profiles.plan]
extends = "work"
posture = "plan"
instructions = "Inspect and produce an implementation-ready plan."

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"
"#,
    )
    .expect("inherited agent profiles");

    let agent = &resolution.config.agent;
    assert_eq!(agent.profile.name, "plan");
    assert_eq!(agent.active_posture(), AgentPosture::Plan);
    assert_eq!(agent.profile.provider.as_ref().unwrap().value, "acme");
    assert_eq!(agent.profile.model.as_ref().unwrap().value, "example-model");
    assert!(agent.profile.supports(ProfileUse::Main));
    assert!(agent.profile.supports(ProfileUse::Child));
    assert_eq!(agent.profile_order.value, ["work", "plan"]);
    assert_eq!(
        agent.profile.instructions.as_ref().unwrap().value,
        "Inspect and produce an implementation-ready plan."
    );
    assert_eq!(agent.profile.revision.len(), 64);
}

#[test]
fn omitted_profile_order_derives_all_real_main_profiles_and_excludes_legacy_adapters() {
    let resolution = resolve_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"
posture = "build"

[profiles.plan]
extends = "work"
posture = "plan"

[profiles.inspect]
extends = "work"
posture = "review"
use = ["child"]

[agent_modes.audit]
posture = "review"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"
"#,
    )
    .expect("derived profile order");

    assert_eq!(
        resolution.config.agent.profile_order.value,
        ["plan", "work"]
    );
    assert!(
        !resolution
            .config
            .agent
            .profile_order
            .value
            .iter()
            .any(|name| name == "audit" || name == "inspect")
    );
}

#[test]
fn profile_inheritance_cycles_fail_before_runtime_construction() {
    let error = resolve_project(
        r#"
default_profile = "one"

[profiles.one]
extends = "two"
provider = "acme"
model = "example-model"

[profiles.two]
extends = "one"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"
"#,
    )
    .expect_err("cyclic profiles");

    assert!(error.to_string().contains("one -> two -> one"), "{error}");
}

#[test]
fn new_profiles_and_user_legacy_declarations_cannot_claim_the_same_name() {
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    fixture.write_user(
        r#"
[profiles.audit]
extends = "work"
posture = "review"

[agent_modes.audit]
posture = "review"
"#,
    );

    let error = resolve(&fixture.request()).expect_err("ambiguous profile name");
    assert!(matches!(error, ConfigError::Ambiguous { .. }));
    assert!(error.to_string().contains("profiles.audit"), "{error}");
}

#[test]
fn profile_placement_and_cycle_order_fail_closed() {
    let main_only = Fixture::new();
    main_only.write_project(BASE_PROJECT_CONFIG);
    let error = resolve(&main_only.request().with_profile_use(ProfileUse::Child))
        .expect_err("main-only profile selected for a child");
    assert!(
        error
            .to_string()
            .contains("not enabled for child-agent use"),
        "{error}"
    );

    let error = resolve_project(
        r#"
default_profile = "work"
profile_order = ["work", "inspect"]

[profiles.work]
provider = "acme"
model = "example-model"
use = ["main"]

[profiles.inspect]
extends = "work"
posture = "review"
use = ["child"]

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"
"#,
    )
    .expect_err("child-only profile in the main cycle");
    assert!(
        error
            .to_string()
            .contains("profile_order` names child-only profile `inspect`"),
        "{error}"
    );
}

#[test]
fn legacy_modes_and_child_presets_are_visible_as_deprecated_profile_placements() {
    let fixture = Fixture::new();
    fixture.write_project(&format!(
        "{BASE_PROJECT_CONFIG}\n\
         [agent_modes.audit]\nposture = \"review\"\ndescription = \"legacy audit\"\n\
         [child_agents.inspect]\nposture = \"plan\"\ndescription = \"legacy inspection\"\n"
    ));
    let resolution = resolve(&fixture.request()).expect("legacy compatibility adapters");

    let audit = resolution
        .config
        .agent
        .profiles
        .get("audit")
        .expect("legacy main profile");
    assert!(audit.legacy);
    assert_eq!(audit.uses.value, [ProfileUse::Main]);
    let inspect = resolution
        .config
        .agent
        .profiles
        .get("inspect")
        .expect("legacy child profile");
    assert!(inspect.legacy);
    assert_eq!(inspect.uses.value, [ProfileUse::Child]);

    let inventory =
        local_inventory(&resolution, &["openai-compatible"]).expect("legacy inventory entries");
    assert!(inventory.profiles.iter().any(|profile| {
        profile.name == "audit" && profile.legacy && profile.uses == [ProfileUse::Main]
    }));
    assert!(inventory.profiles.iter().any(|profile| {
        profile.name == "inspect" && profile.legacy && profile.uses == [ProfileUse::Child]
    }));
}

#[test]
fn glm_5_2_gets_a_source_explainable_32768_request_budget() {
    let resolution = resolve_project(
        r#"
default_profile = "glm"

[profiles.glm]
provider = "zai"
model = "glm-5.2"

[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "env:ZAI_API_KEY"
"#,
    )
    .expect("a cataloged GLM-5.2 selection");
    assert_eq!(
        resolution
            .config
            .max_output_tokens
            .as_ref()
            .expect("product request budget")
            .value,
        32_768
    );
    let explanation = resolution
        .provenance
        .explain("max_output_tokens")
        .expect("request-budget provenance");
    assert_eq!(explanation.source.layer, Layer::BuiltIn);
    assert!(explanation.source.key.contains("glm-5.2"));
    assert!(explanation.source.key.contains("request_output_tokens"));
}

#[cfg(unix)]
#[test]
fn project_inline_keys_are_refused_without_rendering_the_value() {
    const SECRET: &str = "sk-project-inline-must-not-render";
    for local in [false, true] {
        let fixture = Fixture::new();
        let text = format!(
            r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
api_key = "{SECRET}"
"#
        );
        if local {
            fixture.write_project(BASE_PROJECT_CONFIG);
            fixture.write_project_local(&text);
        } else {
            fixture.write_project(&text);
        }

        let error = resolve(&fixture.request()).expect_err("a project inline key");
        assert!(matches!(error, ConfigError::PlaintextSecret { .. }));
        assert!(!error.to_string().contains(SECRET), "{error}");
        assert!(!format!("{error:?}").contains(SECRET), "{error:?}");
    }
}

#[cfg(unix)]
#[test]
fn unsafe_user_config_files_with_inline_keys_are_refused_without_a_leak() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    const SECRET: &str = "sk-unsafe-file-must-not-render";
    let text = format!(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
api_key = "{SECRET}"
"#
    );

    let permissive = Fixture::new();
    permissive.write_user(&text);
    let path = permissive.home.path().join(".smith/config.toml");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("permissive test config");
    let error = resolve(&permissive.request()).expect_err("a permissive inline-key file");
    assert!(matches!(error, ConfigError::PlaintextSecret { .. }));
    assert!(!format!("{error:?}").contains(SECRET), "{error:?}");

    let linked = Fixture::new();
    let target = linked.home.path().join("actual-config.toml");
    std::fs::write(&target, &text).expect("symlink target");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
        .expect("private target");
    symlink(&target, linked.home.path().join(".smith/config.toml")).expect("user config symlink");
    let error = resolve(&linked.request()).expect_err("a symlinked inline-key file");
    assert!(matches!(error, ConfigError::PlaintextSecret { .. }));
    assert!(!format!("{error:?}").contains(SECRET), "{error:?}");
}

#[cfg(unix)]
#[test]
fn two_credential_sources_and_empty_inline_keys_fail_without_rendering_values() {
    const SECRET: &str = "sk-conflicting-source-must-not-render";
    let fixture = Fixture::new();
    fixture.write_private_user(&format!(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "env:ACME_API_KEY"
api_key = "{SECRET}"
"#
    ));
    let error = resolve(&fixture.request()).expect_err("two credential sources");
    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(error.to_string().contains("credential"));
    assert!(error.to_string().contains("api_key"));
    assert!(!format!("{error:?}").contains(SECRET), "{error:?}");

    fixture.write_private_user(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
api_key = ""
"#,
    );
    let error = resolve(&fixture.request()).expect_err("an empty inline key");
    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(error.to_string().contains("cannot be empty"), "{error}");
}

#[test]
fn an_authorization_header_is_refused_as_a_plaintext_secret() {
    let error = resolve_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"

[providers.acme.headers]
Authorization = "Bearer sk-not-a-real-key"
"#,
    )
    .expect_err("a plaintext secret");

    match error {
        ConfigError::PlaintextSecret { ref source, .. } => {
            assert_eq!(source.key, "providers.acme.headers.Authorization");
        }
        other => panic!("expected a plaintext-secret error, got {other:?}"),
    }
    assert!(error.to_string().contains("credential"), "{error}");
}

#[test]
fn provider_options_must_suit_the_adapter_kind() {
    let error = resolve_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "scripted"
model = "example-model"

[providers.scripted]
kind = "fake"
base_url = "https://api.example.test/v1"
"#,
    )
    .expect_err("an incompatible option");
    match error {
        ConfigError::IncompatibleOption {
            ref source,
            ref kind,
            ..
        } => {
            assert_eq!(kind, "fake");
            assert_eq!(source.key, "providers.scripted.base_url");
        }
        other => panic!("expected an incompatible-option error, got {other:?}"),
    }

    let missing = resolve_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
credential = "keychain:smith/acme"
"#,
    )
    .expect_err("a missing endpoint");
    match missing {
        ConfigError::MissingSetting { ref key, .. } => {
            assert_eq!(key, "providers.acme.base_url");
        }
        other => panic!("expected a missing-setting error, got {other:?}"),
    }
}

#[test]
fn native_google_provider_owns_its_endpoint_and_headers() {
    let resolution = resolve_project(
        r#"
default_profile = "gemini"

[profiles.gemini]
provider = "google"
model = "gemini-3.6-flash"

[providers.google]
kind = "gemini-interactions"
credential = "env:GEMINI_API_KEY"
"#,
    )
    .expect("native Google config without a user endpoint");
    assert_eq!(resolution.config.provider.name.value, "google");
    assert_eq!(resolution.config.provider.base_url, None);

    let endpoint = resolve_project(
        r#"
default_profile = "gemini"
[profiles.gemini]
provider = "google"
model = "gemini-3.6-flash"
[providers.google]
kind = "gemini-interactions"
base_url = "https://proxy.example.test/v1beta"
credential = "env:GEMINI_API_KEY"
"#,
    )
    .expect_err("native Google does not accept endpoint overrides");
    assert!(endpoint.to_string().contains("does not accept `base_url`"));

    let header = resolve_project(
        r#"
default_profile = "gemini"
[profiles.gemini]
provider = "google"
model = "gemini-3.6-flash"
[providers.google]
kind = "gemini-interactions"
credential = "env:GEMINI_API_KEY"
[providers.google.headers]
X-Trace = "enabled"
"#,
    )
    .expect_err("native Google does not accept custom headers");
    assert!(
        header
            .to_string()
            .contains("does not accept custom headers")
    );
}

#[test]
fn compaction_watermarks_must_leave_room_below_the_one_that_triggers() {
    let error = resolve_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[context]\ncompaction_high_watermark_percent = 60\ncompaction_low_watermark_percent = 80\n"
    ))
    .expect_err("an unusable pair of watermarks");

    match error {
        ConfigError::InvalidValue { ref source, .. } => {
            assert_eq!(source.key, "context.compaction_low_watermark_percent");
        }
        other => panic!("expected an invalid-value error, got {other:?}"),
    }
}

#[test]
fn a_run_with_no_selected_model_is_refused_rather_than_guessed() {
    let error = resolve_project(
        r#"
[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
"#,
    )
    .expect_err("an incomplete run");

    match error {
        ConfigError::MissingSetting { ref key, .. } => assert_eq!(key, "provider"),
        other => panic!("expected a missing-setting error, got {other:?}"),
    }
}

#[test]
fn the_project_is_found_by_walking_up_from_a_nested_directory() {
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    let nested = fixture.project.path().join("crates/deep/src");
    std::fs::create_dir_all(&nested).expect("a nested directory");

    let resolution = resolve(&ResolveRequest::new(&nested).with_home_dir(fixture.home.path()))
        .expect("a resolved run");

    assert_eq!(
        resolution.layout.project_root.as_deref(),
        Some(fixture.project_root().as_path())
    );
    assert_eq!(
        resolution.layout.project_dir.as_deref(),
        Some(fixture.project_root().join(".smith").as_path())
    );
    assert_eq!(resolution.config.model.value, "example-model");
}

#[test]
fn the_user_root_is_never_adopted_as_a_project() {
    let fixture = Fixture::new();
    fixture.write_user(BASE_PROJECT_CONFIG);
    let inside_home = fixture.home.path().join("notes");
    std::fs::create_dir_all(&inside_home).expect("a directory inside the home root");

    let resolution = resolve(&ResolveRequest::new(&inside_home).with_home_dir(fixture.home.path()))
        .expect("a resolved run");

    // `~/.smith` is user state. Adopting it as the project layer would turn
    // every user setting into a project setting for anything opened at home.
    assert_eq!(resolution.layout.project_root, None);
    assert_eq!(resolution.layout.project_dir, None);
    assert_eq!(resolution.config.model.source.layer, Layer::Profile);
    assert_eq!(
        resolution.config.model.source.file.as_deref(),
        Some(
            fixture
                .home
                .path()
                .canonicalize()
                .expect("a canonical home")
                .join(".smith/config.toml")
                .as_path()
        )
    );
    assert_eq!(
        resolution
            .layout
            .files
            .iter()
            .map(|file| file.layer)
            .collect::<Vec<_>>(),
        vec![Layer::UserFile]
    );
}

#[test]
fn a_project_local_file_layers_over_the_committed_one() {
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    fixture.write_project_local("[profiles.work]\nmodel = \"local-model\"\n");

    let resolution = resolve(&fixture.request()).expect("a resolved run");
    assert_eq!(resolution.config.model.value, "local-model");
    assert_eq!(resolution.config.model.source.layer, Layer::Profile);
    assert_eq!(
        resolution.config.model.source.file.as_deref(),
        Some(
            fixture
                .project_root()
                .join(".smith/config.local.toml")
                .as_path()
        )
    );
    assert_eq!(
        resolution
            .layout
            .files
            .iter()
            .map(|file| file.layer)
            .collect::<Vec<_>>(),
        vec![Layer::ProjectFile, Layer::ProjectLocalFile]
    );
}

#[test]
fn a_missing_project_still_resolves_from_the_user_root() {
    let fixture = Fixture::new();
    fixture.write_user(BASE_PROJECT_CONFIG);
    let bare = tempfile::tempdir().expect("a directory with no project");

    let resolution = resolve(&ResolveRequest::new(bare.path()).with_home_dir(fixture.home.path()))
        .expect("a resolved run");

    assert_eq!(resolution.layout.project_root, None);
    assert_eq!(resolution.config.provider.name.value, "acme");
    assert_eq!(resolution.config.provider.name.source.layer, Layer::Profile);
}

#[test]
fn nothing_in_the_project_is_executed_to_resolve_configuration() {
    // Declarative project settings may be read before the project is trusted,
    // so resolution must succeed here — and the shell-looking values must
    // arrive as the literal strings they are.
    let fixture = Fixture::new();
    fixture.write_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "file:/keys/acme"

[providers.acme.headers]
X-Trace = "$(id)"
"#,
    );

    let resolution = resolve(&fixture.request()).expect("a resolved run");
    assert_eq!(
        resolution.config.provider.headers["X-Trace"].value,
        "$(id)".to_owned()
    );
    assert_eq!(
        resolution
            .config
            .provider
            .credential()
            .expect("a reference")
            .value,
        "file:/keys/acme"
    );
}

#[test]
fn a_session_override_beats_everything_including_a_flag() {
    let fixture = Fixture::new();
    let scenario = Scenario {
        env: BTreeMap::from([("SMITH_MODEL".to_owned(), "env-model".to_owned())]),
        cli: Overrides {
            model: Some("cli-model".to_owned()),
            ..Overrides::default()
        },
        session: Overrides {
            model: Some("session-model".to_owned()),
            ..Overrides::default()
        },
        ..Scenario::default()
    };

    let resolution = scenario.resolve(&fixture).expect("a resolved run");
    assert_eq!(resolution.config.model.value, "session-model");
    assert_eq!(resolution.config.model.source.layer, Layer::SessionOverride);

    let explanation = resolution.provenance.explain("model").expect("an answer");
    let layers: Vec<Layer> = explanation
        .overridden
        .iter()
        .map(|entry| entry.source.layer)
        .collect();
    assert_eq!(
        layers,
        vec![Layer::CommandLine, Layer::Environment, Layer::Profile]
    );
}

#[test]
fn reasoning_defaults_and_exact_controls_are_typed_and_source_explainable() {
    let fixture = Fixture::new();
    fixture.write_project(&format!(
        r#"{BASE_PROJECT_CONFIG}

[profiles.work.reasoning]
enabled = true
effort = "high"

[models."acme/example-model".reasoning]
toggle = true
mandatory = false
efforts = ["none", "low", "high"]
default_enabled = true
default_effort = "low"
dialect = "openai-effort"
"#
    ));

    let resolution = resolve(&fixture.request().with_session(Overrides {
        reasoning_effort: Some("low".to_owned()),
        ..Overrides::default()
    }))
    .expect("a resolved reasoning policy");

    assert!(
        resolution
            .config
            .reasoning
            .enabled
            .expect("profile state")
            .value
    );
    let effort = resolution.config.reasoning.effort.expect("session effort");
    assert_eq!(effort.value, "low");
    assert_eq!(effort.source.layer, Layer::SessionOverride);
    assert_eq!(
        resolution
            .config
            .model_reasoning
            .efforts
            .expect("advertised efforts")
            .value,
        vec!["none", "low", "high"]
    );
    assert_eq!(
        resolution
            .config
            .model_reasoning
            .dialect
            .expect("exact dialect")
            .value
            .as_str(),
        "openai-effort"
    );

    let explanation = resolution
        .provenance
        .explain("reasoning.effort")
        .expect("reasoning provenance");
    assert_eq!(explanation.source.layer, Layer::SessionOverride);
    assert!(
        explanation
            .overridden
            .iter()
            .any(|entry| entry.source.layer == Layer::Profile)
    );
}

#[test]
fn an_invocation_effort_outranks_the_profile_and_the_environment() {
    let fixture = Fixture::new();
    let scenario = Scenario {
        profile: vec!["[profiles.work.reasoning]\neffort = \"low\"\n".to_owned()],
        env: BTreeMap::from([("SMITH_REASONING_EFFORT".to_owned(), "medium".to_owned())]),
        cli: Overrides {
            reasoning_effort: Some("high".to_owned()),
            ..Overrides::default()
        },
        ..Scenario::default()
    };

    let resolution = scenario.resolve(&fixture).expect("a resolved run");
    let effort = resolution
        .config
        .reasoning
        .effort
        .clone()
        .expect("a resolved effort");
    assert_eq!(effort.value, "high");
    assert_eq!(effort.source.layer, Layer::CommandLine);
    // The flag is the control's name, not the key path, so the diagnostic
    // names an option the user can actually type.
    assert_eq!(effort.source.to_string(), "command-line flag `--effort`");

    let explanation = resolution
        .provenance
        .explain("reasoning.effort")
        .expect("reasoning provenance");
    assert_eq!(explanation.source.layer, Layer::CommandLine);
    assert_eq!(
        explanation
            .overridden
            .iter()
            .map(|entry| entry.source.layer)
            .collect::<Vec<_>>(),
        vec![Layer::Environment, Layer::Profile]
    );
    // The overridden profile entry stays visible and unchanged.
    assert_eq!(
        explanation
            .overridden
            .iter()
            .find(|entry| entry.source.layer == Layer::Profile)
            .map(|entry| entry.value.clone()),
        Some(SettingValue::Text("low".to_owned()))
    );
}

#[test]
fn an_in_session_effort_still_outranks_an_invocation_effort() {
    let fixture = Fixture::new();
    let scenario = Scenario {
        cli: Overrides {
            reasoning_effort: Some("high".to_owned()),
            ..Overrides::default()
        },
        session: Overrides {
            reasoning_effort: Some("low".to_owned()),
            ..Overrides::default()
        },
        ..Scenario::default()
    };

    let resolution = scenario.resolve(&fixture).expect("a resolved run");
    let effort = resolution
        .config
        .reasoning
        .effort
        .clone()
        .expect("a resolved effort");
    assert_eq!(effort.value, "low");
    assert_eq!(effort.source.layer, Layer::SessionOverride);
}

#[test]
fn an_invocation_effort_leaves_the_rest_of_the_profile_alone() {
    let fixture = Fixture::new();
    let scenario = Scenario {
        profile: vec!["[profiles.work.reasoning]\nenabled = true\neffort = \"low\"\n".to_owned()],
        cli: Overrides {
            profile: Some("work".to_owned()),
            reasoning_effort: Some("high".to_owned()),
            ..Overrides::default()
        },
        ..Scenario::default()
    };

    let resolution = scenario.resolve(&fixture).expect("a resolved run");
    assert_eq!(resolution.config.profile.expect("a profile").value, "work");
    assert_eq!(resolution.config.provider.name.value, "acme");
    assert_eq!(resolution.config.model.value, "example-model");
    assert_eq!(resolution.config.model.source.layer, Layer::Profile);
    let enabled = resolution
        .config
        .reasoning
        .enabled
        .expect("the profile's thinking state");
    assert!(enabled.value);
    assert_eq!(enabled.source.layer, Layer::Profile);
    assert_eq!(
        resolution
            .config
            .reasoning
            .effort
            .expect("the flag's effort")
            .value,
        "high"
    );
}

#[test]
fn a_profile_selected_by_a_flag_replaces_the_default_profile() {
    let fixture = Fixture::new();
    fixture.write_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[profiles.review]\nprovider = \"acme\"\nmodel = \"review-model\"\n"
    ));

    let resolution = resolve(&fixture.request().with_cli(Overrides {
        profile: Some("review".to_owned()),
        ..Overrides::default()
    }))
    .expect("a resolved run");

    assert_eq!(
        resolution.config.profile.expect("a profile").value,
        "review"
    );
    assert_eq!(resolution.config.model.value, "review-model");
    assert_eq!(resolution.config.model.source.key, "profiles.review.model");
}

#[test]
fn resolution_reads_only_the_files_it_reports() {
    let fixture = Fixture::new();
    fixture.write_user("[context]\ncapability_budget = 12000\n");
    fixture.write_project(BASE_PROJECT_CONFIG);

    let resolution = resolve(&fixture.request()).expect("a resolved run");
    let reported: Vec<&Path> = resolution
        .layout
        .files
        .iter()
        .map(|file| file.path.as_path())
        .collect();

    assert_eq!(reported.len(), 2);
    assert!(reported[0].ends_with(".smith/config.toml"));
    assert_eq!(
        resolution
            .config
            .context
            .capability_budget
            .expect("a budget")
            .source
            .layer,
        Layer::UserFile
    );
}

/// A project declaring `acme` with `settings` appended to the provider table.
fn resolve_provider_with(settings: &str) -> Result<Resolution, ConfigError> {
    resolve_project(&format!(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
{settings}
"#
    ))
}

#[test]
fn a_credential_pool_resolves_in_declared_order_with_its_own_provenance() {
    let resolution = resolve_provider_with(
        r#"credentials = ["keychain:smith/personal", "keychain:smith/work"]"#,
    )
    .expect("a resolved pool");
    let provider = &resolution.config.provider;

    let references: Vec<&str> = provider
        .credentials
        .iter()
        .map(|entry| entry.value.as_str())
        .collect();
    assert_eq!(
        references,
        ["keychain:smith/personal", "keychain:smith/work"]
    );
    // The first entry is where a session with no persisted choice starts.
    assert_eq!(
        provider.credential().expect("an active member").value,
        "keychain:smith/personal"
    );
    assert!(provider.has_pool());
    for entry in &provider.credentials {
        assert_eq!(entry.source.key, "providers.acme.credentials");
        assert_eq!(entry.source.layer, Layer::ProjectFile);
    }
}

#[test]
fn a_single_credential_resolves_as_a_pool_of_one() {
    let resolution =
        resolve_provider_with(r#"credential = "env:ACME_API_KEY""#).expect("a resolved provider");
    let provider = &resolution.config.provider;

    // The legacy spelling needs no migration and produces no warning: it is
    // the same declaration, for one account.
    assert_eq!(provider.credentials.len(), 1);
    assert_eq!(
        provider.credential().expect("an active member").value,
        "env:ACME_API_KEY"
    );
    assert_eq!(
        provider.credentials[0].source.key,
        "providers.acme.credential"
    );
    // One account is not a pool: there is nowhere to rotate to.
    assert!(!provider.has_pool());
}

#[test]
fn an_unparseable_pool_entry_fails_resolution_by_position_without_quoting_it() {
    const PASTED: &str = "sk-live-4kQm2ZpX8vRt7nLb1cWs9aYe";
    let error = resolve_provider_with(&format!(
        r#"credentials = ["keychain:smith/personal", "{PASTED}"]"#
    ))
    .expect_err("an unparseable pool entry");

    let rendered = format!("{error} {error:?}");
    // The offending entry is identified by its position, never by its value:
    // a reference rejected for looking like a pasted key must not be echoed
    // into an error message, a log, or a terminal.
    assert!(rendered.contains("entry 2 of `credentials`"), "{rendered}");
    assert!(!rendered.contains(PASTED), "{rendered}");
    match error {
        ConfigError::PlaintextSecret { ref source, .. }
        | ConfigError::InvalidValue { ref source, .. } => {
            assert_eq!(source.key, "providers.acme.credentials");
        }
        other => panic!("expected a sourced credential error, got {other:?}"),
    }
}

#[test]
fn a_duplicate_pool_entry_is_rejected_rather_than_collapsed() {
    let error = resolve_provider_with(r#"credentials = ["keychain:smith/a", "keychain:smith/a"]"#)
        .expect_err("a duplicate pool entry");

    let rendered = format!("{error}");
    assert!(rendered.contains("keychain:smith/a"), "{rendered}");
    assert!(rendered.contains("more than once"), "{rendered}");
}

#[test]
fn declaring_both_credential_spellings_is_a_contradiction() {
    let error = resolve_provider_with(
        "credential = \"env:ONE\"\ncredentials = [\"env:TWO\", \"env:THREE\"]",
    )
    .expect_err("both spellings");

    // There is no defensible order to splice the single entry into the list,
    // so resolution refuses rather than guessing which account gets billed.
    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(format!("{error}").contains("choose one spelling"));
}

#[test]
fn an_empty_pool_reads_as_no_declaration() {
    // The config round-trips through serde before provenance sees it, and an
    // empty vector is skipped there, so `credentials = []` and an omitted key
    // are the same input by the time resolution runs. This test pins that
    // equivalence so it is a documented property rather than a surprise.
    let resolution = resolve_provider_with("credentials = []").expect("an empty pool");
    assert!(resolution.config.provider.credentials.is_empty());
    assert!(resolution.config.provider.credential().is_none());
}

#[test]
fn a_pool_and_an_inline_key_remain_mutually_exclusive() {
    let fixture = Fixture::new();
    fixture.write_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credentials = ["keychain:smith/a", "keychain:smith/b"]
api_key = "sk-inline"
"#,
    );
    let error = resolve(&fixture.request()).expect_err("a pool beside an inline key");
    // Project files may not carry an inline key at all, which is caught first.
    assert!(matches!(error, ConfigError::PlaintextSecret { .. }));
}

#[test]
fn the_rotation_threshold_resolves_and_is_bounded_to_a_percentage() {
    let resolution = resolve_provider_with(
        "credentials = [\"keychain:smith/a\", \"keychain:smith/b\"]\nrotate_at_percent = 90",
    )
    .expect("a resolved threshold");
    let threshold = resolution
        .config
        .provider
        .rotate_at_percent
        .expect("a threshold");
    assert_eq!(threshold.value, 90);
    assert_eq!(threshold.source.key, "providers.acme.rotate_at_percent");

    for out_of_range in ["0", "101", "255"] {
        let error = resolve_provider_with(&format!(
            "credentials = [\"keychain:smith/a\", \"keychain:smith/b\"]\nrotate_at_percent = {out_of_range}"
        ))
        .expect_err("a threshold outside a percentage");
        assert!(matches!(error, ConfigError::InvalidValue { .. }));
    }
}

#[test]
fn a_rotation_threshold_without_a_pool_is_rejected() {
    let error =
        resolve_provider_with("credential = \"keychain:smith/only\"\nrotate_at_percent = 90")
            .expect_err("a threshold with nowhere to rotate");

    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(format!("{error}").contains("another member"));
}

/// A project whose config is the base plus one `[mcp.servers.*]` block.
fn resolve_mcp_project(text: &str) -> Result<Resolution, ConfigError> {
    resolve_project(&format!("{BASE_PROJECT_CONFIG}\n{text}"))
}

#[test]
fn a_declared_server_resolves_with_its_transport_and_defaults() {
    let resolution = resolve_mcp_project(
        r#"
[mcp.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "keychain:smith/github" }
"#,
    )
    .expect("a declared server");

    let server = &resolution.config.mcp.servers["github"];
    assert_eq!(server.name, "github");
    assert_eq!(server.transport.as_str(), "stdio");
    assert_eq!(
        server.transport.args(),
        ["-y", "@modelcontextprotocol/server-github"]
    );
    // Omitting `enabled` leaves the server on, and says so from the built-in
    // layer rather than from nowhere.
    assert!(server.enabled.value);
    assert_eq!(server.enabled.source.layer, Layer::BuiltIn);
    assert_eq!(
        server.env["GITHUB_TOKEN"].value.credential(),
        Some("keychain:smith/github"),
        "a reference stays a reference rather than being read as a literal"
    );
    assert_eq!(resolution.config.mcp.enabled().count(), 1);
}

#[test]
fn server_declaration_reports_its_winning_layer() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
[mcp.servers.github]
command = "npx"
args = ["-y", "server-github"]
"#,
    );
    fixture.write_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[mcp.servers.github]\ncommand = \"./scripts/github-mcp\"\n"
    ));

    let resolution = resolve(&fixture.request()).expect("a layered declaration");
    let server = &resolution.config.mcp.servers["github"];
    let smith_config::resolve::ResolvedMcpTransport::Stdio { command, args } = &server.transport
    else {
        panic!("a stdio server");
    };

    assert_eq!(command.value, "./scripts/github-mcp");
    assert_eq!(command.source.layer, Layer::ProjectFile);
    // The layer that lost still supplies what the winner did not say.
    assert_eq!(
        args.as_ref().expect("inherited arguments").source.layer,
        Layer::UserFile
    );

    let explanation = resolution
        .provenance
        .explain("mcp.servers.github.command")
        .expect("an explained declaration");
    assert_eq!(explanation.source.layer, Layer::ProjectFile);
    assert_eq!(
        explanation
            .overridden
            .iter()
            .map(|entry| entry.source.layer)
            .collect::<Vec<_>>(),
        vec![Layer::UserFile],
        "the overridden layer stays visible as the explanation"
    );
}

#[test]
fn literal_secret_in_project_config_is_rejected() {
    const TOKEN: &str = "ghp_this_value_must_never_be_reproduced";

    let error = resolve_mcp_project(&format!(
        "[mcp.servers.github]\ncommand = \"npx\"\nenv = {{ GITHUB_TOKEN = \"{TOKEN}\" }}\n"
    ))
    .expect_err("a token written into a repository file");

    let ConfigError::PlaintextSecret { source, message } = &error else {
        panic!("expected a plaintext-secret diagnostic, got {error}");
    };
    assert_eq!(source.key, "mcp.servers.github.env.GITHUB_TOKEN");
    assert!(
        !format!("{error}").contains(TOKEN),
        "the diagnostic must name the key without reproducing the value: {message}"
    );
}

#[cfg(unix)]
#[test]
fn a_literal_in_owner_only_user_config_is_accepted_and_kept_redaction_safe() {
    const TOKEN: &str = "ghp_owner_only_value";

    let fixture = Fixture::new();
    fixture.write_private_user(&format!(
        "[mcp.servers.github]\ncommand = \"npx\"\nenv = {{ GITHUB_TOKEN = \"{TOKEN}\" }}\n"
    ));
    fixture.write_project(BASE_PROJECT_CONFIG);

    let resolution = resolve(&fixture.request()).expect("an owner-only literal");
    let value = &resolution.config.mcp.servers["github"].env["GITHUB_TOKEN"];
    assert!(
        value.value.credential().is_none(),
        "a literal is not a reference"
    );
    // It reaches the child process, and nothing else: neither the explanation
    // ledger nor a debug rendering may reproduce it.
    let explained = resolution
        .provenance
        .explain("mcp.servers.github.env.GITHUB_TOKEN")
        .expect("an explained variable");
    assert!(!format!("{}", explained.value).contains(TOKEN));
    assert!(!format!("{value:?}").contains(TOKEN));
}

#[test]
fn an_ordinary_project_environment_value_is_not_mistaken_for_a_secret() {
    let resolution = resolve_mcp_project(
        "[mcp.servers.docs]\ncommand = \"docs-mcp\"\nenv = { DOCS_ROOT = \"./docs\" }\n",
    )
    .expect("a non-credential variable");
    assert!(
        resolution.config.mcp.servers["docs"]
            .env
            .contains_key("DOCS_ROOT"),
        "a variable whose name claims no credential stays usable in a project file"
    );
}

#[test]
fn a_disabled_server_is_visible_but_not_used() {
    let resolution =
        resolve_mcp_project("[mcp.servers.github]\ncommand = \"npx\"\nenabled = false\n")
            .expect("a disabled server");

    let server = &resolution.config.mcp.servers["github"];
    assert!(!server.enabled.value);
    assert_eq!(server.enabled.source.layer, Layer::ProjectFile);
    assert_eq!(resolution.config.mcp.enabled().count(), 0);
}

#[test]
fn a_declaration_must_name_exactly_one_transport() {
    let both = resolve_mcp_project(
        "[mcp.servers.github]\ncommand = \"npx\"\nurl = \"https://mcp.example.test/v1\"\n",
    )
    .expect_err("two transports");
    assert!(matches!(both, ConfigError::InvalidValue { .. }));
    assert!(format!("{both}").contains("two transports"));

    let neither =
        resolve_mcp_project("[mcp.servers.github]\nargs = [\"-y\"]\n").expect_err("no transport");
    let ConfigError::MissingSetting { key, .. } = &neither else {
        panic!("expected a missing-setting diagnostic, got {neither}");
    };
    assert_eq!(key, "mcp.servers.github.command");
}

#[test]
fn a_url_naming_no_supported_transport_is_rejected() {
    let error = resolve_mcp_project("[mcp.servers.remote]\nurl = \"ftp://mcp.example.test/v1\"\n")
        .expect_err("an unknown transport");
    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(format!("{error}").contains("http"));
}

#[test]
fn a_server_name_no_provider_would_accept_is_rejected() {
    for name in ["git.hub", "a__b", ""] {
        let error = resolve_mcp_project(&format!("[mcp.servers.\"{name}\"]\ncommand = \"npx\"\n"))
            .unwrap_err();
        assert!(
            matches!(error, ConfigError::InvalidValue { .. }),
            "`{name}` should be refused, got {error}"
        );
    }

    let long = "s".repeat(49);
    let error = resolve_mcp_project(&format!("[mcp.servers.{long}]\ncommand = \"npx\"\n"))
        .expect_err("an over-long name");
    assert!(format!("{error}").contains("48 characters"));
}

#[test]
fn a_remote_server_resolves_its_endpoint_credential_and_headers() {
    let resolution = resolve_mcp_project(
        "[mcp.servers.remote]\nurl = \"https://mcp.example.test/v1\"\n\
         credential = \"keychain:smith/remote\"\nheaders = { X-Tenant = \"acme\" }\n",
    )
    .expect("a remote server");

    let server = &resolution.config.mcp.servers["remote"];
    assert_eq!(server.transport.as_str(), "http");
    let smith_config::resolve::ResolvedMcpTransport::StreamableHttp {
        url,
        credential,
        headers,
    } = &server.transport
    else {
        panic!("a remote server");
    };
    assert_eq!(url.value, "https://mcp.example.test/v1");
    assert_eq!(
        credential.as_ref().expect("a bearer credential").value,
        "keychain:smith/remote"
    );
    assert_eq!(headers["X-Tenant"].value.credential(), None);
    // The credential is sent as a header, so it is part of what the user has
    // to approve — by name, never by value.
    assert_eq!(
        server.transport.header_names(),
        vec!["Authorization".to_owned(), "X-Tenant".to_owned()]
    );
}

#[test]
fn an_authorization_header_written_in_plain_text_is_refused() {
    const TOKEN: &str = "Bearer sk-this-must-never-be-reproduced";

    let error = resolve_mcp_project(&format!(
        "[mcp.servers.remote]\nurl = \"https://mcp.example.test/v1\"\n\
         headers = {{ Authorization = \"{TOKEN}\" }}\n"
    ))
    .expect_err("a token written into a repository file");

    let ConfigError::PlaintextSecret { source, .. } = &error else {
        panic!("expected a plaintext-secret diagnostic, got {error}");
    };
    assert_eq!(source.key, "mcp.servers.remote.headers.Authorization");
    assert!(!format!("{error}").contains("sk-this-must"), "{error}");
}

#[test]
fn an_option_the_chosen_transport_cannot_use_is_refused() {
    // Silently ignoring a declared credential is how a user comes to believe a
    // local server is authenticated when nothing was ever sent.
    let local = resolve_mcp_project(
        "[mcp.servers.docs]\ncommand = \"docs-mcp\"\ncredential = \"keychain:smith/docs\"\n",
    )
    .expect_err("a credential a local server cannot send");
    assert!(
        format!("{local}").contains("no use for `credential`"),
        "{local}"
    );

    let remote = resolve_mcp_project(
        "[mcp.servers.remote]\nurl = \"https://mcp.example.test/v1\"\nargs = [\"-y\"]\n",
    )
    .expect_err("arguments a remote server cannot take");
    assert!(
        format!("{remote}").contains("no use for `args`"),
        "{remote}"
    );

    let remote_env = resolve_mcp_project(
        "[mcp.servers.remote]\nurl = \"https://mcp.example.test/v1\"\nenv = { A = \"b\" }\n",
    )
    .expect_err("an environment a remote server cannot receive");
    assert!(
        format!("{remote_env}").contains("no use for `env`"),
        "{remote_env}"
    );
}
