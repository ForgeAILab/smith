//! Bundle compatibility and inventory outcomes, without runtime wiring.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use smith_plugin::{
    CommandSource, PathVariables, PluginError, inspect, needs_configuration, read_manifest,
    substitute_variables, validate_name,
};
use tempfile::TempDir;

fn fixture() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("fixture");
    let root = temp.path().canonicalize().expect("canonical fixture");
    (temp, root)
}

fn write(root: &Path, path: &str, body: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    fs::write(path, body).expect("fixture file");
}

fn manifest(root: &Path, value: serde_json::Value) {
    write(root, ".claude-plugin/plugin.json", value.to_string());
}

#[test]
fn in_repo_claude_plugin_is_read_unchanged() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/claude-code/plugins/smith");
    let inventory = inspect(&root, "fallback").expect("existing plugin");
    assert_eq!(inventory.manifest.name, "smith");
    assert_eq!(inventory.manifest.version.as_deref(), Some("0.3.0"));
    assert_eq!(
        inventory
            .skills
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["smith:smith-cli-runtime", "smith:smith-profiles"]
    );
    assert_eq!(inventory.commands.len(), 4);
    assert!(
        inventory
            .commands
            .iter()
            .all(|c| c.name.starts_with("smith:"))
    );
    assert!(inventory.not_loaded.iter().any(|c| c.kind == "agents"));
}

#[test]
fn standard_layout_and_missing_manifest_use_caller_name() {
    let (_temp, root) = fixture();
    write(
        &root,
        "skills/deploy/SKILL.md",
        "---\ndescription: Deploy service\n---\nDeploy",
    );
    write(
        &root,
        "commands/status.md",
        "---\nargument-hint: service\n---\nStatus",
    );
    write(
        &root,
        ".mcp.json",
        json!({"mcpServers": {"local": {"command": "node", "args": ["server.js"]}}}).to_string(),
    );
    let inventory = inspect(&root, "deploy-tools").expect("no manifest required");
    assert_eq!(inventory.skills[0].name, "deploy-tools:deploy");
    assert_eq!(
        inventory.skills[0].description.as_deref(),
        Some("Deploy service")
    );
    assert_eq!(inventory.commands[0].name, "deploy-tools:status");
    assert_eq!(
        inventory.commands[0].argument_hint.as_deref(),
        Some("service")
    );
    assert_eq!(inventory.mcp_servers[0].name, "deploy-tools:local");
    assert!(inventory.manifest.default_enabled);
    manifest(
        &root,
        json!({"name": "deploy-tools", "author": {"name":"ignored"}}),
    );
    assert_eq!(
        inspect(&root, "fallback").expect("manifest").manifest.name,
        "deploy-tools"
    );
}

#[test]
fn overlay_replaces_keys_without_merging_nested_values() {
    let (_temp, root) = fixture();
    manifest(
        &root,
        json!({"name":"claude", "version":"1", "description":"kept", "skills":["./old"], "mcpServers":{"old":{"command":"old"}}, "defaultEnabled":false}),
    );
    write(&root, ".smith-plugin/plugin.json", json!({"name":"smith", "version":"2", "skills":"./new", "mcpServers":{"new":{"command":"new"}}, "defaultEnabled":true, "unrecognised":42}).to_string());
    write(&root, "new/SKILL.md", "Body");
    let inventory = inspect(&root, "unused").expect("overlay");
    assert_eq!(inventory.manifest.name, "smith");
    assert_eq!(inventory.manifest.description.as_deref(), Some("kept"));
    assert_eq!(inventory.manifest.version.as_deref(), Some("2"));
    assert!(inventory.manifest.default_enabled);
    assert_eq!(inventory.skills[0].name, "smith:new");
    assert_eq!(inventory.mcp_servers[0].name, "smith:new");
    fs::remove_file(root.join(".smith-plugin/plugin.json")).expect("remove overlay");
    assert!(
        !read_manifest(&root, "fallback")
            .expect("Claude switch retained")
            .default_enabled
    );
}

