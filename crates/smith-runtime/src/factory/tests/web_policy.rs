use super::*;

#[tokio::test]
async fn web_fetch_auto_approval_matches_one_prepared_host_only() {
    let mut rule = auto_rule(
        Vec::new(),
        vec![
            AutoApprovalPermission::NetHttp,
            AutoApprovalPermission::DataEgress,
        ],
        &[],
    );
    rule.tool = "smith/web_fetch".into();
    rule.resource = Some("web:docs.rs".into());
    rule.max_risk = AutoApprovalRisk::Low;
    let compiled = ScopedAutoApprovalRule::compile(&sourced(rule.clone())).unwrap();
    let call = |host: &str| {
        let endpoint = format!("https://{host}");
        let effects = ToolEffects::default()
            .with_network_to(&endpoint)
            .with_data_egress_to(&endpoint);
        PreparedToolCall::new(
            ToolCallId::new(host),
            "web_fetch",
            serde_json::json!({"url":format!("{endpoint}/serde/")}),
            effects.permission_upper_bound(),
            SecurityResource::other("web", host),
            effects,
            ToolCallDisplay::new("Web Fetch"),
        )
    };
    assert!(compiled.matches_and_consumes(&call("docs.rs"), "/repo"));
    assert!(!compiled.matches_and_consumes(&call("example.com"), "/repo"));
    let workspace = tempfile::tempdir().unwrap();
    let mut request = RuntimeRequest::new(resolved_config(), HostSurface::Headless);
    request.workspace = Some(Arc::new(ProjectWorkspace::new(workspace.path()).unwrap()));
    request.config.approval.auto.push(sourced(rule));
    let fallback = Arc::new(HeadlessApproval::new());
    request.approval = Some(fallback.clone());
    let policy = approval(&request).unwrap();
    let origin = || ApprovalOrigin::new(SessionId::new("s"), RequestId::new("r"));
    assert!(
        policy
            .decide(&ApprovalRequest::new(
                call("docs.rs"),
                Deadline::never(),
                origin()
            ))
            .await
            .is_allowed()
    );
    assert!(fallback.required().is_none());
    assert!(
        !policy
            .decide(&ApprovalRequest::new(
                call("example.com"),
                Deadline::never(),
                origin()
            ))
            .await
            .is_allowed()
    );
    assert_eq!(fallback.required().unwrap().tool, "web_fetch");
}

#[test]
fn web_posture_network_exception_rejects_writes_and_unreviewed_tools() {
    use agent_runtime_core::tool::ToolSpec;
    let effects = ToolEffects::default()
        .with_network_to("https://docs.rs")
        .with_data_egress_to("https://docs.rs");
    let spec = ToolSpec::new("web_fetch", "", serde_json::Value::Null, effects.clone());
    assert!(capabilities::read_only_network_extension(spec.clone()));
    assert!(!capabilities::read_only_extension(spec));
    let mcp = ToolSpec::new(
        "mcp__external__read",
        "",
        serde_json::Value::Null,
        effects.clone(),
    );
    assert!(!capabilities::read_only_extension(mcp));
    assert!(!capabilities::read_only_network_extension(ToolSpec::new(
        "web_fetch",
        "",
        serde_json::Value::Null,
        effects.with_write("x")
    )));
}
