use agent_runtime_core::content::Message;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::provider_credential::{
    ProviderCredentialTarget, StaticProviderCredentialSource,
};
use agent_runtime_core::store::Secret;
use agent_runtime_testkit::transport::ReplayTransport;
use smith_module::{ImageBinding, ModuleSettings, SessionHistory};

use super::*;

#[derive(Debug)]
struct History(Vec<Message>);

impl SessionHistory for History {
    fn with_history(
        &self,
        _: &SessionId,
        visitor: &mut dyn FnMut(&[Message]),
    ) -> Result<(), RuntimeError> {
        visitor(&self.0);
        Ok(())
    }
}

fn context() -> ModuleContext {
    ModuleContext {
        settings: ModuleSettings::new(),
        user_dir: "user".into(),
        posture: ModulePosture::ReadWrite,
        transport: Arc::new(ReplayTransport::single(Vec::new())),
        image_binding: Some(ImageBinding {
            endpoint: "https://api.openai.com/v1".into(),
            target: ProviderCredentialTarget::new("openai").unwrap(),
            credentials: Arc::new(StaticProviderCredentialSource::new(Secret::new("fixture"))),
            chatgpt: false,
        }),
        session_history: Some(Arc::new(History(Vec::new()))),
        semantic_summary_enabled: false,
        max_input_tokens: 24_000,
        built_in_tools: true,
    }
}

#[test]
fn tool_mounts_only_with_an_image_route_and_writable_builtin_tools() {
    let Mounted::Contributions(contributions) = ImageGenerationModule.mount(&context()).unwrap()
    else {
        panic!("image tool should mount");
    };
    assert_eq!(contributions.len(), 1);
    let ModuleContribution::Tool(tool) = &contributions[0] else {
        panic!("expected tool");
    };
    assert_eq!(tool.spec().name, "generate_image");
    for condition in 0..3 {
        let mut context = context();
        match condition {
            0 => context.image_binding = None,
            1 => context.posture = ModulePosture::ReadOnly,
            _ => context.built_in_tools = false,
        }
        assert!(matches!(
            ImageGenerationModule.mount(&context).unwrap(),
            Mounted::Inactive { .. }
        ));
    }
}

#[test]
fn recent_image_adapter_preserves_newest_first_order_and_short_history_error() {
    use agent_runtime_core::content::ContentPart;
    use smith_tools::RecentImageSource;

    let source = image_history::ConversationImages(Arc::new(History(vec![
        Message::assistant(vec![ContentPart::Image {
            url: "data:image/png;base64,one".into(),
            detail: None,
        }]),
        Message::assistant(vec![ContentPart::Image {
            url: "data:image/png;base64,two".into(),
            detail: None,
        }]),
    ])));
    let images = source.recent_images(&SessionId::new("s"), 2).unwrap();
    assert_eq!(
        images,
        ["data:image/png;base64,two", "data:image/png;base64,one"]
    );
    let error = source.recent_images(&SessionId::new("s"), 3).unwrap_err();
    assert!(error.message.contains("this session has only 2"));
}
