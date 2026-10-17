//! Committed components keep these tests independent of installed guest targets.
use agent_runtime::harness::{ContextView, ToolOutputView};
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock, Timestamp};
use agent_runtime_core::content::{Message, ToolCall};
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId, TurnId};
use agent_runtime_core::tool::{InvocationContext, PreparedToolCall, Tool, ToolOutcome};
use agent_runtime_registry::Fingerprint;
use agent_runtime_testkit::transport::ReplayTransport;
use agent_runtime_testkit::workspace::MemoryWorkspace;
use smith_module::{
    CompiledModule, Module, ModuleCapability as C, ModuleComposition, ModuleContext,
    ModuleContribution, ModulePosture, ModuleState, Mounted, PipelineComponent,
};
use smith_wasm_host::{
    CapabilityGrant, WasmEngine, WasmError, WasmLimits, WasmModule, WasmModuleConfig,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

fn fixture(name: &str) -> Vec<u8> {
    let name = if let Some(legacy) = name.strip_prefix("v0.1/") {
        legacy.to_owned()
    } else if name == "unsupported" || name.starts_with("wasi") {
        name.to_owned()
    } else {
        format!("v0.2/{name}")
    };
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(format!("{name}.wasm")),
    )
    .unwrap()
}
fn config(root: &std::path::Path) -> WasmModuleConfig {
    WasmModuleConfig {
        plugin_name: "test".into(),
        module_id: "test/module".into(),
        requested_capabilities: vec![],
        granted_capabilities: vec![],
        host_state_dir: root.join("host"),
        data_dir: root.join("data"),
        cache_dir: root
            .join("host")
            .join(smith_wasm_host::COMPONENT_CACHE_DIRECTORY),
        limits: WasmLimits {
            tool_deadline: Duration::from_millis(30),
            ..WasmLimits::default()
        },
        session_health: Arc::new(smith_module::ModuleHealth::default()),
        reserved_tool_names: BTreeSet::new(),
    }
}
fn context(root: &std::path::Path, transport: Arc<ReplayTransport>) -> ModuleContext {
    ModuleContext {
        is_child_session: false,
        web: None,
        http: Some(Arc::new(ReplayHttp(transport.clone()))),
        settings: BTreeMap::new(),
        workspace_root: Some(root.canonicalize().unwrap()),
        user_dir: root.join("host"),
        posture: ModulePosture::ReadWrite,
        transport,
        vision: None,
        main_model_accepts_images: false,
        model: None,
        image_binding: None,
        session_history: None,
        semantic_summary_enabled: false,
        max_input_tokens: 32000,
        built_in_tools: true,
    }
}
#[derive(Debug)]
struct ReplayHttp(Arc<ReplayTransport>);
impl smith_module::ModuleHttp for ReplayHttp {
    fn request(
        &self,
        request: smith_module::ModuleHttpRequest,
    ) -> smith_module::ModuleHttpFuture<'_> {
        Box::pin(async move {
            use agent_runtime::provider::transport::HttpTransport;
            use futures_util::StreamExt;
            let response = self
                .0
                .post_response(agent_runtime::provider::transport::HttpRequest {
                    url: request.url,
                    headers: request.headers,
                    body: request.body,
                })
                .await?;
            let mut body = Vec::new();
            let mut stream = response.body;
            while let Some(chunk) = stream.next().await {
                body.extend(chunk?);
            }
            if body.len() > request.max_response_bytes {
                return Err(agent_runtime_core::provider::ProviderError::new(
                    agent_runtime_core::provider::ProviderErrorKind::MalformedStream,
                    "oversized fixture HTTP body",
                ));
            }
            Ok(smith_module::ModuleHttpResponse {
                status: response.status,
                headers: response.headers,
                body,
            })
        })
    }
}
fn mount(
    name: &str,
    config: WasmModuleConfig,
    context: &ModuleContext,
) -> (
    Arc<WasmModule>,
    Vec<ModuleContribution>,
    Arc<smith_module::ModuleHealth>,
) {
    let module = WasmModule::load(WasmEngine::new().unwrap(), fixture(name), config).unwrap();
    let Mounted::TrackedContributions {
        contributions,
        health,
    } = module.mount(context).unwrap()
    else {
        panic!("expected active fixture");
    };
    (module, contributions, health)
}
fn tool(contributions: &[ModuleContribution]) -> Arc<dyn Tool> {
    contributions
        .iter()
        .find_map(|c| match c {
            ModuleContribution::Tool(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap()
}
async fn invoke(
    tool: &dyn Tool,
    action: &str,
) -> Result<ToolOutcome, agent_runtime_core::error::RuntimeError> {
    invoke_in_session(tool, action, "session").await
}
async fn invoke_in_session(
    tool: &dyn Tool,
    action: &str,
    session: &str,
) -> Result<ToolOutcome, agent_runtime_core::error::RuntimeError> {
    invoke_at(tool, action, session, "turn").await
}
async fn invoke_at(
    tool: &dyn Tool,
    action: &str,
    session: &str,
    turn: &str,
) -> Result<ToolOutcome, agent_runtime_core::error::RuntimeError> {
    let call_id = ToolCallId::new("call");
    let spec = tool.spec();
    let prepared = PreparedToolCall::from_static_effects(
        call_id.clone(),
        &spec,
        serde_json::json!({"action": action}),
        "/workspace",
    );
    tool.invoke(
        prepared,
        &InvocationContext {
            session: SessionId::new(session),
            turn: Some(TurnId::new(turn)),
            call_id,
            request: RequestId::new("request"),
            workspace: Arc::new(MemoryWorkspace::new("/workspace")),
            clock: Arc::new(SystemClock),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
            output_limit: 10000,
        },
    )
    .await
}
#[test]
fn unsupported_version_names_found_and_supported_and_wasi_is_absent() {
    let root = tempfile::tempdir().unwrap();
    let engine = WasmEngine::new().unwrap();
    let error =
        WasmModule::load(engine.clone(), fixture("unsupported"), config(root.path())).unwrap_err();
    assert!(matches!(error, WasmError::InterfaceVersion { .. }));
    assert!(error.to_string().contains("9.0.0"));
    assert!(error.to_string().contains("smith:module@0.1.0"));
    assert!(error.to_string().contains("smith:module@0.2.0"));
    for (fixture_name, import) in [
        ("wasi", "wasi:filesystem/types"),
        ("wasi-network", "wasi:sockets/tcp"),
        ("wasi-environment", "wasi:cli/environment"),
        ("wasi-clock", "wasi:clocks/wall-clock"),
    ] {
        let error = WasmModule::load(engine.clone(), fixture(fixture_name), config(root.path()))
            .unwrap_err();
        assert!(error.to_string().contains(import));
    }
}
#[tokio::test]
async fn two_tools_status_and_conservative_authority() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let (_, contributions, _) = mount("tools", config(root.path()), &ctx);
    assert_eq!(
        contributions
            .iter()
            .filter(|c| matches!(c, ModuleContribution::Tool(_)))
            .count(),
        2
    );
    let tool = tool(&contributions);
    assert!(tool.spec().effects.mutates());
    invoke(tool.as_ref(), "ok").await.unwrap();
    let source = contributions
        .iter()
        .find_map(|c| match c {
            ModuleContribution::StatusItem { source, .. } => Some(source),
            _ => None,
        })
        .unwrap();
    assert_eq!(source.current().unwrap().label(), "calls: 1");
}
#[tokio::test]
async fn trap_timeout_memory_oversize_and_malformed_are_isolated() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    for (action, expected) in [
        ("trap", "trapped"),
        ("loop", "deadline"),
        ("memory", "memory/resource"),
        ("oversized", "payload"),
        ("malformed", "malformed"),
    ] {
        let mut cfg = config(root.path());
        if action == "memory" {
            cfg.limits.tool_deadline = Duration::from_secs(5);
        }
        let (_, contributions, health) = mount("tools", cfg, &ctx);
        let result = invoke(tool(&contributions).as_ref(), action)
            .await
            .unwrap_err();
        assert!(result.to_string().contains(expected), "{action}: {result}");
        assert!(health.failure().is_none());
        // An independent module instance remains callable in the same session.
        let (_, healthy, _) = mount("tools", config(root.path()), &ctx);
        invoke(tool(&healthy).as_ref(), "ok").await.unwrap();
    }
}
#[tokio::test]
async fn repeated_failures_disable_only_the_mounted_session_and_update_report() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let module = WasmModule::load(
        WasmEngine::new().unwrap(),
        fixture("tools"),
        config(root.path()),
    )
    .unwrap();
    let composition = ModuleComposition::with_defaults(
        vec![CompiledModule {
            module: module.clone(),
            origin: module.origin(),
        }],
        vec![],
    );
    let plan = smith_module::mount_modules(&composition, &ctx);
    let t = tool(&plan.mounted[0].contributions);
    for _ in 0..3 {
        assert!(invoke(t.as_ref(), "trap").await.is_err());
    }
    assert!(matches!(
        plan.report[0].current_state(),
        ModuleState::Failed { .. }
    ));
    assert!(
        invoke(t.as_ref(), "ok")
            .await
            .unwrap_err()
            .to_string()
            .contains("module disabled")
    );
    let rebuilt = smith_module::mount_modules(&composition, &ctx);
    assert!(rebuilt.mounted.is_empty());
    assert!(matches!(
        rebuilt.report[0].state,
        ModuleState::Failed { .. }
    ));
    let (_, fresh_session, _) = mount("tools", config(root.path()), &ctx);
    invoke(tool(&fresh_session).as_ref(), "ok").await.unwrap();
}
#[tokio::test]
async fn success_resets_consecutive_failure_count() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let (_, contributions, health) = mount("tools", config(root.path()), &ctx);
    let t = tool(&contributions);
    for _ in 0..3 {
        assert!(invoke(t.as_ref(), "malformed").await.is_err());
        invoke(t.as_ref(), "ok").await.unwrap();
    }
    assert!(health.failure().is_none());
}
#[test]
fn inactive_mount_errors_duplicate_tools_and_builtin_collisions() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    for name in ["inactive", "mount-error", "malformed", "collision"] {
        let module = WasmModule::load(
            WasmEngine::new().unwrap(),
            fixture(name),
            config(root.path()),
        )
        .unwrap();
        let mounted = module.mount(&ctx);
        if name == "inactive" {
            assert!(matches!(mounted, Ok(Mounted::Inactive { .. })));
        } else {
            assert!(mounted.is_err(), "{name}");
        }
    }
}
fn grant(capability: C, roots: &[&str], hosts: &[&str]) -> CapabilityGrant {
    CapabilityGrant {
        capability,
        roots: roots.iter().map(|s| (*s).into()).collect(),
        hosts: hosts.iter().map(|s| (*s).into()).collect(),
        allow_http: false,
    }
}
#[tokio::test]
async fn every_ungranted_host_function_fails_even_when_guest_handles_error() {
    let root = tempfile::tempdir().unwrap();
    let transport = Arc::new(ReplayTransport::single(b"response".to_vec()));
    let ctx = context(root.path(), transport.clone());
    for action in ["http", "fs-read", "fs-write", "kv-get", "kv-set", "history"] {
        let (_, contributions, _) = mount("tools", config(root.path()), &ctx);
        assert!(
            invoke(tool(&contributions).as_ref(), action)
                .await
                .unwrap_err()
                .to_string()
                .contains("capability denied"),
            "{action}"
        );
    }
    assert!(transport.requests().is_empty());
}
#[tokio::test]
async fn grants_allow_only_exact_http_hosts_and_confined_files_and_kv() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("fixture.txt"), "original").unwrap();
    let transport = Arc::new(ReplayTransport::single(b"response".to_vec()));
    let ctx = context(root.path(), transport.clone());
    let mut cfg = config(root.path());
    cfg.granted_capabilities = vec![
        grant(C::Network, &[], &["api.example.com"]),
        grant(C::DataEgress, &[], &["api.example.com"]),
        grant(C::WorkspaceRead, &["workspace"], &[]),
        grant(C::WorkspaceWrite, &["workspace"], &[]),
        grant(C::HostRead, &["plugin-data"], &[]),
        grant(C::HostWrite, &["plugin-data"], &[]),
    ];
    cfg.requested_capabilities = cfg.granted_capabilities.clone();
    let (_, contributions, _) = mount("tools", cfg, &ctx);
    let t = tool(&contributions);
    assert!(invoke(t.as_ref(), "http-other").await.is_err());
    assert!(transport.requests().is_empty());
    for action in ["http", "fs-read", "fs-write", "kv-set", "kv-get"] {
        let output = invoke(t.as_ref(), action).await.unwrap();
        assert!(
            format!("{:?}", output.content).contains("Ok"),
            "{action}: {output:?}"
        );
    }
    assert_eq!(transport.requests().len(), 1);
    assert_eq!(
        std::fs::read(root.path().join("fixture.txt")).unwrap(),
        b"written"
    );
}
#[tokio::test]
async fn context_only_and_malformed_context_contributes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let mut cfg = config(root.path());
    cfg.granted_capabilities = vec![grant(C::HostRead, &["session-history"], &[])];
    cfg.requested_capabilities = cfg.granted_capabilities.clone();
    let (_, contributions, _) = mount("context", cfg, &ctx);
    assert_eq!(contributions.len(), 1);
    let ModuleContribution::Pipeline(PipelineComponent::ContextContributor(component)) =
        &contributions[0]
    else {
        panic!("context contributor");
    };
    let mut view = ContextView {
        session: SessionId::new("session"),
        turn: TurnId::new("turn"),
        history: Arc::from([]),
        activation: Fingerprint::of("activation"),
        state: None,
    };
    assert_eq!(
        component.contribute(&view).await.unwrap().fragments.len(),
        1
    );
    view.history = Arc::from([Message::user("malformed")]);
    assert!(
        component
            .contribute(&view)
            .await
            .unwrap()
            .fragments
            .is_empty()
    );
}
fn contributor(
    contributions: &[ModuleContribution],
) -> &Arc<dyn agent_runtime::harness::ContextContributor> {
    contributions
        .iter()
        .find_map(|contribution| match contribution {
            ModuleContribution::Pipeline(PipelineComponent::ContextContributor(component)) => {
                Some(component)
            }
            _ => None,
        })
        .unwrap()
}

