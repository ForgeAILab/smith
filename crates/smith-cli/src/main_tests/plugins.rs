//! Local-store plugins through the ordinary client and runtime paths.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_runtime::provider::fake::{FakeProvider, ScriptedStream};
use agent_runtime::runtime::StartSession;
use agent_runtime_core::content::UserInput;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
use agent_runtime_core::tool::{InvocationContext, PreparedToolCall, Tool, ToolOutcome, ToolSpec};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use smith_client::commands::{MenuRow, ParsedInput};
use smith_client::file_commands::{CommandCatalog, CommandLayer};
use smith_config::resolve::{Layer, ResolveRequest, ResolvedConfig, ResolvedMcp, resolve};
use smith_config::trust::{TrustDecision, TrustStatus, TrustStore};
use smith_host::ProjectWorkspace;
use smith_plugin::{InstalledPlugin, PluginSource};
use smith_runtime::factory::{HostSurface, PluginProfile, RuntimeRequest};
use smith_runtime::harness::{Capability, ModuleProvenance, ModuleTrust};
use smith_runtime::mcp::{McpConnected, McpConnector, McpOptions, McpState, McpSupervisor};
use smith_runtime::skills::{SmithSkillLayer, SmithSkillSources};
use smith_tui::app::{Action, App};

use crate::plugins::{
    ActivePlugins, PluginCatalog, PluginStatus, open_store, record_trust, trust_status,
    with_known_plugins,
};

struct Fixture {
    _home: tempfile::TempDir,
    _project: tempfile::TempDir,
    _source: tempfile::TempDir,
    home: PathBuf,
    project: PathBuf,
    source: PathBuf,
    user: PathBuf,
    installed: InstalledPlugin,
}

fn write(root: &Path, path: &str, body: impl AsRef<[u8]>) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

impl Fixture {
    fn new(trusted: bool) -> Self {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let home_path = home.path().canonicalize().unwrap();
        let project_path = project.path().canonicalize().unwrap();
        let source_path = source.path().canonicalize().unwrap();
        let user = home_path.join(".smith");
        std::fs::create_dir(&user).unwrap();
        write(
            &project_path,
            ".smith/config.toml",
            super::LOCAL_COMMAND_CONFIG,
        );
        write(&user, "config.toml", "[persistence]\nenabled = false\n");
        write(&source_path, ".claude-plugin/plugin.json", json!({"name":"deploy-tools", "commands":{"status":{"source":"./commands/status.md"},"inline":{"content":"Inline ${CLAUDE_PLUGIN_DATA} $ARGUMENTS"}}}).to_string());
        write(
            &source_path,
            "skills/deploy/SKILL.md",
            "---\ndescription: Deploy a service\n---\nDeploy from ${CLAUDE_PLUGIN_ROOT} in ${CLAUDE_PROJECT_DIR}",
        );
        write(
            &source_path,
            "commands/status.md",
            "---\ndescription: Inspect deployment\nargument-hint: [service]\n---\nStatus $ARGUMENTS at ${CLAUDE_PLUGIN_ROOT}",
        );
        write(
            &source_path,
            "hooks/hooks.json",
            json!({"hooks":{"SessionStart":[{"command":"touch SHOULD_NOT_EXIST"}]}}).to_string(),
        );
        write(&source_path, "server.js", "fixture server bytes");
        write(&source_path, ".mcp.json", json!({"mcpServers":{
            "local":{"command":"node", "args":["${CLAUDE_PLUGIN_ROOT}/server.js"], "env":{"PLUGIN_SETTING":"literal", "PLUGIN_TOKEN":"env:PLUGIN_TEST_TOKEN"}},
            "options":{"command":"node", "env":{"TOKEN":"${user_config.api_token}"}}
        }}).to_string());
        let store = open_store(&user);
        let prepared = store
            .prepare_install(
                &PluginSource::Local {
                    path: source_path.clone(),
                },
                None,
            )
            .unwrap();
        let installed = store.commit_install(prepared).unwrap();
        if trusted {
            record_trust(
                &user,
                &installed.name,
                &installed.digest,
                TrustDecision::Allow,
            )
            .unwrap();
        }
        Self {
            _home: home,
            _project: project,
            _source: source,
            home: home_path,
            project: project_path,
            source: source_path,
            user,
            installed,
        }
    }

