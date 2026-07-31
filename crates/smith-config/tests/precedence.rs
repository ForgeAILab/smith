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
use smith_config::model::{ApprovalMode, BackgroundExit};
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
        config
            .provider
            .credential
            .as_ref()
            .expect("a reference")
            .value,
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

    for value in ["keychain:smith/acme", "env:ACME_API_KEY", "file:/keys/acme"] {
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
                .credential
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
    assert!(resolution.config.provider.credential.is_none());
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
            .credential
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
