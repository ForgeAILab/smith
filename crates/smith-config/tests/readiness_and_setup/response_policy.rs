use super::*;

#[test]
fn reasoning_only_response_policy_is_typed_optional_and_provenanced() {
    for (spelling, expected) in [
        ("reasoning", ReasoningOnlyBehavior::Reasoning),
        ("text", ReasoningOnlyBehavior::Text),
    ] {
        let fixture = Fixture::new();
        fixture.write_user(&format!(
            r#"
default_profile = "dev"
[profiles.dev]
provider = "remote"
model = "m"
[providers.remote]
kind = "openai-compatible"
base_url = "https://example.test/v1"
[providers.remote.response]
reasoning_only = "{spelling}"
[models."remote/m"]
context_tokens = 100
max_input_tokens = 90
max_output_tokens = 10
"#
        ));
        let resolved = resolve(&fixture.request()).expect("a resolved response policy");
        let policy = resolved
            .config
            .provider
            .response
            .reasoning_only
            .expect("the response policy");
        assert_eq!(policy.value, expected);
        assert_eq!(policy.source.layer, Layer::UserFile);
        assert!(policy.source.key.ends_with("response.reasoning_only"));
    }

    let fixture = Fixture::new();
    fixture.write_user(READY);
    assert!(
        resolve(&fixture.request())
            .expect("an omitted policy")
            .config
            .provider
            .response
            .reasoning_only
            .is_none()
    );
}

#[test]
fn invalid_or_incompatible_response_policy_fails_during_resolution() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "dev"
[profiles.dev]
provider = "remote"
model = "m"
[providers.remote]
kind = "openai-compatible"
base_url = "https://example.test/v1"
[providers.remote.response]
reasoning_only = "visible-ish"
[models."remote/m"]
context_tokens = 100
max_input_tokens = 90
max_output_tokens = 10
"#,
    );
    assert!(matches!(
        resolve(&fixture.request()),
        Err(ConfigError::Malformed { .. })
    ));

    let fixture = Fixture::new();
    fixture.write_user(
        r#"
default_profile = "dev"
[profiles.dev]
provider = "local"
model = "m"
[providers.local]
kind = "fake"
[providers.local.response]
reasoning_only = "text"
[models."local/m"]
context_tokens = 100
max_input_tokens = 90
max_output_tokens = 10
"#,
    );
    let error = resolve(&fixture.request()).expect_err("fake has no reasoning stream");
    assert!(matches!(error, ConfigError::IncompatibleOption { .. }));
    assert!(error.to_string().contains("response.reasoning_only"));
}
