//! Bundle capabilities, admission and runtime composition without a guest target.
use super::{Fixture, write};
#[cfg(feature = "wasm-modules")]
use agent_runtime::provider::fake::{FakeProvider, ScriptedStream};
#[cfg(feature = "wasm-modules")]
use agent_runtime::runtime::StartSession;
#[cfg(feature = "wasm-modules")]
use agent_runtime_core::{
    content::UserInput,
    provider::{Capabilities, FinishReason, ProviderStreamEvent},
};
use smith_config::{
    resolve::ResolveRequest,
    trust::{TrustDecision, TrustStatus},
};
use std::path::Path;
#[cfg(feature = "wasm-modules")]
use std::sync::Arc;

fn install(fixture: &Fixture, source: &Path) -> String {
    let mut output = Vec::new();
    crate::plugin_command::run_with(
        crate::cli::PluginAction::Add {
            source: source.display().to_string(),
            yes: true,
        },
        ResolveRequest::new(&fixture.project).with_home_dir(&fixture.home),
        false,
        &mut std::io::Cursor::new(""),
        &mut output,
    )
    .unwrap();
    String::from_utf8(output).unwrap()
}
fn bundle(fixture: &Fixture, name: &str, capabilities: serde_json::Value, bytes: &[u8]) {
    write(&fixture.source, ".smith-plugin/plugin.json", serde_json::json!({"name": name, "modules": [{"wasm": "./module.wasm", "capabilities": capabilities}]}).to_string());
    write(&fixture.source, "module.wasm", bytes);
}
fn composition(
    fixture: &Fixture,
    config: &smith_config::resolve::ResolvedConfig,
) -> smith_module::ModuleComposition {
    fixture
        .active(config)
        .wasm_modules(config, &crate::modules::ModuleSessions::default())
        .unwrap()
}
fn example() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/wasm-module-hello")
        .canonicalize()
        .unwrap()
}

#[cfg(feature = "wasm-modules")]
#[tokio::test]
async fn example_installs_and_calls_tool_through_headless_runtime_with_status() {
    let fixture = Fixture::new(true);
    write(&fixture.project, "README.md", "hello workspace");
    let text = install(&fixture, &example());
    assert!(text.contains("read files in the workspace"), "{text}");
    assert!(text.find("read files in the workspace").unwrap() < text.find("digest:").unwrap());
    let config = fixture.config(None);
    let mut request = fixture.request(config.clone());
    request
        .plugin_profiles
        .get_mut(&config.agent.profile.name)
        .unwrap()
        .wasm_modules = composition(&fixture, &config);
    request.approval = Some(Arc::new(agent_runtime_core::approval::AllowAll));
    let name = "wasm_17_wasm-module-hello_hello__hello";
    let mut call = agent_runtime::provider::fake::tool_call_fragments(0, "hello-call", name, "{}");
    call.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    request.provider = Some(Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(call),
            ScriptedStream::new(vec![ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            }]),
        ],
    )));
    let runtime = smith_runtime::factory::build(
        smith_runtime::harness::resolve(smith_runtime::harness::HarnessSpec::trusted(request))
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(
        runtime
            .module_status()
            .iter()
            .any(|status| status.label() == "hello ready")
    );
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session
        .run(UserInput::text(format!("call {name}")))
        .await
        .unwrap();
    assert!(format!("{:?}", session.history()).contains("Hello: README.md contains 15 bytes"));
    assert!(
        runtime
            .module_status()
            .iter()
            .any(|status| status.label() == "hello called")
    );
    session.shutdown().await.unwrap();
}

#[cfg(not(feature = "wasm-modules"))]
#[tokio::test]
async fn feature_free_build_lists_installed_wasm_as_not_supported() {
    let fixture = Fixture::new(true);
    install(&fixture, &example());
    let config = fixture.config(None);
    let mut request = fixture.request(config.clone());
    request
        .plugin_profiles
        .get_mut(&config.agent.profile.name)
        .unwrap()
        .wasm_modules = composition(&fixture, &config);
    let report = smith_runtime::factory::module_report(&request)
        .await
        .unwrap();
    let text = smith_client::modules_report::render_plain(
        &smith_client::modules_report::module_report(&report, &config.modules),
    );
    assert!(text.contains("not supported in this build"), "{text}");
    assert!(
        report
            .iter()
            .any(|module| module.state == smith_module::ModuleState::NotBuilt
                && matches!(
                    module.descriptor.origin,
                    smith_module::ModuleOrigin::Sandboxed { .. }
                ))
    );
}

