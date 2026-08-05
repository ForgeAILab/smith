//! Single-flight renewable credential sources over stored token bundles.
//!
//! Every browser-login provider ends up holding the same thing: a bundle of
//! access token, refresh token, and expiry, persisted at one protected
//! credential reference, renewed ahead of expiry so a subscription behaves
//! like a key that never lapses. What differs per provider is only the bundle
//! wire format and the issuer call that renews it. This module owns everything
//! else — the single-flight refresh, the persist-before-lease ordering, the
//! revision series, and redaction registration — so a new login kind supplies
//! two small trait impls instead of a second copy of this machinery.

use std::fmt;
use std::future::pending;
use std::sync::Arc;
use std::time::Duration;

use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Clock, Deadline, SystemClock, Timestamp};
use agent_runtime_core::provider_credential::{
    CredentialInvalidation, ProviderAuthRejection, ProviderCredentialError,
    ProviderCredentialLease, ProviderCredentialRevision, ProviderCredentialSource,
    ProviderCredentialTarget,
};
use agent_runtime_core::store::Secret;
use async_trait::async_trait;
use smith_config::credential::{CredentialEnroller, CredentialRef, CredentialResolver};

use crate::journal::DefaultRedactor;
use crate::rotation::PoolMemberSources;

/// A stored, renewable login session for one provider.
///
/// The bundle owns its wire format: how it round-trips through protected
/// storage, which of its fields is the bearer, and when it wants renewing.
pub trait RenewableBundle: Clone + Send + Sync + 'static {
    /// The provider-specific bundle failure, kept for login-time callers;
    /// the credential source itself reports only fixed lease errors.
    type Error: Send + Sync + 'static;

    /// Parses a bundle from protected storage.
    fn from_secret(secret: &Secret) -> Result<Self, Self::Error>;

    /// Serializes the bundle for protected storage.
    fn to_secret(&self) -> Result<Secret, Self::Error>;

    /// The bearer alone, never the stored bundle.
    fn access_secret(&self) -> Secret;

    /// When the current access token stops being accepted.
    fn expires_at(&self) -> Timestamp;

    /// Whether a renewal is possible at all.
    ///
    /// A session issued without a refresh token simply ends; reporting that
    /// as a refresh failure would send the user hunting a network problem
    /// instead of signing in again.
    fn can_refresh(&self) -> bool {
        true
    }

    /// Whether the bundle wants renewing ahead of the caller's stated minimum.
    ///
    /// The caller asks for enough validity to *start* a call; a bundle may ask
    /// for enough to finish one, because a token accepted at request time and
    /// expiring mid-stream kills a turn that is already producing output.
    fn needs_refresh(&self, _now_ms: u64) -> bool {
        false
    }

    /// The account identity this bundle's tokens belong to, when the issuer
    /// binds one. It rides on every lease so an adapter whose wire protocol
    /// names the account never pairs one account's token with another's
    /// identity.
    fn account(&self) -> Option<String> {
        None
    }

    /// Registers every secret the bundle carries for output redaction.
    fn register_secrets(&self, redactor: &DefaultRedactor);

    /// Names the revision series this bundle's leases move through.
    fn revision_prefix() -> &'static str;
}

/// The renewal half of a provider's OAuth client, separated so refresh
/// behavior can be driven deterministically in tests without an issuer.
#[async_trait]
pub trait BundleRefresher<B: RenewableBundle>: Send + Sync {
    /// Exchanges the current bundle for a renewed one.
    async fn refresh(&self, bundle: &B, now_ms: u64) -> Result<B, B::Error>;
}

struct RenewableState<B> {
    bundle: B,
    revision_number: u64,
    revision: ProviderCredentialRevision,
    force_refresh: bool,
}

/// Single-flight renewable credential source backed by one stored bundle.
///
/// Renewals persist to the source's own credential reference before any lease
/// is handed out, so a crash mid-turn leaves the renewed session on disk
/// rather than a token the next process cannot renew.
pub struct RenewableCredentialSource<B: RenewableBundle> {
    target: ProviderCredentialTarget,
    reference: CredentialRef,
    enroller: CredentialEnroller,
    refresher: Arc<dyn BundleRefresher<B>>,
    redactor: Option<DefaultRedactor>,
    clock: Arc<dyn Clock>,
    state: tokio::sync::Mutex<RenewableState<B>>,
}

