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
    ProviderCallContext, ProviderError, ProviderErrorKind, ProviderRequest, ProviderStream,
    ProviderStreamEvent, RateLimitSnapshot, RateLimitWindow, ReasoningSupport, ToolChoice,
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
use crate::renewable::{
    BundleRefresher, RenewableBundle, RenewableCredentialSource, wait_for_deadline,
};

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

/// A browser authorization request assembled from memory-only PKCE material.
#[derive(Debug, Clone, Copy)]
pub struct BrowserAuthorization<'a> {
    /// Exact allow-listed localhost callback URL.
    pub redirect_uri: &'a str,
    /// PKCE S256 challenge.
    pub code_challenge: &'a str,
    /// CSRF state value.
    pub state: &'a str,
}

/// Builds the trusted ChatGPT browser authorization URL.
pub fn browser_authorization_url(request: BrowserAuthorization<'_>) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", CHATGPT_CLIENT_ID)
        .append_pair("redirect_uri", request.redirect_uri)
        .append_pair("scope", CHATGPT_SCOPES)
        .append_pair("code_challenge", request.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", request.state)
        .append_pair("originator", "smith")
        .finish();
    format!("{CHATGPT_ISSUER}/oauth/authorize?{query}")
}

/// A pending ChatGPT device-code authorization.
pub struct DeviceAuthorization {
    verification_url: String,
    user_code: String,
    device_auth_id: String,
    interval: Duration,
}

impl fmt::Debug for DeviceAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceAuthorization")
            .field("verification_url", &self.verification_url)
            .field("interval", &self.interval)
            .finish_non_exhaustive()
    }
}

impl Drop for DeviceAuthorization {
    fn drop(&mut self) {
        self.user_code.zeroize();
        self.device_auth_id.zeroize();
    }
}

impl DeviceAuthorization {
    /// Public browser destination for the device ceremony.
    pub fn verification_url(&self) -> &str {
        &self.verification_url
    }

    /// One-time code the user enters at the public destination.
    pub fn user_code(&self) -> &str {
        &self.user_code
    }
}

/// Smith's bounded OAuth HTTP client.
#[derive(Clone)]
pub struct ChatGptOAuthClient {
    client: Client,
}

impl fmt::Debug for ChatGptOAuthClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChatGptOAuthClient")
            .finish_non_exhaustive()
    }
}

impl ChatGptOAuthClient {
    /// Builds an OAuth client that refuses redirects.
    pub fn new() -> Result<Self, ChatGptAuthError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("smith/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ChatGptAuthError::Client)?;
        Ok(Self { client })
    }

    /// Exchanges a loopback authorization code for Smith's protected bundle.
    pub async fn exchange_authorization_code(
        &self,
        code: &str,
        redirect_uri: &str,
        verifier: &str,
    ) -> Result<ChatGptTokenBundle, ChatGptAuthError> {
        let body = form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "authorization_code")
            .append_pair("code", code)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("client_id", CHATGPT_CLIENT_ID)
            .append_pair("code_verifier", verifier)
            .finish();
        let response = self
            .client
            .post(format!("{CHATGPT_ISSUER}/oauth/token"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|_| ChatGptAuthError::Transport)?;
        let tokens: TokenResponse = decode_oauth_response(response).await?;
        bundle_from_response(&tokens, None, None)
    }

    /// Starts the reviewed ChatGPT device-code flow.
    pub async fn request_device_code(&self) -> Result<DeviceAuthorization, ChatGptAuthError> {
        let response = self
            .client
            .post(format!("{CHATGPT_ISSUER}/api/accounts/deviceauth/usercode"))
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&json!({"client_id": CHATGPT_CLIENT_ID}))
                    .map_err(|_| ChatGptAuthError::InvalidResponse)?,
            )
            .send()
            .await
            .map_err(|_| ChatGptAuthError::Transport)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(ChatGptAuthError::DeviceUnavailable);
        }
        if !response.status().is_success() {
            return Err(ChatGptAuthError::Rejected);
        }
        #[derive(Deserialize)]
        struct Response {
            device_auth_id: String,
            #[serde(alias = "usercode")]
            user_code: String,
            interval: String,
        }
        let body: Response = decode_success_json(response).await?;
        let interval = body.interval.trim().parse::<u64>().unwrap_or(5).max(1);
        if body.device_auth_id.is_empty()
            || body.device_auth_id.len() > 512
            || body.user_code.is_empty()
            || body.user_code.len() > 64
        {
            return Err(ChatGptAuthError::InvalidResponse);
        }
        Ok(DeviceAuthorization {
            verification_url: format!("{CHATGPT_ISSUER}/codex/device"),
            user_code: body.user_code,
            device_auth_id: body.device_auth_id,
            interval: Duration::from_secs(interval),
        })
    }

    /// Polls and completes one device-code authorization for at most 15 minutes.
    pub async fn complete_device_code(
        &self,
        authorization: &DeviceAuthorization,
    ) -> Result<ChatGptTokenBundle, ChatGptAuthError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15 * 60);
        loop {
            let response = self
                .client
                .post(format!("{CHATGPT_ISSUER}/api/accounts/deviceauth/token"))
                .header("content-type", "application/json")
                .body(
                    serde_json::to_vec(&json!({
                        "device_auth_id": authorization.device_auth_id,
                        "user_code": authorization.user_code,
                    }))
                    .map_err(|_| ChatGptAuthError::InvalidResponse)?,
                )
                .send()
                .await
                .map_err(|_| ChatGptAuthError::Transport)?;
            if response.status().is_success() {
                #[derive(Deserialize)]
                struct CodeResponse {
                    authorization_code: String,
                    code_verifier: String,
                }
                let code: CodeResponse = decode_success_json(response).await?;
                return self
                    .exchange_authorization_code(
                        &code.authorization_code,
                        &format!("{CHATGPT_ISSUER}/deviceauth/callback"),
                        &code.code_verifier,
                    )
                    .await;
            }
            if !matches!(
                response.status(),
                StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
            ) {
                return Err(ChatGptAuthError::Rejected);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ChatGptAuthError::Rejected);
            }
            tokio::time::sleep(authorization.interval).await;
        }
    }
}

async fn decode_oauth_response(
    response: reqwest::Response,
) -> Result<TokenResponse, ChatGptAuthError> {
    if !response.status().is_success() {
        return Err(ChatGptAuthError::Rejected);
    }
    decode_success_json(response).await
}

async fn decode_success_json<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T, ChatGptAuthError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_OAUTH_RESPONSE_BYTES as u64)
    {
        return Err(ChatGptAuthError::ResponseTooLarge);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ChatGptAuthError::Transport)?;
        if body.len().saturating_add(chunk.len()) > MAX_OAUTH_RESPONSE_BYTES {
            return Err(ChatGptAuthError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| ChatGptAuthError::InvalidResponse)
}

/// Parses the `x-codex-*` rate-limit header family the Codex backend attaches
/// to Responses replies.
///
/// Consumption arrives as a percentage per window. The backend names the
/// reset absolutely today (`…-reset-at`, Unix seconds); older gateways
/// reported a relative `…-reset-after-seconds`. Both are read, so whichever
/// the server sent survives normalization.
fn codex_rate_limit_snapshot(headers: &[(String, String)]) -> RateLimitSnapshot {
    fn value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
    let mut snapshot = RateLimitSnapshot::new();
    for category in ["primary", "secondary"] {
        let prefix = format!("x-codex-{category}");
        let mut window = RateLimitWindow::new(category);
        window.used_percent = value(headers, &format!("{prefix}-used-percent"))
            .and_then(|raw| raw.trim().parse::<f64>().ok())
            .filter(|percent| percent.is_finite());
        window.window_seconds = value(headers, &format!("{prefix}-window-minutes"))
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .map(|minutes| minutes.saturating_mul(60));
        window.resets_at_ms = value(headers, &format!("{prefix}-reset-at"))
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .map(|seconds| seconds.saturating_mul(1_000));
        window.resets_in_ms = value(headers, &format!("{prefix}-reset-after-seconds"))
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .map(|seconds| seconds.saturating_mul(1_000));
        snapshot.push(window);
    }
    snapshot
}

