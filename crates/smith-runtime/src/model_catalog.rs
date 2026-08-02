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
use agent_runtime_core::provider::{AuthKind, Capabilities, ReasoningSupport};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{ETAG, IF_NONE_MATCH};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use smith_config::catalog::{
    CATALOG_SCHEMA_REVISION, CatalogLimits, CatalogModality, CatalogModel, CatalogProvider,
    CatalogReasoningControls, CatalogSnapshot, MODELS_DEV_SOURCE_URL, OPENAI_CATALOG_PROVIDER,
    OPENROUTER_CATALOG_PROVIDER, ZAI_CODING_PLAN_CATALOG_PROVIDER, catalog_provider_for,
};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

/// Maximum accepted Models.dev response size.
pub const MAX_REMOTE_CATALOG_BYTES: usize = 8 * 1024 * 1024;

/// Maximum accepted normalized seed/cache size.
pub const MAX_NORMALIZED_CATALOG_BYTES: usize = 2 * 1024 * 1024;

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
const EXPECTED_PROVIDERS: [&str; 3] = [
    OPENAI_CATALOG_PROVIDER,
    OPENROUTER_CATALOG_PROVIDER,
    ZAI_CODING_PLAN_CATALOG_PROVIDER,
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
        let mut request = self.client.get(MODELS_DEV_SOURCE_URL);
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

    /// Performs one refresh and atomically publishes it for a later snapshot.
    pub async fn refresh(&self, current: &CatalogSnapshot) -> Result<(), CatalogError> {
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

/// Builds one immutable cached-remote source scoped to a configured provider.
pub fn runtime_catalog_source(
    snapshot: &CatalogSnapshot,
    local_provider: &str,
    kind: &str,
    base_url: Option<&str>,
) -> Option<Arc<dyn ModelCatalogSource>> {
    let catalog_provider = catalog_provider_for(kind, base_url)?;
    let provider = snapshot.provider(catalog_provider)?;
    let mut source =
        StaticSource::new("models.dev", CatalogSource::CachedRemote).for_provider(local_provider);
    for model in provider.models.values().filter(|model| {
        model.disabled_reason.is_none()
            && model.tool_call
            && model.has_text_output()
            && model.limits.is_some()
    }) {
        let limits = model.limits.expect("filtered above");
        let mut record = ModelRecord::new()
            .with_limits(ModelLimits::new(
                limits.context_tokens,
                limits.max_input_tokens,
                limits.max_output_tokens,
            ))
            .with_capabilities(Capabilities {
                streaming: true,
                tools: model.tool_call,
                reasoning: if model.reasoning {
                    ReasoningSupport::Fixed
                } else {
                    ReasoningSupport::Unsupported
                },
                structured_output: model.structured_output,
                usage: true,
                cache: false,
                auth: AuthKind::ApiKey,
                continuation: false,
                max_output_tokens: Some(limits.max_output_tokens),
            })
            .with_revision(snapshot.source_revision.clone());
        record.retrieved = Some(Timestamp(snapshot.retrieved_at_ms));
        record.input_modalities = Some(
            model
                .input_modalities
                .iter()
                .copied()
                .map(runtime_modality)
                .collect(),
        );
        record.output_modalities = Some(
            model
                .output_modalities
                .iter()
                .copied()
                .map(runtime_modality)
                .collect(),
        );
        source = source.with_model(&model.id, record);
    }
    Some(Arc::new(source))
}

fn runtime_modality(modality: CatalogModality) -> Modality {
    match modality {
        CatalogModality::Text => Modality::Text,
        CatalogModality::Image => Modality::Image,
        CatalogModality::Audio => Modality::Audio,
        CatalogModality::Video => Modality::Video,
        CatalogModality::Document => Modality::Document,
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

fn parse_snapshot(bytes: &[u8]) -> Result<CatalogSnapshot, CatalogError> {
    if bytes.len() > MAX_NORMALIZED_CATALOG_BYTES {
        return Err(CatalogError::InvalidDocument(
            "normalized document exceeds the 2 MiB limit".to_owned(),
        ));
    }
    let snapshot: CatalogSnapshot = serde_json::from_slice(bytes)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn validate_snapshot(snapshot: &CatalogSnapshot) -> Result<(), CatalogError> {
    invalid_if(
        snapshot.schema_revision != CATALOG_SCHEMA_REVISION,
        "unsupported schema revision",
    )?;
    invalid_if(
        snapshot.source_url != MODELS_DEV_SOURCE_URL,
        "wrong catalog source origin",
    )?;
    invalid_if(
        !valid_digest(&snapshot.source_digest) || !valid_digest(&snapshot.content_digest),
        "catalog digest is malformed",
    )?;
    invalid_if(
        !valid_revision(&snapshot.source_revision),
        "catalog revision is malformed",
    )?;
    invalid_if(snapshot.retrieved_at_ms == 0, "retrieval time is missing")?;
    let actual: BTreeSet<&str> = snapshot.providers.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = EXPECTED_PROVIDERS.into_iter().collect();
    invalid_if(actual != expected, "supported provider set does not match")?;

    for (provider_id, provider) in &snapshot.providers {
        validate_text(provider_id, 128, "provider id")?;
        invalid_if(
            provider.id != *provider_id,
            "provider id does not match its key",
        )?;
        validate_text(&provider.name, MAX_NAME_BYTES, "provider name")?;
        invalid_if(
            provider.models.len() > MAX_MODELS_PER_PROVIDER,
            "provider model count exceeds the limit",
        )?;
        for (model_id, model) in &provider.models {
            validate_text(model_id, MAX_MODEL_ID_BYTES, "model id")?;
            invalid_if(model.id != *model_id, "model id does not match its key")?;
            validate_text(&model.name, MAX_NAME_BYTES, "model name")?;
            if let Some(reason) = &model.disabled_reason {
                validate_text(reason, MAX_DISABLED_REASON_BYTES, "disabled reason")?;
            }
            invalid_if(
                model.input_modalities.len() > MAX_MODALITIES
                    || model.output_modalities.len() > MAX_MODALITIES,
                "model has too many modalities",
            )?;
            invalid_if(
                !sorted_unique(&model.input_modalities) || !sorted_unique(&model.output_modalities),
                "model modalities are not normalized",
            )?;
            if let Some(limits) = model.limits {
                validate_limits(limits)?;
            } else {
                invalid_if(
                    model.disabled_reason.is_none(),
                    "model without limits has no disabled reason",
                )?;
            }
            if let Some(controls) = &model.reasoning_controls {
                invalid_if(
                    !model.reasoning,
                    "reasoning controls are present on a non-reasoning model",
                )?;
                invalid_if(
                    !controls.toggle && controls.efforts.is_empty(),
                    "reasoning controls advertise neither a switch nor efforts",
                )?;
                invalid_if(
                    controls.efforts.len() > MAX_REASONING_EFFORTS,
                    "model advertises too many reasoning efforts",
                )?;
                invalid_if(
                    !controls.efforts.iter().all(|effort| {
                        valid_effort_name(effort)
                            && controls.efforts.iter().filter(|e| *e == effort).count() == 1
                    }),
                    "reasoning efforts are not normalized",
                )?;
            }
        }
    }
    let value = serde_json::to_value(&snapshot.providers)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    invalid_if(
        digest(&canonical) != snapshot.content_digest,
        "normalized content digest does not match",
    )
}

fn validate_limits(limits: CatalogLimits) -> Result<(), CatalogError> {
    invalid_if(
        limits.context_tokens == 0 || limits.max_input_tokens == 0 || limits.max_output_tokens == 0,
        "model limit is zero",
    )?;
    invalid_if(
        limits.max_output_tokens > limits.context_tokens,
        "model output limit exceeds context",
    )?;
    invalid_if(
        limits.max_input_tokens > limits.context_tokens,
        "model input limit exceeds context",
    )
}

fn normalize_remote(
    bytes: &[u8],
    retrieved_at_ms: u64,
    source_revision: Option<&str>,
) -> Result<CatalogSnapshot, CatalogError> {
    invalid_if(
        bytes.len() > MAX_REMOTE_CATALOG_BYTES,
        "remote document exceeds the 8 MiB limit",
    )?;
    invalid_if(retrieved_at_ms == 0, "retrieval time is missing")?;
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    let root = root
        .as_object()
        .ok_or_else(|| CatalogError::InvalidDocument("root is not an object".to_owned()))?;
    let mut providers = BTreeMap::new();
    for provider_id in EXPECTED_PROVIDERS {
        let raw = root.get(provider_id).ok_or_else(|| {
            CatalogError::InvalidDocument(format!("supported provider `{provider_id}` is missing"))
        })?;
        providers.insert(
            provider_id.to_owned(),
            normalize_provider(provider_id, raw)?,
        );
    }
    let value = serde_json::to_value(&providers)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    let source_digest = digest(bytes);
    let revision = source_revision
        .filter(|revision| valid_revision(revision))
        .unwrap_or(&source_digest)
        .to_owned();
    let snapshot = CatalogSnapshot {
        schema_revision: CATALOG_SCHEMA_REVISION,
        source_url: MODELS_DEV_SOURCE_URL.to_owned(),
        source_digest,
        content_digest: digest(&canonical),
        source_revision: revision,
        retrieved_at_ms,
        providers,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn normalize_provider(provider_id: &str, raw: &Value) -> Result<CatalogProvider, CatalogError> {
    let raw = raw.as_object().ok_or_else(|| {
        CatalogError::InvalidDocument(format!("provider `{provider_id}` is not an object"))
    })?;
    let id = required_text(raw, "id", 128, "provider id")?;
    invalid_if(id != provider_id, "provider id does not match its key")?;
    let name = required_text(raw, "name", MAX_NAME_BYTES, "provider name")?.to_owned();
    let models = raw
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CatalogError::InvalidDocument(format!("provider `{provider_id}` has no model object"))
        })?;
    invalid_if(
        models.len() > MAX_MODELS_PER_PROVIDER,
        "provider model count exceeds the limit",
    )?;
    let mut normalized = BTreeMap::new();
    for (model_id, model) in models {
        if let Some(model) = normalize_model(model_id, model)? {
            normalized.insert(model_id.clone(), model);
        }
    }
    Ok(CatalogProvider {
        id: provider_id.to_owned(),
        name,
        models: normalized,
    })
}

fn normalize_model(model_id: &str, raw: &Value) -> Result<Option<CatalogModel>, CatalogError> {
    validate_text(model_id, MAX_MODEL_ID_BYTES, "model id")?;
    let raw = raw.as_object().ok_or_else(|| {
        CatalogError::InvalidDocument(format!("model `{model_id}` is not an object"))
    })?;
    let id = required_text(raw, "id", MAX_MODEL_ID_BYTES, "model id")?;
    invalid_if(id != model_id, "model id does not match its key")?;
    let name = required_text(raw, "name", MAX_NAME_BYTES, "model name")?.to_owned();
    let status = raw.get("status").filter(|value| !value.is_null());
    if status.is_some_and(|value| value.as_str() == Some("deprecated")) {
        return Ok(None);
    }
    let mut disabled_reason = status.map(|_| "catalog model has an unsupported status".to_owned());

    let (limits, limit_error) = normalize_limits(raw.get("limit"));
    let (input_modalities, output_modalities, modality_error) =
        normalize_modalities(raw.get("modalities"));
    let (tool_call, tool_error) = normalized_bool(raw, "tool_call");
    let (reasoning, reasoning_error) = normalized_bool(raw, "reasoning");
    let (structured_output, structured_error) = normalized_bool(raw, "structured_output");
    disabled_reason = disabled_reason
        .or(limit_error)
        .or(modality_error)
        .or(tool_error)
        .or(reasoning_error)
        .or(structured_error);

    Ok(Some(CatalogModel {
        id: model_id.to_owned(),
        name,
        limits,
        input_modalities,
        output_modalities,
        tool_call,
        reasoning,
        reasoning_controls: normalize_reasoning_controls(raw, reasoning),
        structured_output,
        disabled_reason,
    }))
}

/// Keeps only the advertised control shapes Smith can express: an on/off
/// switch and an ordered effort ladder. `budget_tokens` and unknown option
/// types grant nothing, and invalid entries are dropped rather than
/// disabling an otherwise valid model.
///
/// `scripts/generate-model-catalog.py` implements the same normalization;
/// the two must stay byte-identical for the seed reproducibility check.
fn normalize_reasoning_controls(
    raw: &serde_json::Map<String, Value>,
    reasoning: bool,
) -> Option<CatalogReasoningControls> {
    if !reasoning {
        return None;
    }
    let entries = raw.get("reasoning_options")?.as_array()?;
    let mut toggle = false;
    let mut efforts: Vec<String> = Vec::new();
    for entry in entries.iter().take(MAX_REASONING_OPTION_ENTRIES) {
        match entry.get("type").and_then(Value::as_str) {
            Some("toggle") => toggle = true,
            Some("effort") => {
                let values = entry.get("values").and_then(Value::as_array);
                for value in values.into_iter().flatten() {
                    if efforts.len() == MAX_REASONING_EFFORTS {
                        break;
                    }
                    let Some(text) = value.as_str() else { continue };
                    let lowered = text.to_ascii_lowercase();
                    if valid_effort_name(&lowered) && !efforts.contains(&lowered) {
                        efforts.push(lowered);
                    }
                }
            }
            _ => {}
        }
    }
    (toggle || !efforts.is_empty()).then_some(CatalogReasoningControls { toggle, efforts })
}

fn valid_effort_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REASONING_EFFORT_BYTES
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn normalize_limits(raw: Option<&Value>) -> (Option<CatalogLimits>, Option<String>) {
    let Some(raw) = raw.and_then(Value::as_object) else {
        return (
            None,
            Some("catalog model has no valid limit declaration".to_owned()),
        );
    };
    let context = positive_u32(raw.get("context"));
    let output = positive_u32(raw.get("output"));
    let input = match raw.get("input") {
        None | Some(Value::Null) => context,
        value => positive_u32(value),
    };
    let (Some(context), Some(input), Some(output)) = (context, input, output) else {
        return (
            None,
            Some("catalog model has a zero, missing, or out-of-range limit".to_owned()),
        );
    };
    if output > context {
        return (
            None,
            Some("catalog output limit exceeds its context window".to_owned()),
        );
    }
    if input > context {
        return (
            None,
            Some("catalog input limit exceeds its context window".to_owned()),
        );
    }
    (
        Some(CatalogLimits {
            context_tokens: context,
            max_input_tokens: input,
            max_output_tokens: output,
        }),
        None,
    )
}

fn normalize_modalities(
    raw: Option<&Value>,
) -> (Vec<CatalogModality>, Vec<CatalogModality>, Option<String>) {
    let Some(raw) = raw.and_then(Value::as_object) else {
        return (
            Vec::new(),
            Vec::new(),
            Some("catalog model has no valid modality declaration".to_owned()),
        );
    };
    let input = normalize_modality_list(raw.get("input"), "input");
    let output = normalize_modality_list(raw.get("output"), "output");
    match (input, output) {
        (Ok(input), Ok(output)) => (input, output, None),
        (Err(error), _) | (_, Err(error)) => (Vec::new(), Vec::new(), Some(error)),
    }
}

fn normalize_modality_list(
    raw: Option<&Value>,
    direction: &str,
) -> Result<Vec<CatalogModality>, String> {
    let Some(raw) = raw.and_then(Value::as_array) else {
        return Err(format!("catalog `{direction}` modalities are invalid"));
    };
    if raw.len() > MAX_MODALITIES {
        return Err(format!("catalog `{direction}` modalities are invalid"));
    }
    let mut modalities = BTreeSet::new();
    for modality in raw {
        let parsed = match modality.as_str() {
            Some("text") => CatalogModality::Text,
            Some("image") => CatalogModality::Image,
            Some("audio") => CatalogModality::Audio,
            Some("video") => CatalogModality::Video,
            Some("pdf" | "document") => CatalogModality::Document,
            _ => return Err(format!("catalog `{direction}` modality is unsupported")),
        };
        modalities.insert(parsed);
    }
    Ok(modalities.into_iter().collect())
}

fn normalized_bool(raw: &Map<String, Value>, field: &str) -> (bool, Option<String>) {
    match raw.get(field) {
        None => (false, None),
        Some(Value::Bool(value)) => (*value, None),
        Some(_) => (
            false,
            Some(format!("catalog field `{field}` is not boolean")),
        ),
    }
}

fn positive_u32(raw: Option<&Value>) -> Option<u32> {
    u32::try_from(raw?.as_u64()?)
        .ok()
        .filter(|value| *value > 0)
}

fn required_text<'a>(
    raw: &'a Map<String, Value>,
    field: &str,
    maximum: usize,
    label: &str,
) -> Result<&'a str, CatalogError> {
    let value = raw
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| CatalogError::InvalidDocument(format!("{label} must be bounded text")))?;
    validate_text(value, maximum, label)?;
    Ok(value)
}

fn validate_text(value: &str, maximum: usize, label: &str) -> Result<(), CatalogError> {
    invalid_if(
        value.is_empty()
            || value.len() > maximum
            || value.chars().any(|character| character.is_control()),
        &format!("{label} must be non-empty bounded text"),
    )
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn valid_revision(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_REVISION_BYTES && !value.chars().any(char::is_control)
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn sorted_unique<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn invalid_if(condition: bool, message: &str) -> Result<(), CatalogError> {
    if condition {
        Err(CatalogError::InvalidDocument(message.to_owned()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::clock::Timestamp;
    use agent_runtime_core::provider::ModelId;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct FixedClock(u64);

    impl Clock for FixedClock {
        fn now(&self) -> Timestamp {
            Timestamp(self.0)
        }
    }

    #[derive(Debug)]
    struct ScriptedFetcher {
        response: Mutex<Result<CatalogFetchResponse, String>>,
        calls: AtomicUsize,
    }

    impl ScriptedFetcher {
        fn returning(response: CatalogFetchResponse) -> Self {
            Self {
                response: Mutex::new(Ok(response)),
                calls: AtomicUsize::new(0),
            }
        }

        fn failing(message: &str) -> Self {
            Self {
                response: Mutex::new(Err(message.to_owned())),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl CatalogFetcher for ScriptedFetcher {
        async fn fetch(
            &self,
            _if_none_match: Option<&str>,
        ) -> Result<CatalogFetchResponse, CatalogError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.response
                .lock()
                .unwrap()
                .clone()
                .map_err(CatalogError::Fetch)
        }
    }

    fn remote_fixture() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "unrelated": {"id": "unrelated", "models": {}},
            "openai": {
                "id": "openai",
                "name": "OpenAI",
                "models": {
                    "gpt-fixture": {
                        "id": "gpt-fixture",
                        "name": "GPT Fixture",
                        "tool_call": true,
                        "reasoning": true,
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "limit": {"context": 400000, "output": 128000}
                    }
                }
            },
            "openrouter": {
                "id": "openrouter",
                "name": "OpenRouter",
                "api": "https://must-not-be-imported.example",
                "env": ["SECRET"],
                "models": {
                    "vendor/nested": {
                        "id": "vendor/nested",
                        "name": "Nested",
                        "tool_call": true,
                        "reasoning": true,
                        "structured_output": true,
                        "modalities": {"input": ["image", "text"], "output": ["text"]},
                        "limit": {"context": 128000, "output": 16000}
                    },
                    "separate-input": {
                        "id": "separate-input",
                        "name": "Separate Input",
                        "tool_call": true,
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "limit": {"context": 128000, "input": 64000, "output": 8000}
                    },
                    "no-tools": {
                        "id": "no-tools",
                        "name": "No Tools",
                        "tool_call": false,
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "limit": {"context": 32000, "output": 4000}
                    },
                    "no-text": {
                        "id": "no-text",
                        "name": "No Text",
                        "tool_call": true,
                        "modalities": {"input": ["text"], "output": ["image"]},
                        "limit": {"context": 32000, "output": 4000}
                    },
                    "invalid-output": {
                        "id": "invalid-output",
                        "name": "Invalid Output",
                        "tool_call": true,
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "limit": {"context": 1000, "output": 2000}
                    },
                    "unknown-modality": {
                        "id": "unknown-modality",
                        "name": "Unknown Modality",
                        "tool_call": true,
                        "modalities": {"input": ["thought"], "output": ["text"]},
                        "limit": {"context": 32000, "output": 4000}
                    },
                    "old": {
                        "id": "old",
                        "name": "Old",
                        "status": "deprecated",
                        "tool_call": true,
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "limit": {"context": 32000, "output": 4000}
                    }
                }
            },
            "zai-coding-plan": {
                "id": "zai-coding-plan",
                "name": "Z.AI Coding Plan",
                "models": {
                    "glm-fixture": {
                        "id": "glm-fixture",
                        "name": "GLM Fixture",
                        "tool_call": true,
                        "reasoning": true,
                        "modalities": {"input": ["pdf", "text"], "output": ["text"]},
                        "limit": {"context": 200000, "output": 64000}
                    }
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn embedded_seed_is_strictly_valid_and_contains_all_supported_catalogs() {
        let snapshot = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();

        assert_eq!(
            snapshot
                .provider(OPENAI_CATALOG_PROVIDER)
                .unwrap()
                .models
                .len(),
            37
        );
        assert_eq!(
            snapshot
                .provider(OPENROUTER_CATALOG_PROVIDER)
                .unwrap()
                .models
                .len(),
            335
        );
        assert_eq!(
            snapshot
                .provider(ZAI_CODING_PLAN_CATALOG_PROVIDER)
                .unwrap()
                .models
                .len(),
            4
        );
    }

    #[test]
    fn runtime_source_preserves_local_alias_and_nested_model_identity() {
        let snapshot = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();
        let source = runtime_catalog_source(
            &snapshot,
            "coding",
            "openai-compatible",
            Some("https://api.z.ai/api/coding/paas/v4/"),
        )
        .unwrap();

        let record = source
            .lookup("coding", &ModelId::new("glm-4.7"))
            .expect("catalog record under the local provider alias");
        assert_eq!(record.context_tokens, Some(204_800));
        assert_eq!(record.max_input_tokens, Some(204_800));
        assert_eq!(record.max_output_tokens, Some(131_072));
        assert_eq!(
            record.revision.as_deref(),
            Some(snapshot.source_revision.as_str())
        );
        assert!(
            source
                .lookup("zai-coding-plan", &ModelId::new("glm-4.7"))
                .is_none()
        );
    }

    #[test]
    fn remote_normalization_covers_limits_modalities_status_and_capabilities() {
        let snapshot = normalize_remote(&remote_fixture(), 5_000, Some("etag-r2")).unwrap();
        let openrouter = snapshot.provider("openrouter").unwrap();

        assert_eq!(openrouter.models.len(), 6, "deprecated model is omitted");
        let nested = openrouter.models.get("vendor/nested").unwrap();
        assert_eq!(
            nested.limits,
            Some(CatalogLimits {
                context_tokens: 128_000,
                max_input_tokens: 128_000,
                max_output_tokens: 16_000,
            })
        );
        assert_eq!(
            nested.input_modalities,
            [CatalogModality::Text, CatalogModality::Image]
        );
        assert!(nested.tool_call);
        assert!(nested.reasoning);
        assert!(nested.structured_output);
        assert_eq!(
            openrouter
                .models
                .get("separate-input")
                .unwrap()
                .limits
                .unwrap()
                .max_input_tokens,
            64_000
        );
        assert!(
            openrouter
                .models
                .get("invalid-output")
                .unwrap()
                .disabled_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("output limit"))
        );
        assert!(
            openrouter
                .models
                .get("unknown-modality")
                .unwrap()
                .disabled_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("modality"))
        );
        assert!(!openrouter.models.contains_key("old"));
        assert_eq!(snapshot.source_revision, "etag-r2");
        assert_eq!(snapshot.retrieved_at_ms, 5_000);
    }

    #[test]
    fn malformed_identifiers_and_oversized_responses_are_rejected() {
        let mut malformed: Value = serde_json::from_slice(&remote_fixture()).unwrap();
        malformed["openrouter"]["models"]["vendor/nested"]["id"] = json!("other");
        assert!(normalize_remote(&serde_json::to_vec(&malformed).unwrap(), 1, None).is_err());
        assert!(normalize_remote(&vec![b' '; MAX_REMOTE_CATALOG_BYTES + 1], 1, None).is_err());
        assert!(normalize_remote(b"{", 1, None).is_err());
    }

    #[tokio::test]
    async fn corrupt_truncated_oversized_and_wrong_origin_caches_fall_back_to_seed() {
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("models-dev-v1.json");
        let fetcher = Arc::new(ScriptedFetcher::returning(
            CatalogFetchResponse::NotModified,
        ));
        let loader = CatalogLoader::new(
            cache_path.clone(),
            Arc::<str>::from(EMBEDDED_MODELS_DEV_SEED),
            fetcher,
            Arc::new(FixedClock(1785298822000)),
        );

        let mut wrong_origin = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();
        wrong_origin.source_url = "https://example.test/api.json".to_owned();
        let mut wrong_digest = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();
        wrong_digest.content_digest = format!("sha256:{}", "0".repeat(64));
        for bytes in [
            b"{".to_vec(),
            serde_json::to_vec(&wrong_origin).unwrap(),
            serde_json::to_vec(&wrong_digest).unwrap(),
            vec![b'x'; MAX_NORMALIZED_CATALOG_BYTES + 1],
        ] {
            std::fs::write(&cache_path, bytes).unwrap();
            let prepared = loader.prepare(false).await.unwrap();
            assert_eq!(prepared.origin, CatalogLoadOrigin::Embedded);
            assert_eq!(prepared.snapshot.providers.len(), 3);
        }
    }

    #[tokio::test]
    async fn valid_last_good_cache_wins_without_fetching() {
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("models-dev-v1.json");
        let mut cached = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();
        cached.source_revision = "cache-r2".to_owned();
        cached.retrieved_at_ms = 20_000;
        std::fs::write(&cache_path, serde_json::to_vec(&cached).unwrap()).unwrap();
        let fetcher = Arc::new(ScriptedFetcher::returning(
            CatalogFetchResponse::NotModified,
        ));
        let loader = CatalogLoader::new(
            cache_path,
            Arc::<str>::from(EMBEDDED_MODELS_DEV_SEED),
            fetcher.clone(),
            Arc::new(FixedClock(20_001)),
        );

        let prepared = loader.prepare(true).await.unwrap();
        assert_eq!(prepared.origin, CatalogLoadOrigin::LastGoodCache);
        assert_eq!(prepared.snapshot.source_revision, "cache-r2");
        assert!(!prepared.refresh_scheduled);
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_snapshot_schedules_one_non_blocking_refresh() {
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("models-dev-v1.json");
        let snapshot = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();
        let fetcher = Arc::new(ScriptedFetcher::returning(
            CatalogFetchResponse::NotModified,
        ));
        let loader = CatalogLoader::new(
            cache_path.clone(),
            Arc::<str>::from(EMBEDDED_MODELS_DEV_SEED),
            fetcher.clone(),
            Arc::new(FixedClock(snapshot.retrieved_at_ms + 2)),
        )
        .with_max_age_ms(1);

        let prepared = loader.prepare(true).await.unwrap();
        assert_eq!(prepared.origin, CatalogLoadOrigin::Embedded);
        assert!(prepared.refresh_scheduled);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while fetcher.calls.load(Ordering::SeqCst) == 0
                || tokio::fs::metadata(&cache_path).await.is_err()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("background refresh");
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refresh_atomically_publishes_only_for_a_later_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("models-dev-v1.json");
        let current = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();
        let old_revision = current.source_revision.clone();
        let fetcher = Arc::new(ScriptedFetcher::returning(CatalogFetchResponse::Fresh {
            body: remote_fixture(),
            revision: Some("etag-r3".to_owned()),
            final_url: MODELS_DEV_SOURCE_URL.to_owned(),
        }));
        let loader = CatalogLoader::new(
            cache_path.clone(),
            Arc::<str>::from(EMBEDDED_MODELS_DEV_SEED),
            fetcher,
            Arc::new(FixedClock(30_000)),
        );

        loader.refresh(&current).await.unwrap();
        assert_eq!(
            current.source_revision, old_revision,
            "active snapshot stays frozen"
        );
        let published = parse_snapshot(&std::fs::read(&cache_path).unwrap()).unwrap();
        assert_eq!(published.source_revision, "etag-r3");
        assert_eq!(published.retrieved_at_ms, 30_000);
        assert!(published.model("openrouter", "vendor/nested").is_some());
        assert!(std::fs::read_dir(directory.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    #[tokio::test]
    async fn refresh_failures_and_wrong_final_origin_leave_last_good_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("models-dev-v1.json");
        let current = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();
        let original = serde_json::to_vec(&current).unwrap();
        std::fs::write(&cache_path, &original).unwrap();

        let failing = CatalogLoader::new(
            cache_path.clone(),
            Arc::<str>::from(EMBEDDED_MODELS_DEV_SEED),
            Arc::new(ScriptedFetcher::failing("timeout")),
            Arc::new(FixedClock(40_000)),
        );
        assert!(failing.refresh(&current).await.is_err());
        assert_eq!(std::fs::read(&cache_path).unwrap(), original);

        let wrong_origin = CatalogLoader::new(
            cache_path.clone(),
            Arc::<str>::from(EMBEDDED_MODELS_DEV_SEED),
            Arc::new(ScriptedFetcher::returning(CatalogFetchResponse::Fresh {
                body: remote_fixture(),
                revision: None,
                final_url: "https://example.test/api.json".to_owned(),
            })),
            Arc::new(FixedClock(40_000)),
        );
        assert!(wrong_origin.refresh(&current).await.is_err());
        assert_eq!(std::fs::read(&cache_path).unwrap(), original);
    }

    #[tokio::test]
    async fn not_modified_refresh_advances_freshness_without_changing_revision() {
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("models-dev-v1.json");
        let current = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();
        let loader = CatalogLoader::new(
            cache_path.clone(),
            Arc::<str>::from(EMBEDDED_MODELS_DEV_SEED),
            Arc::new(ScriptedFetcher::returning(
                CatalogFetchResponse::NotModified,
            )),
            Arc::new(FixedClock(50_000)),
        );

        loader.refresh(&current).await.unwrap();
        let published = parse_snapshot(&std::fs::read(cache_path).unwrap()).unwrap();
        assert_eq!(published.source_revision, current.source_revision);
        assert_eq!(published.retrieved_at_ms, 50_000);
    }

    #[test]
    fn concurrent_refresh_guard_deduplicates_one_cache_path() {
        let path = PathBuf::from("/tmp/smith-catalog-dedup-test");
        let first = RefreshGuard::acquire(path.clone()).unwrap();
        assert!(RefreshGuard::acquire(path.clone()).is_none());
        drop(first);
        assert!(RefreshGuard::acquire(path).is_some());
    }
}