#[tokio::test]
async fn contributor_without_history_receives_empty_history_and_keeps_tools() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let (_, contributions, health) = mount("context-tools", config(root.path()), &ctx);
    let view = ContextView {
        session: SessionId::new("session"),
        turn: TurnId::new("turn"),
        // This causes a malformed patch if it leaks into the guest.
        history: Arc::from([Message::user("malformed")]),
        activation: Fingerprint::of("activation"),
        state: None,
    };
    for _ in 0..5 {
        let patch = contributor(&contributions).contribute(&view).await.unwrap();
        assert_eq!(patch.fragments.len(), 1);
        assert!(format!("{:?}", patch.fragments[0].content).contains("history: 0"));
    }
    assert!(health.failure().is_none());
    assert!(
        !invoke(tool(&contributions).as_ref(), "ok")
            .await
            .unwrap()
            .is_error
    );
}

#[tokio::test]
async fn oversized_history_skips_contribution_without_resetting_or_incrementing_failures() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let mut cfg = config(root.path());
    cfg.granted_capabilities = vec![grant(C::HostRead, &["session-history"], &[])];
    cfg.requested_capabilities = cfg.granted_capabilities.clone();
    let payload_bytes = cfg.limits.payload_bytes;
    let (_, contributions, health) = mount("context-tools", cfg, &ctx);
    let mut view = ContextView {
        session: SessionId::new("session"),
        turn: TurnId::new("turn"),
        history: Arc::from([Message::user("x".repeat(payload_bytes + 1))]),
        activation: Fingerprint::of("activation"),
        state: None,
    };
    assert!(!health.record_failure("previous failure".into()));
    for _ in 0..5 {
        assert!(
            contributor(&contributions)
                .contribute(&view)
                .await
                .unwrap()
                .fragments
                .is_empty()
        );
    }
    assert!(health.failure().is_none());
    // A skip also does not masquerade as a successful guest call.
    assert!(!health.record_failure("second failure".into()));
    assert!(
        !invoke(tool(&contributions).as_ref(), "ok")
            .await
            .unwrap()
            .is_error
    );
    view.history = Arc::from([Message::user("representable")]);
    let patch = contributor(&contributions).contribute(&view).await.unwrap();
    assert_eq!(patch.fragments.len(), 1);
    assert!(format!("{:?}", patch.fragments[0].content).contains("history: 1"));
    assert!(!health.record_failure("first failure".into()));
    view.history = Arc::from([Message::user("x".repeat(payload_bytes + 1))]);
    assert!(
        contributor(&contributions)
            .contribute(&view)
            .await
            .unwrap()
            .fragments
            .is_empty()
    );
    assert!(!health.record_failure("second failure".into()));
    assert!(health.record_failure("third failure".into()));
}

