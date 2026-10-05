use super::*;

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
