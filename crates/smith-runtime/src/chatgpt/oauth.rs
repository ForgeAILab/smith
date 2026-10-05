use super::*;

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
