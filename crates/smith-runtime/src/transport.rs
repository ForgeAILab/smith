//! Smith's production streaming HTTP transport.
//!
//! Agent Runtime's OpenAI-compatible adapter owns request serialization and SSE
//! normalization, but it takes its bytes from an injected [`HttpTransport`].
//! That split is deliberate: the shared runtime carries no networking
//! dependency and every runtime test runs offline. This module is the other
//! half of the split — the one place in Smith that opens a socket to a model
//! provider.
//!
//! Four properties drive the implementation, and each is a requirement rather
//! than a nicety:
//!
//! * **Nothing is buffered.** An answer arrives token by token, so response
//!   bytes are forwarded chunk by chunk as they land. Collecting the body first
//!   would trade the entire streaming experience for one convenient `await`.
//! * **Dropping the stream cancels the request.** The returned stream owns the
//!   in-flight response and nothing runs on a detached task, so a dropped
//!   stream drops the connection and the provider stops billing.
//! * **Bodies are bounded.** A provider that never stops sending would
//!   otherwise grow memory until the process dies, so the stream ends with a
//!   classified error at a configured byte cap.
//! * **No credential escapes.** Header values, request and response bodies, and
//!   credential-bearing URL components never reach `Debug`, `Display`, an error
//!   message, or a `tracing` field. In particular [`reqwest::Error`]'s own
//!   `Display` embeds the request URL, so a `reqwest` failure is *classified*,
//!   never formatted.
//!
//! Credential resolution and provider construction live elsewhere: this module
//! attaches whatever headers the adapter hands it and never reads Smith
//! configuration.

use std::fmt;
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_runtime::provider::ratelimit;
use agent_runtime::provider::transport::{ByteStream, HttpRequest, HttpResponse, HttpTransport};
use agent_runtime_core::provider::{ProviderError, ProviderErrorKind};
use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, RETRY_AFTER};
use reqwest::{Client, StatusCode, Url};
use tokio::time::Instant;

/// The default time allowed to establish a connection.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The default limit on one whole request, from the first byte sent to the last
/// byte of the response body. Generous, because a reasoning model can legibly
/// stream for minutes.
const DEFAULT_REQUEST_DEADLINE: Duration = Duration::from_secs(300);

/// The default limit on silence *between* two response chunks. This is the
/// bound that actually catches a wedged connection: the overall deadline is too
/// coarse to notice a stream that stopped one minute in.
const DEFAULT_STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The default cap on total response bytes. A conforming SSE turn is orders of
/// magnitude smaller; this exists so a malfunctioning or hostile endpoint
/// cannot exhaust memory.
const DEFAULT_MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// The metadata key carrying the redaction-safe endpoint label.
const ENDPOINT_KEY: &str = "transport.endpoint";

/// The metadata key carrying the HTTP status of a rejected request.
const STATUS_KEY: &str = "http.status";

/// How a [`ReqwestTransport`] behaves on the wire.
///
/// The knobs are limited on purpose. There is no proxy setting and no TLS
/// verification switch: a transport that can be told to skip certificate
/// validation eventually is told to, and the resulting configuration outlives
/// the debugging session that introduced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportConfig {
    /// How long a connection may take to establish. Defaults to 10 seconds.
    pub connect_timeout: Duration,
    /// How long one request may take end to end, including the streamed body.
    /// Defaults to 5 minutes.
    pub request_deadline: Duration,
    /// How long the response may go without producing a byte before the stream
    /// fails. Defaults to 60 seconds.
    pub stall_timeout: Duration,
    /// The most response body bytes accepted before the stream fails. Defaults
    /// to 64 MiB.
    pub max_response_bytes: u64,
    /// The `user-agent` sent with every request. Defaults to `smith/<version>`.
    pub user_agent: String,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_deadline: DEFAULT_REQUEST_DEADLINE,
            stall_timeout: DEFAULT_STALL_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            user_agent: concat!("smith/", env!("CARGO_PKG_VERSION")).to_owned(),
        }
    }
}

/// The production streaming HTTP transport, backed by `reqwest`.
pub struct ReqwestTransport {
    client: Client,
    config: TransportConfig,
}

// Hand-written rather than derived: `reqwest::Client`'s own `Debug` renders the
// default headers it was built with, and this type must never be the thing that
// prints a header value.
impl fmt::Debug for ReqwestTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReqwestTransport")
            .field("connect_timeout", &self.config.connect_timeout)
            .field("request_deadline", &self.config.request_deadline)
            .field("stall_timeout", &self.config.stall_timeout)
            .field("max_response_bytes", &self.config.max_response_bytes)
            .finish_non_exhaustive()
    }
}

