use agent_runtime_core::store::Secret;
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::json;
use smith_module::SearchBackendConfig;
use url::Url;

use crate::{BODY_LIMIT, TIMEOUT, fetch, html};

const DUCKDUCKGO: &str = "https://html.duckduckgo.com/html/";
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
        let client = fetch::client().build().map_err(|_| {
            format!(
                "{} search HTTP client unavailable (HTTP status unavailable)",
                self.name()
            )
        })?;
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
        if matches!(self.backend, Backend::DuckDuckGo) && status != 200 {
            return Err(format!("{DDG_BLOCKED} (HTTP {status})"));
        }
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
        let mut results = match self.backend {
            Backend::DuckDuckGo => parse_duckduckgo(&String::from_utf8_lossy(&bytes))
                .map_err(|message| format!("{message} (HTTP {status})"))?,
            _ => self.parse_json(&bytes).map_err(|_| {
                format!(
                    "{} search returned invalid results (HTTP {status})",
                    self.name()
                )
            })?,
        };
        results.retain(|result| fetch::validate_url(&result.url).is_ok());
        results.truncate(max_results.min(10));
        Ok(results)
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

fn parse_duckduckgo(source: &str) -> Result<Vec<SearchResult>, String> {
    let document = html::parse(source);
    if html::descendants(&document).any(|node| {
        html::has_class(&node, "anomaly-modal")
            || html::attribute(&node, "id").as_deref() == Some("anomaly-modal")
            || (html::is_element(&node, "form")
                && html::attribute(&node, "action")
                    .is_some_and(|action| action.contains("anomaly")))
    }) || !html::descendants(&document).any(|node| {
        html::attribute(&node, "id").as_deref() == Some("links")
            || ["results", "result", "no-results"]
                .iter()
                .any(|class| html::has_class(&node, class))
    }) {
        return Err(DDG_BLOCKED.into());
    }
    let mut results = Vec::new();
    for result in html::descendants(&document).filter(|node| html::has_class(node, "result")) {
        let Some(link) = html::descendants(&result)
            .find(|node| html::is_element(node, "a") && html::has_class(node, "result__a"))
        else {
            continue;
        };
        let Some(href) = html::attribute(&link, "href") else {
            continue;
        };
        let Ok(mut target) = Url::parse(DUCKDUCKGO)
            .expect("static search URL")
            .join(&href)
        else {
            continue;
        };
        if target
            .host_str()
            .is_some_and(|host| host == "duckduckgo.com" || host == "www.duckduckgo.com")
            && target.path() == "/l/"
        {
            let decoded = target
                .query_pairs()
                .find(|(key, _)| key == "uddg")
                .map(|(_, value)| value.into_owned());
            let Some(decoded) = decoded else { continue };
            let Ok(url) = fetch::validate_url(&decoded) else {
                continue;
            };
            target = url;
        }
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
