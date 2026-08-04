//! xAI browser login over the standard OAuth 2.0 device authorization grant.
//!
//! Grok authenticates by logging in, not by an API key, so a Smith user with a
//! subscription and no console key had no way in. This is the device grant
//! (RFC 8628) against xAI's issuer: Smith prints a code, the user approves it
//! in a browser, and Smith polls until a token comes back.
//!
//! Endpoints come from OIDC discovery rather than being hardcoded. The issuer
//! publishes `/.well-known/openid-configuration`, and reading it means a moved
//! endpoint is xAI's deployment detail rather than a Smith release.
//!
//! The client id is xAI's published native-app identifier. A native client
//! cannot hold a secret — RFC 8252 says so plainly — so this value is an
//! identifier rather than a credential, and the device grant needs no secret.
//! `XAI_OAUTH_CLIENT_ID` overrides it for anyone issued their own.

use std::fmt;
use std::time::Duration;

use agent_runtime_core::store::Secret;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// xAI's OAuth issuer.
pub const XAI_ISSUER: &str = "https://auth.x.ai";

/// xAI's published native-app client identifier.
pub const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

/// Environment override for a separately issued client.
pub const XAI_CLIENT_ID_ENV: &str = "XAI_OAUTH_CLIENT_ID";

/// The scopes a coding client needs.
///
/// `offline_access` is the load-bearing one: without it the issuer returns no
/// refresh token and the session dies at the first expiry.
pub const XAI_SCOPES: &[&str] = &["openid", "profile", "email", "offline_access", "api:access"];

/// Longest a login ceremony may run before Smith gives up.
const LOGIN_DEADLINE: Duration = Duration::from_secs(10 * 60);

/// Refresh this far ahead of expiry rather than waiting for a 401.
const REFRESH_SKEW_MS: u64 = 120_000;

/// An OAuth response larger than this is not one; refuse it before buffering.
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// Why an xAI login or refresh failed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum XaiAuthError {
    /// The issuer could not be reached.
    #[error("the xAI authorization server could not be reached")]
    Transport,
    /// The issuer published no usable discovery document.
    #[error("the xAI authorization server published no usable device-grant configuration")]
    Discovery,
    /// The issuer rejected the request.
    #[error("the xAI authorization server rejected this login")]
    Rejected,
    /// The user declined in the browser.
    #[error("the login was declined")]
    Declined,
    /// The user did not finish in time.
    #[error("the login expired before it was approved")]
    Expired,
    /// A response did not match the shape the grant requires.
    #[error("the xAI authorization server returned an unusable response")]
    InvalidResponse,
    /// The stored bundle is not usable.
    #[error("the stored xAI session is unusable; sign in again")]
    InvalidBundle,
}

/// The endpoints one issuer publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XaiEndpoints {
    /// Where a device code is requested.
    pub device_authorization: String,
    /// Where a device code is exchanged and a refresh is performed.
    pub token: String,
}

/// A pending device authorization awaiting the user's approval.
#[derive(Clone)]
pub struct XaiDeviceAuthorization {
    device_code: String,
    /// The code the user types.
    pub user_code: String,
    /// Where the user goes to approve.
    pub verification_url: String,
    /// A URL with the code already filled in, when the issuer offers one.
    pub verification_url_complete: Option<String>,
    /// How long to wait between polls.
    pub interval: Duration,
}

impl fmt::Debug for XaiDeviceAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("XaiDeviceAuthorization")
            .field("user_code", &self.user_code)
            .field("verification_url", &self.verification_url)
            .finish_non_exhaustive()
    }
}

impl Drop for XaiDeviceAuthorization {
    fn drop(&mut self) {
        self.device_code.zeroize();
    }
}

/// A stored xAI session.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XaiTokenBundle {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    expires_at_ms: u64,
}

impl fmt::Debug for XaiTokenBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("XaiTokenBundle([redacted])")
    }
}

impl Drop for XaiTokenBundle {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

impl XaiTokenBundle {
    /// Parses a bundle out of Smith's protected credential storage.
    pub fn from_secret(secret: &Secret) -> Result<Self, XaiAuthError> {
        let bundle: Self =
            serde_json::from_str(secret.expose()).map_err(|_| XaiAuthError::InvalidBundle)?;
        if bundle.access_token.is_empty() {
            return Err(XaiAuthError::InvalidBundle);
        }
        Ok(bundle)
    }

    /// Serializes the bundle for protected storage.
    pub fn to_secret(&self) -> Result<Secret, XaiAuthError> {
        serde_json::to_string(self)
            .map(Secret::new)
            .map_err(|_| XaiAuthError::InvalidBundle)
    }