impl ReqwestTransport {
    /// Builds a transport from `config`.
    ///
    /// Fails only if the configuration cannot produce a client — in practice, a
    /// user agent that is not a legal header value.
    pub fn new(config: TransportConfig) -> Result<Self, ProviderError> {
        let client = Client::builder()
            .connect_timeout(config.connect_timeout)
            .user_agent(config.user_agent.clone())
            // Redirects are not followed. A 3xx on an authenticated POST is a
            // misconfigured endpoint, and following it would forward the
            // authorization header to a host the operator never named.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                // The builder error can quote the offending value, which is why
                // it is classified rather than formatted.
                ProviderError::new(
                    ProviderErrorKind::BadRequest,
                    "the HTTP client could not be built from the transport configuration",
                )
            })?;
        Ok(Self { client, config })
    }

    /// The configuration this transport was built with.
    pub fn config(&self) -> &TransportConfig {
        &self.config
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn post_stream(&self, request: HttpRequest) -> Result<ByteStream, ProviderError> {
        Ok(self.send(request).await?.body)
    }

    async fn post_response(&self, request: HttpRequest) -> Result<HttpResponse, ProviderError> {
        self.send(request).await
    }
}

impl ReqwestTransport {
    /// Performs the request, surfacing what the server reported about itself.
    ///
    /// Both trait methods route through here so the header-observing path and
    /// the body-only one cannot drift: a transport that classified a spent
    /// window on one and not the other would make rotation depend on which
    /// method an adapter happened to call.
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, ProviderError> {
        // Parsed here rather than inside `reqwest` so that a malformed URL fails
        // with a message this module controls; `reqwest`'s would echo it back.
        let url = Url::parse(&request.url).map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::BadRequest,
                "the provider endpoint is not a valid absolute URL",
            )
        })?;
        let endpoint = endpoint_label(&url);
        let headers = header_map(&request.headers, &endpoint)?;

        // One absolute instant governs both the handshake and the body, so a
        // slow response cannot restart the clock by trickling.
        let deadline = Instant::now() + self.config.request_deadline;
        let send = self
            .client
            .post(url)
            .headers(headers)
            .body(request.body)
            .send();

        let response = match tokio::time::timeout_at(deadline, send).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => return Err(send_error(&err, &endpoint)),
            Err(_) => {
                return Err(fail(
                    ProviderErrorKind::Timeout,
                    "the provider request deadline elapsed before a response arrived",
                    &endpoint,
                )
                .retryable());
            }
        };

        let status = response.status();
        // Only the rate-limit families are carried forward. Copying every
        // header would hand the adapter `set-cookie` and whatever else a
        // gateway attached, none of which it has a use for and some of which
        // is credential-bearing.
        let observed = observed_headers(response.headers());

        if let Some(err) = classify_status(status, response.headers(), &endpoint) {
            tracing::debug!(
                status = status.as_u16(),
                endpoint = %endpoint,
                "the provider rejected the request"
            );
            // `response` is dropped unread on this path, and that is the point:
            // a provider error body commonly echoes the offending request,
            // authorization header included.
            return Err(exhaustion_aware(err, status.as_u16(), &observed));
        }

        Ok(HttpResponse {
            status: status.as_u16(),
            headers: observed,
            body: bounded(
                response,
                endpoint,
                deadline,
                self.config.stall_timeout,
                self.config.max_response_bytes,
            ),
        })
    }
}

