use super::*;

use std::time::Duration;

use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::provider::{
    ModelDescriptor, ProviderCallContext, ProviderError, ProviderErrorKind, ProviderRequest,
    ProviderStream,
};

fn failure(error: ProviderError) -> ScriptedStream {
    ScriptedStream::new(vec![ProviderStreamEvent::Error { error }])
}

fn server_failure() -> ScriptedStream {
    failure(ProviderError::new(ProviderErrorKind::Server, "unavailable"))
}

fn success() -> ScriptedStream {
    ScriptedStream::new(vec![
        ProviderStreamEvent::TextDelta {
            text: "recovered".into(),
        },
        ProviderStreamEvent::Finish {
            reason: FinishReason::Stop,
        },
    ])
}

fn retry_delays(recorder: &RecordingObserver) -> Vec<Option<u64>> {
    recorder
        .events()
        .iter()
        .filter_map(|event| match event.payload {
            RuntimeEvent::ProviderAttemptFinished { retry_delay_ms, .. } => Some(retry_delay_ms),
            _ => None,
        })
        .collect()
}

fn assert_retry_windows(delays: &[Option<u64>], ceilings: &[u64]) {
    assert_eq!(delays.len(), ceilings.len() + 1);
    assert_eq!(delays.last(), Some(&None));
    for (delay, ceiling) in delays.iter().zip(ceilings) {
        let delay = delay.expect("an admitted retry");
        assert!((ceiling / 2..=*ceiling).contains(&delay), "{delay}ms");
    }
}

fn total_wait(delays: &[Option<u64>]) -> Duration {
    Duration::from_millis(delays.iter().flatten().sum())
}

async fn setup(
    config: &str,
    scripts: Vec<ScriptedStream>,
) -> (
    smith_runtime::factory::SmithRuntime,
    Arc<FakeProvider>,
    Arc<RecordingObserver>,
) {
    let fixture = Fixture::new(config);
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        scripts,
    ));
    let recorder = RecordingObserver::shared();
    let smith = factory::build_request(RuntimeRequest {
        provider: Some(provider.clone()),
        observers: vec![recorder.clone()],
        ..request(&fixture, HostSurface::Terminal)
    })
    .await
    .expect("a runtime");
    (smith, provider, recorder)
}

