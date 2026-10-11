//! Plugin selection, nonfatal stale settings, provenance, and user edits.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use smith_config::model::{ConfigFile, PluginSection};
use smith_config::resolve::{
    ConfigError, KnownPlugin, Layer, Overrides, Resolution, ResolveRequest, SettingValue, env_name,
    resolve,
};
use smith_config::user_config::prepare_user_config_edit;
use tempfile::TempDir;

const NAME: &str = "deploy-tools";
const KEY: &str = "plugins.deploy-tools.enabled";
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

    fn home_root(&self) -> PathBuf {
        self.home.path().canonicalize().expect("canonical home")
    }

    fn project_root(&self) -> PathBuf {
        self.project
            .path()
            .canonicalize()
            .expect("canonical project")
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path())
            .with_home_dir(self.home.path())
            .with_known_plugins(vec![KnownPlugin {
                name: NAME.to_owned(),
                default_enabled: true,
            }])
    }

    fn set(&self, request: &mut ResolveRequest, layer: Layer, enabled: bool) {
        self.set_named(request, layer, NAME, KEY, enabled);
    }

    fn set_named(
        &self,
        request: &mut ResolveRequest,
        layer: Layer,
        name: &str,
        key: &str,
        enabled: bool,
    ) {
        let text = format!("{key} = {enabled}\n");
        match layer {
            Layer::BuiltIn => request.known_plugins[0].default_enabled = enabled,
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
                    "{existing}\n[profiles.work.{}]\nenabled = {enabled}\n",
                    key.strip_suffix(".enabled").expect("switch key")
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
                overrides.plugins.insert(name.to_owned(), enabled);
            }
        }
    }
}

fn assert_switch(resolution: &Resolution, name: &str, key: &str, enabled: bool, layer: Layer) {
    let plugin = &resolution.config.plugins[name];
    assert_eq!(plugin.enabled.value, enabled);
    assert_eq!(plugin.enabled.source.layer, layer);
    let explanation = resolution
        .provenance
        .explain(key)
        .expect("plugin explanation");
    assert_eq!(explanation.value, SettingValue::Flag(enabled));
    assert_eq!(explanation.source, plugin.enabled.source);
}

#[test]
fn each_layer_selects_a_plugin_with_the_written_source() {
    for &layer in LAYERS {
        let fixture = Fixture::new();
        let mut request = fixture.request();
        fixture.set(&mut request, layer, false);
        let resolution = resolve(&request).expect("plugin selection");
        assert_switch(&resolution, NAME, KEY, false, layer);
        assert!(resolution.config.unknown_plugins.is_empty());
        let source = &resolution.config.plugins[NAME].enabled.source;
        let expected = match layer {
            Layer::Profile => format!("profiles.work.{KEY}"),
            Layer::Environment => env_name(KEY),
            _ => KEY.to_owned(),
        };
        assert_eq!(source.key, expected);
        let expected_file = match layer {
            Layer::UserFile => Some(fixture.home_root().join(".smith/config.toml")),
            Layer::ProjectFile | Layer::Profile => {
                Some(fixture.project_root().join(".smith/config.toml"))
            }
            Layer::ProjectLocalFile => {
                Some(fixture.project_root().join(".smith/config.local.toml"))
            }
            _ => None,
        };
        assert_eq!(source.file, expected_file);
    }
}

#[test]
fn every_ordered_layer_pair_uses_ordinary_precedence_and_explain_history() {
    for (low_index, &low) in LAYERS.iter().enumerate() {
        for &high in &LAYERS[low_index + 1..] {
            let fixture = Fixture::new();
            let mut request = fixture.request();
            fixture.set(&mut request, low, true);
            fixture.set(&mut request, high, false);
            let resolution = resolve(&request).expect("ordinary precedence");
            assert_switch(&resolution, NAME, KEY, false, high);
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

#[test]
fn installed_defaults_are_supplied_by_the_host() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request.known_plugins.push(KnownPlugin {
        name: "opt-in".to_owned(),
        default_enabled: false,
    });
    let resolution = resolve(&request).expect("manifest defaults");
    assert_switch(&resolution, NAME, KEY, true, Layer::BuiltIn);
    assert_switch(
        &resolution,
        "opt-in",
        "plugins.opt-in.enabled",
        false,
        Layer::BuiltIn,
    );
}

#[test]
fn an_omitted_plugin_switch_uses_its_installed_default() {
    for default_enabled in [false, true] {
        let fixture = Fixture::new();
        fixture.write_user("[plugins.deploy-tools]\n");
        let mut request = fixture.request();
        request.known_plugins[0].default_enabled = default_enabled;
        let resolution = resolve(&request).expect("empty table retains manifest default");
        assert_switch(&resolution, NAME, KEY, default_enabled, Layer::BuiltIn);
        assert!(resolution.config.unknown_plugins.is_empty());
    }
}

#[test]
fn unknown_plugins_are_reported_without_failing_in_every_explicit_layer() {
    for &layer in &LAYERS[1..] {
        let fixture = Fixture::new();
        let mut request = fixture.request();
        fixture.set_named(
            &mut request,
            layer,
            "removed",
            "plugins.removed.enabled",
            false,
        );
        let resolution = resolve(&request).expect("stale plugin setting is nonfatal");
        assert_switch(&resolution, NAME, KEY, true, Layer::BuiltIn);
        assert!(!resolution.config.plugins.contains_key("removed"));
        let reports = &resolution.config.unknown_plugins;
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].name, "removed");
        let expected_layer = if layer == Layer::Profile {
            Layer::ProjectFile
        } else {
            layer
        };
        assert_eq!(reports[0].source.layer, expected_layer);
        let key = match layer {
            Layer::Profile => "profiles.work.plugins.removed.enabled".to_owned(),
            Layer::Environment => env_name("plugins.removed.enabled"),
            _ => "plugins.removed.enabled".to_owned(),
        };
        assert_eq!(reports[0].source.key, key);
        let explanation = resolution
            .provenance
            .explain("plugins.removed.enabled")
            .expect("stale setting can still be explained");
        assert_eq!(explanation.value, SettingValue::Flag(false));
        assert_eq!(explanation.source.layer, layer);
    }
}

