//! Plugin management transactions and user-config review through real host seams.

use std::io::Cursor;

use smith_client::commands::{HostCommand, PluginSwitchRequest, PluginsAction};
use smith_client::local_result::LocalResult;
use smith_config::trust::TrustStatus;

use super::{Fixture, write};
use crate::cli::{MarketplaceAction, PluginAction};
use crate::plugins::{commit_switch, open_store, prepare_switch, switch_outcome, trust_status};

fn run(
    fixture: &Fixture,
    action: PluginAction,
    terminal: bool,
    answer: &str,
) -> (anyhow::Result<()>, String) {
    let mut output = Vec::new();
    let result = crate::plugin_command::run_with(
        action,
        smith_config::resolve::ResolveRequest::new(&fixture.project)
            .with_home_dir(&fixture.home)
            .with_known_modules(crate::modules::known_modules()),
        terminal,
        &mut Cursor::new(answer),
        &mut output,
    );
    (result, String::from_utf8(output).unwrap())
}

fn fresh(fixture: &Fixture) {
    let installed = open_store(&fixture.user)
        .remove("deploy-tools", false)
        .unwrap();
    smith_config::trust::TrustStore::open(&fixture.user)
        .unwrap()
        .forget_executable(
            &fixture.user.join("plugins"),
            smith_config::trust::ExecutableKind::Extension,
            &installed.name,
        )
        .unwrap();
}

