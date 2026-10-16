use super::*;

fn web_request(workspace: &std::path::Path) -> RuntimeRequest {
    let mut request = request(
        workspace,
        vec![CompiledModule {
            module: Arc::new(smith_module_web::WebModule),
            origin: ModuleOrigin::FirstParty,
        }],
    );
    request.built_in_tools = true;
    request
}

#[tokio::test]
async fn web_auto_recomposition_switches_chatgpt_hosted_to_glm_backend() {
    for (kind, hosted) in [
        (KIND_CHATGPT_RESPONSES, true),
        (KIND_OPENAI_COMPATIBLE, false),
    ] {
        let workspace = tempfile::tempdir().unwrap();
        let mut request = web_request(workspace.path());
        request.config.provider = provider(kind, Some("https://example.test/v1"));
        let mut capabilities = Capabilities::basic_streaming();
        capabilities.hosted_web_search = hosted;
        let mut activate = agent_runtime::provider::fake::tool_call_fragments(
            0,
            "activate",
            "registry.activate",
            r#"{"ids":["tool:web_search"]}"#,
        );
        activate.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        let reply = || {
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "done".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])
        };
        let fake = Arc::new(FakeProvider::new(
            "example-model",
            capabilities,
            if hosted {
                vec![reply()]
            } else {
                vec![ScriptedStream::new(activate), reply()]
            },
        ));
        request.provider = Some(fake.clone());
        let runtime =
            build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
                .await
                .unwrap();
        assert_eq!(
            runtime
                .policy()
                .tools
                .iter()
                .any(|name| name == "web_search"),
            !hosted
        );
        assert!(
            runtime
                .policy()
                .tools
                .iter()
                .any(|name| name == "web_fetch")
        );
        let session = runtime
            .runtime()
            .start_session(StartSession::new())
            .await
            .unwrap();
        session
            .send(UserInput::text("research"))
            .unwrap()
            .completed()
            .await;
        session.shutdown().await.unwrap();
        let requests = fake.requests();
        assert_eq!(!requests[0].hosted_tools.is_empty(), hosted);
        if hosted {
            assert!(
                !requests[0]
                    .tools
                    .iter()
                    .any(|tool| tool.name == "web_search")
            );
        } else {
            assert!(
                requests[1]
                    .tools
                    .iter()
                    .any(|tool| tool.name == "web_search")
            );
            assert!(
                requests
                    .iter()
                    .all(|request| request.hosted_tools.is_empty())
            );
        }
    }
}

#[tokio::test]
async fn web_composition_routing_modes_and_fallback_keep_fetch() {
    use smith_config::model::{WebSearchBackend, WebSearchMode};
    for (mode, capable, fallback, expected_hosted, expected_backend) in [
        (WebSearchMode::Auto, true, false, true, false),
        (WebSearchMode::Auto, false, false, false, true),
        (WebSearchMode::Auto, true, true, false, true),
        (WebSearchMode::Hosted, true, false, true, false),
        (WebSearchMode::Hosted, false, false, false, false),
        (WebSearchMode::Hosted, true, true, false, false),
        (WebSearchMode::Backend, true, false, false, true),
        (WebSearchMode::Off, true, false, false, false),
    ] {
        let workspace = tempfile::tempdir().unwrap();
        let mut request = web_request(workspace.path());
        request.config.web.search.mode.value = mode;
        if fallback {
            request.web_search_session.record_error(&ProviderError::new(
                agent_runtime_core::provider::ProviderErrorKind::Unsupported,
                "hosted web search is not enabled for this account",
            ));
        }
        let route = crate::factory::web::prepare(&request, capable).await;
        assert_eq!(!route.hosted.is_empty(), expected_hosted);
        assert_eq!(
            matches!(
                route.settings.search,
                Some(smith_module::SearchBackendConfig::DuckDuckGo)
            ),
            expected_backend
        );
    }
    let workspace = tempfile::tempdir().unwrap();
    let mut request = web_request(workspace.path());
    request.config.web.search.mode.value = WebSearchMode::Backend;
    request.config.web.search.backend.value = WebSearchBackend::Brave;
    request.config.web.search.api_key = Some(sourced(
        CredentialRef::parse("env:MISSING_WEB_KEY").unwrap(),
    ));
    // No resolver: failure is deterministic and never opens the host keychain.
    let runtime =
        build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
            .await
            .unwrap();
    assert!(
        runtime
            .policy()
            .tools
            .iter()
            .any(|name| name == "web_fetch")
    );
    assert!(
        !runtime
            .policy()
            .tools
            .iter()
            .any(|name| name == "web_search")
    );
    assert!(!runtime.module_status().is_empty());
}

