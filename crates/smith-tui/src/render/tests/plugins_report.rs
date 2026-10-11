use smith_client::plugins_report::{PluginComponent, PluginRow, PluginState, PluginsReport, UnknownPluginRow};

use super::{App, LocalResult, Theme, transcript_lines};

#[test]
fn every_plugin_state_renders_with_components_reasons_and_cli_guidance() {
    for (state, expected) in [
        (PluginState::On, "on"), (PluginState::Off, "off"), (PluginState::Changed, "content changed"),
        (PluginState::Untrusted("approval denied".into()), "approval denied"),
        (PluginState::Failed("manifest is invalid".into()), "manifest is invalid"),
    ] {
        let result = LocalResult::Plugins(Box::new(PluginsReport {
            rows: vec![PluginRow {
                name: "deploy-tools".into(), state, source: "./fixture-plugin".into(), version: Some("1.2".into()),
                resolved_commit: Some("abc123".into()), digest: "sha256:1234".into(), enabled_source: "user config".into(),
                loaded: vec![PluginComponent { kind: "skill".into(), name: "deploy-tools:deploy".into(), detail: None }],
                not_loaded: vec![PluginComponent { kind: "hooks".into(), name: "deploy-tools:hooks".into(), detail: Some("will not run".into()) }],
            }], unknown: vec![UnknownPluginRow { name: "removed".into(), source: "project config".into() }],
        }));
        let mut app = App::new("model", "project");
        app.show_local_report(result);
        assert!(app.overlay.is_none());
        for width in [44, 100, 180] {
            let text = transcript_lines(&app, Theme::new().without_color(), width).iter().map(ToString::to_string).collect::<Vec<_>>().join("\n");
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            for label in ["/plugins", expected, "./fixture-plugin", "deploy-tools:deploy", "will not run", "removed", "not installed", "smith plugin"] {
                assert!(text.contains(label), "{width}: {text}");
            }
        }
    }
    let mut app = App::new("model", "project");
    app.show_local_report(LocalResult::Plugins(Box::default()));
    let text = transcript_lines(&app, Theme::new(), 100).iter().map(ToString::to_string).collect::<Vec<_>>().join("\n");
    assert!(text.contains("no plugins are installed"));
}

#[test]
fn plugin_skill_command_and_mcp_rows_keep_their_namespace_and_attribution() {
    use smith_client::mcp_report::{McpReport, McpServer, McpServerState};
    use smith_client::skills_report::{SkillEntry, SkillGroup, SkillState, SkillsReport};
    use smith_runtime::skills::SmithSkillLayer;
    let reports = [
        LocalResult::Skills(Box::new(SkillsReport::Indexed { groups: vec![SkillGroup { layer: SmithSkillLayer::Plugin, entries: vec![SkillEntry { name: "deploy-tools:deploy".into(), description: "Deploy a service".into(), state: SkillState::Active }] }], problems: Vec::new() })),
        LocalResult::Mcp(Box::new(McpReport::Servers(vec![McpServer { name: "deploy-tools:local".into(), transport: "stdio".into(), source: "user manifest".into(), state: McpServerState::Connecting, rejected: Vec::new(), values: Vec::new() }]))),
    ];
    for report in reports {
        let mut app = App::new("model", "project");
        app.show_local_report(report);
        let text = transcript_lines(&app, Theme::new(), 100).iter().map(ToString::to_string).collect::<Vec<_>>().join("\n");
        assert!(text.contains("deploy-tools:"), "{text}");
        assert!(text.contains("plugin deploy-tools"), "{text}");
    }
}
