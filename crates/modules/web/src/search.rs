use agent_runtime_core::store::Secret;
use futures_util::StreamExt;
use markup5ever_rcdom::Handle;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::json;
use smith_module::SearchBackendConfig;
use url::Url;

use crate::{BODY_LIMIT, TIMEOUT, fetch, html};

const DUCKDUCKGO: &str = "https://html.duckduckgo.com/html/";
const DUCKDUCKGO_LITE: &str = "https://lite.duckduckgo.com/lite/";
const BRAVE: &str = "https://api.search.brave.com/res/v1/web/search";
const TAVILY: &str = "https://api.tavily.com/search";
const DDG_BLOCKED: &str =
    "duckduckgo rate-limited the request or returned a challenge; try brave, tavily, or searxng";

#[derive(Debug, Clone, Copy)]
enum Backend {
    DuckDuckGo,
    Brave,
    Tavily,
    SearXng,
}

#[derive(Debug, Clone)]
pub(crate) struct SearchClient {
    backend: Backend,
    endpoint: Url,
    duckduckgo_lite_endpoint: Url,
    key: Option<Secret>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SearchResult {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) snippet: String,
}

impl SearchClient {
    pub(crate) fn new(config: SearchBackendConfig) -> Result<Option<Self>, String> {
        let (backend, endpoint, key) = match config {
            SearchBackendConfig::Disabled => return Ok(None),
            SearchBackendConfig::DuckDuckGo => (Backend::DuckDuckGo, DUCKDUCKGO.to_owned(), None),
            SearchBackendConfig::Brave { api_key } => (
                Backend::Brave,
                BRAVE.to_owned(),
                Some(required_key(api_key, "brave")?),
            ),
            SearchBackendConfig::Tavily { api_key } => (
                Backend::Tavily,
                TAVILY.to_owned(),
                Some(required_key(api_key, "tavily")?),
            ),
            SearchBackendConfig::SearXng { url } => {
                let mut url = fetch::validate_url(
                    &url.ok_or("searxng search unavailable: missing instance URL")?,
                )
                .map_err(|_| "searxng search unavailable: invalid instance URL".to_owned())?;
                url.set_query(None);
                let path = format!("{}/search", url.path().trim_end_matches('/'));
                url.set_path(&path);
                (Backend::SearXng, url.into(), None)
            }
        };
        Ok(Some(Self {
            backend,
            endpoint: fetch::validate_url(&endpoint)
                .map_err(|_| "invalid search endpoint".to_owned())?,
            duckduckgo_lite_endpoint: Url::parse(DUCKDUCKGO_LITE).expect("static search URL"),
            key,
        }))
    }