#[tokio::test]
async fn context_contributor_runs_without_history_and_survives_an_unfittable_view() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let contributor = |cfg| {
        let (_, mut contributions, _) = mount("context", cfg, &ctx);
        let ModuleContribution::Pipeline(PipelineComponent::ContextContributor(component)) =
            contributions.remove(0)
        else {
            panic!("context contributor");
        };
        component
    };
    let mut view = ContextView {
        session: SessionId::new("session"),
        turn: TurnId::new("turn"),
        history: Arc::from([Message::user("malformed")]),
        activation: Fingerprint::of("activation"),
        state: None,
    };
    // Ungranted: the guest runs but never sees the text that would make this
    // fixture return a protected fragment.
    let ungranted = contributor(config(root.path()));
    for _ in 0..4 {
        let patch = ungranted.contribute(&view).await.unwrap();
        assert_eq!(patch.fragments.len(), 1);
    }

    let mut cfg = config(root.path());
    cfg.granted_capabilities = vec![grant(C::HostRead, &["session-history"], &[])];
    cfg.requested_capabilities = cfg.granted_capabilities.clone();
    let granted = contributor(cfg);
    let long: Vec<_> = (0..257).map(|_| Message::user("turn")).collect();
    view.history = Arc::from(long);
    for _ in 0..4 {
        let patch = granted.contribute(&view).await.unwrap();
        assert!(patch.fragments.is_empty());
    }
    view.history = Arc::from([Message::user("short")]);
    assert_eq!(granted.contribute(&view).await.unwrap().fragments.len(), 1);
}

