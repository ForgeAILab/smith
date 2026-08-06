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
use std::sync::atomic::{AtomicU64, Ordering};
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
    ProviderCredentialTarget, StaticProviderCredentialSource,
};
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

/// Builds the credential source that authorizes one pool member.
///
/// A member is a *source*, not a secret: an API-key member wraps its resolved
/// value in a static source, while a browser-login member gets a renewable
/// source that refreshes and persists to that member's own reference — which
/// is what keeps a pooled account's token from going stale while another
/// account serves the session. Kept behind a trait so rotation can be tested
/// without a keychain: production implementations open the platform credential
/// service, which is blocking, prompting, and unavailable in CI.
#[async_trait]
pub trait PoolMemberSources: Send + Sync + std::fmt::Debug {
    /// Builds the source leasing `reference`, declared at `position`.
    async fn build(
        &self,
        position: usize,
        reference: &str,
    ) -> Result<Arc<dyn ProviderCredentialSource>, ProviderCredentialError>;
}

/// Member sources for providers that authenticate with opaque static secrets.
#[derive(Debug)]
pub struct StaticMemberSources {
    resolver: CredentialResolver,
}

impl StaticMemberSources {
    /// Resolves members against `resolver`.
    pub fn new(resolver: CredentialResolver) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl PoolMemberSources for StaticMemberSources {
    async fn build(
        &self,
        _position: usize,
        reference: &str,
    ) -> Result<Arc<dyn ProviderCredentialSource>, ProviderCredentialError> {
        let reference =
            CredentialRef::parse(reference).map_err(|_| ProviderCredentialError::Unavailable)?;
        let resolver = self.resolver.clone();
        // The credential service is blocking and may prompt, so it never runs
        // on an executor thread.
        let secret = tokio::task::spawn_blocking(move || resolver.resolve_blocking(&reference))
            .await
            .map_err(|_| ProviderCredentialError::RefreshFailed)?
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        Ok(Arc::new(StaticProviderCredentialSource::new(secret)))
    }
}

/// Leases whichever pool member is active, through that member's own source.
///
/// Member sources are cached after their first construction, so switching back
/// to an account does not re-open the credential service and re-prompt. The
/// cache holds only members the session actually used: an account that is
/// never selected is never read.
#[derive(Debug)]
pub struct PoolCredentialSource {
    pool: SharedPool,
    members: Arc<dyn PoolMemberSources>,
    sources: Mutex<BTreeMap<usize, Arc<dyn ProviderCredentialSource>>>,
    /// Position → the composed revision last leased and the member revision
    /// beneath it, so an invalidation can be routed back to the member that
    /// issued the rejected lease.
    leases: Mutex<BTreeMap<usize, (ProviderCredentialRevision, ProviderCredentialRevision)>>,
    /// Distinguishes successive leases of one position, so a rejection of a
    /// lease that predates a member's renewal is stale rather than a second
    /// forced refresh of a token that was already replaced.
    serial: AtomicU64,
}

impl PoolCredentialSource {
    /// Leases members of `pool`, building their sources through `members`.
    pub fn new(pool: SharedPool, members: Arc<dyn PoolMemberSources>) -> Self {
        Self {
            pool,
            members,
            sources: Mutex::new(BTreeMap::new()),
            leases: Mutex::new(BTreeMap::new()),
            serial: AtomicU64::new(0),
        }
    }

    /// Pre-populates the member source for `position`.
    ///
    /// The active member's secret is already resolved by the time the adapter
    /// is built, and building the member again would open the credential
    /// service — and on some platforms prompt — for a value in hand.
    pub fn seed(&self, position: usize, source: Arc<dyn ProviderCredentialSource>) {
        self.sources
            .lock()
            .expect("member sources poisoned")
            .insert(position, source);
    }

    fn cached(&self, position: usize) -> Option<Arc<dyn ProviderCredentialSource>> {
        self.sources
            .lock()
            .expect("member sources poisoned")
            .get(&position)
            .cloned()
    }
}

#[async_trait]
impl ProviderCredentialSource for PoolCredentialSource {
    async fn acquire(
        &self,
        target: &ProviderCredentialTarget,
        minimum_validity_ms: u64,
        cancel: &Cancellation,
        deadline: Deadline,
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

        let source = match self.cached(position) {
            Some(source) => source,
            None => {
                let source = self.members.build(position, &reference).await?;
                self.sources
                    .lock()
                    .expect("member sources poisoned")
                    .insert(position, source.clone());
                source
            }
        };

        let lease = source
            .acquire(target, minimum_validity_ms, cancel, deadline)
            .await?;
        // The composed revision names the pool position and a serial, so a
        // rejection can be routed to the member that issued the lease — and
        // only to the exact lease it issued. It carries no secret and no
        // account identity beyond the position the user can already see.
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        let composed = ProviderCredentialRevision::new(format!("pool-{position}-{serial}"))?;
        self.leases
            .lock()
            .expect("lease records poisoned")
            .insert(position, (composed.clone(), lease.revision().clone()));
        let relabeled = match lease.expires_at() {
            Some(expires_at) => {
                ProviderCredentialLease::expiring(lease.secret().clone(), expires_at, composed)
            }
            None => ProviderCredentialLease::non_expiring(lease.secret().clone(), composed),
        };
        Ok(match lease.account() {
            Some(account) => relabeled.with_account(account),
            None => relabeled,
        })
    }

