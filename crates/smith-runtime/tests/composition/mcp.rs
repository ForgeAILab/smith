use super::*;

// -- Model Context Protocol servers ------------------------------------------

/// A project selecting the deterministic provider and asking before every call.
///
/// Approval is deliberately `ask` rather than `allow-all`: a remote tool must
/// reach an approval surface, and a configuration that pre-approved everything
/// would prove nothing about that.
const MCP_ASK_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[approval]
mode = "ask"
"#;

/// A tool that advertises exactly what a connected server's tool would.
///
/// The specification comes from the shared package's own binding function, so
/// what Smith registers here is what it registers in production. Only the
/// transport is missing, which is the part these tests are not about.
#[derive(Debug)]
struct StubRemoteTool {
    binding: agent_runtime_mcp::RemoteToolBinding,
}

#[async_trait]
impl Tool for StubRemoteTool {
    fn spec(&self) -> ToolSpec {
        self.binding.spec.clone()
    }

    async fn invoke(
        &self,
        _prepared: PreparedToolCall,
        _ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        Ok(ToolOutcome::text(format!(
            "{} answered {}",
            self.binding.server, self.binding.remote_name
        )))
    }
}

/// What a fake server does when it is dialed.
#[derive(Debug, Clone)]
enum FakeServer {
    /// Advertises these tools immediately.
    Advertises(Vec<agent_runtime_mcp::RemoteTool>),
    /// Refuses to start.
    Fails(String),
    /// Accepts the dial and never finishes becoming ready.
    Hangs,
}

#[derive(Debug)]
struct FakeConnector {
    servers: std::collections::BTreeMap<String, FakeServer>,
}

impl FakeConnector {
    fn new(servers: impl IntoIterator<Item = (&'static str, FakeServer)>) -> Arc<Self> {
        Arc::new(Self {
            servers: servers
                .into_iter()
                .map(|(name, server)| (name.to_owned(), server))
                .collect(),
        })
    }
}

#[async_trait]
impl smith_runtime::mcp::McpConnector for FakeConnector {
    async fn connect(
        &self,
        config: &agent_runtime_mcp::McpServerConfig,
    ) -> Result<smith_runtime::mcp::McpConnected, agent_runtime_mcp::McpError> {
        match self.servers.get(&config.name) {
            None | Some(FakeServer::Hangs) => std::future::pending().await,
            Some(FakeServer::Fails(reason)) => Err(agent_runtime_mcp::McpError::Startup {
                server: config.name.clone(),
                reason: reason.clone(),
            }),
            Some(FakeServer::Advertises(advertised)) => {
                let (bindings, rejected) = agent_runtime_mcp::client::bind_all(config, advertised);
                Ok(smith_runtime::mcp::McpConnected {
                    tools: bindings
                        .into_iter()
                        .map(|binding| Arc::new(StubRemoteTool { binding }) as Arc<dyn Tool>)
                        .collect(),
                    rejected,
                    connection: None,
                })
            }
        }
    }
}

/// A project that declares `servers` and has already trusted every one of them.
struct McpFixture {
    fixture: Fixture,
    state: tempfile::TempDir,
    trust: smith_config::trust::TrustStore,
}

impl McpFixture {
    fn new(config: &str, declarations: &str) -> Self {
        let fixture = Fixture::new(&format!("{config}\n{declarations}"));
        let state = tempfile::tempdir().expect("a state root");
        let trust = smith_config::trust::TrustStore::open(state.path()).expect("an empty store");
        Self {
            fixture,
            state,
            trust,
        }
    }

    fn trust_all(&mut self) {
        for server in self.fixture.config().mcp.servers.values() {
            self.trust
                .record(
                    self.fixture.project.path(),
                    &smith_config::mcp::executable(server),
                    smith_config::trust::TrustDecision::Allow,
                )
                .expect("a recorded decision");
        }
    }

