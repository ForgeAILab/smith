//! Store, git source, marketplace, and update transactions.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;
use smith_plugin::{InstalledState, Marketplace, PluginError, PluginSource, PluginStore};
use tempfile::TempDir;

fn fixture() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("fixture");
    let root = temp.path().canonicalize().expect("canonical root");
    (temp, root)
}

fn write(root: &Path, path: &str, body: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    fs::write(path, body).expect("fixture file");
}

fn plugin(root: &Path, name: &str, command: &str) {
    write(
        root,
        ".claude-plugin/plugin.json",
        json!({"name":name,"version":"1"}).to_string(),
    );
    write(
        root,
        "skills/review/SKILL.md",
        "---\ndescription: Review\n---\nReview",
    );
    write(root, "commands/status.md", "Status");
    write(
        root,
        ".mcp.json",
        json!({"local":{"command":command,"args":["${CLAUDE_PLUGIN_ROOT}/server.js"]}}).to_string(),
    );
}

fn local(root: &Path) -> PluginSource {
    PluginSource::Local { path: root.into() }
}

fn git_available() -> bool {
    if Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        true
    } else {
        eprintln!("Skipping local git fixture: git is unavailable");
        false
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git fixture command");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repository(root: &Path) {
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Fixture"]);
    git(root, &["config", "user.email", "fixture@example.test"]);
    git(root, &["add", "."]);
    git(
        root,
        &["-c", "commit.gpgsign=false", "commit", "-m", "fixture"],
    );
}

#[test]
fn prepare_writes_no_store_and_commit_installs_a_copy() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    let store = PluginStore::new(&state);
    let prepared = store
        .prepare_install(&local(&source), None)
        .expect("prepare");
    assert!(!state.join("plugins").exists());
    assert_eq!(prepared.inventory().manifest.name, "tools");
    let digest = prepared.inventory().digest.clone();
    let installed = store.commit_install(prepared).expect("commit");
    assert_eq!(installed.digest, digest);
    assert_eq!(installed.source, local(&source));
    assert_eq!(
        installed.install_directory,
        state
            .join("plugins/store/tools")
            .join(&digest.as_hex()[..16])
    );
    assert!(store.data_directory("tools").expect("data path").is_dir());
    write(&source, "commands/status.md", "Source changed");
    assert!(matches!(
        store.load("tools").expect("load"),
        InstalledState::Unchanged(_)
    ));
    assert_eq!(
        PluginStore::new(&state).list().expect("persisted list"),
        vec![installed]
    );
}

#[test]
fn no_manifest_uses_directory_or_marketplace_fallback() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    write(&source, "skills/review/SKILL.md", "Body");
    let store = PluginStore::new(state);
    let prepared = store
        .prepare_install(&local(&source), Some("entry-name"))
        .expect("fallback");
    assert_eq!(prepared.inventory().manifest.name, "entry-name");
    let prepared = store
        .prepare_install(&local(&source), None)
        .expect("directory fallback");
    assert_eq!(
        prepared.inventory().manifest.name,
        source.file_name().expect("name").to_string_lossy()
    );
}

#[test]
fn edited_content_is_changed_and_missing_content_is_failed() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    let store = PluginStore::new(state);
    let installed = store
        .commit_install(
            store
                .prepare_install(&local(&source), None)
                .expect("prepare"),
        )
        .expect("commit");
    write(
        &installed.install_directory,
        "commands/status.md",
        "Changed",
    );
    assert!(matches!(
        store.load("tools").expect("changed"),
        InstalledState::Changed { .. }
    ));
    let replacement = store
        .prepare_update("tools")
        .expect("edited installs can be updated");
    assert!(replacement.discards_local_changes);
    let restored = store
        .commit_update(replacement)
        .expect("confirmed replacement");
    assert_eq!(restored.digest, installed.digest);
    assert!(matches!(
        store.load("tools").unwrap(),
        InstalledState::Unchanged(_)
    ));
    fs::remove_dir_all(&installed.install_directory).expect("delete install");
    assert!(matches!(
        store.load("tools").expect("failed"),
        InstalledState::Failed { .. }
    ));
}

