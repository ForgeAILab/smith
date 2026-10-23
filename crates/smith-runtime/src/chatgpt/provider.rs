use super::*;
use agent_runtime::registry::RegistryRevision;

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
        capabilities.hosted_web_search = true;
        capabilities.reasoning = ReasoningSupport::Controllable;
        capabilities.usage = true;
        // A model-scoped explicit unsupported declaration remains
        // authoritative.  This adapter can drive an implicit prefix cache for
        // supported models, but an adapter-wide default must not erase a
        // model-specific unsupported capability.
        let unsupported_cache = capabilities
            .cache_contract
            .as_ref()
            .is_some_and(|contract| contract.behavior == ProviderCacheBehavior::Unsupported);
        capabilities.cache = !unsupported_cache;
        // This adapter sends `prompt_cache_key`, so it drives an implicit
        // prefix cache and must say so. It does not chain `previous_response_id`
        // — that field exists only on the websocket request shape — so every
        // turn still uploads the whole history.
        capabilities.prompt_cache = if unsupported_cache {
            PromptCacheControl::None
        } else {
            PromptCacheControl::Implicit
        };
        if unsupported_cache {
            capabilities.cache_contract = Some(ProviderCacheContract::default());
        } else {
            let mut contract = ProviderCacheContract::from_control(PromptCacheControl::Implicit);
            contract.evidence.stream = true;
            contract.key_revision = Some(RegistryRevision::new(CHATGPT_CACHE_KEY_REVISION));
            capabilities.cache_contract = Some(contract);
        }
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
    /// Each session's sticky-routing token for its current turn, with the
    /// account it was issued to.
    turn_states: std::sync::Mutex<BTreeMap<String, (String, String)>>,
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
            turn_states: std::sync::Mutex::default(),
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

    /// Serializes the planned order using Runtime's routing key, or the session
    /// when no cache identity is available. Later host context stays in input
    /// rather than changing the leading instruction field.
    pub(super) fn build_payload(
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
        // Only the leading instruction block belongs before conversation
        // history. Hoisting later host context here makes a changing todo or
        // continuation invalidate the otherwise unchanged history prefix.
        let (instructions, messages) = match request.messages.split_first() {
            Some((first, rest)) if first.role == Role::System => (first.joined_text(), rest),
            _ => (String::new(), request.messages.as_slice()),
        };
        let tool_names = response_tool_names(&request.tools)?;
        let mut input = Vec::new();
        for message in messages {
            input.extend(response_items(message, &tool_names)?);
        }
        let mut tools = request
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
        for tool in &request.hosted_tools {
            match tool {
                HostedTool::WebSearch { .. } => tools.push(json!({"type": "web_search"})),
                _ => {
                    return Err(ProviderError::new(
                        ProviderErrorKind::Unsupported,
                        "unsupported hosted tool",
                    ));
                }
            }
        }
        let cache_key = request
            .cache_identity
            .as_ref()
            .map(|identity| identity.wire_cache_key().as_str())
            .unwrap_or_else(|| session.as_str());
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
            "prompt_cache_key": cache_key,
        });
        if !request.hosted_tools.is_empty() {
            payload["include"] = json!([
                "reasoning.encrypted_content",
                "web_search_call.action.sources"
            ]);
        }
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

    /// The turn-state token to replay, or none when this request starts a turn.
    ///
    /// The endpoint issues `x-codex-turn-state` on a turn's first response and
    /// expects it back on every later request of that turn. Without it each
    /// tool-loop request is routed afresh, lands on a server that never saw
    /// this session's history, and reuses only the prefix every session
    /// shares. A request whose newest non-system message is a tool result
    /// continues the turn; anything else starts one, which drops the old token
    /// because replaying it into a different turn breaks the server contract.
    /// A token issued to another account is dropped the same way, as Codex
    /// does when the credential owner changes.
    pub(super) fn turn_state(
        &self,
        request: &ProviderRequest,
        session: &str,
        account: &str,
    ) -> Option<String> {
        let continues_turn = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role != Role::System)
            .is_some_and(|message| message.role == Role::Tool);
        let mut states = self.turn_states.lock().expect("turn states");
        match states.get(session) {
            Some((issued_to, token)) if continues_turn && issued_to == account => {
                Some(token.clone())
            }
            _ => {
                states.remove(session);
                None
            }
        }
    }

    /// Keeps the first token a turn's responses carry, as the contract asks.
    pub(super) fn record_turn_state(
        &self,
        session: &str,
        account: &str,
        headers: &[(String, String)],
    ) {
        let Some((_, token)) = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(CHATGPT_TURN_STATE_HEADER))
        else {
            return;
        };
        let mut states = self.turn_states.lock().expect("turn states");
        if !states.contains_key(session) && states.len() >= MAX_TURN_STATES {
            // Sessions that ended mid-turn would otherwise accumulate; losing
            // a live session's token costs routing affinity, not correctness.
            states.clear();
        }
        states
            .entry(session.to_owned())
            .or_insert_with(|| (account.to_owned(), token.clone()));
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
        mut request: ProviderRequest,
        ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        // Runtime carries the exact opaque identity in both request and call
        // context.  Older callers may provide it only in the context; copy it
        // through rather than falling back to a request/session-derived key.
        if request.cache_identity.is_none() {
            request.cache_identity = ctx.cache_identity.clone();
        } else if ctx
            .cache_identity
            .as_ref()
            .is_some_and(|identity| request.cache_identity.as_ref() != Some(identity))
        {
            return Err(ProviderError::new(
                ProviderErrorKind::BadRequest,
                "provider request and call context carry different cache identities",
            ));
        }
        request
            .validate_cache_identity()
            .map_err(|message| ProviderError::new(ProviderErrorKind::BadRequest, message))?;
        let (payload, tool_names) = self.build_payload(&request, &ctx.session)?;
        // The endpoint routes a conversation by this header, as Codex sends
        // it: the same value as `prompt_cache_key`. A per-request id here sent
        // every session's Nth request under one shared id and scattered each
        // session's own requests across servers.
        let routing_key = payload["prompt_cache_key"]
            .as_str()
            .expect("the payload carries a cache key")
            .to_owned();
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
        // Only a request offering tools can be continued by another one.
        // Advisor, vision and module one-shots reuse the caller's session
        // mid-turn and offer none, so they must not touch its token.
        let routes_turn = ctx.purpose == ProviderAttemptPurpose::Ordinary
            && !(request.tools.is_empty() && request.hosted_tools.is_empty());
        let turn_state = routes_turn
            .then(|| self.turn_state(&request, ctx.session.as_str(), &account_id))
            .flatten();
        let mut http = HttpRequest {
            url: CHATGPT_RESPONSES_ENDPOINT.into(),
            headers: vec![
                ("content-type".into(), "application/json".into()),
                ("accept".into(), "text/event-stream".into()),
                (
                    "authorization".into(),
                    format!("Bearer {}", lease.secret().expose()),
                ),
                ("chatgpt-account-id".into(), account_id.clone()),
                ("originator".into(), "smith".into()),
                ("session-id".into(), routing_key),
                ("thread-id".into(), ctx.session.as_str().to_owned()),
            ],
            body,
        };
        if let Some(token) = turn_state {
            http.headers.push((CHATGPT_TURN_STATE_HEADER.into(), token));
        }
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
        if routes_turn {
            self.record_turn_state(ctx.session.as_str(), &account_id, &response.headers);
        }
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
                buffer_text: !request.hosted_tools.is_empty(),
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