#[test]
fn install_and_update_show_capabilities_and_keep_old_grants_until_confirmation() {
    let fixture = Fixture::new(true);
    let bytes = std::fs::read(example().join("module.wasm")).unwrap();
    bundle(
        &fixture,
        "deploy-tools",
        serde_json::json!({"network": ["api.example.com"]}),
        &bytes,
    );
    let text = install(&fixture, &fixture.source);
    assert!(text.contains("network: api.example.com"));
    let store = crate::plugins::open_store(&fixture.user);
    let old = store
        .list()
        .unwrap()
        .into_iter()
        .find(|plugin| plugin.name == "deploy-tools")
        .unwrap();
    bundle(
        &fixture,
        "deploy-tools",
        serde_json::json!({"network": ["api.example.com"], "workspace_write": true}),
        &bytes,
    );
    let update = store.prepare_update("deploy-tools").unwrap();
    assert_eq!(update.difference.changed_modules.len(), 1);
    assert!(
        update.difference.changed_modules[0]
            .new
            .capabilities
            .workspace_write
    );
    assert!(!old.module_grants.values().any(|caps| caps.workspace_write));
    let mut output = Vec::new();
    let result = crate::plugin_command::run_with(
        crate::cli::PluginAction::Update {
            name: Some("deploy-tools".into()),
            yes: false,
        },
        ResolveRequest::new(&fixture.project).with_home_dir(&fixture.home),
        true,
        &mut std::io::Cursor::new("n\n"),
        &mut output,
    );
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains(
            "newly requested capabilities [write files in the workspace] require confirmation"
        ),
        "{text}"
    );
    result.unwrap();
    assert_eq!(
        store
            .list()
            .unwrap()
            .into_iter()
            .find(|plugin| plugin.name == "deploy-tools")
            .unwrap()
            .digest,
        old.digest
    );
    let installed = store.commit_update(update).unwrap();
    assert_eq!(
        crate::plugins::trust_status(&fixture.user, &installed.name, &installed.digest).unwrap(),
        TrustStatus::Changed
    );
    crate::plugins::record_trust(
        &fixture.user,
        &installed.name,
        &installed.digest,
        TrustDecision::Allow,
    )
    .unwrap();
    assert!(
        installed
            .module_grants
            .values()
            .any(|caps| caps.workspace_write)
    );
}

#[test]
fn project_cannot_grant_and_user_only_narrows_confirmed_authority() {
    let fixture = Fixture::new(true);
    bundle(
        &fixture,
        "deploy-tools",
        serde_json::json!({"network": ["api.example.com"], "data_egress": ["api.example.com"], "workspace_read": true}),
        &std::fs::read(example().join("module.wasm")).unwrap(),
    );
    install(&fixture, &fixture.source);
    let config = fixture.config(None);
    let id = config
        .modules
        .keys()
        .find(|id| id.starts_with("wasm."))
        .unwrap()
        .clone();
    let project_text = std::fs::read_to_string(fixture.project.join(".smith/config.toml")).unwrap();
    write(
        &fixture.project,
        ".smith/config.toml",
        format!(
            "{project_text}\n[plugins.deploy-tools.modules.\"{id}\"]\nnetwork = [\"other.example.net\"]\nworkspace_write = true\n"
        ),
    );
    let config = fixture.config(None);
    assert!(config.plugins["deploy-tools"].modules.is_empty());
    assert!(
        config.plugins["deploy-tools"]
            .ignored_module_settings
            .iter()
            .any(|message| message.contains("ignored"))
    );
    write(
        &fixture.user,
        "config.toml",
        format!(
            "[persistence]\nenabled = false\n[plugins.deploy-tools.modules.\"{id}\"]\nnetwork = false\nworkspace_write = true\n"
        ),
    );
    let config = fixture.config(None);
    let composition = composition(&fixture, &config);
    let descriptor = composition
        .compiled
        .iter()
        .map(smith_module::CompiledModule::descriptor)
        .chain(composition.known)
        .find(|module| module.id == id)
        .unwrap();
    let smith_module::ModuleOrigin::Sandboxed {
        granted_capabilities,
        withheld_capabilities,
        ..
    } = descriptor.origin
    else {
        panic!("sandbox origin")
    };
    assert!(!granted_capabilities.contains(&smith_module::ModuleCapability::Network));
    assert!(!granted_capabilities.contains(&smith_module::ModuleCapability::WorkspaceWrite));
    assert!(withheld_capabilities.contains(&"network: api.example.com".into()));
}

#[cfg(feature = "wasm-modules")]
#[tokio::test]
async fn unknown_interface_version_is_failed_in_modules_report() {
    let fixture = Fixture::new(true);
    bundle(
        &fixture,
        "deploy-tools",
        serde_json::json!({}),
        include_bytes!("../../../../smith-wasm-host/tests/fixtures/unsupported.wasm"),
    );
    install(&fixture, &fixture.source);
    let config = fixture.config(None);
    let mut request = fixture.request(config.clone());
    request
        .plugin_profiles
        .get_mut(&config.agent.profile.name)
        .unwrap()
        .wasm_modules = composition(&fixture, &config);
    let report = smith_runtime::factory::module_report(&request)
        .await
        .unwrap();
    let text = smith_client::modules_report::render_plain(
        &smith_client::modules_report::module_report(&report, &config.modules),
    );
    assert!(
        text.contains("failed") && text.contains("0.1.0") && text.contains("9.0.0"),
        "{text}"
    );
}

