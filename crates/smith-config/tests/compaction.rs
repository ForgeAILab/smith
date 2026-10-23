//! Compaction resolution, candidate preflight, and atomic user edits.

use smith_config::resolve::{ResolveRequest, resolve};
use smith_config::user_config::{CompactionSetting, UserConfigEditError, prepare_compaction_edit};

const CONFIG: &str = r#"
default_profile = "dev"
[profiles.dev]
provider = "local"
model = "example"
[providers.local]
kind = "fake"
[models."local/example"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;

#[test]
fn model_cutoff_and_summary_defaults_resolve_with_their_sources() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let user = home.path().join(".smith");
    std::fs::create_dir(&user).unwrap();
    std::fs::write(user.join("config.toml"), format!("{CONFIG}\n[models.\"local/example\".cache]\ninactivity_limit_ms = 900000\n[context.compaction]\nmodel = \"local/example\"\ninstructions = \"Preserve test results\"\n")).unwrap();
    let request = ResolveRequest::new(project.path()).with_home_dir(home.path());
    let context = resolve(&request).unwrap().config.context;
    assert_eq!(context.cache.inactivity_limit_ms.value, 900000);
    assert!(
        context
            .cache
            .inactivity_limit_ms
            .source
            .key
            .contains("models.")
    );
    assert_eq!(context.compaction_model.unwrap().value, "local/example");
    assert_eq!(
        context.compaction_instructions.unwrap().value,
        "Preserve test results"
    );
    std::fs::create_dir(project.path().join(".smith")).unwrap();
    std::fs::write(
        project.path().join(".smith/config.toml"),
        "[context.cache]\ninactivity_limit_ms = 600000\n",
    )
    .unwrap();
    assert_eq!(
        resolve(&request)
            .unwrap()
            .config
            .context
            .cache
            .inactivity_limit_ms
            .value,
        600000
    );
}

#[test]
fn compaction_edits_preflight_without_writing_and_reset_without_removing_siblings() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let user = home.path().join(".smith");
    std::fs::create_dir(&user).unwrap();
    let original = format!(
        "# Preserve this comment\n{CONFIG}\n[context.compaction]\ninstructions = \"Keep evidence\"\n"
    );
    std::fs::write(user.join("config.toml"), &original).unwrap();
    let edit = prepare_compaction_edit(
        &user,
        &CompactionSetting::Model(Some("local/example".to_owned())),
    )
    .unwrap();
    let mut request = ResolveRequest::new(project.path()).with_home_dir(home.path());
    request.user_config_override = Some(Box::new(edit.candidate_config().unwrap()));
    assert_eq!(
        resolve(&request)
            .unwrap()
            .config
            .context
            .compaction_model
            .unwrap()
            .value,
        "local/example"
    );
    assert_eq!(
        std::fs::read_to_string(user.join("config.toml")).unwrap(),
        original
    );
    edit.commit(true).unwrap().accept();
    let reset = prepare_compaction_edit(&user, &CompactionSetting::Model(None)).unwrap();
    reset.commit(true).unwrap().accept();
    let saved = std::fs::read_to_string(user.join("config.toml")).unwrap();
    assert!(saved.starts_with("# Preserve this comment"));
    assert!(saved.contains("Keep evidence"));
    let resolved =
        resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path())).unwrap();
    assert!(resolved.config.context.compaction_model.is_none());
    assert!(resolved.config.context.compaction_instructions.is_some());
}

#[test]
fn compaction_edits_refuse_to_overwrite_an_intervening_writer() {
    let user = tempfile::tempdir().unwrap();
    std::fs::write(user.path().join("config.toml"), CONFIG).unwrap();
    let edit = prepare_compaction_edit(
        user.path(),
        &CompactionSetting::Instructions(Some("Keep evidence".to_owned())),
    )
    .unwrap();
    let newer = format!("{CONFIG}\n# Another editor's change\n");
    std::fs::write(user.path().join("config.toml"), &newer).unwrap();
    assert!(matches!(
        edit.commit(true),
        Err(UserConfigEditError::ConcurrentModification)
    ));
    assert_eq!(
        std::fs::read_to_string(user.path().join("config.toml")).unwrap(),
        newer
    );
}

#[test]
fn rolling_back_a_compaction_edit_preserves_a_later_user_edit() {
    let user = tempfile::tempdir().unwrap();
    std::fs::write(user.path().join("config.toml"), CONFIG).unwrap();
    let edit = prepare_compaction_edit(
        user.path(),
        &CompactionSetting::Instructions(Some("Keep evidence".into())),
    )
    .unwrap();
    let committed = edit.commit(true).unwrap();
    let newer = format!("{CONFIG}\n# A later user edit\n");
    std::fs::write(user.path().join("config.toml"), &newer).unwrap();
    assert!(matches!(
        committed.rollback(),
        Err(UserConfigEditError::ConcurrentModification)
    ));
    assert_eq!(
        std::fs::read_to_string(user.path().join("config.toml")).unwrap(),
        newer
    );
}

#[test]
fn empty_instructions_and_out_of_bounds_model_times_fail_resolution() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let user = home.path().join(".smith");
    std::fs::create_dir(&user).unwrap();
    for extra in [
        "[context.compaction]\ninstructions = \" \"",
        "[models.\"local/example\".cache]\ninactivity_limit_ms = 999",
    ] {
        std::fs::write(user.join("config.toml"), format!("{CONFIG}\n{extra}\n")).unwrap();
        assert!(resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path())).is_err());
    }
}