/// The subset of the Codex usage payload Smith reads. Everything else the
/// endpoint reports — plan type, credits, spend controls — is account
/// commerce, not limit state, and stays unread.
#[derive(Debug, Deserialize)]
struct UsageWindowWire {
    used_percent: Option<f64>,
    limit_window_seconds: Option<u64>,
    reset_after_seconds: Option<u64>,
    reset_at: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct UsageRateLimitWire {
    primary_window: Option<UsageWindowWire>,
    secondary_window: Option<UsageWindowWire>,
}

#[derive(Debug, Deserialize)]
struct UsagePayloadWire {
    rate_limit: Option<UsageRateLimitWire>,
}

/// Converts a `/wham/usage` payload into the normalized snapshot the
/// credential pool records.
fn usage_snapshot_from_json(body: &[u8]) -> Result<RateLimitSnapshot, ChatGptAuthError> {
    let payload: UsagePayloadWire =
        serde_json::from_slice(body).map_err(|_| ChatGptAuthError::InvalidResponse)?;
    let mut snapshot = RateLimitSnapshot::new();
    let Some(details) = payload.rate_limit else {
        return Ok(snapshot);
    };
    for (category, wire) in [
        ("primary", details.primary_window),
        ("secondary", details.secondary_window),
    ] {
        let Some(wire) = wire else { continue };
        let mut window = RateLimitWindow::new(category);
        window.used_percent = wire.used_percent.filter(|percent| percent.is_finite());
        window.window_seconds = wire.limit_window_seconds;
        window.resets_at_ms = wire.reset_at.map(|seconds| seconds.saturating_mul(1_000));
        // The payload carries a zero here when the absolute time is the one
        // that speaks; a zero delay is filler, not a reset happening now.
        window.resets_in_ms = wire
            .reset_after_seconds
            .filter(|seconds| *seconds > 0)
            .map(|seconds| seconds.saturating_mul(1_000));
        snapshot.push(window);
    }
    Ok(snapshot)
}

/// Reads the account's server-reported usage without spending a model request.
///
/// This is the Codex CLI's usage API — `GET /wham/usage` beside the
/// `backend-api` Responses endpoint — and the only way to learn an account's
/// consumption before its first attempt: the Responses stream reports limit
/// state per attempt, and a fresh session has not made one.
pub async fn fetch_usage_snapshot(
    access_token: &str,
    account_id: &str,
) -> Result<RateLimitSnapshot, ChatGptAuthError> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("smith/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| ChatGptAuthError::Client)?;
    let response = client
        .get(CHATGPT_USAGE_ENDPOINT)
        .header("authorization", format!("Bearer {access_token}"))
        .header("chatgpt-account-id", account_id)
        .header("originator", "smith")
        .send()
        .await
        .map_err(|_| ChatGptAuthError::Transport)?;
    if !response.status().is_success() {
        return Err(ChatGptAuthError::Rejected);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_OAUTH_RESPONSE_BYTES as u64)
    {
        return Err(ChatGptAuthError::ResponseTooLarge);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ChatGptAuthError::Transport)?;
        if body.len().saturating_add(chunk.len()) > MAX_OAUTH_RESPONSE_BYTES {
            return Err(ChatGptAuthError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    usage_snapshot_from_json(&body)
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

#[async_trait]
impl BundleRefresher<ChatGptTokenBundle> for ChatGptOAuthClient {
    async fn refresh(
        &self,
        bundle: &ChatGptTokenBundle,
        _now_ms: u64,
    ) -> Result<ChatGptTokenBundle, ChatGptAuthError> {
        let refresh_token = bundle.refresh_secret();
        let body = form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "refresh_token")
            .append_pair("refresh_token", refresh_token.expose())
            .append_pair("client_id", CHATGPT_CLIENT_ID)
            .finish();
        let response = self
            .client
            .post(format!("{CHATGPT_ISSUER}/oauth/token"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|_| ChatGptAuthError::Transport)?;
        let tokens: TokenResponse = decode_oauth_response(response).await?;
        bundle_from_response(&tokens, Some(&refresh_token), Some(bundle.account_id()))
    }
}

/// Single-flight renewable ChatGPT credential source backed by Smith's
/// owner-only auth file.
pub type ChatGptCredentialSource = RenewableCredentialSource<ChatGptTokenBundle>;

impl RenewableCredentialSource<ChatGptTokenBundle> {
    /// Builds the production source from a protected serialized bundle.
    pub fn production(
        target: ProviderCredentialTarget,
        reference: CredentialRef,
        secret: &Secret,
        redactor: Option<DefaultRedactor>,
    ) -> Result<Self, ChatGptAuthError> {
        Ok(Self::new(
            target,
            reference,
            ChatGptTokenBundle::from_secret(secret)?,
            CredentialEnroller::new(),
            Arc::new(ChatGptOAuthClient::new()?),
            redactor,
            Arc::new(SystemClock),
        ))
    }
}

/// Configuration for the direct experimental Responses adapter.
pub struct ChatGptProviderConfig {
    /// Single model served by this binding.
    pub model: ModelId,
    /// Capabilities frozen from Smith's trusted model record.
    pub capabilities: Capabilities,
    /// ChatGPT account header value extracted at login.
    account_id: String,
}

impl fmt::Debug for ChatGptProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChatGptProviderConfig")
            .field("model", &self.model)
            .field("capabilities", &self.capabilities)
            .field("account_configured", &true)
            .finish()
    }
}

impl ChatGptProviderConfig {
    /// Builds a trusted single-model config.
    pub fn new(
        model: impl Into<String>,
        mut capabilities: Capabilities,
        account_id: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let account_id = account_id.into();
        if !valid_account_id(&account_id) {
            return Err(ProviderError::new(
                ProviderErrorKind::Auth,
                "the protected ChatGPT account identity is unusable",
            ));
        }
        capabilities.auth = AuthKind::Bearer;
        capabilities.streaming = true;
        capabilities.tools = true;
        capabilities.reasoning = ReasoningSupport::Controllable;
        capabilities.usage = true;
        capabilities.cache = true;
        // This adapter sends `prompt_cache_key`, so it drives an implicit
        // prefix cache and must say so. It does not chain `previous_response_id`
        // — that field exists only on the websocket request shape — so every
        // turn still uploads the whole history.
        capabilities.prompt_cache = PromptCacheControl::Implicit;
        Ok(Self {
            model: ModelId::new(model),
            capabilities,
            account_id,
        })
    }
}

impl Drop for ChatGptProviderConfig {
    fn drop(&mut self) {
        self.account_id.zeroize();
    }
}

/// Direct ChatGPT Codex Responses provider over Smith's normal runtime loop.
pub struct ChatGptProvider<T: HttpTransport> {
    transport: T,
    config: ChatGptProviderConfig,
    credential_source: Arc<dyn ProviderCredentialSource>,
    credential_target: ProviderCredentialTarget,
    credential_minimum_validity_ms: u64,
    clock: Arc<dyn Clock>,
}

impl<T: HttpTransport> fmt::Debug for ChatGptProvider<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChatGptProvider")
            .field("config", &self.config)
            .field(
                "credential_minimum_validity_ms",
                &self.credential_minimum_validity_ms,
            )
            .finish_non_exhaustive()
    }
}

impl<T: HttpTransport> ChatGptProvider<T> {
    /// Builds the direct adapter from a Smith-owned renewable source.
    pub fn new(
        transport: T,
        config: ChatGptProviderConfig,
        credential_target: ProviderCredentialTarget,
        credential_source: Arc<dyn ProviderCredentialSource>,
    ) -> Self {
        Self {
            transport,
            config,
            credential_source,
            credential_target,
            credential_minimum_validity_ms: CHATGPT_CREDENTIAL_MINIMUM_VALIDITY_MS,
            clock: Arc::new(SystemClock),
        }
    }

