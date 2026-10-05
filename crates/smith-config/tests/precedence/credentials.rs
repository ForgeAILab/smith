use super::*;

#[test]
fn a_credential_written_in_plain_text_is_refused_and_a_reference_is_not() {
    let inline = [
        "sk-not-a-real-key",
        "",
        "keychain:",
        "vault:smith/acme",
        "smith/acme",
    ];
    for value in inline {
        let error = resolve_project(&format!(
            r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "{value}"
"#
        ))
        .expect_err("a plaintext secret");
        match error {
            ConfigError::PlaintextSecret { ref source, .. } => {
                assert_eq!(source.key, "providers.acme.credential");
            }
            other => panic!("expected a plaintext-secret error for `{value}`, got {other:?}"),
        }
    }

    for value in [
        "keychain:smith/acme",
        "authfile:chatgpt",
        "env:ACME_API_KEY",
        "file:/keys/acme",
    ] {
        let resolution = resolve_project(&format!(
            r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "{value}"
"#
        ))
        .unwrap_or_else(|err| panic!("`{value}` should resolve: {err}"));
        // The reference is carried as written; nothing here reads its value.
        assert_eq!(
            resolution
                .config
                .provider
                .credential()
                .expect("a reference")
                .value,
            value
        );
    }
}

#[cfg(unix)]
#[test]
fn an_owner_only_user_api_key_resolves_and_every_public_render_is_redacted() {
    const SECRET: &str = "sk-inline-resolution-must-not-render";
    let fixture = Fixture::new();
    fixture.write_private_user(&format!(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
api_key = "{SECRET}"
"#
    ));

    let resolution = resolve(&fixture.request()).expect("an inline-key configuration");
    let api_key = resolution
        .config
        .provider
        .api_key
        .as_ref()
        .expect("a resolved inline key");
    assert_eq!(api_key.value.expose(), SECRET);
    assert!(resolution.config.provider.credential().is_none());
    assert_eq!(api_key.source.layer, Layer::UserFile);
    assert_eq!(api_key.source.key, "providers.acme.api_key");

    let explanation = resolution
        .provenance
        .explain("providers.acme.api_key")
        .expect("inline-key provenance");
    assert_eq!(explanation.value.to_string(), "[redacted]");
    for rendered in [
        format!("{resolution:?}"),
        format!("{:?}", resolution.config),
        format!("{:?}", resolution.provenance),
        format!("{explanation:?}"),
        explanation.value.to_string(),
        format!(
            "{:?}",
            local_inventory(&resolution, &["openai-compatible"]).expect("a local inventory")
        ),
    ] {
        assert!(!rendered.contains(SECRET), "{rendered}");
    }
}

#[cfg(unix)]
#[test]
fn checkpoint_key_sources_are_private_redacted_and_mutually_exclusive() {
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    fixture.write_private_user(&format!("[persistence]\ncheckpoint_key = \"{KEY}\"\n"));

    let resolution = resolve(&fixture.request()).expect("an owner-only checkpoint key");
    let key = resolution
        .config
        .persistence
        .checkpoint_key
        .as_ref()
        .expect("the configured key");
    assert_eq!(key.value.expose(), KEY);
    assert_eq!(key.source.layer, Layer::UserFile);
    let explanation = resolution
        .provenance
        .explain("persistence.checkpoint_key")
        .expect("checkpoint-key provenance");
    assert_eq!(explanation.value.to_string(), "[redacted]");
    for rendered in [
        format!("{resolution:?}"),
        format!("{:?}", resolution.config),
        format!("{explanation:?}"),
    ] {
        assert!(!rendered.contains(KEY), "{rendered}");
    }

    let mut env = BTreeMap::new();
    env.insert("SMITH_CHECKPOINT_KEY".to_owned(), KEY.to_owned());
    let env_resolution = resolve(&fixture.request().with_env(env.clone()))
        .expect("the environment key overrides the inline value");
    assert_eq!(
        env_resolution
            .config
            .persistence
            .checkpoint_key
            .as_ref()
            .expect("environment key")
            .source
            .layer,
        Layer::Environment
    );

    fixture.write_private_user(
        "[persistence]\ncheckpoint_key_credential = \"env:SMITH_CHECKPOINT_SECRET\"\n",
    );
    let error = resolve(&fixture.request().with_env(env))
        .expect_err("direct and referenced checkpoint keys are mutually exclusive");
    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(!error.to_string().contains(KEY), "{error}");
}

