//! Advisor selection, early placement validation, and source provenance.

use smith_config::model::{AdvisorSelection, ConfigFile, ProfileUse};
use smith_config::resolve::{
    ConfigError, Layer, Overrides, Resolution, ResolveRequest, SettingValue, resolve,
};
use tempfile::TempDir;

const SOL: &str = "use = [\"main\", \"child\", \"advisor\"]";

struct Fixture {
    home: TempDir,
    project: TempDir,
}

impl Fixture {
    fn new(config: &str) -> Self {
        let fixture = Self {
            home: tempfile::tempdir().expect("isolated home"),
            project: tempfile::tempdir().expect("isolated project"),
        };
        std::fs::create_dir_all(fixture.home.path().join(".smith")).expect("user directory");
        std::fs::create_dir_all(fixture.project.path().join(".smith")).expect("project directory");
        fixture.write_project("config.toml", config);
        fixture
    }

    fn write_project(&self, name: &str, config: &str) {
        std::fs::write(self.project.path().join(".smith").join(name), config)
            .expect("project configuration");
    }

    fn write_user(&self, config: &str) {
        std::fs::write(self.home.path().join(".smith/config.toml"), config)
            .expect("user configuration");
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path()).with_home_dir(self.home.path())
    }

    fn resolve(&self) -> Result<Resolution, ConfigError> {
        resolve(&self.request())
    }
}

fn config(default: &str, code: &str, sol: &str) -> String {
    format!(
        r#"
default_profile = "code"
{default}

[profiles.code]
provider = "acme"
model = "working-model"
{code}

[profiles.sol]
provider = "acme"
model = "advisor-model"
{sol}

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "file:must-not-be-read"
"#
    )
}

#[test]
fn advisor_top_level_default_resolves_for_main_profiles() {
    let fixture = Fixture::new(&config("advisor = \"sol\"", "", SOL));
    let resolution = fixture.resolve().expect("a main profile with an advisor");
    let agent = &resolution.config.agent;
    let advisor = agent.profile.advisor.as_ref().expect("resolved advisor");

    assert_eq!(agent.profile.name, "code");
    assert_eq!(advisor.value, "sol");
    assert_eq!(advisor.source.layer, Layer::ProjectFile);
    assert_eq!(advisor.source.key, "advisor");
    assert_eq!(
        agent.profiles["sol"].uses.value,
        [ProfileUse::Main, ProfileUse::Child, ProfileUse::Advisor]
    );
    assert!(agent.profiles["sol"].advisor.is_none());
    assert!(agent.child_profile("sol").is_some());
    let explained = resolution
        .provenance
        .explain("advisor")
        .expect("default source");
    assert_eq!(explained.value, SettingValue::Text("sol".to_owned()));
    assert_eq!(explained.source, advisor.source);
    assert!(explained.overridden.is_empty());
}

#[test]
fn advisor_profile_false_disables_top_level_default() {
    let fixture = Fixture::new(&config("advisor = \"sol\"", "", SOL));
    fixture.write_project(
        "config.local.toml",
        "default_profile = \"glm\"\n[profiles.glm]\nextends = \"code\"\nadvisor = false\n",
    );
    let resolution = fixture.resolve().expect("an explicit opt-out");

    assert_eq!(resolution.config.agent.profile.name, "glm");
    assert!(resolution.config.agent.profile.advisor.is_none());
    let explained = resolution
        .provenance
        .explain("advisor")
        .expect("false source");
    assert_eq!(explained.value, SettingValue::Flag(false));
    assert_eq!(explained.source.layer, Layer::Profile);
    assert_eq!(explained.source.key, "profiles.glm.advisor");
    assert_eq!(explained.overridden.len(), 1);
    assert_eq!(explained.overridden[0].source.key, "advisor");
    assert_eq!(
        explained.overridden[0].value,
        SettingValue::Text("sol".to_owned())
    );
}

#[test]
fn advisor_top_level_file_precedence_is_source_explainable() {
    let fixture = Fixture::new(&config("advisor = false", "", SOL));
    fixture.write_user("advisor = \"sol\"\n");
    fixture.write_project("config.local.toml", "advisor = \"sol\"\n");
    let resolution = fixture.resolve().expect("project-local advisor default");
    let advisor = resolution
        .config
        .agent
        .profile
        .advisor
        .as_ref()
        .expect("advisor");
    let explained = resolution
        .provenance
        .explain("advisor")
        .expect("file precedence");

    assert_eq!(advisor.value, "sol");
    assert_eq!(advisor.source.layer, Layer::ProjectLocalFile);
    assert_eq!(explained.source, advisor.source);
    assert_eq!(explained.overridden.len(), 2);
    assert_eq!(explained.overridden[0].source.layer, Layer::ProjectFile);
    assert_eq!(explained.overridden[0].value, SettingValue::Flag(false));
    assert_eq!(explained.overridden[1].source.layer, Layer::UserFile);
    assert_eq!(
        explained.overridden[1].value,
        SettingValue::Text("sol".to_owned())
    );
}

