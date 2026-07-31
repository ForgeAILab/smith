//! Offline tests for Smith's production HTTP transport.
//!
//! The server is a hand-written HTTP/1.1 stub on a loopback listener rather than
//! a mock crate. That is not frugality: the interesting cases here are the ones
//! a polite mock hides — a body that stops mid-stream, a response larger than
//! the caller will accept, and a client that hangs up early. A raw socket can
//! stage all three, and the test suite gains no dependency for it.
//!
//! Every test binds `127.0.0.1:0`, so the suite runs with no network.

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use agent_runtime::provider::transport::{ByteStream, HttpRequest, HttpTransport};
use agent_runtime_core::provider::{ProviderError, ProviderErrorKind};
use futures_util::StreamExt;
use smith_runtime::transport::{ReqwestTransport, TransportConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// A token shaped like the real thing, so a substring assertion is meaningful.
const TOKEN: &str = "sk-live-9fH3qP0xZ2mB7tKcR1sVnE4d";

/// A credential smuggled in the URL's userinfo.
const URL_SECRET: &str = "sk-live-userinfo-Jd82nQmZ";

/// A credential smuggled in the URL's query string.
const QUERY_SECRET: &str = "sk-live-query-W71pLxCv";

/// The longest any single step of a test may take before it is a failure rather
/// than a hang.
const PATIENCE: Duration = Duration::from_secs(5);

// -- the stub server ---------------------------------------------------------

/// A single-connection HTTP/1.1 stub bound to an ephemeral loopback port.
struct Stub {
    addr: SocketAddr,
    served: JoinHandle<Vec<u8>>,
}

/// Starts a stub that accepts exactly one connection, reads the whole request,
/// and hands it plus the still-open socket to `handler`.
///
/// The join handle yields the raw request bytes, so a test can assert on what
/// reached the wire, and completes only once the handler returns — which is how
/// the cancellation test observes the client hanging up.
async fn stub<F, Fut>(handler: F) -> Stub
where
    F: FnOnce(Vec<u8>, TcpStream) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener");
    let addr = listener.local_addr().expect("a bound address");
    let served = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("an accepted connection");
        let request = read_request(&mut socket).await;
        handler(request.clone(), socket).await;
        request
    });
    Stub { addr, served }
}

/// Reads one request: the head, then exactly `content-length` body bytes.
async fn read_request(socket: &mut TcpStream) -> Vec<u8> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match socket.read(&mut byte).await {
            Ok(0) | Err(_) => return head,
            Ok(_) => head.push(byte[0]),
        }
    }
    let mut body = vec![0u8; content_length(&head)];
    socket
        .read_exact(&mut body)
        .await
        .expect("the request body");
    head.extend_from_slice(&body);
    head
}

fn content_length(head: &[u8]) -> usize {
    String::from_utf8_lossy(head)
        .to_ascii_lowercase()
        .lines()
        .find_map(|line| line.strip_prefix("content-length:")?.trim().parse().ok())
        .unwrap_or(0)
}

/// Writes a response head. `declared_length` is a parameter rather than derived
/// from the body so a test can promise more than it intends to send.
async fn write_head(
    socket: &mut TcpStream,
    status: &str,
    headers: &[(&str, &str)],
    declared_length: usize,
) {
    let mut head = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!("content-length: {declared_length}\r\n\r\n"));
    write_bytes(socket, head.as_bytes()).await;
}

async fn write_bytes(socket: &mut TcpStream, bytes: &[u8]) {
    socket.write_all(bytes).await.expect("a written response");
    socket.flush().await.expect("a flushed response");
}

/// Writes a complete, well-formed response.
async fn respond(socket: &mut TcpStream, status: &str, headers: &[(&str, &str)], body: &str) {
    write_head(socket, status, headers, body.len()).await;
    write_bytes(socket, body.as_bytes()).await;
}

/// Sends a partial body and then holds the connection until the client hangs
/// up, which is what a wedged provider looks like from this side.
async fn stall_after(socket: &mut TcpStream, prefix: &str) {
    write_head(
        socket,
        "200 OK",
        &[("content-type", "text/event-stream")],
        4096,
    )
    .await;
    write_bytes(socket, prefix.as_bytes()).await;
    let mut sink = [0u8; 64];
    // Resolves with 0 or an error once the client drops the connection.
    let _ = socket.read(&mut sink).await;
}

