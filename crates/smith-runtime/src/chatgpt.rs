//! Experimental Smith-native ChatGPT OAuth and Responses provider.
//!
//! This is intentionally not an OpenAI Platform API adapter. It pins the
//! public Codex native-client OAuth parameters and the currently observed
//! ChatGPT Codex Responses endpoint behind Smith's experimental product
//! boundary. Smith owns the token bundle and direct HTTP calls; no Codex
//! executable or auth cache participates.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use agent_runtime::provider::sse::SseFrameParser;
use agent_runtime::provider::transport::{HttpRequest, HttpTransport};
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Clock, Deadline, SystemClock, Timestamp};
use agent_runtime_core::content::{ContentPart, Message, Role};
use agent_runtime_core::provider::{
    AuthKind, Capabilities, FinishReason, ModelDescriptor, ModelId, PromptCacheControl, Provider,
    ProviderCacheBehavior, ProviderCacheContract, ProviderCallContext, ProviderError,
    ProviderErrorKind, ProviderRequest, ProviderStream, ProviderStreamEvent, RateLimitSnapshot,
    RateLimitWindow, ReasoningSupport, ToolChoice,
};
use agent_runtime_core::provider_credential::{
    CredentialInvalidation, ProviderAuthRejection, ProviderCredentialError,
    ProviderCredentialLease, ProviderCredentialRecovery, ProviderCredentialRevision,
    ProviderCredentialSource, ProviderCredentialTarget,
};
use agent_runtime_core::store::Secret;
use agent_runtime_core::usage::{CounterKind, UsageDelta};
use async_stream::stream;
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use smith_config::credential::{CredentialEnroller, CredentialRef};
use url::form_urlencoded;
use zeroize::Zeroize;

use crate::journal::DefaultRedactor;
use crate::renewable::wait_for_deadline;
pub use crate::renewable::{BundleRefresher, RenewableBundle, RenewableCredentialSource};

mod decoding;
mod encoding;
mod oauth;
mod provider;
mod usage;

use decoding::{StreamState, decode_event, push_utf8};
use encoding::{response_items, response_tool_choice, response_tool_names};
#[cfg(test)]
use encoding::{response_wire_tool_name, valid_response_tool_name};
use usage::codex_rate_limit_snapshot;
#[cfg(test)]
use usage::usage_snapshot_from_json;

pub use oauth::{
    BrowserAuthorization, ChatGptOAuthClient, DeviceAuthorization, browser_authorization_url,
};
pub use provider::{ChatGptProvider, ChatGptProviderConfig};
pub use usage::fetch_usage_snapshot;

#[cfg(test)]
mod tests;

/// Fixed OAuth issuer used by the public Codex native client.
pub const CHATGPT_ISSUER: &str = "https://auth.openai.com";
/// Public native OAuth client identifier used by Codex.
pub const CHATGPT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Fixed direct Responses endpoint used by the experimental provider.
pub const CHATGPT_RESPONSES_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";
/// The account usage endpoint beside the Responses one, pinned from the
/// public Codex backend client (`/wham/usage` on `backend-api` hosts).
pub const CHATGPT_USAGE_ENDPOINT: &str = "https://chatgpt.com/backend-api/wham/usage";
/// Browser authorization scopes pinned from the public Codex implementation.
pub const CHATGPT_SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
/// Default remaining lifetime requested before a model call.
pub const CHATGPT_CREDENTIAL_MINIMUM_VALIDITY_MS: u64 = 30_000;
/// Maximum accepted OAuth response size.
const MAX_OAUTH_RESPONSE_BYTES: usize = 1024 * 1024;
/// Current protected token-bundle schema.
const TOKEN_BUNDLE_SCHEMA: u32 = 1;
/// Default token lifetime when the issuer omits both `expires_in` and JWT exp.
const DEFAULT_TOKEN_LIFETIME_MS: u64 = 60 * 60 * 1_000;

/// A fixed, redaction-safe ChatGPT authentication failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChatGptAuthError {
    /// The local OAuth client could not be constructed.
    #[error("the ChatGPT OAuth client could not be initialized")]
    Client,
    /// The issuer could not be reached or did not answer in time.
    #[error("the ChatGPT OAuth service is unavailable")]
    Transport,
    /// The issuer refused the selected ceremony.
    #[error("the ChatGPT OAuth request was rejected")]
    Rejected,
    /// Device authorization is unavailable for this account or workspace.
    #[error("ChatGPT device-code login is unavailable; use browser login")]
    DeviceUnavailable,
    /// The response exceeded the local bound.
    #[error("the ChatGPT OAuth response exceeded Smith's size limit")]
    ResponseTooLarge,
    /// The response did not carry a complete usable token set.
    #[error("the ChatGPT OAuth response was incompatible")]
    InvalidResponse,
    /// No bounded account identity could be extracted.
    #[error("the ChatGPT login did not identify a usable account")]
    MissingAccount,
    /// The protected token bundle is absent or malformed.
    #[error("the protected ChatGPT credential bundle is unusable; reconnect ChatGPT")]
    InvalidBundle,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenBundleWire {
    schema: u32,
    access_token: String,
    refresh_token: String,
    expires_at_ms: u64,
    account_id: String,
}

impl Drop for TokenBundleWire {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.account_id.zeroize();
    }
}

/// Smith's versioned renewable ChatGPT credential bundle.
///
/// Debug output is always redacted. Serialize it only through [`Self::to_secret`]
/// and persist that secret at Smith's fixed protected credential reference.
#[derive(Clone)]
pub struct ChatGptTokenBundle {
    access_token: String,
    refresh_token: String,
    expires_at_ms: u64,
    account_id: String,
}

impl fmt::Debug for ChatGptTokenBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChatGptTokenBundle([redacted])")
    }
}