#[test]
fn advisor_target_requires_placement_before_provider_and_credential_validation() {
    let text = config("advisor = \"plan\"", "", SOL)
        .replace(
            "kind = \"openai-compatible\"",
            "kind = \"invalid-provider\"",
        )
        .replace("file:must-not-be-read", "invalid-credential");
    let fixture = Fixture::new(&format!("{text}\n[profiles.plan]\nuse = [\"main\"]\n"));
    let error = fixture
        .resolve()
        .expect_err("a target without advisor placement");

    match error {
        ConfigError::InvalidValue { source, message } => {
            assert_eq!(source.key, "advisor");
            assert!(message.contains("`plan`"), "{message}");
            assert!(
                message.contains("`use` lacks the `advisor` placement"),
                "{message}"
            );
        }
        other => panic!("expected a placement error before provider resolution, got {other:?}"),
    }
}

#[test]
fn advisor_unknown_target_lists_only_advisor_profiles() {
    let fixture = Fixture::new(&format!(
        "{}\n[profiles.other]\nuse = [\"advisor\"]\n[profiles.main_only]\nuse = [\"main\"]\n",
        config("advisor = \"missing\"", "", SOL)
    ));
    let error = fixture.resolve().expect_err("an unknown advisor");

    match error {
        ConfigError::InvalidValue { source, message } => {
            assert_eq!(source.key, "advisor");
            assert!(message.contains("unknown profile `missing`"), "{message}");
            assert!(
                message.ends_with("profiles placed as advisors: `other`, `sol`"),
                "{message}"
            );
        }
        other => panic!("expected an advisor inventory diagnostic, got {other:?}"),
    }
}

#[test]
fn advisor_selection_is_inherited_through_extends() {
    for selection in ["\"sol\"", "false"] {
        let fixture = Fixture::new(&format!(
            "{}\n[profiles.base]\nextends = \"ancestor\"\n[profiles.ancestor]\nadvisor = {selection}\n",
            config("advisor = \"sol\"", "extends = \"base\"", SOL)
        ));
        let resolution = fixture.resolve().expect("an inherited advisor selection");
        let advisor = &resolution.config.agent.profile.advisor;

        if selection == "false" {
            assert!(advisor.is_none());
        } else {
            let advisor = advisor.as_ref().expect("inherited advisor name");
            assert_eq!(advisor.value, "sol");
            assert_eq!(advisor.source.key, "profiles.ancestor.advisor");
            assert_eq!(advisor.source.layer, Layer::Profile);
        }
        let explained = resolution
            .provenance
            .explain("advisor")
            .expect("inherited source");
        assert_eq!(explained.source.key, "profiles.ancestor.advisor");
        assert_eq!(explained.overridden.len(), 1);
        assert_eq!(explained.overridden[0].source.key, "advisor");
    }
}

#[test]
fn advisor_placed_profile_may_select_another_advisor_for_main_use() {
    for sol in [
        "use = [\"main\", \"child\", \"advisor\"]\nadvisor = \"other\"",
        "use = [\"main\", \"child\", \"advisor\"]\nextends = \"mentor\"",
    ] {
        let fixture = Fixture::new(&format!(
            "{}\n[profiles.mentor]\nadvisor = \"other\"\n[profiles.other]\nuse = [\"main\", \"advisor\"]\n",
            config("advisor = \"sol\"", "", sol)
        ));
        let code = fixture
            .resolve()
            .expect("the target's main advisor is valid");
        assert_eq!(
            code.config.agent.profile.advisor.as_ref().unwrap().value,
            "sol"
        );
        assert_eq!(
            code.config.agent.profiles["sol"]
                .advisor
                .as_ref()
                .unwrap()
                .value,
            "other"
        );
        assert_eq!(
            code.config.agent.profiles["other"]
                .advisor
                .as_ref()
                .unwrap()
                .value,
            "sol"
        );

        let request = fixture.request().with_cli(Overrides {
            profile: Some("sol".to_owned()),
            ..Overrides::default()
        });
        let main = resolve(&request).expect("sol may consult other when used as main");
        assert_eq!(main.config.agent.profile.name, "sol");
        assert_eq!(
            main.config.agent.profile.advisor.as_ref().unwrap().value,
            "other"
        );

        let advisor = resolve(&request.with_profile_use(ProfileUse::Advisor))
            .expect("sol's own selection is ignored while advising");
        assert_eq!(advisor.config.agent.profile.name, "sol");
        assert!(advisor.config.agent.profile.advisor.is_none());
        assert_eq!(
            advisor.config.agent.profiles["sol"]
                .advisor
                .as_ref()
                .unwrap()
                .value,
            "other"
        );
    }
}