    /// Overrides time for deterministic tests.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Underlying transport, exposed for offline request fixtures.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Serializes a Responses request, keyed to the session for prefix caching.
    ///
    /// `prompt_cache_key` is the field the API documents for this, and sending
    /// none was not defensible. But a live probe of this endpoint showed the
    /// cache actually keys off **byte-identical prefix content**, not off this
    /// value: a request carrying a different key still hit a 3,584-token cache
    /// the moment its instructions and tool schemas matched bytes another
    /// session had just sent. So the key is worth sending and is not what earns
    /// the hit — keeping the prefix stable is. That is
    /// `smith_runtime::prompt`'s job, and the reason its stable sections are
    /// ordered ahead of everything that varies.
    fn build_payload(
        &self,
        request: &ProviderRequest,
        session: &agent_runtime_core::ids::SessionId,
    ) -> Result<(Value, BTreeMap<String, String>), ProviderError> {
        if request.sampling.temperature.is_some()
            || request.sampling.top_p.is_some()
            || !request.stop.is_empty()
            || request.structured_output.is_some()
        {
            return Err(ProviderError::new(
                ProviderErrorKind::Unsupported,
                "the experimental ChatGPT Responses binding cannot represent one or more request controls",
            ));
        }
        if !request.vendor_extensions.is_null() {
            return Err(ProviderError::new(
                ProviderErrorKind::BadRequest,
                "the experimental ChatGPT Responses binding does not accept provider extensions",
            ));
        }
        let instructions = request
            .messages
            .iter()
            .filter(|message| message.role == Role::System)
            .map(Message::joined_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let tool_names = response_tool_names(&request.tools)?;
        let mut input = Vec::new();
        for message in &request.messages {
            if message.role != Role::System {
                input.extend(response_items(message, &tool_names)?);
            }
        }
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                let name = tool_names
                    .iter()
                    .find_map(|(wire, canonical)| (canonical == &tool.name).then_some(wire))
                    .expect("every request tool has a wire name");
                json!({
                    "type": "function",
                    "name": name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                    "strict": false,
                })
            })
            .collect::<Vec<_>>();
        let mut payload = json!({
            "model": self.config.model.as_str(),
            "instructions": instructions,
            "input": input,
            "tools": tools,
            "tool_choice": response_tool_choice(&request.tool_choice, &tool_names)?,
            "parallel_tool_calls": true,
            "store": false,
            "stream": true,
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": session.as_str(),
        });
        let object = payload
            .as_object_mut()
            .expect("Responses payload is an object");
        // This endpoint currently rejects the otherwise standard Responses
        // `max_output_tokens` field. Smith still uses the canonical value for
        // local context planning and output reserve policy, but it cannot send
        // that limit on this experimental wire contract.
        if let Some(reasoning) = &request.reasoning {
            let effort = reasoning.effort.as_deref().ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::BadRequest,
                    "the ChatGPT reasoning request has no effort",
                )
            })?;
            object.insert(
                "reasoning".into(),
                json!({"effort": effort, "summary": "auto"}),
            );
        }
        Ok((payload, tool_names))
    }

    async fn acquire_credential(
        &self,
        ctx: &ProviderCallContext,
    ) -> Result<ProviderCredentialLease, ProviderError> {
        let acquire = self.credential_source.acquire(
            &self.credential_target,
            self.credential_minimum_validity_ms,
            &ctx.cancel,
            ctx.deadline,
        );
        tokio::pin!(acquire);
        let lease = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => {
                return Err(credential_error(ProviderCredentialError::Cancelled));
            }
            _ = wait_for_deadline(ctx.deadline, self.clock.as_ref()) => {
                return Err(credential_error(ProviderCredentialError::Timeout));
            }
            result = &mut acquire => result.map_err(credential_error)?,
        };
        if lease.expires_at().is_some_and(|expiry| {
            expiry
                < self
                    .clock
                    .now()
                    .plus_millis(self.credential_minimum_validity_ms)
        }) {
            return Err(credential_error(ProviderCredentialError::InvalidLease));
        }
        Ok(lease)
    }
}

fn response_items(
    message: &Message,
    tool_names: &BTreeMap<String, String>,
) -> Result<Vec<Value>, ProviderError> {
    Ok(match message.role {
        Role::System => Vec::new(),
        Role::User => vec![json!({
            "type": "message",
            "role": "user",
            "content": message.content.iter().filter_map(|part| match part {
                ContentPart::Text { text } => Some(json!({"type": "input_text", "text": text})),
                ContentPart::Image { url, detail } => Some(json!({
                    "type": "input_image",
                    "image_url": url,
                    "detail": detail.as_deref().unwrap_or("auto"),
                })),
                _ => None,
            }).collect::<Vec<_>>(),
        })],
        Role::Assistant => {
            let mut items = Vec::new();
            let content = message
                .content
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text { text } => {
                        Some(json!({"type": "output_text", "text": text, "annotations": []}))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            if !content.is_empty() {
                items.push(json!({"type": "message", "role": "assistant", "content": content}));
            }
            for part in &message.content {
                match part {
                    ContentPart::Reasoning {
                        signature: Some(signature),
                        ..
                    } => items.push(json!({
                        "type": "reasoning",
                        "summary": [],
                        "encrypted_content": signature,
                    })),
                    ContentPart::ToolCall(call) => {
                        let wire = response_wire_tool_name(&call.name);
                        if tool_names
                            .get(&wire)
                            .is_some_and(|canonical| canonical != &call.name)
                        {
                            return Err(tool_name_collision());
                        }
                        items.push(json!({
                            "type": "function_call",
                            "call_id": call.id.as_str(),
                            "name": wire,
                            "arguments": call.arguments.to_string(),
                        }));
                    }
                    _ => {}
                }
            }
            items
        }
        Role::Tool => message
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::ToolResult(result) => Some(json!({
                    "type": "function_call_output",
                    "call_id": result.call_id.as_str(),
                    "output": result.content.iter().filter_map(ContentPart::as_text).collect::<Vec<_>>().join("\n"),
                })),
                _ => None,
            })
            .collect(),
    })
}

fn response_tool_names(
    tools: &[agent_runtime_core::provider::ToolSchema],
) -> Result<BTreeMap<String, String>, ProviderError> {
    let mut names = BTreeMap::new();
    for tool in tools {
        let wire = response_wire_tool_name(&tool.name);
        if names
            .insert(wire, tool.name.clone())
            .is_some_and(|canonical| canonical != tool.name)
        {
            return Err(tool_name_collision());
        }
    }
    Ok(names)
}

fn response_wire_tool_name(name: &str) -> String {
    if valid_response_tool_name(name) {
        return name.to_owned();
    }
    let digest = format!("{:x}", Sha256::digest(name.as_bytes()));
    format!("smith_tool_{}", &digest[..53])
}

fn tool_name_collision() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::BadRequest,
        "ChatGPT tool names collide after wire normalization",
    )
}

fn valid_response_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
}

fn response_tool_choice(
    choice: &ToolChoice,
    tool_names: &BTreeMap<String, String>,
) -> Result<Value, ProviderError> {
    Ok(match choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::None => json!("none"),
        ToolChoice::Required => json!("required"),
        ToolChoice::Named(name) => {
            let wire = tool_names
                .iter()
                .find_map(|(wire, canonical)| (canonical == name).then_some(wire))
                .ok_or_else(|| {
                    ProviderError::new(
                        ProviderErrorKind::BadRequest,
                        "the named ChatGPT tool choice is not in the request",
                    )
                })?;
            json!({"type": "function", "name": wire})
        }
    })
}

