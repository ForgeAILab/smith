use super::*;

#[test]
fn image_cache_prefix_is_stable_on_chatgpt_append_and_provider_restart() {
    let make_provider = || {
        ChatGptProvider::new(
            ReplayTransport::single(Vec::new()),
            ChatGptProviderConfig::new("gpt-6-astra", Capabilities::basic_streaming(), "acct_test")
                .unwrap(),
            ProviderCredentialTarget::new("chatgpt").unwrap(),
            Arc::new(StaticProviderCredentialSource::new(Secret::new(
                "test-token",
            ))),
        )
    };
    let image = ContentPart::Image {
        url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==".into(),
        detail: Some("high".into()),
    };
    let mut user = Message::user("compare the reference");
    user.content.push(image.clone());
    let mut request = ProviderRequest::new(
        ModelId::new("gpt-6-astra"),
        vec![
            Message::system("stable visual comparison instructions"),
            user,
            Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new("read-reference"),
                name: "read".into(),
                arguments: json!({"path":"reference.png"}),
            })]),
            Message::tool_result(agent_runtime_core::content::ToolResultBlock {
                call_id: ToolCallId::new("read-reference"),
                name: "read".into(),
                content: vec![image, ContentPart::text("reference.png")],
                is_error: false,
            }),
        ],
    );
    request.tools.push(ToolSchema {
        name: "read".into(),
        description: "Read the reference image".into(),
        input_schema: json!({"type":"object","properties":{"path":{"type":"string"}}}),
    });
    let session = agent_runtime_core::ids::SessionId::new("image-cache-session");
    let provider = make_provider();
    let (mut previous, _) = provider.build_payload(&request, &session).unwrap();
    for _ in 0..4 {
        request
            .messages
            .push(Message::assistant(vec![ContentPart::text("observed")]));
        request.messages.push(Message::user("continue comparing"));
        let (next, _) = provider.build_payload(&request, &session).unwrap();
        for field in ["instructions", "tools", "prompt_cache_key"] {
            assert_eq!(previous[field], next[field], "changed {field}");
        }
        let prefix = previous["input"].as_array().unwrap();
        assert_eq!(
            serde_json::to_vec(prefix).unwrap(),
            serde_json::to_vec(&next["input"].as_array().unwrap()[..prefix.len()]).unwrap()
        );
        let (cold, _) = make_provider().build_payload(&request, &session).unwrap();
        assert_eq!(
            serde_json::to_vec(&next).unwrap(),
            serde_json::to_vec(&cold).unwrap()
        );
        previous = next;
    }
}