/// The rate-limit headers, lowercased, and nothing else.
fn observed_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| {
            let name = name.as_str();
            name.starts_with("x-ratelimit-")
                || name.starts_with("anthropic-ratelimit-")
                || name.starts_with("x-codex-")
        })
        .filter_map(|(name, value)| {
            // A header whose value is not UTF-8 is dropped rather than
            // lossily decoded: a mangled number is worse than an absent one.
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect()
}

/// Re-classifies a rejection as a spent usage window when the headers say so.
///
/// The shared classifier owns the judgement — a 429 whose reset is seconds
/// away is still a throttle the retry policy should handle, and only a window
/// the provider reports as consumed, or a reset far enough out, means the
/// account is done until it resets.
fn exhaustion_aware(
    err: ProviderError,
    status: u16,
    headers: &[(String, String)],
) -> ProviderError {
    let snapshot = ratelimit::snapshot_from_headers(headers);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    match ratelimit::classify_rejection(status, &snapshot, err.retry_after_ms, now_ms) {
        Some(exhaustion) => ratelimit::apply_exhaustion(err, exhaustion),
        None => err,
    }
}

/// The state carried between chunks of one bounded response stream.
struct BodyState {
    body: Pin<Box<dyn Stream<Item = Result<Vec<u8>, reqwest::Error>> + Send>>,
    endpoint: String,
    deadline: Instant,
    stall_timeout: Duration,
    remaining: u64,
    done: bool,
}

impl BodyState {
    /// Marks the stream terminal so the error just yielded is its last item.
    fn finished(mut self) -> Self {
        self.done = true;
        self
    }
}

/// Wraps a response body in the deadline, stall, and byte-cap checks.
///
/// Everything stays inside the returned stream — no task is spawned — because
/// that is what makes a dropped stream abort the in-flight request instead of
/// leaving it running in the background.
fn bounded(
    response: reqwest::Response,
    endpoint: String,
    deadline: Instant,
    stall_timeout: Duration,
    max_response_bytes: u64,
) -> ByteStream {
    let body = response
        .bytes_stream()
        .map(|chunk| chunk.map(|bytes| bytes.to_vec()));
    let state = BodyState {
        body: Box::pin(body),
        endpoint,
        deadline,
        stall_timeout,
        remaining: max_response_bytes,
        done: false,
    };
    Box::pin(futures_util::stream::unfold(state, next_chunk))
}

/// Yields the next response chunk, or the classified error that ends the stream.
async fn next_chunk(mut state: BodyState) -> Option<(Result<Vec<u8>, ProviderError>, BodyState)> {
    if state.done {
        return None;
    }

    // Whichever bound expires first wins, and the two are distinguished so the
    // caller can tell "the provider went quiet" from "this turn ran too long".
    let stall_at = Instant::now() + state.stall_timeout;
    let wake_at = stall_at.min(state.deadline);

    let chunk = match tokio::time::timeout_at(wake_at, state.body.next()).await {
        Ok(Some(Ok(chunk))) => chunk,
        Ok(Some(Err(err))) => {
            let err = body_error(&err, &state.endpoint);
            return Some((Err(err), state.finished()));
        }
        Ok(None) => return None,
        Err(_) => {
            let message = if state.deadline <= stall_at {
                "the provider request deadline elapsed mid-stream"
            } else {
                "the provider stopped sending response bytes"
            };
            let err = fail(ProviderErrorKind::Timeout, message, &state.endpoint).retryable();
            return Some((Err(err), state.finished()));
        }
    };

    let len = chunk.len() as u64;
    if len > state.remaining {
        tracing::debug!(
            endpoint = %state.endpoint,
            "the provider response exceeded the configured byte limit"
        );
        // The oversized chunk is dropped rather than appended: the whole point
        // of the cap is that memory stops growing here.
        let err = fail(
            ProviderErrorKind::MalformedStream,
            "the provider response exceeded the configured byte limit",
            &state.endpoint,
        );
        return Some((Err(err), state.finished()));
    }
    state.remaining -= len;
    Some((Ok(chunk), state))
}

/// Classifies a non-success status, or returns `None` for 2xx.
///
/// The classification feeds Agent Runtime's shared retry policy, so retryability
/// is set to match what that policy treats as worth another attempt.
fn classify_status(
    status: StatusCode,
    headers: &HeaderMap,
    endpoint: &str,
) -> Option<ProviderError> {
    if status.is_success() {
        return None;
    }

    let mut err = match status.as_u16() {
        401 | 403 => fail(
            ProviderErrorKind::Auth,
            "the provider rejected the credential",
            endpoint,
        ),
        408 => fail(
            ProviderErrorKind::Timeout,
            "the provider timed out receiving the request",
            endpoint,
        )
        .retryable(),
        429 => fail(
            ProviderErrorKind::RateLimited,
            "the provider rate-limited the request",
            endpoint,
        )
        .retryable(),
        300..=399 => fail(
            ProviderErrorKind::BadRequest,
            "the provider endpoint redirected, which is not followed on an authenticated request",
            endpoint,
        ),
        400..=499 => fail(
            ProviderErrorKind::BadRequest,
            "the provider rejected the request",
            endpoint,
        ),
        500..=599 => fail(
            ProviderErrorKind::Server,
            "the provider reported a server error",
            endpoint,
        )
        .retryable(),
        _ => fail(
            ProviderErrorKind::BadRequest,
            "the provider returned an unexpected status",
            endpoint,
        ),
    };

    err.metadata.insert(STATUS_KEY, u64::from(status.as_u16()));
    // A hint is only meaningful where another attempt is planned, and the shared
    // policy already applies a rate-limit floor when none arrives.
    if err.retryable
        && let Some(ms) = retry_after_ms(headers)
    {
        err = err.retry_after(ms);
    }
    Some(err)
}

/// Reads `Retry-After` as a delay in milliseconds.
///
/// Only the delta-seconds form is honored. The HTTP-date form would need a date
/// parser this crate does not carry, and losing the hint degrades to the shared
/// policy's own backoff rather than to a wrong delay.
fn retry_after_ms(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|seconds| seconds.saturating_mul(1_000))
}