fn credential_error(error: ProviderCredentialError) -> ProviderError {
    let kind = match error {
        ProviderCredentialError::Cancelled => ProviderErrorKind::Cancelled,
        ProviderCredentialError::Timeout => ProviderErrorKind::Timeout,
        _ => ProviderErrorKind::Auth,
    };
    ProviderError::new(kind, error.to_string())
}

#[allow(clippy::too_many_arguments)]
async fn classify_auth_rejection(
    error: ProviderError,
    source: Arc<dyn ProviderCredentialSource>,
    target: ProviderCredentialTarget,
    rejected_revision: ProviderCredentialRevision,
    cancel: &Cancellation,
    deadline: Deadline,
    clock: Arc<dyn Clock>,
) -> ProviderError {
    if error.kind != ProviderErrorKind::Auth {
        return error;
    }
    let invalidate = source.invalidate(
        &target,
        &rejected_revision,
        ProviderAuthRejection::Unauthorized,
        cancel,
        deadline,
    );
    tokio::pin!(invalidate);
    let outcome = tokio::select! {
        biased;
        _ = cancel.cancelled() => return credential_error(ProviderCredentialError::Cancelled),
        _ = wait_for_deadline(deadline, clock.as_ref()) => {
            return credential_error(ProviderCredentialError::Timeout);
        }
        result = &mut invalidate => match result {
            Ok(outcome) => outcome,
            Err(error) => return credential_error(error),
        },
    };
    let error = ProviderError::new(ProviderErrorKind::Auth, "provider authentication rejected");
    if outcome == CredentialInvalidation::ReplacementPossible {
        error.with_credential_recovery(ProviderCredentialRecovery::RetryWithRenewedCredential)
    } else {
        error
    }
}

fn push_utf8(pending: &mut Vec<u8>, chunk: &[u8]) -> Result<Option<String>, ProviderError> {
    pending.extend_from_slice(chunk);
    match std::str::from_utf8(pending) {
        Ok(text) => {
            let text = text.to_owned();
            pending.clear();
            Ok((!text.is_empty()).then_some(text))
        }
        Err(error) if error.error_len().is_none() => {
            let valid = error.valid_up_to();
            if valid == 0 {
                return Ok(None);
            }
            let text = std::str::from_utf8(&pending[..valid])
                .expect("validated UTF-8 prefix")
                .to_owned();
            pending.drain(..valid);
            Ok(Some(text))
        }
        Err(_) => Err(ProviderError::new(
            ProviderErrorKind::MalformedStream,
            "ChatGPT Responses stream contained invalid UTF-8",
        )),
    }
}

#[derive(Default)]
struct StreamState {
    saw_semantic: bool,
    saw_tool_call: bool,
    terminal: bool,
    tool_names: BTreeMap<String, String>,
}

fn decode_event(
    data: &str,
    state: &mut StreamState,
) -> Result<Vec<ProviderStreamEvent>, ProviderError> {
    let value: Value = serde_json::from_str(data).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::MalformedStream,
            "invalid ChatGPT Responses stream event",
        )
    })?;
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut events = Vec::new();
    match kind {
        "response.output_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str)
                && !delta.is_empty()
            {
                events.push(ProviderStreamEvent::TextDelta {
                    text: delta.to_owned(),
                });
                state.saw_semantic = true;
            }
        }
        "response.reasoning_summary_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str)
                && !delta.is_empty()
            {
                events.push(ProviderStreamEvent::ReasoningDelta {
                    text: delta.to_owned(),
                    redacted: false,
                    signature: None,
                });
                state.saw_semantic = true;
            }
        }
        "response.output_item.added" => {
            let item = value.get("item").unwrap_or(&Value::Null);
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .and_then(|index| u32::try_from(index).ok())
                    .unwrap_or(0);
                events.push(ProviderStreamEvent::ToolCallDelta {
                    index,
                    id: item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    name: item.get("name").and_then(Value::as_str).map(|wire| {
                        state
                            .tool_names
                            .get(wire)
                            .cloned()
                            .unwrap_or_else(|| wire.to_owned())
                    }),
                    arguments_fragment: String::new(),
                });
                state.saw_tool_call = true;
                state.saw_semantic = true;
            }
        }
        "response.function_call_arguments.delta" => {
            let index = value
                .get("output_index")
                .and_then(Value::as_u64)
                .and_then(|index| u32::try_from(index).ok())
                .unwrap_or(0);
            events.push(ProviderStreamEvent::ToolCallDelta {
                index,
                id: None,
                name: None,
                arguments_fragment: value
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
            state.saw_tool_call = true;
            state.saw_semantic = true;
        }
        "response.output_item.done" => {
            let item = value.get("item").unwrap_or(&Value::Null);
            if item.get("type").and_then(Value::as_str) == Some("reasoning")
                && let Some(signature) = item.get("encrypted_content").and_then(Value::as_str)
            {
                events.push(ProviderStreamEvent::ReasoningDelta {
                    // Agent Runtime intentionally refuses a signature with no
                    // block to seal. A fixed redacted marker preserves the
                    // opaque continuation token without exposing it.
                    text: "[encrypted]".to_owned(),
                    redacted: true,
                    signature: Some(signature.to_owned()),
                });
                state.saw_semantic = true;
            }
        }
        "response.completed" => {
            if let Some(usage) = value.pointer("/response/usage") {
                append_usage(usage, &mut events);
            }
            events.push(ProviderStreamEvent::Finish {
                reason: if state.saw_tool_call {
                    FinishReason::ToolCalls
                } else {
                    FinishReason::Stop
                },
            });
            state.terminal = true;
        }
        "response.incomplete" => {
            if let Some(usage) = value.pointer("/response/usage") {
                append_usage(usage, &mut events);
            }
            let reason = if value
                .pointer("/response/incomplete_details/reason")
                .and_then(Value::as_str)
                == Some("max_output_tokens")
            {
                FinishReason::Length
            } else {
                FinishReason::Error
            };
            events.push(ProviderStreamEvent::Finish { reason });
            state.terminal = true;
        }
        "response.failed" | "error" => {
            state.terminal = true;
            events.push(ProviderStreamEvent::Error {
                error: ProviderError::new(
                    ProviderErrorKind::Server,
                    "the experimental ChatGPT Responses backend reported a failure",
                )
                .retryable(),
            });
        }
        _ => {}
    }
    Ok(events)
}

