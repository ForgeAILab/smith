//! Offline-first Models.dev loading and immutable runtime catalog composition.
//!
//! Loading prefers a schema-validated last-good cache and falls back to the
//! generated seed. Refresh is a separate bounded control-plane task: it never
//! mutates the snapshot held by an active picker or runtime.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use agent_runtime_core::catalog::{
    CatalogSource, Modality, ModelCatalogSource, ModelLimits, ModelRecord, StaticSource,
};
use agent_runtime_core::clock::{Clock, SystemClock, Timestamp};
use agent_runtime_core::provider::{AuthKind, Capabilities, PromptCacheControl, ReasoningSupport};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{ETAG, IF_NONE_MATCH, USER_AGENT};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use smith_config::catalog::{
    CATALOG_SCHEMA_REVISION, CatalogLimits, CatalogModality, CatalogModel, CatalogModelCost,
    CatalogProvider, CatalogReasoningControls, CatalogSnapshot, GOOGLE_CATALOG_PROVIDER,
    MODELS_DEV_SOURCE_URL, OPENAI_CATALOG_PROVIDER, OPENROUTER_CATALOG_PROVIDER,
    XAI_CATALOG_PROVIDER, ZAI_CODING_PLAN_CATALOG_PROVIDER, catalog_provider_for,
};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

/// Maximum accepted Models.dev response size.
pub const MAX_REMOTE_CATALOG_BYTES: usize = 8 * 1024 * 1024;

/// Maximum accepted normalized seed/cache size.
pub const MAX_NORMALIZED_CATALOG_BYTES: usize = 2 * 1024 * 1024;

/// How Smith identifies itself when fetching the public catalog.
pub const CATALOG_USER_AGENT: &str = concat!("smith/", env!("CARGO_PKG_VERSION"));

/// A snapshot older than this schedules a background refresh.
pub const DEFAULT_CATALOG_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1_000;

/// The generated catalog embedded into every Smith build.
pub const EMBEDDED_MODELS_DEV_SEED: &str = include_str!("../data/models-dev-seed.json");

const MAX_MODELS_PER_PROVIDER: usize = 10_000;
const MAX_MODEL_ID_BYTES: usize = 512;
const MAX_NAME_BYTES: usize = 256;
const MAX_REVISION_BYTES: usize = 512;
const MAX_DISABLED_REASON_BYTES: usize = 256;
const MAX_MODALITIES: usize = 16;
const MAX_REASONING_OPTION_ENTRIES: usize = 8;
const MAX_REASONING_EFFORTS: usize = 10;
const MAX_REASONING_EFFORT_BYTES: usize = 32;
const EXPECTED_PROVIDERS: [&str; 5] = [
    OPENAI_CATALOG_PROVIDER,
    OPENROUTER_CATALOG_PROVIDER,
    XAI_CATALOG_PROVIDER,
    ZAI_CODING_PLAN_CATALOG_PROVIDER,
    GOOGLE_CATALOG_PROVIDER,
];

/// Why catalog preparation or refresh failed.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// The built-in seed is invalid, which is a build-time defect.
    #[error("embedded model catalog is invalid: {0}")]
    InvalidSeed(String),
    /// A fetched or cached catalog failed validation.
    #[error("model catalog is invalid: {0}")]
    InvalidDocument(String),
    /// The public catalog request failed safely.
    #[error("Models.dev refresh failed: {0}")]
    Fetch(String),
    /// Atomic cache publication failed.
    #[error("model catalog cache update failed: {0}")]
    Cache(String),
}

/// Where the prepared snapshot came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogLoadOrigin {
    /// The generated snapshot shipped with this build.
    Embedded,
    /// A previously validated user cache.
    LastGoodCache,
}

/// One frozen startup result.
#[derive(Debug, Clone)]
pub struct PreparedCatalog {
    /// Immutable metadata used by both inventory and runtime.
    pub snapshot: Arc<CatalogSnapshot>,
    /// Where the snapshot was loaded from.
    pub origin: CatalogLoadOrigin,
    /// Whether freshness policy requested a background refresh.
    pub refresh_scheduled: bool,
}