/// Classifies a failure to obtain response headers.
fn send_error(err: &reqwest::Error, endpoint: &str) -> ProviderError {
    if err.is_timeout() {
        fail(
            ProviderErrorKind::Timeout,
            "the provider connection timed out",
            endpoint,
        )
        .retryable()
    } else if err.is_connect() {
        fail(
            ProviderErrorKind::Network,
            "the provider endpoint could not be reached",
            endpoint,
        )
        .retryable()
    } else if err.is_builder() {
        fail(
            ProviderErrorKind::BadRequest,
            "the provider request could not be built",
            endpoint,
        )
    } else {
        fail(
            ProviderErrorKind::Network,
            "the provider request failed in transport",
            endpoint,
        )
        .retryable()
    }
}

/// Classifies a failure part-way through the response body.
///
/// A truncated body is reported as a retryable network fault rather than
/// `MalformedStream`, because the bytes that did arrive were well-formed: the
/// connection died, and another attempt can legitimately succeed.
fn body_error(err: &reqwest::Error, endpoint: &str) -> ProviderError {
    if err.is_timeout() {
        fail(
            ProviderErrorKind::Timeout,
            "the provider response timed out mid-stream",
            endpoint,
        )
        .retryable()
    } else {
        fail(
            ProviderErrorKind::Network,
            "the provider response ended before it completed",
            endpoint,
        )
        .retryable()
    }
}

/// Builds a provider error carrying only redaction-safe context.
fn fail(kind: ProviderErrorKind, message: impl Into<String>, endpoint: &str) -> ProviderError {
    let mut err = ProviderError::new(kind, message);
    err.metadata.insert(ENDPOINT_KEY, endpoint.to_owned());
    err
}

/// Renders a URL as scheme, host, and port only.
///
/// Userinfo, path, query, and fragment are dropped, because every one of them
/// is a place an API key is known to be carried. What remains still answers the
/// question an operator actually asks of a failure: which endpoint.
fn endpoint_label(url: &Url) -> String {
    let mut label = String::with_capacity(32);
    label.push_str(url.scheme());
    label.push_str("://");
    label.push_str(url.host_str().unwrap_or("unknown"));
    if let Some(port) = url.port() {
        label.push(':');
        label.push_str(&port.to_string());
    }
    label
}

