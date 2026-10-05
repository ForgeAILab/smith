use super::*;

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
fn cache_miss_notices_default_to_enabled_and_keep_provenance() {
    let resolution = resolve_project(BASE_PROJECT_CONFIG).expect("a resolved run");

    assert!(resolution.cache_miss_notices.value);
    assert_eq!(resolution.cache_miss_notices.source.layer, Layer::BuiltIn);
    let explanation = resolution
        .provenance
        .explain("cache.miss_notices")
        .expect("the built-in cache notice policy");
    assert_eq!(explanation.value, SettingValue::Flag(true));
    assert_eq!(explanation.source.layer, Layer::BuiltIn);
}

#[test]
fn cache_miss_notices_can_be_disabled_by_a_profile() {
    let fixture = Fixture::new();
    fixture.write_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[profiles.work.cache]\nmiss_notices = false\n"
    ));

    let resolution = resolve(&fixture.request()).expect("a resolved run");
    assert!(!resolution.cache_miss_notices.value);
    assert_eq!(resolution.cache_miss_notices.source.layer, Layer::Profile);
}

#[test]
fn cache_miss_notices_follow_the_normal_file_profile_environment_and_flag_order() {
    let fixture = Fixture::new();
    fixture.write_user("[cache]\nmiss_notices = false\n");
    fixture.write_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[cache]\nmiss_notices = true\n[profiles.work.cache]\nmiss_notices = false\n"
    ));

    let environment = BTreeMap::from([(
        String::from("SMITH_CACHE_MISS_NOTICES"),
        String::from("true"),
    )]);
    let resolution = resolve(
        &fixture.request().with_env(environment).with_cli(Overrides {
            cache_miss_notices: Some(false),
            ..Overrides::default()
        }),
    )
    .expect("a resolved run");

    assert!(!resolution.cache_miss_notices.value);
    assert_eq!(
        resolution.cache_miss_notices.source.layer,
        Layer::CommandLine
    );
    let explanation = resolution
        .provenance
        .explain("cache.miss_notices")
        .expect("the cache notice policy");
    assert_eq!(explanation.value, SettingValue::Flag(false));
    assert_eq!(explanation.source.layer, Layer::CommandLine);
    assert_eq!(explanation.overridden[0].value, SettingValue::Flag(true));
    assert_eq!(explanation.overridden[0].source.layer, Layer::Environment);
}

#[test]
fn cache_miss_notices_can_be_enabled_by_a_profile() {
    let fixture = Fixture::new();
    fixture.write_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[profiles.work.cache]\nmiss_notices = true\n"
    ));

    let resolution = resolve(&fixture.request()).expect("a resolved run");
    assert!(resolution.cache_miss_notices.value);
    assert_eq!(resolution.cache_miss_notices.source.layer, Layer::Profile);
    assert_eq!(
        resolution.cache_miss_notices.source.key,
        "profiles.work.cache.miss_notices"
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