#[tokio::test]
async fn web_installed_agent_turns_route_search_to_the_backend() {
    use smith_config::model::WebSearchMode;
    use smith_config::resolve::{ResolvedHarness, Source, Sourced};
    // The provider here only names the model; the CLI that runs the turn
    // never sees hosted tools, so its search must be Smith's backend tool.
    for (mode, expected_backend) in [
        (WebSearchMode::Auto, true),
        (WebSearchMode::Hosted, false),
        (WebSearchMode::Backend, true),
    ] {
        let workspace = tempfile::tempdir().unwrap();
        let mut request = web_request(workspace.path());
        request.config.web.search.mode.value = mode;
        request.config.harness = Some(ResolvedHarness {
            kind: Sourced::new("claude-code".to_owned(), Source::built_in("model")),
            program: "claude".to_owned(),
            model: Sourced::new("opus".to_owned(), Source::built_in("model")),
            executable: None,
            args: Vec::new(),
            allow_own_tools: Sourced::new(false, Source::built_in("harness.allow_own_tools")),
            env: Default::default(),
        });
        let route = crate::factory::web::prepare(&request, true).await;
        assert!(route.hosted.is_empty(), "{mode:?}");
        assert_eq!(
            matches!(
                route.settings.search,
                Some(smith_module::SearchBackendConfig::DuckDuckGo)
            ),
            expected_backend,
            "{mode:?}"
        );
        assert_eq!(
            route.diagnostic.is_some(),
            mode == WebSearchMode::Hosted,
            "{mode:?}"
        );
    }
}

#[tokio::test]
async fn web_plan_posture_default_and_strict_module_lists() {
    for allowed in [true, false] {
        let workspace = tempfile::tempdir().unwrap();
        let mut request = web_request(workspace.path());
        request.config.agent = agent(AgentPosture::Plan);
        if !allowed {
            request.config.agent.read_only_network_modules.value.clear();
        }
        let runtime =
            build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
                .await
                .unwrap();
        assert_eq!(
            runtime
                .policy()
                .tools
                .iter()
                .any(|name| name == "web_fetch"),
            allowed
        );
        assert!(!runtime.policy().tools.iter().any(|name| name == "edit"));
    }
}

#[tokio::test]
async fn web_child_profiles_prepare_their_own_backend_route() {
    let workspace = tempfile::tempdir().unwrap();
    let mut request = web_request(workspace.path());
    request.config.provider = provider(KIND_CHATGPT_RESPONSES, Some("https://example.test/v1"));
    let mut child_config = request.config.clone();
    child_config.agent.profile.name = "glm-child".into();
    child_config.provider = provider(KIND_OPENAI_COMPATIBLE, Some("https://glm.example/v1"));
    child_config.provider.api_key = Some(sourced(Secret::new("child-fixture-token")));
    request.child_profiles.push(ChildProfileRequest {
        config: child_config,
        catalog_sources: Vec::new(),
        vision_profile: None,
    });
    let routes = crate::factory::provider::prepare_child_profile_routes(
        &request,
        None,
        Arc::new(crate::session_history::LiveSessionHistory::default()),
        Arc::new(crate::vision::VisionAccounting::default()),
    )
    .await
    .unwrap();
    let route = routes.values().next().unwrap();
    assert!(route.hosted_tools.is_empty());
    assert!(matches!(
        route.module_context.web.as_ref().unwrap().search,
        Some(smith_module::SearchBackendConfig::DuckDuckGo)
    ));
}