#[tokio::test]
async fn output_processor_replaces_outcome_and_failure_passes_original() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let (_, contributions, _) = mount("processor", config(root.path()), &ctx);
    let ModuleContribution::Pipeline(PipelineComponent::ToolOutputProcessor(component)) =
        &contributions[0]
    else {
        panic!("output processor");
    };
    let mut view = ToolOutputView {
        session: SessionId::new("session"),
        turn: TurnId::new("turn"),
        request: RequestId::new("request"),
        call: ToolCall {
            id: ToolCallId::new("call"),
            name: "read".into(),
            arguments: serde_json::json!({}),
        },
        state: None,
        usage: Arc::from([]),
        now: Timestamp(100),
    };
    let original = ToolOutcome::text("original");
    assert_eq!(
        component
            .process(&view, original.clone())
            .await
            .unwrap()
            .outcome
            .content
            .as_inline()
            .unwrap()[0]
            .as_text(),
        Some("processed")
    );
    view.call.arguments = serde_json::json!({"action":"malformed"});
    let patch = component.process(&view, original.clone()).await.unwrap();
    assert_eq!(patch.outcome, original);
    assert!(patch.state.is_none() && patch.events.is_empty());
}
#[test]
fn cache_reuses_and_recovers_from_corruption() {
    let root = tempfile::tempdir().unwrap();
    let engine = WasmEngine::new().unwrap();
    let bytes = fixture("tools");
    let cfg = config(root.path());
    let cache = cfg.cache_dir.clone();
    WasmModule::load(engine.clone(), &bytes, cfg.clone()).unwrap();
    let path = std::fs::read_dir(&cache)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let before = std::fs::read(&path).unwrap();
    WasmModule::load(engine.clone(), &bytes, cfg.clone()).unwrap();
    assert_eq!(before, std::fs::read(&path).unwrap());
    std::fs::write(&path, b"corrupt").unwrap();
    WasmModule::load(engine, &bytes, cfg).unwrap();
    assert_eq!(before, std::fs::read(&path).unwrap());
}

#[tokio::test]
async fn payload_bounds_apply_to_input_and_host_responses() {
    let root = tempfile::tempdir().unwrap();
    let transport = Arc::new(ReplayTransport::single(vec![0; 1024 * 1024 + 1]));
    let ctx = context(root.path(), transport);
    let mut cfg = config(root.path());
    cfg.granted_capabilities = vec![
        grant(C::Network, &[], &["api.example.com"]),
        grant(C::DataEgress, &[], &["api.example.com"]),
    ];
    cfg.requested_capabilities = cfg.granted_capabilities.clone();
    let (_, contributions, _) = mount("tools", cfg, &ctx);
    let t = tool(&contributions);
    assert!(
        invoke(t.as_ref(), &"x".repeat(1024 * 1024 + 1))
            .await
            .unwrap_err()
            .to_string()
            .contains("payload")
    );
    assert!(
        invoke(t.as_ref(), "http")
            .await
            .unwrap_err()
            .to_string()
            .contains("payload")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn filesystem_grants_refuse_symlink_escape_on_reads_and_writes() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().canonicalize().unwrap().join("secret.txt");
    std::fs::write(&target, "outside secret").unwrap();
    std::os::unix::fs::symlink(&target, root.path().join("fixture.txt")).unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let mut cfg = config(root.path());
    cfg.granted_capabilities = vec![
        grant(C::WorkspaceRead, &["workspace"], &[]),
        grant(C::WorkspaceWrite, &["workspace"], &[]),
    ];
    cfg.requested_capabilities = cfg.granted_capabilities.clone();
    let (_, contributions, _) = mount("tools", cfg, &ctx);
    for action in ["fs-read", "fs-write"] {
        let output = invoke(tool(&contributions).as_ref(), action).await.unwrap();
        let rendered = format!("{:?}", output.content);
        assert!(rendered.contains("symlink refused") && !rendered.contains("outside secret"));
    }
    assert_eq!(std::fs::read_to_string(target).unwrap(), "outside secret");
}

#[derive(Debug)]
struct History;
impl smith_module::SessionHistory for History {
    fn with_history(
        &self,
        session: &SessionId,
        visitor: &mut dyn FnMut(&[Message]),
    ) -> Result<(), agent_runtime_core::error::RuntimeError> {
        assert_eq!(session.as_str(), "session");
        visitor(&[
            Message::user("canonical history"),
            Message::assistant(vec![agent_runtime_core::content::ContentPart::Reasoning {
                text: "hidden original reasoning".into(),
                redacted: true,
                signature: Some("hidden signature".into()),
                producer: None,
            }]),
        ]);
        Ok(())
    }
}
#[tokio::test]
async fn history_is_current_session_canonical_and_redacts_hidden_reasoning() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    ctx.session_history = Some(Arc::new(History));
    let mut cfg = config(root.path());
    cfg.granted_capabilities = vec![grant(C::HostRead, &["session-history"], &[])];
    cfg.requested_capabilities = cfg.granted_capabilities.clone();
    let (_, contributions, _) = mount("tools", cfg, &ctx);
    let output = invoke(tool(&contributions).as_ref(), "history")
        .await
        .unwrap();
    let rendered = format!("{:?}", output.content);
    assert!(rendered.contains("canonical history"));
    assert!(!rendered.contains("hidden original") && !rendered.contains("hidden signature"));
}

