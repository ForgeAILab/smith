from pathlib import Path

def edit(path, old, new):
    p=Path(path); s=p.read_text(); assert s.count(old)==1, (path,s.count(old),old[:90]); p.write_text(s.replace(old,new))

p='crates/smith-runtime/src/tool_output.rs'
edit(p, '    sections.push(prefix(&metadata, limit / 4));', '''    sections.push(prefix(&metadata, limit / 4));
    sections.push("Read more with artifact.read; discover it with registry.search (query: artifact read) if absent.".into());''')
edit(p, '        let prep = preparation();', '''        let prep = preparation();
        let stale = ArtifactReadTool::new(reopened.clone())
            .prepare(json!({"id": reference.id.as_str(), "limit": 65536}), &prep)
            .await.unwrap();
        assert!(reader.invoke(stale, &invocation(&prep)).await.is_err(),
            "an old prepared call must not bypass the current cap");''')
p='crates/smith-runtime/tests/tool_output_context.rs'
edit(p, '''        vec![
            tool_stream(
                "read-after-restart",''', '''        vec![
            tool_stream(
                "discover-after-restart",
                "registry.search",
                serde_json::json!({"query": "artifact read", "max_results": 1}),
            ),
            tool_stream(
                "read-after-restart",''')
edit(p, '''    assert_eq!(resumed_requests.len(), 2);
    let page = resumed_requests[1]''', '''    assert_eq!(resumed_requests.len(), 3);
    assert!(!resumed_requests[0].tools.iter().any(|tool| tool.name == "artifact.read"),
        "the fixture must exercise actual lazy discovery, not an eagerly exposed tool");
    assert!(resumed_requests[1].tools.iter().any(|tool| tool.name == "artifact.read"),
        "discovery must advertise the reader at the next provider boundary");
    let page = resumed_requests[2]''')
edit(p, '    assert!(!page.is_error);', '    assert!(!page.is_error, "artifact tool failed: {page:?}");')
edit(p, '    assert!(final_wire.contains("MIDDLE_DIAGNOSTIC_EXCERPT"));', '''    assert!(final_wire.contains("MIDDLE_DIAGNOSTIC_EXCERPT"));
    assert!(final_wire.contains("discover it with registry.search"));''')
p='docs/context-working-set.md'
edit(p, 'Follow `next_offset` to inspect more evidence rather than loading everything.', '''If `artifact.read` is not in the current tool list, first call `registry.search`
with query `artifact read`; the next provider request advertises the authorized
reader. Previews include this guidance rather than eagerly activating every tool.
Follow `next_offset` to inspect more evidence rather than loading everything.''')