#[test]
fn stale_settings_keep_each_source_including_unselected_profiles_and_empty_tables() {
    let fixture = Fixture::new();
    fixture.write_user("[plugins.removed]\nenabled = false\n[plugins.empty]\n");
    fixture.write_project(&format!(
        "plugins.removed.enabled = true\n{BASE}\n[profiles.unselected.plugins.removed]\nenabled = false\n"
    ));
    let mut request = fixture.request();
    request.known_plugins.clear();
    let resolution = resolve(&request).expect("uninstalled settings do not block startup");
    assert!(resolution.config.plugins.is_empty());
    let reports = &resolution.config.unknown_plugins;
    assert_eq!(reports.len(), 4);
    assert!(reports.iter().any(|report| report.name == "empty"
        && report.source.key == "plugins.empty"
        && report.source.file == Some(fixture.home_root().join(".smith/config.toml"))));
    assert!(reports.iter().any(|report| report.name == "removed"
        && report.source.key == "profiles.unselected.plugins.removed.enabled"
        && report.source.file == Some(fixture.project_root().join(".smith/config.toml"))));
}

#[test]
fn a_quoted_uninstalled_name_is_reported_and_can_be_explained() {
    let fixture = Fixture::new();
    const KEY: &str = "plugins.\"removed:tools\".enabled";
    fixture.write_user(&format!("{KEY} = false\n"));
    let resolution = resolve(&fixture.request()).expect("quoted stale setting is nonfatal");
    let reports = &resolution.config.unknown_plugins;
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].name, "removed:tools");
    assert_eq!(reports[0].source.key, KEY);
    assert_eq!(reports[0].source.layer, Layer::UserFile);
    assert_eq!(
        resolution
            .provenance
            .explain(KEY)
            .expect("stale explanation")
            .source,
        reports[0].source
    );
}

#[test]
fn quoted_names_roundtrip_and_explain_in_every_layer() {
    const QUOTED_NAME: &str = "deploy:tools";
    const QUOTED_KEY: &str = "plugins.\"deploy:tools\".enabled";
    for &layer in LAYERS {
        let fixture = Fixture::new();
        let mut request = fixture.request();
        request.known_plugins[0].name = QUOTED_NAME.to_owned();
        fixture.set_named(&mut request, layer, QUOTED_NAME, QUOTED_KEY, false);
        let resolution = resolve(&request).expect("quoted name resolves");
        assert_switch(&resolution, QUOTED_NAME, QUOTED_KEY, false, layer);
        assert!(resolution.provenance.keys().any(|key| key == QUOTED_KEY));
    }
    let text = "[plugins.\"deploy:tools\"]\nenabled = false\n";
    let file = ConfigFile::parse(text).expect("quoted name parses");
    assert_eq!(file.plugins[QUOTED_NAME].enabled, Some(false));
    let serialized = toml::to_string(&file).expect("quoted name serializes");
    assert_eq!(ConfigFile::parse(&serialized).expect("roundtrip"), file);
}

#[test]
fn selected_profiles_inherit_plugin_switches_and_retain_the_parent_source() {
    let fixture = Fixture::new();
    fixture.write_project(&format!(
        "{BASE}\n[profiles.parent.plugins.deploy-tools]\nenabled = false\n[profiles.child]\nextends = \"parent\"\nprovider = \"local\"\nmodel = \"example-model\"\n"
    ));
    let request = fixture.request().with_cli(Overrides {
        profile: Some("child".to_owned()),
        ..Overrides::default()
    });
    let resolution = resolve(&request).expect("profile inheritance");
    assert_switch(&resolution, NAME, KEY, false, Layer::Profile);
    assert_eq!(
        resolution.config.plugins[NAME].enabled.source.key,
        "profiles.parent.plugins.deploy-tools.enabled"
    );
}