#[test]
fn advisor_top_level_self_default_is_skipped_for_main_profile() {
    let fixture = Fixture::new(&config("advisor = \"sol\"", "", SOL));
    let code = fixture
        .resolve()
        .expect("the natural top-level default loads");
    assert_eq!(
        code.config.agent.profile.advisor.as_ref().unwrap().value,
        "sol"
    );
    assert!(code.config.agent.profiles["sol"].advisor.is_none());

    let sol = resolve(&fixture.request().with_cli(Overrides {
        profile: Some("sol".to_owned()),
        ..Overrides::default()
    }))
    .expect("sol starts as main without advising itself");
    assert_eq!(sol.config.agent.profile.name, "sol");
    assert!(sol.config.agent.profile.advisor.is_none());
    assert_eq!(
        sol.config.agent.profiles["code"]
            .advisor
            .as_ref()
            .unwrap()
            .value,
        "sol"
    );
}

#[test]
fn advisor_explicit_self_selection_is_rejected_including_inherited_selection() {
    for (code, sol, extra, key) in [
        ("advisor = \"code\"", SOL, "", "profiles.code.advisor"),
        (
            "",
            "use = [\"main\", \"child\", \"advisor\"]\nadvisor = \"sol\"",
            "",
            "profiles.sol.advisor",
        ),
        (
            "",
            "use = [\"main\", \"child\", \"advisor\"]\nextends = \"mentor\"",
            "[profiles.mentor]\nadvisor = \"sol\"\n",
            "profiles.mentor.advisor",
        ),
    ] {
        let fixture = Fixture::new(&format!(
            "{}\n{extra}",
            config("advisor = \"sol\"", code, sol)
        ));
        let error = fixture
            .resolve()
            .expect_err("an explicit self-reference is invalid");

        match error {
            ConfigError::InvalidValue { source, message } => {
                assert_eq!(source.key, key);
                assert_eq!(source.layer, Layer::Profile);
                assert!(
                    message.contains("cannot select itself as its `advisor`"),
                    "{message}"
                );
            }
            other => panic!("expected explicit self-reference rejection, got {other:?}"),
        }
    }
}