    pub(crate) fn name(&self) -> &'static str {
        match self.backend {
            Backend::DuckDuckGo => "duckduckgo",
            Backend::Brave => "brave",
            Backend::Tavily => "tavily",
            Backend::SearXng => "searxng",
        }
    }

    pub(crate) fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }

    #[cfg(test)]
    pub(crate) fn with_test_endpoint(mut self, endpoint: &str) -> Self {
        self.endpoint = fetch::validate_url(endpoint).unwrap();
        self
    }

    #[cfg(test)]
    pub(crate) fn with_test_duckduckgo_endpoints(mut self, html: &str, lite: &str) -> Self {
        self = self.with_test_endpoint(html);
        self.duckduckgo_lite_endpoint = fetch::validate_url(lite).unwrap();
        self
    }

    pub(crate) async fn search(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, String> {
        tokio::time::timeout(TIMEOUT, self.search_inner(query, max_results))
            .await
            .map_err(|_| {
                format!(
                    "{} search timed out after 30 seconds (HTTP status unavailable)",
                    self.name()
                )
            })?
    }

    async fn search_inner(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let builder = if matches!(self.backend, Backend::DuckDuckGo) {
            fetch::client_without_user_agent()
        } else {
            fetch::client()
        };
        let client = builder.build().map_err(|_| {
            format!(
                "{} search HTTP client unavailable (HTTP status unavailable)",
                self.name()
            )
        })?;
        let mut results = if matches!(self.backend, Backend::DuckDuckGo) {
            self.search_duckduckgo(&client, query).await?
        } else {
            self.search_json(&client, query, max_results).await?
        };
        results.retain(|result| fetch::validate_url(&result.url).is_ok());
        results.truncate(max_results.min(10));
        Ok(results)
    }

    async fn search_duckduckgo(
        &self,
        client: &reqwest::Client,
        query: &str,
    ) -> Result<Vec<SearchResult>, String> {
        let html_status = match duckduckgo_attempt(client, &self.endpoint, query, false).await {
            Ok(results) => return Ok(results),
            Err(status) => status,
        };
        duckduckgo_attempt(client, &self.duckduckgo_lite_endpoint, query, true)
            .await
            .map_err(|lite_status| {
                format!(
                    "{DDG_BLOCKED} (html {}, lite {})",
                    http_status(html_status),
                    http_status(lite_status)
                )
            })
    }

    async fn search_json(
        &self,
        client: &reqwest::Client,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let mut url = self.endpoint.clone();
        if !matches!(self.backend, Backend::Tavily) {
            url.query_pairs_mut().append_pair("q", query);
            match self.backend {
                Backend::Brave => {
                    url.query_pairs_mut()
                        .append_pair("count", &max_results.to_string());
                }
                Backend::SearXng => {
                    url.query_pairs_mut().append_pair("format", "json");
                }
                _ => {}
            }
        }
        let mut request = if matches!(self.backend, Backend::Tavily) {
            client
                .post(url)
                .header(CONTENT_TYPE, "application/json")
                .body(json!({"query": query, "max_results": max_results}).to_string())
        } else {
            client.get(url)
        };
        if let Some(key) = &self.key {
            let (header, value) = if matches!(self.backend, Backend::Brave) {
                ("X-Subscription-Token", key.expose().to_owned())
            } else {
                (AUTHORIZATION.as_str(), format!("Bearer {}", key.expose()))
            };
            let mut value = HeaderValue::from_str(&value).map_err(|_| {
                format!(
                    "{} search credential is not a valid HTTP header (HTTP status unavailable)",
                    self.name()
                )
            })?;
            value.set_sensitive(true);
            request = request.header(header, value);
        }
        // Never format reqwest errors or backend response bodies: either may
        // contain a credential, including a server that echoes its auth header.
        let response = request.send().await.map_err(|_| {
            format!(
                "{} search HTTP request failed (HTTP status unavailable)",
                self.name()
            )
        })?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(format!("{} search failed (HTTP {status})", self.name()));
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|_| format!("{} search body read failed (HTTP {status})", self.name()))?;
            if chunk.len() > BODY_LIMIT - bytes.len() {
                return Err(format!(
                    "{} search response exceeds 5 MiB (HTTP {status})",
                    self.name()
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        self.parse_json(&bytes).map_err(|_| {
            format!(
                "{} search returned invalid results (HTTP {status})",
                self.name()
            )
        })
    }

    fn parse_json(&self, bytes: &[u8]) -> Result<Vec<SearchResult>, serde_json::Error> {
        #[derive(Deserialize)]
        struct BraveResponse {
            web: BraveWeb,
        }
        #[derive(Deserialize)]
        struct BraveWeb {
            results: Vec<BraveResult>,
        }
        #[derive(Deserialize)]
        struct BraveResult {
            title: String,
            url: String,
            #[serde(default)]
            description: String,
        }
        #[derive(Deserialize)]
        struct JsonResponse {
            results: Vec<JsonResult>,
        }
        #[derive(Deserialize)]
        struct JsonResult {
            title: String,
            url: String,
            #[serde(default)]
            content: String,
        }
        if matches!(self.backend, Backend::Brave) {
            let response: BraveResponse = serde_json::from_slice(bytes)?;
            Ok(response
                .web
                .results
                .into_iter()
                .map(|result| SearchResult {
                    title: result.title,
                    url: result.url,
                    snippet: result.description,
                })
                .collect())
        } else {
            let response: JsonResponse = serde_json::from_slice(bytes)?;
            Ok(response
                .results
                .into_iter()
                .map(|result| SearchResult {
                    title: result.title,
                    url: result.url,
                    snippet: result.content,
                })
                .collect())
        }
    }
}

fn required_key(key: Option<Secret>, backend: &str) -> Result<Secret, String> {
    key.filter(|value| !value.expose().trim().is_empty())
        .ok_or_else(|| format!("{backend} search unavailable: missing API key"))
}

// Never include reqwest errors or response bodies in diagnostics, even for
// keyless search. Preserve only the HTTP status across the two attempts.
async fn duckduckgo_attempt(
    client: &reqwest::Client,
    endpoint: &Url,
    query: &str,
    lite: bool,
) -> Result<Vec<SearchResult>, Option<u16>> {
    let body = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("q", query);
        if !lite {
            form.append_pair("b", "");
        }
        form.finish()
    };
    let response = client
        .post(endpoint.clone())
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|_| None)?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(Some(status));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Some(status))?;
        if chunk.len() > BODY_LIMIT - bytes.len() {
            return Err(Some(status));
        }
        bytes.extend_from_slice(&chunk);
    }
    let source = String::from_utf8_lossy(&bytes);
    if lite {
        parse_duckduckgo_lite(&source)
    } else {
        parse_duckduckgo(&source)
    }
    .map_err(|_| Some(status))
}