#[test]
fn update_reports_command_lines_and_keeps_old_version_until_commit() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    let store = PluginStore::new(state);
    let old = store
        .commit_install(
            store
                .prepare_install(&local(&source), None)
                .expect("prepare"),
        )
        .expect("install");
    let unchanged = store.prepare_update("tools").expect("unchanged update");
    assert!(unchanged.difference.changed_skills.is_empty());
    assert!(unchanged.difference.changed_commands.is_empty());
    plugin(&source, "tools", "new");
    write(&source, "skills/review/SKILL.md", "Changed review");
    write(&source, "skills/added/SKILL.md", "New");
    fs::remove_file(source.join("commands/status.md")).expect("remove command");
    write(&source, "commands/new.md", "New command");
    write(&source, "hooks/hooks.json", "{} ");
    let update = store.prepare_update("tools").expect("update proposal");
    assert_eq!(store.list().expect("record untouched"), vec![old.clone()]);
    assert!(old.install_directory.is_dir());
    let change = &update.difference.changed_mcp_servers[0];
    assert_eq!(change.old.command.as_deref(), Some("old"));
    assert_eq!(change.new.command.as_deref(), Some("new"));
    assert!(change.old.command_line().contains("old"));
    assert!(change.new.command_line().contains("new"));
    assert_eq!(update.difference.changed_skills, ["tools:review"]);
    assert_eq!(update.difference.added_skills, ["tools:added"]);
    assert_eq!(update.difference.added_commands, ["tools:new"]);
    assert_eq!(update.difference.removed_commands, ["tools:status"]);
    assert_eq!(update.difference.added_not_loaded[0].kind, "hooks");
    let updated = store.commit_update(update).expect("confirmed update");
    assert_ne!(old.digest, updated.digest);
    assert_ne!(old.install_directory, updated.install_directory);
    assert!(matches!(
        store.load("tools").expect("updated content"),
        InstalledState::Unchanged(_)
    ));
    assert!(matches!(
        store.commit_update(unchanged),
        Err(PluginError::Changed)
    ));
}

#[test]
fn remove_deletes_all_versions_and_keeps_data_unless_requested() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    let store = PluginStore::new(&state);
    store
        .commit_install(
            store
                .prepare_install(&local(&source), None)
                .expect("prepare"),
        )
        .expect("install");
    let data = store.data_directory("tools").expect("data");
    write(&data, "state.txt", "Retained");
    plugin(&source, "tools", "new");
    store
        .commit_update(store.prepare_update("tools").expect("update"))
        .expect("commit update");
    store.remove("tools", false).expect("remove");
    assert!(!state.join("plugins/store/tools").exists());
    assert!(store.list().expect("empty list").is_empty());
    assert_eq!(
        fs::read_to_string(data.join("state.txt")).expect("data kept"),
        "Retained"
    );
    store
        .commit_install(
            store
                .prepare_install(&local(&source), None)
                .expect("prepare"),
        )
        .expect("reinstall");
    store.remove("tools", true).expect("remove data");
    assert!(!data.exists());
}

#[test]
fn modified_prepared_snapshot_cannot_be_committed() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    let store = PluginStore::new(&state);
    let prepared = store
        .prepare_install(&local(&source), None)
        .expect("prepare");
    write(
        &prepared.inventory().root,
        "commands/status.md",
        "Changed after review",
    );
    assert!(matches!(
        store.commit_install(prepared),
        Err(PluginError::Changed)
    ));
    assert!(store.list().expect("nothing installed").is_empty());
    assert!(!state.join("plugins/store/tools").exists());
}

