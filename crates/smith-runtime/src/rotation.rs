//! Usage-aware credential rotation: leasing the active account, and offering
//! to move to another one when the provider says the current one is spent.
//!
//! Two pieces cooperate, and the split matters.
//!
//! [`PoolCredentialSource`] answers "which account authorizes this attempt".
//! Adapters acquire a lease per attempt rather than carrying a baked-in key,
//! so changing accounts is a state change here — no adapter is rebuilt, no
//! transport is reopened, and the switch takes effect on the very next
//! acquisition.
//!
//! [`PooledProvider`] answers "what happens when that account is spent". It
//! wraps the adapter, and on a typed limit-exhaustion failure it *asks* before
//! replaying on another member. Asking is the point: rotating abandons the
//! provider-side prompt cache, so the replayed turn resubmits its whole
//! context uncached, and it spends a second account's budget. Neither is a
//! decision to make on the user's behalf.
//!
//! The replay fence is deliberately coarse and deliberately safe: once the
//! adapter has returned a stream, this type never replays, whatever the stream
//! goes on to say. A stream that was accepted may already have produced output
//! the user has seen, and re-running the turn underneath that is worse than
//! reporting the failure.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Clock, Deadline};
use agent_runtime_core::provider::{
    Capabilities, ModelDescriptor, ModelId, Provider, ProviderCallContext, ProviderError,
    ProviderErrorKind, ProviderRequest, ProviderStream, ProviderStreamEvent,
};
use agent_runtime_core::provider_credential::{
    CredentialInvalidation, ProviderAuthRejection, ProviderCredentialError,
    ProviderCredentialLease, ProviderCredentialRevision, ProviderCredentialSource,
    ProviderCredentialTarget,
};
use agent_runtime_core::store::Secret;
use async_stream::stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use smith_config::credential::{CredentialRef, CredentialResolver};
use smith_host::rotation::{RotationDecision, RotationPolicy};

use crate::pool::CredentialPool;

/// Pool state shared between the credential source, the provider decorator,
/// and whichever surface shows accounts to the user.
#[derive(Debug, Clone)]
pub struct SharedPool(Arc<Mutex<CredentialPool>>);

impl SharedPool {
    /// Wraps `pool` for shared access.
    pub fn new(pool: CredentialPool) -> Self {
        Self(Arc::new(Mutex::new(pool)))
    }

    /// Reads the pool under its lock.
    pub fn read<T>(&self, f: impl FnOnce(&CredentialPool) -> T) -> T {
        f(&self.0.lock().expect("credential pool poisoned"))
    }

    /// Mutates the pool under its lock.
    pub fn write<T>(&self, f: impl FnOnce(&mut CredentialPool) -> T) -> T {
        f(&mut self.0.lock().expect("credential pool poisoned"))
    }
}

/// Resolves credential references to secrets off the async executor.
///
/// Kept behind a trait so rotation can be tested without a keychain: the
/// production implementation opens the platform credential service, which is
/// blocking, prompting, and unavailable in CI.
#[async_trait]
pub trait PoolSecrets: Send + Sync + std::fmt::Debug {
    /// Resolves one reference to its secret.
    async fn resolve(&self, reference: &str) -> Result<Secret, ProviderCredentialError>;
}

/// The production secret resolver, backed by Smith's credential resolver.
#[derive(Debug)]
pub struct HostPoolSecrets {
    resolver: CredentialResolver,
}