#[test]
fn names_cannot_escape_store_or_namespace() {
    for name in [
        "",
        ".",
        "..",
        "two words",
        "tabs\t",
        "line\n",
        "@name",
        "a:b",
        "a/b",
        "a\\b",
        "control\0",
    ] {
        assert!(validate_name(name).is_err(), "{name:?}");
    }
    for name in ["smith", "deploy-tools", "some.name", "Tool_1"] {
        validate_name(name).expect("valid name");
    }
}

#[test]
fn invalid_manifest_metadata_is_a_plugin_error() {
    let (_temp, root) = fixture();
    for value in [
        json!([]),
        json!({"name":42}),
        json!({"name":"invalid:name"}),
        json!({"version":42}),
    ] {
        manifest(&root, value);
        assert!(inspect(&root, "fallback").is_err());
    }
    write(&root, ".claude-plugin/plugin.json", "not JSON");
    assert!(matches!(
        inspect(&root, "fallback"),
        Err(PluginError::Json(_))
    ));
}

#[test]
fn invalid_component_paths_do_not_hide_other_components() {
    let (_temp, root) = fixture();
    write(&root, "commands/good.md", "Good");
    manifest(
        &root,
        json!({"name":"tools", "skills":["skills", "/absolute", "./../outside", "./missing", "./bad\\path"]}),
    );
    let inventory = inspect(&root, "unused").expect("per-component failures");
    assert!(inventory.skills.is_empty());
    assert_eq!(inventory.commands.len(), 1);
    assert_eq!(inventory.not_loaded.len(), 5);
    assert!(
        inventory
            .not_loaded
            .iter()
            .any(|c| c.reason.contains("escapes"))
    );
    assert!(
        inventory
            .not_loaded
            .iter()
            .any(|c| c.reason.contains("missing"))
    );
    assert!(
        inventory
            .not_loaded
            .iter()
            .any(|c| c.reason.contains("./-relative"))
    );
}

#[test]
fn skills_add_to_defaults_and_root_skill_conventions_work() {
    let (_temp, root) = fixture();
    write(&root, "SKILL.md", "Root body");
    assert_eq!(
        inspect(&root, "root-skill").expect("single skill").skills[0].name,
        "root-skill:root-skill"
    );
    write(&root, "skills/default/SKILL.md", "Default");
    write(&root, "extra/custom/SKILL.md", "Custom");
    for declaration in [json!("./extra"), json!(["./extra", "."]), json!("./")] {
        manifest(&root, json!({"name":"tools", "skills":declaration}));
        let inventory = inspect(&root, "unused").expect("additive skills");
        assert!(inventory.skills.iter().any(|c| c.name == "tools:default"));
        if inventory.manifest.skills == Some(json!("./extra")) {
            assert!(inventory.skills.iter().any(|c| c.name == "tools:custom"));
        } else {
            assert!(inventory.skills.iter().any(|c| c.name == "tools:tools"));
        }
    }
}

#[test]
fn command_paths_replace_defaults_and_object_map_supports_inline() {
    let (_temp, root) = fixture();
    write(&root, "commands/default.md", "Default");
    write(&root, "other/custom.md", "Custom");
    for declaration in [json!("./other"), json!(["./other/custom.md"])] {
        manifest(&root, json!({"name":"tools", "commands":declaration}));
        let inventory = inspect(&root, "unused").expect("custom commands");
        assert_eq!(inventory.commands.len(), 1);
        assert_eq!(inventory.commands[0].name, "tools:custom");
    }
    manifest(
        &root,
        json!({"name":"tools", "commands":{
            "inline":{"content":"Prompt ${CLAUDE_PROJECT_DIR}","description":"Inline", "argumentHint":"target"},
            "alias":{"source":"./other/custom.md","description":"Alias"},
            "configured":{"content":"${user_config.token}"},
            "missing":{}, "malformed":false
        }}),
    );
    let inventory = inspect(&root, "unused").expect("object commands");
    assert_eq!(inventory.commands.len(), 2);
    assert_eq!(inventory.commands[0].name, "tools:alias");
    assert_eq!(inventory.commands[1].description.as_deref(), Some("Inline"));
    assert!(matches!(
        inventory.commands[1].source,
        CommandSource::Inline(_)
    ));
    assert_eq!(inventory.not_loaded.len(), 3);
}