/// A bounded fetch result from the exact public source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogFetchResponse {
    /// The cached source revision is still current.
    NotModified,
    /// A complete response body and optional public revision.
    Fresh {
        /// Response bytes, already bounded by the fetcher.
        body: Vec<u8>,
        /// ETag or equivalent public source revision.
        revision: Option<String>,
        /// Final response URL, checked again by the loader.
        final_url: String,
    },
}

/// Injectable credential-free Models.dev fetch boundary.
#[async_trait]
pub trait CatalogFetcher: Send + Sync + fmt::Debug {
    /// Fetches the public catalog, optionally using a prior public revision.
    async fn fetch(
        &self,
        if_none_match: Option<&str>,
    ) -> Result<CatalogFetchResponse, CatalogError>;
}

/// Production HTTPS fetcher with redirects disabled and no default headers.
#[derive(Debug, Clone)]
pub struct ModelsDevFetcher {
    client: reqwest::Client,
}

impl ModelsDevFetcher {
    /// Builds the bounded public client.
    pub fn new() -> Result<Self, CatalogError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(3))
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|error| CatalogError::Fetch(error.to_string()))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl CatalogFetcher for ModelsDevFetcher {
    async fn fetch(
        &self,
        if_none_match: Option<&str>,
    ) -> Result<CatalogFetchResponse, CatalogError> {
        // Identify the client. Models.dev filters some agents — `Python-urllib`
        // is refused with a 403 while an unset agent is served — so sending a
        // name of our own keeps Smith out of a bucket it did not choose, and
        // gives the operator something to allow-list.
        let mut request = self
            .client
            .get(MODELS_DEV_SOURCE_URL)
            .header(USER_AGENT, CATALOG_USER_AGENT);
        if let Some(revision) = if_none_match.filter(|revision| valid_revision(revision)) {
            request = request.header(IF_NONE_MATCH, revision);
        }
        let response = request
            .send()
            .await
            .map_err(|error| CatalogError::Fetch(error.to_string()))?;
        if response.url().as_str() != MODELS_DEV_SOURCE_URL {
            return Err(CatalogError::Fetch(
                "response did not come from the exact allowed origin".to_owned(),
            ));
        }
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(CatalogFetchResponse::NotModified);
        }
        if response.status() != reqwest::StatusCode::OK {
            return Err(CatalogError::Fetch(format!(
                "public source returned HTTP {}",
                response.status()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_REMOTE_CATALOG_BYTES as u64)
        {
            return Err(CatalogError::Fetch(
                "response exceeds the 8 MiB limit".to_owned(),
            ));
        }
        let revision = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .filter(|revision| valid_revision(revision))
            .map(str::to_owned);
        let final_url = response.url().as_str().to_owned();
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| CatalogError::Fetch(error.to_string()))?;
            if body.len().saturating_add(chunk.len()) > MAX_REMOTE_CATALOG_BYTES {
                return Err(CatalogError::Fetch(
                    "response exceeds the 8 MiB limit".to_owned(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(CatalogFetchResponse::Fresh {
            body,
            revision,
            final_url,
        })
    }
}

/// Host-owned seed/cache loader and background refresher.
#[derive(Clone)]
pub struct CatalogLoader {
    cache_path: PathBuf,
    seed: Arc<str>,
    fetcher: Arc<dyn CatalogFetcher>,
    clock: Arc<dyn Clock>,
    max_age_ms: u64,
}

impl fmt::Debug for CatalogLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogLoader")
            .field("cache_path", &self.cache_path)
            .field("fetcher", &self.fetcher)
            .field("max_age_ms", &self.max_age_ms)
            .finish_non_exhaustive()
    }
}

impl CatalogLoader {
    /// A production loader rooted below Smith's owner-controlled user state.
    pub fn production(user_dir: &Path) -> Result<Self, CatalogError> {
        Ok(Self::new(
            user_dir.join("cache").join("models-dev-v1.json"),
            Arc::<str>::from(EMBEDDED_MODELS_DEV_SEED),
            Arc::new(ModelsDevFetcher::new()?),
            Arc::new(SystemClock),
        ))
    }

    /// An injectable loader for deterministic tests and embedding hosts.
    pub fn new(
        cache_path: PathBuf,
        seed: Arc<str>,
        fetcher: Arc<dyn CatalogFetcher>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            cache_path,
            seed,
            fetcher,
            clock,
            max_age_ms: DEFAULT_CATALOG_MAX_AGE_MS,
        }
    }

    /// Overrides freshness policy.
    #[must_use]
    pub fn with_max_age_ms(mut self, max_age_ms: u64) -> Self {
        self.max_age_ms = max_age_ms;
        self
    }

    /// Loads a frozen last-good/embedded snapshot and optionally refreshes later.
    pub async fn prepare(&self, allow_refresh: bool) -> Result<PreparedCatalog, CatalogError> {
        let seed = parse_snapshot(self.seed.as_bytes())
            .map_err(|error| CatalogError::InvalidSeed(error.to_string()))?;
        let cached = read_bounded(&self.cache_path)
            .await
            .ok()
            .and_then(|bytes| parse_snapshot(&bytes).ok());
        let (snapshot, origin) = match cached {
            Some(snapshot) => (snapshot, CatalogLoadOrigin::LastGoodCache),
            None => (seed, CatalogLoadOrigin::Embedded),
        };
        let stale = self
            .clock
            .now()
            .as_millis()
            .saturating_sub(snapshot.retrieved_at_ms)
            >= self.max_age_ms;
        let refresh_scheduled =
            allow_refresh && stale && schedule_refresh(self.clone(), snapshot.clone());
        Ok(PreparedCatalog {
            snapshot: Arc::new(snapshot),
            origin,
            refresh_scheduled,
        })
    }

    /// The advisory lock guarding this cache against other Smith processes.
    fn lock_path(&self) -> PathBuf {
        let mut path = self.cache_path.clone();
        let name = path
            .file_name()
            .map(|name| format!("{}.lock", name.to_string_lossy()))
            .unwrap_or_else(|| "catalog.lock".to_owned());
        path.set_file_name(name);
        path
    }

    /// Performs one refresh and atomically publishes it for a later snapshot.
    ///
    /// Held under an advisory file lock, because the in-process guard only
    /// stops one Smith from racing itself. Several agents running side by side
    /// would otherwise each notice the same stale snapshot and each fetch the
    /// same bytes. The write was always atomic, so this costs redundant
    /// requests rather than a corrupt cache — but a public endpoint should not
    /// see N identical fetches because one machine started N agents.
    pub async fn refresh(&self, current: &CatalogSnapshot) -> Result<(), CatalogError> {
        let Some(_lock) = CacheLock::try_acquire(&self.lock_path()).await? else {
            // Another process holds it and is doing this work already.
            return Ok(());
        };

        // Re-read under the lock. If someone published while we waited, their
        // snapshot is newer than the one we were handed and refetching would
        // only discard it. Deliberately narrower than a freshness test: this
        // asks "did another process already do this work", not "is the cache
        // young", so a caller refreshing a snapshot for its own reasons is
        // still allowed to.
        if let Ok(bytes) = read_bounded(&self.cache_path).await
            && let Ok(published) = parse_snapshot(&bytes)
            && published.retrieved_at_ms > current.retrieved_at_ms
        {
            return Ok(());
        }

        let response = self.fetcher.fetch(Some(&current.source_revision)).await?;
        let next = match response {
            CatalogFetchResponse::NotModified => {
                let mut next = current.clone();
                next.retrieved_at_ms = self.clock.now().as_millis();
                next
            }
            CatalogFetchResponse::Fresh {
                body,
                revision,
                final_url,
            } => {
                if final_url != MODELS_DEV_SOURCE_URL {
                    return Err(CatalogError::Fetch(
                        "response did not come from the exact allowed origin".to_owned(),
                    ));
                }
                normalize_remote(&body, self.clock.now().as_millis(), revision.as_deref())?
            }
        };
        publish_atomic(&self.cache_path, &next).await
    }
}

fn schedule_refresh(loader: CatalogLoader, current: CatalogSnapshot) -> bool {
    let Some(guard) = RefreshGuard::acquire(loader.cache_path.clone()) else {
        return false;
    };
    tokio::spawn(async move {
        let _guard = guard;
        if let Err(error) = loader.refresh(&current).await {
            tracing::debug!(%error, "Models.dev background refresh kept the last-good catalog");
        }
    });
    true
}

/// An advisory exclusive lock on the catalog cache, released when dropped.
///
/// The kernel releases it if the process dies, so a crash mid-refresh cannot
/// leave a lock nobody can clear — which is exactly the failure a hand-rolled
/// lockfile with a liveness heuristic would have.
struct CacheLock {
    file: std::fs::File,
}

impl CacheLock {
    async fn try_acquire(path: &Path) -> Result<Option<Self>, CatalogError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| CatalogError::Cache(error.to_string()))?;
        }
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            use fs2::FileExt;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)
                .map_err(|error| CatalogError::Cache(error.to_string()))?;
            match file.try_lock_exclusive() {
                Ok(()) => Ok(Some(Self { file })),
                Err(_) => Ok(None),
            }
        })
        .await
        .map_err(|error| CatalogError::Cache(error.to_string()))?
    }
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        use fs2::FileExt;
        let _ = FileExt::unlock(&self.file);
    }
}

