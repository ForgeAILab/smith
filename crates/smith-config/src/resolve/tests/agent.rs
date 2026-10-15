#[test]
fn invalid_agent_profile_name_reports_grammatical_message() {
    let error = crate::resolve::agent::validate_agent_name(
        &"x".repeat(33),
        &Source::built_in("profiles"),
        "agent profile",
    )
    .expect_err("an overlong agent profile name");

    match error {
        ConfigError::InvalidValue { message, .. } => assert_eq!(
            message,
            "the agent profile name must contain 1 to 32 ASCII letters, digits, `-`, or `_`"
        ),
        other => panic!("expected an invalid-value error, got {other:?}"),
    }
}