// -- client-side helpers -----------------------------------------------------

fn transport(config: TransportConfig) -> ReqwestTransport {
    ReqwestTransport::new(config).expect("a transport")
}

fn request_to(addr: SocketAddr) -> HttpRequest {
    HttpRequest {
        url: format!("http://{addr}/v1/chat/completions"),
        headers: vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("authorization".to_owned(), format!("Bearer {TOKEN}")),
        ],
        body: br#"{"model":"m","stream":true}"#.to_vec(),
    }
}

/// Fails the test rather than hanging forever.
async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(PATIENCE, future)
        .await
        .unwrap_or_else(|_| panic!("{what} did not finish within {PATIENCE:?}"))
}

async fn next_chunk(stream: &mut ByteStream) -> Option<Result<Vec<u8>, ProviderError>> {
    within("the next chunk", stream.next()).await
}

/// Runs one request against a stub answering with `status` and `headers`, and
/// returns the resulting classified error.
async fn classify(status: &'static str, headers: &[(&'static str, &'static str)]) -> ProviderError {
    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let stub = stub(move |_request, mut socket| async move {
        let borrowed: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        respond(&mut socket, status, &borrowed, r#"{"error":"denied"}"#).await;
    })
    .await;

    let transport = transport(TransportConfig::default());
    match within("the request", transport.post_stream(request_to(stub.addr))).await {
        // `ByteStream` is not `Debug`, so this cannot be an `expect_err`.
        Ok(_) => panic!("the transport accepted a `{status}` response"),
        Err(err) => err,
    }
}

fn endpoint_of(err: &ProviderError) -> String {
    err.metadata
        .get("transport.endpoint")
        .expect("an endpoint label")
        .to_string()
}

// -- streaming ---------------------------------------------------------------

#[tokio::test]
async fn response_bytes_reach_the_caller_before_the_server_sends_the_last_chunk() {
    let first = "data: {\"choices\":[{\"delta\":{\"content\":\"one\"}}]}\n\n";
    let last = "data: [DONE]\n\n";
    let (release, released) = oneshot::channel::<()>();

    let stub = stub(move |request, mut socket| async move {
        let text = String::from_utf8_lossy(&request).to_ascii_lowercase();
        assert!(
            text.starts_with("post /v1/chat/completions http/1.1"),
            "unexpected request line: {text}"
        );
        assert!(text.contains("authorization: bearer "), "{text}");
        assert!(text.ends_with(r#"{"model":"m","stream":true}"#), "{text}");

        write_head(
            &mut socket,
            "200 OK",
            &[("content-type", "text/event-stream")],
            first.len() + last.len(),
        )
        .await;
        write_bytes(&mut socket, first.as_bytes()).await;
        // The last chunk is withheld until the test has already been handed the
        // first, so a transport that buffered the body would deadlock here
        // rather than quietly pass.
        released.await.expect("the release signal");
        write_bytes(&mut socket, last.as_bytes()).await;
    })
    .await;

    let transport = transport(TransportConfig::default());
    let mut stream = within("the request", transport.post_stream(request_to(stub.addr)))
        .await
        .expect("a byte stream");

    let mut received = next_chunk(&mut stream)
        .await
        .expect("a first chunk")
        .expect("bytes rather than an error");
    assert!(!received.is_empty());
    assert!(
        first.as_bytes().starts_with(received.as_slice()),
        "the first delivery must come from the first write, not the whole body"
    );

    release.send(()).expect("the server is still waiting");
    while let Some(chunk) = next_chunk(&mut stream).await {
        received.extend(chunk.expect("bytes rather than an error"));
    }
    assert_eq!(
        String::from_utf8(received).expect("utf-8"),
        format!("{first}{last}")
    );
}

// -- status classification ---------------------------------------------------

#[tokio::test]
async fn an_unauthorized_response_is_an_auth_failure_that_is_not_retryable() {
    let err = classify("401 Unauthorized", &[]).await;
    assert_eq!(err.kind, ProviderErrorKind::Auth);
    assert!(!err.retryable);
    assert_eq!(err.metadata.get("http.status").unwrap().to_string(), "401");
}

#[tokio::test]
async fn a_forbidden_response_is_an_auth_failure() {
    let err = classify("403 Forbidden", &[]).await;
    assert_eq!(err.kind, ProviderErrorKind::Auth);
    assert!(!err.retryable);
}

#[tokio::test]
async fn a_bad_request_response_is_terminal() {
    let err = classify("400 Bad Request", &[]).await;
    assert_eq!(err.kind, ProviderErrorKind::BadRequest);
    assert!(!err.retryable);
}

#[tokio::test]
async fn a_not_found_response_is_terminal() {
    let err = classify("404 Not Found", &[]).await;
    assert_eq!(err.kind, ProviderErrorKind::BadRequest);
    assert!(!err.retryable);
}

#[tokio::test]
async fn a_request_timeout_response_is_a_retryable_timeout() {
    let err = classify("408 Request Timeout", &[]).await;
    assert_eq!(err.kind, ProviderErrorKind::Timeout);
    assert!(err.retryable);
}

#[tokio::test]
async fn a_rate_limited_response_honors_retry_after() {
    let err = classify("429 Too Many Requests", &[("retry-after", "7")]).await;
    assert_eq!(err.kind, ProviderErrorKind::RateLimited);
    assert!(err.retryable);
    assert_eq!(err.retry_after_ms, Some(7_000));
}

#[tokio::test]
async fn a_rate_limited_response_without_retry_after_is_still_retryable() {
    let err = classify("429 Too Many Requests", &[]).await;
    assert_eq!(err.kind, ProviderErrorKind::RateLimited);
    assert!(err.retryable);
    assert_eq!(err.retry_after_ms, None);
}

#[tokio::test]
async fn a_date_form_retry_after_is_ignored_rather_than_misread() {
    let err = classify(
        "429 Too Many Requests",
        &[("retry-after", "Wed, 21 Oct 2026 07:28:00 GMT")],
    )
    .await;
    assert!(err.retryable);
    assert_eq!(err.retry_after_ms, None);
}

#[tokio::test]
async fn a_server_error_response_is_retryable() {
    let err = classify("500 Internal Server Error", &[]).await;
    assert_eq!(err.kind, ProviderErrorKind::Server);
    assert!(err.retryable);
    assert_eq!(err.metadata.get("http.status").unwrap().to_string(), "500");
}

#[tokio::test]
async fn an_unavailable_response_is_retryable_and_honors_retry_after() {
    let err = classify("503 Service Unavailable", &[("retry-after", "2")]).await;
    assert_eq!(err.kind, ProviderErrorKind::Server);
    assert_eq!(err.retry_after_ms, Some(2_000));
}

#[tokio::test]
async fn a_redirect_is_refused_rather_than_followed_with_the_credential() {
    let err = classify(
        "302 Found",
        &[("location", "http://elsewhere.test/v1/chat/completions")],
    )
    .await;
    assert_eq!(err.kind, ProviderErrorKind::BadRequest);
    assert!(!err.retryable);
    assert!(err.message.contains("redirected"), "{}", err.message);
}

#[tokio::test]
async fn an_unreachable_endpoint_is_a_retryable_network_failure() {
    // Bound, its address noted, then closed: nothing is listening there.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
    let addr = listener.local_addr().expect("an address");
    drop(listener);

    let transport = transport(TransportConfig {
        connect_timeout: Duration::from_millis(500),
        ..TransportConfig::default()
    });
    let err = match within("the request", transport.post_stream(request_to(addr))).await {
        Ok(_) => panic!("a closed port answered"),
        Err(err) => err,
    };

    assert!(
        matches!(
            err.kind,
            ProviderErrorKind::Network | ProviderErrorKind::Timeout
        ),
        "{err:?}"
    );
    assert!(err.retryable);
}

// -- bounds, deadlines, and cancellation -------------------------------------

#[tokio::test]
async fn a_response_past_the_byte_cap_ends_the_stream_with_an_error() {
    let under_cap = "0123456789abcdef";
    let over_cap = "x".repeat(64);
    let (release, released) = oneshot::channel::<()>();

    let body_len = under_cap.len() + over_cap.len();
    let stub = stub(move |_request, mut socket| async move {
        write_head(&mut socket, "200 OK", &[], body_len).await;
        write_bytes(&mut socket, under_cap.as_bytes()).await;
        released.await.expect("the release signal");
        write_bytes(&mut socket, over_cap.as_bytes()).await;
    })
    .await;

    let transport = transport(TransportConfig {
        max_response_bytes: 32,
        ..TransportConfig::default()
    });
    let mut stream = within("the request", transport.post_stream(request_to(stub.addr)))
        .await
        .expect("a byte stream");

    let first = next_chunk(&mut stream)
        .await
        .expect("a first chunk")
        .expect("bytes under the cap");
    assert_eq!(first.len(), under_cap.len());

    release.send(()).expect("the server is still waiting");
    let err = next_chunk(&mut stream)
        .await
        .expect("a second item")
        .expect_err("the oversized chunk is refused");

    assert_eq!(err.kind, ProviderErrorKind::MalformedStream);
    assert!(!err.retryable, "a body that is too large stays too large");
    assert!(err.message.contains("byte limit"), "{}", err.message);
    // The error is terminal: nothing follows it.
    assert!(next_chunk(&mut stream).await.is_none());
}

#[tokio::test]
async fn the_overall_deadline_ends_a_stream_that_never_finishes() {
    let stub = stub(|_request, mut socket| async move {
        stall_after(&mut socket, "data: partial\n\n").await;
    })
    .await;

    let transport = transport(TransportConfig {
        request_deadline: Duration::from_millis(400),
        // Comfortably longer, so only the overall deadline can fire.
        stall_timeout: Duration::from_secs(30),
        ..TransportConfig::default()
    });
    let mut stream = within("the request", transport.post_stream(request_to(stub.addr)))
        .await
        .expect("a byte stream");

    let _first = next_chunk(&mut stream).await.expect("a first chunk");
    let err = next_chunk(&mut stream)
        .await
        .expect("a terminal item")
        .expect_err("the deadline fires");

    assert_eq!(err.kind, ProviderErrorKind::Timeout);
    assert!(err.retryable);
    assert!(err.message.contains("deadline"), "{}", err.message);
}

#[tokio::test]
async fn a_stalled_stream_ends_with_a_timeout() {
    let stub = stub(|_request, mut socket| async move {
        stall_after(&mut socket, "data: partial\n\n").await;
    })
    .await;

    let transport = transport(TransportConfig {
        stall_timeout: Duration::from_millis(300),
        // Comfortably longer, so only the stall timeout can fire.
        request_deadline: Duration::from_secs(30),
        ..TransportConfig::default()
    });
    let mut stream = within("the request", transport.post_stream(request_to(stub.addr)))
        .await
        .expect("a byte stream");

    let _first = next_chunk(&mut stream).await.expect("a first chunk");
    let err = next_chunk(&mut stream)
        .await
        .expect("a terminal item")
        .expect_err("the stall timeout fires");

    assert_eq!(err.kind, ProviderErrorKind::Timeout);
    assert!(err.retryable);
    assert!(err.message.contains("stopped sending"), "{}", err.message);
}

#[tokio::test]
async fn dropping_the_stream_closes_the_connection() {
    let stub = stub(|_request, mut socket| async move {
        stall_after(&mut socket, "data: partial\n\n").await;
    })
    .await;

    let transport = transport(TransportConfig::default());
    let mut stream = within("the request", transport.post_stream(request_to(stub.addr)))
        .await
        .expect("a byte stream");
    let _first = next_chunk(&mut stream).await.expect("a first chunk");

    // The transport stays alive; only the stream goes. The server's read
    // resolves solely because the in-flight request was actually aborted.
    drop(stream);

    within("the server observing the hang-up", stub.served)
        .await
        .expect("the stub server task");
}

// -- redaction ---------------------------------------------------------------

#[tokio::test]
async fn no_credential_from_the_headers_or_the_url_reaches_an_error() {
    let echo = format!(
        r#"{{"error":{{"message":"bad key Bearer {TOKEN}","url_user":"{URL_SECRET}","query":"{QUERY_SECRET}"}}}}"#
    );
    let challenge = format!(r#"Bearer error="invalid_token", token="{TOKEN}""#);
    let stub = stub(move |_request, mut socket| async move {
        respond(
            &mut socket,
            "401 Unauthorized",
            &[("www-authenticate", challenge.as_str())],
            &echo,
        )
        .await;
    })
    .await;

    let port = stub.addr.port();
    let request = HttpRequest {
        url: format!(
            "http://smith:{URL_SECRET}@127.0.0.1:{port}/v1/chat/completions?api_key={QUERY_SECRET}"
        ),
        headers: vec![("authorization".to_owned(), format!("Bearer {TOKEN}"))],
        body: br#"{"messages":[{"role":"user","content":"a private prompt"}]}"#.to_vec(),
    };

    let transport = transport(TransportConfig::default());
    let err = match within("the request", transport.post_stream(request)).await {
        Ok(_) => panic!("the transport accepted a 401"),
        Err(err) => err,
    };

    assert_eq!(err.kind, ProviderErrorKind::Auth);
    let surfaces = format!(
        "{} | {} | {:?} | {}",
        err.message,
        err,
        err,
        serde_json::to_string(&err).expect("a serializable error")
    );
    for secret in [TOKEN, URL_SECRET, QUERY_SECRET, "a private prompt"] {
        assert!(
            !surfaces.contains(secret),
            "`{secret}` leaked into an error surface: {surfaces}"
        );
    }
    // What remains is still enough to name the endpoint that failed.
    assert_eq!(endpoint_of(&err), format!("http://127.0.0.1:{port}"));
}

#[tokio::test]
async fn a_bearer_token_never_appears_in_a_debug_rendering() {
    let stub = stub(|_request, mut socket| async move {
        respond(&mut socket, "500 Internal Server Error", &[], "").await;
    })
    .await;

    let transport = transport(TransportConfig::default());
    let request = request_to(stub.addr);
    let rendered_request = format!("{request:?}");
    let rendered_transport = format!("{transport:?}");

    let err = match within("the request", transport.post_stream(request)).await {
        Ok(_) => panic!("the transport accepted a 500"),
        Err(err) => err,
    };

    for rendered in [&rendered_request, &rendered_transport, &format!("{err:?}")] {
        assert!(!rendered.contains(TOKEN), "the token leaked: {rendered}");
    }
    // The renderings are not merely empty.
    assert!(rendered_request.contains("authorization"));
    assert!(rendered_transport.contains("ReqwestTransport"));
}

#[tokio::test]
async fn an_unparsable_endpoint_is_reported_without_echoing_it() {
    let transport = transport(TransportConfig::default());
    let request = HttpRequest {
        url: format!("://api.example.test/v1/chat/completions?api_key={QUERY_SECRET}"),
        headers: vec![("authorization".to_owned(), format!("Bearer {TOKEN}"))],
        body: Vec::new(),
    };

    let err = match within("the request", transport.post_stream(request)).await {
        Ok(_) => panic!("the transport accepted an unparsable URL"),
        Err(err) => err,
    };

    assert_eq!(err.kind, ProviderErrorKind::BadRequest);
    let surfaces = format!("{err:?} {err}");
    assert!(!surfaces.contains(QUERY_SECRET), "{surfaces}");
    assert!(!surfaces.contains(TOKEN), "{surfaces}");
}

#[tokio::test]
async fn an_invalid_header_value_is_reported_by_name_only() {
    let transport = transport(TransportConfig::default());
    let request = HttpRequest {
        url: "http://127.0.0.1:1/v1/chat/completions".to_owned(),
        // A newline is not a legal header value.
        headers: vec![("authorization".to_owned(), format!("Bearer {TOKEN}\n"))],
        body: Vec::new(),
    };

    let err = match within("the request", transport.post_stream(request)).await {
        Ok(_) => panic!("the transport accepted an illegal header value"),
        Err(err) => err,
    };

    assert_eq!(err.kind, ProviderErrorKind::BadRequest);
    assert!(err.message.contains("authorization"), "{}", err.message);
    assert!(!format!("{err:?} {err}").contains(TOKEN));
}