impl Drop for ChatGptTokenBundle {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.account_id.zeroize();
    }
}

impl ChatGptTokenBundle {
    /// Parses a versioned bundle from protected storage.
    pub fn from_secret(secret: &Secret) -> Result<Self, ChatGptAuthError> {
        let wire: TokenBundleWire =
            serde_json::from_str(secret.expose()).map_err(|_| ChatGptAuthError::InvalidBundle)?;
        if wire.schema != TOKEN_BUNDLE_SCHEMA
            || wire.access_token.is_empty()
            || wire.refresh_token.is_empty()
            || !valid_account_id(&wire.account_id)
            || wire.expires_at_ms == 0
        {
            return Err(ChatGptAuthError::InvalidBundle);
        }
        Ok(Self {
            access_token: wire.access_token.clone(),
            refresh_token: wire.refresh_token.clone(),
            expires_at_ms: wire.expires_at_ms,
            account_id: wire.account_id.clone(),
        })
    }

    /// Serializes the complete bundle into a redaction-safe secret wrapper.
    pub fn to_secret(&self) -> Result<Secret, ChatGptAuthError> {
        let wire = TokenBundleWire {
            schema: TOKEN_BUNDLE_SCHEMA,
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            expires_at_ms: self.expires_at_ms,
            account_id: self.account_id.clone(),
        };
        serde_json::to_string(&wire)
            .map(Secret::new)
            .map_err(|_| ChatGptAuthError::InvalidBundle)
    }

    /// The non-renderable account identity needed at the provider wire boundary.
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    fn access_secret(&self) -> Secret {
        Secret::new(self.access_token.clone())
    }

    fn refresh_secret(&self) -> Secret {
        Secret::new(self.refresh_token.clone())
    }

    fn expires_at(&self) -> Timestamp {
        Timestamp(self.expires_at_ms)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    id_token: Option<String>,
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

impl Drop for TokenResponse {
    fn drop(&mut self) {
        if let Some(token) = &mut self.id_token {
            token.zeroize();
        }
        self.access_token.zeroize();
        if let Some(token) = &mut self.refresh_token {
            token.zeroize();
        }
    }
}

fn bundle_from_response(
    response: &TokenResponse,
    prior_refresh: Option<&Secret>,
    expected_account: Option<&str>,
) -> Result<ChatGptTokenBundle, ChatGptAuthError> {
    if response.access_token.is_empty() {
        return Err(ChatGptAuthError::InvalidResponse);
    }
    let refresh_token = response
        .refresh_token
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| prior_refresh.map(Secret::expose))
        .ok_or(ChatGptAuthError::InvalidResponse)?
        .to_owned();
    let account_id = response
        .id_token
        .as_deref()
        .and_then(extract_account_id)
        .or_else(|| extract_account_id(&response.access_token))
        .or_else(|| expected_account.map(str::to_owned))
        .filter(|value| valid_account_id(value))
        .ok_or(ChatGptAuthError::MissingAccount)?;
    if expected_account.is_some_and(|expected| expected != account_id) {
        return Err(ChatGptAuthError::MissingAccount);
    }
    let now = SystemClock.now().as_millis();
    let expires_at_ms = jwt_claims(&response.access_token)
        .and_then(|claims| claims.get("exp").and_then(Value::as_u64))
        .map(|seconds| seconds.saturating_mul(1_000))
        .or_else(|| {
            response
                .expires_in
                .map(|seconds| now.saturating_add(seconds.saturating_mul(1_000)))
        })
        .unwrap_or_else(|| now.saturating_add(DEFAULT_TOKEN_LIFETIME_MS));
    if expires_at_ms <= now {
        return Err(ChatGptAuthError::InvalidResponse);
    }
    Ok(ChatGptTokenBundle {
        access_token: response.access_token.clone(),
        refresh_token,
        expires_at_ms,
        account_id,
    })
}

fn jwt_claims(token: &str) -> Option<Value> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let _signature = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn extract_account_id(token: &str) -> Option<String> {
    let claims = jwt_claims(token)?;
    claims
        .get("chatgpt_account_id")
        .and_then(Value::as_str)
        .or_else(|| {
            claims
                .get("https://api.openai.com/auth")
                .and_then(|auth| auth.get("chatgpt_account_id"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            claims
                .get("organizations")
                .and_then(Value::as_array)
                .and_then(|organizations| organizations.first())
                .and_then(|organization| organization.get("id"))
                .and_then(Value::as_str)
        })
        .filter(|value| valid_account_id(value))
        .map(str::to_owned)
}

fn valid_account_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

impl RenewableBundle for ChatGptTokenBundle {
    type Error = ChatGptAuthError;

    fn from_secret(secret: &Secret) -> Result<Self, Self::Error> {
        ChatGptTokenBundle::from_secret(secret)
    }

    fn to_secret(&self) -> Result<Secret, Self::Error> {
        ChatGptTokenBundle::to_secret(self)
    }

    fn access_secret(&self) -> Secret {
        ChatGptTokenBundle::access_secret(self)
    }

    fn expires_at(&self) -> Timestamp {
        ChatGptTokenBundle::expires_at(self)
    }

    fn account(&self) -> Option<String> {
        Some(self.account_id.clone())
    }

    fn register_secrets(&self, redactor: &DefaultRedactor) {
        redactor.register_secret(&ChatGptTokenBundle::access_secret(self));
        redactor.register_secret(&self.refresh_secret());
    }

    fn revision_prefix() -> &'static str {
        "chatgpt"
    }
}

/// Single-flight renewable ChatGPT credential source backed by Smith's
/// owner-only auth file.
pub type ChatGptCredentialSource = RenewableCredentialSource<ChatGptTokenBundle>;