    fn config(&self, profile: Option<&str>) -> ResolvedConfig {
        let mut request = with_known_plugins(
            ResolveRequest::new(&self.project)
                .with_home_dir(&self.home)
                .with_known_modules(crate::modules::known_modules()),
        )
        .unwrap();
        request.cli.profile = profile.map(str::to_owned);
        resolve(&request).unwrap().config
    }

    fn active(&self, config: &ResolvedConfig) -> ActivePlugins {
        PluginCatalog::discover(&self.user)
            .unwrap()
            .resolve(config, &self.project)
            .unwrap()
    }

    fn catalog(&self, config: &ResolvedConfig) -> CommandCatalog {
        let trust = TrustStore::open(&self.user).unwrap();
        self.active(config).commands(CommandCatalog::discover(
            Some(&self.user),
            Some(&self.project),
            &trust,
        ))
    }

    fn request(&self, config: ResolvedConfig) -> RuntimeRequest {
        let plugins = self.active(&config);
        let (skills, _) = crate::skills::SkillContext::compose(
            plugins.skills(SmithSkillSources::new()).unwrap(),
            &self.user,
            &self.project,
        )
        .unwrap();
        let profile = PluginProfile {
            skills: plugins.skills(SmithSkillSources::new()).unwrap(),
            modules: plugins.modules().unwrap(),
            ..PluginProfile::default()
        };
        let mut request = RuntimeRequest::new(config, HostSurface::Headless);
        request.workspace = Some(Arc::new(ProjectWorkspace::new(&self.project).unwrap()));
        request.approval = Some(Arc::new(agent_runtime_core::approval::DenyAll));
        request.skills = skills;
        request
            .plugin_profiles
            .insert(request.config.agent.profile.name.clone(), profile);
        request
    }
}

#[test]
fn plugin_commands_use_tui_reducer_completion_help_and_headless_expansion() {
    let fixture = Fixture::new(true);
    let config = fixture.config(None);
    let catalog = fixture.catalog(&config);
    let mut app = App::new("example-model", "project");
    app.set_command_catalog(catalog.clone());
    app.composer.replace("/deploy-tools:status api");
    let Some(Action::FileCommand {
        typed,
        name,
        arguments,
        ..
    }) = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("plugin command must be a normal file-command action");
    };
    assert_eq!(name, "deploy-tools:status");
    let commands = crate::local_command::file_commands::CommandContext::discover(
        &fixture.user,
        &fixture.project,
    )
    .unwrap()
    .with_plugins(&config)
    .unwrap();
    let prepared = commands.prepare(&app, typed, &name, &arguments).unwrap();
    let expected = format!(
        "Status api at {}",
        fixture.installed.install_directory.display()
    );
    assert_eq!(prepared.committed_text(), expected);
    assert!(matches!(
        smith_client::commands::parse_with("/deploy-tools:status api", &catalog).unwrap(),
        ParsedInput::File { .. }
    ));
    let completion = smith_client::commands::matches_with("/deploy-tools:sta", &catalog);
    assert!(completion.iter().any(
        |row| matches!(row, MenuRow::File(entry) if entry.command.layer == CommandLayer::Plugin)
    ));
    assert_eq!(completion[0].layer_label(), Some("deploy-tools"));
    assert!(
        smith_client::commands_report::render_plain(
            &smith_client::commands_report::CommandsReport::from_catalog(&catalog)
        )
        .contains("plugin deploy-tools")
    );
    let help = smith_client::commands::help_with(&catalog);
    assert!(format!("{help:?}").contains("deploy-tools:status"));
    assert!(
        help.file_commands
            .iter()
            .any(|entry| entry.description.contains("plugin deploy-tools"))
    );
    let prompt = crate::headless::prepare_prompt_with_config(
        "/deploy-tools:status api",
        &fixture.user,
        &fixture.project,
        &config,
    )
    .unwrap();
    assert_eq!(prompt, expected);
    let data = open_store(&fixture.user)
        .data_directory("deploy-tools")
        .unwrap();
    assert_eq!(
        crate::headless::prepare_prompt_with_config(
            "/deploy-tools:inline api",
            &fixture.user,
            &fixture.project,
            &config
        )
        .unwrap(),
        format!("Inline {} api", data.display())
    );
    assert!(
        commands
            .confirmation("deploy-tools:status")
            .unwrap_err()
            .contains("plugin `deploy-tools` owns")
    );
    assert!(
        catalog
            .resolve("deploy-tools:status")
            .unwrap()
            .command
            .path
            .starts_with(&fixture.user)
    );
    assert!(
        !catalog
            .resolve("deploy-tools:status")
            .unwrap()
            .command
            .path
            .starts_with(&fixture.source)
    );
}