/// Converts the adapter's header pairs into a header map.
///
/// A rejected header reports its *name* only. The name is not a secret — the
/// shared [`HttpRequest`] `Debug` prints names and redacts values for the same
/// reason — and the value is the one place a credential lives.
fn header_map(headers: &[(String, String)], endpoint: &str) -> Result<HeaderMap, ProviderError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            fail(
                ProviderErrorKind::BadRequest,
                format!("`{name}` is not a valid header name"),
                endpoint,
            )
        })?;
        let value = HeaderValue::from_str(value).map_err(|_| {
            fail(
                ProviderErrorKind::BadRequest,
                format!("the value of header `{header}` is not a valid header value"),
                endpoint,
            )
        })?;
        // Appended, not inserted: a repeated header name is legal and the
        // adapter may rely on all of its values arriving.
        map.append(header, value);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "sk-live-4kQm2ZpX8vRt7nLb1cWs9aYe";

    #[test]
    fn an_endpoint_label_drops_userinfo_path_and_query() {
        let url = Url::parse(&format!(
            "https://smith:{TOKEN}@api.example.test:8443/v1/chat/completions?api_key={TOKEN}"
        ))
        .expect("a parsable url");

        let label = endpoint_label(&url);
        assert_eq!(label, "https://api.example.test:8443");
        assert!(!label.contains(TOKEN));
    }

    #[test]
    fn an_endpoint_label_omits_a_default_port() {
        let url = Url::parse("https://api.example.test/v1").expect("a parsable url");
        assert_eq!(endpoint_label(&url), "https://api.example.test");
    }

    #[test]
    fn a_rejected_header_value_is_reported_by_name_only() {
        // A newline is not a legal header value, so this fails on the value.
        let headers = vec![("authorization".to_owned(), format!("Bearer {TOKEN}\n"))];
        let err = header_map(&headers, "https://api.example.test").unwrap_err();

        assert_eq!(err.kind, ProviderErrorKind::BadRequest);
        assert!(err.message.contains("authorization"));
        assert!(!format!("{err:?} {err}").contains(TOKEN));
    }

    #[test]
    fn retry_after_reads_only_the_delta_seconds_form() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("30"));
        assert_eq!(retry_after_ms(&headers), Some(30_000));

        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
        assert_eq!(retry_after_ms(&headers), None);
    }

    #[test]
    fn the_transport_debug_rendering_omits_the_user_agent_header() {
        let transport = ReqwestTransport::new(TransportConfig {
            user_agent: format!("smith/{TOKEN}"),
            ..TransportConfig::default()
        })
        .expect("a transport");

        let rendered = format!("{transport:?}");
        assert!(!rendered.contains(TOKEN), "{rendered}");
        assert!(rendered.contains("max_response_bytes"));
    }

    #[test]
    fn only_rate_limit_headers_are_carried_to_the_adapter() {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining-tokens", HeaderValue::from_static("40"));
        headers.insert(
            "anthropic-ratelimit-unified-reset",
            HeaderValue::from_static("2026-08-04T17:00:00Z"),
        );
        headers.insert("x-codex-primary-used-percent", HeaderValue::from_static("82"));
        headers.insert("set-cookie", HeaderValue::from_static("session=secret"));
        headers.insert("content-type", HeaderValue::from_static("text/event-stream"));

        let observed = observed_headers(&headers);
        let names: Vec<&str> = observed.iter().map(|(name, _)| name.as_str()).collect();

        assert!(names.contains(&"x-ratelimit-remaining-tokens"));
        assert!(names.contains(&"anthropic-ratelimit-unified-reset"));
        assert!(names.contains(&"x-codex-primary-used-percent"));
        // A gateway's cookie is credential-bearing and no adapter's business.
        assert!(!names.contains(&"set-cookie"));
        assert!(!names.contains(&"content-type"));
        assert!(!format!("{observed:?}").contains("secret"));
    }

    #[test]
    fn a_spent_window_is_reclassified_as_exhaustion() {
        let rejected = fail(
            ProviderErrorKind::RateLimited,
            "the provider rate-limited the request",
            "https://api.example.test",
        )
        .retryable();
        let headers = vec![
            ("x-ratelimit-limit-tokens".to_owned(), "10000".to_owned()),
            ("x-ratelimit-remaining-tokens".to_owned(), "0".to_owned()),
            ("x-ratelimit-reset-tokens".to_owned(), "1h30m0s".to_owned()),
        ];

        let err = exhaustion_aware(rejected, 429, &headers);
        assert_eq!(err.kind, ProviderErrorKind::LimitExhausted);
        // A spent window does not reopen because a backoff elapsed.
        assert!(!err.retryable);
    }

    #[test]
    fn a_short_throttle_keeps_its_retryable_classification() {
        let rejected = fail(
            ProviderErrorKind::RateLimited,
            "the provider rate-limited the request",
            "https://api.example.test",
        )
        .retry_after(5_000);
        let headers = vec![("x-ratelimit-reset-tokens".to_owned(), "5s".to_owned())];

        let err = exhaustion_aware(rejected, 429, &headers);
        // Still the retry policy's business, exactly as before this change.
        assert_eq!(err.kind, ProviderErrorKind::RateLimited);
        assert!(err.retryable);
    }

    #[test]
    fn a_rejection_without_limit_headers_is_left_alone() {
        let rejected = fail(
            ProviderErrorKind::Auth,
            "the provider rejected the credential",
            "https://api.example.test",
        );
        let err = exhaustion_aware(rejected, 401, &[]);
        assert_eq!(err.kind, ProviderErrorKind::Auth);
    }
}
