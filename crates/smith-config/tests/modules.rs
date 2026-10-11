//! Module selection, alias precedence, availability, and user-edit integration.

use std::collections::BTreeMap;
use std::fs;

use smith_config::model::{ConfigFile, ModuleSection};
use smith_config::resolve::{
    ConfigError, KnownModule, Layer, Overrides, Resolution, ResolveRequest, SettingValue, env_name,
    resolve,
};
use smith_config::user_config::prepare_user_config_edit;
use tempfile::TempDir;

const ID: &str = "image-generation";
const KEY: &str = "modules.image-generation.enabled";
const LEGACY: &str = "tools.image_generation.enabled";
const BASE: &str = r#"
default_profile = "work"
[profiles.work]
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
    home: TempDir,
    project: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            home: tempfile::tempdir().expect("isolated user root"),
            project: tempfile::tempdir().expect("isolated project root"),
        };
        fs::create_dir_all(fixture.home.path().join(".smith")).expect("user directory");
        fs::create_dir_all(fixture.project.path().join(".smith")).expect("project directory");
        fixture.write_project(BASE);
        fixture
    }

    fn write_project(&self, text: &str) {
        fs::write(self.project.path().join(".smith/config.toml"), text).expect("project config");
    }

    fn write_user(&self, text: &str) {
        fs::write(self.home.path().join(".smith/config.toml"), text).expect("user config");
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path())
            .with_home_dir(self.home.path())
            .with_known_modules(vec![known(true, true)])
    }

    fn set(&self, request: &mut ResolveRequest, layer: Layer, legacy: bool, enabled: bool) {
        let key = if legacy { LEGACY } else { KEY };
        let text = format!("{key} = {enabled}\n");
        match layer {
            Layer::BuiltIn => request.known_modules[0].default_enabled = enabled,
            Layer::UserFile => self.write_user(&text),
            Layer::ProjectFile => self.write_project(&format!("{text}\n{BASE}")),
            Layer::ProjectLocalFile => {
                fs::write(self.project.path().join(".smith/config.local.toml"), text)
                    .expect("project-local config");
            }
            Layer::Profile => {
                let existing = fs::read_to_string(self.project.path().join(".smith/config.toml"))
                    .expect("existing project config");
                self.write_project(&format!(
                    "{existing}\n{}",
                    profile_switch("work", key, enabled)
                ));
            }
            Layer::Environment => {
                request.env.insert(env_name(key), enabled.to_string());
            }
            Layer::CommandLine | Layer::SessionOverride => {
                let overrides = if layer == Layer::CommandLine {
                    &mut request.cli
                } else {
                    &mut request.session
                };
                if legacy {
                    overrides.image_generation_enabled = Some(enabled);
                } else {
                    overrides.modules.insert(ID.to_owned(), enabled);
                }
            }
        }
    }
}

fn profile_switch(profile: &str, key: &str, enabled: bool) -> String {
    let table = key.strip_suffix(".enabled").expect("switch key");
    format!("[profiles.{profile}.{table}]\nenabled = {enabled}\n")
}

fn known(default_enabled: bool, compiled_in: bool) -> KnownModule {
    KnownModule {
        id: ID.to_owned(),
        default_enabled,
        compiled_in,
        legacy_enabled_key: Some(LEGACY.to_owned()),
    }
}

fn assert_switch(resolution: &Resolution, enabled: bool, layer: Layer) {
    let module = &resolution.config.modules[ID];
    assert_eq!(module.enabled.value, enabled);
    assert_eq!(module.enabled.source.layer, layer);
    assert_eq!(module.enabled, resolution.config.image_generation.enabled);
    let canonical = resolution
        .provenance
        .explain(KEY)
        .expect("module explanation");
    let legacy = resolution
        .provenance
        .explain(LEGACY)
        .expect("legacy explanation");
    assert_eq!(canonical.value, SettingValue::Flag(enabled));
    assert_eq!(canonical.source, module.enabled.source);
    assert_eq!(canonical.source, legacy.source);
    assert_eq!(canonical.overridden, legacy.overridden);
}