#[tokio::test]
async fn plugin_skills_and_user_skills_coexist_and_hooks_never_run() {
    let fixture = Fixture::new(true);
    write(
        &fixture.user,
        "skills/deploy/SKILL.md",
        "---\ndescription: User deployment guidance\n---\nUser body",
    );
    let config = fixture.config(None);
    let plugins = fixture.active(&config);
    let inventory = plugins.entries[0].inventory.as_ref().unwrap();
    assert!(
        inventory
            .not_loaded
            .iter()
            .any(|item| item.kind == "hooks" && item.reason.contains("not"))
    );
    assert!(inventory.not_loaded.iter().any(|item| item.name == "deploy-tools:options" && item.reason == "needs configuration"));
    assert!(!plugins.mcp_servers().contains_key("deploy-tools:options"));
    let mut request = fixture.request(config);
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(vec![
                ProviderStreamEvent::ToolCallDelta {
                    index: 0,
                    id: Some("activate-plugin".into()),
                    name: Some("registry.activate".into()),
                    arguments_fragment: r#"{"ids":["skill:deploy-tools:deploy"]}"#.into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::ToolCalls,
                },
            ]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta { text: "ok".into() },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    request.provider = Some(provider.clone());
    let runtime = smith_runtime::factory::build(
        smith_runtime::harness::resolve(smith_runtime::harness::HarnessSpec::trusted(request))
            .unwrap(),
    )
    .await
    .unwrap();
    let index = runtime.skill_index();
    assert!(
        index
            .iter()
            .any(|entry| entry.name() == "deploy" && entry.layer == SmithSkillLayer::User)
    );
    assert!(index.iter().any(
        |entry| entry.name() == "deploy-tools:deploy" && entry.layer == SmithSkillLayer::Plugin
    ));
    let record = runtime
        .harness_modules()
        .iter()
        .find(|module| module.provenance == ModuleProvenance::UserManifest("deploy-tools".into()))
        .unwrap();
    assert_eq!(record.trust, ModuleTrust::ContentOnly);
    assert!(record.granted_capabilities.is_empty());
    let sources = plugins.skills(SmithSkillSources::new()).unwrap();
    let selected = sources.resolve().unwrap();
    // Explicit activation tests the plugin even when the user skill wins retrieval.
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session
        .run(UserInput::text("Deploy a service with deploy-tools:deploy"))
        .await
        .unwrap();
    session.shutdown().await.unwrap();
    assert_eq!(selected.index()[0].name(), "deploy-tools:deploy");
    let sent = format!("{:?}", provider.requests());
    assert!(
        sent.contains(&format!(
            "Deploy from {} in {}",
            fixture.installed.install_directory.display(),
            fixture.project.display()
        )),
        "{sent}"
    );
    assert!(!sent.contains("${CLAUDE_PLUGIN_ROOT}"));
    assert!(!fixture.project.join("SHOULD_NOT_EXIST").exists());
    assert!(
        !fixture
            .installed
            .install_directory
            .join("SHOULD_NOT_EXIST")
            .exists()
    );
}

#[tokio::test]
async fn edited_installed_content_is_withheld_and_headless_session_still_runs() {
    let fixture = Fixture::new(true);
    write(
        &fixture.installed.install_directory,
        "commands/status.md",
        "Changed command",
    );
    let config = fixture.config(None);
    let plugins = fixture.active(&config);
    assert_eq!(plugins.entries[0].status, PluginStatus::Changed);
    assert!(plugins.modules().unwrap().is_empty());
    assert!(plugins.mcp_servers().is_empty());
    assert!(plugins.diagnostics()[0].contains("deploy-tools` withheld: content changed"));
    assert!(fixture.catalog(&config).entries().is_empty());
    let mut request = fixture.request(config);
    request.provider = Some(Arc::new(FakeProvider::text_reply("session continues")));
    let host = smith_runtime::host::start(smith_runtime::host::HostSessionRequest::new(
        request,
        &fixture.project,
    ))
    .await
    .unwrap();
    host.session().run(UserInput::text("hello")).await.unwrap();
    assert!(!host.runtime().harness_modules().iter().any(|module| matches!(&module.provenance, ModuleProvenance::UserManifest(name) if name == "deploy-tools")));
    host.session().shutdown().await.unwrap();
    assert!(
        record_trust(
            &fixture.user,
            "deploy-tools",
            &fixture.installed.digest,
            TrustDecision::Allow
        )
        .is_err()
    );
}

#[tokio::test]
async fn disabled_and_profile_disabled_plugins_leave_no_composition_record() {
    let fixture = Fixture::new(true);
    write(
        &fixture.user,
        "config.toml",
        "[plugins.deploy-tools]\nenabled = false\n[profiles.off]\nextends = \"dev\"\n[profiles.off.plugins.deploy-tools]\nenabled = false\n",
    );
    for profile in [None, Some("off")] {
        let config = fixture.config(profile);
        assert!(!config.plugins["deploy-tools"].enabled.value);
        if profile.is_some() {
            assert_eq!(
                config.plugins["deploy-tools"].enabled.source.layer,
                Layer::Profile
            );
        }
        let plugins = fixture.active(&config);
        assert_eq!(plugins.entries[0].status, PluginStatus::Off);
        assert!(plugins.modules().unwrap().is_empty());
        assert!(plugins.mcp_servers().is_empty());
        let mut request = fixture.request(config);
        request.provider = Some(Arc::new(FakeProvider::text_reply("ok")));
        let runtime = smith_runtime::factory::build(
            smith_runtime::harness::resolve(smith_runtime::harness::HarnessSpec::trusted(request))
                .unwrap(),
        )
        .await
        .unwrap();
        assert!(!runtime.harness_modules().iter().any(|module| matches!(&module.provenance, ModuleProvenance::UserManifest(name) if name == "deploy-tools")));
    }
}

#[test]
fn untrusted_plugin_mcp_notices_name_servers_and_require_plugin_confirmation() {
    for state in ["untrusted", "denied", "changed", "trusted", "off"] {
        let fixture = Fixture::new(matches!(state, "changed" | "trusted"));
        match state {
            "denied" => record_trust(
                &fixture.user,
                "deploy-tools",
                &fixture.installed.digest,
                TrustDecision::Deny,
            )
            .unwrap(),
            "changed" => write(
                &fixture.installed.install_directory,
                "server.js",
                "changed server bytes",
            ),
            "off" => write(
                &fixture.user,
                "config.toml",
                "[plugins.deploy-tools]\nenabled = false\n",
            ),
            _ => {}
        }
        let config = fixture.config(None);
        let plugins = fixture.active(&config);
        let notices = plugins.mcp_trust_notices(&config);
        if matches!(state, "trusted" | "off") {
            assert!(notices.is_empty(), "{state}: {notices:?}");
            continue;
        }
        assert!(
            plugins.mcp_servers().is_empty(),
            "unconfirmed servers stay inert"
        );
        assert_eq!(notices.len(), 1, "{state}: {notices:?}");
        assert_eq!(
            notices[0],
            "`deploy-tools:local` requires interactive trust — plugin `deploy-tools` owns this server's trust; confirm the plugin instead",
            "{state}"
        );
        assert!(!notices[0].contains("/mcp trust"));
    }
}

#[test]
fn admission_never_records_trust_and_denied_or_stale_decisions_withhold_everything() {
    let fixture = Fixture::new(false);
    let config = fixture.config(None);
    assert_eq!(
        trust_status(&fixture.user, "deploy-tools", &fixture.installed.digest).unwrap(),
        TrustStatus::Untrusted
    );
    let active = fixture.active(&config);
    assert_eq!(
        active.entries[0].status,
        PluginStatus::Untrusted(TrustStatus::Untrusted)
    );
    assert!(active.modules().unwrap().is_empty());
    assert!(!fixture.user.join("trust.json").exists());
    record_trust(
        &fixture.user,
        "deploy-tools",
        &fixture.installed.digest,
        TrustDecision::Deny,
    )
    .unwrap();
    assert_eq!(
        fixture.active(&config).entries[0].status,
        PluginStatus::Untrusted(TrustStatus::Denied)
    );
    record_trust(
        &fixture.user,
        "deploy-tools",
        &fixture.installed.digest,
        TrustDecision::Allow,
    )
    .unwrap();
    let store = open_store(&fixture.user);
    write(
        &fixture.source,
        "commands/status.md",
        "New reviewed snapshot",
    );
    store
        .commit_update(store.prepare_update("deploy-tools").unwrap())
        .unwrap();
    assert_eq!(
        fixture.active(&config).entries[0].status,
        PluginStatus::Changed
    );
    assert!(fixture.active(&config).modules().unwrap().is_empty());
}

#[derive(Debug)]
struct RemoteTool(agent_runtime_mcp::RemoteToolBinding);

#[async_trait]
impl Tool for RemoteTool {
    fn spec(&self) -> ToolSpec {
        self.0.spec.clone()
    }
    async fn invoke(
        &self,
        _: PreparedToolCall,
        _: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        Ok(ToolOutcome::text("remote reply"))
    }
}

#[derive(Debug, Default)]
struct Connector {
    configs: Mutex<Vec<agent_runtime_mcp::McpServerConfig>>,
}

#[async_trait]
impl McpConnector for Connector {
    async fn connect(
        &self,
        config: &agent_runtime_mcp::McpServerConfig,
    ) -> Result<McpConnected, agent_runtime_mcp::McpError> {
        self.configs.lock().unwrap().push(config.clone());
        let (bindings, rejected) = agent_runtime_mcp::client::bind_all(
            config,
            &[agent_runtime_mcp::RemoteTool::new("mutate")],
        );
        Ok(McpConnected {
            tools: bindings
                .into_iter()
                .map(|binding| Arc::new(RemoteTool(binding)) as Arc<dyn Tool>)
                .collect(),
            rejected,
            connection: None,
        })
    }
}

#[derive(Debug, Default)]
struct Approval {
    seen: Mutex<Vec<String>>,
}

#[async_trait]
impl agent_runtime_core::approval::ApprovalPolicy for Approval {
    async fn decide(
        &self,
        request: &agent_runtime_core::approval::ApprovalRequest,
    ) -> agent_runtime_core::approval::ApprovalDecision {
        self.seen
            .lock()
            .unwrap()
            .push(request.prepared().tool().to_owned());
        agent_runtime_core::approval::ApprovalDecision::Deny {
            reason: "fixture denial".into(),
        }
    }
}

#[tokio::test]
async fn plugin_mcp_uses_installed_paths_environment_bundle_trust_and_ordinary_approval() {
    let fixture = Fixture::new(true);
    let config = fixture.config(None);
    let plugins = fixture.active(&config);
    let mut servers = plugins.mcp_servers();
    // Keep the credential behind the normal secret seam without reading the
    // process environment in this hermetic transport fixture.
    assert!(
        servers["deploy-tools:local"].env["PLUGIN_TOKEN"]
            .value
            .credential()
            .is_some()
    );
    servers
        .get_mut("deploy-tools:local")
        .unwrap()
        .env
        .remove("PLUGIN_TOKEN");
    let connector = Arc::new(Connector::default());
    let trust = TrustStore::open(&fixture.user).unwrap();
    let mcp = McpSupervisor::planned_with(
        &ResolvedMcp {
            servers: servers.clone(),
        },
        &trust,
        McpOptions::new(&fixture.project).with_plugin_servers(servers.clone()),
        connector.clone(),
    )
    .unwrap();
    mcp.connect();
    mcp.settle(Duration::from_secs(2)).await;
    assert!(matches!(
        mcp.reports()[0].state,
        McpState::Connected { tools: 1 }
    ));
    {
        let configs = connector.configs.lock().unwrap();
        let agent_runtime_mcp::McpTransport::Stdio { args, env, .. } = &configs[0].transport else {
            panic!("stdio");
        };
        assert_eq!(
            args,
            &[fixture
                .installed
                .install_directory
                .join("server.js")
                .to_string_lossy()]
        );
        assert_eq!(
            env["CLAUDE_PLUGIN_ROOT"],
            fixture.installed.install_directory.to_string_lossy()
        );
        assert_eq!(
            env["CLAUDE_PLUGIN_DATA"],
            open_store(&fixture.user)
                .data_directory("deploy-tools")
                .unwrap()
                .to_string_lossy()
        );
        assert_eq!(env["PLUGIN_SETTING"], "literal");
        assert_eq!(
            std::fs::read_to_string(&args[0]).unwrap(),
            "fixture server bytes"
        );
    }
    let tool = mcp.tools()[0].spec().name.clone();
    let approval = Arc::new(Approval::default());
    let mut request = fixture.request(config.clone());
    request.mcp = Some(mcp.clone());
    request.approval = Some(approval.clone());
    request.provider = Some(Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(vec![
                ProviderStreamEvent::ToolCallDelta {
                    index: 0,
                    id: Some("call-plugin".into()),
                    name: Some(tool.clone()),
                    arguments_fragment: "{}".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::ToolCalls,
                },
            ]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "done".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    )));
    let runtime = smith_runtime::factory::build(
        smith_runtime::harness::resolve(smith_runtime::harness::HarnessSpec::trusted(request))
            .unwrap(),
    )
    .await
    .unwrap();
    let module = runtime
        .harness_modules()
        .iter()
        .find(|module| module.trust == ModuleTrust::TrustedMcp)
        .unwrap();
    assert_eq!(
        module.provenance,
        ModuleProvenance::UserManifest("deploy-tools".into())
    );
    assert!(
        module
            .requested_capabilities
            .contains(&Capability::ExternalWrite)
    );
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session
        .run(UserInput::text(format!(
            "Use {tool} to mutate the deployment"
        )))
        .await
        .unwrap();
    session.shutdown().await.unwrap();
    assert_eq!(*approval.seen.lock().unwrap(), vec![tool]);
    // The CLI surface refuses a redundant server-level confirmation.
    let context = crate::mcp::McpContext::start_with_plugins(
        &config,
        &fixture.user,
        &fixture.project,
        None,
        plugins.mcp_servers(),
    )
    .unwrap()
    .unwrap();
    assert!(
        context
            .confirmation("deploy-tools:local")
            .unwrap_err()
            .contains("plugin `deploy-tools` owns")
    );
    assert!(
        context
            .trust("deploy-tools:local")
            .unwrap_err()
            .contains("plugin `deploy-tools` owns")
    );
    context.supervisor().shutdown().await;
}

