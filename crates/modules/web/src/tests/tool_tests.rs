use std::sync::Arc;

use agent_runtime_core::cancel::{CancelReason, Cancellation};
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::content::ContentPart;
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId};
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::store::Secret;
use agent_runtime_core::tool::{InvocationContext, PreparationContext, Tool};
use agent_runtime_testkit::transport::ReplayTransport;
use serde_json::json;
use smith_module::{Module, ModuleContext, ModuleContribution, ModulePosture, Mounted};

use super::{Fixture, Reply};
use crate::{SearchBackendConfig, WebModule, WebSettings, search::SearchClient, tool};

fn context() -> ModuleContext {
    ModuleContext {
        settings: Default::default(),
        workspace_root: None,
        user_dir: "user".into(),
        is_child_session: false,
        posture: ModulePosture::ReadWrite,
        transport: Arc::new(ReplayTransport::single(Vec::new())),
        http: None,
        model: None,
        image_binding: None,
        vision: None,
        web: None,
        main_model_accepts_images: false,
        session_history: None,
        semantic_summary_enabled: false,
        max_input_tokens: 24_000,
        built_in_tools: true,
    }
}

fn preparation(root: &std::path::Path) -> PreparationContext {
    PreparationContext {
        session: SessionId::new("s"),
        turn: None,
        call_id: ToolCallId::new("c"),
        request: RequestId::new("r"),
        workspace: Arc::new(smith_host::ProjectWorkspace::new(root).unwrap()),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
    }
}

fn invocation(preparation: &PreparationContext) -> InvocationContext {
    InvocationContext {
        session: preparation.session.clone(),
        turn: preparation.turn.clone(),
        call_id: preparation.call_id.clone(),
        request: preparation.request.clone(),
        workspace: preparation.workspace.clone(),
        clock: preparation.clock.clone(),
        cancel: preparation.cancel.clone(),
        deadline: preparation.deadline,
        output_limit: 32_768,
    }
}

fn tool_names(context: &ModuleContext) -> Vec<String> {
    let Mounted::Contributions(contributions) = WebModule.mount(context).unwrap() else {
        panic!("active")
    };
    contributions
        .into_iter()
        .filter_map(|contribution| match contribution {
            ModuleContribution::Tool(tool) => Some(tool.spec().name),
            _ => None,
        })
        .collect()
}

#[test]
fn module_defaults_missing_credentials_disabled_route_and_inactive_reasons() {
    let mut context = context();
    assert!(WebModule.default_enabled());
    assert_eq!(WebModule.id(), "web");
    assert_eq!(tool_names(&context), ["web_fetch", "web_search"]);
    context.posture = ModulePosture::ReadOnly;
    assert_eq!(tool_names(&context), ["web_fetch", "web_search"]);
    for backend in [
        SearchBackendConfig::Disabled,
        SearchBackendConfig::Brave { api_key: None },
        SearchBackendConfig::Tavily { api_key: None },
        SearchBackendConfig::SearXng { url: None },
        SearchBackendConfig::SearXng {
            url: Some("file:///tmp/search".into()),
        },
    ] {
        context.web = Some(WebSettings {
            search: Some(backend),
            ..WebSettings::default()
        });
        assert_eq!(tool_names(&context), ["web_fetch"]);
    }
    context.web = Some(WebSettings {
        search: Some(SearchBackendConfig::Tavily { api_key: None }),
        ..WebSettings::default()
    });
    let Mounted::Contributions(contributions) = WebModule.mount(&context).unwrap() else {
        panic!("active fetch")
    };
    assert!(contributions.iter().any(|contribution| matches!(contribution, ModuleContribution::StatusItem { source, .. } if source.current().unwrap().label().contains("missing API key"))));
    context.web = Some(WebSettings {
        search: Some(SearchBackendConfig::Brave {
            api_key: Some(Secret::new("TOP SECRET")),
        }),
        ..WebSettings::default()
    });
    assert!(!format!("{context:?}").contains("TOP SECRET"));
    assert!(!format!("{:?}", context.web).contains("TOP SECRET"));
    context.built_in_tools = false;
    assert!(
        matches!(WebModule.mount(&context).unwrap(), Mounted::Inactive { reason } if reason == "built-in tools are disabled")
    );
}

