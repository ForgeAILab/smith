//! Credential parsing, resolution, pooling, and renewal.

use super::*;
use smith_config::resolve::ResolvedMcpTransport;

/// Collects every deduplicated environment variable name referenced via
/// `CredentialRef::Env` across a resolved configuration (primary provider and
/// pool, command-provider env, fallback routes, web-search credentials,
/// checkpoint key credential, and MCP server credentials).
pub fn resolved_credential_env_vars(config: &ResolvedConfig) -> Arc<[String]> {
    let mut vars = BTreeSet::new();
    collect_config_credential_env_vars(config, &mut vars);
    Arc::from(vars.into_iter().collect::<Vec<_>>())
}

/// Collects every deduplicated environment variable name referenced via
/// `CredentialRef::Env` across a runtime request, including its primary config
/// and any pre-resolved vision, advisor, compaction, and child profile configs.
pub fn request_credential_env_vars(request: &RuntimeRequest) -> Arc<[String]> {
    let mut vars = BTreeSet::new();
    collect_config_credential_env_vars(&request.config, &mut vars);
    if let Some(vision) = &request.vision_profile {
        collect_config_credential_env_vars(&vision.config, &mut vars);
    }
    if let Some(advisor) = &request.advisor_profile {
        collect_config_credential_env_vars(&advisor.config, &mut vars);
    }
    if let Some(compaction) = &request.compaction_profile {
        collect_config_credential_env_vars(&compaction.config, &mut vars);
    }
    for child in &request.child_profiles {
        collect_config_credential_env_vars(&child.config, &mut vars);
        if let Some(vision) = &child.vision_profile {
            collect_config_credential_env_vars(&vision.config, &mut vars);
        }
    }
    Arc::from(vars.into_iter().collect::<Vec<_>>())
}

fn collect_config_credential_env_vars(config: &ResolvedConfig, vars: &mut BTreeSet<String>) {
    for reference in &config.provider.credentials {
        collect_parsed_credential_ref(&reference.value, vars);
    }
    if let Some(command) = &config.provider.command {
        collect_mcp_values(&command.env, vars);
    }
    for fallback in &config.fallback.models {
        collect_config_credential_env_vars(&fallback.config, vars);
    }
    if let Some(api_key) = &config.web.search.api_key {
        collect_credential_ref(&api_key.value, vars);
    }
    if let Some(checkpoint_key) = &config.persistence.checkpoint_key_credential {
        collect_parsed_credential_ref(&checkpoint_key.value, vars);
    }
    for server in config.mcp.servers.values() {
        collect_mcp_values(&server.env, vars);
        match &server.transport {
            ResolvedMcpTransport::Stdio { .. } => {}
            ResolvedMcpTransport::StreamableHttp {
                credential,
                headers,
                ..
            } => {
                if let Some(credential) = credential {
                    collect_parsed_credential_ref(&credential.value, vars);
                }
                collect_mcp_values(headers, vars);
            }
        }
    }
}

fn collect_mcp_values(values: &BTreeMap<String, Sourced<McpValue>>, vars: &mut BTreeSet<String>) {
    for value in values.values() {
        match &value.value {
            McpValue::Credential(reference) => collect_parsed_credential_ref(reference, vars),
            McpValue::Literal(_) => {}
        }
    }
}

fn collect_parsed_credential_ref(reference: &str, vars: &mut BTreeSet<String>) {
    if let Ok(parsed) = CredentialRef::parse(reference) {
        collect_credential_ref(&parsed, vars);
    }
}

fn collect_credential_ref(reference: &CredentialRef, vars: &mut BTreeSet<String>) {
    match reference {
        CredentialRef::Env { variable } => {
            vars.insert(variable.clone());
        }
        CredentialRef::Keychain { .. }
        | CredentialRef::AuthFile { .. }
        | CredentialRef::SessionJson { .. }
        | CredentialRef::File { .. } => {}
    }
}

