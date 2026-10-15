//! Vision binding composition, approval, canonical image lookup, and usage.

use std::sync::Arc;
use std::time::Duration;

use agent_runtime::provider::fake::{
    FakeProvider, ScriptedStream, tool_call_fragments, usage_event,
};
use agent_runtime_core::content::{ContentPart, UserInput};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::provider::{
    Capabilities, FinishReason, Provider, ProviderStreamEvent, ToolChoice,
};
use agent_runtime_core::tool::{
    InvocationContext, PreparedToolCall, Tool, ToolEffects, ToolOutcome, ToolSpec,
};
use agent_runtime_testkit::scenarios::stop_events;
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use smith_config::resolve::{KnownModule, Overrides, ResolveRequest, ResolvedConfig, resolve};
use smith_module::{CompiledModule, ModuleComposition, ModuleOrigin, ModuleState};
use smith_runtime::factory::{self, HostSurface, RuntimeRequest, VisionProfileRequest};
use smith_runtime::harness::{HarnessSpec, resolve as resolve_harness};
use smith_runtime::host::{HostSessionRequest, start};

const CONFIG: &str = r#"
default_profile = "work"
[profiles.work]
provider = "local"
model = "worker"
vision = "camera"
use = ["main", "child"]
delegation = false
[profiles.camera]
provider = "camera"
model = "eyes"
[providers.local]
kind = "fake"
[providers.camera]
kind = "openai-compatible"
base_url = "https://vision.example/v1"
[models."local/worker"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
input_modalities = ["text"]
[models."local/seeing"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
input_modalities = ["text", "image"]
[models."camera/eyes"]
context_tokens = 32000
max_input_tokens = 30000
max_output_tokens = 2048
input_modalities = ["text", "image"]
[context]
reasoning_reserve = 0
"#;

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}
impl Fixture {
    fn new(config: &str) -> Self {
        Self::with_approval(config, "allow-all")
    }
    fn with_approval(config: &str, approval_mode: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".smith")).unwrap();
        std::fs::write(project.path().join(".smith/config.toml"), config).unwrap();
        std::fs::create_dir_all(home.path().join(".smith")).unwrap();
        std::fs::write(
            home.path().join(".smith/config.toml"),
            format!("[persistence]\nenabled = false\n[approval]\nmode = \"{approval_mode}\"\n"),
        )
        .unwrap();
        Self { home, project }
    }
    fn resolution(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path())
            .with_home_dir(self.home.path())
            .with_known_modules(vec![KnownModule {
                id: "vision".into(),
                default_enabled: true,
                compiled_in: true,
                legacy_enabled_key: None,
            }])
    }
    fn config(&self, model: &str) -> ResolvedConfig {
        resolve(&self.resolution().with_cli(Overrides {
            model: Some(model.to_owned()),
            ..Overrides::default()
        }))
        .unwrap()
        .config
    }
    fn request(
        &self,
        model: &str,
        main: Arc<FakeProvider>,
        vision: Arc<dyn Provider>,
    ) -> RuntimeRequest {
        let config = self.config(model);
        let target = config.agent.profile.vision.clone().map(|target| {
            let config = resolve(&self.resolution().with_advisor_route(target))
                .unwrap()
                .config;
            Box::new(VisionProfileRequest {
                config,
                catalog_sources: Vec::new(),
                provider: Some(vision),
            })
        });
        let mut request = RuntimeRequest::new(config, HostSurface::Headless);
        request.vision_profile = target;
        request.provider = Some(main);
        request.workspace = Some(Arc::new(
            smith_host::ProjectWorkspace::new(self.project.path()).unwrap(),
        ));
        request.modules = ModuleComposition::with_defaults(
            vec![CompiledModule {
                module: Arc::new(smith_module_vision::VisionModule),
                origin: ModuleOrigin::FirstParty,
            }],
            Vec::new(),
        );
        request
    }
}

