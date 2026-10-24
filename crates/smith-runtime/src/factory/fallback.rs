//! Metadata preflight and deferred provider construction for model alternatives.

use super::*;
use agent_runtime_core::provider::{
    ModelDescriptor, ProviderCallContext, ProviderErrorKind, ProviderRequest, ProviderStream,
};
use async_trait::async_trait;
use tokio::sync::OnceCell;

/// Model-bound policy retained for event projection and per-binding reporting.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelFallbackPolicy {
    /// Configured provider identity.
    pub provider_name: String,
    /// Concrete wire adapter.
    pub provider_kind: String,
    /// Validated endpoint, when the adapter has one.
    pub endpoint: Option<String>,
    /// Model identity selected on activation.
    pub model: ModelId,
    /// Enforced catalog/profile metadata.
    pub model_profile: ResolvedModelProfile,
    /// Selected named context window.
    pub context_window: Option<String>,
    /// Available named windows for this model.
    pub context_windows: Vec<String>,
    /// Typed reasoning selection.
    pub reasoning: ReasoningRuntimePolicy,
    /// Context reserves and budgets.
    pub context_policy: ContextPolicy,
    /// Structural compaction policy.
    pub compaction_policy: CompactionPolicy,
    /// Per-request generation ask.
    pub max_output_tokens: Option<u32>,
    /// Effective candidate startup budget.
    pub first_response_timeout_ms: Option<u64>,
}

pub(super) struct PreparedFallbacks {
    pub(super) routes: Vec<agent_runtime::runtime::ModelRoute>,
    pub(super) policies: Vec<ModelFallbackPolicy>,
    pub(super) observers: Vec<Arc<dyn EventObserver>>,
}

#[derive(Debug)]
struct DeferredProvider {
    request: Box<RuntimeRequest>,
    profile: ResolvedModelProfile,
    adapter: provider::Adapter,
    inner: OnceCell<Arc<dyn Provider>>,
}

impl DeferredProvider {
    async fn acquire(&self) -> Result<&Arc<dyn Provider>, ProviderError> {
        self.inner
            .get_or_try_init(|| async {
                let prepared = provider::prepare(&self.request)
                    .await
                    .map_err(preparation_error)?;
                if prepared.profile.profile != self.profile {
                    return Err(ProviderError::new(
                        ProviderErrorKind::BadRequest,
                        "fallback metadata changed while acquiring the provider",
                    ));
                }
                provider::construct_runtime(
                    &self.request,
                    prepared.adapter,
                    prepared.endpoint,
                    prepared.secret,
                    &prepared.profile.profile,
                    &prepared.reasoning,
                    prepared.command.map(|command| command.provider),
                )
                .map(|stage| stage.provider)
                .map_err(preparation_error)
            })
            .await
    }
}

fn preparation_error(error: FactoryError) -> ProviderError {
    match error {
        FactoryError::Credential(_)
        | FactoryError::CredentialTask
        | FactoryError::CredentialTimeout { .. }
        | FactoryError::MissingHostPolicy { .. } => ProviderError::new(
            ProviderErrorKind::Unavailable,
            "configured fallback provider or credential source is unavailable",
        ),
        FactoryError::ChatGptAuth(_) | FactoryError::XaiAuth(_) => ProviderError::new(
            ProviderErrorKind::Auth,
            "configured fallback login is invalid",
        ),
        _ => ProviderError::new(
            ProviderErrorKind::BadRequest,
            "configured fallback provider failed local validation",
        ),
    }
}

#[async_trait]
impl Provider for DeferredProvider {
    async fn prepare_request(
        &self,
        request: &ProviderRequest,
        ctx: &ProviderCallContext,
    ) -> Result<(), ProviderError> {
        self.acquire().await?.prepare_request(request, ctx).await
    }
    fn describe(&self) -> Vec<ModelDescriptor> {
        vec![ModelDescriptor {
            id: self.profile.model.clone(),
            display_name: self.profile.model.to_string(),
            vendor: self.request.config.provider.kind.value.clone(),
            capabilities: self.profile.capabilities.clone(),
        }]
    }

    fn capabilities(&self, model: &ModelId) -> Option<Capabilities> {
        (model == &self.profile.model).then(|| self.profile.capabilities.clone())
    }

    fn pre_output_replay_safe(&self, request: &ProviderRequest) -> bool {
        if let Some(provider) = &self.request.provider {
            return provider.pre_output_replay_safe(request);
        }
        self.adapter != provider::Adapter::CommandJsonl
            && request.hosted_tools.iter().all(|tool| {
                matches!(
                    tool,
                    agent_runtime_core::provider::HostedTool::WebSearch { .. }
                )
            })
    }

    fn requires_nonempty_assistant_content(&self) -> bool {
        self.request.provider.as_ref().map_or(
            self.adapter == provider::Adapter::AnthropicMessages,
            |provider| provider.requires_nonempty_assistant_content(),
        )
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        context: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        self.acquire().await?.stream(request, context).await
    }
}

/// A pool for a fallback route on another provider, started on the account
/// the user last used there. Without a remembered account the route builds
/// its own pool from the first declared member.
fn remembered_pool(
    route: &RuntimeRequest,
    accounts: Option<&crate::pool_state::ActiveAccounts>,
) -> Option<SharedPool> {
    let accounts = accounts?;
    let pool = provider::credential_pool_for(route)?;
    let references = route
        .config
        .provider
        .credentials
        .iter()
        .map(|reference| reference.value.clone())
        .collect::<Vec<_>>();
    let position = accounts.position_in(&route.config.provider.name.value, &references)?;
    pool.write(|pool| pool.set_active(position));
    Some(pool)
}