#[tokio::test]
async fn web_account_disabled_event_changes_the_next_composition() {
    let workspace = tempfile::tempdir().unwrap();
    let mut request = web_request(workspace.path());
    request.config.provider = provider(KIND_CHATGPT_RESPONSES, None);
    let state = request.web_search_session.clone();
    let mut capabilities = Capabilities::basic_streaming();
    capabilities.hosted_web_search = true;
    request.provider = Some(Arc::new(FakeProvider::new(
        "example-model",
        capabilities,
        vec![ScriptedStream::new(vec![ProviderStreamEvent::Error {
            error: ProviderError::new(
                agent_runtime_core::provider::ProviderErrorKind::Unsupported,
                "hosted web search is not enabled for this account",
            ),
        }])],
    )));
    let runtime =
        build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
            .await
            .unwrap();
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session
        .send(UserInput::text("research"))
        .unwrap()
        .completed()
        .await;
    session.shutdown().await.unwrap();
    assert!(state.unavailable());
    let mut next = web_request(workspace.path());
    next.web_search_session = state;
    let route = crate::factory::web::prepare(&next, true).await;
    assert!(route.hosted.is_empty());
    assert!(matches!(
        route.settings.search,
        Some(smith_module::SearchBackendConfig::DuckDuckGo)
    ));
}

#[tokio::test]
async fn web_inherited_child_requests_declare_hosted_search() {
    use agent_runtime::delegation::ChildRuntimeFactory;
    use agent_runtime_core::delegation::{
        ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
    };
    let workspace = tempfile::tempdir().unwrap();
    let mut request = web_request(workspace.path());
    request.config.provider = provider(KIND_CHATGPT_RESPONSES, None);
    request.config.agent.profile.delegation.value = true;
    let mut capabilities = Capabilities::basic_streaming();
    capabilities.hosted_web_search = true;
    let fake = Arc::new(FakeProvider::new(
        "example-model",
        capabilities,
        vec![ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: "done".into(),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])],
    ));
    request.provider = Some(fake.clone());
    let runtime =
        build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
            .await
            .unwrap();
    let spec = ChildSpec {
        task: UserInput::text("research"),
        model: ChildModelSelection::Inherit,
        limits: ChildLimits {
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        },
        tools: ToolViewScope::All,
        workspace: WorkspacePolicy::ReadOnlyView,
    };
    let child_runtime = runtime
        .delegation()
        .unwrap()
        .factory
        .child_builder(&spec)
        .unwrap()
        .build()
        .unwrap();
    let child = child_runtime
        .start_session(StartSession::new())
        .await
        .unwrap();
    child.send(spec.task).unwrap().completed().await;
    child.shutdown().await.unwrap();
    assert_eq!(fake.requests()[0].hosted_tools.len(), 1);
    assert!(
        !fake.requests()[0]
            .tools
            .iter()
            .any(|tool| tool.name == "web_search")
    );
}

#[tokio::test]
async fn web_settings_resolve_backend_credentials_and_private_fetch_policy() {
    #[derive(Debug)]
    struct WebEnvironment;
    impl smith_config::credential::Environment for WebEnvironment {
        fn value(&self, name: &str) -> Option<Secret> {
            (name == "WEB_FIXTURE").then(|| Secret::new("web-fixture-secret"))
        }
    }
    let workspace = tempfile::tempdir().unwrap();
    let mut request = web_request(workspace.path());
    request.config.web.fetch.allow_private_networks.value = true;
    request.config.web.search.api_key =
        Some(sourced(CredentialRef::parse("env:WEB_FIXTURE").unwrap()));
    request.credentials =
        Some(CredentialResolver::new(workspace.path()).with_environment(Arc::new(WebEnvironment)));
    for backend in [
        smith_config::model::WebSearchBackend::Brave,
        smith_config::model::WebSearchBackend::Tavily,
    ] {
        request.config.web.search.backend.value = backend;
        let route = crate::factory::web::prepare(&request, false).await;
        assert!(route.settings.allow_private_networks);
        let key = match route.settings.search.unwrap() {
            smith_module::SearchBackendConfig::Brave { api_key }
            | smith_module::SearchBackendConfig::Tavily { api_key } => api_key.unwrap(),
            other => panic!("unexpected backend: {other:?}"),
        };
        assert_eq!(key.expose(), "web-fixture-secret");
    }
    request.config.web.search.backend.value = smith_config::model::WebSearchBackend::Searxng;
    request.config.web.search.searxng_url = Some(sourced("https://search.example".into()));
    let route = crate::factory::web::prepare(&request, false).await;
    assert!(
        matches!(route.settings.search, Some(smith_module::SearchBackendConfig::SearXng { url: Some(url) }) if url == "https://search.example")
    );
}
