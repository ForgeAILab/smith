//! File-model settings for bounded web reads and search.

use serde::{Deserialize, Serialize};

/// Web-fetch tool settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebFetchSection {
    /// Whether fetch may reach private destinations. User scope only; defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_private_networks: Option<bool>,
}

/// Web-search tool settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchSection {
    /// Search route preference. Defaults to auto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<WebSearchMode>,
    /// Backend used when search runs outside the provider. User scope only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<WebSearchBackend>,
    /// Credential reference for a keyed backend, never an inline token. User scope only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<crate::credential::CredentialRef>,
    /// Absolute HTTP(S) URL of a SearXNG instance. User scope only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub searxng_url: Option<String>,
}

/// Preference used when composing the web-search route.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WebSearchMode {
    /// Prefer hosted search, then the configured backend, then DuckDuckGo.
    #[default]
    Auto,
    /// Require provider-hosted search.
    Hosted,
    /// Use the configured backend.
    Backend,
    /// Disable search.
    Off,
}

impl WebSearchMode {
    /// Parses the stable configuration spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "hosted" => Some(Self::Hosted),
            "backend" => Some(Self::Backend),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    /// The stable configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Hosted => "hosted",
            Self::Backend => "backend",
            Self::Off => "off",
        }
    }
}

/// Backend used for web search outside the provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WebSearchBackend {
    /// Free, keyless search.
    #[default]
    DuckDuckGo,
    /// Brave Search API.
    Brave,
    /// Tavily Search API.
    Tavily,
    /// A user-selected SearXNG instance.
    Searxng,
}

impl WebSearchBackend {
    /// Parses the stable configuration spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "duckduckgo" => Some(Self::DuckDuckGo),
            "brave" => Some(Self::Brave),
            "tavily" => Some(Self::Tavily),
            "searxng" => Some(Self::Searxng),
            _ => None,
        }
    }

    /// The stable configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "duckduckgo",
            Self::Brave => "brave",
            Self::Tavily => "tavily",
            Self::Searxng => "searxng",
        }
    }
}
