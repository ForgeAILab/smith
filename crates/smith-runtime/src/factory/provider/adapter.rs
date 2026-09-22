//! Adapter selection, endpoint validation, command preflight, and construction.

use super::super::authority;
use super::credentials;
use super::*;

/// The shared adapter a configured provider kind maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::factory) enum Adapter {
    /// Agent Runtime's OpenAI-compatible Chat-Completions adapter.
    OpenAiCompatible,
    /// Agent Runtime's native stateless OpenAI Responses adapter.
    OpenAiResponses,
    /// Agent Runtime's native Anthropic Messages API adapter.
    AnthropicMessages,
    /// Smith's experimental direct ChatGPT Codex Responses adapter.
    ChatGptResponses,
    /// The Responses adapter driven by a renewable xAI browser login.
    XaiResponses,
    /// Agent Runtime's native stateless Gemini Interactions adapter.
    GeminiInteractions,
    /// Smith's revision-1 local command JSONL adapter.
    CommandJsonl,
    /// Agent Runtime's deterministic fake.
    Fake,
}

pub(in crate::factory) struct PreparedCommandProvider {
    pub(in crate::factory) provider: Arc<dyn Provider>,
    pub(in crate::factory) implementation: Option<String>,
}

pub(super) struct StaticCommandProvider {
    process: CommandProcessConfig,
    pub(super) adapter: CommandJsonlAdapter,
}

/// Constructs the configured provider.
pub(super) struct ProviderConstructionInputs<'a> {
    pub(super) endpoint: Option<String>,
    pub(super) secret: Option<Secret>,
    pub(super) supported_thinking_levels: &'a [String],
    pub(super) pool: Option<&'a SharedPool>,
    pub(super) command: Option<Arc<dyn Provider>>,
}

/// Maps a configured provider kind onto a shared adapter.
///
/// The check runs even when a provider is injected: which adapters exist is a
/// property of the pinned runtime, and a profile naming one it does not have is
/// a configuration error whether or not this particular run would have used it.
pub(in crate::factory) fn adapter(provider: &ResolvedProvider) -> Result<Adapter, FactoryError> {
    match provider.kind.value.as_str() {
        KIND_OPENAI_COMPATIBLE => Ok(Adapter::OpenAiCompatible),
        KIND_OPENAI_RESPONSES => Ok(Adapter::OpenAiResponses),
        KIND_ANTHROPIC_MESSAGES => Ok(Adapter::AnthropicMessages),
        KIND_CHATGPT_RESPONSES => Ok(Adapter::ChatGptResponses),
        KIND_XAI_RESPONSES => Ok(Adapter::XaiResponses),
        KIND_GEMINI_INTERACTIONS => Ok(Adapter::GeminiInteractions),
        KIND_COMMAND_JSONL => Ok(Adapter::CommandJsonl),
        KIND_FAKE => Ok(Adapter::Fake),
        kind => Err(FactoryError::AdapterUnavailable {
            provider: provider.name.value.clone(),
            kind: kind.to_owned(),
        }),
    }
}