    async fn invalidate(
        &self,
        target: &ProviderCredentialTarget,
        rejected_revision: &ProviderCredentialRevision,
        rejection: ProviderAuthRejection,
        cancel: &Cancellation,
        deadline: Deadline,
    ) -> Result<CredentialInvalidation, ProviderCredentialError> {
        if cancel.is_cancelled() {
            return Err(ProviderCredentialError::Cancelled);
        }
        let record = {
            let leases = self.leases.lock().expect("lease records poisoned");
            leases
                .iter()
                .find(|(_, (composed, _))| composed == rejected_revision)
                .map(|(position, (_, inner))| (*position, inner.clone()))
        };
        // A rejection naming a lease this pool no longer tracks was already
        // superseded — by a renewal, or by another attempt on the same member.
        let Some((position, inner)) = record else {
            return Ok(CredentialInvalidation::StaleRevision);
        };
        let Some(source) = self.cached(position) else {
            return Ok(CredentialInvalidation::StaleRevision);
        };
        let verdict = source
            .invalidate(target, &inner, rejection, cancel, deadline)
            .await?;
        if verdict == CredentialInvalidation::NoReplacement {
            // The stored value may have been rotated by whatever tool owns it,
            // so the member is dropped and the next acquisition re-reads it.
            // Whether a *replacement* exists is not something a static
            // reference can promise, so the verdict passes through rather than
            // inviting an unbounded renewal loop.
            self.sources
                .lock()
                .expect("member sources poisoned")
                .remove(&position);
            self.leases
                .lock()
                .expect("lease records poisoned")
                .remove(&position);
        }
        Ok(verdict)
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
    use agent_runtime_core::store::Secret;
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
                return Err(
                    ProviderError::new(ProviderErrorKind::LimitExhausted, "spent")
                        .limit_resets_at(RESET),
                );
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
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop
            }
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
            assert_eq!(
                pool.active_position(),
                0,
                "no switch after an accepted stream"
            );
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

    /// Builds a static member per reference, counting constructions so a test
    /// can tell a cached member from a rebuilt one.
    #[derive(Debug, Default)]
    struct FixedMemberSources {
        builds: AtomicUsize,
    }

