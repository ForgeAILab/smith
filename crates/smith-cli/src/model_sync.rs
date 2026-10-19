//! Process-owned, bounded metadata synchronization and immutable cache loading.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agent_runtime_core::store::Secret;
use anyhow::{Context, Result};
use futures_util::{StreamExt, stream};
use sha2::{Digest, Sha256};
use smith_config::catalog::{CatalogSnapshot, EndpointCatalog, catalog_provider_for};
use smith_config::credential::{CredentialRef, CredentialResolver};
use smith_config::model::{ConfigFile, KIND_OPENAI_COMPATIBLE};
use smith_config::resolve::{ResolvedProvider, resolve_user_model_sync, resolve_user_provider};
use smith_runtime::model_catalog::CatalogLoader;
use smith_runtime::provider_models::{
    ProviderModelTarget, SyncStatus, binding_matches, load_cached, refresh,
};

const POLL: Duration = Duration::from_secs(60);
const CREDENTIAL_BUDGET: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
pub(super) struct CatalogUpdate {
    user_dir: PathBuf,
    fingerprint: String,
}

fn updates() -> &'static tokio::sync::watch::Sender<CatalogUpdate> {
    static UPDATES: OnceLock<tokio::sync::watch::Sender<CatalogUpdate>> = OnceLock::new();
    UPDATES.get_or_init(|| tokio::sync::watch::channel(CatalogUpdate::default()).0)
}

pub(super) fn subscribe() -> tokio::sync::watch::Receiver<CatalogUpdate> {
    let mut receiver = updates().subscribe();
    receiver.mark_changed();
    receiver
}

pub(super) fn differs(update: &CatalogUpdate, user_dir: &Path, catalog: &CatalogSnapshot) -> bool {
    update.user_dir == user_dir && update.fingerprint != fingerprint(catalog)
}

pub(super) fn snapshots_differ(left: &CatalogSnapshot, right: &CatalogSnapshot) -> bool {
    fingerprint(left) != fingerprint(right)
}

