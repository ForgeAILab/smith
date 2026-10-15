//! Image-generation module calls cross authority and approval before HTTP or storage.

use std::sync::Arc;
use std::time::Duration;

use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
use agent_runtime_core::content::{ContentPart, UserInput};
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
use agent_runtime_core::provider_credential::{
    ProviderCredentialTarget, StaticProviderCredentialSource,
};
use agent_runtime_core::store::Secret;
use agent_runtime_testkit::scenarios::stop_events;
use agent_runtime_testkit::transport::ReplayTransport;
use base64::Engine as _;
use serde_json::{Value, json};
use smith_config::resolve::{KnownModule, ResolveRequest, resolve};
use smith_module::{
    CompiledModule, ImageBinding, Module, ModuleComposition, ModuleContext, ModuleError,
    ModuleOrigin, Mounted,
};
use smith_runtime::factory::{HostSurface, RuntimeRequest};
use smith_runtime::host::{HostSessionRequest, start};

const CONFIG: &str = r#"
default_profile = "work"
[profiles.work]
provider = "local"
model = "worker"
delegation = false
[providers.local]
kind = "fake"
[models."local/worker"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
input_modalities = ["text", "image"]
[context]
reasoning_reserve = 0
"#;

const PNG_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j5xkAAAAASUVORK5CYII=";

/// Supplies an offline image route while mounting the real module and backend.
#[derive(Debug)]
struct OfflineImageModule(Arc<ReplayTransport>);

impl Module for OfflineImageModule {
    fn id(&self) -> &str {
        "image-generation"
    }

    fn revision(&self) -> &str {
        "offline-image-generation-v1"
    }

    fn description(&self) -> &str {
        "Image generation with a replay transport"
    }

    fn default_enabled(&self) -> bool {
        true
    }

    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        let mut context = context.clone();
        context.transport = self.0.clone();
        context.image_binding = Some(ImageBinding {
            endpoint: "https://images.example/v1".into(),
            target: ProviderCredentialTarget::new("openai").unwrap(),
            credentials: Arc::new(StaticProviderCredentialSource::new(Secret::new("fixture"))),
            chatgpt: false,
        });
        smith_module_image_generation::ImageGenerationModule.mount(&context)
    }
}

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new(mode: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".smith")).unwrap();
        std::fs::write(project.path().join(".smith/config.toml"), CONFIG).unwrap();
        std::fs::create_dir_all(home.path().join(".smith")).unwrap();
        std::fs::write(
            home.path().join(".smith/config.toml"),
            format!("[persistence]\nenabled = false\n[approval]\nmode = \"{mode}\"\n"),
        )
        .unwrap();
        Self { home, project }
    }

    fn request(&self, arguments: Value, transport: Arc<ReplayTransport>) -> RuntimeRequest {
        let config = resolve(
            &ResolveRequest::new(self.project.path())
                .with_home_dir(self.home.path())
                .with_known_modules(vec![KnownModule {
                    id: "image-generation".into(),
                    default_enabled: true,
                    compiled_in: true,
                    legacy_enabled_key: None,
                }]),
        )
        .unwrap()
        .config;
        let mut request = RuntimeRequest::new(config, HostSurface::Headless);
        request.provider = Some(Arc::new(FakeProvider::new(
            "worker",
            Capabilities::basic_streaming(),
            vec![
                step(
                    "activate",
                    "registry.activate",
                    json!({"ids":["tool:generate_image"]}),
                ),
                step("image", "generate_image", arguments),
                ScriptedStream::new(stop_events("done")),
            ],
        )));
        request.workspace = Some(Arc::new(
            smith_host::ProjectWorkspace::new(self.project.path()).unwrap(),
        ));
        request.modules = ModuleComposition::with_defaults(
            vec![CompiledModule {
                module: Arc::new(OfflineImageModule(transport)),
                origin: ModuleOrigin::FirstParty,
            }],
            Vec::new(),
        );
        request
    }
}