#[test]
fn plugin_user_edit_is_previewed_committed_and_rollback_capable() {
    for name in [NAME, "deploy:tools"] {
        let fixture = Fixture::new();
        fixture.write_user("# retained\n[limits]\nmax_retries = 7\n");
        let user_dir = fixture.home.path().join(".smith");
        let path = user_dir.join("config.toml");
        let original = fs::read(&path).expect("prior bytes");
        let patch = ConfigFile {
            plugins: BTreeMap::from([(
                name.to_owned(),
                PluginSection {
                    modules: Default::default(),
                    enabled: Some(false),
                },
            )]),
            ..ConfigFile::default()
        };
        let key = if name == NAME {
            KEY
        } else {
            "plugins.\"deploy:tools\".enabled"
        };
        let edit = prepare_user_config_edit(&user_dir, &patch).expect("plugin key accepted");
        assert!(edit.preview().contains(key));
        assert!(edit.collisions().is_empty());
        assert_eq!(fs::read(&path).expect("preview writes nothing"), original);
        let committed = edit.commit(false).expect("committed plugin edit");
        let mut request = fixture.request();
        request.known_plugins[0].name = name.to_owned();
        let resolution = resolve(&request).expect("edited config resolves");
        assert_switch(&resolution, name, key, false, Layer::UserFile);
        let stored = fs::read_to_string(&path).expect("candidate contents");
        assert!(stored.contains("# retained"));
        assert!(stored.contains("max_retries = 7"));
        committed.rollback().expect("rollback plugin edit");
        assert_eq!(fs::read(&path).expect("restored bytes"), original);
    }
}

#[test]
fn plugin_user_edit_reports_collisions_before_replacing_the_switch() {
    let fixture = Fixture::new();
    fixture.write_user("# retained\n[plugins.deploy-tools]\nenabled = true\n");
    let user_dir = fixture.home.path().join(".smith");
    let path = user_dir.join("config.toml");
    let original = fs::read(&path).expect("original");
    let patch = ConfigFile {
        plugins: BTreeMap::from([(
            NAME.to_owned(),
            PluginSection {
                modules: Default::default(),
                enabled: Some(false),
            },
        )]),
        ..ConfigFile::default()
    };
    let edit = prepare_user_config_edit(&user_dir, &patch).expect("prepared switch");
    assert_eq!(edit.collisions().len(), 1);
    assert!(edit.preview().contains(KEY));
    assert!(edit.preview().contains(" -> "));
    assert_eq!(
        edit.changes()[0].previous.as_deref().unwrap().trim(),
        "true"
    );
    assert_eq!(edit.changes()[0].proposed.trim(), "false");
    assert!(edit.commit(false).is_err());
    assert_eq!(fs::read(&path).expect("collision writes nothing"), original);
    let committed = edit.commit(true).expect("reviewed switch");
    assert_switch(
        &resolve(&fixture.request()).expect("edited selection"),
        NAME,
        KEY,
        false,
        Layer::UserFile,
    );
    committed.rollback().expect("rollback");
    assert_eq!(fs::read(&path).expect("restored"), original);
}

#[test]
fn no_plugins_preserves_serialized_files_and_the_existing_ledger() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request.known_plugins.clear();
    let resolution = resolve(&request).expect("existing behavior");
    assert!(resolution.config.plugins.is_empty());
    assert!(resolution.config.unknown_plugins.is_empty());
    assert!(
        !resolution
            .provenance
            .keys()
            .any(|key| key.starts_with("plugins."))
    );
    assert!(
        !toml::to_string(&ConfigFile::default())
            .expect("empty config")
            .contains("plugins")
    );
    let file = ConfigFile::parse(BASE).expect("existing fixture");
    let serialized = toml::to_string(&file).expect("existing serialization");
    assert!(!serialized.contains("plugins"));
    assert_eq!(
        ConfigFile::parse(&serialized).expect("existing roundtrip"),
        file
    );
}

#[test]
fn invalid_plugin_switches_and_environment_ambiguity_still_fail_normally() {
    for name in [NAME, "removed"] {
        let fixture = Fixture::new();
        fixture.write_user(&format!("plugins.{name}.enabled = \"no\"\n"));
        assert!(resolve(&fixture.request()).is_err());
    }
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
}

#[test]
fn plugin_management_resolution_does_not_require_a_provider_and_matches_run_switches() {
    let fixture = Fixture::new();
    fixture.write_user("plugins.deploy-tools.enabled = false\nplugins.removed.enabled = true\n");
    let request = fixture.request();
    let selection = smith_config::resolve::resolve_plugin_selection(&request).unwrap();
    let config = resolve(&request).unwrap().config;
    assert_eq!(selection.plugins, config.plugins);
    assert_eq!(selection.unknown_plugins, config.unknown_plugins);
    std::fs::remove_file(fixture.project.path().join(".smith/config.toml")).unwrap();
    let selection = smith_config::resolve::resolve_plugin_selection(&request).unwrap();
    assert!(!selection.plugins[NAME].enabled.value);
    assert_eq!(
        selection.plugins[NAME].enabled.source.layer,
        Layer::UserFile
    );
    assert_eq!(selection.unknown_plugins[0].name, "removed");
}