fn fingerprint(catalog: &CatalogSnapshot) -> String {
    let mut digest = Sha256::new();
    digest.update(catalog.source_revision.as_bytes());
    digest.update(catalog.retrieved_at_ms.to_le_bytes());
    for (name, endpoint) in &catalog.endpoint_catalogs {
        digest.update(name.as_bytes());
        digest.update(endpoint.revision.as_bytes());
        digest.update(endpoint.retrieved_at_ms.to_le_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// Reads only user-controlled provider declarations. Project settings never
/// authorize forwarding a credential to a discovery endpoint.
fn provider_names(user_dir: &Path) -> Result<Vec<String>> {
    let path = user_dir.join("config.toml");
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => anyhow::bail!("could not read user configuration for model synchronization"),
    };
    let file = ConfigFile::parse(&text)
        .map_err(|_| anyhow::anyhow!("invalid user configuration for model synchronization"))?;
    Ok(file.providers.into_keys().collect())
}

struct MemoCredential {
    declaration: String,
    bearer: Secret,
}

type CredentialMemos = BTreeMap<(PathBuf, String), MemoCredential>;
fn credential_memos() -> &'static Mutex<CredentialMemos> {
    static MEMOS: OnceLock<Mutex<CredentialMemos>> = OnceLock::new();
    MEMOS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn declaration(provider: &ResolvedProvider) -> String {
    let mut digest = Sha256::new();
    for value in [
        provider.name.value.as_str(),
        provider.kind.value.as_str(),
        provider
            .base_url
            .as_ref()
            .map_or("", |value| value.value.as_str()),
    ] {
        scope_field(&mut digest, value.as_bytes());
    }
    for reference in &provider.credentials {
        scope_field(&mut digest, reference.value.as_bytes());
    }
    for (name, value) in &provider.headers {
        scope_field(&mut digest, name.as_bytes());
        scope_field(&mut digest, value.value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// Match the runtime's remembered account, validating it against the declared
/// pool. The bounded owner-only file contains references, never credentials.
fn selected_account(user_dir: &Path, mut provider: ResolvedProvider) -> ResolvedProvider {
    if provider.credentials.len() < 2 {
        return provider;
    }
    let read = || -> Option<String> {
        use std::io::Read;
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
        }
        let file = options.open(user_dir.join("accounts.json")).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > 65_536 {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.permissions().mode() & 0o777 != 0o600
                || metadata.uid() != std::fs::metadata(user_dir).ok()?.uid()
            {
                return None;
            }
        }
        let mut bytes = Vec::new();
        file.take(65_537).read_to_end(&mut bytes).ok()?;
        if bytes.len() > 65_536 {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        if value.get("revision")?.as_u64()? != 1 {
            return None;
        }
        Some(
            value
                .get("active")?
                .get(&provider.name.value)?
                .as_str()?
                .to_owned(),
        )
    };
    if let Some(reference) = read()
        && let Some(position) = provider
            .credentials
            .iter()
            .position(|candidate| candidate.value == reference)
    {
        provider.credentials.rotate_left(position);
    }
    provider
}

fn memoized_bearer(user_dir: &Path, provider: &ResolvedProvider) -> Option<Secret> {
    let memos = credential_memos().lock().ok()?;
    let memo = memos.get(&(user_dir.to_path_buf(), provider.name.value.clone()))?;
    (memo.declaration == declaration(provider)).then(|| memo.bearer.clone())
}

fn remember_bearer(user_dir: &Path, provider: &ResolvedProvider, bearer: &Secret) {
    if let Ok(mut memos) = credential_memos().lock() {
        memos.insert(
            (user_dir.to_path_buf(), provider.name.value.clone()),
            MemoCredential {
                declaration: declaration(provider),
                bearer: bearer.clone(),
            },
        );
    }
}

fn available_reference(reference: &CredentialRef) -> bool {
    matches!(
        reference,
        CredentialRef::Env { .. } | CredentialRef::AuthFile { .. }
    )
}

fn target(provider: ResolvedProvider, bearer: Option<Secret>) -> Option<ProviderModelTarget> {
    if provider.kind.value != KIND_OPENAI_COMPATIBLE {
        return None;
    }
    let base_url = provider.base_url.as_ref()?.value.clone();
    let mut scope = Sha256::new();
    if let Some(reference) = provider.credential() {
        scope_field(&mut scope, reference.value.as_bytes());
    }
    if let Some(secret) = &bearer {
        scope_field(&mut scope, secret.expose().as_bytes());
    }
    let headers: BTreeMap<_, _> = provider
        .headers
        .into_iter()
        .map(|(name, value)| (name, value.value))
        .collect();
    for (name, value) in &headers {
        scope_field(&mut scope, name.as_bytes());
        scope_field(&mut scope, value.as_bytes());
    }
    Some(ProviderModelTarget {
        name: provider.name.value,
        kind: provider.kind.value,
        base_url,
        credential_scope: format!("{:x}", scope.finalize()),
        bearer,
        headers,
    })
}

fn scope_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update(bytes.len().to_le_bytes());
    digest.update(bytes);
}

/// Cache enumeration can reuse immediately available credentials, never unlock
/// a service. The credential digest changes when the account or key changes.
fn available_target(user_dir: &Path, name: &str) -> Result<Option<ProviderModelTarget>> {
    let provider = selected_account(
        user_dir,
        resolve_user_provider(user_dir, name).map_err(|error| anyhow::anyhow!("{error}"))?,
    );
    if provider.kind.value != KIND_OPENAI_COMPATIBLE {
        return Ok(None);
    }
    let bearer = match (&provider.api_key, provider.credential()) {
        (Some(secret), _) => Some(secret.value.clone()),
        (None, Some(reference)) => {
            let reference = CredentialRef::parse(&reference.value)?;
            if !available_reference(&reference) {
                let Some(secret) = memoized_bearer(user_dir, &provider) else {
                    return Ok(None);
                };
                Some(secret)
            } else {
                match CredentialResolver::new(user_dir).resolve_blocking(&reference) {
                    Ok(secret) => Some(secret),
                    Err(_) => return Ok(None),
                }
            }
        }
        (None, None) => None,
    };
    Ok(target(provider, bearer))
}

/// An unresponsive OS credential backend must not pin Tokio runtime shutdown.
/// Its dedicated OS thread can finish independently after the async budget.
async fn bounded_credential_lookup<F>(lookup: F, budget: Duration) -> Result<Secret>
where
    F: FnOnce() -> std::result::Result<Secret, smith_config::credential::CredentialError>
        + Send
        + 'static,
{
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("smith-model-sync-credential".into())
        .spawn(move || {
            let _ = sender.send(lookup());
        })
        .context("could not start the provider credential resolver")?;
    tokio::time::timeout(budget, receiver)
        .await
        .map_err(|_| anyhow::anyhow!("credential resolution exceeded its time budget"))?
        .context("credential resolver stopped")?
        .map_err(|_| anyhow::anyhow!("the provider credential is unavailable"))
}

async fn explicit_target(user_dir: &Path, name: &str) -> Result<Option<ProviderModelTarget>> {
    let provider = selected_account(
        user_dir,
        resolve_user_provider(user_dir, name).map_err(|error| anyhow::anyhow!("{error}"))?,
    );
    if provider.kind.value != KIND_OPENAI_COMPATIBLE {
        return Ok(None);
    }
    let bearer = match (&provider.api_key, provider.credential()) {
        (Some(secret), _) => Some(secret.value.clone()),
        (None, Some(reference)) => {
            let reference = CredentialRef::parse(&reference.value)?;
            let resolver = CredentialResolver::new(user_dir);
            let secret = bounded_credential_lookup(
                move || resolver.resolve_blocking(&reference),
                CREDENTIAL_BUDGET,
            )
            .await?;
            Some(secret)
        }
        (None, None) => None,
    };
    if let Some(secret) = &bearer {
        remember_bearer(user_dir, &provider, secret);
    }
    Ok(target(provider, bearer))
}

/// Confirms a frozen picker/setup snapshot still belongs to the current
/// authorized endpoint and account. A mismatch fails before provider I/O.
pub(super) fn validate_frozen_snapshot(user_dir: &Path, snapshot: &CatalogSnapshot) -> Result<()> {
    for (name, catalog) in &snapshot.endpoint_catalogs {
        let current = available_target(user_dir, name)?;
        if !current
            .as_ref()
            .is_some_and(|target| binding_matches(target, catalog))
        {
            anyhow::bail!(
                "provider model snapshot binding changed; reopen the model picker or setup review"
            );
        }
    }
    Ok(())
}

/// Setup's final review resolves its selected account normally and validates
/// the exact frozen listing before any configuration transaction is committed.
pub(super) async fn validate_frozen_catalog(
    user_dir: &Path,
    name: &str,
    catalog: &EndpointCatalog,
) -> Result<()> {
    let target = explicit_target(user_dir, name).await?;
    if target
        .as_ref()
        .is_some_and(|target| binding_matches(target, catalog))
    {
        return Ok(());
    }
    anyhow::bail!(
        "provider model snapshot binding changed; reopen setup to discover the current provider metadata"
    )
}

/// Normal active-provider credential preparation may unlock its configured
/// backend. This loads metadata only; startup never waits for a network fetch.
pub(super) async fn prepare_active_cache(
    resolution: &smith_config::resolve::Resolution,
    mut snapshot: Arc<CatalogSnapshot>,
    frozen: bool,
) -> Result<Arc<CatalogSnapshot>> {
    let user_dir = &resolution.layout.user_dir;
    let name = &resolution.config.provider.name.value;
    // An effective serving override must not unlock a different user account.
    if !smith_config::resolve::endpoint_discovery_authorized(resolution, name)
        .map_err(|error| anyhow::anyhow!("{error}"))?
    {
        return catalog_for_resolution(resolution, snapshot);
    }
    // A project-only provider grants no authenticated discovery authority.
    if provider_names(user_dir)?
        .iter()
        .any(|candidate| candidate == name)
        && let Some(target) = explicit_target(user_dir, name).await?
        && !frozen
    {
        let snapshot = Arc::make_mut(&mut snapshot);
        snapshot.endpoint_catalogs.remove(name);
        if let Some(catalog) = load_cached(user_dir, &target) {
            snapshot.endpoint_catalogs.insert(name.to_owned(), catalog);
        }
    }
    validate_frozen_snapshot(user_dir, &snapshot)?;
    suppress_pool_prices(user_dir, &mut snapshot)?;
    Ok(snapshot)
}

/// Prices from one account cannot be assigned to a rotating credential pool.
/// Suppress only the composed endpoint rates; private cache bytes remain intact
/// and public Models.dev rates retain their existing reviewed semantics.
fn suppress_pool_prices(user_dir: &Path, snapshot: &mut Arc<CatalogSnapshot>) -> Result<()> {
    let pools = snapshot
        .endpoint_catalogs
        .keys()
        .filter_map(|name| match resolve_user_provider(user_dir, name) {
            Ok(provider) if provider.has_pool() => Some(Ok(name.clone())),
            Ok(_) => None,
            Err(error) => Some(Err(anyhow::anyhow!("{error}"))),
        })
        .collect::<Result<Vec<_>>>()?;
    for name in pools {
        let has_prices = snapshot
            .endpoint_catalogs
            .get(&name)
            .is_some_and(|endpoint| {
                endpoint
                    .catalog
                    .models
                    .values()
                    .any(|model| model.cost.is_some())
            });
        if has_prices
            && let Some(endpoint) = Arc::make_mut(snapshot).endpoint_catalogs.get_mut(&name)
        {
            for model in endpoint.catalog.models.values_mut() {
                model.cost = None;
            }
        }
    }
    Ok(())
}

/// Reject endpoint metadata when the effective serving authority differs from
/// the user-authorized source, including project/environment credential or
/// header overrides. This performs no credential or network access.
pub(super) fn catalog_for_resolution(
    resolution: &smith_config::resolve::Resolution,
    mut snapshot: Arc<CatalogSnapshot>,
) -> Result<Arc<CatalogSnapshot>> {
    let mut unauthorized = Vec::new();
    for name in snapshot.endpoint_catalogs.keys() {
        if !smith_config::resolve::endpoint_discovery_authorized(resolution, name)
            .map_err(|error| anyhow::anyhow!("{error}"))?
        {
            unauthorized.push(name.clone());
        }
    }
    if !unauthorized.is_empty() {
        let snapshot = Arc::make_mut(&mut snapshot);
        for name in unauthorized {
            snapshot.endpoint_catalogs.remove(&name);
        }
    }
    suppress_pool_prices(&resolution.layout.user_dir, &mut snapshot)?;
    Ok(snapshot)
}

/// Loads endpoint caches into a fresh public snapshot before inventory and
/// preflight. Invalid/mismatched caches contribute no metadata.
pub(super) fn cached_snapshot(user_dir: &Path) -> Result<Arc<CatalogSnapshot>> {
    resolve_user_model_sync(user_dir).map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut snapshot = CatalogLoader::production(user_dir)?.snapshot()?;
    for name in provider_names(user_dir)? {
        if let Ok(Some(target)) = available_target(user_dir, &name)
            && let Some(catalog) = load_cached(user_dir, &target)
        {
            Arc::make_mut(&mut snapshot)
                .endpoint_catalogs
                .insert(name, catalog);
        }
    }
    suppress_pool_prices(user_dir, &mut snapshot)?;
    Ok(snapshot)
}

pub(super) async fn sync_one_provider(
    user_dir: &Path,
    name: &str,
    force: bool,
) -> Result<Option<EndpointCatalog>> {
    let policy = resolve_user_model_sync(user_dir).map_err(|error| anyhow::anyhow!("{error}"))?;
    let Some(target) = explicit_target(user_dir, name).await? else {
        return Ok(None);
    };
    let age = Duration::from_secs(u64::from(policy.interval_hours.value) * 60 * 60);
    let outcome = refresh(user_dir, &target, age, force)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(outcome.catalog)
}

struct SourceReport {
    name: String,
    status: &'static str,
    retrieved_at_ms: Option<u64>,
    retained: bool,
    failed: bool,
    metadata: Option<String>,
}

impl SourceReport {
    fn failed(name: String, cached: Option<&EndpointCatalog>) -> Self {
        Self {
            name,
            status: "failed",
            retrieved_at_ms: cached.map(|catalog| catalog.retrieved_at_ms),
            retained: cached.is_some(),
            failed: true,
            metadata: cached.map(endpoint_description),
        }
    }
}

fn endpoint_description(catalog: &EndpointCatalog) -> String {
    let priced = catalog
        .catalog
        .models
        .values()
        .filter(|model| model.cost.is_some())
        .count();
    format!(
        "source {} · revision {} · {} models · {} with published rates (other prices unknown)",
        catalog.source,
        catalog.revision,
        catalog.catalog.models.len(),
        priced
    )
}

#[derive(Clone)]
enum Job {
    Endpoint(ProviderModelTarget),
    Public,
}

impl Job {
    fn name(&self) -> &str {
        match self {
            Self::Endpoint(target) => &target.name,
            Self::Public => "Models.dev",
        }
    }
}

async fn run_job(user_dir: &Path, job: Job, age: Duration, force: bool) -> SourceReport {
    match job {
        Job::Endpoint(target) => match refresh(user_dir, &target, age, force).await {
            Ok(outcome) => SourceReport {
                name: target.name,
                status: match outcome.status {
                    SyncStatus::Fresh => "fresh",
                    SyncStatus::Updated => "updated",
                    SyncStatus::NotModified => "unchanged",
                    SyncStatus::Failed => "failed",
                },
                retrieved_at_ms: outcome
                    .catalog
                    .as_ref()
                    .map(|catalog| catalog.retrieved_at_ms),
                retained: outcome.status == SyncStatus::Failed && outcome.catalog.is_some(),
                failed: outcome.status == SyncStatus::Failed,
                metadata: outcome.catalog.as_ref().map(endpoint_description),
            },
            Err(_) => {
                SourceReport::failed(target.name.clone(), load_cached(user_dir, &target).as_ref())
            }
        },
        Job::Public => {
            let result = async {
                let loader = CatalogLoader::production(user_dir)?;
                let current = loader.snapshot()?;
                let due = now_ms().saturating_sub(current.retrieved_at_ms)
                    >= u64::try_from(age.as_millis()).unwrap_or(u64::MAX);
                if force || due {
                    loader.refresh(&current).await?;
                }
                let next = loader.snapshot()?;
                Ok::<_, smith_runtime::model_catalog::CatalogError>(SourceReport {
                    name: "Models.dev".into(),
                    status: if force || due {
                        if next.source_revision == current.source_revision {
                            "unchanged"
                        } else {
                            "updated"
                        }
                    } else {
                        "fresh"
                    },
                    retrieved_at_ms: Some(next.retrieved_at_ms),
                    retained: false,
                    failed: false,
                    metadata: Some(format!(
                        "source {} · revision {}",
                        next.source_url, next.source_revision
                    )),
                })
            }
            .await;
            result.unwrap_or_else(|_| SourceReport {
                name: "Models.dev".into(),
                status: "failed",
                retrieved_at_ms: CatalogLoader::production(user_dir)
                    .ok()
                    .and_then(|loader| loader.snapshot().ok())
                    .map(|snapshot| snapshot.retrieved_at_ms),
                retained: true,
                failed: true,
                metadata: Some(format!(
                    "source {}",
                    smith_config::catalog::MODELS_DEV_SOURCE_URL
                )),
            })
        }
    }
}

async fn jobs(
    user_dir: &Path,
    selected: Option<&str>,
    explicit: bool,
) -> Result<(Vec<Job>, Vec<SourceReport>)> {
    // Validate the policy before resolving a credential, even for an explicit command.
    resolve_user_model_sync(user_dir).map_err(|error| anyhow::anyhow!("{error}"))?;
    let names = provider_names(user_dir)?;
    if let Some(name) = selected
        && !names.iter().any(|candidate| candidate == name)
    {
        anyhow::bail!("provider `{name}` is not configured in user configuration");
    }
    let mut jobs = Vec::new();
    let mut reports = Vec::new();
    let mut public = false;
    for name in names
        .into_iter()
        .filter(|name| selected.is_none_or(|selected| selected == name))
    {
        let provider =
            resolve_user_provider(user_dir, &name).map_err(|error| anyhow::anyhow!("{error}"))?;
        public |= catalog_provider_for(
            &provider.kind.value,
            provider.base_url.as_ref().map(|value| value.value.as_str()),
        )
        .is_some();
        if provider.kind.value != KIND_OPENAI_COMPATIBLE {
            reports.push(SourceReport {
                name,
                status: "skipped (listing adapter unavailable)",
                retrieved_at_ms: None,
                retained: false,
                failed: false,
                metadata: None,
            });
            continue;
        }
        let resolved = if explicit {
            explicit_target(user_dir, &name).await
        } else {
            available_target(user_dir, &name)
        };
        match resolved {
            Ok(Some(target)) => jobs.push(Job::Endpoint(target)),
            Ok(None) => reports.push(SourceReport {
                name,
                status: "deferred (credential unavailable without unlocking)",
                retrieved_at_ms: None,
                retained: false,
                failed: false,
                metadata: None,
            }),
            Err(_) => reports.push(SourceReport::failed(name, None)),
        }
    }
    if public {
        jobs.push(Job::Public);
    }
    Ok((jobs, reports))
}

pub(super) async fn run(provider: Option<&str>, force: bool) -> Result<u8> {
    let user_dir = smith_config::credential::user_state_root()?;
    let policy = resolve_user_model_sync(&user_dir).map_err(|error| anyhow::anyhow!("{error}"))?;
    let age = Duration::from_secs(u64::from(policy.interval_hours.value) * 3_600);
    let (jobs, mut reports) = jobs(&user_dir, provider, true).await?;
    reports.extend(
        stream::iter(
            jobs.into_iter()
                .map(|job| run_job(&user_dir, job, age, force)),
        )
        .buffer_unordered(2)
        .collect::<Vec<_>>()
        .await,
    );
    reports.sort_by(|left, right| left.name.cmp(&right.name));
    let failed = reports.iter().any(|report| report.failed);
    if reports.is_empty() {
        println!("No eligible configured model sources.");
    }
    for report in reports {
        let age = report
            .retrieved_at_ms
            .map(|time| format!(" · age {}s", now_ms().saturating_sub(time) / 1_000))
            .unwrap_or_default();
        let retained = if report.retained {
            " · last-good metadata retained"
        } else {
            ""
        };
        println!("{}: {}{age}{retained}", report.name, report.status);
        if let Some(metadata) = report.metadata {
            println!("  {metadata}");
        }
    }
    publish_update(&user_dir);
    Ok(u8::from(failed))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn publish_update(user_dir: &Path) {
    if let Ok(snapshot) = cached_snapshot(user_dir) {
        let next = CatalogUpdate {
            user_dir: user_dir.to_path_buf(),
            fingerprint: fingerprint(&snapshot),
        };
        updates().send_if_modified(|current| {
            if current.user_dir == next.user_dir && current.fingerprint == next.fingerprint {
                return false;
            }
            *current = next;
            true
        });
    }
}

/// Owns timer and requests for exactly one process run. Dropping it cancels all
/// async work; no detached refresh or operating-system service survives it.
pub(super) struct BackgroundSync {
    task: tokio::task::JoinHandle<()>,
    user_dir: PathBuf,
}

impl BackgroundSync {
    pub(super) fn start(user_dir: PathBuf) -> Self {
        let owned_user_dir = user_dir.clone();
        let task = tokio::spawn(async move {
            let mut retry: BTreeMap<String, Retry> = BTreeMap::new();
            let mut timer = tokio::time::interval(POLL);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                let Ok(policy) = resolve_user_model_sync(&user_dir) else {
                    continue;
                };
                if !policy.enabled.value {
                    continue;
                }
                let age = Duration::from_secs(u64::from(policy.interval_hours.value) * 3_600);
                let Ok((jobs, _)) = jobs(&user_dir, None, false).await else {
                    continue;
                };
                let now = Instant::now();
                let due = jobs
                    .into_iter()
                    .filter(|job| retry.get(job.name()).is_none_or(|retry| retry.due(now)))
                    .collect::<Vec<_>>();
                let reports = stream::iter(
                    due.into_iter()
                        .map(|job| run_job(&user_dir, job, age, false)),
                )
                .buffer_unordered(2)
                .collect::<Vec<_>>()
                .await;
                for report in reports {
                    if report.failed {
                        retry.entry(report.name).or_default().failed(Instant::now());
                    } else {
                        retry.remove(&report.name);
                    }
                }
                publish_update(&user_dir);
            }
        });
        Self {
            task,
            user_dir: owned_user_dir,
        }
    }
}

impl Drop for BackgroundSync {
    fn drop(&mut self) {
        self.task.abort();
        if let Ok(mut memos) = credential_memos().lock() {
            memos.retain(|(user_dir, _), _| user_dir != &self.user_dir);
        }
    }
}

#[derive(Default)]
struct Retry {
    attempts: usize,
    next: Option<Instant>,
}
impl Retry {
    fn due(&self, now: Instant) -> bool {
        self.next.is_none_or(|next| now >= next)
    }
    fn failed(&mut self, now: Instant) {
        let seconds = [60, 300, 900, 3_600][self.attempts.min(3)];
        self.attempts = self.attempts.saturating_add(1);
        self.next = Some(now + Duration::from_secs(seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_never_opens_interactive_credential_backends() {
        for reference in [
            "keychain:smith/acme",
            "file:secret.enc",
            "session-json:/tmp/auth.json#/token",
        ] {
            assert!(!available_reference(
                &CredentialRef::parse(reference).unwrap()
            ));
        }
        assert!(available_reference(
            &CredentialRef::parse("authfile:acme").unwrap()
        ));
        assert!(available_reference(
            &CredentialRef::parse("env:ACME_KEY").unwrap()
        ));
    }

    #[test]
    fn failure_backoff_is_bounded_and_sleep_coalesces_one_retry() {
        let start = Instant::now();
        let mut retry = Retry::default();
        for seconds in [60, 300, 900, 3_600, 3_600] {
            retry.failed(start);
            assert!(!retry.due(start + Duration::from_secs(seconds - 1)));
            assert!(retry.due(start + Duration::from_secs(seconds)));
        }
        assert!(retry.due(start + Duration::from_secs(24 * 3_600)));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use smith_config::auth_file::{AuthFileBackend, OwnerOnlyAuthFile};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn configuration(user_dir: &Path, endpoint: &str, credential: &str) {
        std::fs::create_dir_all(user_dir).unwrap();
        std::fs::write(user_dir.join("config.toml"), format!(
            "[providers.xiaoge]\nkind = \"openai-compatible\"\nbase_url = {endpoint:?}\n{credential}\n"
        )).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                user_dir.join("config.toml"),
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
    }

    #[tokio::test]
    async fn explicit_refresh_and_restart_compose_same_scoped_metadata_without_config_rewrites() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
            assert!(request.starts_with("get /v1/models "));
            assert!(request.contains("authorization: bearer test-inline-key\r\n"));
            let body = r#"{"data":[{"id":"selected-large","limits":{"context":1048576,"input":1048576,"output":256000},"modalities":{"input":["text"],"output":["text"]},"capabilities":{"tool_call":true}}]}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let home = tempfile::tempdir().unwrap();
        let user_dir = home.path().join(".smith");
        configuration(&user_dir, &endpoint, "api_key = \"test-inline-key\"");
        let before = std::fs::read(user_dir.join("config.toml")).unwrap();
        let refreshed = sync_one_provider(&user_dir, "xiaoge", false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            refreshed.catalog.models["selected-large"]
                .limits
                .unwrap()
                .max_output_tokens,
            256_000
        );
        server.await.unwrap();
        let restarted = cached_snapshot(&user_dir).unwrap();
        assert_eq!(
            restarted.endpoint_catalogs["xiaoge"].revision,
            refreshed.revision
        );
        // Fresh offline reuse performs no second request.
        let offline = sync_one_provider(&user_dir, "xiaoge", false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(offline.revision, refreshed.revision);
        assert_eq!(std::fs::read(user_dir.join("config.toml")).unwrap(), before);
        configuration(&user_dir, &endpoint, "api_key = \"rotated-key\"");
        assert!(
            cached_snapshot(&user_dir)
                .unwrap()
                .endpoint_catalogs
                .is_empty()
        );
    }

    #[test]
    fn auth_file_rotation_changes_cache_scope_and_background_defers_keychain() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = home.path().join(".smith");
        configuration(
            &user_dir,
            "https://example.test/v1",
            "credential = \"authfile:xiaoge\"",
        );
        let auth = OwnerOnlyAuthFile::new(&user_dir);
        auth.store("xiaoge", &Secret::new("first-key")).unwrap();
        let first = available_target(&user_dir, "xiaoge").unwrap().unwrap();
        auth.store("xiaoge", &Secret::new("second-key")).unwrap();
        let second = available_target(&user_dir, "xiaoge").unwrap().unwrap();
        assert_ne!(first.credential_scope, second.credential_scope);
        assert!(!first.credential_scope.contains("first-key"));
        configuration(
            &user_dir,
            "https://example.test/v1",
            "credential = \"keychain:smith/xiaoge\"",
        );
        assert!(available_target(&user_dir, "xiaoge").unwrap().is_none());
    }

    #[tokio::test]
    async fn background_collects_inactive_configured_sources_without_unlocking() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = home.path().join(".smith");
        configuration(
            &user_dir,
            "https://example.test/v1",
            "credential = \"keychain:smith/xiaoge\"",
        );
        let (pending, reports) = jobs(&user_dir, None, false).await.unwrap();
        assert!(pending.is_empty());
        assert_eq!(reports.len(), 1);
        assert!(reports[0].status.starts_with("deferred"));
        let mut config = std::fs::read_to_string(user_dir.join("config.toml")).unwrap();
        config.push_str("\n[providers.router]\nkind = \"openai-compatible\"\nbase_url = \"https://openrouter.ai/api/v1\"\n");
        std::fs::write(user_dir.join("config.toml"), config).unwrap();
        let (jobs, reports) = jobs(&user_dir, None, false).await.unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().any(|job| matches!(job, Job::Public)));
        assert_eq!(reports.len(), 1);
    }

    #[tokio::test]
    async fn remembered_account_changes_the_scope_and_cannot_select_an_undeclared_reference() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = home.path().join(".smith");
        configuration(
            &user_dir,
            "https://example.test/v1",
            "credentials = [\"authfile:first\", \"authfile:second\"]",
        );
        let auth = OwnerOnlyAuthFile::new(&user_dir);
        auth.store("first", &Secret::new("key-one")).unwrap();
        auth.store("second", &Secret::new("key-two")).unwrap();
        let first = available_target(&user_dir, "xiaoge").unwrap().unwrap();
        let mut accounts = smith_runtime::pool_state::ActiveAccounts::load(&user_dir).await;
        accounts.remember("xiaoge", "authfile:second");
        accounts.save().await.unwrap();
        let second = available_target(&user_dir, "xiaoge").unwrap().unwrap();
        assert_ne!(first.credential_scope, second.credential_scope);
        assert_eq!(second.bearer.as_ref().unwrap().expose(), "key-two");
        accounts.remember("xiaoge", "authfile:not-declared");
        accounts.save().await.unwrap();
        let fallback = available_target(&user_dir, "xiaoge").unwrap().unwrap();
        assert_eq!(first.credential_scope, fallback.credential_scope);
    }

    #[tokio::test]
    async fn process_ownership_cancels_timer_on_exit_even_with_sync_disabled() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = home.path().join(".smith");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::write(
            user_dir.join("config.toml"),
            "[model_sync]\nenabled = false\n",
        )
        .unwrap();
        let sync = BackgroundSync::start(user_dir);
        let handle = sync.task.abort_handle();
        tokio::task::yield_now().await;
        assert!(!handle.is_finished());
        drop(sync);
        tokio::task::yield_now().await;
        assert!(handle.is_finished());
    }
}

#[cfg(test)]
mod scope_regressions {
    use super::*;
    use smith_config::catalog::CatalogProvider;
    use smith_config::resolve::{Overrides, ResolveRequest, resolve};

    fn snapshot() -> Arc<CatalogSnapshot> {
        let mut snapshot: CatalogSnapshot =
            serde_json::from_str(smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED).unwrap();
        let mut model = snapshot
            .providers
            .values()
            .flat_map(|provider| provider.models.values())
            .find(|model| model.limits.is_some() && model.cost.is_some())
            .unwrap()
            .clone();
        model.id = "selected-large".into();
        snapshot.endpoint_catalogs.insert(
            "xiaoge".into(),
            EndpointCatalog {
                binding_digest: "fixture-binding".into(),
                kind: KIND_OPENAI_COMPATIBLE.into(),
                base_url: "https://example.test/v1".into(),
                source: "https://example.test/v1/models".into(),
                revision: "fixture-revision".into(),
                retrieved_at_ms: 1,
                catalog: CatalogProvider {
                    id: "xiaoge".into(),
                    name: "Xiaoge".into(),
                    models: BTreeMap::from([("selected-large".into(), model)]),
                },
                partial_limits: BTreeMap::new(),
                unadvertised_capabilities: Default::default(),
            },
        );
        Arc::new(snapshot)
    }

    fn configure(user_dir: &Path, auth: &str) {
        std::fs::create_dir_all(user_dir).unwrap();
        std::fs::write(user_dir.join("config.toml"), format!(
            "[providers.xiaoge]\nkind = \"openai-compatible\"\nbase_url = \"https://example.test/v1\"\n{auth}\n"
        )).unwrap();
    }

    #[test]
    fn pool_prices_are_suppressed_in_frozen_compositions_without_mutating_source_snapshot() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let user_dir = home.path().join(".smith");
        configure(
            &user_dir,
            "credentials = [\"authfile:first\", \"authfile:second\"]",
        );
        let raw = snapshot();
        let request = ResolveRequest::new(project.path())
            .with_home_dir(home.path())
            .with_catalog(raw.clone())
            .with_cli(Overrides {
                provider: Some("xiaoge".into()),
                model: Some("selected-large".into()),
                ..Default::default()
            });
        let resolution = resolve(&request).unwrap();
        let composed = catalog_for_resolution(&resolution, raw.clone()).unwrap();
        assert!(
            composed.endpoint_catalogs["xiaoge"].catalog.models["selected-large"]
                .cost
                .is_none()
        );
        assert!(
            raw.endpoint_catalogs["xiaoge"].catalog.models["selected-large"]
                .cost
                .is_some()
        );
        assert_eq!(
            raw.providers, composed.providers,
            "public Models.dev rates remain available"
        );
        let guarded = crate::config_command::authorize_catalog(request).unwrap();
        assert!(
            guarded.catalog.unwrap().endpoint_catalogs["xiaoge"]
                .catalog
                .models["selected-large"]
                .cost
                .is_none()
        );
    }

    #[tokio::test]
    async fn pooled_cache_keeps_account_rates_on_disk_but_removes_them_from_loaded_and_frozen_snapshots()
     {
        use smith_config::auth_file::{AuthFileBackend, OwnerOnlyAuthFile};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            let body = r#"{"data":[{"id":"selected-large","limits":{"context":1048576,"input":1048576,"output":256000},"modalities":{"input":["text"],"output":["text"]},"capabilities":{"tool_call":true},"cost":{"input":1,"output":2}}]}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let user_dir = home.path().join(".smith");
        configure(
            &user_dir,
            "credentials = [\"authfile:first\", \"authfile:second\"]",
        );
        let config = std::fs::read_to_string(user_dir.join("config.toml"))
            .unwrap()
            .replace("https://example.test/v1", &endpoint);
        std::fs::write(user_dir.join("config.toml"), config).unwrap();
        OwnerOnlyAuthFile::new(&user_dir)
            .store("first", &Secret::new("first-key"))
            .unwrap();
        let raw = sync_one_provider(&user_dir, "xiaoge", true)
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
        assert!(raw.catalog.models["selected-large"].cost.is_some());
        let composed = cached_snapshot(&user_dir).unwrap();
        assert!(
            composed.endpoint_catalogs["xiaoge"].catalog.models["selected-large"]
                .cost
                .is_none()
        );
        let target = available_target(&user_dir, "xiaoge").unwrap().unwrap();
        assert!(
            load_cached(&user_dir, &target).unwrap().catalog.models["selected-large"]
                .cost
                .is_some()
        );
        let mut frozen = CatalogLoader::production(&user_dir)
            .unwrap()
            .snapshot()
            .unwrap();
        Arc::make_mut(&mut frozen)
            .endpoint_catalogs
            .insert("xiaoge".into(), raw);
        let resolution = resolve(
            &ResolveRequest::new(project.path())
                .with_home_dir(home.path())
                .with_catalog(frozen.clone())
                .with_cli(Overrides {
                    provider: Some("xiaoge".into()),
                    model: Some("selected-large".into()),
                    ..Default::default()
                }),
        )
        .unwrap();
        let prepared = prepare_active_cache(&resolution, frozen.clone(), true)
            .await
            .unwrap();
        assert!(
            prepared.endpoint_catalogs["xiaoge"].catalog.models["selected-large"]
                .cost
                .is_none()
        );
        assert!(
            frozen.endpoint_catalogs["xiaoge"].catalog.models["selected-large"]
                .cost
                .is_some()
        );
    }

    #[tokio::test]
    async fn effective_account_override_drops_metadata_before_user_keychain_preparation() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let user_dir = home.path().join(".smith");
        configure(
            &user_dir,
            "credential = \"keychain:smith/sync-test-must-never-unlock\"",
        );
        std::fs::create_dir_all(project.path().join(".smith")).unwrap();
        std::fs::write(project.path().join(".smith/config.toml"), "[providers.xiaoge]\ncredential = \"env:OTHER_ACCOUNT\"\n[providers.xiaoge.headers]\nx-tenant = \"different-tenant\"\n").unwrap();
        let raw = snapshot();
        let request = ResolveRequest::new(project.path())
            .with_home_dir(home.path())
            .with_catalog(raw.clone())
            .with_cli(Overrides {
                provider: Some("xiaoge".into()),
                model: Some("selected-large".into()),
                ..Default::default()
            });
        let resolution = resolve(&request).unwrap();
        let active = prepare_active_cache(&resolution, raw.clone(), true)
            .await
            .unwrap();
        assert!(active.endpoint_catalogs.is_empty());
        let guarded = crate::config_command::authorize_catalog(request).unwrap();
        assert!(guarded.catalog.unwrap().endpoint_catalogs.is_empty());
        assert!(raw.endpoint_catalogs.contains_key("xiaoge"));
    }

    #[test]
    fn timed_out_credential_thread_does_not_hold_runtime_shutdown_open() {
        let (release, blocked) = std::sync::mpsc::channel();
        let (finished, completion) = std::sync::mpsc::channel();
        let runner = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            let result = runtime.block_on(bounded_credential_lookup(
                move || {
                    blocked.recv().unwrap();
                    Ok(Secret::new("late-value"))
                },
                Duration::from_millis(10),
            ));
            assert!(result.is_err());
            drop(runtime);
            finished.send(()).unwrap();
        });
        let stopped = completion.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        assert!(
            stopped.is_ok(),
            "the runtime exits while the OS backend remains blocked"
        );
        runner.join().unwrap();
    }
}