#[derive(Debug, Default)]
struct PendingTransport(std::sync::atomic::AtomicUsize);
impl smith_module::ModuleHttp for PendingTransport {
    fn request(&self, _: smith_module::ModuleHttpRequest) -> smith_module::ModuleHttpFuture<'_> {
        Box::pin(async move {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            std::future::pending().await
        })
    }
}
#[tokio::test(flavor = "current_thread")]
async fn mount_http_deadline_does_not_require_the_callers_reactor_to_progress() {
    let root = tempfile::tempdir().unwrap();
    let transport = Arc::new(PendingTransport::default());
    let mut ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    ctx.http = Some(transport.clone());
    let mut cfg = config(root.path());
    cfg.limits.component_deadline = Duration::from_millis(30);
    cfg.granted_capabilities = vec![
        grant(C::Network, &[], &["api.example.com"]),
        grant(C::DataEgress, &[], &["api.example.com"]),
    ];
    cfg.requested_capabilities = cfg.granted_capabilities.clone();
    let module = WasmModule::load(WasmEngine::new().unwrap(), fixture("mount-http"), cfg).unwrap();
    let started = std::time::Instant::now();
    assert!(
        module
            .mount(&ctx)
            .unwrap_err()
            .to_string()
            .contains("deadline")
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    tokio::task::yield_now().await;
    assert_eq!(
        transport.0.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "the queued request must be cancelled before it can run"
    );
}

#[test]
fn sdk_guests_without_host_service_calls_keep_a_versioned_interface() {
    let root = tempfile::tempdir().unwrap();
    let module = WasmModule::load(
        WasmEngine::new().unwrap(),
        fixture("quiet"),
        config(root.path()),
    )
    .unwrap();
    assert_eq!(module.id(), "guest");
}

#[tokio::test]
async fn workspace_grants_cannot_modify_native_component_caches() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let mut cfg = config(root.path());
    cfg.granted_capabilities = vec![grant(C::WorkspaceWrite, &["workspace"], &[])];
    cfg.requested_capabilities = cfg.granted_capabilities.clone();
    let (_, contributions, _) = mount("tools", cfg, &ctx);
    assert!(
        invoke(tool(&contributions).as_ref(), "cache-write")
            .await
            .unwrap_err()
            .to_string()
            .contains("capability denied")
    );
    assert!(!root.path().join("host/plugins/wasm-cache/poison").exists());
}

#[test]
fn plugin_data_alias_cannot_cover_the_component_cache() {
    let root = tempfile::tempdir().unwrap();
    let mut cfg = config(root.path());
    cfg.data_dir = cfg.host_state_dir.clone();
    let error = WasmModule::load(WasmEngine::new().unwrap(), fixture("tools"), cfg).unwrap_err();
    assert!(error.to_string().contains("must not overlap"));
}

#[tokio::test]
async fn child_health_is_independent_and_retained_by_actual_session_identity() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let cfg = config(root.path());
    let parent_health = cfg.session_health.clone();
    let module =
        WasmModule::load(WasmEngine::new().unwrap(), fixture("tools"), cfg.clone()).unwrap();
    ctx.is_child_session = true;
    let Mounted::TrackedContributions { contributions, .. } = module.mount(&ctx).unwrap() else {
        panic!("tracked mount")
    };
    let guest = tool(&contributions);
    for _ in 0..3 {
        assert!(
            invoke_in_session(guest.as_ref(), "malformed", "child-one")
                .await
                .is_err()
        );
    }
    assert!(parent_health.failure().is_none());
    assert!(
        parent_health
            .child_session(&SessionId::new("child-one"))
            .failure()
            .is_some()
    );
    let Mounted::TrackedContributions { contributions, .. } = module.mount(&ctx).unwrap() else {
        panic!("tracked mount")
    };
    let guest = tool(&contributions);
    assert!(
        invoke_in_session(guest.as_ref(), "ok", "child-one")
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
    let Mounted::TrackedContributions { contributions, .. } = module.mount(&ctx).unwrap() else {
        panic!("tracked mount")
    };
    assert!(
        invoke_in_session(tool(&contributions).as_ref(), "ok", "child-two")
            .await
            .is_ok()
    );
    parent_health.fail("parent disabled".into());
    let rebuilt = WasmModule::load(WasmEngine::new().unwrap(), fixture("tools"), cfg).unwrap();
    let Mounted::TrackedContributions { contributions, .. } = rebuilt.mount(&ctx).unwrap() else {
        panic!("child mount")
    };
    assert!(
        invoke_in_session(tool(&contributions).as_ref(), "ok", "child-three")
            .await
            .is_ok()
    );
    ctx.is_child_session = false;
    assert!(
        rebuilt.mount(&ctx).is_err(),
        "a parent rebuild must keep the parent disabled"
    );
}