impl<B: RenewableBundle> fmt::Debug for RenewableCredentialSource<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenewableCredentialSource")
            .field("target", &self.target)
            .field("reference", &self.reference)
            .finish_non_exhaustive()
    }
}

impl<B: RenewableBundle> RenewableCredentialSource<B> {
    /// Builds a source over an already-parsed bundle.
    ///
    /// Injectable in every part, so refresh and storage behavior can be
    /// driven deterministically in tests.
    pub fn new(
        target: ProviderCredentialTarget,
        reference: CredentialRef,
        bundle: B,
        enroller: CredentialEnroller,
        refresher: Arc<dyn BundleRefresher<B>>,
        redactor: Option<DefaultRedactor>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        if let Some(redactor) = &redactor {
            bundle.register_secrets(redactor);
        }
        let revision = ProviderCredentialRevision::new(format!("{}-v1", B::revision_prefix()))
            .expect("static revision is valid");
        Self {
            target,
            reference,
            enroller,
            refresher,
            redactor,
            clock,
            state: tokio::sync::Mutex::new(RenewableState {
                bundle,
                revision_number: 1,
                revision,
                force_refresh: false,
            }),
        }
    }

    async fn refresh_locked(
        &self,
        state: &mut RenewableState<B>,
        cancel: &Cancellation,
        deadline: Deadline,
    ) -> Result<(), ProviderCredentialError> {
        if !state.bundle.can_refresh() {
            return Err(ProviderCredentialError::Unavailable);
        }
        let now_ms = self.clock.now().0;
        // Cloned so the in-flight request does not hold a borrow on the state
        // this function has to write back into.
        let current = state.bundle.clone();
        let operation = self.refresher.refresh(&current, now_ms);
        tokio::pin!(operation);
        let bundle = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ProviderCredentialError::Cancelled),
            _ = wait_for_deadline(deadline, self.clock.as_ref()) => {
                return Err(ProviderCredentialError::Timeout);
            }
            result = &mut operation => result.map_err(|_| ProviderCredentialError::RefreshFailed)?,
        };
        let serialized = bundle
            .to_secret()
            .map_err(|_| ProviderCredentialError::RefreshFailed)?;
        // Persisting before the lease is handed out means a crash mid-turn
        // leaves the renewed session on disk rather than a token the next
        // process cannot renew.
        let reference = self.reference.clone();
        let enroller = self.enroller.clone();
        let persist = tokio::task::spawn_blocking(move || enroller.enroll(&reference, &serialized));
        let receipt = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ProviderCredentialError::Cancelled),
            _ = wait_for_deadline(deadline, self.clock.as_ref()) => {
                return Err(ProviderCredentialError::Timeout);
            }
            result = persist => result
                .map_err(|_| ProviderCredentialError::RefreshFailed)?
                .map_err(|_| ProviderCredentialError::RefreshFailed)?,
        };
        drop(receipt);
        if let Some(redactor) = &self.redactor {
            bundle.register_secrets(redactor);
        }
        state.revision_number = state.revision_number.saturating_add(1);
        state.revision = ProviderCredentialRevision::new(format!(
            "{}-v{}",
            B::revision_prefix(),
            state.revision_number
        ))
        .map_err(|_| ProviderCredentialError::InvalidRevision)?;
        state.bundle = bundle;
        state.force_refresh = false;
        Ok(())
    }
}

#[async_trait]
impl<B: RenewableBundle> ProviderCredentialSource for RenewableCredentialSource<B> {
    async fn acquire(
        &self,
        target: &ProviderCredentialTarget,
        minimum_validity_ms: u64,
        cancel: &Cancellation,
        deadline: Deadline,
    ) -> Result<ProviderCredentialLease, ProviderCredentialError> {
        if target != &self.target {
            return Err(ProviderCredentialError::Unavailable);
        }
        if cancel.is_cancelled() {
            return Err(ProviderCredentialError::Cancelled);
        }
        let mut state = self.state.lock().await;
        // Two thresholds, and the wider one wins: the caller's stated minimum,
        // and whatever skew the bundle asks for itself.
        let now = self.clock.now();
        if state.force_refresh
            || state.bundle.needs_refresh(now.0)
            || state.bundle.expires_at() < now.plus_millis(minimum_validity_ms)
        {
            self.refresh_locked(&mut state, cancel, deadline).await?;
        }
        if state.bundle.expires_at() < self.clock.now().plus_millis(minimum_validity_ms) {
            return Err(ProviderCredentialError::InvalidLease);
        }
        let lease = ProviderCredentialLease::expiring(
            state.bundle.access_secret(),
            state.bundle.expires_at(),
            state.revision.clone(),
        );
        Ok(match state.bundle.account() {
            Some(account) => lease.with_account(account),
            None => lease,
        })
    }