#[test]
fn unknown_capability_refuses_only_its_module() {
    let fixture = Fixture::new(true);
    bundle(
        &fixture,
        "deploy-tools",
        serde_json::json!({"process": true}),
        &std::fs::read(example().join("module.wasm")).unwrap(),
    );
    write(
        &fixture.source,
        "good.wasm",
        std::fs::read(example().join("module.wasm")).unwrap(),
    );
    write(
        &fixture.source,
        ".smith-plugin/plugin.json",
        serde_json::json!({"name": "deploy-tools", "modules": [
            {"wasm": "./module.wasm", "capabilities": {"process": true}},
            {"wasm": "./good.wasm", "capabilities": {"workspace_read": true}}
        ]})
        .to_string(),
    );
    let inventory = smith_plugin::inspect(&fixture.source, "deploy-tools").unwrap();
    assert_eq!(inventory.modules.len(), 1);
    assert_eq!(inventory.modules[0].wasm, "good.wasm");
    assert!(
        inventory
            .not_loaded
            .iter()
            .any(|problem| problem.kind == "modules" && problem.reason.contains("unknown field"))
    );
    assert!(!inventory.skills.is_empty());
}

#[cfg(feature = "wasm-modules")]
#[tokio::test]
async fn user_withheld_network_fails_host_function_through_runtime() {
    let fixture = Fixture::new(true);
    bundle(
        &fixture,
        "deploy-tools",
        serde_json::json!({"network": ["api.example.com"], "data_egress": ["api.example.com"]}),
        include_bytes!("../../../../smith-wasm-host/tests/fixtures/tools.wasm"),
    );
    install(&fixture, &fixture.source);
    let config = fixture.config(None);
    let id = config
        .modules
        .keys()
        .find(|id| id.starts_with("wasm."))
        .unwrap();
    write(
        &fixture.user,
        "config.toml",
        format!(
            "[persistence]\nenabled = false\n[plugins.deploy-tools.modules.\"{id}\"]\nnetwork = false\n"
        ),
    );
    let config = fixture.config(None);
    let mut request = fixture.request(config.clone());
    request
        .plugin_profiles
        .get_mut(&config.agent.profile.name)
        .unwrap()
        .wasm_modules = composition(&fixture, &config);
    request.approval = Some(Arc::new(agent_runtime_core::approval::AllowAll));
    let name = "wasm_12_deploy-tools_guest__probe";
    let mut events = agent_runtime::provider::fake::tool_call_fragments(
        0,
        "http-call",
        name,
        r#"{"action":"http"}"#,
    );
    events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    request.provider = Some(Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(events),
            ScriptedStream::new(vec![ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            }]),
        ],
    )));
    let runtime = smith_runtime::factory::build(
        smith_runtime::harness::resolve(smith_runtime::harness::HarnessSpec::trusted(request))
            .unwrap(),
    )
    .await
    .unwrap();
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session
        .run(UserInput::text(format!("call {name}")))
        .await
        .unwrap();
    assert!(format!("{:?}", session.history()).contains("capability denied: Network"));
    session.shutdown().await.unwrap();
    let reports = runtime.module_report();
    let text = smith_client::modules_report::render_plain(
        &smith_client::modules_report::module_report(reports, &config.modules),
    );
    assert!(text.contains("withheld by the user: network: api.example.com"));
}

#[test]
fn install_confirmation_displays_network_before_installing_any_bytes() {
    let fixture = Fixture::new(true);
    bundle(
        &fixture,
        "network-example",
        serde_json::json!({"network": ["api.example.com"]}),
        &std::fs::read(example().join("module.wasm")).unwrap(),
    );
    let mut output = Vec::new();
    crate::plugin_command::run_with(
        crate::cli::PluginAction::Add {
            source: fixture.source.display().to_string(),
            yes: false,
        },
        ResolveRequest::new(&fixture.project).with_home_dir(&fixture.home),
        true,
        &mut std::io::Cursor::new("n\n"),
        &mut output,
    )
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains("sandboxed module") && text.contains("network: api.example.com"),
        "{text}"
    );
    assert!(text.find("network: api.example.com").unwrap() < text.find("digest:").unwrap());
    assert!(
        !crate::plugins::open_store(&fixture.user)
            .list()
            .unwrap()
            .iter()
            .any(|plugin| plugin.name == "network-example")
    );
}

#[test]
fn a_user_endpoint_list_intersects_without_widening_the_grant() {
    let confirmed = smith_plugin::ModuleCapabilities {
        network: vec!["api.example.com".into(), "second.example.com".into()],
        ..Default::default()
    };
    let restriction = smith_config::model::PluginModuleSection {
        network: Some(smith_config::model::ModuleEndpointRestriction::Hosts(vec![
            "api.example.com".into(),
            "ungranted.example.net".into(),
        ])),
        ..Default::default()
    };
    let narrowed = crate::plugins::wasm::narrow(confirmed, Some(&restriction));
    assert_eq!(narrowed.network, ["api.example.com"]);
}