impl HostPoolSecrets {
    /// Resolves against `resolver`.
    pub fn new(resolver: CredentialResolver) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl PoolSecrets for HostPoolSecrets {
    async fn resolve(&self, reference: &str) -> Result<Secret, ProviderCredentialError> {
        let reference =
            CredentialRef::parse(reference).map_err(|_| ProviderCredentialError::Unavailable)?;
        let resolver = self.resolver.clone();
        // The credential service is blocking and may prompt, so it never runs
        // on an executor thread.
        tokio::task::spawn_blocking(move || resolver.resolve_blocking(&reference))
            .await
            .map_err(|_| ProviderCredentialError::RefreshFailed)?
            .map_err(|_| ProviderCredentialError::Unavailable)
    }
}

/// Leases whichever pool member is active.
///
/// Secrets are cached per member after their first resolution, so switching
/// back to an account does not re-open the credential service and re-prompt.
/// The cache holds only members the session actually used: an account that is
/// never selected is never read.
#[derive(Debug)]
pub struct PoolCredentialSource {
    pool: SharedPool,
    secrets: Arc<dyn PoolSecrets>,
    cache: Mutex<BTreeMap<usize, Secret>>,
}

impl PoolCredentialSource {
    /// Leases members of `pool`, resolving them through `secrets`.
    pub fn new(pool: SharedPool, secrets: Arc<dyn PoolSecrets>) -> Self {
        Self {
            pool,
            secrets,
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    /// Pre-populates the cache for `position`.
    ///
    /// The active member's secret is already resolved by the time the adapter
    /// is built, and reading it a second time would open the credential
    /// service — and on some platforms prompt — for a value in hand.
    pub fn seed(&self, position: usize, secret: Secret) {
        self.cache
            .lock()
            .expect("credential cache poisoned")
            .insert(position, secret);
    }

    fn cached(&self, position: usize) -> Option<Secret> {
        self.cache
            .lock()
            .expect("credential cache poisoned")
            .get(&position)
            .cloned()
    }
}

#[async_trait]
impl ProviderCredentialSource for PoolCredentialSource {
    async fn acquire(
        &self,
        _target: &ProviderCredentialTarget,
        _minimum_validity_ms: u64,
        cancel: &Cancellation,
        _deadline: Deadline,
    ) -> Result<ProviderCredentialLease, ProviderCredentialError> {
        if cancel.is_cancelled() {
            return Err(ProviderCredentialError::Cancelled);
        }
        let (position, reference) = self.pool.read(|pool| {
            (
                pool.active_position(),
                pool.active().map(|member| member.reference.clone()),
            )
        });
        let reference = reference.ok_or(ProviderCredentialError::Unavailable)?;

        let secret = match self.cached(position) {
            Some(secret) => secret,
            None => {
                let secret = self.secrets.resolve(&reference).await?;
                self.cache
                    .lock()
                    .expect("credential cache poisoned")
                    .insert(position, secret.clone());
                secret
            }
        };

        // The revision names the pool position, so a rejection of one member's
        // lease can never invalidate another's. It carries no secret and no
        // account identity beyond the position the user can already see.
        let revision = ProviderCredentialRevision::new(format!("pool-{position}"))?;
        Ok(ProviderCredentialLease::non_expiring(secret, revision))
    }

    async fn invalidate(
        &self,
        _target: &ProviderCredentialTarget,
        rejected_revision: &ProviderCredentialRevision,
        _rejection: ProviderAuthRejection,
        _cancel: &Cancellation,
        _deadline: Deadline,
    ) -> Result<CredentialInvalidation, ProviderCredentialError> {
        // A rejected credential is dropped from the cache so the next attempt
        // re-reads it: the stored value may have been rotated by whatever tool
        // owns it. Whether a *replacement* exists is not something a static
        // reference can promise, so this reports no replacement rather than
        // inviting an unbounded renewal loop.
        let position = self.pool.read(CredentialPool::active_position);
        if format!("pool-{position}") == format!("{rejected_revision:?}") {
            // Unreachable in practice: revisions are opaque in Debug. Kept as
            // a no-op branch rather than pretending the comparison is possible.
        }
        self.cache
            .lock()
            .expect("credential cache poisoned")
            .remove(&position);
        Ok(CredentialInvalidation::NoReplacement)
    }
}

/// Wraps a provider so a spent account can move to another one, with consent.
#[derive(Debug)]
pub struct PooledProvider {
    inner: Arc<dyn Provider>,
    pool: SharedPool,
    policy: Arc<dyn RotationPolicy>,
    clock: Arc<dyn Clock>,
}

impl PooledProvider {
    /// Wraps `inner`, offering rotation across `pool` through `policy`.
    pub fn new(
        inner: Arc<dyn Provider>,
        pool: SharedPool,
        policy: Arc<dyn RotationPolicy>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            inner,
            pool,
            policy,
            clock,
        }
    }

    fn now_ms(&self) -> u64 {
        self.clock.now().as_millis()
    }

    /// Records that the active member's window is spent.
    ///
    /// Kept separate from offering, and performed unconditionally, because the
    /// cooldown is a fact the provider stated. Whether anywhere else can serve
    /// the turn does not change whether *this* account is spent, and skipping
    /// the record on the last member would send the next turn straight back at
    /// an account already known to be exhausted.
    fn record_exhaustion(&self, error: &ProviderError) {
        let now_ms = self.now_ms();
        self.pool.write(|pool| {
            let active = pool.active_position();
            pool.exhaust(active, error.limit_resets_at_ms, now_ms);
        });
    }

    /// Asks whether to move to another member.
    ///
    /// Returns `true` when the caller should replay the attempt.
    async fn offer_rotation(&self) -> bool {
        let now_ms = self.now_ms();
        // Built after the cooldown lands, so the offer reports the reset the
        // user is actually waiting on.
        let request = self.pool.read(|pool| pool.exhaustion_offer(now_ms));

        // No eligible member means there is no question worth asking: every
        // account is spent, and the caller fails the turn with the soonest
        // reset instead.
        let Some(request) = request else {
            return false;
        };

        match self.policy.decide(&request).await {
            RotationDecision::Switch { position } => {
                // Only a member the offer actually listed may be selected, so
                // a confused surface cannot select a cooling account.
                if !request
                    .eligible
                    .iter()
                    .any(|member| member.position == position)
                {
                    return false;
                }
                self.pool.write(|pool| pool.set_active(position))
            }
            RotationDecision::Decline | RotationDecision::Unavailable { .. } => false,
        }
    }

    /// Offers rotation when the active member has crossed the configured
    /// threshold, if one is configured.
    ///
    /// Silent when no threshold is set, when nothing has measured the active
    /// member, or when it is still below the line — an unmeasured account is
    /// not over the line any more than it is at zero.
    async fn offer_threshold(&self) {
        let now_ms = self.now_ms();
        let Some(request) = self.pool.read(|pool| pool.threshold_offer(now_ms)) else {
            return;
        };
        // Marked before asking, so a declined offer is not repeated for every
        // attempt within the turn.
        self.pool.write(CredentialPool::mark_threshold_offered);

        if let RotationDecision::Switch { position } = self.policy.decide(&request).await
            && request
                .eligible
                .iter()
                .any(|member| member.position == position)
        {
            self.pool.write(|pool| pool.set_active(position));
        }
    }

    /// Adds the pool's soonest reset to a terminal exhaustion error.
    fn with_earliest_reset(&self, mut error: ProviderError) -> ProviderError {
        if error.limit_resets_at_ms.is_none()
            && let Some(reset) = self.pool.read(|pool| pool.earliest_reset(self.now_ms()))
        {
            error = error.limit_resets_at(reset);
        }
        error
    }
}

#[async_trait]
impl Provider for PooledProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        self.inner.describe()
    }