    #[async_trait]
    impl PoolMemberSources for FixedMemberSources {
        async fn build(
            &self,
            _position: usize,
            reference: &str,
        ) -> Result<Arc<dyn ProviderCredentialSource>, ProviderCredentialError> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(StaticProviderCredentialSource::new(Secret::new(
                format!("secret-for-{reference}"),
            ))))
        }
    }

    #[tokio::test]
    async fn the_credential_source_leases_whichever_member_is_active() {
        let pool = pool(2);
        let source =
            PoolCredentialSource::new(pool.clone(), Arc::new(FixedMemberSources::default()));
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

    /// A rejection routes to the member that issued the lease, and only for
    /// the exact lease it issued. A static member cannot renew, so it is
    /// dropped and rebuilt on the next acquisition — the stored value may have
    /// been rotated by whatever tool owns it.
    #[tokio::test]
    async fn a_rejection_rebuilds_the_member_that_issued_the_lease() {
        let members = Arc::new(FixedMemberSources::default());
        let source = PoolCredentialSource::new(pool(2), members.clone());
        let target = ProviderCredentialTarget::new("acme").expect("a target");

        let lease = source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect("a lease");
        assert_eq!(members.builds.load(Ordering::SeqCst), 1);

        let verdict = source
            .invalidate(
                &target,
                lease.revision(),
                ProviderAuthRejection::Unauthorized,
                &Cancellation::new(),
                Deadline::never(),
            )
            .await
            .expect("a verdict");
        assert_eq!(verdict, CredentialInvalidation::NoReplacement);

        source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect("a lease");
        assert_eq!(members.builds.load(Ordering::SeqCst), 2);
    }

    /// A rejection naming a lease the pool no longer tracks is stale: acting
    /// on it would punish a member for an attempt that was already superseded.
    #[tokio::test]
    async fn a_superseded_lease_rejection_is_stale_and_keeps_the_member() {
        let members = Arc::new(FixedMemberSources::default());
        let source = PoolCredentialSource::new(pool(2), members.clone());
        let target = ProviderCredentialTarget::new("acme").expect("a target");

        let first = source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect("a lease");
        let _second = source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect("a lease");

        let verdict = source
            .invalidate(
                &target,
                first.revision(),
                ProviderAuthRejection::Unauthorized,
                &Cancellation::new(),
                Deadline::never(),
            )
            .await
            .expect("a verdict");
        assert_eq!(verdict, CredentialInvalidation::StaleRevision);

        // The member survived: the next acquisition reuses it.
        source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect("a lease");
        assert_eq!(members.builds.load(Ordering::SeqCst), 1);
    }

    /// A renewable member keeps its identity across the pool boundary: the
    /// lease's expiry and account pass through, an invalidation that can be
    /// recovered by a refresh reports so, and the member is kept — dropping it
    /// would discard the very state that can renew.
    #[tokio::test]
    async fn a_renewable_member_passes_expiry_account_and_recovery_through() {
        #[derive(Debug, Default)]
        struct RenewableMember {
            invalidations: AtomicUsize,
        }

        #[async_trait]
        impl ProviderCredentialSource for RenewableMember {
            async fn acquire(
                &self,
                _target: &ProviderCredentialTarget,
                _minimum_validity_ms: u64,
                _cancel: &Cancellation,
                _deadline: Deadline,
            ) -> Result<ProviderCredentialLease, ProviderCredentialError> {
                Ok(ProviderCredentialLease::expiring(
                    Secret::new("access".to_owned()),
                    Timestamp::ZERO.plus_millis(RESET),
                    ProviderCredentialRevision::new("member-v1")?,
                )
                .with_account("acct_member"))
            }

            async fn invalidate(
                &self,
                _target: &ProviderCredentialTarget,
                rejected_revision: &ProviderCredentialRevision,
                _rejection: ProviderAuthRejection,
                _cancel: &Cancellation,
                _deadline: Deadline,
            ) -> Result<CredentialInvalidation, ProviderCredentialError> {
                assert_eq!(
                    rejected_revision,
                    &ProviderCredentialRevision::new("member-v1")?,
                    "the pool must hand the member its own revision back"
                );
                self.invalidations.fetch_add(1, Ordering::SeqCst);
                Ok(CredentialInvalidation::ReplacementPossible)
            }
        }

        #[derive(Debug, Default)]
        struct RenewableMembers {
            member: Arc<RenewableMember>,
        }

        #[async_trait]
        impl PoolMemberSources for RenewableMembers {
            async fn build(
                &self,
                _position: usize,
                _reference: &str,
            ) -> Result<Arc<dyn ProviderCredentialSource>, ProviderCredentialError> {
                Ok(self.member.clone())
            }
        }

        let members = Arc::new(RenewableMembers::default());
        let member = members.member.clone();
        let source = PoolCredentialSource::new(pool(2), members);
        let target = ProviderCredentialTarget::new("acme").expect("a target");

        let lease = source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect("a lease");
        assert_eq!(lease.expires_at(), Some(Timestamp::ZERO.plus_millis(RESET)));
        assert_eq!(lease.account(), Some("acct_member"));

        let verdict = source
            .invalidate(
                &target,
                lease.revision(),
                ProviderAuthRejection::Unauthorized,
                &Cancellation::new(),
                Deadline::never(),
            )
            .await
            .expect("a verdict");
        assert_eq!(verdict, CredentialInvalidation::ReplacementPossible);
        assert_eq!(member.invalidations.load(Ordering::SeqCst), 1);

        // Still leasable through the same member: it was not dropped.
        source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect("a lease");
    }

    #[tokio::test]
    async fn an_empty_pool_has_no_credential_to_lease() {
        let source = PoolCredentialSource::new(pool(0), Arc::new(FixedMemberSources::default()));
        let target = ProviderCredentialTarget::new("acme").expect("a target");

        let error = source
            .acquire(&target, 0, &Cancellation::new(), Deadline::never())
            .await
            .expect_err("nothing to lease");
        assert_eq!(error, ProviderCredentialError::Unavailable);
    }

    #[tokio::test]
    async fn acquisition_observes_cancellation() {
        let source = PoolCredentialSource::new(pool(2), Arc::new(FixedMemberSources::default()));
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
