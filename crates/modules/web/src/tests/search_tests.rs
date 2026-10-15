use agent_runtime_core::store::Secret;
use serde_json::json;

use super::{Fixture, Reply};
use crate::{SearchBackendConfig, search::SearchClient};

const KEY: &str = "fixture-secret-never-in-errors";
const HTML_CHALLENGE: &str = include_str!("fixtures/duckduckgo-html-challenge.html");
const LITE_CHALLENGE: &str = include_str!("fixtures/duckduckgo-lite-challenge.html");
const LITE_RESULTS: &str = include_str!("fixtures/duckduckgo-lite-results.html");

fn duckduckgo_client(server: &Fixture) -> SearchClient {
    SearchClient::new(SearchBackendConfig::DuckDuckGo)
        .unwrap()
        .unwrap()
        .with_test_duckduckgo_endpoints(&server.url("/html/"), &server.url("/lite/"))
}

fn assert_duckduckgo_post(request: &str, path: &str, query: &str, lite: bool) {
    assert_eq!(
        request.lines().next().unwrap(),
        format!("POST {path} HTTP/1.1")
    );
    let (headers, body) = request.split_once("\r\n\r\n").unwrap();
    let headers = headers.to_lowercase();
    assert!(headers.contains("content-type: application/x-www-form-urlencoded"));
    for forbidden in ["user-agent:", "authorization:", "cookie:"] {
        assert!(!headers.contains(forbidden), "{headers}");
    }
    let form: std::collections::BTreeMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(form["q"], query);
    if lite {
        assert_eq!(form.len(), 1);
    } else {
        assert_eq!(form.len(), 2);
        assert_eq!(form["b"], "");
    }
}

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
          <div class="result"><a class="result__a" href="https://duckduckgo.com/y.js?ad=fixture">Ad</a></div>
          <div class="result"><a class="result__a" href="https://html.duckduckgo.com/html/">Internal</a></div>
          <div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Flite.duckduckgo.com%2F">Redirected internal</a></div>
        </div>"#))]).await;
    let client = duckduckgo_client(&server);
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
    assert_duckduckgo_post(&request, "/html/", "tokio select & cancellation", false);
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
        let server = Fixture::new(vec![
            ("/html/", reply),
            (
                "/lite/",
                Reply {
                    status: 429,
                    ..Reply::new("text/html", LITE_CHALLENGE)
                },
            ),
        ])
        .await;
        let error = duckduckgo_client(&server)
            .search("rust", 5)
            .await
            .unwrap_err();
        for expected in [
            "duckduckgo",
            "rate-limited",
            "brave",
            "tavily",
            "searxng",
            &format!("html HTTP {status}"),
            "lite HTTP 429",
        ] {
            assert!(error.contains(expected), "{error}");
        }
        assert_duckduckgo_post(&server.request().await, "/html/", "rust", false);
        assert_duckduckgo_post(&server.request().await, "/lite/", "rust", true);
    }
    let server = Fixture::new(vec![(
        "/html/",
        Reply::new(
            "text/html",
            "<div class='results'><div class='no-results'>Nothing</div></div>",
        ),
    )])
    .await;
    assert!(
        duckduckgo_client(&server)
            .search("nothing", 5)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn duckduckgo_falls_back_to_lite_on_http_202_or_http_200_challenges() {
    for status in [202, 200] {
        let server = Fixture::new(vec![
            (
                "/html/",
                Reply {
                    status,
                    ..Reply::new("text/html", HTML_CHALLENGE)
                },
            ),
            ("/lite/", Reply::new("text/html", LITE_RESULTS)),
        ])
        .await;
        let results = duckduckgo_client(&server)
            .search("tokio select & café", 5)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "select in tokio - Rust - Docs.rs");
        assert_eq!(
            results[0].url,
            "https://docs.rs/tokio/latest/tokio/macro.select.html"
        );
        assert!(
            results[0]
                .snippet
                .starts_with("The macro aggregates all <async expression>")
        );
        assert!(
            results[0]
                .snippet
                .contains("branch's <handler> expression.")
        );
        assert_eq!(results[1].url, "https://tokio.rs/tokio/tutorial/select");
        assert!(
            results[1]
                .snippet
                .starts_with("The tokio::select ! macro returns")
        );
        assert_duckduckgo_post(
            &server.request().await,
            "/html/",
            "tokio select & café",
            false,
        );
        assert_duckduckgo_post(
            &server.request().await,
            "/lite/",
            "tokio select & café",
            true,
        );
    }
}