#[test]
fn mcp_bare_and_wrapped_maps_merge_with_later_name_winning() {
    let (_temp, root) = fixture();
    write(&root, ".mcp.json", json!({"keep":{"command":"keep"}, "override":{"command":"old"}, "configured":{"command":"old"}}).to_string());
    write(
        &root,
        "more.json",
        json!({"mcpServers":{"override":{"command":"middle"}}}).to_string(),
    );
    manifest(
        &root,
        json!({"name":"tools", "mcpServers":["./more.json", {"override":{"command":"new"},"configured":{"command":"${user_config.command}"},"remote":{"type":"http","url":"https://example.test/mcp","headers":{"x-key":"${CLAUDE_PLUGIN_ROOT}"}}}]}),
    );
    let inventory = inspect(&root, "unused").expect("MCP merge");
    assert_eq!(inventory.mcp_servers.len(), 3);
    assert_eq!(inventory.mcp_servers[1].command.as_deref(), Some("new"));
    assert_eq!(inventory.not_loaded.len(), 1);
    assert_eq!(inventory.not_loaded[0].name, "tools:configured");
}

#[test]
fn all_recognised_unsupported_components_are_reported() {
    let (_temp, root) = fixture();
    for path in [
        "hooks/hooks.json",
        "agents/agent.md",
        ".lsp.json",
        "output-styles/style.md",
        "workflows/one.md",
        "themes/one.json",
        "monitors/one.json",
        "channels/one.json",
        "bin/program",
        "settings.json",
    ] {
        write(&root, path, "Unloaded content");
    }
    manifest(
        &root,
        json!({"name":"tools", "userConfig":{"token":{}}, "mcpServers":["./bundle.mcpb","./bundle.dxt",{"sse":{"type":"sse","url":"https://example.test"}, "websocket":{"type":"websocket","url":"wss://example.test"}, "bad":{"args":[]}}]}),
    );
    let inventory = inspect(&root, "unused").expect("unsupported components do not fail plugin");
    assert!(inventory.mcp_servers.is_empty());
    for kind in [
        "hooks",
        "agents",
        "lspServers",
        "outputStyles",
        "workflows",
        "themes",
        "monitors",
        "channels",
        "bin",
        "settings",
        "userConfig",
        "mcpServers",
    ] {
        assert!(
            inventory.not_loaded.iter().any(|c| c.kind == kind),
            "{kind}"
        );
    }
    assert!(
        inventory
            .not_loaded
            .iter()
            .any(|c| c.reason == "hooks are not run")
    );
    assert!(
        inventory
            .not_loaded
            .iter()
            .any(|c| c.reason.contains("sse transport"))
    );
    assert!(
        inventory
            .not_loaded
            .iter()
            .any(|c| c.reason.contains("websocket transport"))
    );
    assert_eq!(
        inventory
            .not_loaded
            .iter()
            .filter(|c| c.reason.contains("MCP bundles"))
            .count(),
        2
    );
    assert_eq!(inventory.not_loaded.len(), 16);
}

#[test]
fn unsupported_manifest_components_without_default_paths_are_reported() {
    let (_temp, root) = fixture();
    let mut value = json!({"name":"tools"});
    for key in [
        "hooks",
        "agents",
        "lspServers",
        "outputStyles",
        "workflows",
        "themes",
        "monitors",
        "channels",
        "bin",
        "settings",
        "userConfig",
    ] {
        value[key] = json!({});
    }
    manifest(&root, value);
    assert_eq!(
        inspect(&root, "unused")
            .expect("manifest components")
            .not_loaded
            .len(),
        11
    );
}