static ACTIVE_REFRESHES: OnceLock<Mutex<BTreeSet<PathBuf>>> = OnceLock::new();

struct RefreshGuard {
    path: PathBuf,
}

impl RefreshGuard {
    fn acquire(path: PathBuf) -> Option<Self> {
        let active = ACTIVE_REFRESHES.get_or_init(|| Mutex::new(BTreeSet::new()));
        let mut active = active.lock().expect("catalog refresh registry poisoned");
        active.insert(path.clone()).then(|| Self { path })
    }
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        if let Some(active) = ACTIVE_REFRESHES.get() {
            active
                .lock()
                .expect("catalog refresh registry poisoned")
                .remove(&self.path);
        }
    }
}

async fn read_bounded(path: &Path) -> Result<Vec<u8>, CatalogError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| CatalogError::Cache(error.to_string()))?;
    if metadata.len() > MAX_NORMALIZED_CATALOG_BYTES as u64 {
        return Err(CatalogError::InvalidDocument(
            "normalized cache exceeds the 2 MiB limit".to_owned(),
        ));
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| CatalogError::Cache(error.to_string()))?;
    if bytes.len() > MAX_NORMALIZED_CATALOG_BYTES {
        return Err(CatalogError::InvalidDocument(
            "normalized cache exceeds the 2 MiB limit".to_owned(),
        ));
    }
    Ok(bytes)
}

