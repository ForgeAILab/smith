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
        http: Some(Arc::new(ReplayHttp(transport.clone()))),
        settings: BTreeMap::new(),
        workspace_root: Some(root.canonicalize().unwrap()),
        user_dir: root.join("host"),
        posture: ModulePosture::ReadWrite,
        transport,
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
            turn: Some(TurnId::new("turn")),
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
    assert!(error.to_string().contains("9.0.0") && error.to_string().contains("0.1.x"));
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
