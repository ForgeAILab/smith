use super::*;

#[test]
fn no_setup_intent_is_unconfigured_with_discovered_locations() {
    let fixture = Fixture::new();
    let ConfigReadiness::Unconfigured(context) = inspect(&fixture.request()) else {
        panic!("a fresh user should be unconfigured");
    };
    assert_eq!(
        context.layout.user_dir,
        fixture
            .home
            .path()
            .canonicalize()
            .expect("a canonical home")
            .join(".smith")
    );
    assert!(context.layout.project_root.is_none());

    fixture.write_user(
        r#"
        [approval]
        mode = "deny"
        "#,
    );
    assert!(
        matches!(
            inspect(&fixture.request()),
            ConfigReadiness::Unconfigured(_)
        ),
        "policy without provider/model intent remains first-run state"
    );
}

#[test]
fn ready_inspection_is_the_ordinary_resolution() {
    let fixture = Fixture::new();
    fixture.write_user(READY);
    let expected = resolve(&fixture.request()).expect("ordinary resolution");
    assert_eq!(
        inspect(&fixture.request()),
        ConfigReadiness::Ready(Box::new(expected))
    );
}

#[test]
fn partial_or_malformed_setup_intent_is_invalid() {
    let cases = [
        (
            "provider declaration",
            r#"
            [providers.local]
            kind = "fake"
            "#,
        ),
        (
            "selected provider without model",
            r#"
            default_profile = "dev"
            [profiles.dev]
            provider = "local"
            [providers.local]
            kind = "fake"
            "#,
        ),
        (
            "model without complete limits",
            r#"
            default_profile = "dev"
            [profiles.dev]
            provider = "local"
            model = "example-model"
            [providers.local]
            kind = "fake"
            [models."local/example-model"]
            context_tokens = "many"
            "#,
        ),
        ("malformed file", "default_profile = ["),
    ];

    for (name, text) in cases {
        let fixture = Fixture::new();
        fixture.write_user(text);
        assert!(
            matches!(inspect(&fixture.request()), ConfigReadiness::Invalid(_)),
            "{name} must not be rewritten as first-run setup"
        );
    }
}

#[test]
fn environment_cli_and_session_selection_count_as_setup_intent() {
    let fixture = Fixture::new();
    let cases = [
        fixture.request().with_env([("SMITH_PROVIDER", "missing")]),
        fixture.request().with_cli(Overrides {
            model: Some("example-model".into()),
            ..Overrides::default()
        }),
        fixture.request().with_session(Overrides {
            profile: Some("missing".into()),
            ..Overrides::default()
        }),
    ];

    for request in cases {
        assert!(
            matches!(inspect(&request), ConfigReadiness::Invalid(_)),
            "an explicit selection must not trigger automatic setup"
        );
    }
}

#[test]
fn removing_the_only_ready_configuration_derives_unconfigured_again() {
    let fixture = Fixture::new();
    fixture.write_user(READY);
    assert!(matches!(
        inspect(&fixture.request()),
        ConfigReadiness::Ready(_)
    ));

    fs::remove_file(fixture.home.path().join(".smith/config.toml"))
        .expect("the prior setup was removed");
    assert!(matches!(
        inspect(&fixture.request()),
        ConfigReadiness::Unconfigured(_)
    ));
}

#[test]
fn invalid_state_keeps_the_actionable_resolver_error() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
        default_profile = "dev"
        [profiles.dev]
        provider = "missing"
        model = "m"
        "#,
    );
    let ConfigReadiness::Invalid(error) = inspect(&fixture.request()) else {
        panic!("an unknown selected provider is invalid");
    };
    assert!(matches!(
        error,
        ConfigError::UnusableReference { name, .. } if name == "missing"
    ));
}
