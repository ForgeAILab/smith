//! The inline evidence threshold participates in normal configuration layering.
use smith_config::resolve::{ResolveRequest, resolve};

const BASE: &str = r#"
default_profile = "dev"
[profiles.dev]
provider = "fake"
model = "example"
[providers.fake]
kind = "fake"
"#;

#[test]
fn default_threshold_is_independent_of_the_legacy_output_limit() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join(".smith")).unwrap();
    std::fs::write(
        project.path().join(".smith/config.toml"),
        format!("{BASE}\n[limits]\ntool_output_limit_bytes = 131072\n"),
    )
    .unwrap();
    let result = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path())).unwrap();
    assert_eq!(result.config.context.tool_output_inline_bytes.value, 8192);
    assert_eq!(result.config.limits.tool_output_limit_bytes.value, 131072);
}

#[test]
fn configured_threshold_is_layered_and_environment_provenance_is_preserved() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join(".smith")).unwrap();
    std::fs::write(
        project.path().join(".smith/config.toml"),
        format!("{BASE}\n[context]\ntool_output_inline_bytes = 16384\n"),
    )
    .unwrap();
    let request = ResolveRequest::new(project.path()).with_home_dir(home.path());
    assert_eq!(
        resolve(&request)
            .unwrap()
            .config
            .context
            .tool_output_inline_bytes
            .value,
        16384
    );
    let result =
        resolve(&request.with_env([("SMITH_CONTEXT_TOOL_OUTPUT_INLINE_BYTES", "4096")])).unwrap();
    assert_eq!(result.config.context.tool_output_inline_bytes.value, 4096);
    assert_eq!(
        result.config.context.tool_output_inline_bytes.source.key,
        "SMITH_CONTEXT_TOOL_OUTPUT_INLINE_BYTES"
    );
}

#[test]
fn unsafe_or_mistyped_thresholds_are_rejected_at_configuration_resolution() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join(".smith")).unwrap();
    for invalid in ["0", "255", "1048577", "-1", "\"8192\"", "true"] {
        std::fs::write(
            project.path().join(".smith/config.toml"),
            format!("{BASE}\n[context]\ntool_output_inline_bytes = {invalid}\n"),
        )
        .unwrap();
        assert!(
            resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path())).is_err(),
            "{invalid}"
        );
    }
}