#[test]
fn add_prints_inventory_and_hooks_before_yes_or_terminal_confirmation() {
    for (yes, terminal, answer) in [(true, false, ""), (false, true, "y\n")] {
        let fixture = Fixture::new(false);
        fresh(&fixture);
        let (result, text) = run(
            &fixture,
            PluginAction::Add {
                source: fixture.source.display().to_string(),
                yes,
            },
            terminal,
            answer,
        );
        result.unwrap();
        for expected in [
            "Plugin deploy-tools",
            "source:",
            "resolved commit:",
            "deploy-tools:deploy",
            "deploy-tools:status",
            "MCP server deploy-tools:local",
            "server.js",
            "will not run",
            "needs configuration",
            "digest:",
            "Installed and trusted",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        let store = open_store(&fixture.user);
        let installed = &store.list().unwrap()[0];
        assert_eq!(
            trust_status(&fixture.user, &installed.name, &installed.digest).unwrap(),
            TrustStatus::Trusted
        );
        assert!(!fixture.project.join("SHOULD_NOT_EXIST").exists());
        assert!(text.contains(&installed.install_directory.display().to_string()));
        assert!(!text.contains("${CLAUDE_PLUGIN_ROOT}"));
    }
}

#[test]
fn add_without_a_terminal_never_reads_input_or_installs() {
    struct Unreadable;
    impl std::io::Read for Unreadable {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("nonterminal stdin was read")
        }
    }
    impl std::io::BufRead for Unreadable {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            panic!("nonterminal stdin was read")
        }
        fn consume(&mut self, _: usize) {
            panic!("nonterminal stdin was read")
        }
    }
    let fixture = Fixture::new(false);
    fresh(&fixture);
    let mut output = Vec::new();
    let error = crate::plugin_command::run_with(
        PluginAction::Add {
            source: fixture.source.display().to_string(),
            yes: false,
        },
        smith_config::resolve::ResolveRequest::new(&fixture.project).with_home_dir(&fixture.home),
        false,
        &mut Unreadable,
        &mut output,
    )
    .unwrap_err();
    assert!(error.to_string().contains("--yes"));
    assert!(String::from_utf8(output).unwrap().contains("will not run"));
    assert!(open_store(&fixture.user).list().unwrap().is_empty());
    assert!(
        smith_config::trust::TrustStore::open(&fixture.user)
            .unwrap()
            .records(&fixture.user.join("plugins"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn declining_add_writes_no_install_or_trust_and_default_is_no() {
    for answer in ["n\n", "\n", ""] {
        let fixture = Fixture::new(false);
        fresh(&fixture);
        let (result, text) = run(
            &fixture,
            PluginAction::Add {
                source: fixture.source.display().to_string(),
                yes: false,
            },
            true,
            answer,
        );
        result.unwrap();
        assert!(text.contains("Install and trust? [y/N]"));
        assert!(text.contains("Cancelled"));
        assert!(open_store(&fixture.user).list().unwrap().is_empty());
    }
}

#[test]
fn update_reviews_old_and_new_mcp_lines_and_retains_old_until_confirmation() {
    let fixture = Fixture::new(true);
    write(
        &fixture.source,
        ".mcp.json",
        r#"{"local":{"command":"new-node","args":["--new","${CLAUDE_PLUGIN_ROOT}/server.js"]}}"#,
    );
    let action = PluginAction::Update {
        name: Some("deploy-tools".into()),
        yes: false,
    };
    let (result, text) = run(&fixture, action.clone(), true, "n\n");
    result.unwrap();
    for expected in [
        "changed MCP deploy-tools:local",
        "old:",
        "new:",
        "\"node\"",
        "\"new-node\"",
        "old digest:",
        "new digest:",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    assert_eq!(
        open_store(&fixture.user).list().unwrap(),
        std::slice::from_ref(&fixture.installed)
    );
    let (result, _) = run(&fixture, action, true, "yes\n");
    result.unwrap();
    let installed = &open_store(&fixture.user).list().unwrap()[0];
    assert_ne!(installed.digest, fixture.installed.digest);
    assert_eq!(
        trust_status(&fixture.user, &installed.name, &installed.digest).unwrap(),
        TrustStatus::Trusted
    );
    assert!(
        !run(
            &fixture,
            PluginAction::Update {
                name: None,
                yes: true
            },
            false,
            ""
        )
        .1
        .contains("Update and trust?")
    );
    assert!(
        run(
            &fixture,
            PluginAction::Update {
                name: None,
                yes: true
            },
            false,
            ""
        )
        .1
        .contains("Already up to date")
    );
}

#[test]
fn edited_supporting_program_is_changed_withheld_and_restored_by_update_or_add() {
    for add in [false, true] {
        let fixture = Fixture::new(true);
        write(
            &fixture.installed.install_directory,
            "server.js",
            "local edits",
        );
        let config = fixture.config(None);
        let plugins = fixture.active(&config);
        assert_eq!(
            plugins.entries[0].status,
            crate::plugins::PluginStatus::Changed
        );
        assert!(plugins.mcp_servers().is_empty());
        assert!(
            plugins
                .skills(smith_runtime::skills::SmithSkillSources::new())
                .unwrap()
                .resolve()
                .unwrap()
                .index()
                .is_empty()
        );
        assert!(plugins.modules().unwrap().is_empty());
        assert!(fixture.catalog(&config).entries().is_empty());
        let action = if add {
            PluginAction::Add {
                source: fixture.source.display().to_string(),
                yes: false,
            }
        } else {
            PluginAction::Update {
                name: Some("deploy-tools".into()),
                yes: false,
            }
        };
        let (result, text) = run(&fixture, action.clone(), true, "n\n");
        result.unwrap();
        assert!(
            text.contains("Local edits") && text.contains("discarded on confirmation"),
            "{text}"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.installed.install_directory.join("server.js")).unwrap(),
            "local edits"
        );
        run(&fixture, action, true, "y\n").0.unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.installed.install_directory.join("server.js")).unwrap(),
            "fixture server bytes"
        );
        assert_eq!(
            fixture.active(&fixture.config(None)).entries[0].status,
            crate::plugins::PluginStatus::On
        );
    }
}

#[test]
fn remove_forgets_only_its_trust_and_retains_data_unless_requested() {
    for delete_data in [false, true] {
        let fixture = Fixture::new(true);
        let store = open_store(&fixture.user);
        let data = store.data_directory("deploy-tools").unwrap();
        write(&data, "saved.txt", "keep me");
        let other = smith_config::trust::Executable::new(
            smith_config::trust::ExecutableKind::Extension,
            "other",
            fixture.installed.digest.clone(),
        );
        let mut trust = smith_config::trust::TrustStore::open(&fixture.user).unwrap();
        trust
            .record(
                &fixture.user.join("plugins"),
                &other,
                smith_config::trust::TrustDecision::Allow,
            )
            .unwrap();
        run(
            &fixture,
            PluginAction::Remove {
                name: "deploy-tools".into(),
                delete_data,
            },
            false,
            "",
        )
        .0
        .unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(!fixture.installed.install_directory.exists());
        assert_eq!(data.exists(), !delete_data);
        assert_eq!(
            trust_status(&fixture.user, "deploy-tools", &fixture.installed.digest).unwrap(),
            TrustStatus::Untrusted
        );
        assert_eq!(
            smith_config::trust::TrustStore::open(&fixture.user)
                .unwrap()
                .status(&fixture.user.join("plugins"), &other)
                .unwrap(),
            TrustStatus::Trusted
        );
    }
}

#[test]
fn plugin_list_and_typed_report_cover_every_admission_state_and_unknown_settings() {
    for expected in ["on", "off", "changed", "untrusted", "failed"] {
        let fixture = Fixture::new(expected != "untrusted");
        match expected {
            "off" => write(
                &fixture.user,
                "config.toml",
                "plugins.deploy-tools.enabled = false\n",
            ),
            "changed" => write(&fixture.installed.install_directory, "server.js", "edited"),
            "failed" => write(
                &fixture.installed.install_directory,
                ".claude-plugin/plugin.json",
                "invalid JSON",
            ),
            _ => (),
        }
        let mut config_text = std::fs::read_to_string(fixture.user.join("config.toml")).unwrap();
        config_text.push_str("\nplugins.removed.enabled = false\n");
        // Use a top-level table so preceding fixture tables cannot capture the key.
        config_text = config_text.replace(
            "plugins.removed.enabled = false",
            "[plugins.removed]\nenabled = false",
        );
        write(&fixture.user, "config.toml", config_text);
        let (result, text) = run(&fixture, PluginAction::List, false, "");
        result.unwrap();
        assert!(
            text.contains(&format!("deploy-tools · {expected}")),
            "{text}"
        );
        assert!(text.contains("removed · not installed"), "{text}");
        if expected == "failed" {
            assert!(text.contains("expected"), "{text}");
        }
        if expected == "on" {
            assert!(text.contains("loaded: skill deploy-tools:deploy"), "{text}");
            assert!(text.contains("will not run"), "{text}");
        } else if expected != "failed" {
            assert!(
                text.contains("not loaded: skill deploy-tools:deploy"),
                "{text}"
            );
        }
        let config = fixture.config(None);
        let context = crate::local_command::file_commands::CommandContext::discover(
            &fixture.user,
            &fixture.project,
        )
        .unwrap()
        .with_plugins(&config)
        .unwrap();
        let report = context.plugins_report().unwrap();
        assert_eq!(
            text.trim(),
            smith_client::plugins_report::render_plain(&report)
        );
        let result = LocalResult::Plugins(Box::new(report));
        assert_eq!(result.title(), "plugins");
        let mut app = smith_tui::App::new("model", "project");
        app.show_local_report(result);
        assert!(app.overlay.is_none());
        assert!(
            !app.transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, smith_tui::Block::User { .. }))
        );
    }
}

