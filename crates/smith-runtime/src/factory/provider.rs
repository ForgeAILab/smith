//! Provider stage assembly around resolution, credentials, and concrete adapters.

use super::*;

pub(in crate::factory) mod adapter;
mod children;
pub(super) mod credentials;
mod resolution;

pub(super) use adapter::{Adapter, PreparedCommandProvider};
pub(super) use children::{
    cache_endpoint_identity, prepare_advisor_route, prepare_child_profile_routes,
    prepare_vision_route,
};
pub(super) use credentials::{active_credential_reference, validate_pool_references};
pub(super) use resolution::prepare;
pub(super) use resolution::prepare_metadata;

pub(super) struct CompactionRoute {
    pub provider: Arc<dyn Provider>,
    pub provider_name: String,
    pub model: ModelId,
    pub max_output_tokens: u32,
}

pub(super) fn prepare_compaction_route(
    request: &RuntimeRequest,
) -> futures_util::future::BoxFuture<'_, Result<Option<CompactionRoute>, FactoryError>> {
    Box::pin(prepare_compaction_route_inner(request))
}

async fn prepare_compaction_route_inner(
    request: &RuntimeRequest,
) -> Result<Option<CompactionRoute>, FactoryError> {
    if request.semantic_summary.is_none() {
        return Ok(None);
    }
    let Some(selected) = request.config.context.compaction_model.as_ref() else {
        return Ok(None);
    };
    let binding = request.compaction_profile.as_ref().ok_or_else(|| {
        FactoryError::Runtime(RuntimeError::config(
            "resolve the configured compaction model before composition",
        ))
    })?;
    let matches = match smith_config::resolve::AdvisorTarget::parse(&selected.value) {
        Some(smith_config::resolve::AdvisorTarget::Profile(name)) => {
            binding.config.agent.profile.name == name
        }
        Some(smith_config::resolve::AdvisorTarget::Model { provider, model }) => {
            binding.config.provider.name.value == provider && binding.config.model.value == model
        }
        _ => false,
    };
    if !matches || binding.config.harness.is_some() {
        return Err(FactoryError::Runtime(RuntimeError::config(
            "compaction requires a matching API model route",
        )));
    }
    let mut route = RuntimeRequest::new(binding.config.clone(), HostSurface::Child);
    route.workspace = request.workspace.clone();
    route.approval = request.approval.clone();
    route.credentials = request.credentials.clone();
    route.transport = request.transport.clone();
    route.credential_timeout_ms = request.credential_timeout_ms;
    route.catalog_sources = binding.catalog_sources.clone();
    route.model_catalog = request.model_catalog.clone();
    route.persistence_redactor = request.persistence_redactor.clone();
    route.provider = binding.provider.clone();
    route.built_in_tools = false;
    validate_pool_references(&route)?;
    let prepared = prepare(&route).await?;
    let provider = construct_runtime(
        &route,
        prepared.adapter,
        prepared.endpoint,
        prepared.secret,
        &prepared.profile.profile,
        &prepared.reasoning,
        prepared.command.map(|command| command.provider),
    )?
    .provider;
    Ok(Some(CompactionRoute {
        provider,
        provider_name: prepared.provider_name,
        model: prepared.model,
        max_output_tokens: prepared.profile.profile.limits.max_output_tokens,
    }))
}

struct ImageBackendBinding {
    endpoint: String,
    target: ProviderCredentialTarget,
    credentials: Arc<dyn ProviderCredentialSource>,
    chatgpt: bool,
}

pub(super) struct ProviderStage {
    pub(super) provider: Arc<dyn Provider>,
    pub(super) image_binding: Option<smith_module::ImageBinding>,
}

pub(super) fn construct_runtime(
    request: &RuntimeRequest,
    adapter: Adapter,
    endpoint: Option<String>,
    secret: Option<Secret>,
    profile: &ResolvedModelProfile,
    reasoning: &ReasoningRuntimePolicy,
    command: Option<Arc<dyn Provider>>,
) -> Result<ProviderStage, FactoryError> {
    if let (Some(secret), Some(redactor)) = (&secret, &request.persistence_redactor) {
        redactor.register_secret(secret);
    }
    // Built before the adapter so both halves share one pool: the credential
    // source reads the active member from it, and the rotation decorator
    // writes to it. Two pools would mean rotation changed a member the
    // adapter never consulted.
    let pool = credentials::credential_pool_for(request);
    let mut image_binding = None;
    let provider = match request.provider.clone() {
        Some(provider) => provider,
        None => adapter::construct(
            adapter,
            request,
            profile,
            adapter::ProviderConstructionInputs {
                endpoint,
                secret,
                supported_thinking_levels: &reasoning.efforts,
                pool: pool.as_ref(),
                command,
            },
            Some(&mut image_binding),
        )?,
    };
    let provider = Arc::new(crate::image_fitting::ImageFittingProvider::new(
        provider,
        crate::factory::image_limits_for_provider_kind(&request.config.provider.kind.value),
    )) as Arc<dyn Provider>;
    let provider = crate::response::apply_response_policy(
        provider,
        request
            .config
            .provider
            .response
            .reasoning_only
            .as_ref()
            .map(|policy| policy.value),
    );
    let provider = match reasoning.dialect {
        Some(dialect) => {
            Arc::new(ReasoningDialectProvider::new(provider, dialect)) as Arc<dyn Provider>
        }
        None => provider,
    };
    let module_image_binding = image_binding
        .as_ref()
        .map(|binding| smith_module::ImageBinding {
            endpoint: binding.endpoint.clone(),
            target: binding.target.clone(),
            credentials: binding.credentials.clone(),
            chatgpt: binding.chatgpt,
        });
    Ok(ProviderStage {
        provider: credentials::apply_credential_pool(request, provider, pool),
        image_binding: module_image_binding,
    })
}
