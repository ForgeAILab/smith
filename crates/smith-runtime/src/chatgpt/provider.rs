use super::*;

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
