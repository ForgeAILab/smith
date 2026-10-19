//! Advisor selection by profile or model, early validation, and source provenance.

use smith_config::model::{AdvisorSelection, ConfigFile, ProfileUse};
use smith_config::resolve::{
    AdvisorOverride, AdvisorTarget, ConfigError, Layer, Overrides, ReferenceKind, Resolution,
    ResolveRequest, SettingValue, resolve,
};
use tempfile::TempDir;

const SOL: &str = "use = [\"main\", \"child\"]";

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

    /// Resolves the advisor binding the main resolution selected.
    fn resolve_route(&self, main: &Resolution) -> Result<Resolution, ConfigError> {
        let target = main
            .config
            .agent
            .profile
            .advisor
            .clone()
            .expect("main resolution selects an advisor");
        resolve(&self.request().with_advisor_route(target))
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

fn profile(name: &str) -> AdvisorTarget {
    AdvisorTarget::Profile(name.to_owned())
}

fn model(provider: &str, model: &str) -> AdvisorTarget {
    AdvisorTarget::Model {
        provider: provider.to_owned(),
        model: model.to_owned(),
    }
}

#[test]
fn advisor_top_level_default_resolves_for_main_profiles() {
    let fixture = Fixture::new(&config("advisor = \"sol\"", "", SOL));
    let resolution = fixture.resolve().expect("a main profile with an advisor");
    let agent = &resolution.config.agent;
    let advisor = agent.profile.advisor.as_ref().expect("resolved advisor");

    assert_eq!(agent.profile.name, "code");
    assert_eq!(advisor.value, profile("sol"));
    assert_eq!(advisor.source.layer, Layer::ProjectFile);
    assert_eq!(advisor.source.key, "advisor");
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
fn advisor_profile_needs_no_placement() {
    for sol in ["use = [\"main\"]", "use = [\"child\"]", ""] {
        let fixture = Fixture::new(&config("advisor = \"sol\"", "", sol));
        let main = fixture.resolve().expect("any profile may advise");
        assert_eq!(
            main.config.agent.profile.advisor.as_ref().unwrap().value,
            profile("sol")
        );

        let route = fixture
            .resolve_route(&main)
            .expect("the advisor route resolves whatever the placements");
        assert_eq!(route.config.agent.profile.name, "sol");
        assert_eq!(route.config.provider.name.value, "acme");
        assert_eq!(route.config.model.value, "advisor-model");
        assert!(route.config.agent.profile.advisor.is_none());
    }
}

#[test]
fn advisor_placement_spelling_is_rejected() {
    let fixture = Fixture::new(&config("", "", "use = [\"main\", \"child\", \"advisor\"]"));
    let error = fixture
        .resolve()
        .expect_err("advisor is no longer a placement");

    match error {
        ConfigError::Malformed { message, .. } => {
            assert!(
                message.contains("unknown variant `advisor`, expected `main` or `child`"),
                "{message}"
            );
        }
        other => panic!("expected an invalid placement, got {other:?}"),
    }
}

#[test]
fn advisor_profile_route_beats_the_main_profile_selection() {
    let fixture = Fixture::new(&config("advisor = \"sol\"", "", SOL));
    let main = fixture.resolve().expect("main selects sol");
    let target = main.config.agent.profile.advisor.clone().unwrap();
    let route = resolve(
        &fixture
            .request()
            .with_cli(Overrides {
                profile: Some("code".to_owned()),
                ..Overrides::default()
            })
            .with_advisor_route(target),
    )
    .expect("the route selects its own profile");

    assert_eq!(route.config.agent.profile.name, "sol");
    assert_eq!(route.config.model.value, "advisor-model");
}

#[test]
fn advisor_model_reference_resolves_without_any_profile_layer() {
    let fixture = Fixture::new(&config(
        "advisor = \"acme/review-model\"",
        "max_output_tokens = 8192\ninstructions = \"You are the code agent.\"\n[profiles.code.reasoning]\neffort = \"low\"",
        SOL,
    ));
    let main = fixture.resolve().expect("a model advisor");
    let advisor = main.config.agent.profile.advisor.as_ref().unwrap();
    assert_eq!(advisor.value, model("acme", "review-model"));
    assert_eq!(advisor.source.key, "advisor");
    assert_eq!(
        main.config
            .max_output_tokens
            .as_ref()
            .map(|value| value.value),
        Some(8192)
    );

    let route = fixture.resolve_route(&main).expect("the model route");
    assert_eq!(route.config.provider.name.value, "acme");
    assert_eq!(route.config.model.value, "review-model");
    assert_eq!(route.config.model.source.key, "advisor");
    assert!(route.config.agent.profile.instructions.is_none());
    assert!(route.config.agent.profile.advisor.is_none());
    assert_ne!(
        route
            .config
            .max_output_tokens
            .as_ref()
            .map(|value| value.value),
        Some(8192),
        "the main profile's output cap must not reach the advisor"
    );
    assert!(route.config.reasoning.effort.is_none());
}

#[test]
fn advisor_model_reference_beats_main_session_provider_and_model_overrides() {
    let fixture = Fixture::new(&format!(
        "{}\n[providers.other]\nkind = \"openai-compatible\"\nbase_url = \"https://other.example.test/v1\"\n",
        config("advisor = \"acme/review-model\"", "", SOL)
    ));
    let main = fixture.resolve().expect("a model advisor");
    let target = main.config.agent.profile.advisor.clone().unwrap();
    let route = resolve(
        &fixture
            .request()
            .with_env([("SMITH_PROVIDER", "other"), ("SMITH_MODEL", "env-model")])
            .with_advisor_route(target),
    )
    .expect("the advisor binding wins");

    assert_eq!(route.config.provider.name.value, "acme");
    assert_eq!(route.config.model.value, "review-model");
}

#[test]
fn advisor_model_reference_keeps_slashes_after_the_provider() {
    let fixture = Fixture::new(&config("advisor = \"acme/vendor/large\"", "", SOL));
    let main = fixture.resolve().expect("a namespaced model");
    assert_eq!(
        main.config.agent.profile.advisor.as_ref().unwrap().value,
        model("acme", "vendor/large")
    );
    let route = fixture.resolve_route(&main).expect("the namespaced route");
    assert_eq!(route.config.model.value, "vendor/large");
}

#[test]
fn advisor_model_reference_requires_a_declared_provider_before_credentials() {
    let text = config("advisor = \"missing/review-model\"", "", SOL)
        .replace("file:must-not-be-read", "invalid-credential");
    let fixture = Fixture::new(&text);
    let error = fixture.resolve().expect_err("an unknown provider");

    match error {
        ConfigError::UnusableReference {
            source, what, name, ..
        } => {
            assert_eq!(source.key, "advisor");
            assert_eq!(what, ReferenceKind::Provider);
            assert_eq!(name, "missing");
        }
        other => panic!("expected an unknown provider, got {other:?}"),
    }
}

#[test]
fn advisor_model_reference_naming_the_main_binding_is_skipped() {
    let fixture = Fixture::new(&config("advisor = \"acme/advisor-model\"", "", SOL));
    let code = fixture.resolve().expect("code consults sol's model");
    assert_eq!(
        code.config.agent.profile.advisor.as_ref().unwrap().value,
        model("acme", "advisor-model")
    );

    let sol = resolve(&fixture.request().with_cli(Overrides {
        profile: Some("sol".to_owned()),
        ..Overrides::default()
    }))
    .expect("sol never consults its own model");
    assert!(sol.config.agent.profile.advisor.is_none());

    let overridden = resolve(&fixture.request().with_cli(Overrides {
        model: Some("advisor-model".to_owned()),
        ..Overrides::default()
    }))
    .expect("a main override to the advisor's model skips it");
    assert_eq!(overridden.config.agent.profile.name, "code");
    assert!(overridden.config.agent.profile.advisor.is_none());
}

#[test]
fn advisor_rejects_malformed_values() {
    for value in ["\"\"", "\"/model\"", "\"acme/\""] {
        let fixture = Fixture::new(&config(&format!("advisor = {value}"), "", SOL));
        let error = fixture.resolve().expect_err("a malformed advisor");
        match error {
            ConfigError::InvalidValue { source, message } => {
                assert_eq!(source.key, "advisor");
                assert!(
                    message.contains("a profile name, `provider/model`, or `false`"),
                    "{message}"
                );
            }
            other => panic!("expected a malformed advisor, got {other:?}"),
        }
    }
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
    fixture.write_project("config.local.toml", "advisor = \"acme/review-model\"\n");
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

    assert_eq!(advisor.value, model("acme", "review-model"));
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
fn advisor_unknown_profile_is_rejected_before_provider_and_credential_validation() {
    let text = config("advisor = \"sool\"", "", SOL)
        .replace(
            "kind = \"openai-compatible\"",
            "kind = \"invalid-provider\"",
        )
        .replace("file:must-not-be-read", "invalid-credential");
    let fixture = Fixture::new(&text);
    let error = fixture.resolve().expect_err("an unknown advisor");

    match error {
        ConfigError::UnusableReference {
            source,
            what,
            name,
            suggestions,
        } => {
            assert_eq!(source.key, "advisor");
            assert_eq!(what, ReferenceKind::Profile);
            assert_eq!(name, "sool");
            assert_eq!(suggestions, ["sol"]);
        }
        other => panic!("expected an unknown profile before provider resolution, got {other:?}"),
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
            assert_eq!(advisor.value, profile("sol"));
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
fn advisor_profile_may_select_another_advisor_for_main_use() {
    for sol in [
        "use = [\"main\", \"child\"]\nadvisor = \"other\"",
        "use = [\"main\", \"child\"]\nextends = \"mentor\"",
    ] {
        let fixture = Fixture::new(&format!(
            "{}\n[profiles.mentor]\nadvisor = \"other\"\n[profiles.other]\nuse = [\"main\"]\n",
            config("advisor = \"sol\"", "", sol)
        ));
        let code = fixture
            .resolve()
            .expect("the target's main advisor is valid");
        assert_eq!(
            code.config.agent.profile.advisor.as_ref().unwrap().value,
            profile("sol")
        );
        assert_eq!(
            code.config.agent.profiles["sol"]
                .advisor
                .as_ref()
                .unwrap()
                .value,
            profile("other")
        );
        assert_eq!(
            code.config.agent.profiles["other"]
                .advisor
                .as_ref()
                .unwrap()
                .value,
            profile("sol")
        );

        let request = fixture.request().with_cli(Overrides {
            profile: Some("sol".to_owned()),
            ..Overrides::default()
        });
        let main = resolve(&request).expect("sol may consult other when used as main");
        assert_eq!(main.config.agent.profile.name, "sol");
        assert_eq!(
            main.config.agent.profile.advisor.as_ref().unwrap().value,
            profile("other")
        );

        let route = fixture
            .resolve_route(&code)
            .expect("sol's own selection is ignored while advising");
        assert_eq!(route.config.agent.profile.name, "sol");
        assert!(route.config.agent.profile.advisor.is_none());
        assert_eq!(
            route.config.agent.profiles["sol"]
                .advisor
                .as_ref()
                .unwrap()
                .value,
            profile("other")
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
        profile("sol")
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
        profile("sol")
    );
}

#[test]
fn advisor_explicit_self_selection_is_rejected_including_inherited_selection() {
    for (code, sol, extra, key) in [
        ("advisor = \"code\"", SOL, "", "profiles.code.advisor"),
        (
            "",
            "use = [\"main\", \"child\"]\nadvisor = \"sol\"",
            "",
            "profiles.sol.advisor",
        ),
        (
            "",
            "use = [\"main\", \"child\"]\nextends = \"mentor\"",
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

    assert_eq!(advisor.value, profile("sol"));
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
    let fixture = Fixture::new(&format!(
        "{}\n[profiles.helper]\nuse = [\"child\"]\n",
        config("advisor = \"sol\"", "", SOL)
    ));
    let resolution = fixture
        .resolve()
        .expect("a child-only profile has no main default");

    assert_eq!(
        resolution
            .config
            .agent
            .profile
            .advisor
            .as_ref()
            .unwrap()
            .value,
        profile("sol")
    );
    assert!(resolution.config.agent.profiles["helper"].advisor.is_none());
    assert_eq!(
        resolution.config.agent.profiles["helper"].uses.value,
        [ProfileUse::Child]
    );
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
    let with_profile = fixture.resolve().expect("a profile advisor");
    fixture.write_project("config.local.toml", "advisor = \"acme/advisor-model\"\n");
    let with_model = fixture.resolve().expect("a model advisor");

    assert_ne!(
        without.config.agent.profile.revision,
        with_profile.config.agent.profile.revision
    );
    assert_ne!(
        with_profile.config.agent.profile.revision,
        with_model.config.agent.profile.revision
    );
}

#[test]
fn profile_revisions_do_not_depend_on_where_the_config_file_lives() {
    let text = config("advisor = \"sol\"", "", SOL);
    let here = Fixture::new(&text);
    let there = Fixture::new(&text);
    assert_ne!(here.project.path(), there.project.path());
    let here_user = Fixture::new("");
    here_user.write_user(&text);
    let there_user = Fixture::new("");
    there_user.write_user(&text);

    let revision = |fixture: &Fixture| {
        fixture
            .resolve()
            .expect("the shared configuration")
            .config
            .agent
            .profile
            .revision
    };
    // A moved home or project keeps every child route a session recorded.
    assert_eq!(revision(&here), revision(&there));
    assert_eq!(revision(&here_user), revision(&there_user));
    // The layer still counts: a project may not stand in for the user.
    assert_ne!(revision(&here), revision(&here_user));

    here.write_project("config.local.toml", "advisor = false\n");
    assert_ne!(revision(&here), revision(&there));
}

#[test]
fn advisor_target_parsing_splits_on_the_first_slash() {
    assert_eq!(AdvisorTarget::parse("sol"), Some(profile("sol")));
    assert_eq!(
        AdvisorTarget::parse("openrouter/openai/gpt-4o-mini"),
        Some(model("openrouter", "openai/gpt-4o-mini"))
    );
    for invalid in ["", "/", "/model", "provider/"] {
        assert_eq!(AdvisorTarget::parse(invalid), None, "{invalid}");
    }
    assert_eq!(
        model("chatgpt", "gpt-6.1-sol").to_string(),
        "chatgpt/gpt-6.1-sol"
    );
}

#[test]
fn advisor_file_values_round_trip_and_reject_true_or_wrong_types() {
    let text = config("advisor = \"acme/review-model\"", "advisor = false", SOL);
    let file = ConfigFile::parse(&text).expect("advisor file model");
    assert_eq!(
        file.advisor,
        Some(AdvisorSelection::Target("acme/review-model".to_owned()))
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

#[test]
fn a_session_override_turns_the_configured_advisor_off() {
    let fixture = Fixture::new(&config("advisor = \"sol\"", "", SOL));
    let configured = fixture.resolve().expect("configured advisor");
    let resolution = resolve(
        &fixture
            .request()
            .with_advisor_override(AdvisorOverride::Off),
    )
    .expect("an override never invalidates configuration");
    let agent = &resolution.config.agent;

    assert!(agent.profile.advisor.is_none());
    assert!(agent.advisor_overridden);
    assert_eq!(
        agent
            .configured_advisor
            .as_ref()
            .map(|advisor| &advisor.value),
        Some(&profile("sol"))
    );
    assert_ne!(
        agent.profile.revision, configured.config.agent.profile.revision,
        "the tool list and guidance changed, so the profile identity must too"
    );
}

#[test]
fn a_session_override_selects_another_advisor_with_session_provenance() {
    let fixture = Fixture::new(&config("", "", SOL));
    let unset = fixture.resolve().expect("no advisor configured");
    assert!(unset.config.agent.profile.advisor.is_none());
    assert!(!unset.config.agent.advisor_overridden);

    for target in [profile("sol"), model("acme", "advisor-model")] {
        let resolution = resolve(
            &fixture
                .request()
                .with_advisor_override(AdvisorOverride::Target(target.clone())),
        )
        .expect("a resolvable override");
        let advisor = resolution
            .config
            .agent
            .profile
            .advisor
            .as_ref()
            .expect("the override is effective");
        assert_eq!(advisor.value, target);
        assert_eq!(advisor.source.layer, Layer::SessionOverride);
        assert!(resolution.config.agent.configured_advisor.is_none());
    }
}

#[test]
fn a_session_override_naming_itself_consults_nobody() {
    let fixture = Fixture::new(&config("advisor = \"sol\"", "", SOL));
    for target in [profile("code"), model("acme", "working-model")] {
        let resolution = resolve(
            &fixture
                .request()
                .with_advisor_override(AdvisorOverride::Target(target)),
        )
        .expect("landing on the override's own target must not fail the rebuild");
        assert!(resolution.config.agent.profile.advisor.is_none());
        assert!(resolution.config.agent.advisor_overridden);
    }
}

#[test]
fn a_session_override_that_cannot_resolve_is_refused() {
    let fixture = Fixture::new(&config("advisor = \"sol\"", "", SOL));
    for target in [profile("missing"), model("nowhere", "model")] {
        let error = resolve(
            &fixture
                .request()
                .with_advisor_override(AdvisorOverride::Target(target)),
        )
        .expect_err("an unknown target");
        assert!(
            matches!(error, ConfigError::UnusableReference { .. }),
            "{error}"
        );
    }
}

#[test]
fn a_session_override_does_not_reach_child_or_advisor_resolutions() {
    let fixture = Fixture::new(&config("advisor = \"sol\"", "", SOL));
    let main = fixture.resolve().expect("configured advisor");
    let route = resolve(
        &fixture
            .request()
            .with_advisor_route(main.config.agent.profile.advisor.clone().unwrap())
            .with_advisor_override(AdvisorOverride::Target(profile("code"))),
    )
    .expect("the advisor route ignores the override");
    assert!(route.config.agent.profile.advisor.is_none());
    assert!(!route.config.agent.advisor_overridden);

    let child = resolve(
        &fixture
            .request()
            .with_cli(Overrides {
                profile: Some("sol".to_owned()),
                ..Overrides::default()
            })
            .with_profile_use(ProfileUse::Child)
            .with_advisor_override(AdvisorOverride::Target(profile("code"))),
    )
    .expect("a child resolution");
    assert!(!child.config.agent.advisor_overridden);
}

#[test]
fn profile_capability_limits_resolve_with_their_source() {
    let fixture = Fixture::new(&config(
        "",
        "[profiles.code.capabilities]\nallow = [\"tool:*\"]\ndeny = [\"tool:shell\"]",
        SOL,
    ));
    let unlimited = Fixture::new(&config("", "", SOL))
        .resolve()
        .expect("no limits");
    let resolution = fixture.resolve().expect("valid limits");
    let limits = &resolution.config.agent.profile.capabilities;

    assert_eq!(limits.allow.as_ref().unwrap().value, ["tool:*"]);
    assert_eq!(limits.deny.as_ref().unwrap().value, ["tool:shell"]);
    assert_eq!(limits.deny.as_ref().unwrap().source.layer, Layer::Profile);
    assert!(unlimited.config.agent.profile.capabilities.is_empty());
    assert_ne!(
        resolution.config.agent.profile.revision, unlimited.config.agent.profile.revision,
        "limits change the tool surface, so they change the profile identity"
    );
}

#[test]
fn an_invalid_capability_pattern_fails_resolution_with_its_source() {
    for (pattern, reason) in [
        ("shell", "is not `<domain>:<name>`"),
        ("tools:shell", "is not a capability domain"),
        ("tool:", "needs a capability name"),
        ("tool:a:b", "needs a capability name"),
    ] {
        let fixture = Fixture::new(&config(
            "",
            &format!("[profiles.code.capabilities]\ndeny = [\"{pattern}\"]"),
            SOL,
        ));
        let error = fixture.resolve().expect_err("an invalid pattern");
        assert!(
            matches!(error, ConfigError::InvalidValue { .. }),
            "{pattern}: {error}"
        );
        assert!(error.to_string().contains(reason), "{pattern}: {error}");
    }
}

#[test]
fn advisor_installed_agent_targets_round_trip_before_the_provider_split() {
    for (kind, model) in [
        ("claude-code", "opus"),
        ("codex", "gpt-6-sol"),
        ("codex", "vendor/model"),
    ] {
        let id = format!("cli/{kind}/{model}");
        let target = AdvisorTarget::parse(&id).expect("installed agent");
        assert_eq!(
            target,
            AdvisorTarget::InstalledAgent {
                kind: kind.to_owned(),
                model: model.to_owned()
            }
        );
        assert_eq!(target.to_string(), id);
    }
    assert_eq!(
        AdvisorTarget::parse("cli/emacs/sonnet"),
        Some(model("cli", "emacs/sonnet"))
    );
}

#[test]
fn advisor_installed_agent_resolves_with_main_provider_and_top_level_settings() {
    for (global, profile) in [
        ("advisor = \"cli/claude-code/opus\"", ""),
        ("", "advisor = \"cli/claude-code/opus\""),
    ] {
        let fixture = Fixture::new(&config(
            &format!("{global}\n[context]\nreasoning_reserve = 512\n"),
            &format!("{profile}\nmax_output_tokens = 8192\ninstructions = \"MAIN_ONLY\""),
            SOL,
        ));
        fixture.write_user("[harness.claude-code]\nargs = [\"--fixture\"]\n");
        let main = fixture.resolve().expect("installed advisor selection");
        let mut request = fixture.request();
        request.session.provider = Some(main.config.provider.name.value.clone());
        let advisor = resolve(
            &request.with_advisor_route(main.config.agent.profile.advisor.clone().unwrap()),
        )
        .expect("installed advisor route");
        assert_eq!(advisor.config.provider.name.value, "acme");
        assert_eq!(advisor.config.model.value, "cli/claude-code/opus");
        assert_ne!(
            advisor
                .config
                .max_output_tokens
                .as_ref()
                .map(|value| value.value),
            Some(8192)
        );
        assert_eq!(advisor.config.context.reasoning_reserve.value, 512);
        assert!(advisor.config.agent.profile.instructions.is_none());
        let harness = advisor.config.harness.unwrap();
        assert_eq!(harness.kind.value, "claude-code");
        assert_eq!(harness.model.value, "opus");
        assert_eq!(harness.args, ["--fixture"]);
    }
}

#[test]
fn advisor_installed_agent_profile_resolves_its_own_instructions_and_provider() {
    let fixture = Fixture::new(
        &config(
            "advisor = \"sol\"",
            "",
            "model = \"cli/codex/gpt-6-sol\"\ninstructions = \"REVIEWER\"",
        )
        .replace("model = \"advisor-model\"\n", ""),
    );
    let main = fixture.resolve().expect("profile advisor selection");
    let advisor = fixture
        .resolve_route(&main)
        .expect("profile installed advisor route");
    assert_eq!(advisor.config.provider.name.value, "acme");
    assert_eq!(advisor.config.model.value, "cli/codex/gpt-6-sol");
    assert_eq!(
        advisor.config.agent.profile.instructions.unwrap().value,
        "REVIEWER"
    );
    assert_eq!(advisor.config.harness.unwrap().kind.value, "codex");
}

#[test]
fn advisor_installed_agent_skips_the_main_model_including_session_overrides() {
    let fixture = Fixture::new(
        &config("advisor = \"cli/claude-code/opus\"", "", SOL)
            .replace("working-model", "cli/claude-code/opus"),
    );
    for request in [
        fixture.request(),
        fixture
            .request()
            .with_advisor_override(AdvisorOverride::Target(
                AdvisorTarget::parse("cli/claude-code/opus").unwrap(),
            )),
    ] {
        let main = resolve(&request).expect("self advice is skipped");
        assert!(main.config.agent.profile.advisor.is_none());
    }
}

#[test]
fn advisor_unknown_installed_kind_is_an_unknown_provider_before_credentials() {
    let fixture = Fixture::new(&config("advisor = \"cli/emacs/sonnet\"", "", SOL));
    let error = fixture.resolve().unwrap_err();
    assert!(
        matches!(error, ConfigError::UnusableReference { what: ReferenceKind::Provider, ref name, .. } if name == "cli")
    );
    assert!(error.to_string().contains("advisor"));
}

#[test]
fn advisor_installed_agent_is_rejected_for_vision_directly_or_through_a_profile() {
    for vision in ["cli/claude-code/opus", "sol"] {
        let fixture = Fixture::new(
            &config(&format!("vision = \"{vision}\""), "", SOL)
                .replace("advisor-model", "cli/claude-code/opus"),
        );
        let error = fixture.resolve().unwrap_err().to_string();
        assert!(error.contains("vision"), "{error}");
        assert!(
            error.contains("installed agent cannot serve image requests"),
            "{error}"
        );
    }
}
