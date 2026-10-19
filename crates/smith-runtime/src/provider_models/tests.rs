use super::*;
use serde_json::json;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

struct Server {
    base_url: String,
    requests: mpsc::UnboundedReceiver<String>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(responses: Vec<Vec<u8>>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sender, requests) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let byte = socket.read_u8().await.unwrap();
                request.push(byte);
                assert!(request.len() < 32 * 1024);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            sender.send(String::from_utf8(request).unwrap()).unwrap();
            let _ = socket.write_all(&response).await;
        }
    });
    Server {
        base_url,
        requests,
        task,
    }
}

fn response(status: u16, headers: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn listing(id: &str) -> Value {
    json!({"data":[{
        "id":id,"name":"Gateway Model",
        "limits":{"context":1048576,"input":1048576,"output":256000},
        "modalities":{"input":["text","image"],"output":["text"]},
        "capabilities":{"tool_call":true,"reasoning":true},
        "reasoning_options":[{"type":"effort","values":["low","high","unknown"]}],
        "secret_extension":{"api_key":"must-not-persist"}
    }]})
}

fn target(base_url: &str) -> ProviderModelTarget {
    ProviderModelTarget {
        name: "xiaoge".to_owned(),
        kind: KIND_OPENAI_COMPATIBLE.to_owned(),
        base_url: base_url.to_owned(),
        credential_scope: "account-1-revision-2".to_owned(),
        bearer: Some(Secret::new("test-provider-key")),
        headers: BTreeMap::from([(
            "x-provider-extra".to_owned(),
            "secret-header-value".to_owned(),
        )]),
    }
}

const DAILY: Duration = Duration::from_secs(24 * 3600);
const START: u64 = 1_000_000;

#[tokio::test]
async fn authenticated_nested_listing_is_private_allowlisted_and_offline_usable() {
    let mut server = server(vec![response(
        200,
        "ETag: \"one\"\r\n",
        &listing("selected-large").to_string(),
    )])
    .await;
    let user_dir = tempfile::tempdir().unwrap();
    let target = target(&server.base_url);
    let outcome = refresh_at(user_dir.path(), &target, DAILY, false, || START)
        .await
        .unwrap();
    assert_eq!(outcome.status, SyncStatus::Updated);
    let catalog = outcome.catalog.unwrap();
    let model = &catalog.catalog.models["selected-large"];
    assert_eq!(model.limits.unwrap().max_output_tokens, 256000);
    assert!(model.tool_call && model.reasoning);
    assert_eq!(model.input_modalities.len(), 2);
    assert_eq!(
        model.reasoning_controls.as_ref().unwrap().efforts,
        vec!["low", "high"]
    );
    assert!(model.cost.is_none());
    assert!(model.disabled_reason.is_none());
    let request = server.requests.recv().await.unwrap().to_ascii_lowercase();
    assert!(request.starts_with("get /v1/models "));
    assert!(request.contains("authorization: bearer test-provider-key\r\n"));
    assert!(request.contains("x-provider-extra: secret-header-value\r\n"));
    assert_eq!(
        load_cached(user_dir.path(), &target).unwrap().revision,
        catalog.revision
    );
    let path = user_dir.path().join("cache/provider-models-v1");
    for entry in std::fs::read_dir(&path).unwrap() {
        let entry = entry.unwrap();
        let bytes = std::fs::read(entry.path()).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        for secret in [
            "must-not-persist",
            "test-provider-key",
            "secret-header-value",
            "account-1-revision-2",
        ] {
            assert!(!text.contains(secret));
            assert!(!entry.file_name().to_string_lossy().contains(secret));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                entry.metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    // Server has served its only request and closed. A failure preserves the
    // exact revision/time, with no need for an inference endpoint.
    let offline = refresh_at(user_dir.path(), &target, DAILY, true, || START + 100)
        .await
        .unwrap();
    assert_eq!(offline.status, SyncStatus::Failed);
    assert_eq!(offline.catalog.unwrap().retrieved_at_ms, START);
}

#[tokio::test]
async fn freshness_restart_daily_boundary_and_force_use_successful_validation_time() {
    let mut server = server(vec![
        response(
            200,
            "ETag: \"one\"\r\n",
            &listing("selected-large").to_string(),
        ),
        response(304, "", ""),
        response(200, "", &listing("replacement").to_string()),
    ])
    .await;
    let user_dir = tempfile::tempdir().unwrap();
    let target = target(&server.base_url);
    let first = refresh_at(user_dir.path(), &target, DAILY, false, || START)
        .await
        .unwrap();
    let day = DAILY.as_millis() as u64;
    assert_eq!(
        refresh_at(user_dir.path(), &target, DAILY, false, || START + day - 1)
            .await
            .unwrap()
            .status,
        SyncStatus::Fresh
    );
    let unchanged = refresh_at(user_dir.path(), &target, DAILY, false, || START + day)
        .await
        .unwrap();
    assert_eq!(unchanged.status, SyncStatus::NotModified);
    assert_eq!(
        unchanged.catalog.as_ref().unwrap().revision,
        first.catalog.unwrap().revision
    );
    assert_eq!(unchanged.catalog.unwrap().retrieved_at_ms, START + day);
    let forced = refresh_at(user_dir.path(), &target, DAILY, true, || START + day + 1)
        .await
        .unwrap();
    assert_eq!(forced.status, SyncStatus::Updated);
    assert!(
        forced
            .catalog
            .as_ref()
            .unwrap()
            .catalog
            .models
            .contains_key("replacement")
    );
    assert!(
        !forced
            .catalog
            .unwrap()
            .catalog
            .models
            .contains_key("selected-large")
    );
    server.requests.recv().await.unwrap();
    let conditional = server.requests.recv().await.unwrap().to_ascii_lowercase();
    assert!(conditional.contains("if-none-match: \"one\""));
    assert!(server.requests.recv().await.is_some());
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn concurrent_refreshers_recheck_freshness_under_the_file_lock() {
    let server = server(vec![response(
        200,
        "",
        &listing("selected-large").to_string(),
    )])
    .await;
    let user_dir = tempfile::tempdir().unwrap();
    let target = Arc::new(target(&server.base_url));
    let (left, right) = tokio::join!(
        refresh_at(user_dir.path(), &target, DAILY, false, || START),
        refresh_at(user_dir.path(), &target, DAILY, false, || START),
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert!([left.status, right.status].contains(&SyncStatus::Updated));
    assert!([left.status, right.status].contains(&SyncStatus::Fresh));
    assert_eq!(
        left.catalog.unwrap().revision,
        right.catalog.unwrap().revision
    );
}

#[tokio::test]
async fn failures_keep_the_last_good_revision_and_success_time() {
    let server = server(vec![
        response(200, "", &listing("selected-large").to_string()),
        response(401, "", "bad key test-provider-key"),
        response(200, "", "{\"data\":not-json}"),
        response(
            200,
            "",
            &json!({"data":[{"id":"duplicate"},{"id":"duplicate"}]}).to_string(),
        ),
    ])
    .await;
    let user_dir = tempfile::tempdir().unwrap();
    let target = target(&server.base_url);
    let first = refresh_at(user_dir.path(), &target, DAILY, false, || START)
        .await
        .unwrap()
        .catalog
        .unwrap();
    for _ in 0..3 {
        let failed = refresh_at(user_dir.path(), &target, DAILY, true, || START + 100)
            .await
            .unwrap();
        assert_eq!(failed.status, SyncStatus::Failed);
        let retained = failed.catalog.unwrap();
        assert_eq!(retained.revision, first.revision);
        assert_eq!(retained.retrieved_at_ms, START);
        assert!(!failed.detail.unwrap().contains("test-provider-key"));
    }
    assert_eq!(
        load_cached(user_dir.path(), &target)
            .unwrap()
            .retrieved_at_ms,
        START
    );
}

#[tokio::test]
async fn redirect_is_not_followed_and_never_receives_credentials() {
    let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redirect = format!("Location: http://{}/stolen\r\n", sink.local_addr().unwrap());
    let server = server(vec![response(302, &redirect, "")]).await;
    let user_dir = tempfile::tempdir().unwrap();
    let target = target(&server.base_url);
    let outcome = refresh_at(user_dir.path(), &target, DAILY, false, || START)
        .await
        .unwrap();
    assert_eq!(outcome.status, SyncStatus::Failed);
    assert!(outcome.catalog.is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(30), sink.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn both_content_length_and_streamed_chunk_bounds_are_enforced() {
    let large = "x".repeat(MAX_LISTING_BYTES + 1);
    let chunked=format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{large}\r\n0\r\n\r\n",large.len()).into_bytes();
    let server = server(vec![response(200, "", &large), chunked]).await;
    let user_dir = tempfile::tempdir().unwrap();
    let target = target(&server.base_url);
    for _ in 0..2 {
        let outcome = refresh_at(user_dir.path(), &target, DAILY, true, || START)
            .await
            .unwrap();
        assert_eq!(outcome.status, SyncStatus::Failed);
        assert!(outcome.detail.unwrap().contains("size bound"));
    }
}

#[tokio::test]
async fn malformed_counts_id_lengths_and_endpoint_options_are_refused() {
    let too_many =
        json!({"data":(0..=MAX_MODELS).map(|id| json!({"id":id.to_string()})).collect::<Vec<_>>()});
    assert!(
        normalizing::normalize_listing(&too_many, "xiaoge")
            .unwrap_err()
            .contains("count bound")
    );
    for listing in [
        json!({"data":[{"id":"x".repeat(513)}]}),
        json!({"data":[{"id":"x","name":"n".repeat(1025)}]}),
        json!({"data":[{"id":"bad\n"}]}),
        json!({"models":[]}),
    ] {
        assert!(normalizing::normalize_listing(&listing, "xiaoge").is_err());
    }
    for endpoint in [
        "https://secret@example.test/v1",
        "https://example.test/v1?key=secret",
        "file:///secret",
        "https://example.test/v1#secret",
    ] {
        assert!(Binding::new(&target(endpoint)).is_err());
    }
    assert_eq!(
        normalized_endpoint("https://EXAMPLE.test/v1/").unwrap(),
        "https://example.test/v1"
    );
}

#[tokio::test]
async fn binding_isolates_account_rotation_adapter_endpoint_and_provider_name() {
    let server = server(vec![response(
        200,
        "",
        &listing("selected-large").to_string(),
    )])
    .await;
    let user_dir = tempfile::tempdir().unwrap();
    let target = target(&server.base_url);
    refresh_at(user_dir.path(), &target, DAILY, false, || START)
        .await
        .unwrap();
    for other in [
        ProviderModelTarget {
            credential_scope: "rotated".to_owned(),
            ..target.clone()
        },
        ProviderModelTarget {
            kind: "openai_responses".to_owned(),
            ..target.clone()
        },
        ProviderModelTarget {
            base_url: format!("{}/other", target.base_url),
            ..target.clone()
        },
        ProviderModelTarget {
            name: "different".to_owned(),
            ..target.clone()
        },
    ] {
        assert!(load_cached(user_dir.path(), &other).is_none());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn cache_schema_content_private_modes_and_symlinks_are_validated() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let server = server(vec![response(
        200,
        "",
        &listing("selected-large").to_string(),
    )])
    .await;
    let user_dir = tempfile::tempdir().unwrap();
    let target = target(&server.base_url);
    refresh_at(user_dir.path(), &target, DAILY, false, || START)
        .await
        .unwrap();
    let binding = Binding::new(&target).unwrap();
    let path = user_dir
        .path()
        .join(format!("cache/provider-models-v1/{}.json", binding.digest));
    let original = std::fs::read(&path).unwrap();
    let mut record: Value = serde_json::from_slice(&original).unwrap();
    record["schema_revision"] = json!(99);
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(load_cached(user_dir.path(), &target).is_none());
    std::fs::write(&path, &original).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_cached(user_dir.path(), &target).is_none());
    std::fs::remove_file(&path).unwrap();
    let outside = user_dir.path().join("outside");
    std::fs::write(&outside, &original).unwrap();
    symlink(&outside, &path).unwrap();
    assert!(load_cached(user_dir.path(), &target).is_none());
    let second = tempfile::tempdir().unwrap();
    symlink(user_dir.path().join("cache"), second.path().join("cache")).unwrap();
    assert!(
        refresh_at(second.path(), &target, DAILY, true, || START)
            .await
            .is_err()
    );
}

#[test]
fn explicit_capabilities_only_and_missing_invalid_limits_keep_diagnostics() {
    let normalized=normalizing::normalize_listing(&json!({"data":[
        {"id":"missing","reasoning":true,"random_tools":true,"random":{"modalities":["text"]}},
        {"id":"invalid","limits":{"context":0,"output":2}},
        {"id":"oversized-output","limits":{"context":100,"output":101}},
        {"id":"invalid-input","limits":{"context":100,"input":0,"output":10}},
        {"id":"ordinary","context_window":100,"max_tokens":10,"input_modalities":["text","unknown"],"output_modalities":["text"],"supported_parameters":["tools","structured_outputs"]}
    ]}),"xiaoge").unwrap();
    for id in ["missing", "invalid", "oversized-output", "invalid-input"] {
        let model = &normalized.models[id];
        assert!(model.limits.is_none());
        assert!(model.disabled_reason.is_some());
    }
    let missing = &normalized.models["missing"];
    assert!(!missing.tool_call);
    assert!(missing.reasoning_controls.is_none());
    assert!(missing.input_modalities.is_empty());
    let ordinary = &normalized.models["ordinary"];
    assert!(ordinary.tool_call && ordinary.structured_output);
    assert_eq!(ordinary.input_modalities.len(), 1);
    assert_eq!(ordinary.limits.unwrap().max_input_tokens, 100);
}

#[test]
fn pricing_preserves_partial_zero_and_checked_units_without_disabling_models() {
    let mut source = listing("selected-large");
    source["data"][0]["cost"] =
        json!({"currency":"USD","input":2.5,"output":-1,"cache_read":0,"cache_write":"free"});
    let provider = normalizing::normalize_listing(&source, "xiaoge").unwrap();
    let model = &provider.models["selected-large"];
    assert_eq!(
        model.cost,
        Some(smith_config::catalog::CatalogModelCost {
            input: Some(2_500_000),
            output: None,
            cache_read: Some(0),
            cache_write: None
        })
    );
    assert!(model.disabled_reason.is_none());
    source["data"][0]["cost"] = json!({"currency":"CNY","input":1});
    source["data"][0]["pricing"] = json!({"prompt":"0.0000025","completion":"0","input_cache_read":"-1","input_cache_write":"1e50"});
    let provider = normalizing::normalize_listing(&source, "xiaoge").unwrap();
    assert_eq!(
        provider.models["selected-large"].cost,
        Some(smith_config::catalog::CatalogModelCost {
            input: Some(2_500_000),
            output: Some(0),
            cache_read: None,
            cache_write: None
        })
    );
    source["data"][0]["pricing"]["unit"] = json!("tokens_per_dollar");
    assert!(
        normalizing::normalize_listing(&source, "xiaoge")
            .unwrap()
            .models["selected-large"]
            .cost
            .is_none()
    );
}

#[tokio::test]
async fn same_id_price_change_gets_a_distinct_revision_without_rewriting_the_old_snapshot() {
    let mut a = listing("selected-large");
    a["data"][0]["cost"] = json!({"input":1});
    let mut b = listing("selected-large");
    b["data"][0]["cost"] = json!({"input":2});
    let server = server(vec![
        response(200, "ETag: \"same\"\r\n", &a.to_string()),
        response(200, "ETag: \"same\"\r\n", &b.to_string()),
    ])
    .await;
    let user_dir = tempfile::tempdir().unwrap();
    let target = target(&server.base_url);
    let old = refresh_at(user_dir.path(), &target, DAILY, false, || START)
        .await
        .unwrap()
        .catalog
        .unwrap();
    let new = refresh_at(user_dir.path(), &target, DAILY, true, || START + 1)
        .await
        .unwrap()
        .catalog
        .unwrap();
    assert_ne!(old.revision, new.revision);
    assert_eq!(
        old.catalog.models["selected-large"].cost.unwrap().input,
        Some(1_000_000)
    );
    assert_eq!(
        new.catalog.models["selected-large"].cost.unwrap().input,
        Some(2_000_000)
    );
}

#[tokio::test]
async fn subprocess_refresh_worker() {
    let Some(root) = std::env::var_os("SMITH_PROVIDER_MODELS_TEST_ROOT") else {
        return;
    };
    let endpoint = std::env::var("SMITH_PROVIDER_MODELS_TEST_ENDPOINT").unwrap();
    let result = refresh_at(Path::new(&root), &target(&endpoint), DAILY, false, || START)
        .await
        .unwrap();
    assert!(matches!(
        result.status,
        SyncStatus::Updated | SyncStatus::Fresh
    ));
}

#[tokio::test]
async fn separate_process_refreshers_share_the_same_freshness_lock() {
    let mut server = server(vec![response(
        200,
        "",
        &listing("selected-large").to_string(),
    )])
    .await;
    let user_dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "provider_models::tests::subprocess_refresh_worker",
            "--nocapture",
        ])
        .env("SMITH_PROVIDER_MODELS_TEST_ROOT", user_dir.path())
        .env("SMITH_PROVIDER_MODELS_TEST_ENDPOINT", &server.base_url)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let outcome = refresh_at(
        user_dir.path(),
        &target(&server.base_url),
        DAILY,
        false,
        || START,
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome.status,
        SyncStatus::Updated | SyncStatus::Fresh
    ));
    let exit = tokio::task::spawn_blocking(move || child.wait().unwrap())
        .await
        .unwrap();
    assert!(exit.success());
    assert!(server.requests.recv().await.is_some());
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn partial_limits_are_allowlisted_and_revisioned_without_inventing_context() {
    let source = json!({"data":[{"id":"partial","limits":{"input":1000000,"output":256000},"tool_call":true}]});
    let server = server(vec![response(200, "", &source.to_string())]).await;
    let user_dir = tempfile::tempdir().unwrap();
    let target = target(&server.base_url);
    let catalog = refresh_at(user_dir.path(), &target, DAILY, false, || START)
        .await
        .unwrap()
        .catalog
        .unwrap();
    assert!(catalog.catalog.models["partial"].limits.is_none());
    let partial = catalog.partial_limits["partial"];
    assert_eq!(partial.context_tokens, None);
    assert_eq!(partial.max_input_tokens, Some(1000000));
    assert_eq!(partial.max_output_tokens, Some(256000));
    assert_eq!(
        load_cached(user_dir.path(), &target)
            .unwrap()
            .partial_limits,
        catalog.partial_limits
    );
}

#[test]
fn capability_presence_distinguishes_unknown_existing_models_from_explicit_denial() {
    let source = json!({"data":[
        {"id":"unknown"},
        {"id":"missing-tools","output_modalities":["text"]},
        {"id":"missing-text","capabilities":{"tool_calls":true}},
        {"id":"denied-tools","tool_call":false,"supported_parameters":["tools"],"output_modalities":["text"]},
        {"id":"nontext","tool_call":true,"output_modalities":["image"]},
        {"id":"complete","capabilities":{"tool_calls":true},"output_modalities":["text"]}
    ]});
    let missing = normalizing::unadvertised_capabilities(&source);
    assert_eq!(
        missing,
        BTreeSet::from([
            "unknown".to_owned(),
            "missing-tools".to_owned(),
            "missing-text".to_owned()
        ])
    );
    let normalized = normalizing::normalize_listing(&source, "xiaoge").unwrap();
    assert!(!normalized.models["denied-tools"].tool_call);
    assert!(normalized.models["complete"].tool_call);
}

#[test]
fn numeric_price_literals_do_not_pass_through_floating_point_rounding() {
    let source:Value=serde_json::from_str(r#"{"data":[{"id":"price-boundary","cost":{"input":18446744073709.551615,"output":18446744073709.551616}}]}"#).unwrap();
    let provider = normalizing::normalize_listing(&source, "xiaoge").unwrap();
    let cost = provider.models["price-boundary"].cost.unwrap();
    assert_eq!(cost.input, Some(u64::MAX));
    assert_eq!(cost.output, None);
}

#[tokio::test]
async fn frozen_binding_guard_rejects_changed_provider_authority() {
    let server = server(vec![response(
        200,
        "",
        &listing("selected-large").to_string(),
    )])
    .await;
    let user_dir = tempfile::tempdir().unwrap();
    let original = target(&server.base_url);
    let catalog = refresh_at(user_dir.path(), &original, DAILY, false, || START)
        .await
        .unwrap()
        .catalog
        .unwrap();
    assert!(binding_matches(&original, &catalog));
    assert!(!binding_matches(
        &ProviderModelTarget {
            credential_scope: "new-account".to_owned(),
            ..original.clone()
        },
        &catalog
    ));
    assert!(!binding_matches(
        &ProviderModelTarget {
            base_url: format!("{}/new", original.base_url),
            ..original
        },
        &catalog
    ));
}

#[tokio::test]
async fn stalled_listing_is_cancelled_at_the_total_budget_with_a_fake_clock() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let request = tokio::spawn(async move {
        read_listing(&endpoint, Some("not-for-errors"), &BTreeMap::new(), None).await
    });
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        header.push(socket.read_u8().await.unwrap());
    }
    // The peer accepted the authenticated request but never sends a response.
    // Advancing Tokio's clock covers the whole pending HTTP operation.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    let error = request
        .await
        .unwrap()
        .err()
        .expect("a stalled request must time out");
    assert!(!error.contains("not-for-errors"));
}