#[tokio::test]
async fn frozen_v01_tools_and_status_still_work() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let (_, contributions, health) = mount("v0.1/tools", config(root.path()), &ctx);
    assert_eq!(contributions.len(), 3);
    let output = invoke(tool(&contributions).as_ref(), "ok").await.unwrap();
    assert_eq!(output.value, serde_json::Value::Null);
    assert!(format!("{:?}", output.content).contains("ok"));
    assert!(!output.is_error);
    let source = contributions
        .iter()
        .find_map(|c| match c {
            ModuleContribution::StatusItem { source, .. } => Some(source),
            _ => None,
        })
        .unwrap();
    assert_eq!(source.current().unwrap().label(), "calls: 1");
    assert!(health.failure().is_none());
}

#[tokio::test]
async fn completion_refusals_are_typed_and_do_not_disable_the_module() {
    let root = tempfile::tempdir().unwrap();
    for granted in [false, true] {
        let transport = Arc::new(ReplayTransport::single(Vec::new()));
        let ctx = context(root.path(), transport.clone());
        let mut config = config(root.path());
        if granted {
            let grant = CapabilityGrant {
                capability: C::Model,
                roots: vec![],
                hosts: vec![],
                allow_http: false,
            };
            config.requested_capabilities.push(grant.clone());
            config.granted_capabilities.push(grant);
        }
        let (_, contributions, health) = mount("complete", config, &ctx);
        let tool = tool(&contributions);
        for _ in 0..4 {
            let output = invoke(tool.as_ref(), "complete").await.unwrap();
            let text = serde_json::to_string(&output).unwrap();
            if granted {
                assert!(
                    text.contains("Unavailable") && text.contains("no model service is attached")
                );
            } else {
                // wit-bindgen's Debug output qualifies both enum names.
                assert!(
                    text.contains("Err(HostError::CapabilityDenied(Capability::Model))"),
                    "{text}"
                );
            }
        }
        assert!(health.failure().is_none());
        assert!(transport.requests().is_empty());
    }
}

fn watcher(contributions: &[ModuleContribution]) -> Arc<dyn smith_module::ModuleWatcher> {
    contributions
        .iter()
        .find_map(|c| match c {
            ModuleContribution::Watcher { source, .. } => Some(source.clone()),
            _ => None,
        })
        .unwrap()
}

fn notify_config(root: &std::path::Path) -> WasmModuleConfig {
    let mut config = config(root);
    let grant = CapabilityGrant {
        capability: C::SessionNotify,
        roots: vec![],
        hosts: vec![],
        allow_http: false,
    };
    config.requested_capabilities.push(grant.clone());
    config.granted_capabilities.push(grant);
    config
}

#[test]
fn watcher_without_notify_is_refused_without_losing_other_contributions() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    for requested in [false, true] {
        let mut config = notify_config(root.path());
        config.granted_capabilities.clear();
        if !requested {
            config.requested_capabilities.clear();
        }
        let (_, contributions, health) = mount("complete", config, &ctx);
        assert!(
            !contributions
                .iter()
                .any(|c| matches!(c, ModuleContribution::Watcher { .. }))
        );
        assert!(
            contributions
                .iter()
                .any(|c| matches!(c, ModuleContribution::Tool(_)))
        );
        assert!(
            contributions
                .iter()
                .any(|c| matches!(c, ModuleContribution::StatusItem { .. }))
        );
        assert!(health.failure().is_none());
        assert!(health.contribution_diagnostics().iter().any(|reason| {
            reason.contains("watcher refused") && reason.contains("session_notify")
        }));
    }
}

#[tokio::test]
async fn watcher_polls_notices_without_a_scheduler() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let (_, contributions, health) = mount("watcher", notify_config(root.path()), &ctx);
    let notices = watcher(&contributions)
        .poll(&SessionId::new("session"))
        .await
        .unwrap();
    assert_eq!(
        notices,
        vec![smith_module::ModuleNotice {
            text: "fixture notice".into()
        }]
    );
    assert!(health.failure().is_none());
}

#[tokio::test]
async fn invalid_poll_results_and_deadlines_share_invocation_health() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    for (fixture, expected) in [
        ("poll-oversize", "payload too large"),
        ("poll-many", "payload too large"),
        ("poll-unknown", "UnknownId"),
        ("poll-loop", "deadline exceeded"),
    ] {
        let mut config = notify_config(root.path());
        config.limits.component_deadline = Duration::from_millis(100);
        let (_, contributions, health) = mount(fixture, config, &ctx);
        let watcher = watcher(&contributions);
        for attempt in 0..3 {
            let error = watcher.poll(&SessionId::new("session")).await.unwrap_err();
            // An epoch interrupt poisons the component instance. The first
            // poll must report the deadline, as tool calls do; later polls
            // trap with CannotEnterComponent before entering guest code.
            let expected = if fixture == "poll-loop" && attempt > 0 {
                "module trapped"
            } else {
                expected
            };
            assert!(error.to_string().contains(expected), "{fixture}: {error}");
        }
        assert!(health.failure().is_some(), "{fixture}");
        assert!(
            watcher
                .poll(&SessionId::new("session"))
                .await
                .unwrap_err()
                .to_string()
                .contains("disabled")
        );
    }
}

