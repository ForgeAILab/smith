//! Bounded authenticated discovery and private, account-scoped last-good caches.
//!
//! No raw response or request headers are persisted. These snapshots are inputs
//! to a later immutable host composition, never mutations of a running model.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_runtime_core::store::Secret;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use smith_config::catalog::{CatalogProvider, EndpointCatalog, PartialCatalogLimits};
use smith_config::model::KIND_OPENAI_COMPATIBLE;
use url::Url;

mod normalizing;
mod storage;

#[cfg(test)]
mod tests;

const SCHEMA_REVISION: u32 = 1;
const LISTING_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LISTING_BYTES: usize = 1024 * 1024;
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MODELS: usize = 10_000;

/// The exact authorized serving binding used for discovery.
#[derive(Clone)]
pub struct ProviderModelTarget {
    /// Local configuration identity, preserved in inventory.
    pub name: String,
    /// Configured protocol adapter; only OpenAI-compatible listing is supported.
    pub kind: String,
    /// Configured API base, without credentials, query, or fragment.
    pub base_url: String,
    /// Non-secret broker account/revision identity; rotate when credentials change.
    pub credential_scope: String,
    /// Already available credential. Background refresh never unlocks a store.
    pub bearer: Option<Secret>,
    /// Configured headers, sent only to this exact endpoint.
    pub headers: BTreeMap<String, String>,
}

impl fmt::Debug for ProviderModelTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Even malformed URLs or user-controlled provider names can carry a
        // credential. Debug is intentionally useful without echoing any input.
        formatter
            .debug_struct("ProviderModelTarget")
            .finish_non_exhaustive()
    }
}

/// One source's refresh result, independent of any inventory adoption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    /// The last successful validation is younger than the requested interval.
    Fresh,
    /// A complete new normalized listing was published.
    Updated,
    /// Conditional HTTP validation confirmed the previous revision.
    NotModified,
    /// Refresh failed; a last-good snapshot, when available, remains usable.
    Failed,
}

/// Secret-free status plus the latest usable immutable endpoint catalog.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    /// Outcome for this source.
    pub status: SyncStatus,
    /// Latest usable catalog, including last-good data after a failed refresh.
    pub catalog: Option<EndpointCatalog>,
    /// Bounded diagnostic containing no request values or response body.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedModels {
    schema_revision: u32,
    binding_digest: String,
    kind: String,
    base_url: String,
    source: String,
    revision: String,
    retrieved_at_ms: u64,
    etag: Option<String>,
    catalog: CatalogProvider,
    partial_limits: BTreeMap<String, PartialCatalogLimits>,
    unadvertised_capabilities: BTreeSet<String>,
}

impl CachedModels {
    fn endpoint_catalog(&self) -> EndpointCatalog {
        EndpointCatalog {
            binding_digest: self.binding_digest.clone(),
            kind: self.kind.clone(),
            base_url: self.base_url.clone(),
            source: self.source.clone(),
            revision: self.revision.clone(),
            retrieved_at_ms: self.retrieved_at_ms,
            catalog: self.catalog.clone(),
            partial_limits: self.partial_limits.clone(),
            unadvertised_capabilities: self.unadvertised_capabilities.clone(),
        }
    }
}

struct Binding {
    digest: String,
    base_url: String,
    source: String,
}

impl Binding {
    fn new(target: &ProviderModelTarget) -> Result<Self, String> {
        if target.kind != KIND_OPENAI_COMPATIBLE {
            return Err("this provider adapter has no model-listing reader".to_owned());
        }
        if !normalizing::valid_text(&target.name, 256)
            || !normalizing::valid_text(&target.credential_scope, 1024)
        {
            return Err("invalid provider or credential-scope identity".to_owned());
        }
        let base_url = normalized_endpoint(&target.base_url)?;
        // JSON array boundaries prevent ambiguous string concatenations. The
        // account scope is hashed, never serialized in the cache record.
        let identity = serde_json::to_vec(&[
            target.name.as_str(),
            target.kind.as_str(),
            base_url.as_str(),
            target.credential_scope.as_str(),
        ])
        .map_err(|_| "cannot encode provider-listing identity".to_owned())?;
        Ok(Self {
            digest: digest(&identity),
            source: format!("{base_url}/models"),
            base_url,
        })
    }
}

/// Loads only a schema-valid cache for the exact provider/endpoint/account.
///
/// This is synchronous and network-free so startup can compose immediately.
/// Missing, corrupt, overlarge, non-private, or symlinked caches contribute no
/// metadata. Freshness is deliberately separate from last-good usability.
pub fn load_cached(user_dir: &Path, target: &ProviderModelTarget) -> Option<EndpointCatalog> {
    let binding = Binding::new(target).ok()?;
    let cache = storage::CacheDirectory::open(user_dir, false).ok()?;
    read_cached(&cache, &binding, target).map(|cached| cached.endpoint_catalog())
}

