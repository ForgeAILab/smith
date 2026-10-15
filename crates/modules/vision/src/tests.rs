use std::sync::{Arc, Mutex};

use agent_runtime::harness::{
    ComponentDescriptor, TurnCommitHook, TurnCommitPatch, TurnCommitView,
};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::content::{ContentPart, Message, ToolResultBlock};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId};
use agent_runtime_core::provider::ModelId;
use agent_runtime_core::tool::{PreparationContext, Tool};
use agent_runtime_core::usage::UsageRecord;
use agent_runtime_testkit::transport::ReplayTransport;
use async_trait::async_trait;
use serde_json::json;
use sha2::{Digest, Sha256};
use smith_module::{ModulePosture, SessionHistory, VisionBinding, VisionUsageRecorder};

use super::*;

#[derive(Debug, Default)]
struct Usage(Mutex<Vec<UsageRecord>>);
impl VisionUsageRecorder for Usage {
    fn record(&self, _: &SessionId, record: UsageRecord) {
        self.0.lock().unwrap().push(record);
    }
}
#[async_trait]
impl TurnCommitHook for Usage {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new("test.usage", RegistryRevision::new("1"))
    }
    async fn after_commit(&self, _: &TurnCommitView) -> Result<TurnCommitPatch, RuntimeError> {
        Ok(TurnCommitPatch::default())
    }
}
#[derive(Debug)]
struct History;
impl SessionHistory for History {
    fn with_history(
        &self,
        _: &SessionId,
        visitor: &mut dyn FnMut(&[Message]),
    ) -> Result<(), RuntimeError> {
        visitor(&[]);
        Ok(())
    }
}

fn context() -> ModuleContext {
    ModuleContext {
        settings: Default::default(),
        workspace_root: None,
        user_dir: "user".into(),
        is_child_session: false,
        posture: ModulePosture::ReadWrite,
        transport: Arc::new(ReplayTransport::single(Vec::new())),
        http: None,
        image_binding: None,
        main_model_accepts_images: false,
        vision: Some(VisionBinding {
            provider: Arc::new(agent_runtime::provider::fake::FakeProvider::text_reply(
                "answer",
            )),
            provider_name: "vision".into(),
            model: ModelId::new("vision"),
            endpoint: "https://vision.example/v1".into(),
            output_token_cap: 1024,
            usage: Arc::new(Usage::default()),
        }),
        session_history: Some(Arc::new(History)),
        semantic_summary_enabled: false,
        max_input_tokens: 24_000,
        built_in_tools: true,
    }
}

#[test]
fn module_inactive_reasons_and_recomposition_follow_the_active_model() {
    let mut context = context();
    assert!(matches!(
        VisionModule.mount(&context).unwrap(),
        Mounted::Contributions(_)
    ));
    context.main_model_accepts_images = true;
    let Mounted::Inactive { reason } = VisionModule.mount(&context).unwrap() else {
        panic!("inactive")
    };
    assert!(reason.contains("main model accepts images"));
    context.main_model_accepts_images = false;
    assert!(matches!(
        VisionModule.mount(&context).unwrap(),
        Mounted::Contributions(_)
    ));
    context.vision = None;
    let Mounted::Inactive { reason } = VisionModule.mount(&context).unwrap() else {
        panic!("inactive")
    };
    assert!(reason.contains("no vision model"));
    context.built_in_tools = false;
    let Mounted::Inactive { reason } = VisionModule.mount(&context).unwrap() else {
        panic!("inactive")
    };
    assert!(reason.contains("built-in tools are disabled"));
}

#[test]
fn digest_lookup_scans_newest_first_and_retains_nested_source_tool() {
    let url = "data:image/png;base64,one";
    let id = &format!("{:x}", Sha256::digest(url.as_bytes()))[..8];
    let image = ContentPart::Image {
        url: url.into(),
        detail: None,
    };
    let history = vec![
        Message::assistant(vec![image.clone()]),
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new("outer"),
            name: "outer".into(),
            is_error: false,
            content: vec![ContentPart::ToolResult(ToolResultBlock {
                call_id: ToolCallId::new("inner"),
                name: "screenshot".into(),
                is_error: false,
                content: vec![image],
            })],
        }),
    ];
    let found = super::history::find(&history, id).unwrap();
    assert_eq!(found.url, url);
    assert_eq!(found.source_tool.as_deref(), Some("screenshot"));
    assert!(super::history::find(&history[..1], "00000000").is_none());
}

#[tokio::test]
async fn prepare_rejects_invalid_arguments_and_declares_exact_effects() {
    let root = tempfile::tempdir().unwrap();
    let workspace = Arc::new(smith_host::ProjectWorkspace::new(root.path()).unwrap());
    let ctx = PreparationContext {
        session: SessionId::new("s"),
        turn: None,
        call_id: ToolCallId::new("c"),
        request: RequestId::new("r"),
        workspace,
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
    };
    let binding = context().vision.unwrap();
    let tool = tool::ViewImageTool {
        binding,
        history: Arc::new(History),
        generated_dir: root.path().join("generated"),
    };
    for args in [
        json!({"question":"Where?"}),
        json!({"image":"1234abcd","path":"a.png","question":"Where?"}),
        json!({"image":"1234ABCD","question":"Where?"}),
        json!({"image":"1234abcd","question":""}),
        json!({"image":"1234abcd","question":"x".repeat(2001)}),
        json!({"path":null,"question":"Where?"}),
        json!({"path":"/etc/hosts.png","question":"Where?"}),
    ] {
        assert!(tool.prepare(args, &ctx).await.is_err());
    }
    let prepared = tool
        .prepare(
            json!({"image":"1234abcd","question":"x".repeat(2000)}),
            &ctx,
        )
        .await
        .unwrap();
    let permissions = prepared
        .required_permissions()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(
        permissions
            .iter()
            .any(|permission| permission.contains("net"))
    );
    assert!(
        permissions
            .iter()
            .any(|permission| permission.contains("egress"))
    );
    assert!(
        !permissions
            .iter()
            .any(|permission| permission.contains("read"))
    );
    assert!(tool.spec().description.contains("[image <id>:"));
}

#[tokio::test]
async fn mounted_module_contributes_placeholder_guidance() {
    use agent_runtime::context::{FragmentContent, FragmentKind};
    use agent_runtime::harness::ContextView;
    use agent_runtime::registry::Fingerprint;
    use agent_runtime_core::ids::TurnId;

    let Mounted::Contributions(contributions) = VisionModule.mount(&context()).unwrap() else {
        panic!("vision mounts");
    };
    let contributor = contributions
        .into_iter()
        .find_map(|contribution| match contribution {
            ModuleContribution::Pipeline(PipelineComponent::ContextContributor(component)) => {
                Some(component)
            }
            _ => None,
        })
        .expect("mounted vision guidance");
    let patch = contributor
        .contribute(&ContextView {
            session: SessionId::new("s"),
            turn: TurnId::new("t"),
            history: Arc::from([]),
            activation: Fingerprint::of("fixture"),
            state: None,
        })
        .await
        .unwrap();
    let [fragment] = patch.fragments.as_slice() else {
        panic!("one guidance fragment");
    };
    assert_eq!(fragment.kind, FragmentKind::DeveloperInstruction);
    let FragmentContent::Text(body) = &fragment.content else {
        panic!("text guidance");
    };
    assert!(body.contains("[image <id>: …] placeholders"));
    assert!(body.contains("calling view_image with that id as `image`"));
    assert_eq!(body.lines().count(), 1);
}
