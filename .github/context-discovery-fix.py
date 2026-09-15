from pathlib import Path

def edit(path, old, new):
    p=Path(path); s=p.read_text(); assert s.count(old)==1, (path,s.count(old),old[:90]); p.write_text(s.replace(old,new))

p='crates/smith-runtime/src/tool_output.rs'
edit(p, 'use smith_config::resolve::ResolvedConfig;\n', '''use smith_config::resolve::ResolvedConfig;

/// Dedicated retrieval affordance: a broad `read` query can rank the file reader first.
pub const ARTIFACT_DISCOVERY_QUERY: &str = "artifact-read";
''')
edit(p, '    sections.push("Read more with artifact.read; discover it with registry.search (query: artifact read) if absent.".into());', '    sections.push(format!("Read more with artifact.read; discover it with registry.search (query: {ARTIFACT_DISCOVERY_QUERY}) if absent."));')
p='crates/smith-runtime/tests/tool_output_context.rs'
edit(p, 'serde_json::json!({"query": "artifact read", "max_results": 1})', 'serde_json::json!({"query": smith_runtime::tool_output::ARTIFACT_DISCOVERY_QUERY, "max_results": 1})')
edit(p, '"discovery must advertise the reader at the next provider boundary"', '"discovery must advertise the reader at the next provider boundary: {:?}", resumed_requests[1].messages')
p='docs/context-working-set.md'
edit(p, 'with query `artifact read`;', 'with query `artifact-read`;')
p=Path('crates/smith-runtime/src/abilities.rs')
s=p.read_text(); end=s.rfind('\n}')
assert end>0 and s[end:].strip()=='}'
s=s[:end]+'''

    #[test]
    fn artifact_discovery_guidance_selects_the_artifact_reader_not_the_file_reader() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::session::SessionPaths::new(
            dir.path(), &crate::session::ProjectId::new("artifact-discovery").unwrap(),
        );
        let store = Arc::new(crate::artifact::SmithArtifactStore::new(paths));
        let policy = crate::tool_output::ToolOutputContextPolicy {
            inline_bytes: 8192, artifact_page_bytes: 2048,
        };
        let mut tools = built_in_and_agent_tools();
        tools.push(Arc::new(policy.reader(store)));
        let view = view_for(tools);
        let query = RoutingQuery::derive(
            crate::tool_output::ARTIFACT_DISCOVERY_QUERY, Vec::<String>::new(),
        );
        let candidates = CapabilityResolver::new().retrieve(&view, &query).candidates;
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].descriptor.id(), &RegistryId::tool(ARTIFACT_READ_TOOL_NAME));
        assert_eq!(selected_from(&view, crate::tool_output::ARTIFACT_DISCOVERY_QUERY),
            BTreeSet::from([RegistryId::tool(ARTIFACT_READ_TOOL_NAME)]));
    }
'''+s[end:]
p.write_text(s)