/// The credential source an adapter leases from.
///
/// With a pool this reads whichever member is active at the moment of the
/// attempt, which is what makes a rotation take effect without rebuilding the
/// adapter. The already-resolved secret seeds the active member's source so it
/// is not read from the credential service twice at startup.
pub(super) fn credential_source(
    request: &RuntimeRequest,
    pool: Option<&SharedPool>,
    secret: Secret,
) -> Arc<dyn ProviderCredentialSource> {
    match (pool, request.credentials.clone()) {
        (Some(pool), Some(resolver)) => {
            let source = PoolCredentialSource::new(
                pool.clone(),
                Arc::new(StaticMemberSources::new(resolver)),
            );
            let active = pool.read(CredentialPool::active_position);
            source.seed(
                active,
                Arc::new(StaticProviderCredentialSource::new(secret)),
            );
            Arc::new(source) as Arc<dyn ProviderCredentialSource>
        }
        // No pool, or nothing to resolve one with: the session has a single
        // secret and it is already in hand.
        _ => Arc::new(StaticProviderCredentialSource::new(secret))
            as Arc<dyn ProviderCredentialSource>,
    }
}

/// The credential source for a browser-login adapter.
///
/// With a pool, each member is its own renewable source persisting to its own
/// reference; the already-built active source is seeded so the startup secret
/// is not read from the credential service twice. Without one, the active
/// source is the whole story — exactly the pre-pool behavior.
pub(super) fn renewable_credential_source<B: RenewableBundle>(
    request: &RuntimeRequest,
    pool: Option<&SharedPool>,
    target: &ProviderCredentialTarget,
    refresher: Arc<dyn BundleRefresher<B>>,
    active: Arc<dyn ProviderCredentialSource>,
) -> Arc<dyn ProviderCredentialSource> {
    match (pool, request.credentials.clone()) {
        (Some(pool), Some(resolver)) => {
            let members = RenewableMemberSources::new(
                resolver,
                target.clone(),
                refresher,
                request.persistence_redactor.clone(),
            );
            let source = PoolCredentialSource::new(pool.clone(), Arc::new(members));
            let position = pool.read(CredentialPool::active_position);
            source.seed(position, active);
            Arc::new(source) as Arc<dyn ProviderCredentialSource>
        }
        _ => active,
    }
}

/// The reference whose secret authorizes this session's first attempt.
///
/// With a pool this is the *remembered active* member, not the first declared
/// one: a session resumed on the second account must authenticate as the
/// second account, not lease the first account's secret from the active
/// position until an invalidation happens to correct it.
pub(in crate::factory) fn active_credential_reference(request: &RuntimeRequest) -> Option<String> {
    if let Some(pool) = credential_pool_for(request)
        && let Some(reference) =
            pool.read(|pool| pool.active().map(|member| member.reference.clone()))
    {
        return Some(reference);
    }
    request
        .config
        .provider
        .credential()
        .map(|reference| reference.value.clone())
}

/// Builds the pool for this provider, or `None` when it declares one account.
pub(in crate::factory) fn credential_pool_for(request: &RuntimeRequest) -> Option<SharedPool> {
    if !request.config.provider.has_pool() {
        return None;
    }
    // A host-supplied pool is already seeded with the remembered account and
    // is the handle the surfaces draw from, so it wins over building a fresh
    // one that would silently start back at the first member.
    Some(request.credential_pool.clone().unwrap_or_else(|| {
        SharedPool::new(CredentialPool::new(
            request.config.provider.name.value.clone(),
            request
                .config
                .provider
                .credentials
                .iter()
                .map(|reference| reference.value.clone()),
            request
                .config
                .provider
                .rotate_at_percent
                .as_ref()
                .map(|threshold| threshold.value),
        ))
    }))
}

/// Wraps `provider` so a spent account can offer to move to another one.
///
/// Outermost on purpose. Rotation replays the whole attempt, so it has to sit
/// outside the response-policy and reasoning-dialect decorators — a replay
/// that skipped them would produce a differently normalized turn than the one
/// it replaced.
pub(super) fn apply_credential_pool(
    request: &RuntimeRequest,
    provider: Arc<dyn Provider>,
    pool: Option<SharedPool>,
) -> Arc<dyn Provider> {
    // A provider with one account behaves exactly as it did before pools
    // existed: no wrapper, no offer, no extra state.
    let Some(pool) = pool else {
        return provider;
    };
    // No surface to ask means declining, which `HeadlessRotation` does while
    // recording the exhaustion for machine output.
    let policy = request
        .rotation
        .clone()
        .unwrap_or_else(|| Arc::new(HeadlessRotation::new()) as Arc<dyn RotationPolicy>);
    Arc::new(
        PooledProvider::new(
            provider,
            pool,
            policy,
            request
                .clock
                .clone()
                .unwrap_or_else(|| Arc::new(SystemClock) as Arc<dyn Clock>),
        )
        .defer_quota_rotation(
            request.defer_quota_rotation || !request.config.fallback.models.is_empty(),
        ),
    ) as Arc<dyn Provider>
}