#[test]
fn record_failure_rolls_back_install_and_remove() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    let store = PluginStore::new(&state);
    let installed = store
        .commit_install(
            store
                .prepare_install(&local(&source), None)
                .expect("prepare"),
        )
        .expect("install");
    let record = state.join("plugins/installed.toml");
    let original = fs::read(&record).expect("record bytes");
    fs::remove_file(&record).expect("remove record");
    fs::create_dir(&record).expect("block record");
    plugin(&source, "tools", "new");
    let prepared = store
        .prepare_install(&local(&source), None)
        .expect("new snapshot");
    let candidate = state
        .join("plugins/store/tools")
        .join(&prepared.inventory().digest.as_hex()[..16]);
    assert!(store.commit_install(prepared).is_err());
    assert!(!candidate.exists());
    assert!(installed.install_directory.exists());
    fs::remove_dir(record.clone()).expect("unblock record");
    fs::write(record, original).expect("restore record");
}

#[test]
fn source_parser_handles_git_and_marketplace_forms() {
    assert!(matches!(
        PluginSource::parse("./local").expect("local"),
        PluginSource::Local { .. }
    ));
    assert!(matches!(
        PluginSource::parse("/absolute").expect("absolute"),
        PluginSource::Local { .. }
    ));
    for source in [
        "github:owner/repo#main",
        "https://example.test/repo.git#main",
        "git@example.test:owner/repo.git#main",
        "ssh://git@example.test/repo.git#main",
    ] {
        let parsed = PluginSource::parse(source).expect("git source");
        assert_eq!(parsed.reference(), Some("main"));
    }
    assert!(matches!(
        PluginSource::parse("tools@catalogue").expect("marketplace"),
        PluginSource::Marketplace { .. }
    ));
    assert!(PluginSource::parse("github:owner").is_err());
    assert!(PluginSource::parse("github:../repo").is_err());
}

#[test]
fn missing_git_is_distinct_and_installs_nothing() {
    let (_state, state) = fixture();
    let store = PluginStore::new(&state).with_git_program(state.join("nonexistent-git"));
    let error = store
        .prepare_install(
            &PluginSource::parse("github:owner/repo").expect("source"),
            None,
        )
        .expect_err("missing git");
    assert!(matches!(error, PluginError::GitUnavailable { .. }));
    assert!(error.to_string().contains("git is unavailable"));
    assert!(!state.join("plugins").exists());
}

#[test]
fn local_git_checkout_installs_and_records_commit() {
    if !git_available() {
        return;
    }
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "git-tools", "old");
    repository(&source);
    let parsed =
        PluginSource::parse(&format!("file://{}#main", source.display())).expect("git source");
    let store = PluginStore::new(&state);
    let prepared = store
        .prepare_install(&parsed, None)
        .expect("git preparation");
    assert_eq!(prepared.resolved_commit().expect("commit").len(), 40);
    let installed = store.commit_install(prepared).expect("install");
    assert_eq!(installed.reference.as_deref(), Some("main"));
    assert_eq!(installed.source, parsed);
    assert!(!installed.install_directory.join(".git").exists());
    plugin(&source, "git-tools", "new");
    git(&source, &["add", "."]);
    git(
        &source,
        &["-c", "commit.gpgsign=false", "commit", "-m", "update"],
    );
    let update = store
        .prepare_update("git-tools")
        .expect("fetch new version");
    assert_eq!(
        update.difference.changed_mcp_servers[0]
            .new
            .command
            .as_deref(),
        Some("new")
    );
    assert_ne!(
        update.prepared.resolved_commit(),
        installed.resolved_commit.as_deref()
    );
    assert_eq!(store.list().expect("old kept"), vec![installed]);
}

