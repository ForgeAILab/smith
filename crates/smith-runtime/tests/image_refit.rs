//! The host rebuild used by `/model` must fit retained images to its new provider.

use std::sync::Arc;

use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
use agent_runtime_core::approval::AllowAll;
use agent_runtime_core::content::{ContentPart, Message, UserInput};
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
use agent_runtime_testkit::scenarios::stop_events;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use image::{DynamicImage, ImageFormat};
use smith_config::resolve::{Overrides, ResolveRequest, resolve};
use smith_runtime::checkpoint::{CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError};
use smith_runtime::factory::{HostSurface, RuntimeRequest, image_limits_for_provider_kind};
use smith_runtime::host::{HostSessionRequest, start};

const CONFIG: &str = r#"
default_profile = "visual"
[profiles.visual]
provider = "chatgpt"
model = "seeing"
delegation = false
[profiles.strict]
provider = "xai"
model = "seeing"
delegation = false
[providers.chatgpt]
kind = "chatgpt-responses"
base_url = "https://chatgpt.com/backend-api/codex"
credential = "authfile:chatgpt"
[providers.xai]
kind = "xai-responses"
base_url = "https://api.x.ai/v1"
credential = "authfile:xai"
[models."chatgpt/seeing"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
input_modalities = ["text", "image"]
[models."xai/seeing"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
input_modalities = ["text", "image"]
"#;

#[derive(Debug)]
struct Keys;
impl CheckpointKeyProvider for Keys {
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
        Ok(CheckpointKey::new([0x51; 32]))
    }
}

fn images(history: &[Message]) -> Vec<String> {
    fn visit(parts: &[ContentPart], urls: &mut Vec<String>) {
        for part in parts {
            match part {
                ContentPart::Image { url, .. } => urls.push(url.clone()),
                ContentPart::ToolResult(result) => visit(&result.content, urls),
                _ => {}
            }
        }
    }
    let mut urls = Vec::new();
    for message in history {
        visit(&message.content, &mut urls);
    }
    urls
}

#[tokio::test]
async fn image_refit_model_switch_from_chatgpt_to_xai_preserves_canonical_image() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join(".smith")).unwrap();
    std::fs::write(project.path().join(".smith/config.toml"), CONFIG).unwrap();
    let mut encoded = std::io::Cursor::new(Vec::new());
    DynamicImage::new_rgb8(1800, 20)
        .write_to(&mut encoded, ImageFormat::WebP)
        .unwrap();
    let original = encoded.into_inner();
    std::fs::write(project.path().join("picture.webp"), &original).unwrap();
    let original_url = format!("data:image/webp;base64,{}", STANDARD.encode(&original));
    let mut session_id: Option<SessionId> = None;

    // `/model` saves the session and rebuilds the host with the resolved profile.
    // Switch back too: request-time conversion must not overwrite stored WebP.
    for (index, profile) in ["visual", "strict", "visual"].into_iter().enumerate() {
        let config = resolve(
            &ResolveRequest::new(project.path())
                .with_home_dir(home.path())
                .with_cli(Overrides {
                    profile: Some(profile.into()),
                    ..Overrides::default()
                }),
        )
        .unwrap()
        .config;
        let mut scripts = Vec::new();
        if index == 0 {
            let mut read =
                tool_call_fragments(0, "read-image", "read", r#"{"path":"picture.webp"}"#);
            read.push(ProviderStreamEvent::Finish {
                reason: FinishReason::ToolCalls,
            });
            scripts.push(ScriptedStream::new(read));
        }
        scripts.push(ScriptedStream::new(stop_events("seen")));
        let provider = Arc::new(FakeProvider::new(
            "seeing",
            Capabilities::basic_streaming(),
            scripts,
        ));
        let mut runtime = RuntimeRequest::new(config, HostSurface::Headless);
        runtime.provider = Some(provider.clone());
        runtime.workspace = Some(Arc::new(
            smith_host::ProjectWorkspace::new(project.path()).unwrap(),
        ));
        runtime.approval = Some(Arc::new(AllowAll));
        let mut request =
            HostSessionRequest::new(runtime, project.path()).checkpoint_keys(Arc::new(Keys));
        if let Some(id) = &session_id {
            request = request.resume(id.clone());
        }
        let host = start(request).await.unwrap();
        if let Some(id) = &session_id {
            assert_eq!(host.session().id(), id);
        }
        session_id = Some(host.session().id().clone());
        host.session()
            .run(UserInput::text(if index == 0 {
                "Read picture.webp"
            } else {
                "Describe the earlier image"
            }))
            .await
            .unwrap();
        let requests = provider.requests();
        assert_eq!(requests.len(), if index == 0 { 2 } else { 1 });
        let urls = images(&requests.last().unwrap().messages);
        assert_eq!(urls.len(), 1);
        if profile == "strict" {
            assert!(urls[0].starts_with("data:image/png;base64,"));
            let bytes = STANDARD.decode(urls[0].split_once(',').unwrap().1).unwrap();
            assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Png);
            assert!(smith_tools::image_preparation::image_fits(
                &bytes,
                &image_limits_for_provider_kind("xai-responses")
            ));
        } else {
            assert_eq!(urls[0], original_url);
        }
        assert_eq!(
            images(&host.session().history()),
            std::slice::from_ref(&original_url)
        );
        host.shutdown().await.unwrap();
    }
}
