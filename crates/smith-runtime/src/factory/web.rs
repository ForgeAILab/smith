//! Search routing is re-evaluated for each provider composition.

use std::sync::atomic::{AtomicBool, Ordering};

use agent_runtime_core::event::{EventEnvelope, RuntimeEvent};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::provider::{HostedTool, ProviderError, ProviderErrorKind};
use smith_config::model::{WebSearchBackend, WebSearchMode};
use smith_module::{SearchBackendConfig, WebSettings};

use super::RuntimeRequest;

/// Session-local memory of a provider rejecting hosted search.
///
/// Hosts retain this handle across runtime rebuilds. A rejected turn is never
/// replayed automatically; the next safe composition selects the backend.
#[derive(Debug, Default)]
pub struct WebSearchSession {
    unavailable: AtomicBool,
}

impl WebSearchSession {
    /// Whether this session has encountered an account-disabled hosted search.
    pub fn unavailable(&self) -> bool {
        self.unavailable.load(Ordering::Relaxed)
    }

    /// Records only the provider's classified account-disabled search error.
    /// Returns true for the first such rejection in this session.
    pub fn record_error(&self, error: &ProviderError) -> bool {
        error.kind == ProviderErrorKind::Unsupported
            && error.message == "hosted web search is not enabled for this account"
            && !self.unavailable.swap(true, Ordering::Relaxed)
    }
}

impl EventObserver for WebSearchSession {
    fn observe(&self, event: &EventEnvelope) {
        if let RuntimeEvent::ProviderAttemptFinished {
            error: Some(error), ..
        } = &event.payload
        {
            self.record_error(error);
        }
    }
}

pub(super) struct WebRoute {
    pub(super) settings: WebSettings,
    pub(super) hosted: Vec<HostedTool>,
    pub(super) diagnostic: Option<String>,
}

fn hosted(mode: WebSearchMode, capable: bool, unavailable: bool) -> bool {
    capable && !unavailable && matches!(mode, WebSearchMode::Auto | WebSearchMode::Hosted)
}

pub(super) async fn prepare(request: &RuntimeRequest, capable: bool) -> WebRoute {
    let config = &request.config;
    let enabled = request.built_in_tools
        && request.modules.enabled.contains("web")
        && request.modules.compiled.iter().any(|module| {
            module.module.id() == "web"
                && matches!(module.origin, smith_module::ModuleOrigin::FirstParty)
        });
    let allowed = enabled
        && (!config.agent.active_posture().is_read_only()
            || config
                .agent
                .read_only_network_modules
                .value
                .iter()
                .any(|id| id == "web"));
    let mode = config.web.search.mode.value;
    let use_hosted = allowed && hosted(mode, capable, request.web_search_session.unavailable());
    let backend =
        allowed && (mode == WebSearchMode::Backend || (mode == WebSearchMode::Auto && !use_hosted));
    let diagnostic = (allowed && mode == WebSearchMode::Hosted && !use_hosted).then(|| {
        "tools.web_search.mode: hosted web search is unavailable for this provider or account"
            .to_owned()
    });
    if let Some(reason) = &diagnostic {
        tracing::warn!(%reason);
    }
    let search = if backend {
        backend_settings(request).await
    } else {
        SearchBackendConfig::Disabled
    };
    WebRoute {
        diagnostic,
        settings: WebSettings {
            allow_private_networks: config.web.fetch.allow_private_networks.value,
            search: Some(search),
        },
        hosted: if use_hosted {
            vec![HostedTool::WebSearch { max_uses: None }]
        } else {
            Vec::new()
        },
    }
}

async fn backend_settings(request: &RuntimeRequest) -> SearchBackendConfig {
    let search = &request.config.web.search;
    let api_key = if matches!(
        search.backend.value,
        WebSearchBackend::Brave | WebSearchBackend::Tavily
    ) {
        match &search.api_key {
            Some(reference) => {
                match super::provider::credentials::secret(request, &reference.value.to_string())
                    .await
                {
                    Ok(secret) => {
                        if let Some(redactor) = &request.persistence_redactor {
                            redactor.register_secret(&secret);
                        }
                        Some(secret)
                    }
                    Err(_) => {
                        // Do not echo a resolver error or its credential locator.
                        tracing::warn!(
                            "tools.web_search.api_key: credential resolution failed; backend search is unavailable"
                        );
                        None
                    }
                }
            }
            None => None,
        }
    } else {
        None
    };
    match search.backend.value {
        WebSearchBackend::DuckDuckGo => SearchBackendConfig::DuckDuckGo,
        WebSearchBackend::Brave => SearchBackendConfig::Brave { api_key },
        WebSearchBackend::Tavily => SearchBackendConfig::Tavily { api_key },
        WebSearchBackend::Searxng => SearchBackendConfig::SearXng {
            url: search.searxng_url.as_ref().map(|url| url.value.clone()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_routing_table_and_session_fallback() {
        for (mode, capable, unavailable, expected) in [
            (WebSearchMode::Auto, true, false, true),
            (WebSearchMode::Auto, false, false, false),
            (WebSearchMode::Auto, true, true, false),
            (WebSearchMode::Hosted, true, false, true),
            (WebSearchMode::Hosted, false, false, false),
            (WebSearchMode::Hosted, true, true, false),
            (WebSearchMode::Backend, true, false, false),
            (WebSearchMode::Off, true, false, false),
        ] {
            assert_eq!(hosted(mode, capable, unavailable), expected);
        }
        let state = WebSearchSession::default();
        assert!(!state.record_error(&ProviderError::new(
            ProviderErrorKind::Server,
            "hosted web search is not enabled for this account"
        )));
        let error = ProviderError::new(
            ProviderErrorKind::Unsupported,
            "hosted web search is not enabled for this account",
        );
        assert!(state.record_error(&error));
        assert!(state.unavailable());
        assert!(!state.record_error(&error));
    }
}