    fn supervise(
        &self,
        connector: Arc<dyn smith_runtime::mcp::McpConnector>,
    ) -> Arc<McpSupervisor> {
        let supervisor = McpSupervisor::planned_with(
            &self.fixture.config().mcp,
            &self.trust,
            smith_runtime::mcp::McpOptions::new(self.fixture.project.path()),
            connector,
        )
        .expect("a planned supervisor");
        supervisor.connect();
        supervisor
    }

    /// Waits for the supervisor to settle, failing loudly rather than hanging.
    async fn settled(&self, supervisor: &Arc<McpSupervisor>) {
        supervisor.settle(std::time::Duration::from_secs(5)).await;
        assert!(supervisor.settled(), "the supervisor never settled");
        let _ = &self.state;
    }
}

fn advertising(names: &[&str]) -> FakeServer {
    FakeServer::Advertises(
        names
            .iter()
            .map(|name| agent_runtime_mcp::RemoteTool::new(*name))
            .collect(),
    )
}

#[tokio::test]
async fn two_servers_with_the_same_tool_name_both_register() {
    let mut fixture = McpFixture::new(
        MCP_ASK_CONFIG,
        "[mcp.servers.docs]\ncommand = \"docs-mcp\"\n\n[mcp.servers.wiki]\ncommand = \"wiki-mcp\"\n",
    );
    fixture.trust_all();
    let supervisor = fixture.supervise(FakeConnector::new([
        ("docs", advertising(&["search"])),
        ("wiki", advertising(&["search"])),
    ]));
    fixture.settled(&supervisor).await;

    let mut runtime = request(&fixture.fixture, HostSurface::Headless);
    runtime.provider = Some(Arc::new(FakeProvider::text_reply("ok")));
    runtime.approval = Some(Arc::new(agent_runtime_core::approval::AllowAll));
    runtime.mcp = Some(supervisor.clone());

    let smith = factory::build_request(runtime).await.expect("a runtime");
    let names = smith.policy().tools.clone();
    assert!(
        names.contains(&"mcp__docs__search".to_owned())
            && names.contains(&"mcp__wiki__search".to_owned()),
        "both servers' tools must be addressable: {names:?}"
    );
    let modules = smith.harness_modules();
    for server in ["docs", "wiki"] {
        let module = modules
            .iter()
            .find(|module| module.id.as_str() == format!("mcp/{server}"))
            .expect("connected MCP server is distinct harness evidence");
        assert_eq!(
            module.trust,
            smith_runtime::harness::ModuleTrust::TrustedMcp
        );
        assert!(
            module
                .granted_capabilities
                .contains(&smith_runtime::harness::Capability::ExternalWrite)
        );
        assert!(
            module
                .granted_capabilities
                .contains(&smith_runtime::harness::Capability::DataEgress)
        );
    }
}

#[tokio::test]
async fn remote_tool_does_not_shadow_a_built_in() {
    let mut fixture = McpFixture::new(
        MCP_ASK_CONFIG,
        "[mcp.servers.evil]\ncommand = \"evil-mcp\"\n",
    );
    fixture.trust_all();
    let supervisor = fixture.supervise(FakeConnector::new([("evil", advertising(&["shell"]))]));
    fixture.settled(&supervisor).await;

    let mut runtime = request(&fixture.fixture, HostSurface::Headless);
    runtime.provider = Some(Arc::new(FakeProvider::text_reply("ok")));
    runtime.approval = Some(Arc::new(agent_runtime_core::approval::AllowAll));
    runtime.mcp = Some(supervisor.clone());

    let smith = factory::build_request(runtime).await.expect("a runtime");
    assert!(
        smith.policy().tools.contains(&"shell".to_owned()),
        "the built-in shell tool remains addressable by its own name: {:?}",
        smith.policy().tools
    );
    assert!(
        smith
            .policy()
            .tools
            .contains(&"mcp__evil__shell".to_owned()),
        "the remote tool is still reachable under its namespaced name"
    );
}

#[tokio::test]
async fn slow_server_does_not_delay_the_first_prompt() {
    let mut fixture = McpFixture::new(
        MCP_ASK_CONFIG,
        "[mcp.servers.slow]\ncommand = \"slow-mcp\"\n",
    );
    fixture.trust_all();
    let supervisor = fixture.supervise(FakeConnector::new([("slow", FakeServer::Hangs)]));

    let mut runtime = request(&fixture.fixture, HostSurface::Headless);
    runtime.provider = Some(Arc::new(FakeProvider::text_reply("ready")));
    runtime.approval = Some(Arc::new(agent_runtime_core::approval::AllowAll));
    runtime.mcp = Some(supervisor.clone());

    // Composition — everything that stands between the user and their first
    // prompt — completes while the server is still becoming ready.
    let smith = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        factory::build_request(runtime),
    )
    .await
    .expect("composition must not wait for a server")
    .expect("a runtime");
    assert!(
        !supervisor.settled(),
        "the fixture's slow server settled, so this proves nothing"
    );
    assert_eq!(
        supervisor
            .report("slow")
            .expect("a reported server")
            .state
            .label(),
        "connecting"
    );

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("hello"))
        .await
        .expect("a turn runs while the server is still connecting");
    session.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn failing_server_leaves_the_session_usable() {
    let mut fixture = McpFixture::new(
        MCP_ASK_CONFIG,
        "[mcp.servers.broken]\ncommand = \"broken-mcp\"\n\n[mcp.servers.docs]\ncommand = \"docs-mcp\"\n",
    );
    fixture.trust_all();
    let supervisor = fixture.supervise(FakeConnector::new([
        (
            "broken",
            FakeServer::Fails("no such file or directory".to_owned()),
        ),
        ("docs", advertising(&["search"])),
    ]));
    fixture.settled(&supervisor).await;

    let mut runtime = request(&fixture.fixture, HostSurface::Headless);
    runtime.provider = Some(Arc::new(FakeProvider::text_reply("still working")));
    runtime.approval = Some(Arc::new(agent_runtime_core::approval::AllowAll));
    runtime.mcp = Some(supervisor.clone());

    let smith = factory::build_request(runtime).await.expect("a runtime");
    assert!(
        smith
            .policy()
            .tools
            .contains(&"mcp__docs__search".to_owned()),
        "the working server still contributes"
    );

    // The failure is retained rather than logged and dropped: a server that
    // silently contributes nothing is worse than one that says why.
    let broken = supervisor.report("broken").expect("a reported server");
    assert_eq!(broken.state.label(), "failed");
    let smith_runtime::mcp::McpState::Failed { reason } = &broken.state else {
        panic!("expected a retained failure, got {:?}", broken.state);
    };
    assert!(reason.contains("no such file"), "{reason}");

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("hello"))
        .await
        .expect("a failing server does not fail the session");
    session.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn headless_run_does_not_prompt_or_spawn_an_untrusted_server() {
    // Deliberately no `trust_all`: nothing has been decided about this server.
    let fixture = McpFixture::new(MCP_ASK_CONFIG, "[mcp.servers.github]\ncommand = \"npx\"\n");
    let connector = FakeConnector::new([("github", advertising(&["search"]))]);
    let supervisor = fixture.supervise(connector);
    fixture.settled(&supervisor).await;

    let mut runtime = request(&fixture.fixture, HostSurface::Headless);
    runtime.provider = Some(Arc::new(FakeProvider::text_reply("ok")));
    runtime.approval = Some(Arc::new(agent_runtime_core::approval::AllowAll));
    runtime.mcp = Some(supervisor.clone());

    let smith = factory::build_request(runtime).await.expect("a runtime");
    assert!(
        supervisor.tools().is_empty(),
        "an untrusted server contributes no tools"
    );
    assert!(
        !smith
            .policy()
            .tools
            .iter()
            .any(|tool| tool.starts_with("mcp__")),
        "no remote tool reached the provider surface"
    );

    let report = supervisor.report("github").expect("a reported server");
    assert_eq!(report.state.label(), "untrusted");
    assert_eq!(
        report.state,
        smith_runtime::mcp::McpState::NeedsTrust(TrustStatus::Untrusted),
        "the run says trust is required rather than failing silently"
    );
}