/// Checks a frozen endpoint snapshot against current provider/account authority.
pub fn binding_matches(target: &ProviderModelTarget, catalog: &EndpointCatalog) -> bool {
    Binding::new(target).is_ok_and(|binding| {
        catalog.binding_digest == binding.digest
            && catalog.kind == target.kind
            && catalog.base_url == binding.base_url
            && catalog.catalog.id == target.name
            && catalog.source == binding.source
    })
}

/// Refreshes one endpoint under a cross-process lock and freshness recheck.
///
/// A transport/validation failure returns `Failed` with last-good metadata and
/// preserves its successful validation time. `force` bypasses freshness only.
/// No inference call is made and no configuration is rewritten.
pub async fn refresh(
    user_dir: &Path,
    target: &ProviderModelTarget,
    max_age: Duration,
    force: bool,
) -> Result<SyncOutcome, String> {
    refresh_at(user_dir, target, max_age, force, now_ms).await
}

async fn refresh_at(
    user_dir: &Path,
    target: &ProviderModelTarget,
    max_age: Duration,
    force: bool,
    clock: impl Fn() -> u64,
) -> Result<SyncOutcome, String> {
    let binding = Binding::new(target)?;
    let directory = storage::CacheDirectory::open(user_dir, true)?;
    let _lock = directory.lock(&binding.digest).await?;
    let previous = read_cached(&directory, &binding, target);
    let current_time = clock();
    let max_age_ms = u64::try_from(max_age.as_millis()).unwrap_or(u64::MAX);
    if !force
        && previous.as_ref().is_some_and(|cached| {
            current_time >= cached.retrieved_at_ms
                && current_time - cached.retrieved_at_ms < max_age_ms
        })
    {
        return Ok(SyncOutcome {
            status: SyncStatus::Fresh,
            catalog: previous.map(|cached| cached.endpoint_catalog()),
            detail: None,
        });
    }
    let result = read_listing(
        &binding.base_url,
        target.bearer.as_ref().map(Secret::expose),
        &target.headers,
        previous.as_ref().and_then(|cached| cached.etag.as_deref()),
    )
    .await;
    let normalized = match result {
        Ok(ListingResponse::NotModified) => previous
            .as_ref()
            .filter(|cached| cached.etag.is_some())
            .cloned()
            .ok_or_else(|| "the model listing returned 304 without a cached revision".to_owned())
            .map(|mut cached| {
                cached.retrieved_at_ms = clock();
                (SyncStatus::NotModified, cached)
            }),
        Ok(ListingResponse::Fresh { listing, etag }) => {
            normalizing::normalize_listing(&listing, &target.name).and_then(|catalog| {
                let partial_limits = normalizing::partial_limits(&listing, &catalog);
                let unadvertised_capabilities = normalizing::unadvertised_capabilities(&listing);
                let bytes = serde_json::to_vec(&(
                    &binding.digest,
                    &catalog,
                    &partial_limits,
                    &unadvertised_capabilities,
                ))
                .map_err(|_| "cannot encode normalized model listing".to_owned())?;
                Ok((
                    SyncStatus::Updated,
                    CachedModels {
                        schema_revision: SCHEMA_REVISION,
                        binding_digest: binding.digest.clone(),
                        kind: target.kind.clone(),
                        base_url: binding.base_url.clone(),
                        source: binding.source.clone(),
                        // Content, rather than an ETag, versions prices. A
                        // server reusing its ETag cannot reprice old usage.
                        revision: digest(&bytes),
                        retrieved_at_ms: clock(),
                        etag,
                        catalog,
                        partial_limits,
                        unadvertised_capabilities,
                    },
                ))
            })
        }
        Err(error) => Err(error),
    };
    let (status, cached) = match normalized {
        Ok(value) => value,
        Err(detail) => {
            return Ok(SyncOutcome {
                status: SyncStatus::Failed,
                catalog: previous.map(|cached| cached.endpoint_catalog()),
                detail: Some(detail),
            });
        }
    };
    let bytes =
        serde_json::to_vec(&cached).map_err(|_| "cannot encode provider model cache".to_owned())?;
    if bytes.len() > MAX_CACHE_BYTES {
        return Ok(SyncOutcome {
            status: SyncStatus::Failed,
            catalog: previous.map(|cached| cached.endpoint_catalog()),
            detail: Some("normalized model cache exceeds its size bound".to_owned()),
        });
    }
    if let Err(detail) = directory.publish(&binding.digest, &bytes) {
        return Ok(SyncOutcome {
            status: SyncStatus::Failed,
            catalog: previous.map(|cached| cached.endpoint_catalog()),
            detail: Some(detail),
        });
    }
    Ok(SyncOutcome {
        status,
        catalog: Some(cached.endpoint_catalog()),
        detail: None,
    })
}

