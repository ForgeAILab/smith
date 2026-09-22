//! OpenAI Images API client sharing Smith's provider transport and credential
//! lease source.

use std::fmt;

use agent_runtime::provider::transport::{HttpRequest, HttpTransport};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::metadata::MetaValue;
use agent_runtime_core::provider::{ProviderError, ProviderErrorKind};
use agent_runtime_core::provider_credential::{
    CredentialInvalidation, ProviderAuthRejection, ProviderCredentialSource,
    ProviderCredentialTarget,
};
use async_trait::async_trait;
use base64::Engine as _;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use smith_tools::{ImageGenerationBackend, ImageGenerationRequest, MAX_GENERATED_PNG_BYTES};

const MAX_RESPONSE_JSON_BYTES: usize = 33 * 1024 * 1024;
const MINIMUM_CREDENTIAL_VALIDITY_MS: u64 = 30_000;

/// Images API adapter sharing the active provider credential lease.
pub struct ImagesApiBackend {
    endpoint: String,
    transport: std::sync::Arc<dyn HttpTransport>,
    target: ProviderCredentialTarget,
    credentials: std::sync::Arc<dyn ProviderCredentialSource>,
    chatgpt: bool,
}

impl fmt::Debug for ImagesApiBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImagesApiBackend")
            .field("endpoint", &self.endpoint)
            .field("target", &self.target)
            .field("chatgpt", &self.chatgpt)
            .finish_non_exhaustive()
    }
}