#[test]
fn plugin_list_works_before_provider_setup_and_reports_user_switches() {
    let fixture = Fixture::new(true);
    std::fs::remove_file(fixture.project.join(".smith/config.toml")).unwrap();
    write(
        &fixture.user,
        "config.toml",
        "plugins.deploy-tools.enabled = false\nplugins.removed.enabled = true\n",
    );
    let (result, text) = run(&fixture, PluginAction::List, false, "");
    result.unwrap();
    assert!(text.contains("deploy-tools · off"), "{text}");
    assert!(text.contains("removed · not installed"), "{text}");
}

#[tokio::test]
async fn plugins_command_reviews_on_off_then_applies_at_boundary_or_rolls_back() {
    for enabled in [false, true] {
        let fixture = Fixture::new(true);
        let config = fixture.config(None);
        let context = crate::local_command::file_commands::CommandContext::discover(
            &fixture.user,
            &fixture.project,
        )
        .unwrap()
        .with_plugins(&config)
        .unwrap();
        let report = context.plugins_report().unwrap();
        let original = std::fs::read(fixture.user.join("config.toml")).unwrap();
        let prepared = prepare_switch(&fixture.user, &report, "deploy-tools", enabled).unwrap();
        assert!(
            prepared
                .edit
                .preview()
                .contains("plugins.deploy-tools.enabled")
        );
        assert_eq!(
            std::fs::read(fixture.user.join("config.toml")).unwrap(),
            original
        );
        let request = PluginSwitchRequest {
            name: "deploy-tools".into(),
            enabled,
            fingerprint: prepared.fingerprint,
        };
        let committed = commit_switch(&fixture.user, &report, &request).unwrap();
        let config = fixture.config(None);
        assert_eq!(config.plugins["deploy-tools"].enabled.value, enabled);
        assert_eq!(
            !fixture.active(&config).modules().unwrap().is_empty(),
            enabled
        );
        assert!(
            switch_outcome("deploy-tools", enabled, &config.plugins["deploy-tools"])
                .contains("safe boundary")
        );
        committed.rollback().unwrap();
        assert_eq!(
            std::fs::read(fixture.user.join("config.toml")).unwrap(),
            original
        );
    }
    let fixture = Fixture::new(true);
    let config = fixture.config(None);
    let mut request = fixture.request(config.clone());
    request.provider = Some(std::sync::Arc::new(
        agent_runtime::provider::fake::FakeProvider::text_reply("ok"),
    ));
    let host = smith_runtime::host::start(smith_runtime::host::HostSessionRequest::new(
        request,
        &fixture.project,
    ))
    .await
    .unwrap();
    let (_, skills) = crate::skills::SkillContext::compose(
        smith_runtime::skills::SmithSkillSources::new(),
        &fixture.user,
        &fixture.project,
    )
    .unwrap();
    let context = crate::local_command::file_commands::CommandContext::discover(
        &fixture.user,
        &fixture.project,
    )
    .unwrap()
    .with_plugins(&config)
    .unwrap();
    let mut app = smith_tui::App::new("model", "project");
    crate::local_command::handle_local_command(
        &mut app,
        &host,
        &fixture.project,
        None,
        &skills,
        &context,
        HostCommand::Plugins(PluginsAction::List),
    )
    .await;
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|block| matches!(block, smith_tui::Block::Local(LocalResult::Plugins(_))))
    );
    crate::local_command::handle_local_command(
        &mut app,
        &host,
        &fixture.project,
        None,
        &skills,
        &context,
        HostCommand::Plugins(PluginsAction::Switch {
            name: "deploy-tools".into(),
            enabled: false,
        }),
    )
    .await;
    assert!(matches!(app.overlay, Some(smith_tui::Overlay::Confirm(_))));
    assert!(fixture.config(None).plugins["deploy-tools"].enabled.value);
    host.session().shutdown().await.unwrap();
}