fn provider(model: &str, scripts: Vec<ScriptedStream>) -> Arc<FakeProvider> {
    Arc::new(FakeProvider::new(
        model,
        Capabilities::basic_streaming(),
        scripts,
    ))
}
fn step(id: &str, name: &str, args: Value) -> ScriptedStream {
    let mut events = tool_call_fragments(0, id, name, &args.to_string());
    events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    ScriptedStream::new(events)
}
fn activate(ids: &[&str]) -> ScriptedStream {
    step("activate", "registry.activate", json!({"ids":ids}))
}
fn vision_provider() -> Arc<FakeProvider> {
    let mut events = vec![usage_event(50, 10)];
    events.extend(stop_events("Save is at (720, 450)."));
    provider("eyes", vec![ScriptedStream::new(events)])
}
fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend(1440_u32.to_be_bytes());
    bytes.extend(900_u32.to_be_bytes());
    bytes
}
fn image_url() -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png())
    )
}
fn image_id() -> String {
    format!("{:x}", Sha256::digest(image_url().as_bytes()))[..8].to_owned()
}

#[derive(Debug)]
struct Screenshot;
#[async_trait]
impl Tool for Screenshot {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "capture",
            "Capture a screenshot.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
            ToolEffects::default(),
        )
    }
    async fn invoke(
        &self,
        _: PreparedToolCall,
        _: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        Ok(ToolOutcome {
            value: Value::Null,
            content: vec![ContentPart::Image {
                url: image_url(),
                detail: Some("high".into()),
            }]
            .into(),
            is_error: false,
        })
    }
}

