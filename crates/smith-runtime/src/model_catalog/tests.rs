use super::parsing::{micro_usd_per_million, normalize_model};
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
                    "limit": {"context": 400000, "output": 128000},
                    "cost": {"input": 5, "output": 30, "cache_read": 1.25, "cache_write": 6.25}
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
                    "limit": {"context": 128000, "input": 64000, "output": 8000},
                    "cost": {"input": 2.5}
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
        "xai": {
            "id": "xai",
            "name": "xAI",
            "models": {
                "grok-fixture": {
                    "id": "grok-fixture",
                    "name": "Grok Fixture",
                    "tool_call": true,
                    "reasoning": true,
                    "modalities": {"input": ["text", "image"], "output": ["text"]},
                    "limit": {"context": 500000, "output": 64000}
                }
            }
        },
        "google": {
            "id": "google",
            "name": "Google",
            "models": {
                "gemini-fixture": {
                    "id": "gemini-fixture",
                    "name": "Gemini Fixture",
                    "tool_call": true,
                    "reasoning": true,
                    "reasoning_options": [{"type": "effort", "values": ["minimal", "low", "medium", "high"]}],
                    "structured_output": true,
                    "modalities": {"input": ["text", "image", "video", "audio", "pdf"], "output": ["text"]},
                    "limit": {"context": 1000000, "output": 65536}
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

    // Every supported catalog is present and non-empty. Exact per-provider
    // counts are deliberately not asserted: the seed is regenerated when
    // Models.dev is refreshed, and pinning a literal makes every refresh
    // look like a regression.
    for provider in EXPECTED_PROVIDERS {
        let entry = snapshot
            .provider(provider)
            .unwrap_or_else(|| panic!("`{provider}` is missing from the embedded seed"));
        assert!(
            !entry.models.is_empty(),
            "`{provider}` contributed no models"
        );
    }

    let openai = snapshot.provider("openai").expect("OpenAI catalog");
    for model in ["gpt-6-astra", "gpt-6.1-sol", "gpt-6-sol", "gpt-6-luna"] {
        assert!(
            openai.models.contains_key(model),
            "embedded OpenAI catalog should contain {model}"
        );
    }
    assert!(
        snapshot
            .provider("openrouter")
            .expect("OpenRouter catalog")
            .models
            .contains_key("anthropic/claude-opus-5.5"),
        "embedded catalog should contain Claude Opus 5.5"
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
fn native_gemini_catalog_binding_uses_the_fixed_endpoint_without_user_url() {
    let snapshot = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();
    let source = runtime_catalog_source(&snapshot, "google", "gemini-interactions", None).unwrap();
    let record = source
        .lookup("google", &ModelId::new("gemini-3.6-flash"))
        .expect("the embedded Gemini model");
    assert_eq!(record.max_output_tokens, Some(65_536));
    assert_eq!(
        record
            .capabilities
            .as_ref()
            .map(|capabilities| capabilities.reasoning),
        Some(ReasoningSupport::Fixed)
    );
}

#[test]
fn remote_normalization_covers_limits_modalities_status_and_capabilities() {
    let snapshot = normalize_remote(&remote_fixture(), 5_000, Some("etag-r2")).unwrap();
    let openai = snapshot.provider("openai").unwrap();
    let openrouter = snapshot.provider("openrouter").unwrap();
    let google = snapshot.provider("google").unwrap();

    // A complete published price is normalized on every counter, through
    // the full remote-normalization-and-validation pipeline, not just
    // `normalize_model` in isolation: `validate_snapshot` must not treat
    // a priced model any differently than an unpriced one.
    assert_eq!(
        openai.models.get("gpt-fixture").unwrap().cost,
        Some(CatalogModelCost {
            input: Some(5_000_000),
            output: Some(30_000_000),
            cache_read: Some(1_250_000),
            cache_write: Some(6_250_000),
        })
    );
    // A partial price keeps only the published counter; nothing is
    // inferred for the others, and the model stays selectable.
    let separate_input = openrouter.models.get("separate-input").unwrap();
    assert_eq!(
        separate_input.cost,
        Some(CatalogModelCost {
            input: Some(2_500_000),
            output: None,
            cache_read: None,
            cache_write: None,
        })
    );
    assert!(separate_input.disabled_reason.is_none());
    // A model that publishes no `cost` at all is selectable exactly as
    // it was before prices existed.
    assert!(
        openrouter
            .models
            .get("vendor/nested")
            .unwrap()
            .cost
            .is_none()
    );

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
    let gemini = google.models.get("gemini-fixture").unwrap();
    assert_eq!(
        gemini
            .reasoning_controls
            .as_ref()
            .map(|controls| controls.efforts.clone()),
        Some(vec![
            "minimal".to_owned(),
            "low".to_owned(),
            "medium".to_owned(),
            "high".to_owned(),
        ])
    );
}

#[test]
fn malformed_identifiers_and_oversized_responses_are_rejected() {
    let mut malformed: Value = serde_json::from_slice(&remote_fixture()).unwrap();
    malformed["openrouter"]["models"]["vendor/nested"]["id"] = json!("other");
    assert!(normalize_remote(&serde_json::to_vec(&malformed).unwrap(), 1, None).is_err());
    assert!(normalize_remote(&vec![b' '; MAX_REMOTE_CATALOG_BYTES + 1], 1, None).is_err());
    assert!(normalize_remote(b"{", 1, None).is_err());
}

fn priced_model_json(cost: Value) -> Value {
    json!({
        "id": "priced",
        "name": "Priced",
        "tool_call": true,
        "modalities": {"input": ["text"], "output": ["text"]},
        "limit": {"context": 128000, "output": 8000},
        "cost": cost,
    })
}

#[test]
fn cost_drops_negative_out_of_range_and_non_numeric_counters_individually() {
    // `serde_json::Number` cannot represent a literal non-finite `f64`
    // at all — parsing an out-of-`f64`-range exponent like `1e400`
    // fails the document, and `Number::from_f64` refuses `NaN`/infinite
    // inputs outright — so `micro_usd_per_million`'s `is_finite` guard
    // is exercised directly below rather than through a `Value`. Here,
    // an absurdly large but syntactically ordinary finite price stands
    // in for "unusable" by overflowing the micro-USD `u64` bound.
    let raw = priced_model_json(json!({
        "input": -1.0,
        "output": "expensive",
        "cache_read": 1e30,
        "cache_write": 4.5
    }));
    let model = normalize_model("priced", &raw).unwrap().unwrap();
    assert_eq!(
        model.cost,
        Some(CatalogModelCost {
            input: None,
            output: None,
            cache_read: None,
            cache_write: Some(4_500_000),
        }),
        "only the individually valid counter survives"
    );
    assert!(
        model.disabled_reason.is_none(),
        "a price, however invalid, must never disable the model"
    );
}

#[test]
fn micro_usd_per_million_rejects_absent_negative_non_numeric_and_overflow() {
    assert_eq!(micro_usd_per_million(None), None, "an absent counter");
    assert_eq!(
        micro_usd_per_million(Some(&Value::Null)),
        None,
        "a null counter"
    );
    assert_eq!(
        micro_usd_per_million(Some(&json!("free"))),
        None,
        "a non-numeric counter"
    );
    assert_eq!(
        micro_usd_per_million(Some(&json!(-0.0001))),
        None,
        "a negative counter"
    );
    assert_eq!(
        micro_usd_per_million(Some(&json!(1e30))),
        None,
        "a finite but unrepresentable counter overflows the micro-USD u64 bound"
    );
    assert_eq!(
        micro_usd_per_million(Some(&json!(0))),
        Some(0),
        "zero is a valid, free price"
    );
    assert_eq!(micro_usd_per_million(Some(&json!(2.5))), Some(2_500_000));

    // `serde_json::Value` cannot represent a literal NaN or infinite
    // `f64` at all: `Value::from(f64::NAN)` and `Value::from(f64::INFINITY)`
    // both collapse to `Value::Null` rather than a `Number`, so the
    // `is_finite` guard in `micro_usd_per_million` is unreachable
    // through any real `Value` — it exists as documented defense in
    // depth against a caller constructing a `Number` some other way.
    assert_eq!(Value::from(f64::NAN), Value::Null);
    assert_eq!(Value::from(f64::INFINITY), Value::Null);
}

#[test]
fn cost_block_with_every_counter_invalid_normalizes_like_no_cost_at_all() {
    let raw = priced_model_json(json!({"input": -1, "output": "nope"}));
    let with_bad_cost = normalize_model("priced", &raw).unwrap().unwrap();

    let mut without_cost = raw.clone();
    without_cost
        .as_object_mut()
        .unwrap()
        .remove("cost")
        .unwrap();
    let without_cost = normalize_model("priced", &without_cost).unwrap().unwrap();

    assert!(with_bad_cost.cost.is_none());
    assert_eq!(with_bad_cost.cost, without_cost.cost);
    assert_eq!(with_bad_cost.disabled_reason, without_cost.disabled_reason);
    assert!(without_cost.disabled_reason.is_none());
}

#[test]
fn absent_cost_leaves_a_model_selectable_with_no_diagnostic() {
    let mut raw = priced_model_json(json!({}));
    raw.as_object_mut().unwrap().remove("cost").unwrap();
    let model = normalize_model("priced", &raw).unwrap().unwrap();
    assert!(model.cost.is_none());
    assert!(
        model.disabled_reason.is_none(),
        "the absence of a price is not a validation diagnostic"
    );
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
        assert_eq!(prepared.snapshot.providers.len(), EXPECTED_PROVIDERS.len());
    }
}

#[tokio::test]
async fn a_second_process_holding_the_lock_skips_the_fetch() {
    // Several agents on one machine notice the same stale snapshot at the
    // same moment. Only one of them should reach the network.
    let directory = tempfile::tempdir().unwrap();
    let cache_path = directory.path().join("models-dev-v1.json");
    let counting = Arc::new(ScriptedFetcher::returning(
        CatalogFetchResponse::NotModified,
    ));
    let loader = CatalogLoader::new(
        cache_path.clone(),
        Arc::<str>::from(EMBEDDED_MODELS_DEV_SEED),
        counting.clone(),
        Arc::new(SystemClock),
    );
    let seed = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();

    // Stand in for the other process by holding the same advisory lock.
    let held = CacheLock::try_acquire(&loader.lock_path())
        .await
        .unwrap()
        .expect("an uncontended lock");

    loader.refresh(&seed).await.unwrap();
    assert_eq!(
        counting.calls.load(Ordering::SeqCst),
        0,
        "a refresh ran while another process held the lock"
    );

    drop(held);
    loader.refresh(&seed).await.unwrap();
    assert_eq!(
        counting.calls.load(Ordering::SeqCst),
        1,
        "the refresh did not resume once the lock was released"
    );
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
async fn revision_one_cache_is_rejected_as_stale_and_falls_back_to_the_seed() {
    // `CatalogModelCost` was added at schema revision 2. A cache file
    // written at revision 1 predates `cost` entirely, so loading it
    // unchanged would silently and permanently under-report every model
    // as unpriced. Nothing new needs to reject it: the existing
    // `schema_revision != CATALOG_SCHEMA_REVISION` check in
    // `validate_snapshot` already fails to parse it, which is exactly
    // the same path an unrelated stale/corrupt cache takes.
    assert_eq!(CATALOG_SCHEMA_REVISION, 2, "the revision bump itself");

    let directory = tempfile::tempdir().unwrap();
    let cache_path = directory.path().join("models-dev-v1.json");
    let seed = parse_snapshot(EMBEDDED_MODELS_DEV_SEED.as_bytes()).unwrap();
    let mut prior_revision = seed.clone();
    prior_revision.schema_revision = 1;
    std::fs::write(&cache_path, serde_json::to_vec(&prior_revision).unwrap()).unwrap();

    let fetcher = Arc::new(ScriptedFetcher::returning(
        CatalogFetchResponse::NotModified,
    ));
    let loader = CatalogLoader::new(
        cache_path.clone(),
        Arc::<str>::from(EMBEDDED_MODELS_DEV_SEED),
        fetcher.clone(),
        Arc::new(FixedClock(seed.retrieved_at_ms + 2)),
    )
    .with_max_age_ms(1);

    let prepared = loader.prepare(true).await.unwrap();
    assert_eq!(
        prepared.origin,
        CatalogLoadOrigin::Embedded,
        "a revision-1 cache is rejected as stale, so the embedded seed loads instead"
    );
    assert_eq!(prepared.snapshot.schema_revision, CATALOG_SCHEMA_REVISION);
    assert!(
        prepared.refresh_scheduled,
        "falling back still schedules a refresh, as for any stale snapshot"
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while fetcher.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("background refresh");
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
