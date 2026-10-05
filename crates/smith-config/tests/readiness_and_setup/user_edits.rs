use super::*;

#[test]
fn user_config_edits_preserve_comments_and_unrelated_tables() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"# keep this explanation
[limits]
max_retries = 7 # and this inline note

[providers.unrelated]
kind = "fake"
"#,
    );
    let prepared = prepare_user_config_edit(
        fixture.home.path().join(".smith"),
        &glm_patch("env:ZAI_API_KEY"),
    )
    .expect("a prepared edit");
    assert!(prepared.collisions().is_empty());
    assert!(prepared.preview().contains("providers.zai.credential"));
    assert!(!prepared.preview().contains("ZAI_API_KEY="));
    prepared.commit(false).expect("an atomic commit").accept();

    let text = fs::read_to_string(fixture.home.path().join(".smith/config.toml"))
        .expect("the merged config");
    assert!(text.contains("# keep this explanation"), "{text}");
    assert!(text.contains("# and this inline note"), "{text}");
    assert!(text.contains("[providers.unrelated]"), "{text}");
    assert!(text.contains("[providers.zai]"), "{text}");
    assert!(text.contains("[models.\"zai/glm-4.7\"]"), "{text}");
}

#[test]
fn differing_existing_values_require_confirmation() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"# owned by the user
default_profile = "existing"

[profiles.existing]
provider = "local"
model = "m"

[providers.local]
kind = "fake"

[models."local/m"]
context_tokens = 100
max_input_tokens = 90
max_output_tokens = 10
"#,
    );
    let path = fixture.home.path().join(".smith/config.toml");
    let before = fs::read(&path).expect("prior bytes");
    let prepared = prepare_user_config_edit(
        fixture.home.path().join(".smith"),
        &glm_patch("env:ZAI_API_KEY"),
    )
    .expect("a prepared edit");
    assert_eq!(prepared.collisions().len(), 1);
    assert_eq!(prepared.collisions()[0].key, "default_profile");
    assert!(matches!(
        prepared.commit(false),
        Err(UserConfigEditError::UnconfirmedCollisions { count: 1 })
    ));
    assert_eq!(fs::read(&path).expect("unchanged bytes"), before);

    prepared
        .commit(true)
        .expect("the reviewed collision was accepted")
        .accept();
    let after = fs::read_to_string(path).expect("the committed config");
    assert!(after.contains("default_profile = \"glm\""), "{after}");
    assert!(after.contains("# owned by the user"), "{after}");
    assert!(after.contains("[profiles.existing]"), "{after}");
}

#[test]
fn committed_edits_can_restore_exact_prior_bytes() {
    let fixture = Fixture::new();
    fixture.write_user("# exact prior bytes\n[approval]\nmode = \"deny\"\n");
    let path = fixture.home.path().join(".smith/config.toml");
    let before = fs::read(&path).expect("prior bytes");
    let committed = prepare_user_config_edit(
        fixture.home.path().join(".smith"),
        &glm_patch("env:ZAI_API_KEY"),
    )
    .expect("a prepared edit")
    .commit(false)
    .expect("a commit");
    assert_ne!(fs::read(&path).expect("candidate bytes"), before);
    committed.rollback().expect("rollback");
    assert_eq!(fs::read(path).expect("restored bytes"), before);
}