#[tokio::test]
async fn locate_control_round_trip_sends_one_image_metadata_question_and_no_history_and_records_usage()
 {
    let fixture = Fixture::new(CONFIG);
    let main = provider(
        "worker",
        vec![
            activate(&["tool:capture", "tool:view_image"]),
            step("capture", "capture", json!({})),
            step(
                "view",
                "view_image",
                json!({"image":image_id(),"question":"Where is Save?"}),
            ),
            ScriptedStream::new(stop_events("done")),
        ],
    );
    let vision = vision_provider();
    let mut request = fixture.request("worker", main.clone(), vision.clone());
    request.trusted_native.add_tool(Arc::new(Screenshot));
    let host = start(HostSessionRequest::new(request, fixture.project.path()))
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(10),
        host.session()
            .run(UserInput::text("PRIVATE HISTORY: locate the control")),
    )
    .await
    .unwrap()
    .unwrap();
    let requests = vision.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.messages.len(), 2);
    assert!(
        request.messages[0]
            .joined_text()
            .contains("pixel coordinates")
    );
    assert!(request.messages[1].joined_text().contains("1440×900"));
    assert!(
        request.messages[1]
            .joined_text()
            .contains("Source tool: capture")
    );
    assert!(request.messages[1].joined_text().contains("Where is Save?"));
    assert!(!format!("{:?}", request.messages).contains("PRIVATE HISTORY"));
    assert!(
        matches!(&request.messages[1].content[1], ContentPart::Image { url, detail } if *url == image_url() && detail.as_deref() == Some("high"))
    );
    assert_eq!(request.max_output_tokens, Some(1024));
    assert!(request.tools.is_empty());
    assert_eq!(request.tool_choice, ToolChoice::None);
    assert!(host.session().history().iter().any(|message| message.content.iter().any(|part| matches!(part, ContentPart::ToolResult(result) if result.name == "view_image" && !result.is_error && result.content.iter().any(|part| part.as_text() == Some("Save is at (720, 450)."))))));
    let snapshot = host.snapshot();
    let usage = snapshot
        .usage
        .records()
        .iter()
        .filter(|record| record.provenance.purpose.as_deref() == Some("vision"))
        .collect::<Vec<_>>();
    assert_eq!(usage.len(), 1);
    assert!(!usage[0].provenance.failed);
    assert_eq!(
        usage[0]
            .delta
            .get(agent_runtime_core::usage::CounterKind::InputUncached),
        50
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn workspace_image_is_read_and_sent_after_approval() {
    let fixture = Fixture::new(CONFIG);
    std::fs::write(fixture.project.path().join("mockup.png"), png()).unwrap();
    let vision = vision_provider();
    let main = provider(
        "worker",
        vec![
            activate(&["tool:view_image"]),
            step(
                "view",
                "view_image",
                json!({"path":"mockup.png","question":"Where is Save?"}),
            ),
            ScriptedStream::new(stop_events("done")),
        ],
    );
    let host = start(HostSessionRequest::new(
        fixture.request("worker", main, vision.clone()),
        fixture.project.path(),
    ))
    .await
    .unwrap();
    host.session()
        .run(UserInput::text("Inspect the workspace mockup."))
        .await
        .unwrap();
    let requests = vision.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].messages[1].joined_text().contains("1440×900"));
    assert!(
        matches!(&requests[0].messages[1].content[1], ContentPart::Image { url, .. } if *url == image_url())
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn compacted_id_and_outside_path_make_no_vision_request() {
    for (args, expected) in [
        (
            json!({"image":image_id(),"question":"Where is Save?"}),
            "is no longer in session history",
        ),
        (
            json!({"path":"/etc/hosts.png","question":"Where is Save?"}),
            "image reference",
        ),
    ] {
        let fixture = Fixture::new(CONFIG);
        let main = provider(
            "worker",
            vec![
                activate(&["tool:view_image"]),
                step("view", "view_image", args),
                ScriptedStream::new(stop_events("done")),
            ],
        );
        let vision = vision_provider();
        let host = start(HostSessionRequest::new(
            fixture.request("worker", main, vision.clone()),
            fixture.project.path(),
        ))
        .await
        .unwrap();
        host.session()
            .run(UserInput::text("Inspect an unavailable image."))
            .await
            .unwrap();
        assert!(vision.requests().is_empty());
        assert!(format!("{:?}", host.session().history()).contains(expected));
        host.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn ask_mode_requires_approval_before_reading_or_sending_image() {
    let fixture = Fixture::with_approval(CONFIG, "ask");
    // This file does not exist: approval must happen before the read can fail.
    let main = provider(
        "worker",
        vec![
            activate(&["tool:view_image"]),
            step(
                "view",
                "view_image",
                json!({"path":"missing.png","question":"Where is Save?"}),
            ),
            ScriptedStream::new(stop_events("done")),
        ],
    );
    let vision = vision_provider();
    let mut request = fixture.request("worker", main, vision.clone());
    let approval = Arc::new(smith_host::HeadlessApproval::new());
    request.approval = Some(approval.clone());
    let host = start(HostSessionRequest::new(request, fixture.project.path()))
        .await
        .unwrap();
    host.session()
        .run(UserInput::text("Inspect the mockup."))
        .await
        .unwrap();
    let required = approval
        .required()
        .expect("vision effects require approval");
    assert_eq!(required.tool, "view_image");
    for permission in ["host.fs.read", "net.http", "data.egress"] {
        assert!(
            required.permissions.iter().any(|value| value == permission),
            "{required:?}"
        );
    }
    assert!(vision.requests().is_empty());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn model_rebuild_mounts_and_unmounts_vision_from_the_active_binding() {
    let fixture = Fixture::new(CONFIG);
    let vision = vision_provider();
    for (model, available) in [("seeing", false), ("worker", true), ("seeing", false)] {
        let mut request = fixture.request(model, provider(model, Vec::new()), vision.clone());
        if !available {
            request.vision_profile = None;
        }
        let smith = factory::build(resolve_harness(HarnessSpec::trusted(request)).unwrap())
            .await
            .unwrap();
        assert_eq!(smith.abilities().names().contains(&"view_image"), available);
        let report = smith
            .module_report()
            .iter()
            .find(|report| report.descriptor.id == "vision")
            .unwrap();
        if !available {
            assert!(
                matches!(&report.state, ModuleState::Inactive { reason } if reason.contains("main model accepts images"))
            );
        }
    }
    assert!(vision.requests().is_empty());
}

#[tokio::test]
async fn inactive_and_disabled_modules_need_no_vision_provider() {
    for condition in ["unconfigured", "builtins-disabled", "module-off"] {
        let config = if condition == "unconfigured" {
            CONFIG.replace("vision = \"camera\"", "")
        } else {
            CONFIG.to_owned()
        };
        let fixture = Fixture::new(&config);
        let mut request =
            fixture.request("worker", provider("worker", Vec::new()), vision_provider());
        request.vision_profile = None;
        match condition {
            "builtins-disabled" => request.built_in_tools = false,
            "module-off" => {
                request.modules.enabled.remove("vision");
            }
            _ => {}
        }
        let smith = factory::build(resolve_harness(HarnessSpec::trusted(request)).unwrap())
            .await
            .unwrap();
        assert!(!smith.abilities().names().contains(&"view_image"));
        let state = &smith
            .module_report()
            .iter()
            .find(|report| report.descriptor.id == "vision")
            .unwrap()
            .state;
        match condition {
            "unconfigured" => assert!(
                matches!(state, ModuleState::Inactive { reason } if reason.contains("no vision model"))
            ),
            "builtins-disabled" => assert!(
                matches!(state, ModuleState::Inactive { reason } if reason.contains("built-in tools are disabled"))
            ),
            _ => assert_eq!(state, &ModuleState::Off),
        }
    }
}

#[tokio::test]
async fn child_composition_mounts_its_own_vision_target() {
    let fixture = Fixture::new(CONFIG);
    let vision = vision_provider();
    let main = provider(
        "worker",
        vec![
            activate(&["tool:view_image"]),
            step(
                "view",
                "view_image",
                json!({"image":image_id(),"question":"Where is Save?"}),
            ),
            ScriptedStream::new(stop_events("done")),
        ],
    );
    let mut request = fixture.request("worker", main, vision.clone());
    request.surface = HostSurface::Child;
    let host = start(HostSessionRequest::new(request, fixture.project.path()))
        .await
        .unwrap();
    host.session()
        .run(UserInput {
            parts: vec![
                ContentPart::text("Find Save."),
                ContentPart::Image {
                    url: image_url(),
                    detail: None,
                },
            ],
        })
        .await
        .unwrap();
    assert_eq!(vision.requests().len(), 1);
    host.shutdown().await.unwrap();
}

#[derive(Debug)]
struct UsageBarrier {
    inner: Arc<FakeProvider>,
    consumed: Arc<tokio::sync::Notify>,
}
#[async_trait]
impl Provider for UsageBarrier {
    fn describe(&self) -> Vec<agent_runtime_core::provider::ModelDescriptor> {
        self.inner.describe()
    }
    fn capabilities(&self, model: &agent_runtime_core::provider::ModelId) -> Option<Capabilities> {
        self.inner.capabilities(model)
    }
    async fn stream(
        &self,
        request: agent_runtime_core::provider::ProviderRequest,
        context: agent_runtime_core::provider::ProviderCallContext,
    ) -> Result<
        agent_runtime_core::provider::ProviderStream,
        agent_runtime_core::provider::ProviderError,
    > {
        use futures_util::StreamExt;
        let mut stream = self.inner.stream(request, context).await?;
        let consumed = self.consumed.clone();
        Ok(Box::pin(async_stream::stream! {
            while let Some(event) = stream.next().await {
                let usage = matches!(&event, ProviderStreamEvent::Usage { .. });
                yield event;
                if usage { consumed.notify_one(); }
            }
        }))
    }
}

#[tokio::test]
async fn interrupted_vision_usage_remains_in_live_and_saved_session_accounting() {
    let fixture = Fixture::new(CONFIG);
    let consumed = Arc::new(tokio::sync::Notify::new());
    let vision = Arc::new(UsageBarrier {
        inner: provider(
            "eyes",
            vec![ScriptedStream::blocking(vec![usage_event(50, 10)])],
        ),
        consumed: consumed.clone(),
    });
    let main = provider(
        "worker",
        vec![
            activate(&["tool:view_image"]),
            step(
                "view",
                "view_image",
                json!({"image":image_id(),"question":"Where is Save?"}),
            ),
        ],
    );
    let mut request = fixture.request("worker", main, vision);
    let store = Arc::new(agent_runtime_testkit::InMemorySessionStore::new());
    request.session_store = Some(store.clone());
    let host = start(HostSessionRequest::new(request, fixture.project.path()))
        .await
        .unwrap();
    let turn = host
        .session()
        .send(UserInput {
            parts: vec![
                ContentPart::text("Find Save."),
                ContentPart::Image {
                    url: image_url(),
                    detail: None,
                },
            ],
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), consumed.notified())
        .await
        .expect("vision received usage");
    host.session()
        .interrupt_current_turn(agent_runtime_core::cancel::CancelReason::UserRequested)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), turn.completed())
        .await
        .expect("vision cancellation completes");
    let snapshot = host.snapshot();
    let records = snapshot
        .usage
        .records()
        .iter()
        .filter(|record| record.provenance.purpose.as_deref() == Some("vision"))
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert!(records[0].provenance.failed);
    assert_eq!(
        records[0]
            .delta
            .get(agent_runtime_core::usage::CounterKind::InputUncached),
        50
    );
    host.shutdown().await.unwrap();
    use agent_runtime_core::store::SessionStore;
    let saved = store.load(host.session().id()).await.unwrap().unwrap();
    assert_eq!(saved.usage.total(), snapshot.usage.total());
}

#[tokio::test]
async fn vision_route_uses_its_own_catalog_price() {
    let fixture = Fixture::new(&CONFIG.replace(
        "kind = \"openai-compatible\"\nbase_url = \"https://vision.example/v1\"",
        "kind = \"gemini-interactions\"",
    ));
    let price = smith_config::catalog::CatalogModelCost {
        input: Some(10_000_000),
        output: Some(20_000_000),
        cache_read: Some(1_000_000),
        cache_write: Some(20_000_000),
    };
    let mut catalog: smith_config::catalog::CatalogSnapshot =
        serde_json::from_str(smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED).unwrap();
    let google = catalog.providers.get_mut("google").unwrap();
    let mut model = google
        .models
        .values()
        .find(|model| {
            model
                .input_modalities
                .contains(&smith_config::catalog::CatalogModality::Image)
        })
        .unwrap()
        .clone();
    model.id = "eyes".into();
    model.cost = Some(price);
    google.models.insert(model.id.clone(), model);
    let mut request = fixture.request("worker", provider("worker", Vec::new()), vision_provider());
    request.model_catalog = Some(Arc::new(catalog));
    let runtime = factory::build(resolve_harness(HarnessSpec::trusted(request)).unwrap())
        .await
        .unwrap();
    let vision = runtime.vision_route().unwrap();
    assert_eq!(vision.provider_name, "camera");
    assert_eq!(vision.model.as_str(), "eyes");
    assert_eq!(runtime.vision_price(), Some(&price));
}

#[tokio::test]
async fn view_image_is_offered_with_placeholder_guidance_on_the_first_request() {
    for surface in [HostSurface::Headless, HostSurface::Child] {
        let fixture = Fixture::new(CONFIG);
        let main = provider("worker", vec![ScriptedStream::new(stop_events("done"))]);
        let mut request = fixture.request("worker", main.clone(), vision_provider());
        request.surface = surface;
        let host = start(HostSessionRequest::new(request, fixture.project.path()))
            .await
            .unwrap();
        host.session().run(UserInput::text("hi")).await.unwrap();
        let requests = main.requests();
        assert!(
            requests[0]
                .tools
                .iter()
                .any(|tool| tool.name == "view_image")
        );
        let instructions = requests[0]
            .messages
            .iter()
            .map(|message| message.joined_text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            instructions.contains("Images shown as [image <id>: …] placeholders"),
            "{instructions}"
        );
        assert!(instructions.contains("calling view_image with that id as `image`"));
        host.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn registry_search_finds_view_image_by_image_vision_screenshot_and_view_image() {
    for query in ["image", "vision", "screenshot", "view image"] {
        let fixture = Fixture::new(CONFIG);
        let main = provider(
            "worker",
            vec![
                step("search", "registry.search", json!({"query":query})),
                ScriptedStream::new(stop_events("done")),
            ],
        );
        let host = start(HostSessionRequest::new(
            fixture.request("worker", main, vision_provider()),
            fixture.project.path(),
        ))
        .await
        .unwrap();
        host.session().run(UserInput::text("hi")).await.unwrap();
        let history = host.session().history();
        let result = history
            .iter()
            .flat_map(|message| &message.content)
            .find_map(|part| match part {
                ContentPart::ToolResult(result) if result.name == "registry.search" => Some(result),
                _ => None,
            })
            .expect("registry search result");
        assert!(!result.is_error, "{query}: {result:?}");
        assert!(
            format!("{:?}", result.content).contains("tool:view_image"),
            "{query}: {result:?}"
        );
        host.shutdown().await.unwrap();
    }
}