#[tokio::test]
async fn rebuild_observes_disable_install_update_and_remove_without_mutating_the_live_catalog() {
    let fixture = Fixture::new(true);
    let config = fixture.config(None);
    let initial = fixture.catalog(&config);
    assert!(initial.resolve("deploy-tools:status").is_some());
    let root_profile = fixture
        .active(&config)
        .profile(&config, &fixture.user, &fixture.project, None, None)
        .unwrap();
    assert_eq!(root_profile.skills.resolve().unwrap().index().len(), 1);
    // Model/module rebuilds re-resolve the profile and the installed catalog.
    write(
        &fixture.user,
        "config.toml",
        "[plugins.deploy-tools]\nenabled = false\n",
    );
    let off = fixture.config(None);
    let off_profile = fixture
        .active(&off)
        .profile(&off, &fixture.user, &fixture.project, None, None)
        .unwrap();
    assert!(off_profile.modules.is_empty());
    assert!(off_profile.skills.resolve().unwrap().index().is_empty());
    assert!(off_profile.servers.is_empty());
    assert!(initial.resolve("deploy-tools:status").is_some());
    // Each route (child and advisor included) consumes its own resolved switch.
    assert_eq!(root_profile.skills.resolve().unwrap().index().len(), 1);
    std::fs::remove_file(fixture.user.join("config.toml")).unwrap();
    write(&fixture.source, "commands/status.md", "Updated $ARGUMENTS");
    let store = open_store(&fixture.user);
    let installed = store
        .commit_update(store.prepare_update("deploy-tools").unwrap())
        .unwrap();
    assert!(fixture.catalog(&fixture.config(None)).entries().is_empty());
    record_trust(
        &fixture.user,
        "deploy-tools",
        &installed.digest,
        TrustDecision::Allow,
    )
    .unwrap();
    let updated = fixture.config(None);
    assert_eq!(
        crate::headless::prepare_prompt_with_config(
            "/deploy-tools:status api",
            &fixture.user,
            &fixture.project,
            &updated
        )
        .unwrap(),
        "Updated api"
    );
    store.remove("deploy-tools", false).unwrap();
    let removed = fixture.config(None);
    assert!(removed.plugins.is_empty());
    assert!(fixture.catalog(&removed).entries().is_empty());
    assert!(fixture.active(&removed).modules().unwrap().is_empty());
    if let Some(mcp) = root_profile.mcp {
        mcp.shutdown().await;
    }
}