    /// The bearer to send.
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Whether this session should be refreshed before the next request.
    ///
    /// Refreshing on a skew rather than on a 401 keeps a long turn from dying
    /// mid-stream on a token that expired while the model was thinking.
    pub fn needs_refresh(&self, now_ms: u64) -> bool {
        self.expires_at_ms <= now_ms.saturating_add(REFRESH_SKEW_MS)
    }

    /// Whether a refresh is even possible.
    pub fn can_refresh(&self) -> bool {
        !self.refresh_token.is_empty()
    }
}

/// Smith's xAI OAuth client.
pub struct XaiOAuthClient {
    client: Client,
    issuer: String,
    client_id: String,
}

impl fmt::Debug for XaiOAuthClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("XaiOAuthClient")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .finish()
    }
}

impl XaiOAuthClient {
    /// Builds a client against xAI's issuer, honoring the environment override.
    pub fn new() -> Result<Self, XaiAuthError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| XaiAuthError::Transport)?;
        Ok(Self {
            client,
            issuer: std::env::var("XAI_OAUTH_ISSUER").unwrap_or_else(|_| XAI_ISSUER.to_owned()),
            client_id: std::env::var(XAI_CLIENT_ID_ENV)
                .unwrap_or_else(|_| XAI_CLIENT_ID.to_owned()),
        })
    }

    /// Points the client at a different issuer, for tests and staging.
    pub fn with_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = issuer.into();
        self
    }

    /// Reads the issuer's published endpoints.
    pub async fn discover(&self) -> Result<XaiEndpoints, XaiAuthError> {
        #[derive(Deserialize)]
        struct Document {
            device_authorization_endpoint: Option<String>,
            token_endpoint: Option<String>,
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.issuer.trim_end_matches('/')
        );
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| XaiAuthError::Transport)?;
        if !response.status().is_success() {
            return Err(XaiAuthError::Discovery);
        }
        let document: Document = decode_json(response).await?;
        match (
            document.device_authorization_endpoint,
            document.token_endpoint,
        ) {
            (Some(device_authorization), Some(token))
                if !device_authorization.is_empty() && !token.is_empty() =>
            {
                Ok(XaiEndpoints {
                    device_authorization,
                    token,
                })
            }
            // An issuer without a device endpoint cannot serve a headless
            // login, and silently falling back to a browser redirect would
            // strand anyone on a remote shell.
            _ => Err(XaiAuthError::Discovery),
        }
    }

    /// Starts a device authorization.
    pub async fn request_device_code(
        &self,
        endpoints: &XaiEndpoints,
    ) -> Result<XaiDeviceAuthorization, XaiAuthError> {
        #[derive(Deserialize)]
        struct Response {
            device_code: String,
            user_code: String,
            verification_uri: Option<String>,
            verification_uri_complete: Option<String>,
            #[serde(default)]
            interval: Option<u64>,
        }
        let response = self
            .client
            .post(&endpoints.device_authorization)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("scope", &XAI_SCOPES.join(" ")),
            ])
            .send()
            .await
            .map_err(|_| XaiAuthError::Transport)?;
        if !response.status().is_success() {
            return Err(XaiAuthError::Rejected);
        }
        let body: Response = decode_json(response).await?;
        let verification_url = body
            .verification_uri
            .or_else(|| body.verification_uri_complete.clone())
            .ok_or(XaiAuthError::InvalidResponse)?;
        if body.device_code.is_empty() || body.user_code.is_empty() {
            return Err(XaiAuthError::InvalidResponse);
        }
        Ok(XaiDeviceAuthorization {
            device_code: body.device_code,
            user_code: body.user_code,
            verification_url,
            verification_url_complete: body.verification_uri_complete,
            // RFC 8628 makes the interval optional and defaults it to 5s.
            interval: Duration::from_secs(body.interval.unwrap_or(5).clamp(1, 60)),
        })
    }

    /// Polls until the user approves, declines, or the code expires.
    pub async fn complete_device_code(
        &self,
        endpoints: &XaiEndpoints,
        authorization: &XaiDeviceAuthorization,
        now_ms: u64,
    ) -> Result<XaiTokenBundle, XaiAuthError> {
        let deadline = tokio::time::Instant::now() + LOGIN_DEADLINE;
        let mut interval = authorization.interval;
        loop {
            let response = self
                .client
                .post(&endpoints.token)
                .form(&[
                    ("client_id", self.client_id.as_str()),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", authorization.device_code.as_str()),
                ])
                .send()
                .await
                .map_err(|_| XaiAuthError::Transport)?;

            if response.status().is_success() {
                return decode_token(response, now_ms).await;
            }

            // RFC 8628 carries the pending/slow-down/denied signal in the body,
            // not the status, so a 400 is routine rather than fatal here.
            let error = error_code(response).await;
            match error.as_deref() {
                Some("authorization_pending") => {}
                Some("slow_down") => interval = interval.saturating_add(Duration::from_secs(5)),
                Some("access_denied") => return Err(XaiAuthError::Declined),
                Some("expired_token") => return Err(XaiAuthError::Expired),
                _ => return Err(XaiAuthError::Rejected),
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(XaiAuthError::Expired);
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Exchanges a refresh token for a fresh session.
    pub async fn refresh(
        &self,
        endpoints: &XaiEndpoints,
        bundle: &XaiTokenBundle,
        now_ms: u64,
    ) -> Result<XaiTokenBundle, XaiAuthError> {
        if !bundle.can_refresh() {
            return Err(XaiAuthError::InvalidBundle);
        }
        let response = self
            .client
            .post(&endpoints.token)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", bundle.refresh_token.as_str()),
            ])
            .send()
            .await
            .map_err(|_| XaiAuthError::Transport)?;
        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(XaiAuthError::InvalidBundle);
        }
        if !response.status().is_success() {
            return Err(XaiAuthError::Rejected);
        }
        let mut refreshed = decode_token(response, now_ms).await?;
        // An issuer may rotate the refresh token or omit it on refresh; keeping
        // the prior one when none comes back is what stops a silent downgrade
        // to a session that cannot renew itself again.
        if refreshed.refresh_token.is_empty() {
            refreshed.refresh_token = bundle.refresh_token.clone();
        }
        Ok(refreshed)
    }
}