#[test]
fn marketplace_local_and_git_entries_install_and_npm_is_named() {
    let (_catalogue, catalogue) = fixture();
    let (_repo, repo) = fixture();
    let (_state, state) = fixture();
    plugin(&catalogue.join("plugins/local"), "local-tools", "local");
    plugin(&repo, "git-tools", "git");
    write(&catalogue, ".claude-plugin/marketplace.json", json!({"name":"catalogue","plugins":[
        {"name":"local-tools","source":"./plugins/local"},
        {"name":"git-tools","source":{"source":"url","url":format!("file://{}",repo.display()),"ref":"main"}},
        {"name":"npm-tools","source":{"source":"npm","package":"tools"}},
        {"name":"github-tools","source":{"source":"github","repo":"owner/repo","ref":"main"}}
    ]}).to_string());
    let store = PluginStore::new(&state);
    let record = store
        .add_marketplace(&local(&catalogue))
        .expect("register catalogue");
    assert!(record.directory.starts_with(&state));
    assert_eq!(store.list_marketplaces().expect("catalogues"), vec![record]);
    assert_eq!(
        store
            .marketplace("catalogue")
            .expect("catalogue")
            .plugins
            .len(),
        4
    );
    let source = PluginSource::parse("local-tools@catalogue").expect("entry source");
    let installed = store
        .commit_install(store.prepare_install(&source, None).expect("local entry"))
        .expect("install local entry");
    assert_eq!(installed.source, source);
    assert!(
        matches!(store.resolve_marketplace("catalogue", "npm-tools"), Err(PluginError::UnsupportedSource { kind }) if kind == "npm")
    );
    assert_eq!(
        store
            .resolve_marketplace("catalogue", "github-tools")
            .expect("github source")
            .reference(),
        Some("main")
    );
    if git_available() {
        repository(&repo);
        let source = PluginSource::parse("git-tools@catalogue").expect("git entry");
        let installed = store
            .commit_install(
                store
                    .prepare_install(&source, None)
                    .expect("fetch git entry"),
            )
            .expect("install git entry");
        assert!(installed.resolved_commit.is_some());
        assert_eq!(installed.source, source);
    }
    write(
        &catalogue,
        "plugins/local/commands/status.md",
        "New command",
    );
    store
        .update_marketplace("catalogue")
        .expect("refresh catalogue");
    let update = store
        .prepare_update("local-tools")
        .expect("refreshed local entry");
    assert_eq!(update.difference.changed_commands, ["local-tools:status"]);
    store
        .remove_marketplace("catalogue")
        .expect("remove catalogue");
    assert!(
        store
            .list_marketplaces()
            .expect("catalogues gone")
            .is_empty()
    );
    assert!(matches!(
        store.load("local-tools").expect("plugin preserved"),
        InstalledState::Unchanged(_)
    ));
}

#[test]
fn marketplace_sources_reject_escape_and_report_unknown_kinds() {
    let (_catalogue, root) = fixture();
    write(&root, ".claude-plugin/marketplace.json", json!({"name":"catalogue","plugins":[
        {"name":"escape","source":"./../outside"},
        {"name":"unknown","source":{"source":"custom"}},
        {"name":"subdir","source":{"source":"git-subdir","url":"https://example.test/repo.git","path":"plugins/tools"}}
    ]}).to_string());
    let marketplace = Marketplace::read(&root).expect("catalogue");
    assert!(matches!(
        marketplace.resolve(&root, "escape"),
        Err(PluginError::PathEscape(_))
    ));
    assert!(
        matches!(marketplace.resolve(&root, "unknown"), Err(PluginError::UnsupportedSource { kind }) if kind == "custom")
    );
    assert!(
        matches!(marketplace.resolve(&root, "subdir").expect("subdir"), PluginSource::Git { subdirectory: Some(path), .. } if path == "plugins/tools")
    );
}

#[cfg(unix)]
#[test]
fn copy_refuses_escaping_symlinks_and_cycles_but_materialises_internal_links() {
    let (_source, source) = fixture();
    let (_outside, outside) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    write(&outside, "secret", "Outside");
    std::os::unix::fs::symlink(outside.join("secret"), source.join("escape"))
        .expect("escaping link");
    let store = PluginStore::new(&state);
    assert!(matches!(
        store.prepare_install(&local(&source), None),
        Err(PluginError::PathEscape(_))
    ));
    assert!(!state.join("plugins").exists());
    fs::remove_file(source.join("escape")).expect("remove escape");
    std::os::unix::fs::symlink(&source, source.join("cycle")).expect("cycle");
    assert!(
        store
            .prepare_install(&local(&source), None)
            .expect_err("cycle")
            .to_string()
            .contains("symlink cycle")
    );
    fs::remove_file(source.join("cycle")).expect("remove cycle");
    std::os::unix::fs::symlink(
        source.join("commands/status.md"),
        source.join("commands/alias.md"),
    )
    .expect("internal link");
    let installed = store
        .commit_install(
            store
                .prepare_install(&local(&source), None)
                .expect("internal symlink"),
        )
        .expect("commit");
    assert!(
        !fs::symlink_metadata(installed.install_directory.join("commands/alias.md"))
            .expect("materialised file")
            .file_type()
            .is_symlink()
    );
    assert!(matches!(
        store.load("tools").expect("same digest"),
        InstalledState::Unchanged(_)
    ));
}