#[tokio::test]
async fn duckduckgo_reports_both_blocked_statuses_without_echoing_bodies() {
    for (status, body) in [
        (202, LITE_CHALLENGE),
        (200, LITE_CHALLENGE),
        (200, "<div class='anomaly-modal'>challenge</div>"),
        (200, "<div id='anomaly-modal'>challenge</div>"),
    ] {
        let server = Fixture::new(vec![
            (
                "/html/",
                Reply {
                    status: 202,
                    ..Reply::new("text/html", HTML_CHALLENGE)
                },
            ),
            (
                "/lite/",
                Reply {
                    status,
                    ..Reply::new("text/html", format!("{body}{KEY}"))
                },
            ),
        ])
        .await;
        let error = duckduckgo_client(&server)
            .search("rust", 5)
            .await
            .unwrap_err();
        for expected in [
            "duckduckgo rate-limited",
            "brave",
            "tavily",
            "searxng",
            "html HTTP 202",
            &format!("lite HTTP {status}"),
        ] {
            assert!(error.contains(expected), "{error}");
        }
        assert!(!error.contains(KEY));
        assert_duckduckgo_post(&server.request().await, "/html/", "rust", false);
        assert_duckduckgo_post(&server.request().await, "/lite/", "rust", true);
    }
}

#[tokio::test]
async fn duckduckgo_lite_unwraps_targets_filters_internal_links_and_bounds_snippets() {
    let server = Fixture::new(vec![
        ("/html/", Reply::new("text/html", HTML_CHALLENGE)),
        ("/lite/", Reply::new("text/html", r#"<table>
          <tr><td><a class='result-link' href='https://example.com/no-snippet'>No snippet</a></td></tr>
          <tr><td><a class='result-link' href='//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Ftokio'>Tokio <b>docs</b></a></td></tr>
          <tr><td class='result-snippet'>First <b>snippet</b></td></tr>
          <tr><td class='result-snippet'>Ignore extra snippet</td></tr>
          <tr><td><a class='result-link' href='https://example.org/next'>Next</a></td></tr>
          <tr><td><a class='result-link' href='https://ads.duckduckgo.com/y.js'>Ad</a></td></tr>
          <tr><td class='result-snippet'>Ad snippet</td></tr>
          <tr><td><a class='result-link' href='//duckduckgo.com/l/?uddg=https%3A%2F%2Fduckduckgo.com%2Fabout'>Internal redirect</a></td></tr>
          <tr><td><a class='result-link' href='javascript:alert(1)'>Unsafe</a></td></tr>
          <tr><td><a class='result-link' href='https://duckduckgo.com.example.org/'>External</a></td></tr>
          <tr><td class='result-snippet'>External snippet</td></tr>
        </table>"#)),
    ]).await;
    let results = duckduckgo_client(&server).search("rust", 5).await.unwrap();
    assert_eq!(results.len(), 4);
    assert_eq!(results[0].snippet, "");
    assert_eq!(results[1].title, "Tokio docs");
    assert_eq!(results[1].url, "https://docs.rs/tokio");
    assert_eq!(results[1].snippet, "First snippet");
    assert_eq!(results[2].snippet, "");
    assert_eq!(results[3].url, "https://duckduckgo.com.example.org/");
    assert_eq!(results[3].snippet, "External snippet");
}

#[tokio::test]
async fn duckduckgo_lite_accepts_a_non_challenge_page_with_no_links() {
    let server = Fixture::new(vec![
        ("/html/", Reply::new("text/html", HTML_CHALLENGE)),
        (
            "/lite/",
            Reply::new("text/html", "<body>No results found</body>"),
        ),
    ])
    .await;
    assert!(
        duckduckgo_client(&server)
            .search("nothing", 5)
            .await
            .unwrap()
            .is_empty()
    );
    assert_duckduckgo_post(&server.request().await, "/html/", "nothing", false);
    assert_duckduckgo_post(&server.request().await, "/lite/", "nothing", true);
}

#[tokio::test]
async fn duckduckgo_transport_failure_falls_back_and_preserves_available_statuses() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let unavailable = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    let server = Fixture::new(vec![
        (
            "/html/",
            Reply {
                status: 202,
                ..Reply::new("text/html", HTML_CHALLENGE)
            },
        ),
        ("/lite/", Reply::new("text/html", LITE_RESULTS)),
    ])
    .await;
    let client = || {
        SearchClient::new(SearchBackendConfig::DuckDuckGo)
            .unwrap()
            .unwrap()
    };
    let results = client()
        .with_test_duckduckgo_endpoints(&unavailable, &server.url("/lite/"))
        .search("rust", 5)
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_duckduckgo_post(&server.request().await, "/lite/", "rust", true);
    for (html, lite, expected) in [
        (
            unavailable.clone(),
            unavailable.clone(),
            "html HTTP status unavailable, lite HTTP status unavailable",
        ),
        (
            server.url("/html/"),
            unavailable.clone(),
            "html HTTP 202, lite HTTP status unavailable",
        ),
        (
            unavailable,
            server.url("/html/"),
            "html HTTP status unavailable, lite HTTP 202",
        ),
    ] {
        let error = client()
            .with_test_duckduckgo_endpoints(&html, &lite)
            .search("rust", 5)
            .await
            .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
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
        .expect("DuckDuckGo HTML and lite search both failed; inspect both HTTP statuses for challenges/rate limits and review the parsers for layout drift");
    assert!(
        !results.is_empty(),
        "DuckDuckGo returned no parsed results; review its HTML and lite layouts"
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