#[test]
fn fresh_commit_is_restrictive_and_rollback_removes_only_the_config() {
    let fixture = Fixture::new();
    let directory = fixture.home.path().join(".smith");
    let path = directory.join("config.toml");
    let committed = prepare_user_config_edit(&directory, &glm_patch("env:ZAI_API_KEY"))
        .expect("a prepared fresh edit")
        .commit(false)
        .expect("a fresh commit");
    assert!(path.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            fs::metadata(&directory)
                .expect("directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path)
                .expect("file metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    committed.rollback().expect("fresh rollback");
    assert!(!path.exists());
    assert!(
        directory.is_dir(),
        "rollback must not remove broader user state"
    );
}

#[test]
fn config_edit_errors_and_debug_never_echo_secret_values() {
    let fixture = Fixture::new();
    let secret = "sk-test-do-not-print-0123456789";
    let error = prepare_user_config_edit(fixture.home.path().join(".smith"), &glm_patch(secret))
        .expect_err("a plaintext credential");
    assert!(!format!("{error:?} {error}").contains(secret));

    let mut patch = glm_patch("env:ZAI_API_KEY");
    patch
        .providers
        .get_mut("zai")
        .expect("zai")
        .headers
        .insert("x-trace".into(), secret.into());
    let prepared = prepare_user_config_edit(fixture.home.path().join(".smith"), &patch)
        .expect("a non-authorization header is allowed");
    let rendered = format!("{prepared:?}\n{}", prepared.preview());
    assert!(!rendered.contains(secret), "{rendered}");
    assert!(rendered.contains("[configured header value]"), "{rendered}");
}

#[cfg(unix)]
#[test]
fn inline_credential_replacement_is_redacted_restrictive_and_exactly_reversible() {
    use std::os::unix::fs::PermissionsExt;

    const SECRET: &str = "sk-inline-edit-must-not-render";
    let fixture = Fixture::new();
    fixture.write_user(
        r#"# exact prior credential config
[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "keychain:smith/zai"
"#,
    );
    let path = fixture.home.path().join(".smith/config.toml");
    let before = fs::read(&path).expect("prior bytes");
    let patch = ConfigFile {
        providers: BTreeMap::from([(
            "zai".to_owned(),
            ProviderSection {
                api_key: Some(ConfigSecret::new(SECRET)),
                ..ProviderSection::default()
            },
        )]),
        ..ConfigFile::default()
    };

    let prepared = prepare_user_config_edit(fixture.home.path().join(".smith"), &patch)
        .expect("an inline credential edit");
    assert_eq!(prepared.collisions().len(), 1);
    assert_eq!(prepared.collisions()[0].key, "providers.zai.credential");
    let rendered = format!("{prepared:?}\n{}", prepared.preview());
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(rendered.contains("api_key"), "{rendered}");
    assert!(rendered.contains("[redacted]"), "{rendered}");
    assert!(matches!(
        prepared.commit(false),
        Err(UserConfigEditError::UnconfirmedCollisions { count: 1 })
    ));
    assert_eq!(fs::read(&path).expect("unchanged bytes"), before);

    let committed = prepared.commit(true).expect("the reviewed replacement");
    let candidate = fs::read_to_string(&path).expect("candidate config");
    assert!(candidate.contains(&format!("api_key = \"{SECRET}\"")));
    assert!(!candidate.contains("credential ="));
    assert_eq!(
        fs::metadata(&path)
            .expect("config metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    committed.rollback().expect("exact rollback");
    assert_eq!(fs::read(path).expect("restored bytes"), before);
}

#[test]
fn replacing_one_inline_key_redacts_both_sides_of_collision_review() {
    const OLD_SECRET: &str = "sk-old-inline-must-not-render";
    const NEW_SECRET: &str = "sk-new-inline-must-not-render";
    let fixture = Fixture::new();
    fixture.write_user(&format!(
        "[providers.zai]\nkind = \"openai-compatible\"\nbase_url = \"https://example.test/v1\"\napi_key = \"{OLD_SECRET}\"\n"
    ));
    let patch = ConfigFile {
        providers: BTreeMap::from([(
            "zai".to_owned(),
            ProviderSection {
                api_key: Some(ConfigSecret::new(NEW_SECRET)),
                ..ProviderSection::default()
            },
        )]),
        ..ConfigFile::default()
    };
    let prepared = prepare_user_config_edit(fixture.home.path().join(".smith"), &patch)
        .expect("a replacement");
    let rendered = format!("{prepared:?}\n{}", prepared.preview());
    assert!(!rendered.contains(OLD_SECRET), "{rendered}");
    assert!(!rendered.contains(NEW_SECRET), "{rendered}");
    assert!(rendered.matches("[redacted]").count() >= 2, "{rendered}");
}
