use super::*;

/// Immutable per-turn/provider-request reasoning patch.
#[derive(Debug)]
pub(crate) struct ReasoningInterceptor {
    request: Option<ReasoningConfig>,
}

impl ReasoningInterceptor {
    pub(crate) fn new(policy: &ReasoningRuntimePolicy) -> Self {
        Self {
            request: policy.request_config(),
        }
    }
}

#[async_trait]
impl ModelInterceptor for ReasoningInterceptor {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(
            "smith.reasoning.selection",
            RegistryRevision::new(INTERCEPTOR_REVISION),
        )
    }

    async fn before_model(&self, _view: &ModelView) -> Result<ModelRequestPatch, RuntimeError> {
        Ok(ModelRequestPatch {
            reasoning: Some(self.request.clone()),
            ..ModelRequestPatch::default()
        })
    }
}

/// Provider wrapper translating normalized reasoning into one exact dialect.
#[derive(Debug)]
pub(crate) struct ReasoningDialectProvider {
    inner: Arc<dyn Provider>,
    dialect: ReasoningDialect,
}

impl ReasoningDialectProvider {
    pub(crate) fn new(inner: Arc<dyn Provider>, dialect: ReasoningDialect) -> Self {
        Self { inner, dialect }
    }
}

#[async_trait]
impl Provider for ReasoningDialectProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        self.inner.describe()
    }

    fn capabilities(&self, model: &ModelId) -> Option<Capabilities> {
        self.inner.capabilities(model)
    }

    async fn stream(
        &self,
        mut request: ProviderRequest,
        ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        adapt_request(&mut request, self.dialect)?;
        self.inner.stream(request, ctx).await
    }
}

pub(super) fn adapt_request(
    request: &mut ProviderRequest,
    dialect: ReasoningDialect,
) -> Result<(), ProviderError> {
    match dialect {
        ReasoningDialect::OpenaiEffort => {}
        ReasoningDialect::Openrouter => {
            if let Some(reasoning) = request.reasoning.take() {
                let effort = reasoning.effort.ok_or_else(|| {
                    ProviderError::new(
                        ProviderErrorKind::BadRequest,
                        "OpenRouter reasoning selection has no typed value",
                    )
                })?;
                let value = match effort.as_str() {
                    SENTINEL_ENABLED => json!({"enabled": true}),
                    SENTINEL_DISABLED => json!({"enabled": false}),
                    effort => json!({"effort": effort}),
                };
                insert_extension(&mut request.vendor_extensions, "reasoning", value)?;
            }
        }
        ReasoningDialect::ZaiThinking => {
            if let Some(reasoning) = request.reasoning.take() {
                let state = match reasoning.effort.as_deref() {
                    Some(SENTINEL_ENABLED) => "enabled",
                    Some(SENTINEL_DISABLED) => "disabled",
                    _ => {
                        return Err(ProviderError::new(
                            ProviderErrorKind::BadRequest,
                            "Z.AI thinking selection is not an enabled/disabled state",
                        ));
                    }
                };
                insert_extension(
                    &mut request.vendor_extensions,
                    "thinking",
                    json!({"type": state}),
                )?;
            }
        }
        ReasoningDialect::GeminiThinking | ReasoningDialect::AnthropicEffort => {}
    }
    Ok(())
}

fn insert_extension(extensions: &mut Value, key: &str, value: Value) -> Result<(), ProviderError> {
    if extensions.is_null() {
        *extensions = Value::Object(Map::new());
    }
    let Value::Object(object) = extensions else {
        return Err(ProviderError::new(
            ProviderErrorKind::BadRequest,
            "provider extensions must be a JSON object",
        ));
    };
    if object.insert(key.to_owned(), value).is_some() {
        return Err(ProviderError::new(
            ProviderErrorKind::BadRequest,
            format!("provider extension `{key}` was already set"),
        ));
    }
    Ok(())
}