#[cfg(unix)]
#[test]
fn store_directory_escape_is_refused_without_touching_outside() {
    let (_source, source) = fixture();
    let (_outside, outside) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    fs::create_dir_all(state.join("plugins")).expect("plugins directory");
    std::os::unix::fs::symlink(&outside, state.join("plugins/store")).expect("store escape");
    let store = PluginStore::new(&state);
    assert!(matches!(
        store.commit_install(
            store
                .prepare_install(&local(&source), None)
                .expect("prepare")
        ),
        Err(PluginError::PathEscape(_))
    ));
    assert_eq!(fs::read_dir(outside).expect("untouched outside").count(), 0);
}

#[test]
fn differences_cover_removed_skills_changed_commands_and_mcp_add_remove() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    write(&source, "hooks/hooks.json", "{}");
    let store = PluginStore::new(state);
    store
        .commit_install(
            store
                .prepare_install(&local(&source), None)
                .expect("prepare"),
        )
        .expect("install");
    fs::remove_file(source.join("skills/review/SKILL.md")).expect("remove skill");
    fs::remove_file(source.join("hooks/hooks.json")).expect("remove hooks");
    write(&source, "commands/status.md", "Changed");
    write(
        &source,
        ".mcp.json",
        json!({"added":{"url":"https://example.test/mcp"}}).to_string(),
    );
    let update = store.prepare_update("tools").expect("difference");
    assert_eq!(update.difference.removed_skills, ["tools:review"]);
    assert_eq!(update.difference.changed_commands, ["tools:status"]);
    assert_eq!(update.difference.added_mcp_servers[0].name, "tools:added");
    assert_eq!(update.difference.removed_mcp_servers[0].name, "tools:local");
    assert_eq!(update.difference.removed_not_loaded[0].kind, "hooks");
}

#[test]
fn in_repo_marketplace_installs_unchanged_bundle() {
    let (_state, state) = fixture();
    let store = PluginStore::new(state);
    let catalogue = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/claude-code")
        .canonicalize()
        .expect("repository marketplace");
    store
        .add_marketplace(&local(&catalogue))
        .expect("existing marketplace");
    let source = PluginSource::parse("smith@smith").expect("marketplace source");
    let prepared = store
        .prepare_install(&source, None)
        .expect("existing bundle");
    assert_eq!(prepared.inventory().skills.len(), 2);
    assert_eq!(prepared.inventory().commands.len(), 4);
    let installed = store
        .commit_install(prepared)
        .expect("install unchanged bundle");
    assert_eq!(installed.name, "smith");
    assert!(matches!(
        store.load("smith").expect("stored content"),
        InstalledState::Unchanged(_)
    ));
}

#[test]
fn git_subdirectory_and_manifestless_repo_name_are_supported() {
    if !git_available() {
        return;
    }
    let (_repo, repo) = fixture();
    let (_state, state) = fixture();
    write(&repo, "skills/root/SKILL.md", "Root");
    plugin(&repo.join("plugins/nested"), "nested-tools", "nested");
    repository(&repo);
    let store = PluginStore::new(state);
    let source = PluginSource::Git {
        url: format!("file://{}", repo.display()),
        reference: Some("main".into()),
        subdirectory: Some("plugins/nested".into()),
    };
    let prepared = store
        .prepare_install(&source, None)
        .expect("git subdirectory");
    assert_eq!(prepared.inventory().manifest.name, "nested-tools");
    let source = PluginSource::Git {
        url: format!("file://{}", repo.display()),
        reference: Some("main".into()),
        subdirectory: None,
    };
    let prepared = store
        .prepare_install(&source, None)
        .expect("manifestless repository");
    assert_eq!(
        prepared.inventory().manifest.name,
        repo.file_name().expect("repo name").to_string_lossy()
    );
}