#[test]
fn configuration_references_withhold_each_supported_component() {
    let (_temp, root) = fixture();
    write(&root, "skills/configured/SKILL.md", "${user_config.token}");
    write(&root, "commands/configured.md", "${user_config.token}");
    write(&root, "commands/good.md", "Good");
    write(&root, ".mcp.json", json!({"env":{"command":"cmd","env":{"TOKEN":"${user_config.token}"}},"url":{"url":"${user_config.url}"},"header":{"url":"https://example.test", "headers":{"token":"${user_config.token}"}},"arg":{"command":"cmd","args":["${user_config.token}"]},"good":{"command":"cmd"}}).to_string());
    let inventory = inspect(&root, "tools").expect("needs configuration");
    assert!(inventory.skills.is_empty());
    assert_eq!(inventory.commands.len(), 1);
    assert_eq!(inventory.mcp_servers.len(), 1);
    assert_eq!(inventory.not_loaded.len(), 6);
    assert!(
        inventory
            .not_loaded
            .iter()
            .all(|c| c.reason == "needs configuration")
    );
    assert!(needs_configuration("before ${user_config.api_token} after"));
    assert!(!needs_configuration("${user_config.unclosed"));
}

#[test]
fn variables_expand_once_in_all_mcp_fields_and_preserve_unknowns() {
    let (_temp, root) = fixture();
    let plugin = root.join("${CLAUDE_PROJECT_DIR}");
    let data = root.join("data");
    let project = root.join("project");
    let paths = PathVariables {
        plugin_root: &plugin,
        plugin_data: &data,
        project_dir: &project,
    };
    let expanded = substitute_variables(
        "${CLAUDE_PLUGIN_ROOT}|${CLAUDE_PLUGIN_DATA}|${CLAUDE_PROJECT_DIR}|${other}|${unterminated",
        paths,
    );
    assert_eq!(
        expanded,
        format!(
            "{}|{}|{}|${{other}}|${{unterminated",
            plugin.display(),
            data.display(),
            project.display()
        )
    );
    write(&root, ".mcp.json", json!({"local":{"command":"${CLAUDE_PLUGIN_ROOT}/server", "args":["${CLAUDE_PLUGIN_ROOT}/server.js", "${CLAUDE_PROJECT_DIR}"], "env":{"DATA":"${CLAUDE_PLUGIN_DATA}"}}, "remote":{"url":"${CLAUDE_PROJECT_DIR}/mcp", "headers":{"x-root":"${CLAUDE_PLUGIN_ROOT}"}}}).to_string());
    let inventory = inspect(&root, "tools").expect("variable declarations");
    let resolved = inventory.resolved_mcp(paths);
    assert_eq!(
        resolved[0].args[0],
        format!("{}/server.js", plugin.display())
    );
    assert_eq!(
        resolved[0].env["CLAUDE_PLUGIN_ROOT"],
        plugin.to_string_lossy()
    );
    assert_eq!(
        resolved[0].env["CLAUDE_PLUGIN_DATA"],
        data.to_string_lossy()
    );
    assert_eq!(resolved[0].env["DATA"], data.to_string_lossy());
    assert_eq!(
        resolved[1].url.as_deref(),
        Some(format!("{}/mcp", project.display()).as_str())
    );
    assert_eq!(resolved[1].headers["x-root"], plugin.to_string_lossy());
}