#[test]
fn plugin_enablement_reports_overrides_and_refuses_unknown_or_stale_previews() {
    let fixture = Fixture::new(true);
    write(
        &fixture.project,
        ".smith/config.toml",
        format!(
            "{}\n[plugins.deploy-tools]\nenabled = true\n",
            super::super::LOCAL_COMMAND_CONFIG
        ),
    );
    let config = fixture.config(None);
    let report = fixture
        .active(&config)
        .report(
            &fixture.user,
            &config.plugins,
            &config.unknown_plugins,
            &fixture.project,
        )
        .unwrap();
    let prepared = prepare_switch(&fixture.user, &report, "deploy-tools", false).unwrap();
    let request = PluginSwitchRequest {
        name: "deploy-tools".into(),
        enabled: false,
        fingerprint: prepared.fingerprint,
    };
    let committed = commit_switch(&fixture.user, &report, &request).unwrap();
    assert!(
        switch_outcome(
            "deploy-tools",
            false,
            &fixture.config(None).plugins["deploy-tools"]
        )
        .contains("project config overrides")
    );
    committed.rollback().unwrap();
    let error = prepare_switch(&fixture.user, &report, "unknown", false)
        .err()
        .unwrap();
    assert!(error.to_string().contains("installed names: deploy-tools"));
    let original = std::fs::read(fixture.user.join("config.toml")).unwrap();
    write(
        &fixture.user,
        "config.toml",
        "# changed after review\n[persistence]\nenabled = false\n",
    );
    assert!(
        commit_switch(&fixture.user, &report, &request)
            .err()
            .unwrap()
            .to_string()
            .contains("changed since the preview")
    );
    assert_ne!(
        std::fs::read(fixture.user.join("config.toml")).unwrap(),
        original
    );
}

