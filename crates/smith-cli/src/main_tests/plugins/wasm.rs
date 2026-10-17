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

#[cfg(feature = "wasm-modules")]
#[tokio::test]
async fn relay_example_delivers_a_message_between_two_sessions() {
    let fixture = Fixture::new(true);
    let relay = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/wasm-module-relay")
        .canonicalize()
        .unwrap();
    let text = install(&fixture, &relay);
    assert!(text.contains("relay/module"), "{text}");
    assert!(!text.contains("session history"), "{text}");
    let config = fixture.config(None);
    let mut request = fixture.request(config.clone());
    request
        .plugin_profiles
        .get_mut(&config.agent.profile.name)
        .unwrap()
        .wasm_modules = composition(&fixture, &config);
    request.approval = Some(Arc::new(agent_runtime_core::approval::AllowAll));
    let raw = |name: &str, arguments: &str| {
        let mut events = agent_runtime::provider::fake::tool_call_fragments(
            0,
            &format!("{name}-call"),
            name,
            arguments,
        );
        events.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        ScriptedStream::new(events)
    };
    let call = |tool: &str, arguments: &str| raw(&format!("wasm_5_relay_relay__{tool}"), arguments);
    // Discovery offers only the closest few tools; the model activates the rest.
    let activate = |tool: &str| {
        raw(
            "registry.activate",
            &format!(r#"{{"ids":["tool:wasm_5_relay_relay__{tool}"]}}"#),
        )
    };
    // Naming a tool in the prompt activates it for that turn.
    let prompt = |tool: &str| UserInput::text(format!("call wasm_5_relay_relay__{tool}"));
    let stop = || {
        ScriptedStream::new(vec![ProviderStreamEvent::Finish {
            reason: FinishReason::Stop,
        }])
    };
    request.provider = Some(Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            call("join", r#"{"name":"alice"}"#),
            stop(),
            call("join", r#"{"name":"bob"}"#),
            stop(),
            activate("send"),
            call("send", r#"{"to":"alice","text":"ping from bob"}"#),
            stop(),
            activate("inbox"),
            call("inbox", "{}"),
            stop(),
        ],
    )));
    let runtime = smith_runtime::factory::build(
        smith_runtime::harness::resolve(smith_runtime::harness::HarnessSpec::trusted(request))
            .unwrap(),
    )
    .await
    .unwrap();
    let labels = || -> Vec<String> {
        runtime
            .module_status()
            .iter()
            .map(|status| status.label().to_owned())
            .collect()
    };
    assert_eq!(labels(), ["relay ready"]);
    let alice = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    alice.run(prompt("join")).await.unwrap();
    assert_eq!(labels(), ["relay alice"]);
    let bob = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    bob.run(prompt("join")).await.unwrap();
    bob.run(prompt("send")).await.unwrap();
    let sent = format!("{:?}", bob.history());
    assert!(sent.contains("sent to alice"), "{sent}");
    // The contributor runs without a history grant, so the module stays
    // mounted and the next turn of the recipient can read its mail.
    alice.run(prompt("inbox")).await.unwrap();
    let read = format!("{:?}", alice.history());
    assert!(read.contains("from bob: ping from bob"), "{read}");
    assert_eq!(labels(), ["relay alice"]);
    bob.shutdown().await.unwrap();
    alice.shutdown().await.unwrap();
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
        .find(|id| id.starts_with("deploy-tools/"))
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

#[cfg(feature = "wasm-modules")]
#[tokio::test]
async fn a_record_from_an_older_id_scheme_fails_closed_instead_of_running_ungranted() {
    let fixture = Fixture::new(true);
    bundle(
        &fixture,
        "deploy-tools",
        serde_json::json!({"workspace_read": true}),
        &std::fs::read(example().join("module.wasm")).unwrap(),
    );
    install(&fixture, &fixture.source);
    let record = fixture.user.join("plugins/installed.toml");
    let text = std::fs::read_to_string(&record).unwrap();
    assert!(text.contains("deploy-tools/module"), "{text}");
    std::fs::write(
        &record,
        text.replace(
            "deploy-tools/module",
            "wasm.0123456789abcdef.0123456789abcdef",
        ),
    )
    .unwrap();
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
        text.contains("failed") && text.contains("older module id"),
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
        .find(|id| id.starts_with("deploy-tools/"))
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

#[tokio::test]
async fn hashed_module_install_grants_and_configuration_survive_upgrade() {
    let fixture = Fixture::new(true);
    bundle(
        &fixture,
        "deploy-tools",
        serde_json::json!({
            "workspace_read": true, "network": ["api.example.com"],
            "data_egress": ["api.example.com"]
        }),
        &std::fs::read(example().join("module.wasm")).unwrap(),
    );
    install(&fixture, &fixture.source);
    let id = "deploy-tools/module";
    let alias = "wasm.28e97dd683bfc96f.a93923545d211ea8".to_owned();
    let record = fixture.user.join("plugins/installed.toml");
    let prior = std::fs::read_to_string(&record).unwrap();
    assert!(prior.contains(id));
    std::fs::write(&record, prior.replace(id, &alias)).unwrap();
    let before = std::fs::read(&record).unwrap();
    let stored = crate::plugins::open_store(&fixture.user).list().unwrap();
    let installed = stored
        .iter()
        .find(|plugin| plugin.name == "deploy-tools")
        .unwrap();
    assert!(installed.module_grants[&alias].workspace_read);
    assert!(matches!(
        crate::plugins::open_store(&fixture.user)
            .load("deploy-tools")
            .unwrap(),
        smith_plugin::InstalledState::Unchanged(_)
    ));

    write(
        &fixture.user,
        "config.toml",
        format!(
            "[persistence]\nenabled = false\n[modules.\"{alias}\"]\nenabled = true\n[plugins.deploy-tools.modules.\"{alias}\"]\nnetwork = false\n"
        ),
    );
    let request = crate::plugins::with_known_plugins(
        ResolveRequest::new(&fixture.project)
            .with_home_dir(&fixture.home)
            .with_known_modules(crate::modules::known_modules()),
    )
    .unwrap();
    let resolution = smith_config::resolve::resolve(&request).unwrap();
    assert!(resolution.config.modules[id].enabled.value);
    assert!(!resolution.config.modules.contains_key(&alias));
    let canonical = format!("modules.\"{id}\".enabled");
    let legacy = format!("modules.\"{alias}\".enabled");
    let explanation = resolution.provenance.explain(&canonical).unwrap();
    let old_explanation = resolution.provenance.explain(&legacy).unwrap();
    assert_eq!(explanation.value, old_explanation.value);
    assert_eq!(explanation.source, old_explanation.source);
    assert_eq!(explanation.source.key, legacy);
    let modules = composition(&fixture, &resolution.config);
    assert!(modules.enabled.contains(id));
    #[cfg(feature = "wasm-modules")]
    {
        let mut runtime_request = fixture.request(resolution.config.clone());
        runtime_request
            .plugin_profiles
            .get_mut(&resolution.config.agent.profile.name)
            .unwrap()
            .wasm_modules = modules.clone();
        let report = smith_runtime::factory::module_report(&runtime_request)
            .await
            .unwrap();
        let module = report
            .iter()
            .find(|module| module.descriptor.id == id)
            .unwrap();
        assert_eq!(module.state, smith_module::ModuleState::Mounted);
    }
    let descriptor = modules
        .compiled
        .iter()
        .map(smith_module::CompiledModule::descriptor)
        .chain(modules.known)
        .find(|module| module.id == id)
        .unwrap();
    let smith_module::ModuleOrigin::Sandboxed {
        granted_capabilities,
        ..
    } = &descriptor.origin
    else {
        panic!("sandbox origin")
    };
    assert!(granted_capabilities.contains(&smith_module::ModuleCapability::WorkspaceRead));
    assert!(!granted_capabilities.contains(&smith_module::ModuleCapability::Network));
    #[cfg(feature = "wasm-modules")]
    assert!(!modules.compiled.is_empty());
    let reports = vec![smith_module::ModuleReport {
        descriptor,
        state: smith_module::ModuleState::Off,
        health: None,
    }];
    let switch = crate::modules::prepare_switch(&fixture.user, &reports, &alias, false).unwrap();
    assert_eq!(switch.id, id);
    switch.edit.commit(true).unwrap().accept();
    let switched = smith_config::model::ConfigFile::parse(
        &std::fs::read_to_string(fixture.user.join("config.toml")).unwrap(),
    )
    .unwrap();
    assert_eq!(switched.modules[id].enabled, Some(false));
    assert_eq!(switched.modules[&alias].enabled, Some(false));
    let off = fixture.config(None);
    assert!(!off.modules[id].enabled.value);
    assert!(!composition(&fixture, &off).enabled.contains(id));
    assert_eq!(std::fs::read(&record).unwrap(), before);
}

#[cfg(feature = "wasm-modules")]
#[tokio::test]
async fn module_complete_uses_current_route_and_accounts_module_usage() {
    use agent_runtime_core::usage::{CounterKind, UsageDelta};
    let fixture = Fixture::new(true);
    bundle(
        &fixture,
        "model-tools",
        serde_json::json!({"model": true}),
        include_bytes!("../../../../smith-wasm-host/tests/fixtures/v0.2/complete.wasm"),
    );
    let installed = install(&fixture, &fixture.source);
    assert!(
        installed.contains("send data to the active model provider"),
        "{installed}"
    );
    let config = fixture.config(None);
    let mut request = fixture.request(config.clone());
    request
        .plugin_profiles
        .get_mut(&config.agent.profile.name)
        .unwrap()
        .wasm_modules = composition(&fixture, &config);
    request.approval = Some(Arc::new(agent_runtime_core::approval::AllowAll));
    let name = "wasm_11_model-tools_guest__probe";
    let raw = |name: &str, id: &str, arguments: &str| {
        let mut events = agent_runtime::provider::fake::tool_call_fragments(0, id, name, arguments);
        events.push(ProviderStreamEvent::Usage {
            delta: UsageDelta::new()
                .with(CounterKind::InputUncached, 3)
                .with(CounterKind::Output, 2),
        });
        events.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        ScriptedStream::new(events)
    };
    let model = Arc::new(FakeProvider::new(
        config.model.value.clone(),
        Capabilities::basic_streaming(),
        vec![
            raw(
                "registry.activate",
                "activate",
                &format!(r#"{{"ids":["tool:{name}"]}}"#),
            ),
            raw(name, "module-call", "{}"),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "module model answer".into(),
                },
                ProviderStreamEvent::Usage {
                    delta: UsageDelta::new()
                        .with(CounterKind::InputUncached, 50)
                        .with(CounterKind::Output, 10),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::Usage {
                    delta: UsageDelta::new()
                        .with(CounterKind::InputUncached, 3)
                        .with(CounterKind::Output, 2),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    request.provider = Some(model.clone());
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
    let history = format!("{:?}", session.history());
    assert!(
        history.contains("module model answer")
            && history.contains("Output")
            && history.contains("10"),
        "{history}"
    );
    let requests = model.requests();
    assert_eq!(requests.len(), 4);
    let side = &requests[2];
    assert_eq!(side.model.as_str(), config.model.value);
    assert!(side.tools.is_empty());
    assert_eq!(
        side.tool_choice,
        agent_runtime_core::provider::ToolChoice::None
    );
    assert!(side.reasoning.is_none());
    assert_eq!(side.max_output_tokens, Some(32));
    assert_eq!(side.messages.len(), 2);
    assert_eq!(side.messages[0].joined_text(), "fixture system");
    assert_eq!(side.messages[1].joined_text(), "fixture prompt");
    let snapshot = runtime.accounted_snapshot(&session);
    let records = snapshot.usage.records();
    assert_eq!(
        records
            .iter()
            .filter(
                |record| record.provenance.purpose.as_deref() == Some("module:model-tools/module")
            )
            .count(),
        1
    );
    assert_eq!(snapshot.usage.total().get(CounterKind::InputUncached), 59);
    assert_eq!(snapshot.usage.total().get(CounterKind::Output), 16);
    let mut status = smith_client::status::Status::new(&config.model.value, "fixture");
    status.set_price(Some(smith_client::status::PriceReference {
        provider: config.provider.name.value.clone(),
        model: config.model.value.clone(),
        table: smith_client::status::PriceTable {
            input: Some(1_000_000),
            output: Some(2_000_000),
            cache_read: None,
            cache_write: None,
        },
    }));
    for record in records {
        status.record_usage_record(record);
    }
    let usage = status.session_usage();
    assert_eq!(usage.merged_total_tokens(), 75);
    assert!(
        usage
            .render()
            .unwrap()
            .contains("module model-tools/module")
    );
    let price = status.price().unwrap();
    assert_eq!(
        smith_client::status::SessionCost::compute(&usage, price).micro_usd,
        91
    );
    let logged = smith_client::usage_log::SessionUsageRecord::new(
        snapshot.id.to_string(),
        Some(config.provider.name.value.clone()),
        config.model.value,
        "fixture",
        &usage,
    );
    assert_eq!(
        logged.module_usage["model-tools/module"][0].totals["output"],
        10
    );
    session.shutdown().await.unwrap();
}

#[cfg(feature = "wasm-modules")]
#[test]
fn model_and_notify_review_and_user_restrictions_are_visible() {
    let fixture = Fixture::new(true);
    let bytes = std::fs::read(example().join("module.wasm")).unwrap();
    bundle(
        &fixture,
        "model-tools",
        serde_json::json!({"model": true, "session_notify": true}),
        &bytes,
    );
    let text = install(&fixture, &fixture.source);
    assert!(
        text.contains("send data to the active model provider"),
        "{text}"
    );
    assert!(
        text.contains("notify the live session and start turns"),
        "{text}"
    );
    let project_text = std::fs::read_to_string(fixture.project.join(".smith/config.toml")).unwrap();
    write(
        &fixture.project,
        ".smith/config.toml",
        format!(
            "{project_text}\n[plugins.model-tools.modules.\"model-tools/module\"]\nmodel_max_calls_per_turn = 100\nmodel = true\n"
        ),
    );
    let config = fixture.config(None);
    assert!(config.plugins["model-tools"].modules.is_empty());
    assert!(
        config.plugins["model-tools"]
            .ignored_module_settings
            .iter()
            .any(|message| message.contains("ignored"))
    );
    write(
        &fixture.user,
        "config.toml",
        "[persistence]\nenabled = false\n[plugins.model-tools.modules.\"model-tools/module\"]\nmodel = false\nsession_notify = false\nmodel_max_output_tokens = 1024\nmodel_max_calls_per_turn = 1\n",
    );
    let config = fixture.config(None);
    let rule = &config.plugins["model-tools"].modules["model-tools/module"];
    assert_eq!(rule.model_max_output_tokens, Some(1024));
    assert_eq!(rule.model_max_calls_per_turn, Some(1));
    let composition = composition(&fixture, &config);
    let origin = &composition
        .compiled
        .iter()
        .find(|module| module.module.id() == "model-tools/module")
        .unwrap()
        .origin;
    let smith_module::ModuleOrigin::Sandboxed {
        withheld_capabilities,
        granted_capabilities,
        ..
    } = origin
    else {
        panic!("sandboxed")
    };
    assert!(
        withheld_capabilities
            .iter()
            .any(|label| label.contains("active model provider"))
    );
    assert!(
        withheld_capabilities
            .iter()
            .any(|label| label.contains("notify the live session"))
    );
    assert!(!granted_capabilities.contains(&smith_module::ModuleCapability::Model));
    assert!(!granted_capabilities.contains(&smith_module::ModuleCapability::SessionNotify));
    write(
        &fixture.user,
        "config.toml",
        "[persistence]\nenabled = false\n[plugins.model-tools.modules.\"model-tools/module\"]\nmodel_max_output_tokens = 4097\nmodel_max_calls_per_turn = 5\n",
    );
    let config = fixture.config(None);
    assert_eq!(
        config.plugins["model-tools"].modules["model-tools/module"].model_max_output_tokens,
        None
    );
    assert_eq!(
        config.plugins["model-tools"].modules["model-tools/module"].model_max_calls_per_turn,
        None
    );
    assert!(
        config.plugins["model-tools"]
            .ignored_module_settings
            .iter()
            .any(|message| message.contains("model_max_calls_per_turn"))
    );
}

#[cfg(feature = "wasm-modules")]
#[tokio::test]
async fn user_withheld_notify_refuses_the_watcher_and_lists_the_reason() {
    let fixture = Fixture::new(true);
    bundle(
        &fixture,
        "watch-tools",
        serde_json::json!({"session_notify": true}),
        include_bytes!("../../../../smith-wasm-host/tests/fixtures/v0.2/watcher.wasm"),
    );
    install(&fixture, &fixture.source);
    write(
        &fixture.user,
        "config.toml",
        "[persistence]\nenabled = false\n[plugins.watch-tools.modules.\"watch-tools/module\"]\nsession_notify = false\n",
    );
    let config = fixture.config(None);
    let mut request = fixture.request(config.clone());
    request
        .plugin_profiles
        .get_mut(&config.agent.profile.name)
        .unwrap()
        .wasm_modules = composition(&fixture, &config);
    let runtime = smith_runtime::factory::build(
        smith_runtime::harness::resolve(smith_runtime::harness::HarnessSpec::trusted(request))
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(runtime.mounted_modules().iter().all(|module| {
        !module.contributions.iter().any(|contribution| {
            matches!(
                contribution,
                smith_module::ModuleContribution::Watcher { .. }
            )
        })
    }));
    let text = smith_client::modules_report::render_plain(
        &smith_client::modules_report::module_report(runtime.module_report(), &config.modules),
    );
    assert!(
        text.contains("withheld by the user: notify the live session"),
        "{text}"
    );
    assert!(
        text.contains("watcher refused: session_notify capability was not granted"),
        "{text}"
    );
    assert!(text.contains("mounted"), "{text}");
}

#[test]
fn model_and_notify_update_requests_require_review_before_granting() {
    let fixture = Fixture::new(true);
    let bytes = std::fs::read(example().join("module.wasm")).unwrap();
    bundle(&fixture, "model-tools", serde_json::json!({}), &bytes);
    install(&fixture, &fixture.source);
    bundle(
        &fixture,
        "model-tools",
        serde_json::json!({"model": true, "session_notify": true}),
        &bytes,
    );
    let mut output = Vec::new();
    crate::plugin_command::run_with(
        crate::cli::PluginAction::Update {
            name: Some("model-tools".into()),
            yes: false,
        },
        ResolveRequest::new(&fixture.project).with_home_dir(&fixture.home),
        true,
        &mut std::io::Cursor::new("n\n"),
        &mut output,
    )
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("newly requested capabilities"), "{text}");
    assert!(
        text.contains("send data to the active model provider"),
        "{text}"
    );
    assert!(
        text.contains("notify the live session and start turns"),
        "{text}"
    );
    let installed = crate::plugins::open_store(&fixture.user).list().unwrap();
    let plugin = installed
        .iter()
        .find(|plugin| plugin.name == "model-tools")
        .unwrap();
    assert!(
        plugin
            .module_grants
            .values()
            .all(|caps| !caps.model && !caps.session_notify)
    );
}