#[tokio::test]
async fn preparation_validates_arguments_and_scopes_resources_and_effects() {
    let root = tempfile::tempdir().unwrap();
    let ctx = preparation(root.path());
    let fetch = tool::FetchTool {
        allow_private_networks: false,
    };
    for args in [
        json!({}),
        json!({"url":1}),
        json!({"url":"https://docs.rs/", "extra":true}),
        json!({"url":"file:///etc/passwd"}),
        json!({"url":"https://user:pass@docs.rs/"}),
    ] {
        assert!(fetch.prepare(args, &ctx).await.is_err());
    }
    let prepared = fetch
        .prepare(json!({"url":"https://docs.rs/serde/#top"}), &ctx)
        .await
        .unwrap();
    assert_eq!(
        prepared.resource(),
        &SecurityResource::other("web", "docs.rs")
    );
    assert_eq!(prepared.arguments()["url"], "https://docs.rs/serde/");
    assert_eq!(
        prepared
            .effects()
            .network_endpoints()
            .map(|endpoint| endpoint.as_str())
            .collect::<Vec<_>>(),
        ["https://docs.rs"]
    );
    assert_eq!(
        prepared
            .effects()
            .data_egress_destinations()
            .map(|endpoint| endpoint.as_str())
            .collect::<Vec<_>>(),
        ["https://docs.rs"]
    );
    assert!(!prepared.effects().has_filesystem_read());
    assert!(!prepared.effects().mutates());
    assert!(prepared.verify_fingerprint());
    let other = fetch
        .prepare(json!({"url":"https://example.com/"}), &ctx)
        .await
        .unwrap();
    assert_ne!(prepared.resource(), other.resource());

    let search = tool::SearchTool {
        client: SearchClient::new(SearchBackendConfig::DuckDuckGo)
            .unwrap()
            .unwrap(),
    };
    for args in [
        json!({}),
        json!({"query": ""}),
        json!({"query":" "}),
        json!({"query":"x".repeat(401)}),
        json!({"query":"rust", "max_results":0}),
        json!({"query":"rust", "max_results":11}),
        json!({"query":"rust", "max_results":1.5}),
        json!({"query":"rust", "max_results":null}),
        json!({"query":"rust", "extra":true}),
    ] {
        assert!(search.prepare(args, &ctx).await.is_err());
    }
    for args in [
        json!({"query":"rust"}),
        json!({"query":"x".repeat(400),"max_results":10}),
    ] {
        let prepared = search.prepare(args, &ctx).await.unwrap();
        assert_eq!(
            prepared.resource(),
            &SecurityResource::other("web-search", "duckduckgo")
        );
        assert!(prepared.effects().has_network());
        assert!(prepared.effects().has_data_egress());
        assert!(!prepared.effects().mutates());
        assert!(!prepared.effects().has_filesystem_read());
        assert!(prepared.verify_fingerprint());
    }
}

#[tokio::test]
async fn invocation_returns_structured_fetch_and_search_results_with_untrusted_labels() {
    let server = Fixture::new(vec![
        ("/fetch", Reply::new("text/plain", "documentation")),
        ("/search", Reply::new("application/json", r#"{"web":{"results":[{"title":"Docs","url":"https://example.com/","description":"Snippet"}]}}"#)),
    ]).await;
    let root = tempfile::tempdir().unwrap();
    let prepare = preparation(root.path());
    let invoke = invocation(&prepare);
    let fetch = tool::FetchTool {
        allow_private_networks: true,
    };
    let prepared = fetch
        .prepare(json!({"url":server.url("/fetch")}), &prepare)
        .await
        .unwrap();
    let outcome = fetch.invoke(prepared, &invoke).await.unwrap();
    assert!(!outcome.is_error);
    assert_eq!(outcome.value["status"], 200);
    assert!(
        matches!(&outcome.content.as_inline().unwrap()[0], ContentPart::Text { text } if text.contains("external, untrusted"))
    );
    let search = tool::SearchTool {
        client: SearchClient::new(SearchBackendConfig::Brave {
            api_key: Some(Secret::new("fixture")),
        })
        .unwrap()
        .unwrap()
        .with_test_endpoint(&server.url("/search")),
    };
    let prepared = search
        .prepare(json!({"query":"rust"}), &prepare)
        .await
        .unwrap();
    assert_eq!(
        prepared.resource(),
        &SecurityResource::other("web-search", "brave")
    );
    let outcome = search.invoke(prepared, &invoke).await.unwrap();
    assert!(!outcome.is_error);
    assert_eq!(outcome.value["results"][0]["snippet"], "Snippet");
    assert!(
        matches!(&outcome.content.as_inline().unwrap()[0], ContentPart::Text { text } if text.starts_with("1 results\n") && text.contains("external, untrusted"))
    );
}

#[tokio::test]
async fn fetch_errors_are_tool_outcomes_and_cancelled_calls_make_no_request() {
    let root = tempfile::tempdir().unwrap();
    let prepare = preparation(root.path());
    let mut invoke = invocation(&prepare);
    let fetch = tool::FetchTool {
        allow_private_networks: false,
    };
    let prepared = fetch
        .prepare(
            json!({"url":"http://169.254.169.254/latest/meta-data/"}),
            &prepare,
        )
        .await
        .unwrap();
    let outcome = fetch.invoke(prepared, &invoke).await.unwrap();
    assert!(outcome.is_error);
    assert!(outcome.value.as_str().unwrap().contains("private address"));
    let server = Fixture::new(vec![("/", Reply::new("text/plain", "should not fetch"))]).await;
    invoke.cancel = Cancellation::new();
    invoke.cancel.cancel(CancelReason::Host("test".into()));
    let prepared = fetch
        .prepare(json!({"url":server.url("/")}), &prepare)
        .await
        .unwrap();
    assert!(fetch.invoke(prepared, &invoke).await.unwrap().is_error);
    assert!(server.requests.lock().await.try_recv().is_err());
}