#[test]
fn advisor_explain_reports_winner_and_all_overridden_sources() {
    let fixture = Fixture::new(&format!(
        "{}\n[profiles.base]\nadvisor = \"sol\"\n",
        config(
            "advisor = \"sol\"",
            "extends = \"base\"\nadvisor = \"sol\"",
            SOL
        )
    ));
    fixture.write_user(
        "advisor = false\n[profiles.base]\nadvisor = false\n[profiles.code]\nadvisor = false\n",
    );
    fixture.write_project(
        "config.local.toml",
        "advisor = false\n[profiles.base]\nadvisor = false\n[profiles.code]\nadvisor = false\n",
    );
    let resolution = fixture.resolve().expect("a layered opt-out");
    let explained = resolution
        .provenance
        .explain("advisor")
        .expect("all advisor sources");
    let user = fixture
        .home
        .path()
        .canonicalize()
        .unwrap()
        .join(".smith/config.toml");
    let project_dir = fixture
        .project
        .path()
        .canonicalize()
        .unwrap()
        .join(".smith");
    let project = project_dir.join("config.toml");
    let local = project_dir.join("config.local.toml");

    assert!(resolution.config.agent.profile.advisor.is_none());
    assert_eq!(explained.value, SettingValue::Flag(false));
    assert_eq!(explained.source.key, "profiles.code.advisor");
    assert_eq!(explained.source.layer, Layer::Profile);
    assert_eq!(explained.source.file.as_ref(), Some(&local));
    let sources = explained
        .overridden
        .iter()
        .map(|entry| {
            (
                entry.source.key.as_str(),
                entry.source.file.as_ref().expect("file source").as_path(),
                entry.value.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sources,
        vec![
            (
                "profiles.code.advisor",
                project.as_path(),
                SettingValue::Text("sol".to_owned())
            ),
            (
                "profiles.code.advisor",
                user.as_path(),
                SettingValue::Flag(false)
            ),
            (
                "profiles.base.advisor",
                local.as_path(),
                SettingValue::Flag(false)
            ),
            (
                "profiles.base.advisor",
                project.as_path(),
                SettingValue::Text("sol".to_owned())
            ),
            (
                "profiles.base.advisor",
                user.as_path(),
                SettingValue::Flag(false)
            ),
            ("advisor", local.as_path(), SettingValue::Flag(false)),
            (
                "advisor",
                project.as_path(),
                SettingValue::Text("sol".to_owned())
            ),
            ("advisor", user.as_path(), SettingValue::Flag(false)),
        ]
    );
}

#[test]
fn advisor_profile_selection_overrides_top_level_false() {
    let fixture = Fixture::new(&config("advisor = false", "advisor = \"sol\"", SOL));
    let resolution = fixture
        .resolve()
        .expect("a profile selection beats the default");
    let advisor = resolution
        .config
        .agent
        .profile
        .advisor
        .expect("selected advisor");

    assert_eq!(advisor.value, "sol");
    assert_eq!(advisor.source.key, "profiles.code.advisor");
    let explained = resolution
        .provenance
        .explain("advisor")
        .expect("selection source");
    assert_eq!(explained.source, advisor.source);
    assert_eq!(explained.overridden[0].value, SettingValue::Flag(false));
}

#[test]
fn advisor_defaults_only_apply_to_main_placements() {
    let fixture = Fixture::new(&config("advisor = \"sol\"", "", "use = [\"advisor\"]"));
    let resolution = fixture
        .resolve()
        .expect("an advisor-only target has no main default");

    assert_eq!(
        resolution
            .config
            .agent
            .profile
            .advisor
            .as_ref()
            .unwrap()
            .value,
        "sol"
    );
    assert!(resolution.config.agent.profiles["sol"].advisor.is_none());
    assert_eq!(resolution.config.agent.profile_order.value, ["code"]);
    assert!(resolution.config.agent.child_profile("sol").is_none());
}

#[test]
fn advisor_unset_is_distinct_from_false_in_provenance() {
    let fixture = Fixture::new(&config("", "", SOL));
    let unset = fixture.resolve().expect("no advisor configured");
    assert!(unset.config.agent.profile.advisor.is_none());
    assert!(matches!(
        unset.provenance.explain("advisor"),
        Err(ConfigError::MissingSetting { .. })
    ));

    fixture.write_project("config.local.toml", "advisor = false\n");
    let disabled = fixture.resolve().expect("advisor explicitly disabled");
    assert!(disabled.config.agent.profile.advisor.is_none());
    assert_eq!(
        disabled.provenance.explain("advisor").unwrap().value,
        SettingValue::Flag(false)
    );
    assert_eq!(
        unset.config.agent.profile.revision,
        disabled.config.agent.profile.revision
    );
}

#[test]
fn advisor_changes_are_reflected_in_profile_revisions() {
    let fixture = Fixture::new(&config("", "", SOL));
    let without = fixture.resolve().expect("no advisor");
    fixture.write_project("config.local.toml", "advisor = \"sol\"\n");
    let with = fixture.resolve().expect("an advisor");

    assert_ne!(
        without.config.agent.profile.revision,
        with.config.agent.profile.revision
    );
}

#[test]
fn advisor_file_values_round_trip_and_reject_true_or_wrong_types() {
    let text = config("advisor = \"sol\"", "advisor = false", SOL);
    let file = ConfigFile::parse(&text).expect("advisor file model");
    assert_eq!(
        file.advisor,
        Some(AdvisorSelection::Profile("sol".to_owned()))
    );
    assert_eq!(
        file.profiles["code"].advisor,
        Some(AdvisorSelection::Disabled)
    );
    let serialized = toml::to_string(&file).expect("serialize advisor values");
    assert_eq!(ConfigFile::parse(&serialized).unwrap(), file);

    for prefix in ["", "[profiles.code]\n"] {
        for value in ["true", "7", "[\"sol\"]"] {
            assert!(ConfigFile::parse(&format!("{prefix}advisor = {value}\n")).is_err());
        }
    }
}