#[tokio::test]
async fn malformed_installed_content_is_failed_and_does_not_stop_startup() {
    let fixture = Fixture::new(true);
    write(
        &fixture.installed.install_directory,
        ".claude-plugin/plugin.json",
        "not JSON",
    );
    let config = fixture.config(None);
    let plugins = fixture.active(&config);
    assert!(
        matches!(&plugins.entries[0].status, PluginStatus::Failed(reason) if !reason.is_empty())
    );
    assert!(plugins.modules().unwrap().is_empty());
    assert!(plugins.mcp_servers().is_empty());
    let mut request = fixture.request(config);
    request.provider = Some(Arc::new(FakeProvider::text_reply("still running")));
    let host = smith_runtime::host::start(smith_runtime::host::HostSessionRequest::new(
        request,
        &fixture.project,
    ))
    .await
    .unwrap();
    host.session().run(UserInput::text("hello")).await.unwrap();
    host.session().shutdown().await.unwrap();
}

#[test]
fn plugin_mcp_registry_names_do_not_alias_config_servers_or_other_namespaces() {
    let fixture = Fixture::new(true);
    let mut config = fixture.config(None);
    let registry = smith_config::mcp::registry_server_name("deploy-tools:local");
    assert_ne!(
        registry,
        smith_config::mcp::registry_server_name("deploy:tools-local")
    );
    let mut conflict = fixture
        .active(&config)
        .mcp_servers()
        .remove("deploy-tools:local")
        .unwrap();
    conflict.name = registry.clone();
    config.mcp.servers.insert(registry, conflict);
    let plugins = fixture.active(&config);
    assert!(plugins.mcp_servers().is_empty());
    assert_eq!(fixture.catalog(&config).runnable().count(), 2);
    assert!(
        plugins.entries[0]
            .inventory
            .as_ref()
            .unwrap()
            .not_loaded
            .iter()
            .any(|item| item.reason.contains("conflicts"))
    );
}