    async fn invalidate(
        &self,
        target: &ProviderCredentialTarget,
        rejected_revision: &ProviderCredentialRevision,
        _rejection: ProviderAuthRejection,
        cancel: &Cancellation,
        _deadline: Deadline,
    ) -> Result<CredentialInvalidation, ProviderCredentialError> {
        if target != &self.target {
            return Err(ProviderCredentialError::Unavailable);
        }
        if cancel.is_cancelled() {
            return Err(ProviderCredentialError::Cancelled);
        }
        let mut state = self.state.lock().await;
        // A rejection naming a superseded revision is a request already in
        // flight when the renewal landed, not evidence the new token is bad.
        if &state.revision != rejected_revision {
            return Ok(CredentialInvalidation::StaleRevision);
        }
        if !state.bundle.can_refresh() {
            return Ok(CredentialInvalidation::NoReplacement);
        }
        state.force_refresh = true;
        Ok(CredentialInvalidation::ReplacementPossible)
    }
}

/// Pool member sources whose members are stored renewable login bundles.
///
/// Each member's source refreshes and persists to that member's own
/// reference, so an account that is not currently serving the session still
/// renews — instead of going stale — whenever it is next leased.
pub struct RenewableMemberSources<B: RenewableBundle> {
    resolver: CredentialResolver,
    target: ProviderCredentialTarget,
    refresher: Arc<dyn BundleRefresher<B>>,
    redactor: Option<DefaultRedactor>,
}

impl<B: RenewableBundle> RenewableMemberSources<B> {
    /// Builds members against `resolver`, renewing them through `refresher`.
    pub fn new(
        resolver: CredentialResolver,
        target: ProviderCredentialTarget,
        refresher: Arc<dyn BundleRefresher<B>>,
        redactor: Option<DefaultRedactor>,
    ) -> Self {
        Self {
            resolver,
            target,
            refresher,
            redactor,
        }
    }
}

impl<B: RenewableBundle> fmt::Debug for RenewableMemberSources<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenewableMemberSources")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<B: RenewableBundle> PoolMemberSources for RenewableMemberSources<B> {
    async fn build(
        &self,
        _position: usize,
        reference: &str,
    ) -> Result<Arc<dyn ProviderCredentialSource>, ProviderCredentialError> {
        let parsed =
            CredentialRef::parse(reference).map_err(|_| ProviderCredentialError::Unavailable)?;
        let resolver = self.resolver.clone();
        let resolve_reference = parsed.clone();
        // The credential service is blocking and may prompt, so it never runs
        // on an executor thread.
        let secret =
            tokio::task::spawn_blocking(move || resolver.resolve_blocking(&resolve_reference))
                .await
                .map_err(|_| ProviderCredentialError::RefreshFailed)?
                .map_err(|_| ProviderCredentialError::Unavailable)?;
        let bundle = B::from_secret(&secret).map_err(|_| ProviderCredentialError::Unavailable)?;
        Ok(Arc::new(RenewableCredentialSource::new(
            self.target.clone(),
            parsed,
            bundle,
            CredentialEnroller::new(),
            self.refresher.clone(),
            self.redactor.clone(),
            Arc::new(SystemClock),
        )))
    }
}

/// Resolves when `deadline` passes, or never for an unbounded deadline.
pub(crate) async fn wait_for_deadline(deadline: Deadline, clock: &dyn Clock) {
    match deadline.remaining_millis(clock) {
        Some(0) => {}
        Some(milliseconds) => tokio::time::sleep(Duration::from_millis(milliseconds)).await,
        None => pending::<()>().await,
    }
}