#[test]
fn watcher_ids_are_validated_before_publication() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    for fixture_name in ["watcher-duplicate", "watcher-invalid"] {
        let module = WasmModule::load(
            WasmEngine::new().unwrap(),
            fixture(fixture_name),
            config(root.path()),
        )
        .unwrap();
        assert!(
            module
                .mount(&ctx)
                .unwrap_err()
                .to_string()
                .contains("watcher id")
        );
    }
}

#[derive(Debug)]
struct FakeModel {
    calls: std::sync::atomic::AtomicU32,
    delay: Duration,
    oversized: bool,
    fail: bool,
}
impl smith_module::ModuleModel for FakeModel {
    fn complete(
        &self,
        request: smith_module::ModuleModelRequest,
        invocation: smith_module::ModuleModelInvocation,
    ) -> smith_module::ModuleModelFuture<'_> {
        Box::pin(async move {
            use agent_runtime_core::usage::{CounterKind, UsageDelta};
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            assert_eq!(request.system.as_deref(), Some("fixture system"));
            assert_eq!(
                request.messages,
                [(
                    agent_runtime_core::content::Role::User,
                    "fixture prompt".into()
                )]
            );
            assert_eq!(request.max_output_tokens, 32);
            assert_eq!(invocation.module_id, "test/module");
            tokio::time::sleep(self.delay).await;
            if self.fail {
                return Err(agent_runtime_core::error::RuntimeError::tool(
                    "credential-that-must-not-leak",
                ));
            }
            Ok(smith_module::ModuleModelResponse {
                text: if self.oversized {
                    "x".repeat(1024 * 1024 + 1)
                } else {
                    "model answer".into()
                },
                finish_reason: "stop".into(),
                usage: UsageDelta::new()
                    .with(CounterKind::InputUncached, 11)
                    .with(CounterKind::Output, 7),
            })
        })
    }
}
fn fake_model(delay: Duration) -> Arc<FakeModel> {
    Arc::new(FakeModel {
        calls: std::sync::atomic::AtomicU32::new(0),
        delay,
        oversized: false,
        fail: false,
    })
}
fn model_config(root: &std::path::Path) -> WasmModuleConfig {
    let mut config = config(root);
    let grant = CapabilityGrant {
        capability: C::Model,
        roots: vec![],
        hosts: vec![],
        allow_http: false,
    };
    config.requested_capabilities.push(grant.clone());
    config.granted_capabilities.push(grant);
    let notify = CapabilityGrant {
        capability: C::SessionNotify,
        roots: vec![],
        hosts: vec![],
        allow_http: false,
    };
    config.requested_capabilities.push(notify.clone());
    config.granted_capabilities.push(notify);
    config
}

#[derive(Debug, Default)]
struct GatedModel {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl smith_module::ModuleModel for GatedModel {
    fn complete(
        &self,
        _: smith_module::ModuleModelRequest,
        _: smith_module::ModuleModelInvocation,
    ) -> smith_module::ModuleModelFuture<'_> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(smith_module::ModuleModelResponse {
                text: "answer".into(),
                finish_reason: "stop".into(),
                usage: agent_runtime_core::usage::UsageDelta::new(),
            })
        })
    }
}

#[tokio::test]
async fn watcher_skips_a_busy_instance_without_queuing_or_counting_a_failure() {
    let root = tempfile::tempdir().unwrap();
    let model = Arc::new(GatedModel::default());
    let mut ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    ctx.model = Some(model.clone());
    let (_, contributions, health) = mount("complete", model_config(root.path()), &ctx);
    let tool = tool(&contributions);
    let call = tokio::spawn(async move { invoke(tool.as_ref(), "complete").await });
    model.entered.notified().await;
    assert!(
        watcher(&contributions)
            .poll(&SessionId::new("session"))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(health.failure().is_none());
    model.release.notify_one();
    assert!(!call.await.unwrap().unwrap().is_error);
    assert_eq!(
        watcher(&contributions)
            .poll(&SessionId::new("session"))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(health.failure().is_none());
}

#[tokio::test]
async fn model_calls_suspend_guest_deadline_and_survive_limits_and_remounts() {
    let root = tempfile::tempdir().unwrap();
    let model = fake_model(Duration::from_millis(80));
    let mut ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    ctx.model = Some(model.clone());
    let config = model_config(root.path());
    let (module, contributions, health) = mount("complete", config, &ctx);
    let tool = tool(&contributions);
    for _ in 0..4 {
        let output = invoke(tool.as_ref(), "complete").await.unwrap();
        let text = serde_json::to_string(&output).unwrap();
        assert!(
            text.contains("model answer") && text.contains("stop") && text.contains('7'),
            "{text}"
        );
    }
    for _ in 0..3 {
        let output = invoke(tool.as_ref(), "complete").await.unwrap();
        assert!(
            serde_json::to_string(&output)
                .unwrap()
                .contains("LimitReached")
        );
    }
    assert!(health.failure().is_none());
    let Mounted::TrackedContributions { contributions, .. } = module.mount(&ctx).unwrap() else {
        panic!("active")
    };
    let output = invoke(self::tool(&contributions).as_ref(), "complete")
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&output)
            .unwrap()
            .contains("LimitReached")
    );
    assert_eq!(model.calls.load(std::sync::atomic::Ordering::Relaxed), 4);
    let output = invoke_at(tool.as_ref(), "complete", "session", "next-turn")
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&output)
            .unwrap()
            .contains("model answer")
    );
    assert_eq!(model.calls.load(std::sync::atomic::Ordering::Relaxed), 5);
}