async fn publish_atomic(path: &Path, snapshot: &CatalogSnapshot) -> Result<(), CatalogError> {
    let bytes = serde_json::to_vec_pretty(snapshot)
        .map_err(|error| CatalogError::Cache(error.to_string()))?;
    if bytes.len() > MAX_NORMALIZED_CATALOG_BYTES {
        return Err(CatalogError::Cache(
            "normalized cache exceeds the 2 MiB limit".to_owned(),
        ));
    }
    parse_snapshot(&bytes)?;
    let parent = path
        .parent()
        .ok_or_else(|| CatalogError::Cache("cache path has no parent".to_owned()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| CatalogError::Cache(error.to_string()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("models-dev-v1.json");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .await
            .map_err(|error| CatalogError::Cache(error.to_string()))?;
        file.write_all(&bytes)
            .await
            .map_err(|error| CatalogError::Cache(error.to_string()))?;
        file.write_all(b"\n")
            .await
            .map_err(|error| CatalogError::Cache(error.to_string()))?;
        file.sync_all()
            .await
            .map_err(|error| CatalogError::Cache(error.to_string()))?;
        drop(file);
        tokio::fs::rename(&temporary, path)
            .await
            .map_err(|error| CatalogError::Cache(error.to_string()))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

mod parsing;
mod runtime_source;

use parsing::{normalize_remote, parse_snapshot, valid_revision};
pub use runtime_source::runtime_catalog_source;

#[cfg(test)]
mod tests;