/// Applies the selected adapter's ordinary model-cache declaration without
/// manufacturing synthetic-maintenance safety.  Catalog sources describe the
/// model and limits, while the serving adapter owns the wire cache behavior;
/// an explicit normalized unsupported contract remains authoritative so an
/// adapter can preserve a genuinely unsupported model.
pub(in crate::factory) fn apply_adapter_cache_capability(
    adapter: Adapter,
    endpoint: Option<&str>,
    credential_partition_can_rotate: bool,
    capabilities: &mut Capabilities,
) {
    // Runtime cache identities are immutable for a session. A credential pool
    // may switch the account/tenant partition immediately before provider
    // I/O, so the initial member cannot be an exact identity for later calls.
    // Keep ordinary provider behavior available but disable Runtime cache
    // planning and every synthetic action until rotation can publish a new
    // immutable partition and force replanning.
    if credential_partition_can_rotate {
        capabilities.cache = false;
        capabilities.prompt_cache = PromptCacheControl::None;
        capabilities.cache_contract = Some(ProviderCacheContract::default());
        return;
    }
    if matches!(adapter, Adapter::CommandJsonl | Adapter::Fake) {
        capabilities.cache = false;
        capabilities.prompt_cache = PromptCacheControl::None;
        capabilities.cache_contract = Some(ProviderCacheContract::default());
        return;
    }
    if capabilities
        .cache_contract
        .as_ref()
        .is_some_and(|contract| contract.behavior == ProviderCacheBehavior::Unsupported)
    {
        capabilities.cache = false;
        capabilities.prompt_cache = PromptCacheControl::None;
        return;
    }
    if capabilities.cache_contract.is_none() {
        capabilities.cache = true;
        // Anthropic's Messages adapter has an explicit four-breakpoint wire
        // contract; the other built-in request adapters expose an implicit
        // stable prefix.
        let control = if matches!(adapter, Adapter::AnthropicMessages) {
            PromptCacheControl::Explicit { max_breakpoints: 4 }
        } else {
            PromptCacheControl::Implicit
        };
        capabilities.prompt_cache = control;
        let mut contract = ProviderCacheContract::from_control(control);
        contract.evidence.stream = true;
        capabilities.cache_contract = Some(contract);
    }

    // The native Responses wire shape has offline fixtures for every
    // SyntheticConformance gate only at OpenAI's official endpoint. Generic
    // compatible and user-supplied endpoints remain observation-only even
    // when they publish prompt-cache usage fields.
    if adapter == Adapter::OpenAiResponses && endpoint == Some(OPENAI_ENDPOINT) {
        capabilities.cache = true;
        capabilities.prompt_cache = PromptCacheControl::Implicit;
        let contract = capabilities.cache_contract.get_or_insert_with(|| {
            ProviderCacheContract::from_control(PromptCacheControl::Implicit)
        });
        contract.behavior = ProviderCacheBehavior::ImplicitPrefix;
        contract.evidence.stream = true;
        contract.key_revision = Some(RegistryRevision::new("openai-responses-prompt-cache-1"));
        contract.maintenance.extend([
            ProviderAttemptPurpose::CacheKeepalive,
            ProviderAttemptPurpose::CacheHandoffCheckpoint,
        ]);
        contract.conformance = Some(SyntheticConformance::complete());
    }
}

pub(super) fn prepare_static_command_provider(
    request: &RuntimeRequest,
    model: &str,
) -> Result<StaticCommandProvider, FactoryError> {
    let command = request
        .config
        .provider
        .command
        .as_ref()
        .ok_or(FactoryError::CommandIncompatible)?;
    let cwd = match command.cwd.as_ref().map(|cwd| &cwd.value) {
        None | Some(CommandWorkingDirectory::Workspace) => {
            std::path::PathBuf::from(authority::require_workspace(request)?.root())
        }
        Some(CommandWorkingDirectory::Absolute(path)) => path.clone(),
    };
    let process = CommandProcessConfig::new(command.executable.value.clone(), cwd)
        .map_err(FactoryError::CommandConfiguration)?
        .with_fixed_args(
            command
                .args
                .as_ref()
                .map_or_else(Vec::new, |args| args.value.clone()),
        )
        .map_err(FactoryError::CommandConfiguration)?;
    let adapter = CommandJsonlAdapter::new(model).map_err(FactoryError::CommandAdapter)?;
    Ok(StaticCommandProvider { process, adapter })
}

pub(super) async fn prepare_command_provider(
    request: &RuntimeRequest,
    command: StaticCommandProvider,
) -> Result<PreparedCommandProvider, FactoryError> {
    let declaration = request
        .config
        .provider
        .command
        .as_ref()
        .ok_or(FactoryError::CommandIncompatible)?;
    let mut process = command.process;
    for (name, value) in &declaration.env {
        let secret = match &value.value {
            McpValue::Literal(literal) => literal.clone(),
            McpValue::Credential(reference) => credentials::secret(request, reference)
                .await
                .map_err(|source| FactoryError::CommandEnvironment {
                    variable: name.clone(),
                    source: Box::new(source),
                })?,
        };
        if let Some(redactor) = &request.persistence_redactor {
            redactor.register_secret(&secret);
        }
        process = process
            .with_env(name.clone(), secret.expose().to_owned())
            .map_err(FactoryError::CommandConfiguration)?;
    }

    let provider = Arc::new(
        CommandProvider::new(process, Arc::new(command.adapter))
            .map_err(FactoryError::CommandProviderBuild)?,
    );
    let preflight = provider
        .preflight()
        .await
        .map_err(FactoryError::CommandPreflight)?
        .ok_or(FactoryError::CommandIncompatible)?;
    if !preflight.is_compatible() {
        return Err(FactoryError::CommandIncompatible);
    }
    Ok(PreparedCommandProvider {
        implementation: preflight.version().map(str::to_owned),
        provider: Arc::new(CommandProtocolProvider::new(provider)) as Arc<dyn Provider>,
    })
}