/// Buffers a bounded response body and decodes it.
async fn decode_json<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T, XaiAuthError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(XaiAuthError::InvalidResponse);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| XaiAuthError::Transport)?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(XaiAuthError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| XaiAuthError::InvalidResponse)
}

async fn decode_token(
    response: reqwest::Response,
    now_ms: u64,
) -> Result<XaiTokenBundle, XaiAuthError> {
    #[derive(Deserialize)]
    struct Token {
        access_token: String,
        #[serde(default)]
        refresh_token: String,
        #[serde(default)]
        expires_in: Option<u64>,
    }
    let token: Token = decode_json(response).await?;
    if token.access_token.is_empty() {
        return Err(XaiAuthError::InvalidResponse);
    }
    // An issuer that omits `expires_in` gets a conservative hour rather than a
    // token Smith believes is valid forever.
    let lifetime_ms = token.expires_in.unwrap_or(3_600).saturating_mul(1_000);
    Ok(XaiTokenBundle {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at_ms: now_ms.saturating_add(lifetime_ms),
    })
}

async fn error_code(response: reqwest::Response) -> Option<String> {
    #[derive(Deserialize)]
    struct Body {
        error: Option<String>,
    }
    let body: Body = decode_json(response).await.ok()?;
    body.error
}

/// The credential payload a configured provider stores.
pub fn bundle_from_json(raw: &str) -> Result<XaiTokenBundle, XaiAuthError> {
    serde_json::from_str(raw).map_err(|_| XaiAuthError::InvalidBundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(expires_at_ms: u64, refresh: &str) -> XaiTokenBundle {
        XaiTokenBundle {
            access_token: "token".into(),
            refresh_token: refresh.into(),
            expires_at_ms,
        }
    }

    #[test]
    fn a_bundle_round_trips_through_protected_storage_without_leaking_in_debug() {
        let original = bundle(1_000, "refresh");
        let secret = original.to_secret().expect("a secret");
        let parsed = XaiTokenBundle::from_secret(&secret).expect("a bundle");

        assert_eq!(parsed.access_token(), "token");
        assert!(parsed.can_refresh());
        assert_eq!(format!("{parsed:?}"), "XaiTokenBundle([redacted])");
    }

    #[test]
    fn a_bundle_without_an_access_token_is_refused() {
        let secret = Secret::new(r#"{"access_token":"","refresh_token":"r"}"#.to_owned());
        assert_eq!(
            XaiTokenBundle::from_secret(&secret),
            Err(XaiAuthError::InvalidBundle)
        );
    }

    #[test]
    fn refresh_is_due_before_expiry_rather_than_after_it() {
        // A token that expires mid-turn would fail a request the user already
        // paid for, so the skew is the point.
        let expiring = bundle(200_000, "r");
        assert!(!expiring.needs_refresh(0));
        assert!(expiring.needs_refresh(200_000 - REFRESH_SKEW_MS));
        assert!(expiring.needs_refresh(300_000));
    }

    #[test]
    fn a_session_with_no_refresh_token_cannot_renew() {
        assert!(!bundle(1_000, "").can_refresh());
    }

    #[test]
    fn the_client_id_is_overridable_for_a_separately_issued_client() {
        // Anyone with their own registration should not have to patch Smith.
        assert_eq!(XAI_CLIENT_ID_ENV, "XAI_OAUTH_CLIENT_ID");
        assert!(!XAI_CLIENT_ID.is_empty());
        assert!(XAI_SCOPES.contains(&"offline_access"));
    }
}