#[tokio::test(start_paused = true)]
async fn provider_retries_grow_with_jitter_and_reset_for_the_next_request() {
    let (smith, provider, recorder) = setup(
        FAKE_CONFIG,
        vec![
            server_failure(),
            server_failure(),
            server_failure(),
            server_failure(),
            server_failure(),
            success(),
            server_failure(),
            success(),
        ],
    )
    .await;
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    session.run(UserInput::text("hello")).await.unwrap();
    let delays = retry_delays(&recorder);
    assert_retry_windows(&delays, &[1_000, 2_000, 4_000, 8_000, 16_000]);
    assert_eq!(started.elapsed(), total_wait(&delays));
    session.run(UserInput::text("again")).await.unwrap();
    let delays = retry_delays(&recorder);
    assert_retry_windows(&delays[6..], &[1_000]);
    assert_eq!(started.elapsed(), total_wait(&delays));
    assert_eq!(provider.requests().len(), 8);
    assert!(
        session
            .history()
            .iter()
            .any(|message| message.joined_text() == "recovered")
    );
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn provider_retries_stop_after_five_retries() {
    let (smith, provider, recorder) =
        setup(FAKE_CONFIG, (0..6).map(|_| server_failure()).collect()).await;
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session.run(UserInput::text("hello")).await.unwrap();
    assert_eq!(provider.requests().len(), 6);
    assert_retry_windows(
        &retry_delays(&recorder),
        &[1_000, 2_000, 4_000, 8_000, 16_000],
    );
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn provider_retries_with_a_larger_budget_cap_the_jitter_window() {
    let config = format!("{FAKE_CONFIG}\n[limits]\nmax_retries = 7\n");
    let mut scripts: Vec<_> = (0..7).map(|_| server_failure()).collect();
    scripts.push(success());
    let (smith, provider, recorder) = setup(&config, scripts).await;
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    session.run(UserInput::text("hello")).await.unwrap();
    let delays = retry_delays(&recorder);
    assert_retry_windows(
        &delays,
        &[1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000],
    );
    assert_eq!(started.elapsed(), total_wait(&delays));
    assert_eq!(provider.requests().len(), 8);
    session.shutdown().await.unwrap();
}

#[derive(Debug)]
struct FailsBeforeStream(FakeProvider);

#[async_trait]
impl Provider for FailsBeforeStream {
    fn describe(&self) -> Vec<ModelDescriptor> {
        self.0.describe()
    }

    fn capabilities(&self, model: &ModelId) -> Option<Capabilities> {
        self.0.capabilities(model)
    }

    async fn stream(
        &self,
        _request: ProviderRequest,
        _ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        Err(ProviderError::new(ProviderErrorKind::Server, "unavailable"))
    }
}

#[tokio::test(start_paused = true)]
async fn provider_retries_also_back_off_when_opening_the_stream_fails() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let recorder = RecordingObserver::shared();
    let smith = factory::build_request(RuntimeRequest {
        provider: Some(Arc::new(FailsBeforeStream(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![],
        )))),
        observers: vec![recorder.clone()],
        ..request(&fixture, HostSurface::Headless)
    })
    .await
    .unwrap();
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    session.run(UserInput::text("hello")).await.unwrap();
    let delays = retry_delays(&recorder);
    assert_retry_windows(&delays, &[1_000, 2_000, 4_000, 8_000, 16_000]);
    assert_eq!(started.elapsed(), total_wait(&delays));
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn provider_retries_honor_longer_server_hints() {
    let (smith, provider, recorder) = setup(
        FAKE_CONFIG,
        vec![
            failure(ProviderError::new(ProviderErrorKind::RateLimited, "busy").retry_after(15_000)),
            failure(ProviderError::new(ProviderErrorKind::Server, "busy").retry_after(100)),
            success(),
        ],
    )
    .await;
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    session.run(UserInput::text("hello")).await.unwrap();
    let delays = retry_delays(&recorder);
    assert_eq!(delays[0], Some(15_000));
    assert_retry_windows(&delays[1..], &[2_000]);
    assert_eq!(started.elapsed(), total_wait(&delays));
    assert_eq!(provider.requests().len(), 3);
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn provider_retries_preserve_the_rate_limit_floor_without_a_server_hint() {
    let (smith, provider, recorder) = setup(
        FAKE_CONFIG,
        vec![
            failure(ProviderError::new(ProviderErrorKind::RateLimited, "busy")),
            success(),
        ],
    )
    .await;
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    session.run(UserInput::text("hello")).await.unwrap();
    assert_eq!(started.elapsed(), Duration::from_secs(2));
    assert_eq!(retry_delays(&recorder), [Some(2_000), None]);
    assert_eq!(provider.requests().len(), 2);
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn provider_retries_respect_disabled_retries_and_permanent_errors() {
    for (config, error) in [
        (
            format!("{FAKE_CONFIG}\n[limits]\nmax_retries = 0\n"),
            ProviderErrorKind::Server,
        ),
        (FAKE_CONFIG.to_owned(), ProviderErrorKind::BadRequest),
    ] {
        let (smith, provider, recorder) = setup(
            &config,
            vec![failure(ProviderError::new(error, "rejected"))],
        )
        .await;
        let session = smith
            .runtime()
            .start_session(StartSession::new())
            .await
            .unwrap();
        session.run(UserInput::text("hello")).await.unwrap();
        assert_eq!(provider.requests().len(), 1);
        assert_eq!(retry_delays(&recorder), [None]);
        session.shutdown().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn provider_retry_wait_can_be_cancelled() {
    let (smith, provider, recorder) = setup(FAKE_CONFIG, vec![server_failure(), success()]).await;
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let run = session.run(UserInput::text("hello"));
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => panic!("the turn finished before cancellation: {result:?}"),
        _ = async {
            while retry_delays(&recorder).is_empty() {
                tokio::task::yield_now().await;
            }
        } => {}
    }
    session.cancel(CancelReason::UserRequested);
    run.await.unwrap();
    assert_eq!(provider.requests().len(), 1);
    let delays = retry_delays(&recorder);
    assert_eq!(delays.len(), 1);
    assert!((500..=1_000).contains(&delays[0].unwrap()));
    session.shutdown().await.unwrap();
}
