//! The process dispatches plugin management without provider setup or a session.

use std::path::Path;

use super::Fixture;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn source(fixture: &Fixture) -> String {
    let root = fixture.project.path().join("plugin-source");
    write(
        &root,
        ".claude-plugin/plugin.json",
        r#"{"name":"process-tools","version":"1"}"#,
    );
    write(
        &root,
        "skills/deploy/SKILL.md",
        "---\ndescription: Deploy service\n---\nDeploy guidance",
    );
    write(&root, "commands/status.md", "Status $ARGUMENTS");
    write(
        &root,
        "hooks/hooks.json",
        r#"{"hooks":{"SessionStart":[{"command":"touch SHOULD_NOT_RUN"}]}}"#,
    );
    write(
        &root,
        ".mcp.json",
        r#"{"local":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/server.js"]}}"#,
    );
    write(&root, "server.js", "server program");
    root.canonicalize().unwrap().display().to_string()
}

#[test]
fn plugin_process_refuses_nonterminal_confirmation_and_yes_installs_without_setup() {
    let fixture = Fixture::unconfigured();
    let source = source(&fixture);
    let refused = fixture.run(&["plugin", "add", &source]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--yes"));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("will not run"));
    let state = fixture.home.path().canonicalize().unwrap().join(".smith");
    assert!(
        smith_plugin::PluginStore::new(&state)
            .list()
            .unwrap()
            .is_empty()
    );
    let installed = fixture.run(&["plugin", "add", &source, "--yes"]);
    assert!(installed.status.success(), "{installed:?}");
    assert!(installed.stderr.is_empty());
    let output = String::from_utf8_lossy(&installed.stdout);
    for expected in [
        "process-tools:deploy",
        "process-tools:status",
        "server.js",
        "will not run",
        "digest:",
        "Installed and trusted",
    ] {
        assert!(output.contains(expected), "{output}");
    }
    assert!(!fixture.project.path().join("SHOULD_NOT_RUN").exists());
    let listed = fixture.run(&["plugin", "list"]);
    assert!(listed.status.success(), "{listed:?}");
    assert!(String::from_utf8_lossy(&listed.stdout).contains("process-tools · on"));
    assert!(
        !state.join("sessions").exists(),
        "plugin management started session persistence"
    );
}

#[test]
fn plugin_process_updates_mcp_invocation_and_remove_keeps_data() {
    let fixture = Fixture::unconfigured();
    let source = source(&fixture);
    assert!(
        fixture
            .run(&["plugin", "add", &source, "--yes"])
            .status
            .success()
    );
    write(
        Path::new(&source),
        ".mcp.json",
        r#"{"local":{"command":"new-node","args":["--new"]}}"#,
    );
    let updated = fixture.run(&["plugin", "update", "process-tools", "--yes"]);
    assert!(updated.status.success(), "{updated:?}");
    let output = String::from_utf8_lossy(&updated.stdout);
    for expected in [
        "changed MCP process-tools:local",
        "old:",
        "new:",
        "new-node",
        "new digest:",
    ] {
        assert!(output.contains(expected), "{output}");
    }
    let state = fixture.home.path().canonicalize().unwrap().join(".smith");
    let store = smith_plugin::PluginStore::new(&state);
    let installed = store.list().unwrap().remove(0);
    let data = store.data_directory("process-tools").unwrap();
    write(&data, "saved.txt", "persistent data");
    let removed = fixture.run(&["plugin", "remove", "process-tools"]);
    assert!(removed.status.success(), "{removed:?}");
    assert!(!installed.install_directory.exists());
    assert!(data.join("saved.txt").exists());
    assert!(store.list().unwrap().is_empty());
    assert!(
        smith_config::trust::TrustStore::open(&state)
            .unwrap()
            .records(&state.join("plugins"))
            .unwrap()
            .is_empty()
    );
}
