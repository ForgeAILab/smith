#[test]
fn only_the_official_anthropic_endpoint_drops_rebound_thinking_blocks() {
    use agent_runtime::provider::anthropic::ThinkingBlockBinding;
    assert_eq!(
        crate::factory::provider::adapter::anthropic_block_binding("https://api.anthropic.com/v1"),
        Some(ThinkingBlockBinding::DropBlock)
    );
    for gateway in [
        "https://gateway.example/v1",
        "https://api.anthropic.com/v1/proxy",
        "",
    ] {
        assert_eq!(
            crate::factory::provider::adapter::anthropic_block_binding(gateway),
            None,
            "{gateway}"
        );
    }
}