#[test]
fn digest_is_location_independent_and_covers_every_file() {
    let (_first, root) = fixture();
    let (_second, other) = fixture();
    for root in [&root, &other] {
        manifest(root, json!({"name":"tools", "unknown":{"z":1,"a":2}}));
        write(root, "commands/status.md", "Original\r\n");
        write(
            root,
            ".mcp.json",
            json!({"local":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/server.js"]}})
                .to_string(),
        );
    }
    let original = inspect(&root, "unused").expect("first").digest;
    assert_eq!(
        original,
        inspect(&other, "unused")
            .expect("same content elsewhere")
            .digest
    );
    write(&root, "notes.txt", "not loaded");
    assert_ne!(
        original,
        inspect(&root, "unused").expect("supporting file").digest
    );
    write(&root, "hooks/hooks.json", "hooks");
    let hooks = inspect(&root, "unused").expect("new presence").digest;
    assert_ne!(original, hooks);
    write(&root, "hooks/hooks.json", "different hooks");
    assert_ne!(
        hooks,
        inspect(&root, "unused")
            .expect("unsupported bytes covered")
            .digest
    );
    write(&root, "commands/status.md", "Changed");
    assert_ne!(
        hooks,
        inspect(&root, "unused")
            .expect("loaded bytes changed")
            .digest
    );
}

#[test]
fn mcp_whitespace_and_rejected_bytes_contribute_to_digest() {
    let (_temp, root) = fixture();
    write(
        &root,
        ".mcp.json",
        r#"{"b":{"command":"b","env":{"Z":"z","A":"a"}},"a":{"command":"a"},"sse":{"type":"sse","url":"https://old.test"}}"#,
    );
    let original = inspect(&root, "tools").expect("original").digest;
    write(
        &root,
        ".mcp.json",
        r#"{ "sse":{"url":"https://new.test","type":"sse"}, "a":{"command":"a"}, "b":{"env":{"A":"a","Z":"z"},"command":"b"} }"#,
    );
    assert_ne!(
        original,
        inspect(&root, "tools")
            .expect("canonical declarations")
            .digest
    );
    write(
        &root,
        ".mcp.json",
        r#"{"a":{"command":"changed"},"b":{"command":"b","env":{"Z":"z","A":"a"}},"sse":{"type":"sse","url":"https://new.test"}}"#,
    );
    assert_ne!(
        original,
        inspect(&root, "tools").expect("MCP changed").digest
    );
}

#[cfg(unix)]
#[test]
fn symlink_component_and_manifest_escapes_are_refused() {
    let (_temp, root) = fixture();
    let (_outside, outside) = fixture();
    write(&outside, "SKILL.md", "Outside");
    fs::create_dir_all(root.join("skills")).expect("skills");
    std::os::unix::fs::symlink(&outside, root.join("skills/escape")).expect("symlink");
    write(&root, "commands/good.md", "Good");
    assert!(matches!(
        inspect(&root, "tools"),
        Err(PluginError::PathEscape(_))
    ));
    write(
        &outside,
        "manifest.json",
        json!({"name":"evil"}).to_string(),
    );
    fs::create_dir_all(root.join(".claude-plugin")).expect("manifest directory");
    std::os::unix::fs::symlink(
        outside.join("manifest.json"),
        root.join(".claude-plugin/plugin.json"),
    )
    .expect("manifest symlink");
    assert!(matches!(
        inspect(&root, "tools"),
        Err(PluginError::PathEscape(_))
    ));
}

#[test]
fn malformed_and_unreadable_components_are_retained_as_problems() {
    let (_temp, root) = fixture();
    write(&root, "commands/bad.md", [0xff, 0xfe]);
    write(&root, "skills/bad/SKILL.md", [0xff]);
    write(&root, "commands/good.md", "Good");
    write(&root, ".mcp.json", "invalid JSON");
    manifest(
        &root,
        json!({"name":"tools", "skills":[false], "mcpServers":[42, {"invalid":{"command":42},"bundle":"./server.mcpb","good":{"command":"good"}}]}),
    );
    let inventory = inspect(&root, "unused").expect("failed components");
    assert_eq!(inventory.commands.len(), 1);
    assert_eq!(inventory.mcp_servers.len(), 1);
    assert_eq!(
        inventory
            .not_loaded
            .iter()
            .filter(|c| c.reason.contains("not UTF-8"))
            .count(),
        2
    );
    assert!(
        inventory
            .not_loaded
            .iter()
            .any(|c| c.reason.contains("MCP bundles"))
    );
    assert_eq!(inventory.not_loaded.len(), 7);
}

#[test]
fn command_source_and_unsupported_manifest_config_references_are_detected() {
    let (_temp, root) = fixture();
    manifest(
        &root,
        json!({"name":"tools","commands":{"config":{"source":"./${user_config.path}"}},"hooks":{"command":"${user_config.command}"}}),
    );
    let inventory = inspect(&root, "unused").expect("configuration references");
    assert_eq!(inventory.not_loaded.len(), 2);
    assert!(
        inventory
            .not_loaded
            .iter()
            .all(|c| c.reason == "needs configuration")
    );
}

#[test]
fn mcp_declaration_files_cannot_recursively_include_themselves() {
    let (_temp, root) = fixture();
    for value in [
        json!("./.mcp.json"),
        json!({"mcpServers":"./.mcp.json"}),
        json!(["./.mcp.json"]),
    ] {
        write(&root, ".mcp.json", value.to_string());
        let inventory = inspect(&root, "tools").expect("malformed declaration file");
        assert!(inventory.mcp_servers.is_empty());
        assert_eq!(inventory.not_loaded.len(), 1);
        assert!(inventory.not_loaded[0].reason.contains("server map"));
    }
}

#[test]
fn overridden_component_files_contribute_to_digest() {
    let (_temp, root) = fixture();
    write(&root, "skills/review/SKILL.md", "Default review");
    write(&root, "extra/review/SKILL.md", "Loaded review");
    write(&root, "first/status.md", "Overridden command");
    write(&root, "second/status.md", "Loaded command");
    manifest(
        &root,
        json!({"name":"tools","skills":"./extra","commands":["./first", "./second"]}),
    );
    let original = inspect(&root, "unused").expect("override");
    assert_eq!(original.skills.len(), 1);
    assert_eq!(original.commands.len(), 1);
    write(&root, "skills/review/SKILL.md", "Changed overridden skill");
    write(&root, "first/status.md", "Changed overridden command");
    assert_ne!(
        original.digest,
        inspect(&root, "unused").expect("covered overrides").digest
    );
}

#[test]
fn unsupported_files_and_command_metadata_report_configuration_references() {
    let (_temp, root) = fixture();
    write(&root, "hooks/hooks.json", "${user_config.hook}");
    write(
        &root,
        "agents/nested/helper.md",
        format!("{}${{user_config.token}}", "x".repeat(8190)),
    );
    manifest(
        &root,
        json!({"name":"tools","commands":{"inline":{"content":"Body","description":"${user_config.summary}"}}}),
    );
    let inventory = inspect(&root, "unused").expect("all component references");
    assert!(inventory.commands.is_empty());
    assert_eq!(inventory.not_loaded.len(), 3);
    assert!(
        inventory
            .not_loaded
            .iter()
            .all(|item| item.reason == "needs configuration")
    );
}

#[test]
fn unsupported_custom_manifest_paths_detect_configuration_without_loading() {
    let (_temp, root) = fixture();
    write(&root, "custom-hooks.json", "${user_config.hook}");
    write(&root, "custom-agent.md", "${user_config.agent}");
    manifest(
        &root,
        json!({"name":"tools","hooks":"./custom-hooks.json","agents":["./custom-agent.md"]}),
    );
    let inventory = inspect(&root, "unused").expect("custom unsupported paths");
    assert_eq!(inventory.not_loaded.len(), 2);
    assert!(
        inventory
            .not_loaded
            .iter()
            .all(|item| item.reason == "needs configuration")
    );
}

#[test]
fn default_enabled_is_a_standard_effective_manifest_field() {
    let (_temp, root) = fixture();
    manifest(&root, json!({"name":"tools", "defaultEnabled":false}));
    assert!(!read_manifest(&root, "unused").unwrap().default_enabled);
    write(
        &root,
        ".smith-plugin/plugin.json",
        json!({"description":"overlay inherits"}).to_string(),
    );
    assert!(!read_manifest(&root, "unused").unwrap().default_enabled);
    manifest(&root, json!({"name":"tools", "defaultEnabled":"false"}));
    assert!(read_manifest(&root, "unused").is_err());
    manifest(&root, json!({"name":"tools"}));
    assert!(read_manifest(&root, "unused").unwrap().default_enabled);
}

#[test]
fn vcs_metadata_is_excluded_but_paths_and_binary_bytes_are_covered() {
    let (_temp, root) = fixture();
    write(&root, "server.js", [0, 1, 255]);
    let original = inspect(&root, "tools").unwrap().digest;
    for vcs in [".git", ".hg", ".svn", ".bzr"] {
        write(&root, &format!("{vcs}/metadata"), "changes independently");
    }
    assert_eq!(original, inspect(&root, "tools").unwrap().digest);
    fs::rename(root.join("server.js"), root.join("other.js")).unwrap();
    assert_ne!(original, inspect(&root, "tools").unwrap().digest);
}