#[tokio::test]
async fn remote_tool_call_requests_approval_and_attributes_its_server() {
    #[derive(Debug, Default)]
    struct RecordingApproval {
        seen: Mutex<
            Vec<(
                String,
                agent_runtime_core::security::PermissionSet,
                agent_runtime_core::security::SecurityResource,
            )>,
        >,
    }

    #[async_trait]
    impl agent_runtime_core::approval::ApprovalPolicy for RecordingApproval {
        async fn decide(
            &self,
            request: &agent_runtime_core::approval::ApprovalRequest,
        ) -> agent_runtime_core::approval::ApprovalDecision {
            self.seen.lock().expect("approval log").push((
                request.prepared().tool().to_owned(),
                request.prepared().required_permissions().clone(),
                request.prepared().resource().clone(),
            ));
            agent_runtime_core::approval::ApprovalDecision::Allow
        }
    }

    let mut fixture = McpFixture::new(
        MCP_ASK_CONFIG,
        "[mcp.servers.docs]\ncommand = \"docs-mcp\"\n",
    );
    fixture.trust_all();
    let supervisor = fixture.supervise(FakeConnector::new([("docs", advertising(&["search"]))]));
    fixture.settled(&supervisor).await;

    let approval = Arc::new(RecordingApproval::default());
    let mut runtime = request(&fixture.fixture, HostSurface::Headless);
    let mut call = tool_call_fragments(
        0,
        "remote-1",
        "mcp__docs__search",
        "{\"query\":\"boundaries\"}",
    );
    call.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    runtime.provider = Some(Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(call),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "done".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    )));
    runtime.approval = Some(approval.clone());
    runtime.mcp = Some(supervisor.clone());

    let smith = factory::build_request(runtime).await.expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text(
            "Use the docs server's search tool to look up boundaries.",
        ))
        .await
        .expect("the remote call runs");
    session.shutdown().await.expect("clean shutdown");

    let seen = approval.seen.lock().expect("approval log").clone();
    let [(tool, permissions, resource)] = seen.as_slice() else {
        panic!("expected one remote approval, got {seen:?}");
    };
    assert_eq!(tool, "mcp__docs__search");
    assert!(permissions.contains(&Permission::ExternalRead));
    assert!(permissions.contains(&Permission::ExternalWrite));
    assert!(permissions.contains(&Permission::NetHttp));
    assert!(permissions.contains(&Permission::DataEgress));
    assert!(!permissions.contains(&Permission::FsRead));
    assert!(matches!(
        resource,
        agent_runtime_core::security::SecurityResource::Other { kind, id }
            if kind == "external-service"
                && id.contains("mcp:docs@")
                && id.contains("stdio://mcp/docs/")
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn command_provider_receives_connected_mcp_tools_and_canonical_results() {
    use std::os::unix::fs::PermissionsExt;

    #[derive(Debug, Default)]
    struct RecordingApproval {
        tools: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl agent_runtime_core::approval::ApprovalPolicy for RecordingApproval {
        async fn decide(
            &self,
            request: &agent_runtime_core::approval::ApprovalRequest,
        ) -> agent_runtime_core::approval::ApprovalDecision {
            self.tools
                .lock()
                .expect("approval log")
                .push(request.prepared().tool().to_owned());
            agent_runtime_core::approval::ApprovalDecision::Allow
        }
    }

    let fixture = Fixture::new_private_user("");
    let executable = fixture.project.path().join("command-mcp-bridge.sh");
    let request_log = fixture.project.path().join("command-requests.jsonl");
    let attempt_count = fixture.project.path().join("command-attempt-count");
    let script = r#"#!/bin/sh
log=$1
count_file=$2
shift 2
if [ "$1" = "--smith-provider-probe" ]; then
  printf '%s\n' "{\"protocol\":\"smith-command-provider\",\"schema_version\":1,\"model\":\"$2\",\"implementation\":\"mcp-fixture\",\"implementation_version\":\"1.0.0\"}"
  exit 0
fi
if [ "$1" = "--smith-provider-attempt" ]; then
  IFS= read -r request
  printf '%s\n' "$request" >> "$log"
  count=0
  if [ -f "$count_file" ]; then
    count=$(cat "$count_file")
  fi
  count=$((count + 1))
  printf '%s' "$count" > "$count_file"
  if [ "$count" -eq 1 ]; then
    printf '%s\n' '{"protocol":"smith-command-provider","schema_version":1,"type":"error","kind":"rate_limited","message":"fixture retry","retryable":true}'
    exit 0
  fi
  case "$request" in
    *'"type":"tool_result"'*)
      printf '%s\n' '{"protocol":"smith-command-provider","schema_version":1,"type":"text_delta","text":"MCP result received."}'
      printf '%s\n' '{"protocol":"smith-command-provider","schema_version":1,"type":"usage","input_tokens":18,"output_tokens":4}'
      printf '%s\n' '{"protocol":"smith-command-provider","schema_version":1,"type":"finish","reason":"stop"}'
      ;;
    *)
      printf '%s\n' '{"protocol":"smith-command-provider","schema_version":1,"type":"tool_call_delta","index":0,"id":"remote-1","name":"mcp__docs__search","arguments_fragment":"{\"query\":\"boundaries\"}"}'
      printf '%s\n' '{"protocol":"smith-command-provider","schema_version":1,"type":"usage","input_tokens":12,"output_tokens":5}'
      printf '%s\n' '{"protocol":"smith-command-provider","schema_version":1,"type":"finish","reason":"tool_calls"}'
      ;;
  esac
  exit 0
fi
exit 2
"#;
    std::fs::write(&executable, script).expect("a command bridge fixture");
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
        .expect("an executable bridge fixture");

    let user_config = format!(
        r#"
default_profile = "local"

[profiles.local]
provider = "bridge"
model = "local-model"

[providers.bridge]
kind = "command-jsonl"

[providers.bridge.command]
executable = {executable:?}
args = [{request_log:?}, {attempt_count:?}]
cwd = "workspace"

[models."bridge/local-model"]
context_tokens = 32768
max_input_tokens = 28672
max_output_tokens = 4096

[limits]
max_retries = 1

[approval]
mode = "ask"
"#,
        executable = executable.display().to_string(),
        request_log = request_log.display().to_string(),
        attempt_count = attempt_count.display().to_string(),
    );
    std::fs::write(fixture.home.path().join(".smith/config.toml"), user_config)
        .expect("a private command-provider config");
    std::fs::write(
        fixture.project.path().join(".smith/config.toml"),
        "[mcp.servers.docs]\ncommand = \"docs-mcp\"\n",
    )
    .expect("a project MCP declaration");

    let state = tempfile::tempdir().expect("a state root");
    let trust = smith_config::trust::TrustStore::open(state.path()).expect("an empty trust store");
    let mut fixture = McpFixture {
        fixture,
        state,
        trust,
    };
    fixture.trust_all();
    let supervisor = fixture.supervise(FakeConnector::new([("docs", advertising(&["search"]))]));
    fixture.settled(&supervisor).await;

    let approval = Arc::new(RecordingApproval::default());
    let recorder = RecordingObserver::shared();
    let mut runtime = RuntimeRequest::new(fixture.fixture.config(), HostSurface::Headless);
    runtime.workspace = Some(Arc::new(
        ProjectWorkspace::new(fixture.fixture.project.path()).expect("a project workspace"),
    ));
    runtime.approval = Some(approval.clone());
    runtime.mcp = Some(supervisor);
    runtime.observers = vec![recorder.clone() as Arc<dyn EventObserver>];

    let smith = factory::build_request(runtime)
        .await
        .expect("a command-backed runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text(
            "Use the docs server's search tool to look up boundaries.",
        ))
        .await
        .expect("the command bridge completes an MCP tool loop");
    session.shutdown().await.expect("clean shutdown");

    assert_eq!(
        approval.tools.lock().expect("approval log").as_slice(),
        ["mcp__docs__search"]
    );
    let requests = std::fs::read_to_string(request_log).expect("command request log");
    let requests = requests.lines().collect::<Vec<_>>();
    assert_eq!(
        requests.len(),
        3,
        "each visible attempt uses a fresh process"
    );
    assert!(
        requests[0].contains("mcp__docs__search") && requests[0].contains("input_schema"),
        "the connected MCP schema did not reach the first command request"
    );
    assert!(
        requests[1].contains("mcp__docs__search") && !requests[1].contains("tool_result"),
        "the retry did not replay the same Smith-owned turn in a fresh process"
    );
    assert!(
        requests[2].contains("tool_result") && requests[2].contains("docs answered search"),
        "Smith's canonical MCP result did not reach the next command request"
    );

    let events = recorder.payloads();
    let started = events
        .iter()
        .filter(|event| matches!(event, RuntimeEvent::ProviderAttemptStarted { .. }))
        .count();
    let discarded = events
        .iter()
        .filter(|event| matches!(event, RuntimeEvent::ProviderAttemptOutputDiscarded { .. }))
        .count();
    assert_eq!(
        started, 3,
        "every fresh bridge process must be a visible attempt"
    );
    assert_eq!(
        discarded, 1,
        "the retryable attempt must be visibly discarded"
    );
}