#[test]
fn project_checkpoint_key_sources_are_rejected_before_use() {
    const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    for setting in [
        format!("checkpoint_key = \"{KEY}\""),
        "checkpoint_key_credential = \"env:CHECKPOINT_KEY\"".to_owned(),
    ] {
        let fixture = Fixture::new();
        fixture.write_project(&format!(
            "{BASE_PROJECT_CONFIG}\n[persistence]\n{setting}\n"
        ));
        let error = resolve(&fixture.request()).expect_err("project-controlled checkpoint key");
        assert!(matches!(error, ConfigError::InvalidValue { .. }));
        assert!(!format!("{error:?} {error}").contains(KEY));
    }
}

#[cfg(unix)]
#[test]
fn project_inline_keys_are_refused_without_rendering_the_value() {
    const SECRET: &str = "sk-project-inline-must-not-render";
    for local in [false, true] {
        let fixture = Fixture::new();
        let text = format!(
            r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
api_key = "{SECRET}"
"#
        );
        if local {
            fixture.write_project(BASE_PROJECT_CONFIG);
            fixture.write_project_local(&text);
        } else {
            fixture.write_project(&text);
        }

        let error = resolve(&fixture.request()).expect_err("a project inline key");
        assert!(matches!(error, ConfigError::PlaintextSecret { .. }));
        assert!(!error.to_string().contains(SECRET), "{error}");
        assert!(!format!("{error:?}").contains(SECRET), "{error:?}");
    }
}

#[cfg(unix)]
#[test]
fn unsafe_user_config_files_with_inline_keys_are_refused_without_a_leak() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    const SECRET: &str = "sk-unsafe-file-must-not-render";
    let text = format!(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
api_key = "{SECRET}"
"#
    );

    let permissive = Fixture::new();
    permissive.write_user(&text);
    let path = permissive.home.path().join(".smith/config.toml");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("permissive test config");
    let error = resolve(&permissive.request()).expect_err("a permissive inline-key file");
    assert!(matches!(error, ConfigError::PlaintextSecret { .. }));
    assert!(!format!("{error:?}").contains(SECRET), "{error:?}");

    let linked = Fixture::new();
    let target = linked.home.path().join("actual-config.toml");
    std::fs::write(&target, &text).expect("symlink target");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
        .expect("private target");
    symlink(&target, linked.home.path().join(".smith/config.toml")).expect("user config symlink");
    let error = resolve(&linked.request()).expect_err("a symlinked inline-key file");
    assert!(matches!(error, ConfigError::PlaintextSecret { .. }));
    assert!(!format!("{error:?}").contains(SECRET), "{error:?}");
}

#[cfg(unix)]
#[test]
fn two_credential_sources_and_empty_inline_keys_fail_without_rendering_values() {
    const SECRET: &str = "sk-conflicting-source-must-not-render";
    let fixture = Fixture::new();
    fixture.write_private_user(&format!(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "env:ACME_API_KEY"
api_key = "{SECRET}"
"#
    ));
    let error = resolve(&fixture.request()).expect_err("two credential sources");
    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(error.to_string().contains("credential"));
    assert!(error.to_string().contains("api_key"));
    assert!(!format!("{error:?}").contains(SECRET), "{error:?}");

    fixture.write_private_user(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
api_key = ""
"#,
    );
    let error = resolve(&fixture.request()).expect_err("an empty inline key");
    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(error.to_string().contains("cannot be empty"), "{error}");
}

#[test]
fn an_authorization_header_is_refused_as_a_plaintext_secret() {
    let error = resolve_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"

[providers.acme.headers]
Authorization = "Bearer sk-not-a-real-key"
"#,
    )
    .expect_err("a plaintext secret");

    match error {
        ConfigError::PlaintextSecret { ref source, .. } => {
            assert_eq!(source.key, "providers.acme.headers.Authorization");
        }
        other => panic!("expected a plaintext-secret error, got {other:?}"),
    }
    assert!(error.to_string().contains("credential"), "{error}");
}

