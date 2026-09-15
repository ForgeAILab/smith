from pathlib import Path

def edit(path, old, new, n=1):
    p = Path(path)
    s = p.read_text()
    assert s.count(old) == n, (path, s.count(old), old[:100])
    p.write_text(s.replace(old, new))

edit('crates/smith-config/src/model.rs',
    'pub struct ContextSection {\n',
    '''pub struct ContextSection {
    /// Serialized tool outcomes larger than this are stored as artifacts instead
    /// of replayed inline. This is a byte threshold, not an exact token budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_output_inline_bytes: Option<u32>,
''')
edit('crates/smith-config/src/resolve/types.rs', 'pub struct ResolvedContext {\n',
    '''pub struct ResolvedContext {
    /// Serialized tool-outcome offload threshold, independent of capture limits.
    pub tool_output_inline_bytes: Sourced<u32>,
''')
edit('crates/smith-config/src/resolve/load.rs',
    '        context: Some(ContextSection {\n',
    '        context: Some(ContextSection {\n            tool_output_inline_bytes: Some(8 * 1024),\n')
edit('crates/smith-config/src/resolve/provenance.rs',
    '    ("context.reasoning_reserve", ValueKind::Integer),',
    '    ("context.reasoning_reserve", ValueKind::Integer),\n    ("context.tool_output_inline_bytes", ValueKind::Integer),')
edit('crates/smith-config/src/resolve/provider.rs',
    '    Ok(ResolvedContext {\n',
    '''    Ok(ResolvedContext {
        tool_output_inline_bytes: bounded_u32(
            provenance, "context.tool_output_inline_bytes", 256, 1024 * 1024,
        )?,
''')
edit('crates/smith-runtime/src/lib.rs', 'pub mod transport;','pub mod tool_output;\npub mod transport;')
p = 'crates/smith-runtime/src/factory.rs'
edit(p, '    ArtifactOffloader, ArtifactReadTool, CreateGoalTool, GetGoalTool, GoalComponent,',
        '    CreateGoalTool, GetGoalTool, GoalComponent,')
edit(p, 'use crate::transport::{ReqwestTransport, TransportConfig};',
        'use crate::tool_output::ToolOutputContextPolicy;\nuse crate::transport::{ReqwestTransport, TransportConfig};')
edit(p, '    pub output_limit: usize,',
    '''    pub output_limit: usize,
    /// Effective inline threshold and bounded artifact retrieval policy.
    pub tool_output_context: ToolOutputContextPolicy,
    /// Whether full tool outcomes can actually be retained by this composition.
    pub artifact_offloading: bool,''')
edit(p, '        output_limit: loop_config.output_limit,',
    '''        output_limit: loop_config.output_limit,
        tool_output_context: ToolOutputContextPolicy::from_config(config),
        artifact_offloading: request.artifact_store.is_some(),''')
edit(p, '''        let offloader = ArtifactOffloader::new(store)
            .with_threshold_bytes(loop_config.output_limit)
            .map_err(FactoryError::Runtime)?;''',
    '''        let offloader = ToolOutputContextPolicy::from_config(config)
            .offloader(store)
            .map_err(FactoryError::Runtime)?;''')
edit(p, '        tools.push(Arc::new(ArtifactReadTool::new(store)));',
    '        tools.push(Arc::new(ToolOutputContextPolicy::from_config(&request.config).reader(store)));')
edit(p, '                    context_policy,\n                    loop_config,',
    '                    context_policy,\n                    tool_output_context: ToolOutputContextPolicy::from_config(config),\n                    loop_config,')
edit(p, '                context_policy,\n                loop_config,',
    '                context_policy,\n                tool_output_context: ToolOutputContextPolicy::from_config(&route_request.config),\n                loop_config,')
edit(p, '        ResolvedContext {\n',
    '        ResolvedContext {\n            tool_output_inline_bytes: sourced(8 * 1024),\n')
p = 'crates/smith-runtime/src/delegation.rs'
edit(p, '    ArtifactOffloader, ArtifactReadTool, MemoryContributor, QuestionnaireTool,',
        '    MemoryContributor, QuestionnaireTool,')
edit(p, '    pub(crate) context_policy: ContextPolicy,',
    '''    pub(crate) context_policy: ContextPolicy,
    pub(crate) tool_output_context: crate::tool_output::ToolOutputContextPolicy,''')
edit(p, '            tools.push(Arc::new(ArtifactReadTool::new(store)));',
    '            tools.push(Arc::new(route.tool_output_context.reader(store)));')
edit(p, '            builder = builder.tool_output_processor(Arc::new(ArtifactOffloader::new(store)));',
    '            builder = builder.tool_output_processor(Arc::new(route.tool_output_context.offloader(store)?));')
p = 'crates/smith-cli/src/local_command.rs'
# Only the context inspector changes; ordinary status stays concise.
def edit_context(path, old, new):
    p = Path(path)
    before, after = p.read_text().split('pub(super) fn render_context_view', 1)
    assert after.count(old) == 1
    p.write_text(before + 'pub(super) fn render_context_view' + after.replace(old, new))
edit_context(p, '''    lines.push(format!(
        "provider input (session): {}",''',
    '''    lines.push(if policy.artifact_offloading {
        format!(
            "tool context: offload above {} serialized bytes · artifact pages up to {} bytes",
            policy.tool_output_context.inline_bytes,
            policy.tool_output_context.artifact_page_bytes,
        )
    } else {
        "tool context: artifact storage unavailable; ordinary output limits still apply".to_owned()
    });
    lines.push("Input occupancy above is the last planned request, not cumulative session usage.".to_owned());
    lines.push(format!(
        "provider input (session): {}",''')

p = 'crates/smith-runtime/tests/delegation.rs'
edit(p, 'async fn a_child_artifact_is_explicitly_transferred_without_widening_source_ownership() {\n    let fixture = Fixture::new();\n    let large_fixture = "child-owned artifact line\\n".repeat(10_000);',
'async fn a_child_artifact_is_explicitly_transferred_without_widening_source_ownership() {\n    child_artifact_policy_case(10_000, 8192).await;\n}\n\n#[tokio::test]\nasync fn a_child_uses_the_resolved_small_inline_threshold_not_the_runtime_default() {\n    child_artifact_policy_case(100, 1024).await;\n}\n\nasync fn child_artifact_policy_case(lines: usize, inline_bytes: u32) {\n    let fixture = Fixture::new();\n    let large_fixture = "child-owned artifact line\\n".repeat(lines);')
edit(p, '    runtime_request.artifact_store = Some(store.clone());',
    '    runtime_request.artifact_store = Some(store.clone());\n    runtime_request.config.context.tool_output_inline_bytes.value = inline_bytes;')