    fn capabilities(&self, model: &ModelId) -> Option<Capabilities> {
        self.inner.capabilities(model)
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        // Checked before the attempt is spent, not after it fails: the whole
        // point of a threshold is to move while the current account still has
        // room, so the switch costs one uncached turn instead of a failed one
        // plus an uncached turn.
        self.offer_threshold().await;

        // At most one replay per member: the loop can visit each account once,
        // so a pool that is entirely spent terminates instead of cycling.
        let mut remaining = self.pool.read(|pool| pool.members().len());

        loop {
            let attempt = self.inner.stream(request.clone(), ctx.clone()).await;
            let error = match attempt {
                // The adapter accepted a stream. From here the fence is
                // absolute: whatever the stream reports, this attempt is the
                // one the user gets.
                Ok(stream) => return Ok(self.observe(stream)),
                Err(error) => error,
            };

            if error.kind != ProviderErrorKind::LimitExhausted {
                return Err(error);
            }
            self.record_exhaustion(&error);
            remaining = remaining.saturating_sub(1);
            if remaining == 0 || !self.offer_rotation().await {
                return Err(self.with_earliest_reset(error));
            }
        }
    }
}

impl PooledProvider {
    /// Wraps an accepted stream to record what it reports about the account.
    ///
    /// Nothing here can replay. It exists so a snapshot reaches the pool (and
    /// so the surfaces have a meter to draw), and so a member that reports
    /// exhaustion mid-stream still enters cooldown for the *next* attempt.
    fn observe(&self, mut inner: ProviderStream) -> ProviderStream {
        let pool = self.pool.clone();
        let clock = Arc::clone(&self.clock);
        Box::pin(stream! {
            let position = pool.read(CredentialPool::active_position);
            while let Some(event) = inner.next().await {
                match &event {
                    ProviderStreamEvent::RateLimit { snapshot } => {
                        pool.write(|pool| pool.record_snapshot(position, snapshot.clone()));
                    }
                    ProviderStreamEvent::Error { error }
                        if error.kind == ProviderErrorKind::LimitExhausted =>
                    {
                        let now_ms = clock.now().as_millis();
                        pool.write(|pool| pool.exhaust(position, error.limit_resets_at_ms, now_ms));
                    }
                    _ => {}
                }
                yield event;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::clock::Timestamp;
    use agent_runtime_core::ids::{AttemptId, RequestId, SessionId};
    use agent_runtime_core::provider::{
        FinishReason, RateLimitSnapshot, RateLimitWindow, ReasoningSupport,
    };
    use smith_host::rotation::{
        HeadlessRotation, InteractiveRotation, RotationRequest, RotationTrigger,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    const NOW: u64 = 1_785_862_800_000;
    const RESET: u64 = NOW + 3_600_000;

    #[derive(Debug)]
    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> Timestamp {
            Timestamp::ZERO.plus_millis(NOW)
        }
    }

    /// A provider that fails with exhaustion for the first `exhaust` attempts,
    /// then succeeds, recording how many attempts it served.
    #[derive(Debug)]
    struct ScriptedProvider {
        exhaust: usize,
        attempts: AtomicUsize,
    }

    impl ScriptedProvider {
        fn new(exhaust: usize) -> Arc<Self> {
            Arc::new(Self {
                exhaust,
                attempts: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn describe(&self) -> Vec<ModelDescriptor> {
            Vec::new()
        }

        fn capabilities(&self, _model: &ModelId) -> Option<Capabilities> {
            Some(Capabilities {
                reasoning: ReasoningSupport::Unsupported,
                ..Capabilities::basic_streaming()
            })
        }

        async fn stream(
            &self,
            _request: ProviderRequest,
            _ctx: ProviderCallContext,
        ) -> Result<ProviderStream, ProviderError> {
            let index = self.attempts.fetch_add(1, Ordering::SeqCst);
            if index < self.exhaust {
                return Err(ProviderError::new(
                    ProviderErrorKind::LimitExhausted,
                    "spent",
                )
                .limit_resets_at(RESET));
            }
            Ok(Box::pin(stream! {
                yield ProviderStreamEvent::TextDelta { text: "ok".into() };
                yield ProviderStreamEvent::Finish { reason: FinishReason::Stop };
            }))
        }
    }

    /// A provider whose accepted stream later reports exhaustion.
    #[derive(Debug)]
    struct MidStreamExhaustion {
        attempts: AtomicUsize,
    }

    #[async_trait]
    impl Provider for MidStreamExhaustion {
        fn describe(&self) -> Vec<ModelDescriptor> {
            Vec::new()
        }

        fn capabilities(&self, _model: &ModelId) -> Option<Capabilities> {
            None
        }

        async fn stream(
            &self,
            _request: ProviderRequest,
            _ctx: ProviderCallContext,
        ) -> Result<ProviderStream, ProviderError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(stream! {
                yield ProviderStreamEvent::TextDelta { text: "partial".into() };
                yield ProviderStreamEvent::Error {
                    error: ProviderError::new(ProviderErrorKind::LimitExhausted, "spent")
                        .limit_resets_at(RESET),
                };
            }))
        }
    }

    /// A provider reporting a rate-limit snapshot on a successful attempt.
    #[derive(Debug)]
    struct ReportingProvider;

    #[async_trait]
    impl Provider for ReportingProvider {
        fn describe(&self) -> Vec<ModelDescriptor> {
            Vec::new()
        }

        fn capabilities(&self, _model: &ModelId) -> Option<Capabilities> {
            None
        }

        async fn stream(
            &self,
            _request: ProviderRequest,
            _ctx: ProviderCallContext,
        ) -> Result<ProviderStream, ProviderError> {
            let mut snapshot = RateLimitSnapshot::new();
            snapshot.push(RateLimitWindow {
                used_percent: Some(82.0),
                ..RateLimitWindow::new("primary")
            });
            Ok(Box::pin(stream! {
                yield ProviderStreamEvent::RateLimit { snapshot };
                yield ProviderStreamEvent::Finish { reason: FinishReason::Stop };
            }))
        }
    }

    /// A policy that always switches to the first eligible member.
    #[derive(Debug)]
    struct AlwaysSwitch;

    #[async_trait]
    impl RotationPolicy for AlwaysSwitch {
        async fn decide(&self, request: &RotationRequest) -> RotationDecision {
            RotationDecision::Switch {
                position: request.eligible[0].position,
            }
        }
    }

    /// A policy that always refuses.
    #[derive(Debug)]
    struct AlwaysDecline;

    #[async_trait]
    impl RotationPolicy for AlwaysDecline {
        async fn decide(&self, _request: &RotationRequest) -> RotationDecision {
            RotationDecision::Decline
        }
    }

    fn pool(members: usize) -> SharedPool {
        let references = (0..members).map(|index| format!("env:ACCOUNT_{index}"));
        SharedPool::new(CredentialPool::new("acme", references, None))
    }

    fn pool_with_threshold(percent: u8) -> SharedPool {
        let references = (0..2).map(|index| format!("env:ACCOUNT_{index}"));
        SharedPool::new(CredentialPool::new("acme", references, Some(percent)))
    }

    /// A policy that counts how often it was asked.
    #[derive(Debug)]
    struct CountingDecline {
        asked: AtomicUsize,
    }

    #[async_trait]
    impl RotationPolicy for CountingDecline {
        async fn decide(&self, _request: &RotationRequest) -> RotationDecision {
            self.asked.fetch_add(1, Ordering::SeqCst);
            RotationDecision::Decline
        }
    }

    fn ctx() -> ProviderCallContext {
        ProviderCallContext {
            session: SessionId::new("s1"),
            request_id: RequestId::new("r1"),
            attempt_id: AttemptId::new("a1"),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
        }
    }

    fn request() -> ProviderRequest {
        ProviderRequest::new(ModelId::new("m"), Vec::new())
    }


    /// `expect_err` needs the success type to be `Debug`, and a boxed stream
    /// is not. This keeps the assertion without weakening the stream type.
    fn expect_exhausted(result: Result<ProviderStream, ProviderError>) -> ProviderError {
        match result {
            Ok(_) => panic!("expected the attempt to fail"),
            Err(error) => error,
        }
    }

    async fn collect(stream: ProviderStream) -> Vec<ProviderStreamEvent> {
        stream.collect::<Vec<_>>().await
    }

    #[tokio::test]
    async fn a_confirmed_rotation_replays_on_the_next_member() {
        let pool = pool(2);
        let inner = ScriptedProvider::new(1);
        let provider = PooledProvider::new(
            Arc::clone(&inner) as Arc<dyn Provider>,
            pool.clone(),
            Arc::new(AlwaysSwitch),
            Arc::new(FixedClock),
        );

        let events = collect(
            provider
                .stream(request(), ctx())
                .await
                .expect("the replay succeeded"),
        )
        .await;

        assert!(events.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::Finish { reason: FinishReason::Stop }
        )));
        // Exactly two attempts: the spent one, and the replay.
        assert_eq!(inner.attempts.load(Ordering::SeqCst), 2);
        pool.read(|pool| {
            assert_eq!(pool.active_position(), 1);
            // The outgoing member cools until the reset the provider reported.
            assert_eq!(pool.cooling_until(0, NOW), Some(RESET));
        });
    }

    #[tokio::test]
    async fn a_declined_rotation_fails_the_turn_and_spends_no_second_account() {
        let pool = pool(2);
        let inner = ScriptedProvider::new(1);
        let provider = PooledProvider::new(
            Arc::clone(&inner) as Arc<dyn Provider>,
            pool.clone(),
            Arc::new(AlwaysDecline),
            Arc::new(FixedClock),
        );

        let error = expect_exhausted(provider.stream(request(), ctx()).await);

        assert_eq!(error.kind, ProviderErrorKind::LimitExhausted);
        assert_eq!(error.limit_resets_at_ms, Some(RESET));
        // One attempt only: the second account was never touched.
        assert_eq!(inner.attempts.load(Ordering::SeqCst), 1);
        pool.read(|pool| assert_eq!(pool.active_position(), 0));
    }

    #[tokio::test]
    async fn a_headless_run_never_rotates() {
        let pool = pool(2);
        let inner = ScriptedProvider::new(1);
        let policy = Arc::new(HeadlessRotation::new());
        let provider = PooledProvider::new(
            Arc::clone(&inner) as Arc<dyn Provider>,
            pool.clone(),
            Arc::clone(&policy) as Arc<dyn RotationPolicy>,
            Arc::new(FixedClock),
        );

        let error = expect_exhausted(provider.stream(request(), ctx()).await);

        assert_eq!(error.kind, ProviderErrorKind::LimitExhausted);
        assert_eq!(inner.attempts.load(Ordering::SeqCst), 1);
        pool.read(|pool| assert_eq!(pool.active_position(), 0));
        // The run's outcome is recorded for machine output.
        let required = policy.required().expect("a recorded exhaustion");
        assert_eq!(required.outgoing, "env:ACCOUNT_0");
        assert_eq!(required.resets_at_ms, Some(RESET));
    }

    #[tokio::test]
    async fn an_all_exhausted_pool_fails_with_the_earliest_reset_and_no_loop() {
        let pool = pool(2);
        // Every attempt is spent, so no member can serve the turn.
        let inner = ScriptedProvider::new(usize::MAX);
        let provider = PooledProvider::new(
            Arc::clone(&inner) as Arc<dyn Provider>,
            pool.clone(),
            Arc::new(AlwaysSwitch),
            Arc::new(FixedClock),
        );

        let error = expect_exhausted(provider.stream(request(), ctx()).await);

        assert_eq!(error.kind, ProviderErrorKind::LimitExhausted);
        assert_eq!(error.limit_resets_at_ms, Some(RESET));
        // Bounded: one attempt per member, never a cycle.
        assert_eq!(inner.attempts.load(Ordering::SeqCst), 2);
        pool.read(|pool| {
            assert!(!pool.is_eligible(0, NOW));
            assert!(!pool.is_eligible(1, NOW));
            assert_eq!(pool.earliest_reset(NOW), Some(RESET));
        });
    }

    #[tokio::test]
    async fn a_single_member_pool_never_offers_and_fails_once() {
        let pool = pool(1);
        let inner = ScriptedProvider::new(usize::MAX);
        let provider = PooledProvider::new(
            Arc::clone(&inner) as Arc<dyn Provider>,
            pool.clone(),
            Arc::new(AlwaysSwitch),
            Arc::new(FixedClock),
        );

        let error = expect_exhausted(provider.stream(request(), ctx()).await);

        assert_eq!(error.kind, ProviderErrorKind::LimitExhausted);
        assert_eq!(inner.attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_accepted_stream_is_never_replayed_but_still_cools_the_member() {
        let pool = pool(2);
        let inner = Arc::new(MidStreamExhaustion {
            attempts: AtomicUsize::new(0),
        });
        let provider = PooledProvider::new(
            Arc::clone(&inner) as Arc<dyn Provider>,
            pool.clone(),
            Arc::new(AlwaysSwitch),
            Arc::new(FixedClock),
        );

        let events = collect(
            provider
                .stream(request(), ctx())
                .await
                .expect("the stream was accepted"),
        )
        .await;

        // The failure is surfaced as-is; the user keeps the output they saw.
        assert!(matches!(events[0], ProviderStreamEvent::TextDelta { .. }));
        assert!(matches!(events[1], ProviderStreamEvent::Error { .. }));
        assert_eq!(inner.attempts.load(Ordering::SeqCst), 1);
        pool.read(|pool| {
            assert_eq!(pool.active_position(), 0, "no switch after an accepted stream");
            // The next attempt still avoids the spent account.
            assert_eq!(pool.cooling_until(0, NOW), Some(RESET));
        });
    }

    #[tokio::test]
    async fn a_reported_snapshot_reaches_the_active_member() {
        let pool = pool(2);
        let provider = PooledProvider::new(
            Arc::new(ReportingProvider) as Arc<dyn Provider>,
            pool.clone(),
            Arc::new(AlwaysDecline),
            Arc::new(FixedClock),
        );

        collect(provider.stream(request(), ctx()).await.expect("a stream")).await;

        pool.read(|pool| {
            assert_eq!(pool.used_percent(0), Some(82.0));
            // The member that never served an attempt stays unmeasured.
            assert_eq!(pool.used_percent(1), None);
        });
    }

    #[tokio::test]
    async fn a_non_exhaustion_failure_is_returned_untouched() {
        #[derive(Debug)]
        struct Failing;

        #[async_trait]
        impl Provider for Failing {
            fn describe(&self) -> Vec<ModelDescriptor> {
                Vec::new()
            }
            fn capabilities(&self, _model: &ModelId) -> Option<Capabilities> {
                None
            }
            async fn stream(
                &self,
                _request: ProviderRequest,
                _ctx: ProviderCallContext,
            ) -> Result<ProviderStream, ProviderError> {
                Err(ProviderError::new(ProviderErrorKind::Server, "500").retryable())
            }
        }

        let pool = pool(2);
        let provider = PooledProvider::new(
            Arc::new(Failing) as Arc<dyn Provider>,
            pool.clone(),
            Arc::new(AlwaysSwitch),
            Arc::new(FixedClock),
        );

        let error = expect_exhausted(provider.stream(request(), ctx()).await);
        // A server error is the retry policy's business, not rotation's.
        assert_eq!(error.kind, ProviderErrorKind::Server);
        pool.read(|pool| assert_eq!(pool.active_position(), 0));
    }

    #[tokio::test]
    async fn a_surface_choosing_an_unoffered_member_does_not_switch() {
        /// A policy that names a member the offer never listed.
        #[derive(Debug)]
        struct ChoosesUnoffered;

        #[async_trait]
        impl RotationPolicy for ChoosesUnoffered {
            async fn decide(&self, _request: &RotationRequest) -> RotationDecision {
                RotationDecision::Switch { position: 7 }
            }
        }

        let pool = pool(2);
        let inner = ScriptedProvider::new(1);
        let provider = PooledProvider::new(
            Arc::clone(&inner) as Arc<dyn Provider>,
            pool.clone(),
            Arc::new(ChoosesUnoffered),
            Arc::new(FixedClock),
        );

        let error = expect_exhausted(provider.stream(request(), ctx()).await);
        assert_eq!(error.kind, ProviderErrorKind::LimitExhausted);
        pool.read(|pool| assert_eq!(pool.active_position(), 0));
    }

    #[tokio::test]
    async fn the_interactive_policy_drives_a_replay_end_to_end() {
        let (policy, mut requests) = InteractiveRotation::new(1);
        let surface = tokio::spawn(async move {
            let prompt = requests.recv().await.expect("an offer");
            assert_eq!(prompt.request().trigger, RotationTrigger::Exhausted);
            assert_eq!(prompt.request().outgoing.label, "env:ACCOUNT_0");
            assert_eq!(prompt.request().outgoing_resets_at_ms, Some(RESET));
            let position = prompt.request().eligible[0].position;
            prompt.switch_to(position);
        });

        let pool = pool(2);
        let inner = ScriptedProvider::new(1);
        let provider = PooledProvider::new(
            Arc::clone(&inner) as Arc<dyn Provider>,
            pool.clone(),
            Arc::new(policy),
            Arc::new(FixedClock),
        );

        let stream = provider.stream(request(), ctx()).await.expect("a replay");
        surface.await.expect("the surface finished");
        collect(stream).await;

        assert_eq!(inner.attempts.load(Ordering::SeqCst), 2);
        pool.read(|pool| assert_eq!(pool.active_position(), 1));
    }

    #[tokio::test]
    async fn the_threshold_switches_before_the_attempt_is_spent() {
        let pool = pool_with_threshold(90);
        pool.write(|pool| {
            let mut snapshot = RateLimitSnapshot::new();
            snapshot.push(RateLimitWindow {
                used_percent: Some(93.0),
                ..RateLimitWindow::new("primary")
            });
            pool.record_snapshot(0, snapshot);
        });

        // Never fails: the point is that the switch happens *before* an
        // attempt is spent, not after one is lost.
        let inner = ScriptedProvider::new(0);
        let provider = PooledProvider::new(
            Arc::clone(&inner) as Arc<dyn Provider>,
            pool.clone(),
            Arc::new(AlwaysSwitch),
            Arc::new(FixedClock),
        );

        collect(provider.stream(request(), ctx()).await.expect("a stream")).await;

        assert_eq!(inner.attempts.load(Ordering::SeqCst), 1);
        pool.read(|pool| assert_eq!(pool.active_position(), 1));
    }

    #[tokio::test]
    async fn a_declined_threshold_is_asked_once_per_turn_not_once_per_attempt() {
        let pool = pool_with_threshold(90);
        pool.write(|pool| {
            let mut snapshot = RateLimitSnapshot::new();
            snapshot.push(RateLimitWindow {
                used_percent: Some(93.0),
                ..RateLimitWindow::new("primary")
            });
            pool.record_snapshot(0, snapshot);
        });

        let policy = Arc::new(CountingDecline {
            asked: AtomicUsize::new(0),
        });
        let provider = PooledProvider::new(
            ScriptedProvider::new(0) as Arc<dyn Provider>,
            pool.clone(),
            Arc::clone(&policy) as Arc<dyn RotationPolicy>,
            Arc::new(FixedClock),
        );

        collect(provider.stream(request(), ctx()).await.expect("a stream")).await;
        collect(provider.stream(request(), ctx()).await.expect("a stream")).await;
        // Two attempts, one question: declining at 93% must not be re-asked
        // on every attempt in the same turn.
        assert_eq!(policy.asked.load(Ordering::SeqCst), 1);
        pool.read(|pool| assert_eq!(pool.active_position(), 0));

        // A new turn asks again, because the account is still filling up.
        pool.write(CredentialPool::begin_turn);
        collect(provider.stream(request(), ctx()).await.expect("a stream")).await;
        assert_eq!(policy.asked.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn no_threshold_means_no_question_before_an_attempt() {
        let pool = pool(2);
        let policy = Arc::new(CountingDecline {
            asked: AtomicUsize::new(0),
        });
        let provider = PooledProvider::new(
            ScriptedProvider::new(0) as Arc<dyn Provider>,
            pool.clone(),
            Arc::clone(&policy) as Arc<dyn RotationPolicy>,
            Arc::new(FixedClock),
        );

        collect(provider.stream(request(), ctx()).await.expect("a stream")).await;
        assert_eq!(policy.asked.load(Ordering::SeqCst), 0);
    }

    #[derive(Debug)]
    struct FixedSecrets;

    #[async_trait]
    impl PoolSecrets for FixedSecrets {
        async fn resolve(&self, reference: &str) -> Result<Secret, ProviderCredentialError> {
            Ok(Secret::new(format!("secret-for-{reference}")))
        }
    }

    #[tokio::test]
    async fn the_credential_source_leases_whichever_member_is_active() {
        let pool = pool(2);
        let source = PoolCredentialSource::new(pool.clone(), Arc::new(FixedSecrets));
        let target = ProviderCredentialTarget::new("acme").expect("a target");

        let first = source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect("a lease");
        assert_eq!(first.secret().expose(), "secret-for-env:ACCOUNT_0");

        pool.write(|pool| pool.set_active(1));
        let second = source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect("a lease");
        // The switch takes effect on the next acquisition, with no adapter
        // rebuilt and no transport reopened.
        assert_eq!(second.secret().expose(), "secret-for-env:ACCOUNT_1");
        assert_ne!(first.revision(), second.revision());
    }

    #[tokio::test]
    async fn an_empty_pool_has_no_credential_to_lease() {
        let source = PoolCredentialSource::new(pool(0), Arc::new(FixedSecrets));
        let target = ProviderCredentialTarget::new("acme").expect("a target");

        let error = source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect_err("nothing to lease");
        assert_eq!(error, ProviderCredentialError::Unavailable);
    }

    #[tokio::test]
    async fn acquisition_observes_cancellation() {
        let source = PoolCredentialSource::new(pool(2), Arc::new(FixedSecrets));
        let target = ProviderCredentialTarget::new("acme").expect("a target");
        let cancel = Cancellation::new();
        cancel.cancel(CancelReason::UserRequested);

        let error = source
            .acquire(&target, 0, &cancel, Deadline::never())
            .await
            .expect_err("cancelled");
        assert_eq!(error, ProviderCredentialError::Cancelled);
    }
}
