//! Provider stage assembly around resolution, credentials, and concrete adapters.

use super::*;

pub(in crate::factory) mod adapter;
mod children;
mod credentials;
mod resolution;

pub(super) use adapter::{Adapter, PreparedCommandProvider};
pub(super) use children::{cache_endpoint_identity, prepare_child_profile_routes};
pub(super) use credentials::{active_credential_reference, validate_pool_references};
pub(super) use resolution::prepare;

struct ImageBackendBinding {
    endpoint: String,
    target: ProviderCredentialTarget,
    credentials: Arc<dyn ProviderCredentialSource>,
    chatgpt: bool,
}

pub(super) struct ProviderStage {
    pub(super) provider: Arc<dyn Provider>,
    pub(super) image_backend: Option<Arc<dyn smith_tools::ImageGenerationBackend>>,
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
    let image_backend =
        if request.config.image_generation.enabled.value
            && !request.config.agent.active_posture().is_read_only()
            && request.built_in_tools
        {
            image_binding
            .map(|binding| -> Result<Arc<dyn smith_tools::ImageGenerationBackend>, FactoryError> {
                let transport = Arc::new(
                    ReqwestTransport::new(request.transport.clone())
                        .map_err(FactoryError::Transport)?,
                )
                    as Arc<dyn agent_runtime::provider::transport::HttpTransport>;
                Ok(Arc::new(crate::image_api::ImagesApiBackend::new(
                    binding.endpoint,
                    transport,
                    binding.target,
                    binding.credentials,
                    binding.chatgpt,
                ))
                    as Arc<dyn smith_tools::ImageGenerationBackend>)
            })
            .transpose()?
        } else {
            None
        };
    Ok(ProviderStage {
        provider: credentials::apply_credential_pool(request, provider, pool),
        image_backend,
    })
}