#[test]
fn stale_or_renamed_update_is_refused() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    let store = PluginStore::new(state);
    let installed = store
        .commit_install(
            store
                .prepare_install(&local(&source), None)
                .expect("prepare"),
        )
        .expect("install");
    plugin(&source, "tools", "new");
    let update = store.prepare_update("tools").expect("proposed update");
    write(
        &installed.install_directory,
        "commands/status.md",
        "Edited old version",
    );
    assert!(matches!(
        store.commit_update(update),
        Err(PluginError::Changed)
    ));
    assert_eq!(
        store.list().expect("old still recorded"),
        vec![installed.clone()]
    );
    write(&installed.install_directory, "commands/status.md", "Status");
    plugin(&source, "renamed", "new");
    assert!(
        store
            .prepare_update("tools")
            .expect_err("rename")
            .to_string()
            .contains("cannot rename")
    );
    assert_eq!(store.list().expect("old remains"), vec![installed]);
}

#[test]
fn planned_install_paths_expand_for_confirmation_without_creating_state() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    let root = state.join("new-state");
    plugin(&source, "tools", "old");
    let store = PluginStore::new(&root);
    let prepared = store
        .prepare_install(&local(&source), None)
        .expect("snapshot");
    let directory = store
        .install_directory("tools", &prepared.inventory().digest)
        .expect("planned path");
    let data = store.data_directory("tools").expect("data path");
    let project = state.join("workspace");
    let resolved = prepared
        .inventory()
        .resolved_mcp(smith_plugin::PathVariables {
            plugin_root: &directory,
            plugin_data: &data,
            project_dir: &project,
        });
    assert_eq!(
        resolved[0].args[0],
        format!("{}/server.js", directory.display())
    );
    assert!(!root.exists());
    let installed = store.commit_install(prepared).expect("install");
    assert_eq!(installed.install_directory, directory);
}

#[test]
fn first_install_record_failure_leaves_no_plugin_directory_or_data() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "old");
    fs::create_dir_all(state.join("plugins/installed.toml")).expect("block record write");
    let store = PluginStore::new(&state);
    let prepared = store
        .prepare_install(&local(&source), None)
        .expect("prepare");
    assert!(store.commit_install(prepared).is_err());
    assert!(!state.join("plugins/store/tools").exists());
    assert!(!state.join("plugins/data/tools").exists());
}

#[test]
fn editing_an_installed_supporting_program_changes_the_bundle_and_confirmed_update_restores_it() {
    let (_source, source) = fixture();
    let (_state, state) = fixture();
    plugin(&source, "tools", "node");
    write(&source, "server.js", "reviewed server program");
    let store = PluginStore::new(state);
    let installed = store
        .commit_install(store.prepare_install(&local(&source), None).unwrap())
        .unwrap();
    write(
        &installed.install_directory,
        "server.js",
        "edited server program",
    );
    assert!(matches!(
        store.load("tools").unwrap(),
        InstalledState::Changed { .. }
    ));
    let update = store.prepare_update("tools").unwrap();
    assert!(update.discards_local_changes);
    assert_eq!(update.prepared.inventory().digest, installed.digest);
    assert_eq!(
        fs::read_to_string(installed.install_directory.join("server.js")).unwrap(),
        "edited server program"
    );
    store.commit_update(update).unwrap();
    assert!(matches!(
        store.load("tools").unwrap(),
        InstalledState::Unchanged(_)
    ));
    assert_eq!(
        fs::read_to_string(installed.install_directory.join("server.js")).unwrap(),
        "reviewed server program"
    );
}