impl ImagesApiBackend {
    /// Builds an Images API backend for one OpenAI-authenticated provider.
    pub fn new(
        endpoint: impl Into<String>,
        transport: std::sync::Arc<dyn HttpTransport>,
        target: ProviderCredentialTarget,
        credentials: std::sync::Arc<dyn ProviderCredentialSource>,
        chatgpt: bool,
    ) -> Self {
        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_owned(),
            transport,
            target,
            credentials,
            chatgpt,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ImagesResponse {
    data: Vec<ImagesData>,
}

#[derive(Debug, Deserialize)]
struct ImagesData {
    b64_json: String,
}

#[async_trait]
impl ImageGenerationBackend for ImagesApiBackend {
    async fn generate(
        &self,
        request: ImageGenerationRequest,
        ctx: &agent_runtime_core::tool::InvocationContext,
    ) -> Result<Vec<u8>, RuntimeError> {
        let editing = !request.references.is_empty();
        let path = if editing {
            "images/edits"
        } else {
            "images/generations"
        };
        let url = format!("{}/{path}", self.endpoint);
        let mut body = json!({
            "prompt": request.prompt,
            "model": request.model,
            "quality": request.quality,
            "size": request.size,
            "background": "auto",
        });
        if editing {
            body["images"] = Value::Array(
                request
                    .references
                    .iter()
                    .map(|reference| json!({"image_url": reference.data_url}))
                    .collect(),
            );
        }
        let encoded_body = serde_json::to_vec(&body).map_err(|_| {
            RuntimeError::new(
                ErrorKind::Serialization,
                "image request could not be encoded",
            )
        })?;

        let mut renewed = false;
        loop {
            if ctx.should_stop() {
                return Err(RuntimeError::cancelled(
                    "image generation stopped before the provider request completed",
                ));
            }
            let lease = self
                .credentials
                .acquire(
                    &self.target,
                    MINIMUM_CREDENTIAL_VALIDITY_MS,
                    &ctx.cancel,
                    ctx.deadline,
                )
                .await
                .map_err(|_| {
                    RuntimeError::new(
                        ErrorKind::Provider,
                        "the active image-generation credential is unavailable",
                    )
                })?;
            let mut headers = vec![
                (
                    "authorization".to_owned(),
                    format!("Bearer {}", lease.secret().expose()),
                ),
                ("content-type".to_owned(), "application/json".to_owned()),
            ];
            if self.chatgpt {
                let account = lease.account().ok_or_else(|| {
                    RuntimeError::new(
                        ErrorKind::Provider,
                        "the active ChatGPT credential has no account identity",
                    )
                })?;
                headers.push(("chatgpt-account-id".to_owned(), account.to_owned()));
                headers.push(("originator".to_owned(), "smith".to_owned()));
            }
            let http_request = HttpRequest {
                url: url.clone(),
                headers,
                body: encoded_body.clone(),
            };
            let response = match self.transport.post_response(http_request).await {
                Ok(response) => response,
                Err(error) if !renewed && is_unauthorized(&error) => {
                    let invalidation = self
                        .credentials
                        .invalidate(
                            &self.target,
                            lease.revision(),
                            ProviderAuthRejection::Unauthorized,
                            &ctx.cancel,
                            ctx.deadline,
                        )
                        .await
                        .map_err(|_| provider_error(error.clone()))?;
                    if invalidation == CredentialInvalidation::ReplacementPossible {
                        renewed = true;
                        continue;
                    }
                    return Err(provider_error(error));
                }
                Err(error) => return Err(provider_error(error)),
            };
            let response_bytes = bounded_response(response.body).await?;
            let parsed: ImagesResponse = serde_json::from_slice(&response_bytes).map_err(|_| {
                RuntimeError::new(
                    ErrorKind::Provider,
                    "image provider returned an invalid response",
                )
            })?;
            let image = parsed.data.into_iter().next().ok_or_else(|| {
                RuntimeError::new(ErrorKind::Provider, "image provider returned no image")
            })?;
            let png = base64::engine::general_purpose::STANDARD
                .decode(image.b64_json.as_bytes())
                .map_err(|_| {
                    RuntimeError::new(
                        ErrorKind::Provider,
                        "image provider returned invalid encoded image data",
                    )
                })?;
            if png.len() > MAX_GENERATED_PNG_BYTES {
                return Err(RuntimeError::new(
                    ErrorKind::Provider,
                    "generated image exceeded the 24 MiB limit",
                ));
            }
            return Ok(png);
        }
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

async fn bounded_response(
    mut stream: agent_runtime::provider::transport::ByteStream,
) -> Result<Vec<u8>, RuntimeError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(provider_error)?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_JSON_BYTES {
            return Err(RuntimeError::new(
                ErrorKind::Provider,
                "image provider response exceeded the 33 MiB limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn is_unauthorized(error: &ProviderError) -> bool {
    error.kind == ProviderErrorKind::Auth
        && matches!(error.metadata.get("http.status"), Some(MetaValue::Int(401)))
}

fn provider_error(error: ProviderError) -> RuntimeError {
    let kind = match error.kind {
        ProviderErrorKind::Timeout => ErrorKind::Timeout,
        ProviderErrorKind::Cancelled => ErrorKind::Cancelled,
        ProviderErrorKind::BadRequest | ProviderErrorKind::Unsupported => ErrorKind::Config,
        _ => ErrorKind::Provider,
    };
    let mut mapped = RuntimeError::new(kind, error.message);
    mapped.retryable = error.retryable;
    mapped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use agent_runtime::provider::transport::{ByteStream, HttpResponse};
    use agent_runtime_core::cancel::Cancellation;
    use agent_runtime_core::clock::{Deadline, SystemClock};
    use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId};
    use agent_runtime_core::provider::{ProviderError, ProviderErrorKind};
    use agent_runtime_core::provider_credential::{
        ProviderCredentialError, ProviderCredentialLease, ProviderCredentialRevision,
    };
    use agent_runtime_core::store::Secret;
    use agent_runtime_core::tool::InvocationContext;
    use agent_runtime_core::workspace::DenyAllWorkspace;
    use futures_util::stream;
    use serde_json::json;

    #[derive(Debug)]
    struct FakeTransport {
        requests: Mutex<Vec<HttpRequest>>,
        responses: Mutex<VecDeque<Result<Vec<u8>, ProviderError>>>,
    }

    impl FakeTransport {
        fn new(responses: impl IntoIterator<Item = Result<Vec<u8>, ProviderError>>) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into_iter().collect()),
            }
        }

        fn requests(&self) -> Vec<HttpRequest> {
            self.requests.lock().expect("request lock").clone()
        }
    }

    #[async_trait]
    impl HttpTransport for FakeTransport {
        async fn post_stream(&self, request: HttpRequest) -> Result<ByteStream, ProviderError> {
            self.requests.lock().expect("request lock").push(request);
            let response = self
                .responses
                .lock()
                .expect("response lock")
                .pop_front()
                .expect("test response configured")?;
            Ok(Box::pin(stream::iter([Ok(response)])))
        }

        async fn post_response(&self, request: HttpRequest) -> Result<HttpResponse, ProviderError> {
            let body = self.post_stream(request).await?;
            Ok(HttpResponse::body_only(body))
        }
    }

    #[derive(Debug)]
    struct RotatingCredentials {
        leases: Mutex<VecDeque<ProviderCredentialLease>>,
        invalidations: Mutex<usize>,
    }

    #[async_trait]
    impl ProviderCredentialSource for RotatingCredentials {
        async fn acquire(
            &self,
            _target: &ProviderCredentialTarget,
            _minimum_validity_ms: u64,
            _cancel: &Cancellation,
            _deadline: Deadline,
        ) -> Result<ProviderCredentialLease, ProviderCredentialError> {
            self.leases
                .lock()
                .expect("lease lock")
                .pop_front()
                .ok_or(ProviderCredentialError::Unavailable)
        }

        async fn invalidate(
            &self,
            _target: &ProviderCredentialTarget,
            _rejected_revision: &ProviderCredentialRevision,
            _rejection: ProviderAuthRejection,
            _cancel: &Cancellation,
            _deadline: Deadline,
        ) -> Result<CredentialInvalidation, ProviderCredentialError> {
            *self.invalidations.lock().expect("invalidation lock") += 1;
            Ok(CredentialInvalidation::ReplacementPossible)
        }
    }

    fn context() -> InvocationContext {
        InvocationContext {
            session: SessionId::new("session"),
            turn: None,
            call_id: ToolCallId::new("call"),
            request: RequestId::new("request"),
            workspace: std::sync::Arc::new(DenyAllWorkspace),
            clock: std::sync::Arc::new(SystemClock),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
            output_limit: 4096,
        }
    }

    fn generation_request(references: Vec<smith_tools::ImageReference>) -> ImageGenerationRequest {
        ImageGenerationRequest {
            prompt: "A quiet mountain lake".into(),
            model: "gpt-image-2".into(),
            quality: "auto".into(),
            size: "auto".into(),
            references,
        }
    }

    fn success_response(png: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "data": [{
                "b64_json": base64::engine::general_purpose::STANDARD.encode(png)
            }]
        }))
        .expect("serialize response")
    }

    fn lease(token: &str, revision: &str, account: Option<&str>) -> ProviderCredentialLease {
        let lease = ProviderCredentialLease::non_expiring(
            Secret::new(token),
            ProviderCredentialRevision::new(revision).expect("revision"),
        );
        if let Some(account) = account {
            lease.with_account(account)
        } else {
            lease
        }
    }

    fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
        request
            .headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn request_body(request: &HttpRequest) -> Value {
        serde_json::from_slice(&request.body).expect("valid request JSON")
    }

    #[tokio::test]
    async fn response_body_is_bounded() {
        let bytes = vec![b'x'; MAX_RESPONSE_JSON_BYTES + 1];
        let stream = Box::pin(stream::iter([Ok::<_, ProviderError>(bytes)]));
        let error = bounded_response(stream).await.unwrap_err();
        assert!(error.message.contains("33 MiB"));
    }

    #[tokio::test]
    async fn generation_posts_expected_payload_and_platform_auth() {
        let transport = std::sync::Arc::new(FakeTransport::new([Ok(success_response(b"png"))]));
        let credentials = std::sync::Arc::new(RotatingCredentials {
            leases: Mutex::new(VecDeque::from([lease("platform-key", "v1", None)])),
            invalidations: Mutex::new(0),
        });
        let backend = ImagesApiBackend::new(
            "https://api.openai.com/v1/",
            transport.clone(),
            ProviderCredentialTarget::new("openai").expect("target"),
            credentials,
            false,
        );

        assert_eq!(
            backend
                .generate(generation_request(Vec::new()), &context())
                .await
                .expect("generation"),
            b"png"
        );

        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].url,
            "https://api.openai.com/v1/images/generations"
        );
        assert_eq!(
            header(&requests[0], "authorization"),
            Some("Bearer platform-key")
        );
        let body = request_body(&requests[0]);
        assert_eq!(body["prompt"], "A quiet mountain lake");
        assert_eq!(body["model"], "gpt-image-2");
        assert_eq!(body["quality"], "auto");
        assert_eq!(body["size"], "auto");
        assert_eq!(body["background"], "auto");
        assert!(body.get("images").is_none());
    }

    #[tokio::test]
    async fn edit_posts_reference_data_urls_and_chatgpt_headers() {
        let transport = std::sync::Arc::new(FakeTransport::new([Ok(success_response(b"png"))]));
        let credentials = std::sync::Arc::new(RotatingCredentials {
            leases: Mutex::new(VecDeque::from([lease(
                "chatgpt-token",
                "v1",
                Some("account-17"),
            )])),
            invalidations: Mutex::new(0),
        });
        let backend = ImagesApiBackend::new(
            "https://chatgpt.com/backend-api/codex",
            transport.clone(),
            ProviderCredentialTarget::new("chatgpt").expect("target"),
            credentials,
            true,
        );
        let references = vec![
            smith_tools::ImageReference {
                data_url: "data:image/png;base64,Zmlyc3Q=".into(),
            },
            smith_tools::ImageReference {
                data_url: "data:image/jpeg;base64,c2Vjb25k".into(),
            },
        ];

        backend
            .generate(generation_request(references), &context())
            .await
            .expect("edit");

        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].url,
            "https://chatgpt.com/backend-api/codex/images/edits"
        );
        assert_eq!(
            header(&requests[0], "authorization"),
            Some("Bearer chatgpt-token")
        );
        assert_eq!(
            header(&requests[0], "chatgpt-account-id"),
            Some("account-17")
        );
        assert_eq!(header(&requests[0], "originator"), Some("smith"));
        let body = request_body(&requests[0]);
        assert_eq!(body["background"], "auto");
        assert_eq!(
            body["images"][0]["image_url"],
            "data:image/png;base64,Zmlyc3Q="
        );
        assert_eq!(
            body["images"][1]["image_url"],
            "data:image/jpeg;base64,c2Vjb25k"
        );
    }

    #[tokio::test]
    async fn unauthorized_chatgpt_request_rotates_the_active_lease_once() {
        let mut unauthorized = ProviderError::new(ProviderErrorKind::Auth, "unauthorized");
        unauthorized.metadata.insert("http.status", 401_i64);
        let transport = std::sync::Arc::new(FakeTransport::new([
            Err(unauthorized),
            Ok(success_response(b"png")),
        ]));
        let credentials = std::sync::Arc::new(RotatingCredentials {
            leases: Mutex::new(VecDeque::from([
                lease("old-token", "v1", Some("account-old")),
                lease("new-token", "v2", Some("account-new")),
            ])),
            invalidations: Mutex::new(0),
        });
        let backend = ImagesApiBackend::new(
            "https://chatgpt.com/backend-api/codex",
            transport.clone(),
            ProviderCredentialTarget::new("chatgpt").expect("target"),
            credentials.clone(),
            true,
        );

        assert_eq!(
            backend
                .generate(generation_request(Vec::new()), &context())
                .await
                .expect("retry with replacement"),
            b"png"
        );

        let requests = transport.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            header(&requests[0], "authorization"),
            Some("Bearer old-token")
        );
        assert_eq!(
            header(&requests[0], "chatgpt-account-id"),
            Some("account-old")
        );
        assert_eq!(
            header(&requests[1], "authorization"),
            Some("Bearer new-token")
        );
        assert_eq!(
            header(&requests[1], "chatgpt-account-id"),
            Some("account-new")
        );
        assert_eq!(
            *credentials.invalidations.lock().expect("invalidation lock"),
            1
        );
    }

    #[tokio::test]
    async fn provider_errors_keep_retry_classification() {
        let transport = std::sync::Arc::new(FakeTransport::new([Err(ProviderError::new(
            ProviderErrorKind::RateLimited,
            "rate limited",
        )
        .retryable())]));
        let credentials = std::sync::Arc::new(RotatingCredentials {
            leases: Mutex::new(VecDeque::from([lease("key", "v1", None)])),
            invalidations: Mutex::new(0),
        });
        let backend = ImagesApiBackend::new(
            "https://api.openai.com/v1",
            transport,
            ProviderCredentialTarget::new("openai").expect("target"),
            credentials,
            false,
        );

        let error = backend
            .generate(generation_request(Vec::new()), &context())
            .await
            .expect_err("provider error");
        assert_eq!(error.kind, ErrorKind::Provider);
        assert!(error.retryable);
        assert_eq!(error.message, "rate limited");
    }

    #[test]
    fn provider_failures_keep_retry_classification() {
        let mut provider = ProviderError::new(ProviderErrorKind::RateLimited, "rate limited");
        provider.retryable = true;
        let mapped = provider_error(provider);
        assert_eq!(mapped.kind, ErrorKind::Provider);
        assert!(mapped.retryable);
    }
}
