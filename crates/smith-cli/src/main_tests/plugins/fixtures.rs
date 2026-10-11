//! Golden plugin reports use the shared typed capture path.

use crate::tests::fixtures::{fixture_record, fixture_support};
use smith_client::local_result::LocalResult;
use smith_tui::App;

#[test]
fn fixtures_local_plugins_preserve_typed_states_and_component_reasons() {
    use smith_client::plugins_report::{
        PluginComponent, PluginRow, PluginState, PluginsReport, UnknownPluginRow,
    };
    for (name, state) in [
        ("on", PluginState::On),
        ("off", PluginState::Off),
        ("changed", PluginState::Changed),
        ("untrusted", PluginState::Untrusted("needs approval".into())),
        ("failed", PluginState::Failed("invalid manifest".into())),
    ] {
        let mut app = App::new("example-model", "<PROJECT>");
        let admitted = state == PluginState::On;
        let component = PluginComponent {
            kind: "skill".into(),
            name: "deploy-tools:deploy".into(),
            detail: if admitted { None } else { Some(state.label()) },
        };
        let mut not_loaded = vec![PluginComponent {
            kind: "hooks".into(),
            name: "deploy-tools:hooks".into(),
            detail: Some("will not run".into()),
        }];
        if !admitted {
            not_loaded.push(component.clone());
        }
        app.show_local_report(LocalResult::Plugins(Box::new(PluginsReport {
            rows: vec![PluginRow {
                name: "deploy-tools".into(),
                state,
                source: "<HOME>/plugin-source".into(),
                version: Some("1.0".into()),
                resolved_commit: None,
                digest: "<PLUGIN_DIGEST>".into(),
                enabled_source: "user config".into(),
                loaded: if admitted {
                    vec![component]
                } else {
                    Vec::new()
                },
                not_loaded,
            }],
            unknown: vec![UnknownPluginRow {
                name: "removed".into(),
                source: "project config".into(),
            }],
        })));
        fixture_record(
            &format!("plugins-{name}"),
            &app,
            &mut fixture_support::Normalizer::default(),
        );
    }
    let mut app = App::new("example-model", "<PROJECT>");
    app.show_local_report(LocalResult::Plugins(Box::default()));
    fixture_record(
        "plugins-empty",
        &app,
        &mut fixture_support::Normalizer::default(),
    );
}