/// Validates the configured endpoint and normalizes it for the adapter.
///
/// `default` supplies the endpoint for adapters whose wire protocol has one
/// well-known home (the Anthropic Messages API); without it, a missing
/// `base_url` is a configuration error.
///
/// No message repeats the URL. A base URL is the other place a key is known to
/// be pasted — as userinfo or as a query parameter — and both are refused here
/// rather than forwarded, because a credential in a URL ends up in a log the
/// moment anything prints the request target.
pub(in crate::factory) fn endpoint(
    provider: &ResolvedProvider,
    default: Option<&str>,
) -> Result<String, FactoryError> {
    let refuse = |message: &str| FactoryError::Endpoint {
        provider: provider.name.value.clone(),
        message: message.to_owned(),
    };
    let configured = match (&provider.base_url, default) {
        (Some(configured), _) => configured,
        (None, Some(default)) => return Ok(default.trim_end_matches('/').to_owned()),
        (None, None) => {
            return Err(refuse("the provider needs the endpoint it talks to"));
        }
    };

    let url = Url::parse(&configured.value).map_err(|_| refuse("it is not an absolute URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(refuse("only `http` and `https` endpoints are supported"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refuse(
            "it carries credentials in the URL; move them to the provider's `credential` reference",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(refuse(
            "it must be a plain endpoint; per-request options belong in `headers` and secrets in \
             `credential`",
        ));
    }
    let host = url.host_str().ok_or_else(|| refuse("it names no host"))?;

    let mut endpoint = format!("{}://{host}", url.scheme());
    if let Some(port) = url.port() {
        endpoint.push(':');
        endpoint.push_str(&port.to_string());
    }
    // The adapter appends its own path segment, so a trailing slash here would
    // produce `…/v1//chat/completions`.
    endpoint.push_str(url.path().trim_end_matches('/'));
    Ok(endpoint)
}

fn remember_image_binding(
    binding: &mut Option<ImageBackendBinding>,
    adapter: Adapter,
    endpoint: &str,
    target: ProviderCredentialTarget,
    credentials: Arc<dyn ProviderCredentialSource>,
) {
    let chatgpt = adapter == Adapter::ChatGptResponses;
    let openai_platform = matches!(
        adapter,
        Adapter::OpenAiCompatible | Adapter::OpenAiResponses
    ) && endpoint.trim_end_matches('/')
        == OPENAI_ENDPOINT.trim_end_matches('/');
    if chatgpt || openai_platform {
        *binding = Some(ImageBackendBinding {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            target,
            credentials,
            chatgpt,
        });
    }
}

pub(super) fn construct(
    adapter: Adapter,
    request: &RuntimeRequest,
    profile: &ResolvedModelProfile,
    inputs: ProviderConstructionInputs<'_>,
    image_binding: Option<&mut Option<ImageBackendBinding>>,
) -> Result<Arc<dyn Provider>, FactoryError> {
    let ProviderConstructionInputs {
        endpoint,
        secret,
        supported_thinking_levels,
        pool,
        command,
    } = inputs;
    let endpoint_for_images = endpoint.clone().unwrap_or_default();
    match adapter {
        Adapter::CommandJsonl => command.ok_or(FactoryError::CommandIncompatible),
        Adapter::Fake => Ok(Arc::new(FakeProvider::new(
            request.config.model.value.clone(),
            profile.capabilities.clone(),
            vec![ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: DEVELOPMENT_REPLY.to_owned(),
                },
                usage_event(6, 3),
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])],
        ))),
        Adapter::OpenAiCompatible => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let mut config = OpenAiConfig::new(
                endpoint.unwrap_or_default(),
                request.config.model.value.clone(),
            );
            // The resolved profile governs request validation, so the adapter
            // is told what the profile declared rather than a provider-wide
            // guess about every model the endpoint might serve.
            config.capabilities = profile.capabilities.clone();
            config.extra_headers = request
                .config
                .provider
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.value.clone()))
                .collect();
            match secret {
                Some(secret) => {
                    let target =
                        ProviderCredentialTarget::new(request.config.provider.name.value.clone())
                            .map_err(|error| {
                            FactoryError::Runtime(RuntimeError::config(error.to_string()))
                        })?;
                    let source = credentials::credential_source(request, pool, secret);
                    if let Some(binding) = image_binding {
                        remember_image_binding(
                            binding,
                            adapter,
                            &endpoint_for_images,
                            target.clone(),
                            source.clone(),
                        );
                    }
                    let provider =
                        OpenAiProvider::with_credential_source(transport, config, target, source)
                            .map_err(FactoryError::Transport)?;
                    Ok(Arc::new(provider))
                }
                None => Ok(Arc::new(OpenAiProvider::new(transport, config))),
            }
        }
        Adapter::OpenAiResponses => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let mut config = ResponsesConfig::new(
                endpoint.unwrap_or_default(),
                request.config.model.value.clone(),
            );
            // The resolved profile governs request validation, so the adapter
            // is told what the profile declared rather than a provider-wide
            // guess about every model the endpoint might serve.
            config.capabilities = profile.capabilities.clone();
            config.extra_headers = request
                .config
                .provider
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.value.clone()))
                .collect();
            let target = ProviderCredentialTarget::new(request.config.provider.name.value.clone())
                .map_err(|error| FactoryError::Runtime(RuntimeError::config(error.to_string())))?;
            match secret {
                Some(secret) => {
                    let source = credentials::credential_source(request, pool, secret);
                    if let Some(binding) = image_binding {
                        remember_image_binding(
                            binding,
                            adapter,
                            &endpoint_for_images,
                            target.clone(),
                            source.clone(),
                        );
                    }
                    let provider = ResponsesProvider::with_credential_source(
                        transport, config, target, source,
                    )
                    .map_err(FactoryError::Transport)?;
                    Ok(Arc::new(provider))
                }
                None => Ok(Arc::new(
                    ResponsesProvider::new(transport, config).map_err(FactoryError::Transport)?,
                )),
            }
        }
        Adapter::AnthropicMessages => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let mut config = AnthropicConfig::new(
                endpoint.unwrap_or_default(),
                request.config.model.value.clone(),
            );
            // The resolved profile governs request validation, so the adapter
            // is told what the profile declared rather than a provider-wide
            // guess about every model the endpoint might serve.
            config.capabilities = profile.capabilities.clone();
            config.extra_headers = request
                .config
                .provider
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.value.clone()))
                .collect();
            // Anthropic retains its reviewed static compatibility path until
            // that upstream adapter exposes the credential-source contract.
            config.api_key = secret;
            Ok(Arc::new(AnthropicProvider::new(transport, config)))
        }
        Adapter::ChatGptResponses => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let secret = secret.ok_or(FactoryError::ChatGptAuth(
                crate::chatgpt::ChatGptAuthError::InvalidBundle,
            ))?;
            let bundle =
                ChatGptTokenBundle::from_secret(&secret).map_err(FactoryError::ChatGptAuth)?;
            let account_id = bundle.account_id().to_owned();
            // The active member's reference, which is where its renewed bundle
            // goes back, so a refresh survives the process rather than being
            // re-earned every launch.
            let reference = credentials::active_credential_reference(request)
                .ok_or(FactoryError::ChatGptAuth(
                    crate::chatgpt::ChatGptAuthError::InvalidBundle,
                ))
                .and_then(|reference| {
                    CredentialRef::parse(&reference).map_err(|source| {
                        FactoryError::CredentialReference {
                            provider: request.config.provider.name.value.clone(),
                            source,
                        }
                    })
                })?;
            let target = ProviderCredentialTarget::new(request.config.provider.name.value.clone())
                .map_err(|error| FactoryError::Runtime(RuntimeError::config(error.to_string())))?;
            let refresher: Arc<dyn BundleRefresher<ChatGptTokenBundle>> =
                Arc::new(ChatGptOAuthClient::new().map_err(FactoryError::ChatGptAuth)?);
            let active = Arc::new(ChatGptCredentialSource::new(
                target.clone(),
                reference,
                bundle,
                CredentialEnroller::new(),
                refresher.clone(),
                request.persistence_redactor.clone(),
                Arc::new(SystemClock),
            )) as Arc<dyn ProviderCredentialSource>;
            if let Some(pool) = pool {
                let members = request.credentials.clone().map(|resolver| {
                    Arc::new(RenewableMemberSources::new(
                        resolver,
                        target.clone(),
                        refresher.clone(),
                        request.persistence_redactor.clone(),
                    )) as Arc<dyn PoolMemberSources>
                });
                credentials::spawn_chatgpt_usage_probe(
                    pool.clone(),
                    members,
                    active.clone(),
                    target.clone(),
                    account_id.clone(),
                );
            }
            let source =
                credentials::renewable_credential_source(request, pool, &target, refresher, active);
            if let Some(binding) = image_binding {
                remember_image_binding(
                    binding,
                    adapter,
                    if endpoint_for_images.is_empty() {
                        smith_config::setup::CHATGPT_ENDPOINT
                    } else {
                        &endpoint_for_images
                    },
                    target.clone(),
                    source.clone(),
                );
            }
            let config = ChatGptProviderConfig::new(
                request.config.model.value.clone(),
                profile.capabilities.clone(),
                account_id,
            )
            .map_err(FactoryError::Transport)?;
            Ok(Arc::new(ChatGptProvider::new(
                transport, config, target, source,
            )))
        }
        Adapter::XaiResponses => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let mut config = ResponsesConfig::new(
                endpoint.unwrap_or_default(),
                request.config.model.value.clone(),
            );
            config.capabilities = profile.capabilities.clone();
            // xAI runs an implicit prefix cache on its own: a repeated-prefix
            // session reports large cached reads with no request field asked
            // of the adapter. Declare it so cache planning and reporting
            // reflect the reuse instead of claiming none exists.
            config.capabilities.prompt_cache = PromptCacheControl::Implicit;
            config.extra_headers = request
                .config
                .provider
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.value.clone()))
                .collect();
            let secret = secret.ok_or(FactoryError::XaiAuth(
                crate::xai::XaiAuthError::InvalidBundle,
            ))?;
            let bundle = XaiTokenBundle::from_secret(&secret).map_err(FactoryError::XaiAuth)?;
            // The active member's reference, which is where its renewed bundle
            // goes back, so a refresh survives the process rather than being
            // re-earned every launch.
            let reference = credentials::active_credential_reference(request)
                .ok_or(FactoryError::XaiAuth(
                    crate::xai::XaiAuthError::InvalidBundle,
                ))
                .and_then(|reference| {
                    CredentialRef::parse(&reference).map_err(|source| {
                        FactoryError::CredentialReference {
                            provider: request.config.provider.name.value.clone(),
                            source,
                        }
                    })
                })?;
            let target = ProviderCredentialTarget::new(request.config.provider.name.value.clone())
                .map_err(|error| FactoryError::Runtime(RuntimeError::config(error.to_string())))?;
            let refresher: Arc<dyn BundleRefresher<XaiTokenBundle>> =
                Arc::new(XaiOAuthClient::new().map_err(FactoryError::XaiAuth)?);
            let active = Arc::new(XaiCredentialSource::new(
                target.clone(),
                reference,
                bundle,
                CredentialEnroller::new(),
                refresher.clone(),
                request.persistence_redactor.clone(),
                Arc::new(SystemClock),
            )) as Arc<dyn ProviderCredentialSource>;
            let source =
                credentials::renewable_credential_source(request, pool, &target, refresher, active);
            let provider =
                ResponsesProvider::with_credential_source(transport, config, target, source)
                    .map_err(FactoryError::Transport)?;
            // The generic Responses adapter serializes the request identity,
            // while XAI's host boundary may receive a context-only identity
            // from a maintenance/admission caller.  Preserve the exact
            // Runtime identity before delegating; never derive a replacement
            // key from the prompt or session id.
            Ok(Arc::new(XaiCacheIdentityProvider::new(Arc::new(provider))))
        }
        Adapter::GeminiInteractions => {
            let transport = ReqwestTransport::new(request.transport.clone())
                .map_err(FactoryError::Transport)?;
            let secret = secret.ok_or_else(|| {
                FactoryError::Runtime(RuntimeError::config(
                    "native Gemini provider credential is not configured",
                ))
            })?;
            let mut config = GeminiInteractionsConfig::new(
                endpoint.unwrap_or_else(|| smith_config::catalog::GEMINI_ENDPOINT.to_owned()),
                request.config.model.value.clone(),
            );
            config.capabilities = profile.capabilities.clone();
            // Models.dev describes the model; the native adapter owns the
            // implicit prefix-cache behavior it actually drives.
            config.capabilities.prompt_cache = PromptCacheControl::Implicit;
            config =
                config.with_supported_thinking_levels(supported_thinking_levels.iter().cloned());
            let target = ProviderCredentialTarget::new(request.config.provider.name.value.clone())
                .map_err(|error| FactoryError::Runtime(RuntimeError::config(error.to_string())))?;
            let source = credentials::credential_source(request, pool, secret);
            let provider = GeminiInteractionsProvider::with_credential_source(
                transport, config, target, source,
            )
            .map_err(FactoryError::Transport)?;
            // Wrapped here rather than at either call site, so the root
            // session and every preflighted child route get the same
            // treatment: an attributed internal turn is delivered on
            // whichever of them the delegation happened on.
            Ok(crate::gemini::accept_internal_turns(Arc::new(provider)))
        }
    }
}