/// Resolves a configured credential reference into a secret.
///
/// The resolver's backend is synchronous and may wait on an unlock prompt. A
/// dedicated thread keeps that wait off the executor; a bounded async receive
/// lets startup fail actionably even if the platform call itself cannot be
/// cancelled.
/// Parses every declared pool member's reference, resolving none of them.
///
/// Parsing is free and offline; resolving opens the credential service and can
/// prompt. Checking shape for all members while reading the value of only the
/// active one is what lets a misconfigured pool fail early without turning
/// startup into one keychain prompt per account.
pub(in crate::factory) fn validate_pool_references(
    request: &RuntimeRequest,
) -> Result<(), FactoryError> {
    for reference in &request.config.provider.credentials {
        CredentialRef::parse(&reference.value).map_err(|source| {
            FactoryError::CredentialReference {
                provider: request.config.provider.name.value.clone(),
                source,
            }
        })?;
    }
    Ok(())
}

pub(in crate::factory) async fn secret(
    request: &RuntimeRequest,
    reference: &str,
) -> Result<Secret, FactoryError> {
    let provider = request.config.provider.name.value.clone();
    let reference =
        CredentialRef::parse(reference).map_err(|source| FactoryError::CredentialReference {
            provider: provider.clone(),
            source,
        })?;
    let resolver = request
        .credentials
        .clone()
        .ok_or_else(|| FactoryError::MissingHostPolicy {
            what: "credential resolver",
            message: format!(
                "provider `{provider}` configures the credential `{reference}`, and nothing was \
                 supplied that can resolve it"
            ),
        })?;

    let timeout_ms = request.credential_timeout_ms.max(1);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("smith-credential-lookup".into())
        .spawn(move || {
            let _ = sender.send(resolver.resolve_blocking(&reference));
        })
        .map_err(|_| FactoryError::CredentialTask)?;

    match tokio::time::timeout(Duration::from_millis(timeout_ms), receiver).await {
        Ok(Ok(result)) => result.map_err(FactoryError::Credential),
        Ok(Err(_)) => Err(FactoryError::CredentialTask),
        Err(_) => Err(FactoryError::CredentialTimeout { timeout_ms }),
    }
}

/// Reads every pool member's server-reported usage once at session start and
/// records it in the credential pool, so the account surfaces can show a
/// measured number for each account before it has served an attempt.
///
/// The active member reuses the source already built for the session; the
/// others are built from their references, which for this provider are
/// `authfile:` entries that resolve without prompting. Members run
/// sequentially so two token refreshes never race on the auth file.
///
/// Best-effort by design: a failure leaves that member unmeasured, and the
/// surfaces already render an unmeasured member honestly as "usage unknown".
/// Requires a running Tokio runtime; without one there is no session about to
/// serve attempts, so there is nothing to probe for.
pub(super) fn spawn_chatgpt_usage_probe(
    pool: SharedPool,
    members: Option<Arc<dyn PoolMemberSources>>,
    active_source: Arc<dyn ProviderCredentialSource>,
    target: ProviderCredentialTarget,
    active_account: String,
) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        let (active_position, roster) = pool.read(|pool| {
            (
                pool.active_position(),
                pool.members()
                    .iter()
                    .map(|member| (member.position, member.reference.clone()))
                    .collect::<Vec<_>>(),
            )
        });
        for (position, reference) in roster {
            let source = if position == active_position {
                active_source.clone()
            } else if let Some(members) = &members {
                match members.build(position, &reference).await {
                    Ok(source) => source,
                    Err(_) => continue,
                }
            } else {
                continue;
            };
            let Ok(lease) = source
                .acquire(
                    &target,
                    crate::chatgpt::CHATGPT_CREDENTIAL_MINIMUM_VALIDITY_MS,
                    &Cancellation::new(),
                    Deadline::never(),
                )
                .await
            else {
                continue;
            };
            // Only the active member may fall back to the account identity
            // frozen at construction: pairing another member's token with it
            // would query the wrong account's usage.
            let account = match (lease.account(), position == active_position) {
                (Some(account), _) => account.to_owned(),
                (None, true) => active_account.clone(),
                (None, false) => continue,
            };
            if let Ok(snapshot) =
                crate::chatgpt::fetch_usage_snapshot(lease.secret().expose(), &account).await
            {
                pool.write(|pool| pool.record_snapshot(position, snapshot));
            }
        }
    });
}