const LAYERS: &[Layer] = &[
    Layer::BuiltIn,
    Layer::UserFile,
    Layer::ProjectFile,
    Layer::ProjectLocalFile,
    Layer::Profile,
    Layer::Environment,
    Layer::CommandLine,
    Layer::SessionOverride,
];

#[test]
fn each_spelling_works_in_every_layer_with_the_written_source() {
    for &layer in LAYERS {
        for legacy in [false, true] {
            let fixture = Fixture::new();
            let mut request = fixture.request();
            fixture.set(&mut request, layer, legacy, false);
            let resolution = resolve(&request).expect("module selection");
            assert_switch(&resolution, false, layer);
            let key = if legacy || layer == Layer::BuiltIn {
                LEGACY
            } else {
                KEY
            };
            let expected = match layer {
                Layer::Profile => format!("profiles.work.{key}"),
                Layer::Environment => env_name(key),
                _ => key.to_owned(),
            };
            assert_eq!(resolution.config.modules[ID].enabled.source.key, expected);
            let file = resolution.config.modules[ID].enabled.source.file.as_ref();
            match layer {
                Layer::UserFile => {
                    assert_eq!(file, Some(&fixture.home.path().join(".smith/config.toml")))
                }
                Layer::ProjectFile | Layer::Profile => assert_eq!(
                    file,
                    Some(&fixture.project.path().join(".smith/config.toml"))
                ),
                Layer::ProjectLocalFile => assert_eq!(
                    file,
                    Some(&fixture.project.path().join(".smith/config.local.toml"))
                ),
                _ => assert!(file.is_none()),
            }
        }
    }
}

#[test]
fn every_ordered_layer_pair_uses_one_switch_across_both_spellings() {
    for (low_index, &low) in LAYERS.iter().enumerate() {
        for &high in &LAYERS[low_index + 1..] {
            for low_legacy in [false, true] {
                for high_legacy in [false, true] {
                    let fixture = Fixture::new();
                    let mut request = fixture.request();
                    fixture.set(&mut request, low, low_legacy, true);
                    fixture.set(&mut request, high, high_legacy, false);
                    let resolution = resolve(&request).expect("ordinary layer precedence");
                    assert_switch(&resolution, false, high);
                    let explanation = resolution.provenance.explain(KEY).expect("history");
                    assert!(
                        explanation
                            .overridden
                            .iter()
                            .any(|entry| entry.source.layer == low)
                    );
                }
            }
        }
    }
}

#[test]
fn disagreeing_aliases_fail_in_each_layer_and_name_both_sources() {
    for &layer in &LAYERS[1..] {
        let fixture = Fixture::new();
        let mut request = fixture.request();
        if layer == Layer::Profile {
            fixture.write_project(&format!(
                "{BASE}\n{}\n{}",
                profile_switch("work", KEY, false),
                profile_switch("work", LEGACY, true)
            ));
        } else if matches!(
            layer,
            Layer::UserFile | Layer::ProjectFile | Layer::ProjectLocalFile
        ) {
            let text = format!("{KEY} = false\n{LEGACY} = true\n");
            match layer {
                Layer::UserFile => fixture.write_user(&text),
                Layer::ProjectFile => fixture.write_project(&format!("{text}{BASE}")),
                _ => fs::write(
                    fixture.project.path().join(".smith/config.local.toml"),
                    text,
                )
                .expect("local file"),
            }
        } else {
            fixture.set(&mut request, layer, false, false);
            fixture.set(&mut request, layer, true, true);
        }
        let error = resolve(&request).expect_err("conflicting aliases");
        assert!(matches!(error, ConfigError::Ambiguous { .. }), "{error}");
        let message = error.to_string();
        assert!(message.contains(KEY), "{message}");
        assert!(
            message.contains(LEGACY)
                || message.contains(&env_name(LEGACY))
                || message.contains(&LEGACY.replace(['.', '_'], "-")),
            "{message}"
        );
        let path = match layer {
            Layer::UserFile => Some(fixture.home.path().join(".smith/config.toml")),
            Layer::ProjectFile | Layer::Profile => {
                Some(fixture.project.path().join(".smith/config.toml"))
            }
            Layer::ProjectLocalFile => {
                Some(fixture.project.path().join(".smith/config.local.toml"))
            }
            _ => None,
        };
        if let Some(path) = path {
            assert!(message.contains(&path.display().to_string()), "{message}");
        }
    }
}

