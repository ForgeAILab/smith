use agent_runtime_core::store::Secret;
use serde_json::json;

use super::{Fixture, Reply};
use crate::{SearchBackendConfig, search::SearchClient};

const KEY: &str = "fixture-secret-never-in-errors";

fn client(config: SearchBackendConfig, endpoint: &str) -> SearchClient {
    SearchClient::new(config)
        .unwrap()
        .unwrap()
        .with_test_endpoint(endpoint)
}

fn query(request: &str) -> std::collections::BTreeMap<String, String> {
    url::Url::parse(&format!(
        "http://fixture{}",
        request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
    ))
    .unwrap()
    .query_pairs()
    .map(|(key, value)| (key.into_owned(), value.into_owned()))
    .collect()
}

#[tokio::test]
async fn duckduckgo_fixture_unwraps_targets_and_normalizes_snippets_without_a_key() {
    let server = Fixture::new(vec![("/html/", Reply::new("text/html", r#"
        <div id="links" class="results">
          <div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Ftokio%2F%3Fa%3D1%26b%3Dtwo">Tokio <b>docs</b></a><a class="result__snippet">Cancellation <b>safety</b></a></div>
          <div class="result"><a class="result__a" href="https://example.com/next">Next</a><div class="result__snippet">More</div></div>
          <div class="result"><a class="result__a" href="javascript:alert(1)">Unsafe link</a></div>
        </div>"#))]).await;
    let client = client(SearchBackendConfig::DuckDuckGo, &server.url("/html/"));
    let results = client
        .search("tokio select & cancellation", 5)
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].title, "Tokio docs");
    assert_eq!(results[0].url, "https://docs.rs/tokio/?a=1&b=two");
    assert!(results[0].snippet.contains("Cancellation"));
    assert!(results[0].snippet.contains("safety"));
    let request = server.request().await;
    assert_eq!(query(&request)["q"], "tokio select & cancellation");
    assert!(request.starts_with("GET /html/?"));
    assert!(!request.to_lowercase().contains("authorization:"));
    assert!(!request.to_lowercase().contains("cookie:"));
}

#[tokio::test]
async fn duckduckgo_detects_challenges_non_200_and_layout_drift_but_accepts_no_results() {
    for reply in [
        Reply::new(
            "text/html",
            "<div class='results anomaly-modal'>challenge</div>",
        ),
        Reply::new(
            "text/html",
            "<div class='results' id='anomaly-modal'>challenge</div>",
        ),
        Reply::new(
            "text/html",
            "<div class='results'><form action='/anomaly/check'>challenge</form></div>",
        ),
        Reply {
            status: 202,
            ..Reply::new("text/html", "challenge")
        },
        Reply {
            status: 429,
            ..Reply::new("text/html", "blocked")
        },
        Reply::new("text/html", "<body>Changed layout</body>"),
    ] {
        let status = reply.status;
        let server = Fixture::new(vec![("/", reply)]).await;
        let error = client(SearchBackendConfig::DuckDuckGo, &server.url("/"))
            .search("rust", 5)
            .await
            .unwrap_err();
        for expected in [
            "duckduckgo",
            "rate-limited",
            "brave",
            "tavily",
            "searxng",
            &format!("HTTP {status}"),
        ] {
            assert!(error.contains(expected), "{error}");
        }
    }
    let server = Fixture::new(vec![(
        "/",
        Reply::new(
            "text/html",
            "<div class='results'><div class='no-results'>Nothing</div></div>",
        ),
    )])
    .await;
    assert!(
        client(SearchBackendConfig::DuckDuckGo, &server.url("/"))
            .search("nothing", 5)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn brave_uses_token_header_query_and_count_and_normalizes_description() {
    let body = json!({"web":{"results":(0..12).map(|i| json!({"title":format!("Title {i}"),"url":format!("https://example.com/{i}"),"description":"A snippet"})).collect::<Vec<_>>()}});
    let server = Fixture::new(vec![(
        "/search",
        Reply::new("application/json", body.to_string()),
    )])
    .await;
    let config = SearchBackendConfig::Brave {
        api_key: Some(Secret::new(KEY)),
    };
    let results = client(config, &server.url("/search"))
        .search("rust & web", 5)
        .await
        .unwrap();
    assert_eq!(results.len(), 5);
    assert_eq!(results[0].title, "Title 0");
    assert_eq!(results[0].url, "https://example.com/0");
    assert_eq!(results[0].snippet, "A snippet");
    let request = server.request().await;
    assert!(request.starts_with("GET /search?"));
    assert!(
        request
            .to_lowercase()
            .contains(&format!("x-subscription-token: {KEY}"))
    );
    assert_eq!(query(&request)["q"], "rust & web");
    assert_eq!(query(&request)["count"], "5");
    assert!(!request.lines().next().unwrap().contains(KEY));
}

#[tokio::test]
async fn tavily_posts_bearer_auth_and_json_and_normalizes_content() {
    let server = Fixture::new(vec![("/search", Reply::new("application/json", json!({"results":[{"title":"Tavily result","url":"https://example.com/tavily","content":"Tavily snippet"}]}).to_string()))]).await;
    let config = SearchBackendConfig::Tavily {
        api_key: Some(Secret::new(KEY)),
    };
    let results = client(config, &server.url("/search"))
        .search("query & stuff", 3)
        .await
        .unwrap();
    assert_eq!(results[0].snippet, "Tavily snippet");
    let request = server.request().await;
    assert!(request.starts_with("POST /search HTTP/1.1"));
    assert!(
        request
            .to_lowercase()
            .contains(&format!("authorization: bearer {KEY}"))
    );
    let body: serde_json::Value =
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body, json!({"query":"query & stuff","max_results":3}));
    assert!(!body.to_string().contains(KEY));
}

#[tokio::test]
async fn searxng_uses_instance_path_json_format_and_no_auth() {
    let server = Fixture::new(vec![("/instance/search", Reply::new("application/json", json!({"results":[{"title":"SearXNG result","url":"https://example.com/searxng","content":"SearXNG snippet"}]}).to_string()))]).await;
    let client = SearchClient::new(SearchBackendConfig::SearXng {
        url: Some(server.url("/instance/")),
    })
    .unwrap()
    .unwrap();
    let results = client.search("rust web", 5).await.unwrap();
    assert_eq!(results[0].title, "SearXNG result");
    assert_eq!(results[0].snippet, "SearXNG snippet");
    let request = server.request().await;
    assert!(request.starts_with("GET /instance/search?"));
    assert_eq!(query(&request)["q"], "rust web");
    assert_eq!(query(&request)["format"], "json");
    assert!(!request.to_lowercase().contains("authorization:"));
}

#[tokio::test]
async fn backend_errors_carry_name_and_status_without_echoing_credentials_or_bodies() {
    for (name, config) in [
        (
            "brave",
            SearchBackendConfig::Brave {
                api_key: Some(Secret::new(KEY)),
            },
        ),
        (
            "tavily",
            SearchBackendConfig::Tavily {
                api_key: Some(Secret::new(KEY)),
            },
        ),
        (
            "searxng",
            SearchBackendConfig::SearXng {
                url: Some("https://example.com/".into()),
            },
        ),
    ] {
        for status in [200, 401, 429, 500, 302] {
            let server = Fixture::new(vec![(
                "/",
                Reply {
                    status,
                    ..Reply::new("application/json", format!("secret echoed: {KEY}"))
                },
            )])
            .await;
            let client = client(config.clone(), &server.url("/"));
            assert!(!format!("{client:?}").contains(KEY));
            let error = client.search("rust", 5).await.unwrap_err();
            assert!(error.contains(name));
            assert!(error.contains(&format!("HTTP {status}")));
            assert!(!error.contains(KEY));
            assert!(!error.contains("secret echoed"));
        }
    }
    let config = SearchBackendConfig::Brave {
        api_key: Some(Secret::new("bad\nsecret")),
    };
    let error = client(config, "http://127.0.0.1:1/")
        .search("rust", 5)
        .await
        .unwrap_err();
    assert!(!error.contains("bad\nsecret"));
}

#[tokio::test]
#[ignore = "Live DuckDuckGo layout canary; run cargo test -p smith-module-web duckduckgo_live_drift -- --ignored --nocapture"]
async fn duckduckgo_live_drift() {
    let client = SearchClient::new(SearchBackendConfig::DuckDuckGo)
        .unwrap()
        .unwrap();
    let results = client
        .search("Rust programming language official documentation", 5)
        .await
        .expect("DuckDuckGo changed layout, challenged or rate-limited this request");
    assert!(
        !results.is_empty(),
        "DuckDuckGo returned no parsed results; review its HTML layout"
    );
    assert!(
        results
            .iter()
            .all(|result| result.url.starts_with("http") && !result.title.is_empty())
    );
}

#[tokio::test]
async fn search_is_bounded_at_ten_results_and_five_mib() {
    let body = json!({"results": (0..12).map(|index| json!({"title": format!("Title {index}"), "url": format!("https://example.com/{index}"), "content": "Snippet"})).collect::<Vec<_>>()});
    let server = Fixture::new(vec![(
        "/search",
        Reply::new("application/json", body.to_string()),
    )])
    .await;
    let config = SearchBackendConfig::Tavily {
        api_key: Some(Secret::new(KEY)),
    };
    assert_eq!(
        client(config.clone(), &server.url("/search"))
            .search("rust", 10)
            .await
            .unwrap()
            .len(),
        10
    );
    let server = Fixture::new(vec![(
        "/search",
        Reply::new("application/json", vec![b' '; crate::BODY_LIMIT + 1]),
    )])
    .await;
    let error = client(config, &server.url("/search"))
        .search("rust", 5)
        .await
        .unwrap_err();
    assert!(error.contains("tavily"));
    assert!(error.contains("HTTP 200"));
    assert!(error.contains("5 MiB"));
    assert!(!error.contains(KEY));
}