#[tokio::test]
async fn model_input_bounds_and_withheld_grant_do_not_call_provider_or_disable() {
    let root = tempfile::tempdir().unwrap();
    let model = fake_model(Duration::ZERO);
    let mut ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    ctx.model = Some(model.clone());
    let mut config = model_config(root.path());
    config.limits.model_max_output_tokens = 16;
    let (_, contributions, health) = mount("complete", config, &ctx);
    for action in ["complete", "output-limit", "many-messages", "large-input"] {
        let text =
            serde_json::to_string(&invoke(tool(&contributions).as_ref(), action).await.unwrap())
                .unwrap();
        assert!(
            text.contains("LimitReached") || text.contains("PayloadTooLarge"),
            "{text}"
        );
    }
    assert!(health.failure().is_none());
    let (_, contributions, health) = mount("complete", self::config(root.path()), &ctx);
    for _ in 0..4 {
        let text = serde_json::to_string(
            &invoke(tool(&contributions).as_ref(), "complete")
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(
            text.contains("CapabilityDenied(Capability::Model)"),
            "{text}"
        );
    }
    assert!(health.failure().is_none());
    assert_eq!(model.calls.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[tokio::test]
async fn model_service_deadline_and_errors_are_recoverable_and_secret_safe() {
    for (delay, oversized, fail, expected) in [
        (Duration::from_millis(80), false, false, "deadline exceeded"),
        (Duration::ZERO, true, false, "PayloadTooLarge"),
        (Duration::ZERO, false, true, "model request failed"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let model = Arc::new(FakeModel {
            calls: 0.into(),
            delay,
            oversized,
            fail,
        });
        let mut ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
        ctx.model = Some(model.clone());
        let mut config = model_config(root.path());
        config.limits.model_deadline = Duration::from_millis(20);
        let (_, contributions, health) = mount("complete", config, &ctx);
        for _ in 0..3 {
            let text = serde_json::to_string(
                &invoke(tool(&contributions).as_ref(), "complete")
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert!(text.contains(expected), "{text}");
            assert!(!text.contains("credential-that-must-not-leak"));
        }
        assert!(health.failure().is_none());
    }
}

#[tokio::test]
async fn model_is_refused_in_every_non_tool_export() {
    let root = tempfile::tempdir().unwrap();
    let model = fake_model(Duration::ZERO);
    let mut ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    ctx.model = Some(model.clone());
    // The fixture asserts phase refusals in describe, mount, status and poll.
    let (_, contributions, health) = mount("complete", model_config(root.path()), &ctx);
    watcher(&contributions)
        .poll(&SessionId::new("session"))
        .await
        .unwrap();
    // Its pipeline exports also assert refusal and preserve the supplied outcome.
    for contribution in &contributions {
        match contribution {
            ModuleContribution::Pipeline(PipelineComponent::ContextContributor(component)) => {
                let view = ContextView {
                    session: SessionId::new("session"),
                    turn: TurnId::new("turn"),
                    history: Arc::from([]),
                    activation: Fingerprint::of("activation"),
                    state: None,
                };
                component.contribute(&view).await.unwrap();
            }
            ModuleContribution::Pipeline(PipelineComponent::ToolOutputProcessor(component)) => {
                let view = ToolOutputView {
                    session: SessionId::new("session"),
                    turn: TurnId::new("turn"),
                    request: RequestId::new("request"),
                    call: ToolCall {
                        id: ToolCallId::new("call"),
                        name: "probe".into(),
                        arguments: serde_json::json!({}),
                    },
                    state: None,
                    usage: Arc::from([]),
                    now: Timestamp(100),
                };
                component
                    .process(&view, ToolOutcome::text("original"))
                    .await
                    .unwrap();
            }
            _ => {}
        }
    }
    assert_eq!(model.calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(health.failure().is_none());
}

#[tokio::test]
async fn model_input_limit_and_lowered_turn_limit_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    let model = fake_model(Duration::ZERO);
    let mut ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    ctx.model = Some(model.clone());
    ctx.max_input_tokens = 1;
    let (_, contributions, health) = mount("complete", model_config(root.path()), &ctx);
    let text = serde_json::to_string(
        &invoke(tool(&contributions).as_ref(), "complete")
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(text.contains("LimitReached"));
    assert!(health.failure().is_none());
    assert_eq!(model.calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    ctx.max_input_tokens = 32000;
    let mut config = model_config(root.path());
    config.limits.model_max_calls_per_turn = 1;
    let (_, contributions, health) = mount("complete", config, &ctx);
    invoke(tool(&contributions).as_ref(), "complete")
        .await
        .unwrap();
    for _ in 0..3 {
        let text = serde_json::to_string(
            &invoke(tool(&contributions).as_ref(), "complete")
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(text.contains("LimitReached"));
    }
    assert_eq!(model.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(health.failure().is_none());
}

#[tokio::test]
async fn guest_deadline_still_interrupts_a_loop_after_model_completion() {
    let root = tempfile::tempdir().unwrap();
    let model = fake_model(Duration::from_millis(80));
    let mut ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    ctx.model = Some(model.clone());
    let (_, contributions, _) = mount("complete", model_config(root.path()), &ctx);
    let error = invoke(tool(&contributions).as_ref(), "loop-after-complete")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("deadline"), "{error}");
    assert_eq!(model.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[tokio::test]
async fn propagated_model_refusals_are_tool_errors_without_disabling() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let (_, contributions, health) = mount("complete", config(root.path()), &ctx);
    for _ in 0..4 {
        assert!(
            invoke(tool(&contributions).as_ref(), "propagate")
                .await
                .unwrap()
                .is_error
        );
    }
    assert!(health.failure().is_none());
}