#[derive(Debug, Default)]
struct PendingConnector {
    entered: tokio::sync::Notify,
    dropped: std::sync::atomic::AtomicBool,
}

struct PendingGuard<'a>(&'a std::sync::atomic::AtomicBool);
impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait]
impl McpConnector for PendingConnector {
    async fn connect(
        &self,
        _: &agent_runtime_mcp::McpServerConfig,
    ) -> Result<McpConnected, agent_runtime_mcp::McpError> {
        let _guard = PendingGuard(&self.dropped);
        self.entered.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn retiring_a_plugin_supervisor_cancels_pending_connections() {
    let fixture = Fixture::new(true);
    let config = fixture.config(None);
    let mut servers = fixture.active(&config).mcp_servers();
    servers
        .get_mut("deploy-tools:local")
        .unwrap()
        .env
        .remove("PLUGIN_TOKEN");
    let connector = Arc::new(PendingConnector::default());
    let mcp = McpSupervisor::planned_with(
        &ResolvedMcp {
            servers: servers.clone(),
        },
        &TrustStore::open(&fixture.user).unwrap(),
        McpOptions::new(&fixture.project).with_plugin_servers(servers),
        connector.clone(),
    )
    .unwrap();
    mcp.connect();
    tokio::time::timeout(Duration::from_secs(2), connector.entered.notified())
        .await
        .unwrap();
    mcp.shutdown().await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while !connector.dropped.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(mcp.tools().is_empty());
}

#[test]
fn manifest_command_alias_uses_its_component_name_in_the_ordinary_reader() {
    let fixture = Fixture::new(true);
    write(
        &fixture.source,
        "commands/aliased.md",
        "---\nname: status\ndescription: Status alias\n---\nAlias $ARGUMENTS",
    );
    write(
        &fixture.source,
        ".claude-plugin/plugin.json",
        json!({"name":"deploy-tools", "commands":{"status":{"source":"./commands/aliased.md"}}})
            .to_string(),
    );
    let store = open_store(&fixture.user);
    let installed = store
        .commit_update(store.prepare_update("deploy-tools").unwrap())
        .unwrap();
    record_trust(
        &fixture.user,
        "deploy-tools",
        &installed.digest,
        TrustDecision::Allow,
    )
    .unwrap();
    let config = fixture.config(None);
    assert_eq!(
        crate::headless::prepare_prompt_with_config(
            "/deploy-tools:status api",
            &fixture.user,
            &fixture.project,
            &config
        )
        .unwrap(),
        "Alias api"
    );
}

mod end_to_end;
mod surfaces;

mod fixtures;
mod invocation;

mod wasm;

#[test]
fn resolution_metadata_reuses_one_catalog_for_main_child_and_advisor() {
    use smith_config::catalog::CatalogModality;
    use smith_config::model::ProfileUse;
    use smith_config::resolve::{ImageInput, Overrides};

    let home = tempfile::tempdir().expect("user root");
    let project = tempfile::tempdir().expect("project");
    let snapshot: smith_config::catalog::CatalogSnapshot =
        serde_json::from_str(smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED)
            .expect("embedded catalog");
    let model = snapshot
        .provider("google")
        .expect("Google catalog")
        .models
        .values()
        .find(|model| model.input_modalities.contains(&CatalogModality::Image))
        .expect("a catalog vision model")
        .id
        .clone();
    std::fs::create_dir(project.path().join(".smith")).expect("config directory");
    std::fs::write(
        project.path().join(".smith/config.toml"),
        format!(
            r#"
default_profile = "dev"
vision = "google/{model}"
advisor = "reviewer"
[profiles.dev]
provider = "google"
model = "{model}"
[profiles.worker]
provider = "google"
model = "{model}"
vision = "google/{model}"
use = ["child"]
[profiles.reviewer]
provider = "google"
model = "{model}"
[providers.google]
kind = "gemini-interactions"
"#
        ),
    )
    .expect("configuration");
    let request =
        with_known_plugins(ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("host resolution metadata");
    let catalog = request.catalog.clone().expect("frozen catalog");
    let request =
        with_known_plugins(request.with_catalog(catalog.clone())).expect("reuse frozen metadata");
    assert!(Arc::ptr_eq(request.catalog.as_ref().unwrap(), &catalog));
    let main = resolve(&request).expect("main validates catalog-only vision");
    let child = resolve(
        &request
            .clone()
            .with_cli(Overrides {
                profile: Some("worker".into()),
                ..Overrides::default()
            })
            .with_profile_use(ProfileUse::Child),
    )
    .expect("child validates catalog-only vision");
    let advisor = resolve(
        &request.with_advisor_route(
            main.config
                .agent
                .profile
                .advisor
                .clone()
                .expect("advisor target"),
        ),
    )
    .expect("advisor validates catalog-only vision");
    for resolution in [main, child, advisor] {
        assert_eq!(
            resolution.config.model_image_input.value,
            ImageInput::Accepts
        );
        assert!(resolution.config.agent.profile.vision.selected().is_some());
        let key = format!("models.\"google/{model}\".input_modalities");
        assert!(
            resolution
                .provenance
                .explain(&key)
                .expect("explanation")
                .source
                .key
                .starts_with("catalog google@")
        );
    }
}