fn append_usage(usage: &Value, events: &mut Vec<ProviderStreamEvent>) {
    let input = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let details = usage.get("input_tokens_details");
    let cached = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64);
    let written = details
        .and_then(|details| details.get("cache_write_tokens"))
        .and_then(Value::as_u64);
    // Keep the billing counters disjoint while preserving the provider's
    // independent field presence in CacheObservation. An explicit zero is
    // evidence; an omitted field is not a zero.
    let cached_count = cached.unwrap_or_default().min(input);
    let write_count = written
        .unwrap_or_default()
        .min(input.saturating_sub(cached_count));
    let output = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let reasoning = usage
        .get("output_tokens_details")
        .and_then(|details| details.get("reasoning_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
        .min(output);
    let mut delta = UsageDelta::new();
    let uncached = input
        .saturating_sub(cached_count)
        .saturating_sub(write_count);
    if uncached > 0 {
        delta.add(CounterKind::InputUncached, uncached);
    }
    if cached_count > 0 {
        delta.add(CounterKind::InputCached, cached_count);
    }
    if write_count > 0 {
        delta.add(CounterKind::CacheWrite, write_count);
    }
    if cached.is_some() || written.is_some() {
        events.push(ProviderStreamEvent::CacheObservation {
            read_tokens: cached,
            write_tokens: written,
        });
    }
    if output.saturating_sub(reasoning) > 0 {
        delta.add(CounterKind::Output, output.saturating_sub(reasoning));
    }
    if reasoning > 0 {
        delta.add(CounterKind::Reasoning, reasoning);
    }
    if !delta.is_empty() {
        events.push(ProviderStreamEvent::Usage { delta });
    }
}

#[async_trait]
impl<T: HttpTransport> Provider for ChatGptProvider<T> {
    fn describe(&self) -> Vec<ModelDescriptor> {
        vec![ModelDescriptor {
            id: self.config.model.clone(),
            display_name: self.config.model.to_string(),
            vendor: "chatgpt-experimental".into(),
            capabilities: self.config.capabilities.clone(),
        }]
    }

    fn capabilities(&self, model: &ModelId) -> Option<Capabilities> {
        (model == &self.config.model).then(|| self.config.capabilities.clone())
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        let (payload, tool_names) = self.build_payload(&request, &ctx.session)?;
        let body = serde_json::to_vec(&payload).map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::BadRequest,
                "the ChatGPT Responses request could not be encoded",
            )
        })?;
        let lease = self.acquire_credential(&ctx).await?;
        let rejected_revision = lease.revision().clone();
        // The lease's account wins over the one frozen at construction: a
        // source that rotates between accounts issues each lease from a
        // different identity, and the header has to match the token beside it.
        let account_id = lease
            .account()
            .unwrap_or(&self.config.account_id)
            .to_owned();
        let http = HttpRequest {
            url: CHATGPT_RESPONSES_ENDPOINT.into(),
            headers: vec![
                ("content-type".into(), "application/json".into()),
                ("accept".into(), "text/event-stream".into()),
                (
                    "authorization".into(),
                    format!("Bearer {}", lease.secret().expose()),
                ),
                ("chatgpt-account-id".into(), account_id),
                ("originator".into(), "smith".into()),
                ("session-id".into(), ctx.request_id.as_str().to_owned()),
            ],
            body,
        };
        let post = self.transport.post_response(http);
        tokio::pin!(post);
        let response = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => {
                return Err(ProviderError::new(ProviderErrorKind::Cancelled, "cancelled"));
            }
            _ = wait_for_deadline(ctx.deadline, self.clock.as_ref()) => {
                return Err(ProviderError::new(ProviderErrorKind::Timeout, "provider deadline elapsed"));
            }
            result = &mut post => match result {
                Ok(response) => response,
                Err(error) => {
                    return Err(classify_auth_rejection(
                        error,
                        self.credential_source.clone(),
                        self.credential_target.clone(),
                        rejected_revision,
                        &ctx.cancel,
                        ctx.deadline,
                        self.clock.clone(),
                    ).await);
                }
            },
        };
        // Read before the body moves: these headers describe the credential
        // that served this attempt, and nothing downstream can recover them
        // once the stream is running.
        let rate_limits = codex_rate_limit_snapshot(&response.headers);
        let mut bytes = response.body;
        let cancel = ctx.cancel.clone();
        let deadline = ctx.deadline;
        let clock = self.clock.clone();
        let out = stream! {
            // Emitted first so a consumer sees the limit state that governed
            // this attempt before any of its output.
            if !rate_limits.is_empty() {
                yield ProviderStreamEvent::RateLimit { snapshot: rate_limits };
            }
            let mut parser = SseFrameParser::new();
            let mut pending_utf8 = Vec::new();
            let mut state = StreamState {
                tool_names,
                ..StreamState::default()
            };
            loop {
                let next = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        yield ProviderStreamEvent::Error {
                            error: ProviderError::new(ProviderErrorKind::Cancelled, "cancelled"),
                        };
                        return;
                    }
                    _ = wait_for_deadline(deadline, clock.as_ref()) => {
                        yield ProviderStreamEvent::Error {
                            error: ProviderError::new(ProviderErrorKind::Timeout, "provider deadline elapsed"),
                        };
                        return;
                    }
                    chunk = bytes.next() => chunk,
                };
                let Some(chunk) = next else { break; };
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        yield ProviderStreamEvent::Error { error };
                        return;
                    }
                };
                let text = match push_utf8(&mut pending_utf8, &chunk) {
                    Ok(Some(text)) => text,
                    Ok(None) => continue,
                    Err(error) => {
                        yield ProviderStreamEvent::Error { error };
                        return;
                    }
                };
                parser.push_str(&text);
                for frame in parser.drain_frames() {
                    let data = frame.data.trim();
                    if data.is_empty() || data == "[DONE]" {
                        continue;
                    }
                    match decode_event(data, &mut state) {
                        Ok(events) => {
                            for event in events {
                                yield event;
                            }
                        }
                        Err(error) => {
                            yield ProviderStreamEvent::Error { error };
                            return;
                        }
                    }
                    if state.terminal {
                        return;
                    }
                }
            }
            if !pending_utf8.is_empty() {
                yield ProviderStreamEvent::Error {
                    error: ProviderError::new(
                        ProviderErrorKind::MalformedStream,
                        "ChatGPT Responses stream ended with incomplete UTF-8",
                    ),
                };
                return;
            }
            if let Some(frame) = parser.finish() {
                let data = frame.data.trim();
                if !data.is_empty() && data != "[DONE]" {
                    match decode_event(data, &mut state) {
                        Ok(events) => {
                            for event in events {
                                yield event;
                            }
                        }
                        Err(error) => {
                            yield ProviderStreamEvent::Error { error };
                            return;
                        }
                    }
                }
            }
            if !state.terminal {
                yield ProviderStreamEvent::Error {
                    error: ProviderError::new(
                        ProviderErrorKind::MalformedStream,
                        "ChatGPT Responses stream ended without a terminal event",
                    ),
                };
            }
        };
        Ok(Box::pin(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::content::ToolCall;
    use agent_runtime_core::ids::{AttemptId, RequestId, ToolCallId};
    use agent_runtime_core::provider::ToolSchema;
    use agent_runtime_core::provider_credential::StaticProviderCredentialSource;
    use agent_runtime_testkit::{
        CredentialLeaseFixture, RenewableProviderCredentialSource, ReplayTransport,
    };
    use smith_config::auth_file::{AuthFileBackend, AuthFileError};
    use smith_config::credential::{CredentialEnrollmentBackend, KeychainError};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, Default)]
    struct MemoryEnrollment {
        value: Mutex<Option<Secret>>,
    }

    impl AuthFileBackend for MemoryEnrollment {
        fn read(&self, _entry: &str) -> Result<Option<Secret>, AuthFileError> {
            Ok(self.value.lock().expect("memory store").clone())
        }

        fn store(&self, _entry: &str, secret: &Secret) -> Result<(), AuthFileError> {
            *self.value.lock().expect("memory store") = Some(secret.clone());
            Ok(())
        }

        fn remove(&self, _entry: &str) -> Result<(), AuthFileError> {
            *self.value.lock().expect("memory store") = None;
            Ok(())
        }
    }

    #[derive(Debug)]
    struct PanicKeychainEnrollment;

    impl CredentialEnrollmentBackend for PanicKeychainEnrollment {
        fn prior(&self, _service: &str, _account: &str) -> Result<Option<Secret>, KeychainError> {
            panic!("ChatGPT must not query the keychain")
        }

        fn store(
            &self,
            _service: &str,
            _account: &str,
            _secret: &Secret,
        ) -> Result<(), KeychainError> {
            panic!("ChatGPT must not write the keychain")
        }

        fn remove(&self, _service: &str, _account: &str) -> Result<(), KeychainError> {
            panic!("ChatGPT must not remove a keychain entry")
        }
    }

    #[derive(Debug, Default)]
    struct RotatingEndpoint {
        calls: AtomicUsize,
    }

    #[derive(Debug)]
    struct AuthRejectingTransport;

    #[async_trait]
    impl HttpTransport for AuthRejectingTransport {
        async fn post_stream(
            &self,
            _request: HttpRequest,
        ) -> Result<agent_runtime::provider::transport::ByteStream, ProviderError> {
            Err(ProviderError::new(
                ProviderErrorKind::Auth,
                "transport-classified unauthorized",
            ))
        }
    }

    #[async_trait]
    impl BundleRefresher<ChatGptTokenBundle> for RotatingEndpoint {
        async fn refresh(
            &self,
            bundle: &ChatGptTokenBundle,
            _now_ms: u64,
        ) -> Result<ChatGptTokenBundle, ChatGptAuthError> {
            assert_eq!(bundle.refresh_secret().expose(), "refresh-old");
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            Ok(ChatGptTokenBundle {
                access_token: "access-new".into(),
                refresh_token: "refresh-new".into(),
                expires_at_ms: SystemClock
                    .now()
                    .as_millis()
                    .saturating_add(60 * 60 * 1_000),
                account_id: bundle.account_id().to_owned(),
            })
        }
    }

    fn jwt(claims: Value) -> String {
        format!(
            "e30.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("claims"))
        )
    }

    #[test]
    fn account_claims_are_extracted_without_rendering_tokens() {
        let token = jwt(json!({
            "exp": 4_102_444_800_u64,
            "https://api.openai.com/auth": {"chatgpt_account_id": "acct_test-1"}
        }));
        assert_eq!(extract_account_id(&token).as_deref(), Some("acct_test-1"));
        let response = TokenResponse {
            id_token: None,
            access_token: token.clone(),
            refresh_token: Some("refresh-canary".into()),
            expires_in: None,
        };
        let bundle = bundle_from_response(&response, None, None).expect("bundle");
        let debug = format!("{bundle:?}");
        assert!(!debug.contains(&token));
        assert!(!debug.contains("refresh-canary"));
        assert_eq!(bundle.account_id(), "acct_test-1");
    }

    #[test]
    fn token_bundle_round_trips_only_through_a_secret() {
        let bundle = ChatGptTokenBundle {
            access_token: "access-canary".into(),
            refresh_token: "refresh-canary".into(),
            expires_at_ms: 9_999_999_999_999,
            account_id: "acct_test".into(),
        };
        let encoded = bundle.to_secret().expect("encoded");
        let decoded = ChatGptTokenBundle::from_secret(&encoded).expect("decoded");
        assert_eq!(decoded.account_id(), "acct_test");
        assert!(!format!("{decoded:?}").contains("canary"));
    }

    #[test]
    fn codex_rate_limit_headers_parse_both_reset_shapes() {
        let headers = vec![
            ("x-codex-primary-used-percent".to_owned(), "12.5".to_owned()),
            (
                "x-codex-primary-window-minutes".to_owned(),
                "300".to_owned(),
            ),
            (
                "x-codex-primary-reset-at".to_owned(),
                "1704069000".to_owned(),
            ),
            ("x-codex-secondary-used-percent".to_owned(), "80".to_owned()),
            (
                "x-codex-secondary-reset-after-seconds".to_owned(),
                "3600".to_owned(),
            ),
        ];
        let snapshot = codex_rate_limit_snapshot(&headers);
        assert_eq!(snapshot.windows.len(), 2);
        assert_eq!(snapshot.windows[0].id.as_deref(), Some("primary"));
        assert_eq!(snapshot.windows[0].used_percent, Some(12.5));
        assert_eq!(snapshot.windows[0].window_seconds, Some(300 * 60));
        assert_eq!(snapshot.windows[0].resets_at_ms, Some(1_704_069_000_000));
        assert_eq!(snapshot.windows[1].id.as_deref(), Some("secondary"));
        assert_eq!(snapshot.windows[1].used_percent, Some(80.0));
        assert_eq!(snapshot.windows[1].resets_in_ms, Some(3_600_000));
    }

    #[test]
    fn absent_rate_limit_headers_yield_an_empty_snapshot_not_zeroes() {
        let headers = vec![("content-type".to_owned(), "text/event-stream".to_owned())];
        assert!(codex_rate_limit_snapshot(&headers).is_empty());
    }

    #[test]
    fn usage_payload_maps_windows_and_treats_zero_delay_as_filler() {
        let body = json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 42,
                    "limit_window_seconds": 300,
                    "reset_after_seconds": 0,
                    "reset_at": 1704069000,
                },
                "secondary_window": {
                    "used_percent": 84.5,
                    "limit_window_seconds": 3600,
                    "reset_after_seconds": 1800,
                },
            },
            "credits": {"has_credits": true, "unlimited": false},
        });
        let snapshot =
            usage_snapshot_from_json(&serde_json::to_vec(&body).expect("encodes")).expect("parses");
        assert_eq!(snapshot.windows.len(), 2);
        assert_eq!(snapshot.windows[0].id.as_deref(), Some("primary"));
        assert_eq!(snapshot.windows[0].used_percent, Some(42.0));
        assert_eq!(snapshot.windows[0].window_seconds, Some(300));
        assert_eq!(snapshot.windows[0].resets_at_ms, Some(1_704_069_000_000));
        // The zero delay beside an absolute reset is filler, not "resets now".
        assert_eq!(snapshot.windows[0].resets_in_ms, None);
        assert_eq!(snapshot.windows[1].used_percent, Some(84.5));
        assert_eq!(snapshot.windows[1].resets_in_ms, Some(1_800_000));
    }

    #[test]
    fn usage_payload_without_rate_limit_reads_as_unmeasured() {
        let body = json!({"plan_type": "plus"});
        let snapshot =
            usage_snapshot_from_json(&serde_json::to_vec(&body).expect("encodes")).expect("parses");
        assert!(snapshot.is_empty());
    }

    #[test]
    fn responses_usage_is_disjoint_and_terminal() {
        let mut state = StreamState::default();
        let events = decode_event(
            &json!({
                "type": "response.completed",
                "response": {"usage": {
                    "input_tokens": 100,
                    "input_tokens_details": {"cached_tokens": 25},
                    "output_tokens": 40,
                    "output_tokens_details": {"reasoning_tokens": 10}
                }}
            })
            .to_string(),
            &mut state,
        )
        .expect("event");
        let usage = events
            .iter()
            .find_map(|event| match event {
                ProviderStreamEvent::Usage { delta } => Some(delta),
                _ => None,
            })
            .expect("usage");
        assert_eq!(usage.get(CounterKind::InputUncached), 75);
        assert_eq!(usage.get(CounterKind::InputCached), 25);
        assert_eq!(usage.get(CounterKind::Output), 30);
        assert_eq!(usage.get(CounterKind::Reasoning), 10);
        assert!(state.terminal);
    }

    #[test]
    fn responses_cache_observation_preserves_presence_and_write_tokens() {
        let cases = [
            (
                "zero-read",
                json!({
                    "input_tokens": 10,
                    "input_tokens_details": {"cached_tokens": 0},
                    "output_tokens": 2
                }),
                Some(0),
                None,
                10,
                0,
            ),
            (
                "omitted",
                json!({"input_tokens": 10, "output_tokens": 2}),
                None,
                None,
                10,
                0,
            ),
            (
                "write-only",
                json!({
                    "input_tokens": 10,
                    "input_tokens_details": {"cache_write_tokens": 3},
                    "output_tokens": 2
                }),
                None,
                Some(3),
                7,
                3,
            ),
        ];

        for (name, usage, expected_read, expected_write, uncached, written) in cases {
            let mut state = StreamState::default();
            let events = decode_event(
                &json!({"type": "response.completed", "response": {"usage": usage}}).to_string(),
                &mut state,
            )
            .expect("event");
            let observations: Vec<_> = events
                .iter()
                .filter_map(|event| match event {
                    ProviderStreamEvent::CacheObservation {
                        read_tokens,
                        write_tokens,
                    } => Some((*read_tokens, *write_tokens)),
                    _ => None,
                })
                .collect();
            if expected_read.is_none() && expected_write.is_none() {
                assert!(observations.is_empty(), "{name} must stay absent");
            } else {
                assert_eq!(
                    observations,
                    vec![(expected_read, expected_write)],
                    "{name}"
                );
            }
            let delta = events.iter().find_map(|event| match event {
                ProviderStreamEvent::Usage { delta } => Some(delta),
                _ => None,
            });
            let delta = delta.expect("usage");
            assert_eq!(delta.get(CounterKind::InputUncached), uncached, "{name}");
            assert_eq!(delta.get(CounterKind::CacheWrite), written, "{name}");
        }
    }

    #[test]
    fn browser_url_pins_pkce_state_scopes_and_smith_originator() {
        let url = browser_authorization_url(BrowserAuthorization {
            redirect_uri: "http://localhost:1455/auth/callback",
            code_challenge: "challenge",
            state: "state",
        });
        let parsed = reqwest::Url::parse(&url).expect("url");
        let query = parsed
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            query.get("client_id").map(|value| value.as_ref()),
            Some(CHATGPT_CLIENT_ID)
        );
        assert_eq!(
            query
                .get("code_challenge_method")
                .map(|value| value.as_ref()),
            Some("S256")
        );
        assert_eq!(
            query.get("state").map(|value| value.as_ref()),
            Some("state")
        );
        assert_eq!(
            query.get("originator").map(|value| value.as_ref()),
            Some("smith")
        );
        assert_eq!(
            query.get("scope").map(|value| value.as_ref()),
            Some(CHATGPT_SCOPES)
        );
    }

    #[test]
    fn response_tool_names_are_backend_safe_collision_free_and_reversible() {
        let tools = vec![
            ToolSchema {
                name: "smith_tool_1".into(),
                description: "Already valid".into(),
                input_schema: json!({"type": "object"}),
            },
            ToolSchema {
                name: "tool:registry.search".into(),
                description: "Needs a wire alias".into(),
                input_schema: json!({"type": "object"}),
            },
        ];
        let names = response_tool_names(&tools).expect("distinct tool names");
        let alias = response_wire_tool_name("tool:registry.search");
        assert_eq!(
            names.get("smith_tool_1").map(String::as_str),
            Some("smith_tool_1")
        );
        assert_eq!(
            names.get(&alias).map(String::as_str),
            Some("tool:registry.search")
        );
        assert!(names.keys().all(|name| valid_response_tool_name(name)));
        assert_eq!(
            response_tool_choice(&ToolChoice::Named("tool:registry.search".into()), &names,)
                .expect("named choice")
                .pointer("/name")
                .and_then(Value::as_str),
            Some(alias.as_str())
        );
    }

    #[test]
    fn tool_call_history_keeps_its_wire_name_when_activation_grows() {
        let registry = ToolSchema {
            name: "registry.search".into(),
            description: "Search the tool registry".into(),
            input_schema: json!({"type": "object"}),
        };
        let initial =
            response_tool_names(std::slice::from_ref(&registry)).expect("initial tool names");
        let expanded = response_tool_names(&[
            ToolSchema {
                name: "artifact.read".into(),
                description: "Read an artifact".into(),
                input_schema: json!({"type": "object"}),
            },
            ToolSchema {
                name: "list".into(),
                description: "List files".into(),
                input_schema: json!({"type": "object"}),
            },
            registry,
        ])
        .expect("expanded tool names");
        let wire = response_wire_tool_name("registry.search");
        assert_eq!(initial.get(&wire), Some(&"registry.search".to_owned()));
        assert_eq!(expanded.get(&wire), Some(&"registry.search".to_owned()));

        let history = Message::assistant(vec![ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new("call-1"),
            name: "registry.search".into(),
            arguments: json!({"query": "workspace repository inspection"}),
        })]);
        let replayed = response_items(&history, &expanded).expect("history encodes");
        assert_eq!(
            replayed[0].get("name").and_then(Value::as_str),
            Some(wire.as_str())
        );
        assert!(valid_response_tool_name(&wire));
    }

    /// The wire pairs a bearer with the account that issued it. When the
    /// source can name that account, its lease overrides the identity frozen
    /// into the provider config at construction — otherwise a source that
    /// changed accounts would send one account's token under another's header.
    #[tokio::test]
    async fn the_account_header_follows_the_lease_not_the_frozen_config() {
        let target = ProviderCredentialTarget::new("chatgpt").expect("target");
        let source = Arc::new(ChatGptCredentialSource::new(
            target.clone(),
            CredentialRef::parse("authfile:chatgpt").expect("reference"),
            ChatGptTokenBundle {
                access_token: "access-lease".into(),
                refresh_token: "refresh".into(),
                expires_at_ms: u64::MAX,
                account_id: "acct_lease".into(),
            },
            CredentialEnroller::with_backends(
                Arc::new(PanicKeychainEnrollment),
                Arc::new(MemoryEnrollment::default()),
            ),
            Arc::new(RotatingEndpoint::default()),
            None,
            Arc::new(SystemClock),
        )) as Arc<dyn ProviderCredentialSource>;
        let provider = ChatGptProvider::new(
            ReplayTransport::single(
                "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
            ),
            ChatGptProviderConfig::new(
                "gpt-5.6-terra",
                Capabilities::basic_streaming(),
                "acct_config",
            )
            .expect("config"),
            target,
            source,
        );
        let ctx = ProviderCallContext {
            session: agent_runtime_core::ids::SessionId::new("session-test"),
            request_id: RequestId::new("request-1"),
            attempt_id: AttemptId::new("attempt-1"),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
        };
        let _events = provider
            .stream(
                ProviderRequest::new(ModelId::new("gpt-5.6-terra"), vec![Message::user("hello")]),
                ctx,
            )
            .await
            .expect("stream")
            .collect::<Vec<_>>()
            .await;
        let sent = provider.transport().requests();
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0]
                .headers
                .iter()
                .any(|(name, value)| name == "chatgpt-account-id" && value == "acct_lease")
        );
        assert!(
            !sent[0]
                .headers
                .iter()
                .any(|(_, value)| value == "acct_config")
        );
    }

    #[tokio::test]
    async fn concurrent_expiry_refreshes_once_and_persists_rotated_bundle() {
        let backend = Arc::new(MemoryEnrollment::default());
        let endpoint = Arc::new(RotatingEndpoint::default());
        let source = Arc::new(ChatGptCredentialSource::new(
            ProviderCredentialTarget::new("chatgpt").expect("target"),
            CredentialRef::parse("authfile:chatgpt").expect("reference"),
            ChatGptTokenBundle {
                access_token: "access-old".into(),
                refresh_token: "refresh-old".into(),
                expires_at_ms: 1,
                account_id: "acct_test".into(),
            },
            CredentialEnroller::with_backends(Arc::new(PanicKeychainEnrollment), backend.clone()),
            endpoint.clone(),
            None,
            Arc::new(SystemClock),
        ));
        let target = ProviderCredentialTarget::new("chatgpt").expect("target");
        let cancel = Cancellation::new();
        let (left, right) = tokio::join!(
            source.acquire(&target, 30_000, &cancel, Deadline::never()),
            source.acquire(&target, 30_000, &cancel, Deadline::never()),
        );
        let left = left.expect("left lease");
        let right = right.expect("right lease");
        assert_eq!(left.secret().expose(), "access-new");
        assert_eq!(right.secret().expose(), "access-new");
        assert_eq!(endpoint.calls.load(Ordering::SeqCst), 1);
        let stored = backend
            .value
            .lock()
            .expect("memory store")
            .clone()
            .expect("stored bundle");
        let stored = ChatGptTokenBundle::from_secret(&stored).expect("rotated bundle");
        assert_eq!(stored.access_secret().expose(), "access-new");
        assert_eq!(stored.refresh_secret().expose(), "refresh-new");
    }

    #[tokio::test]
    async fn pre_stream_auth_rejection_invalidates_exact_revision_for_one_runtime_replay() {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let source = Arc::new(RenewableProviderCredentialSource::new(
            clock,
            CredentialLeaseFixture::non_expiring("access-old", "revision-1").expect("lease"),
            [CredentialLeaseFixture::non_expiring("access-new", "revision-2").expect("lease")],
        ));
        let provider = ChatGptProvider::new(
            AuthRejectingTransport,
            ChatGptProviderConfig::new(
                "gpt-5.6-terra",
                Capabilities::basic_streaming(),
                "acct_test",
            )
            .expect("config"),
            ProviderCredentialTarget::new("chatgpt").expect("target"),
            source.clone(),
        );
        let result = provider
            .stream(
                ProviderRequest::new(ModelId::new("gpt-5.6-terra"), vec![Message::user("hello")]),
                ProviderCallContext {
                    session: agent_runtime_core::ids::SessionId::new("session-test"),
                    request_id: RequestId::new("request-1"),
                    attempt_id: AttemptId::new("attempt-1"),
                    cancel: Cancellation::new(),
                    deadline: Deadline::never(),
                },
            )
            .await;
        let error = match result {
            Ok(_) => panic!("401 classification must fail before a stream is accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind, ProviderErrorKind::Auth);
        assert_eq!(
            error.credential_recovery,
            Some(ProviderCredentialRecovery::RetryWithRenewedCredential)
        );
        assert_eq!(source.invalidations().len(), 1);
    }

    #[tokio::test]
    async fn direct_adapter_maps_tools_usage_headers_and_terminal_without_codex() {
        let wire = response_wire_tool_name("tool:registry.search");
        let sse = [
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"call-1\",\"name\":\"smith_tool_0\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"{\\\"path\\\":\\\"README.md\\\"}\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":12,\"input_tokens_details\":{\"cached_tokens\":2},\"output_tokens\":5,\"output_tokens_details\":{\"reasoning_tokens\":1}}}}\n\n",
        ]
        .concat()
        .replace("smith_tool_0", &wire);
        let target = ProviderCredentialTarget::new("chatgpt").expect("target");
        let source = Arc::new(StaticProviderCredentialSource::new(Secret::new(
            "access-token-canary",
        ))) as Arc<dyn ProviderCredentialSource>;
        let provider = ChatGptProvider::new(
            ReplayTransport::single(sse),
            ChatGptProviderConfig::new(
                "gpt-5.6-terra",
                Capabilities::basic_streaming(),
                "acct_test",
            )
            .expect("config"),
            target,
            source,
        );
        let mut request = ProviderRequest::new(
            ModelId::new("gpt-5.6-terra"),
            vec![Message::system("Be concise"), Message::user("Read it")],
        );
        request.max_output_tokens = Some(16);
        request.tools.push(ToolSchema {
            name: "tool:registry.search".into(),
            description: "Search the tool registry".into(),
            input_schema: json!({"type": "object"}),
        });
        let ctx = ProviderCallContext {
            session: agent_runtime_core::ids::SessionId::new("session-test"),
            request_id: RequestId::new("request-1"),
            attempt_id: AttemptId::new("attempt-1"),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
        };
        let events = provider
            .stream(request, ctx)
            .await
            .expect("stream")
            .collect::<Vec<_>>()
            .await;
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::ToolCallDelta {
                id: Some(id),
                name: Some(name),
                ..
            } if id == "call-1" && name == "tool:registry.search"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::Finish {
                reason: FinishReason::ToolCalls
            }
        )));
        let usage = events
            .iter()
            .find_map(|event| match event {
                ProviderStreamEvent::Usage { delta } => Some(delta),
                _ => None,
            })
            .expect("usage");
        assert_eq!(usage.total(), 17);

        let sent = provider.transport().requests();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].url, CHATGPT_RESPONSES_ENDPOINT);
        assert!(sent[0].headers.iter().any(|(name, value)| {
            name == "authorization" && value == "Bearer access-token-canary"
        }));
        assert!(
            sent[0]
                .headers
                .iter()
                .any(|(name, value)| { name == "chatgpt-account-id" && value == "acct_test" })
        );
        assert!(!format!("{:?}", sent[0]).contains("access-token-canary"));
        let body: Value = serde_json::from_slice(&sent[0].body).expect("body");
        assert_eq!(body.get("store"), Some(&Value::Bool(false)));
        assert_eq!(body.get("max_output_tokens"), None);
        assert_eq!(
            body.get("instructions").and_then(Value::as_str),
            Some("Be concise")
        );
        assert_eq!(
            body.pointer("/tools/0/name").and_then(Value::as_str),
            Some(wire.as_str())
        );
    }

    #[tokio::test]
    async fn truncated_responses_stream_fails_malformed_instead_of_inventing_finish() {
        let target = ProviderCredentialTarget::new("chatgpt").expect("target");
        let source = Arc::new(StaticProviderCredentialSource::new(Secret::new("token")))
            as Arc<dyn ProviderCredentialSource>;
        let provider = ChatGptProvider::new(
            ReplayTransport::single(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
            ),
            ChatGptProviderConfig::new(
                "gpt-5.6-terra",
                Capabilities::basic_streaming(),
                "acct_test",
            )
            .expect("config"),
            target,
            source,
        );
        let ctx = ProviderCallContext {
            session: agent_runtime_core::ids::SessionId::new("session-test"),
            request_id: RequestId::new("request-1"),
            attempt_id: AttemptId::new("attempt-1"),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
        };
        let events = provider
            .stream(
                ProviderRequest::new(ModelId::new("gpt-5.6-terra"), vec![Message::user("hello")]),
                ctx,
            )
            .await
            .expect("stream")
            .collect::<Vec<_>>()
            .await;
        assert!(
            matches!(events.last(), Some(ProviderStreamEvent::Error { error })
            if error.kind == ProviderErrorKind::MalformedStream)
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ProviderStreamEvent::Finish { .. }))
        );
    }

    #[tokio::test]
    #[ignore = "requires explicitly injected ChatGPT test credentials and spends a bounded live request"]
    async fn live_chatgpt_responses() {
        let access_token = Secret::new(
            std::env::var("SMITH_CHATGPT_TEST_ACCESS_TOKEN")
                .expect("inject SMITH_CHATGPT_TEST_ACCESS_TOKEN explicitly"),
        );
        let account_id = std::env::var("SMITH_CHATGPT_TEST_ACCOUNT_ID")
            .expect("inject SMITH_CHATGPT_TEST_ACCOUNT_ID explicitly");
        let target = ProviderCredentialTarget::new("chatgpt").expect("target");
        let source = Arc::new(StaticProviderCredentialSource::new(access_token))
            as Arc<dyn ProviderCredentialSource>;
        let provider = ChatGptProvider::new(
            crate::transport::ReqwestTransport::new(Default::default()).expect("transport"),
            ChatGptProviderConfig::new(
                "gpt-5.6-terra",
                Capabilities::basic_streaming(),
                account_id,
            )
            .expect("config"),
            target,
            source,
        );
        let clock = SystemClock;
        let ctx = ProviderCallContext {
            session: agent_runtime_core::ids::SessionId::new("session-test"),
            request_id: RequestId::new("smith-live-chatgpt"),
            attempt_id: AttemptId::new("smith-live-chatgpt-1"),
            cancel: Cancellation::new(),
            deadline: Deadline::after(&clock, 180_000),
        };
        let mut request = ProviderRequest::new(
            ModelId::new("gpt-5.6-terra"),
            vec![Message::user("Reply with exactly: SMITH_OK")],
        );
        request.max_output_tokens = Some(16);
        let events = provider
            .stream(request, ctx)
            .await
            .expect("live stream")
            .collect::<Vec<_>>()
            .await;
        let text = events
            .iter()
            .filter_map(|event| match event {
                ProviderStreamEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(
            text.contains("SMITH_OK"),
            "live response did not contain the canary"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop
            }
        )));
    }
}