fn read_cached(
    directory: &storage::CacheDirectory,
    binding: &Binding,
    target: &ProviderModelTarget,
) -> Option<CachedModels> {
    let bytes = directory.read(&binding.digest, MAX_CACHE_BYTES)?;
    let cached: CachedModels = serde_json::from_slice(&bytes).ok()?;
    if cached.schema_revision != SCHEMA_REVISION
        || cached.binding_digest != binding.digest
        || cached.kind != target.kind
        || cached.base_url != binding.base_url
        || cached.source != binding.source
        || cached.catalog.id != target.name
        || cached.catalog.name != target.name
        || cached.etag.as_deref().is_some_and(|etag| !valid_etag(etag))
        || !normalizing::valid_catalog(&cached.catalog)
        || !normalizing::valid_partial_limits(&cached.partial_limits, &cached.catalog)
        || cached.unadvertised_capabilities.len() > MAX_MODELS
        || !cached.unadvertised_capabilities.iter().all(|id| {
            cached
                .catalog
                .models
                .get(id)
                .is_some_and(|model| !model.tool_call || !model.has_text_output())
        })
    {
        return None;
    }
    let content = serde_json::to_vec(&(
        &binding.digest,
        &cached.catalog,
        &cached.partial_limits,
        &cached.unadvertised_capabilities,
    ))
    .ok()?;
    (digest(&content) == cached.revision).then_some(cached)
}

pub(crate) enum ListingResponse {
    NotModified,
    Fresh {
        listing: Value,
        etag: Option<String>,
    },
}

/// Shared transport for add-model probes and persistent endpoint discovery.
pub(crate) async fn read_listing(
    endpoint: &str,
    bearer: Option<&str>,
    headers: &BTreeMap<String, String>,
    etag: Option<&str>,
) -> Result<ListingResponse, String> {
    // Includes streaming and JSON/listing validation in one total budget.
    tokio::time::timeout(
        LISTING_TIMEOUT,
        read_listing_inner(endpoint, bearer, headers, etag),
    )
    .await
    .map_err(|_| "the model listing exceeded its five-second time bound".to_owned())?
}

async fn read_listing_inner(
    endpoint: &str,
    bearer: Option<&str>,
    headers: &BTreeMap<String, String>,
    etag: Option<&str>,
) -> Result<ListingResponse, String> {
    let source = format!("{}/models", normalized_endpoint(endpoint)?);
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(LISTING_TIMEOUT)
        .timeout(LISTING_TIMEOUT)
        .user_agent(concat!("smith/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "cannot build the model-listing client".to_owned())?;
    let mut request = client.get(&source);
    if headers.len() > 64 {
        return Err("too many configured model-listing headers".to_owned());
    }
    for (name, value) in headers {
        if name.len() > 256 || value.len() > 8192 {
            return Err("a configured model-listing header exceeds its size bound".to_owned());
        }
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "invalid configured model-listing header".to_owned())?;
        // Request routing and transport framing are owned by the client.
        if matches!(
            name.as_str(),
            "host" | "content-length" | "transfer-encoding" | "connection"
        ) {
            return Err("unsupported model-listing transport header".to_owned());
        }
        let value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| "invalid configured model-listing header".to_owned())?;
        request = request.header(name, value);
    }
    if let Some(bearer) = bearer {
        request = request.bearer_auth(bearer);
    }
    if let Some(etag) = etag.filter(|etag| valid_etag(etag)) {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "reading the model listing failed".to_owned())?;
    if response.url().as_str() != source {
        return Err("the model listing came from a different endpoint".to_owned());
    }
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(ListingResponse::NotModified);
    }
    if response.status() != reqwest::StatusCode::OK {
        return Err(format!(
            "the model listing answered HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_LISTING_BYTES as u64)
    {
        return Err("the model listing exceeded its one-MiB size bound".to_owned());
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|etag| valid_etag(etag))
        .map(str::to_owned);
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "reading the model listing body failed".to_owned())?;
        if bytes.len().saturating_add(chunk.len()) > MAX_LISTING_BYTES {
            return Err("the model listing exceeded its one-MiB size bound".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    let listing: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "the model listing was not valid JSON".to_owned())?;
    normalizing::validate_listing(&listing)?;
    Ok(ListingResponse::Fresh { listing, etag })
}

fn normalized_endpoint(endpoint: &str) -> Result<String, String> {
    if endpoint.len() > 4096 {
        return Err("the configured model-listing endpoint is too long".to_owned());
    }
    let mut url = Url::parse(endpoint)
        .map_err(|_| "the configured model-listing endpoint is invalid".to_owned())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "model-listing endpoints require HTTP(S) without URL credentials or options".to_owned(),
        );
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn valid_etag(etag: &str) -> bool {
    !etag.is_empty() && etag.len() <= 512 && etag.bytes().all(|byte| (0x20..0x7f).contains(&byte))
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}