fn step(id: &str, name: &str, args: Value) -> ScriptedStream {
    let mut events = tool_call_fragments(0, id, name, &args.to_string());
    events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    ScriptedStream::new(events)
}

fn transport() -> Arc<ReplayTransport> {
    Arc::new(ReplayTransport::single(
        serde_json::to_vec(&json!({"data":[{"b64_json":PNG_BASE64}]})).unwrap(),
    ))
}

#[tokio::test]
async fn generate_image_reaches_module_backend_and_saves_png_after_approval() {
    for references in [false, true] {
        let fixture = Fixture::new("allow-all");
        let png = base64::engine::general_purpose::STANDARD
            .decode(PNG_BASE64)
            .unwrap();
        std::fs::write(fixture.project.path().join("reference.png"), &png).unwrap();
        let arguments = if references {
            json!({"prompt":"A small blue bird", "reference_paths":["reference.png"]})
        } else {
            json!({"prompt":"A small blue bird"})
        };
        let transport = transport();
        let host = start(HostSessionRequest::new(
            fixture.request(arguments, transport.clone()),
            fixture.project.path(),
        ))
        .await
        .unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            host.session().run(UserInput::text("Generate a bird.")),
        )
        .await
        .unwrap()
        .unwrap();
        let history = host.session().history();
        let result = history
            .iter()
            .flat_map(|message| &message.content)
            .find_map(|part| match part {
                ContentPart::ToolResult(result) if result.name == "generate_image" => Some(result),
                _ => None,
            })
            .expect("image tool result");
        assert_eq!(transport.requests().len(), 1, "{result:?}");
        assert!(!result.is_error, "{result:?}");
        let endpoint = if references { "edits" } else { "generations" };
        assert_eq!(
            transport.requests()[0].url,
            format!("https://images.example/v1/images/{endpoint}")
        );
        assert!(result.content.iter().any(|part| matches!(
            part,
            ContentPart::Image { url, .. } if url == &format!("data:image/png;base64,{PNG_BASE64}")
        )));
        let path = fixture
            .home
            .path()
            .join(".smith/generated_images")
            .join(host.session().id().as_str())
            .join("image.png");
        assert_eq!(std::fs::read(path).unwrap(), png);
        host.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn generate_image_waits_for_approval_before_reference_reads_http_and_storage() {
    let fixture = Fixture::new("ask");
    let transport = transport();
    let mut request = fixture.request(
        // Approval must happen before the missing reference can be read.
        json!({"prompt":"A small blue bird", "reference_paths":["missing.png"]}),
        transport.clone(),
    );
    let approval = Arc::new(smith_host::HeadlessApproval::new());
    request.approval = Some(approval.clone());
    let host = start(HostSessionRequest::new(request, fixture.project.path()))
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(10),
        host.session().run(UserInput::text("Generate a bird.")),
    )
    .await
    .unwrap()
    .unwrap();
    let required = approval.required().expect("image effects require approval");
    assert_eq!(required.tool, "generate_image");
    for permission in ["host.fs.read", "host.fs.write", "net.http", "data.egress"] {
        assert!(
            required.permissions.iter().any(|value| value == permission),
            "{required:?}"
        );
    }
    assert!(transport.requests().is_empty());
    assert!(!fixture.home.path().join(".smith/generated_images").exists());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn generate_image_denied_approval_makes_no_backend_request_or_directory() {
    let fixture = Fixture::new("deny");
    let transport = transport();
    let host = start(HostSessionRequest::new(
        fixture.request(json!({"prompt":"A small blue bird"}), transport.clone()),
        fixture.project.path(),
    ))
    .await
    .unwrap();
    tokio::time::timeout(
        Duration::from_secs(10),
        host.session().run(UserInput::text("Generate a bird.")),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(transport.requests().is_empty());
    assert!(!fixture.home.path().join(".smith/generated_images").exists());
    assert!(host.session().history().iter().any(|message| {
        message.content.iter().any(|part| matches!(
            part,
            ContentPart::ToolResult(result) if result.name == "generate_image" && result.is_error
        ))
    }));
    host.shutdown().await.unwrap();
}
