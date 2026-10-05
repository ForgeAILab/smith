use super::*;

/// Parses the `x-codex-*` rate-limit header family the Codex backend attaches
/// to Responses replies.
///
/// Consumption arrives as a percentage per window. The backend names the
/// reset absolutely today (`…-reset-at`, Unix seconds); older gateways
/// reported a relative `…-reset-after-seconds`. Both are read, so whichever
/// the server sent survives normalization.
pub(super) fn codex_rate_limit_snapshot(headers: &[(String, String)]) -> RateLimitSnapshot {
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
pub(super) fn usage_snapshot_from_json(body: &[u8]) -> Result<RateLimitSnapshot, ChatGptAuthError> {
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