#[test]
fn agreeing_aliases_are_accepted_in_every_explicit_layer() {
    for &layer in &LAYERS[1..] {
        let fixture = Fixture::new();
        let mut request = fixture.request();
        let prefix = if layer == Layer::Profile {
            "profiles.work."
        } else {
            ""
        };
        let text = format!("{prefix}{KEY} = false\n{prefix}{LEGACY} = false\n");
        match layer {
            Layer::UserFile => fixture.write_user(&text),
            Layer::ProjectFile => fixture.write_project(&format!("{text}{BASE}")),
            Layer::Profile => fixture.write_project(&format!(
                "{BASE}\n{}\n{}",
                profile_switch("work", KEY, false),
                profile_switch("work", LEGACY, false)
            )),
            Layer::ProjectLocalFile => fs::write(
                fixture.project.path().join(".smith/config.local.toml"),
                text,
            )
            .expect("local config"),
            _ => {
                fixture.set(&mut request, layer, false, false);
                fixture.set(&mut request, layer, true, false);
            }
        }
        assert_switch(&resolve(&request).expect("agreeing aliases"), false, layer);
    }
}

#[test]
fn profile_carries_its_own_module_set_above_the_user_value() {
    let fixture = Fixture::new();
    fixture.write_user(&format!("{KEY} = true"));
    fixture.write_project(&format!("{BASE}\n{}", profile_switch("work", KEY, false)));
    let resolution = resolve(&fixture.request()).expect("profile selection");
    assert_switch(&resolution, false, Layer::Profile);
    assert_eq!(
        resolution.config.modules[ID].enabled.source.key,
        format!("profiles.work.{KEY}")
    );
}

#[test]
fn unknown_module_ids_list_the_entire_catalog_even_in_empty_tables_and_unused_profiles() {
    for text in [
        "modules.nope.enabled = true",
        "[modules.nope]",
        "profiles.unused.modules.nope.enabled = true",
        "[profiles.unused.modules.nope]",
    ] {
        let fixture = Fixture::new();
        fixture.write_user(text);
        let mut request = fixture.request();
        request.known_modules.push(KnownModule {
            id: "budget-notice".to_owned(),
            default_enabled: true,
            compiled_in: true,
            legacy_enabled_key: None,
        });
        let error = resolve(&request).expect_err("unknown module");
        let message = error.to_string();
        assert!(message.contains("nope"), "{message}");
        assert!(message.contains(ID), "{message}");
        assert!(message.contains("budget-notice"), "{message}");
        assert!(matches!(error, ConfigError::UnknownModule { .. }));
    }
}

#[test]
fn unknown_override_and_environment_module_ids_are_rejected() {
    for layer in [
        Layer::CommandLine,
        Layer::SessionOverride,
        Layer::Environment,
    ] {
        let fixture = Fixture::new();
        let mut request = fixture.request();
        match layer {
            Layer::CommandLine => {
                request.cli.modules.insert("nope".to_owned(), true);
            }
            Layer::SessionOverride => {
                request.session.modules.insert("nope".to_owned(), true);
            }
            _ => {
                request
                    .env
                    .insert("SMITH_MODULES_NOPE_ENABLED".to_owned(), "true".to_owned());
            }
        }
        assert!(
            matches!(resolve(&request), Err(ConfigError::UnknownModule { id, .. }) if id == "nope")
        );
    }
}

