//! Hosted-search rejection and backend fallback tests.

use super::*;

#[cfg(feature = "module-web")]
#[tokio::test]
async fn headless_hosted_search_rejection_is_not_retried_and_resume_uses_backend() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    std::fs::create_dir_all(project.path().join(".smith")).expect("configuration directory");
    std::fs::write(
        project.path().join(".smith/config.toml"),
        r#"
default_profile = "dev"
[profiles.dev]
provider = "local"
model = "example-model"
delegation = false
[providers.local]
kind = "fake"
[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#,
    )
    .expect("configuration");
    let mut config = resolve(
        &ResolveRequest::new(project.path())
            .with_known_modules(crate::modules::known_modules())
            .with_home_dir(home.path()),
    )
    .expect("configuration resolves")
    .config;
    config.persistence.enabled.value = true;
    let request = |provider: Arc<FakeProvider>| {
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("workspace"),
            )),
            approval: Some(Arc::new(AllowAll)),
            provider: Some(provider),
            modules: crate::modules::composition_from_resolved_config(&config),
            ..RuntimeRequest::new(config.clone(), HostSurface::Headless)
        };
        host_request(runtime, project.path())
    };
    let mut capabilities = Capabilities::basic_streaming();
    capabilities.hosted_web_search = true;
    let rejected = Arc::new(FakeProvider::new(
        "example-model",
        capabilities.clone(),
        vec![ScriptedStream::new(vec![ProviderStreamEvent::Error {
            error: ProviderError::new(
                ProviderErrorKind::Unsupported,
                "hosted web search is not enabled for this account",
            ),
        }])],
    ));
    let host = smith_runtime::host::start(request(rejected.clone()))
        .await
        .expect("host");
    let session = host.session().id().clone();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = tokio::time::timeout(
        HEADLESS_TEST_WATCHDOG,
        run_with_io(
            &host,
            "research".into(),
            OutputFormat::Json,
            HeadlessBrokers::default(),
            BackgroundExit::Error,
            &mut stdout,
            &mut stderr,
        ),
    )
    .await
    .expect("headless watchdog")
    .expect("headless failure");
    assert_ne!(outcome.exit_code, 0);
    assert_eq!(rejected.requests().len(), 1, "headless does not retry");
    assert_eq!(rejected.requests()[0].hosted_tools.len(), 1);
    assert_eq!(
        String::from_utf8(stderr)
            .expect("notice")
            .matches("unavailable for this organization")
            .count(),
        1
    );
    host.shutdown().await.expect("shutdown");

    let mut activate = tool_call_fragments(
        0,
        "activate",
        "registry.activate",
        r#"{"ids":["tool:web_search"]}"#,
    );
    activate.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let resumed_provider = Arc::new(FakeProvider::new(
        "example-model",
        capabilities,
        vec![
            ScriptedStream::new(activate),
            ScriptedStream::new(vec![ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            }]),
        ],
    ));
    // A fresh RuntimeRequest has no shared flag from the first invocation.
    let host = smith_runtime::host::start(request(resumed_provider.clone()).resume(session))
        .await
        .expect("resumed headless host");
    let outcome = tokio::time::timeout(
        HEADLESS_TEST_WATCHDOG,
        run_with_io(
            &host,
            "research again".into(),
            OutputFormat::Json,
            HeadlessBrokers::default(),
            BackgroundExit::Error,
            &mut Vec::new(),
            &mut Vec::new(),
        ),
    )
    .await
    .expect("resume watchdog")
    .expect("backend turn");
    assert_eq!(outcome.exit_code, 0);
    let requests = resumed_provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.hosted_tools.is_empty())
    );
    assert!(
        requests[1]
            .tools
            .iter()
            .any(|tool| tool.name == "web_search")
    );
    host.shutdown().await.expect("shutdown");
}