#[test]
fn marketplace_registration_and_relative_install_work_for_paths_and_local_git() {
    for use_git in [false, true] {
        if use_git && !super::end_to_end::git_available() {
            continue;
        }
        let fixture = Fixture::new(false);
        fresh(&fixture);
        let marketplace = tempfile::tempdir().unwrap();
        let root = marketplace.path().canonicalize().unwrap();
        super::end_to_end::copy_tree(&fixture.source, &root.join("plugins/deploy-tools"));
        write(
            &root,
            ".claude-plugin/marketplace.json",
            r#"{"name":"local-market","plugins":[{"name":"deploy-tools","source":"./plugins/deploy-tools"},{"name":"unsupported","source":{"source":"npm","package":"example"}}]}"#,
        );
        let source = if use_git {
            super::end_to_end::init_repo(&root);
            format!("file://{}", root.display())
        } else {
            root.display().to_string()
        };
        let (result, text) = run(
            &fixture,
            PluginAction::Marketplace(MarketplaceAction::Add(source)),
            false,
            "",
        );
        result.unwrap();
        assert!(text.contains("Registered marketplace `local-market`"));
        assert!(
            run(
                &fixture,
                PluginAction::Marketplace(MarketplaceAction::List),
                false,
                ""
            )
            .1
            .contains("deploy-tools")
        );
        let (result, _) = run(
            &fixture,
            PluginAction::Add {
                source: "deploy-tools@local-market".into(),
                yes: true,
            },
            false,
            "",
        );
        result.unwrap();
        assert_eq!(
            open_store(&fixture.user).list().unwrap()[0].source,
            smith_plugin::PluginSource::Marketplace {
                plugin: "deploy-tools".into(),
                marketplace: "local-market".into()
            }
        );
        assert!(
            run(
                &fixture,
                PluginAction::Add {
                    source: "unsupported@local-market".into(),
                    yes: true
                },
                false,
                ""
            )
            .0
            .unwrap_err()
            .to_string()
            .contains("npm sources are not supported")
        );
        run(
            &fixture,
            PluginAction::Marketplace(MarketplaceAction::Update(None)),
            false,
            "",
        )
        .0
        .unwrap();
        run(
            &fixture,
            PluginAction::Marketplace(MarketplaceAction::Remove("local-market".into())),
            false,
            "",
        )
        .0
        .unwrap();
        assert!(
            open_store(&fixture.user)
                .list_marketplaces()
                .unwrap()
                .is_empty()
        );
        assert_eq!(open_store(&fixture.user).list().unwrap().len(), 1);
    }
}

#[test]
fn a_locally_broken_manifest_can_be_replaced_by_a_confirmed_source_update() {
    let fixture = Fixture::new(true);
    write(
        &fixture.installed.install_directory,
        ".claude-plugin/plugin.json",
        "broken JSON",
    );
    let (result, text) = run(
        &fixture,
        PluginAction::Update {
            name: Some("deploy-tools".into()),
            yes: true,
        },
        false,
        "",
    );
    result.unwrap();
    assert!(text.contains("Installed inventory unavailable"));
    assert!(text.contains("discarded on confirmation"));
    assert_eq!(
        fixture.active(&fixture.config(None)).entries[0].status,
        crate::plugins::PluginStatus::On
    );
}

#[test]
fn install_review_lists_components_rejected_by_the_ordinary_resolvers() {
    let fixture = Fixture::new(false);
    fresh(&fixture);
    write(
        &fixture.source,
        "commands/status.md",
        "---\nname: another-name\n---\nStatus",
    );
    write(
        &fixture.source,
        ".mcp.json",
        r#"{"local":{"url":"ftp://unsupported.example.test"}}"#,
    );
    let (result, text) = run(
        &fixture,
        PluginAction::Add {
            source: fixture.source.display().to_string(),
            yes: true,
        },
        false,
        "",
    );
    result.unwrap();
    assert!(
        text.contains("will not load: command deploy-tools:status"),
        "{text}"
    );
    assert!(
        text.contains("will not load: MCP server deploy-tools:local"),
        "{text}"
    );
    assert!(
        !text.contains("will load: command deploy-tools:status"),
        "{text}"
    );
    assert!(
        !text.contains("will load: MCP server deploy-tools:local"),
        "{text}"
    );
    let active = fixture.active(&fixture.config(None));
    assert!(active.mcp_servers().is_empty());
}