#[test]
fn an_enabled_module_not_in_the_build_is_reported_without_failing_resolution() {
    let fixture = Fixture::new();
    fixture.write_user(&format!("{KEY} = true"));
    let mut request = fixture.request();
    request.known_modules[0].compiled_in = false;
    let resolution = resolve(&request).expect("missing build does not prevent startup");
    assert_switch(&resolution, true, Layer::UserFile);
    assert!(resolution.config.modules[ID].on_but_not_built());
    request.session.modules.insert(ID.to_owned(), false);
    let disabled = resolve(&request).expect("off and not built");
    assert!(!disabled.config.modules[ID].on_but_not_built());
}

#[test]
fn declared_defaults_apply_to_modules_without_computed_legacy_defaults() {
    for default_enabled in [false, true] {
        let fixture = Fixture::new();
        let mut request = fixture.request();
        request.known_modules[0].default_enabled = default_enabled;
        request.known_modules.push(KnownModule {
            id: "third-party".to_owned(),
            default_enabled,
            compiled_in: true,
            legacy_enabled_key: None,
        });
        let resolution = resolve(&request).expect("declared defaults");
        assert_switch(&resolution, false, Layer::BuiltIn);
        assert_eq!(
            resolution.config.modules["third-party"].enabled.value,
            default_enabled
        );
        assert!(!resolution.config.modules["third-party"].on_but_not_built());
    }
}

#[test]
fn computed_legacy_defaults_match_existing_provider_behavior() {
    for (kind, endpoint) in [
        ("fake", None),
        ("chatgpt-responses", None),
        ("openai-compatible", Some("https://api.openai.com/v1")),
        ("openai-responses", Some("https://api.openai.com/v1/")),
        ("openai-compatible", Some("https://example.test/v1")),
    ] {
        for declared_default in [false, true] {
            let fixture = Fixture::new();
            let provider = match endpoint {
                Some(endpoint) => format!("kind = \"{kind}\"\nbase_url = \"{endpoint}\""),
                None if kind == "chatgpt-responses" => format!(
                    "kind = \"{kind}\"\nbase_url = \"{}\"\ncredential = \"authfile:chatgpt\"",
                    smith_config::setup::CHATGPT_ENDPOINT,
                ),
                None => format!("kind = \"{kind}\""),
            };
            fixture.write_project(&BASE.replace("kind = \"fake\"", &provider));
            let mut request = fixture.request();
            request.known_modules.clear();
            let original = resolve(&request).expect("legacy configuration");
            request.known_modules.push(known(declared_default, true));
            let modular = resolve(&request).expect("module configuration");
            assert_switch(
                &modular,
                original.config.image_generation.enabled.value,
                Layer::BuiltIn,
            );
            assert_eq!(
                modular.config.image_generation,
                original.config.image_generation
            );
        }
    }
}

#[test]
fn an_alias_without_a_computed_default_uses_the_declared_default() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request.known_modules.push(KnownModule {
        id: "external".into(),
        default_enabled: true,
        compiled_in: true,
        legacy_enabled_key: Some("external.enabled".into()),
    });
    let resolution = resolve(&request).expect("fallback default");
    assert!(resolution.config.modules["external"].enabled.value);
}

#[test]
fn an_empty_catalog_keeps_existing_resolution_and_rejects_any_module_table() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request.known_modules.clear();
    let resolution = resolve(&request).expect("original config");
    assert!(resolution.config.modules.is_empty());
    assert!(
        !resolution.config.image_generation.enabled.value,
        "fake provider default"
    );
    fixture.write_user(&format!("{LEGACY} = true"));
    let resolution = resolve(&request).expect("existing switch");
    assert!(resolution.config.image_generation.enabled.value);
    fixture.write_user(&format!("{KEY} = true"));
    let error = resolve(&request).expect_err("no known ids");
    assert!(matches!(&error, ConfigError::UnknownModule { known_ids, .. } if known_ids.is_empty()));
    assert!(error.to_string().contains("(none)"));
}