pub(super) fn prepare(
    request: &RuntimeRequest,
) -> futures_util::future::BoxFuture<'_, Result<PreparedFallbacks, FactoryError>> {
    Box::pin(prepare_inner(request))
}

async fn prepare_inner(request: &RuntimeRequest) -> Result<PreparedFallbacks, FactoryError> {
    let mut result = PreparedFallbacks {
        routes: Vec::new(),
        policies: Vec::new(),
        observers: Vec::new(),
    };
    for candidate in &request.config.fallback.models {
        let mut route = RuntimeRequest::new((*candidate.config).clone(), HostSurface::Child);
        route.workspace = request.workspace.clone();
        route.approval = request.approval.clone();
        route.credentials = request.credentials.clone();
        route.transport = request.transport.clone();
        route.credential_timeout_ms = request.credential_timeout_ms;
        route.catalog_sources = request.catalog_sources.clone();
        route.model_catalog = request.model_catalog.clone();
        route.persistence_redactor = request.persistence_redactor.clone();
        route.clock = request.clock.clone();
        route.rotation = request.rotation.clone();
        route.defer_quota_rotation = true;
        route.credential_pool =
            if route.config.provider.name.value == request.config.provider.name.value {
                request.credential_pool.clone()
            } else {
                remembered_pool(&route, request.active_accounts.as_ref())
            };
        route.provider = request
            .fallback_providers
            .get(&(
                route.config.provider.name.value.clone(),
                route.config.model.value.clone(),
            ))
            .cloned();
        route.modules = modules::for_config(&request.modules, &route.config);
        route.built_in_tools = request.built_in_tools;
        route.web_search_session = request.web_search_session.clone();
        if route.config.provider.name.value != request.config.provider.name.value
            && let Some(snapshot) = &route.model_catalog
            && let Some(source) = crate::model_catalog::runtime_catalog_source(
                snapshot,
                &route.config.provider.name.value,
                &route.config.provider.kind.value,
                route
                    .config
                    .provider
                    .base_url
                    .as_ref()
                    .map(|url| url.value.as_str()),
            )
        {
            route.catalog_sources.push(source);
        }
        provider::validate_pool_references(&route)?;
        let prepared = provider::prepare_metadata(&route).await.map_err(|error| {
            FactoryError::Runtime(RuntimeError::config(format!(
                "{}: {error}",
                candidate.source
            )))
        })?;
        let timeout = candidate
            .first_response_timeout_ms
            .as_ref()
            .map(|timeout| timeout.value);
        let hosted = web::hosted_tools(
            &route,
            prepared.profile.profile.capabilities.hosted_web_search,
        );
        let endpoint_identity = provider::cache_endpoint_identity(
            &prepared.provider_name,
            &prepared.provider_kind,
            prepared.endpoint.as_deref(),
            provider::active_credential_reference(&route).as_deref(),
        );
        let lazy = Arc::new(DeferredProvider {
            request: Box::new(route),
            profile: prepared.profile.profile.clone(),
            adapter: prepared.adapter,
            inner: OnceCell::new(),
        });
        let backoff = Arc::new(crate::provider_backoff::ProviderBackoff::new(lazy));
        let mut builder = RuntimeBuilder::new(prepared.model.clone())
            // The candidate plans images at its own provider's rates, as the
            // primary and child routes do.
            .request_sizer(Arc::new(
                crate::image_sizing::SmithRequestSizer::for_provider_and_model(
                    &prepared.provider_kind,
                    prepared.model.as_str(),
                ),
            ))
            .provider(backoff.clone())
            .provider_name(prepared.provider_name.clone())
            .model_profile(prepared.profile.profile.clone())
            .loop_config(prepared.loop_config.clone())
            .hosted_tools(hosted)
            .context_policy(prepared.context_policy.clone())
            .compactor(StructuralCompactor::new(prepared.compaction_policy.clone()))
            .cache_capability(ProviderCacheCapability::from_control(
                RegistryRevision::new(CACHE_CAPABILITY_REVISION),
                prepared.provider_name.clone(),
                prepared.profile.profile.capabilities.prompt_cache,
            ))
            .clock(
                request
                    .clock
                    .clone()
                    .unwrap_or_else(|| Arc::new(SystemClock)),
            );
        if let Some(identity) = endpoint_identity {
            builder = builder.cache_endpoint_identity(identity);
        }
        let runtime = builder.build().map_err(|error| {
            FactoryError::Runtime(RuntimeError::config(format!(
                "{}: {error}",
                candidate.source
            )))
        })?;
        result.routes.push(runtime.model_route(timeout));
        result.observers.push(backoff);
        result.policies.push(ModelFallbackPolicy {
            provider_name: prepared.provider_name,
            provider_kind: prepared.provider_kind,
            endpoint: prepared.endpoint,
            model: prepared.model,
            model_profile: prepared.profile.profile,
            context_window: prepared.context_window.active.map(|window| window.name),
            context_windows: prepared.context_window.available,
            reasoning: prepared.reasoning,
            context_policy: prepared.context_policy,
            compaction_policy: prepared.compaction_policy,
            max_output_tokens: prepared.loop_config.max_output_tokens,
            first_response_timeout_ms: timeout,
        });
    }
    Ok(result)
}