#[test]
fn provider_options_must_suit_the_adapter_kind() {
    let error = resolve_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "scripted"
model = "example-model"

[providers.scripted]
kind = "fake"
base_url = "https://api.example.test/v1"
"#,
    )
    .expect_err("an incompatible option");
    match error {
        ConfigError::IncompatibleOption {
            ref source,
            ref kind,
            ..
        } => {
            assert_eq!(kind, "fake");
            assert_eq!(source.key, "providers.scripted.base_url");
        }
        other => panic!("expected an incompatible-option error, got {other:?}"),
    }

    let missing = resolve_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
credential = "keychain:smith/acme"
"#,
    )
    .expect_err("a missing endpoint");
    match missing {
        ConfigError::MissingSetting { ref key, .. } => {
            assert_eq!(key, "providers.acme.base_url");
        }
        other => panic!("expected a missing-setting error, got {other:?}"),
    }
}

#[test]
fn native_google_provider_owns_its_endpoint_and_headers() {
    let resolution = resolve_project(
        r#"
default_profile = "gemini"

[profiles.gemini]
provider = "google"
model = "gemini-3.6-flash"

[providers.google]
kind = "gemini-interactions"
credential = "env:GEMINI_API_KEY"
"#,
    )
    .expect("native Google config without a user endpoint");
    assert_eq!(resolution.config.provider.name.value, "google");
    assert_eq!(resolution.config.provider.base_url, None);

    let endpoint = resolve_project(
        r#"
default_profile = "gemini"
[profiles.gemini]
provider = "google"
model = "gemini-3.6-flash"
[providers.google]
kind = "gemini-interactions"
base_url = "https://proxy.example.test/v1beta"
credential = "env:GEMINI_API_KEY"
"#,
    )
    .expect_err("native Google does not accept endpoint overrides");
    assert!(endpoint.to_string().contains("does not accept `base_url`"));

    let header = resolve_project(
        r#"
default_profile = "gemini"
[profiles.gemini]
provider = "google"
model = "gemini-3.6-flash"
[providers.google]
kind = "gemini-interactions"
credential = "env:GEMINI_API_KEY"
[providers.google.headers]
X-Trace = "enabled"
"#,
    )
    .expect_err("native Google does not accept custom headers");
    assert!(
        header
            .to_string()
            .contains("does not accept custom headers")
    );
}

/// A project declaring `acme` with `settings` appended to the provider table.
fn resolve_provider_with(settings: &str) -> Result<Resolution, ConfigError> {
    resolve_project(&format!(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
{settings}
"#
    ))
}

#[test]
fn a_credential_pool_resolves_in_declared_order_with_its_own_provenance() {
    let resolution = resolve_provider_with(
        r#"credentials = ["keychain:smith/personal", "keychain:smith/work"]"#,
    )
    .expect("a resolved pool");
    let provider = &resolution.config.provider;

    let references: Vec<&str> = provider
        .credentials
        .iter()
        .map(|entry| entry.value.as_str())
        .collect();
    assert_eq!(
        references,
        ["keychain:smith/personal", "keychain:smith/work"]
    );
    // The first entry is where a session with no persisted choice starts.
    assert_eq!(
        provider.credential().expect("an active member").value,
        "keychain:smith/personal"
    );
    assert!(provider.has_pool());
    for entry in &provider.credentials {
        assert_eq!(entry.source.key, "providers.acme.credentials");
        assert_eq!(entry.source.layer, Layer::ProjectFile);
    }
}