#[test]
fn profile_aliases_across_files_and_inheritance_use_normal_precedence() {
    let fixture = Fixture::new();
    fixture.write_user(&format!("profiles.parent.{LEGACY} = true"));
    fixture.write_project(&format!(
        "{}\n{}",
        BASE.replace("[profiles.work]", "[profiles.work]\nextends = \"parent\""),
        profile_switch("parent", KEY, false)
    ));
    let inherited = resolve(&fixture.request()).expect("profile inherits module switch");
    assert_switch(&inherited, false, Layer::Profile);
    assert_eq!(
        inherited.config.modules[ID].enabled.source.key,
        format!("profiles.parent.{KEY}")
    );
    fixture.write_user(&format!(
        "profiles.work.{LEGACY} = true\nprofiles.parent.{KEY} = false"
    ));
    fixture
        .write_project(&BASE.replace("[profiles.work]", "[profiles.work]\nextends = \"parent\""));
    let child = resolve(&fixture.request()).expect("child overrides parent alias");
    assert_switch(&child, true, Layer::Profile);
    assert_eq!(
        child.config.modules[ID].enabled.source.key,
        format!("profiles.work.{LEGACY}")
    );
}

#[test]
fn environment_equal_precedence_collisions_and_invalid_booleans_fail() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request.env.insert(env_name(KEY), "false".to_owned());
    request
        .env
        .insert(env_name(KEY).to_lowercase(), "true".to_owned());
    assert!(matches!(
        resolve(&request),
        Err(ConfigError::Ambiguous { .. })
    ));
    request.env.clear();
    request.env.insert(env_name(KEY), "yes".to_owned());
    assert!(matches!(
        resolve(&request),
        Err(ConfigError::InvalidValue { .. })
    ));
    fixture.write_user(&format!("{KEY} = \"false\""));
    request.env.clear();
    assert!(matches!(
        resolve(&request),
        Err(ConfigError::Malformed { .. })
    ));
}

#[test]
fn unrelated_image_settings_keep_their_existing_keys() {
    let fixture = Fixture::new();
    fixture.write_user("tools.image_generation.model = \"custom-image\"\ntools.image_generation.quality = \"high\"\ntools.image_generation.size = \"1024x1024\"");
    let resolution = resolve(&fixture.request()).expect("remaining feature settings");
    assert_eq!(
        resolution.config.image_generation.model.value,
        "custom-image"
    );
    assert_eq!(resolution.config.image_generation.quality.value, "high");
    assert_eq!(resolution.config.image_generation.size.value, "1024x1024");
    assert_eq!(
        resolution.config.image_generation.model.source.key,
        "tools.image_generation.model"
    );
}

#[test]
fn module_user_edit_is_previewed_committed_and_rollback_capable() {
    let fixture = Fixture::new();
    fixture.write_user("# retained\n[limits]\nmax_retries = 7\n");
    let user_dir = fixture.home.path().join(".smith");
    let path = user_dir.join("config.toml");
    let original = fs::read(&path).expect("prior bytes");
    let patch = ConfigFile {
        modules: BTreeMap::from([(
            ID.to_owned(),
            ModuleSection {
                enabled: Some(false),
            },
        )]),
        ..ConfigFile::default()
    };
    let edit = prepare_user_config_edit(&user_dir, &patch).expect("module key accepted");
    assert!(edit.preview().contains(KEY));
    assert!(edit.collisions().is_empty());
    assert_eq!(fs::read(&path).expect("preview writes nothing"), original);
    let committed = edit.commit(false).expect("committed module edit");
    assert_switch(
        &resolve(&fixture.request()).expect("edited config resolves"),
        false,
        Layer::UserFile,
    );
    let stored = fs::read_to_string(&path).expect("candidate contents");
    assert!(stored.contains("# retained"));
    assert!(stored.contains("max_retries = 7"));
    committed.rollback().expect("rollback module edit");
    assert_eq!(fs::read(&path).expect("restored bytes"), original);
}

#[test]
fn module_session_overrides_use_the_existing_typed_host_path() {
    let fixture = Fixture::new();
    let request = fixture.request().with_session(Overrides {
        modules: BTreeMap::from([(ID.to_owned(), false)]),
        ..Overrides::default()
    });
    assert_switch(
        &resolve(&request).expect("session override"),
        false,
        Layer::SessionOverride,
    );
}