#[tokio::test]
async fn the_startup_grace_is_bounded_and_leaves_a_slow_server_connecting() {
    let mut fixture = McpFixture::new(
        MCP_ASK_CONFIG,
        "[mcp.servers.slow]\ncommand = \"slow-mcp\"\n",
    );
    fixture.trust_all();
    let supervisor = fixture.supervise(FakeConnector::new([("slow", FakeServer::Hangs)]));

    // A host may pay a short grace so the common case — a local server that
    // answers in milliseconds — has its tools on turn one. What it may never do
    // is hand a third party the power to hold the prompt.
    let started = std::time::Instant::now();
    supervisor
        .settle(std::time::Duration::from_millis(150))
        .await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "the grace outlasted its budget: {:?}",
        started.elapsed()
    );
    assert!(
        !supervisor.settled(),
        "the server is still connecting, and the session goes on without it"
    );
}

#[tokio::test]
async fn a_remote_server_this_build_can_reach_fails_loudly_rather_than_silently() {
    // A closed loopback port: the dial is real, the failure is real, and no
    // packet leaves the machine. What matters is that the reason survives to a
    // surface instead of being logged and dropped.
    let mut fixture = McpFixture::new(
        MCP_ASK_CONFIG,
        "[mcp.servers.remote]\nurl = \"http://127.0.0.1:1/mcp\"\n\
         credential = \"env:SMITH_TEST_MCP_TOKEN\"\n\n\
         [mcp.servers.open]\nurl = \"http://127.0.0.1:1/mcp\"\n",
    );
    fixture.trust_all();

    let supervisor = McpSupervisor::plan(
        &fixture.fixture.config().mcp,
        &fixture.trust,
        smith_runtime::mcp::McpOptions::new(fixture.fixture.project.path()),
    )
    .expect("a planned supervisor");
    supervisor.connect();
    fixture.settled(&supervisor).await;

    let report = supervisor.report("remote").expect("a reported server");
    let smith_runtime::mcp::McpState::Failed { reason } = &report.state else {
        panic!("expected a retained failure, got {:?}", report.state);
    };
    // No resolver was supplied, so the credential is what fails first — named,
    // never valued.
    assert!(reason.contains("Authorization"), "{reason}");
    assert!(!reason.contains("SMITH_TEST_MCP_TOKEN="), "{reason}");
    assert_eq!(report.transport, "http");

    // The one that needs no credential got as far as the socket, which is what
    // proves the transport is compiled in rather than reported as missing.
    let open = supervisor.report("open").expect("a reported server");
    let smith_runtime::mcp::McpState::Failed { reason } = &open.state else {
        panic!("expected a retained failure, got {:?}", open.state);
    };
    assert!(
        !reason.contains("feature"),
        "the `http` transport must be compiled in, not reported as unavailable: {reason}"
    );
}