#[test]
fn a_single_credential_resolves_as_a_pool_of_one() {
    let resolution =
        resolve_provider_with(r#"credential = "env:ACME_API_KEY""#).expect("a resolved provider");
    let provider = &resolution.config.provider;

    // The legacy spelling needs no migration and produces no warning: it is
    // the same declaration, for one account.
    assert_eq!(provider.credentials.len(), 1);
    assert_eq!(
        provider.credential().expect("an active member").value,
        "env:ACME_API_KEY"
    );
    assert_eq!(
        provider.credentials[0].source.key,
        "providers.acme.credential"
    );
    // One account is not a pool: there is nowhere to rotate to.
    assert!(!provider.has_pool());
}

#[test]
fn an_unparseable_pool_entry_fails_resolution_by_position_without_quoting_it() {
    const PASTED: &str = "sk-live-4kQm2ZpX8vRt7nLb1cWs9aYe";
    let error = resolve_provider_with(&format!(
        r#"credentials = ["keychain:smith/personal", "{PASTED}"]"#
    ))
    .expect_err("an unparseable pool entry");

    let rendered = format!("{error} {error:?}");
    // The offending entry is identified by its position, never by its value:
    // a reference rejected for looking like a pasted key must not be echoed
    // into an error message, a log, or a terminal.
    assert!(rendered.contains("entry 2 of `credentials`"), "{rendered}");
    assert!(!rendered.contains(PASTED), "{rendered}");
    match error {
        ConfigError::PlaintextSecret { ref source, .. }
        | ConfigError::InvalidValue { ref source, .. } => {
            assert_eq!(source.key, "providers.acme.credentials");
        }
        other => panic!("expected a sourced credential error, got {other:?}"),
    }
}

#[test]
fn a_duplicate_pool_entry_is_rejected_rather_than_collapsed() {
    let error = resolve_provider_with(r#"credentials = ["keychain:smith/a", "keychain:smith/a"]"#)
        .expect_err("a duplicate pool entry");

    let rendered = format!("{error}");
    assert!(rendered.contains("keychain:smith/a"), "{rendered}");
    assert!(rendered.contains("more than once"), "{rendered}");
}

#[test]
fn declaring_both_credential_spellings_is_a_contradiction() {
    let error = resolve_provider_with(
        "credential = \"env:ONE\"\ncredentials = [\"env:TWO\", \"env:THREE\"]",
    )
    .expect_err("both spellings");

    // There is no defensible order to splice the single entry into the list,
    // so resolution refuses rather than guessing which account gets billed.
    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(format!("{error}").contains("choose one spelling"));
}

#[test]
fn an_empty_pool_reads_as_no_declaration() {
    // The config round-trips through serde before provenance sees it, and an
    // empty vector is skipped there, so `credentials = []` and an omitted key
    // are the same input by the time resolution runs. This test pins that
    // equivalence so it is a documented property rather than a surprise.
    let resolution = resolve_provider_with("credentials = []").expect("an empty pool");
    assert!(resolution.config.provider.credentials.is_empty());
    assert!(resolution.config.provider.credential().is_none());
}

#[test]
fn a_pool_and_an_inline_key_remain_mutually_exclusive() {
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
credentials = ["keychain:smith/a", "keychain:smith/b"]
api_key = "sk-inline"
"#,
    );
    let error = resolve(&fixture.request()).expect_err("a pool beside an inline key");
    // Project files may not carry an inline key at all, which is caught first.
    assert!(matches!(error, ConfigError::PlaintextSecret { .. }));
}

#[test]
fn the_rotation_threshold_resolves_and_is_bounded_to_a_percentage() {
    let resolution = resolve_provider_with(
        "credentials = [\"keychain:smith/a\", \"keychain:smith/b\"]\nrotate_at_percent = 90",
    )
    .expect("a resolved threshold");
    let threshold = resolution
        .config
        .provider
        .rotate_at_percent
        .expect("a threshold");
    assert_eq!(threshold.value, 90);
    assert_eq!(threshold.source.key, "providers.acme.rotate_at_percent");

    for out_of_range in ["0", "101", "255"] {
        let error = resolve_provider_with(&format!(
            "credentials = [\"keychain:smith/a\", \"keychain:smith/b\"]\nrotate_at_percent = {out_of_range}"
        ))
        .expect_err("a threshold outside a percentage");
        assert!(matches!(error, ConfigError::InvalidValue { .. }));
    }
}

#[test]
fn a_rotation_threshold_without_a_pool_is_rejected() {
    let error =
        resolve_provider_with("credential = \"keychain:smith/only\"\nrotate_at_percent = 90")
            .expect_err("a threshold with nowhere to rotate");

    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(format!("{error}").contains("another member"));
}