fn http_status(status: Option<u16>) -> String {
    status.map_or_else(|| "HTTP status unavailable".into(), |s| format!("HTTP {s}"))
}

fn duckduckgo_challenge(document: &Handle) -> bool {
    html::descendants(document).any(|node| {
        html::has_class(&node, "anomaly-modal")
            || html::attribute(&node, "id").as_deref() == Some("anomaly-modal")
            || (html::is_element(&node, "form")
                && html::attribute(&node, "action")
                    .is_some_and(|action| action.contains("anomaly")))
    })
}

fn is_duckduckgo(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        let host = host.trim_end_matches('.');
        host == "duckduckgo.com" || host.ends_with(".duckduckgo.com")
    })
}

fn duckduckgo_target(href: &str) -> Option<Url> {
    let mut target = Url::parse(DUCKDUCKGO)
        .expect("static search URL")
        .join(href)
        .ok()?;
    if is_duckduckgo(&target) && target.path() == "/l/" {
        let decoded = target
            .query_pairs()
            .find(|(key, _)| key == "uddg")?
            .1
            .into_owned();
        target = fetch::validate_url(&decoded).ok()?;
    }
    if is_duckduckgo(&target) {
        return None;
    }
    Some(target)
}

fn parse_duckduckgo(source: &str) -> Result<Vec<SearchResult>, String> {
    let document = html::parse(source);
    if duckduckgo_challenge(&document)
        || !html::descendants(&document).any(|node| {
            html::attribute(&node, "id").as_deref() == Some("links")
                || ["results", "result", "no-results"]
                    .iter()
                    .any(|class| html::has_class(&node, class))
        })
    {
        return Err(DDG_BLOCKED.into());
    }
    let mut results = Vec::new();
    for result in html::descendants(&document).filter(|node| html::has_class(node, "result")) {
        let Some(link) = html::descendants(&result)
            .find(|node| html::is_element(node, "a") && html::has_class(node, "result__a"))
        else {
            continue;
        };
        let Some(target) = html::attribute(&link, "href").and_then(|href| duckduckgo_target(&href))
        else {
            continue;
        };
        results.push(SearchResult {
            title: html::text(&link),
            url: target.to_string(),
            snippet: html::descendants(&result)
                .find(|node| html::has_class(node, "result__snippet"))
                .map(|node| html::text(&node))
                .unwrap_or_default(),
        });
    }
    Ok(results)
}

fn parse_duckduckgo_lite(source: &str) -> Result<Vec<SearchResult>, String> {
    let document = html::parse(source);
    if duckduckgo_challenge(&document) {
        return Err(DDG_BLOCKED.into());
    }
    let mut results: Vec<SearchResult> = Vec::new();
    let mut pending_snippet = None;
    for node in html::descendants(&document) {
        if html::is_element(&node, "a") && html::has_class(&node, "result-link") {
            // Every link ends the previous result's snippet range, including
            // ads and invalid destinations that will be dropped.
            pending_snippet = None;
            if let Some(target) =
                html::attribute(&node, "href").and_then(|href| duckduckgo_target(&href))
            {
                results.push(SearchResult {
                    title: html::text(&node),
                    url: target.to_string(),
                    snippet: String::new(),
                });
                pending_snippet = Some(results.len() - 1);
            }
        } else if html::is_element(&node, "td")
            && html::has_class(&node, "result-snippet")
            && let Some(index) = pending_snippet.take()
        {
            results[index].snippet = html::text(&node);
        }
    }
    Ok(results)
}
