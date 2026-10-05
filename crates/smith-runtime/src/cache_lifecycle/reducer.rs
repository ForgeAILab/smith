use super::*;

/// The reducer's bounded effect, useful to status projections without adding
/// a second canonical event vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheLifecycleEffect {
    /// Event was already reduced or did not carry an exact identity.
    Ignored,
    /// Structural planning facts changed.
    StructuralChanged,
    /// A lease was created for a new exact identity.
    LeaseCreated,
    /// An existing exact lease changed.
    LeaseUpdated,
    /// A prior identity was retired and a new one became current.
    IdentityRetired,
    /// Synthetic work for an identity was suspended.
    Suspended,
    /// Actual synthetic usage was attributed.
    UsageAttributed,
    /// Lifecycle shutdown suspended all leases.
    Shutdown,
}

/// Live/replay-equivalent Smith cache lifecycle projection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheLifecycleReducer {
    /// All current and historical exact leases.
    pub leases: Vec<CacheLease>,
    /// Exact current identity, when a plan has been installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_identity: Option<CacheIdentity>,
    /// Structural planning facts.
    pub structural: StructuralCacheProjection,
    /// Lifecycle has begun shutdown.
    #[serde(default)]
    pub shutdown: bool,
    /// Highest canonical event sequence already reduced. Persisting this
    /// watermark keeps replay idempotent across process restarts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_sequence: Option<u64>,
}

impl CacheLifecycleReducer {
    /// Restores an exact persisted identity only as a cold comparison
    /// baseline. Provider warmth, guarantees, and synthetic authority never
    /// survive a process boundary.
    pub fn restore_cold_identity(
        &mut self,
        identity: CacheIdentity,
        event_sequence: u64,
        at: Timestamp,
    ) {
        self.current_identity = Some(identity.clone());
        let mut lease = CacheLease::from_plan(identity, true, true);
        lease.cold_resume(at);
        self.push_lease(lease);
        self.last_event_sequence = Some(event_sequence);
    }

    /// Installs one exact Runtime plan and retires any incompatible current
    /// lease without transferring evidence or maintenance budget.
    pub fn install_plan(
        &mut self,
        identity: CacheIdentity,
        provider_supported: bool,
        has_comparable_predecessor: bool,
        preserved_prefix_tokens: u32,
        at: Timestamp,
    ) -> CacheLifecycleEffect {
        self.structural.provider_cache_supported = provider_supported;
        self.structural.has_comparable_predecessor = has_comparable_predecessor;
        self.structural.preserved_prefix_tokens = preserved_prefix_tokens;
        let changed = self
            .current_identity
            .as_ref()
            .is_some_and(|current| current != &identity);
        if changed
            && let Some(current) = self.current_identity.clone()
            && let Some(previous) = self.find_active_mut(&current)
        {
            previous.retire_for_identity_change(at);
        }
        self.current_identity = Some(identity.clone());
        let status = if !provider_supported {
            CacheLeaseStatus::Unsupported
        } else if has_comparable_predecessor {
            CacheLeaseStatus::Unknown
        } else {
            CacheLeaseStatus::Eligible
        };
        let existed = self.find_active_index(&identity).is_some();
        let index = match self.find_active_index(&identity) {
            Some(index) => index,
            None => self.push_lease(CacheLease::from_plan(
                identity.clone(),
                provider_supported,
                has_comparable_predecessor,
            )),
        };
        let should_reset_status = changed
            || !provider_supported
            || self.leases[index].status == CacheLeaseStatus::Unsupported;
        if should_reset_status {
            self.leases[index].status = status;
        }
        self.leases[index].structurally_preserved_prefix_tokens = preserved_prefix_tokens;
        if changed {
            CacheLifecycleEffect::IdentityRetired
        } else if existed {
            CacheLifecycleEffect::LeaseUpdated
        } else {
            CacheLifecycleEffect::LeaseCreated
        }
    }

    /// Returns the current exact lease.
    pub fn current(&self) -> Option<&CacheLease> {
        self.current_identity
            .as_ref()
            .and_then(|identity| self.find_active(identity))
    }

    /// Returns the current lease mutably for host-side lifecycle boundaries.
    pub fn current_mut(&mut self) -> Option<&mut CacheLease> {
        let identity = self.current_identity.clone()?;
        self.find_active_mut(&identity)
    }

    /// Returns an exact lease by identity, including historical state.
    pub fn lease(&self, identity: &CacheIdentity) -> Option<&CacheLease> {
        self.leases
            .iter()
            .rev()
            .find(|lease| &lease.identity == identity)
    }

    /// Starts a parked interval on the current lease.
    pub fn begin_parked_interval(&mut self, interval_id: impl Into<String>) {
        if let Some(identity) = self.current_identity.clone()
            && let Some(lease) = self.find_active_mut(&identity)
        {
            lease.begin_parked_interval(interval_id);
        }
    }

    /// Records meaningful real parent activity without creating cache
    /// evidence.
    pub fn record_parent_activity(&mut self, at: Timestamp) {
        if let Some(identity) = self.current_identity.clone()
            && let Some(lease) = self.find_active_mut(&identity)
        {
            lease.record_meaningful_activity(at);
        }
    }

    /// Records a real matching parent request, touching both clocks.
    pub fn record_parent_request(&mut self, identity: &CacheIdentity, at: Timestamp) {
        if let Some(lease) = self.find_active_mut(identity) {
            lease.record_real_parent_request(at);
        }
    }

    /// Marks cold resume for the current identity.  The old exact identity is
    /// retained only as a comparison baseline and no prewarm is authorized.
    pub fn cold_resume(&mut self, at: Timestamp) {
        if let Some(identity) = self.current_identity.clone()
            && let Some(lease) = self.find_active_mut(&identity)
        {
            lease.cold_resume(at);
        }
    }

    /// Retires the current identity after compaction; the next real request
    /// must establish any new provider cache naturally.
    pub fn retire_after_compaction(&mut self, at: Timestamp) {
        if let Some(identity) = self.current_identity.clone()
            && let Some(lease) = self.find_active_mut(&identity)
        {
            lease.retire_after_compaction(at);
        }
    }

    /// Freezes the projection at the host shutdown boundary even when the
    /// controller stops consuming events before Runtime emits its terminal
    /// `SessionShutdown` event.
    pub fn begin_shutdown(&mut self) {
        self.shutdown = true;
        for lease in &mut self.leases {
            lease.suspend(LeaseSuspensionReason::Shutdown, None, Timestamp::ZERO);
        }
    }

    /// Reduces one canonical Runtime event.  The same method is used for live
    /// streams and journal replay.
    pub fn apply(&mut self, envelope: &EventEnvelope) -> CacheLifecycleEffect {
        if self
            .last_event_sequence
            .is_some_and(|sequence| envelope.seq <= sequence)
        {
            return CacheLifecycleEffect::Ignored;
        }
        self.last_event_sequence = Some(envelope.seq);
        let at = envelope.timestamp;
        match &envelope.payload {
            RuntimeEvent::CachePlanChanged {
                cache_plan,
                preserved_prefix_tokens,
                invalidated_prefix_tokens,
                provider_cache_supported,
            } => {
                self.structural.cache_plan = Some(cache_plan.to_string());
                self.structural.preserved_prefix_tokens = *preserved_prefix_tokens;
                self.structural.invalidated_prefix_tokens = *invalidated_prefix_tokens;
                self.structural.provider_cache_supported = *provider_cache_supported;
                CacheLifecycleEffect::StructuralChanged
            }
            RuntimeEvent::CacheObservation {
                request,
                attempt,
                cache_identity: Some(identity),
                read_tokens,
                write_tokens,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let evidence = CacheAvailabilityEvidence {
                    source: CacheEvidenceSource::Stream,
                    kind: CacheEvidenceKind::Observation,
                    identity: identity.clone(),
                    request: request.clone(),
                    attempt: attempt.clone(),
                    operation: None,
                    ordering: 0,
                    read_tokens: *read_tokens,
                    write_tokens: *write_tokens,
                    refresh_cause: None,
                    guaranteed_until: None,
                    refreshed: None,
                    resource: None,
                    exists: None,
                };
                self.leases[index].apply_evidence(&evidence, at);
                CacheLifecycleEffect::LeaseUpdated
            }
            RuntimeEvent::CacheObservation { .. } => CacheLifecycleEffect::Ignored,
            RuntimeEvent::CacheStateChanged {
                cache_identity: Some(identity),
                state,
                expected_read_tokens,
                observed_read_tokens,
                observed_write_tokens,
                missed_tokens,
                ..
            } => {
                let Some(index) =
                    self.ensure_event_identity(identity, at, expected_read_tokens.is_some())
                else {
                    return CacheLifecycleEffect::Ignored;
                };
                self.leases[index].apply_cache_state(
                    *state,
                    *expected_read_tokens,
                    *observed_read_tokens,
                    *observed_write_tokens,
                    *missed_tokens,
                    at,
                );
                if matches!(
                    state,
                    CacheState::MissObserved | CacheState::Expired | CacheState::Suspended
                ) {
                    CacheLifecycleEffect::Suspended
                } else {
                    CacheLifecycleEffect::LeaseUpdated
                }
            }
            RuntimeEvent::CacheStateChanged { .. } => CacheLifecycleEffect::Ignored,
            RuntimeEvent::CacheAvailabilityEvidenceRecorded { evidence } => {
                let Some(index) = self.ensure_event_identity(&evidence.identity, at, false) else {
                    return CacheLifecycleEffect::Ignored;
                };
                self.leases[index].apply_evidence(evidence, at);
                if evidence.suspends_maintenance() {
                    CacheLifecycleEffect::Suspended
                } else {
                    CacheLifecycleEffect::LeaseUpdated
                }
            }
            RuntimeEvent::CacheOperationPrepared {
                operation,
                identity,
                purpose,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let lease = &mut self.leases[index];
                lease.last_operation_stage = Some(CacheOperationStage::Prepared);
                lease.last_operation_id = Some(operation.to_string());
                lease.last_operation_purpose = Some(*purpose);
                lease.last_operation_outcome = None;
                lease.last_operation_reason = None;
                lease.last_operation_metrics.clear();
                CacheLifecycleEffect::LeaseUpdated
            }
            RuntimeEvent::CacheOperationRejected {
                operation,
                identity,
                purpose,
                reason,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let lease = &mut self.leases[index];
                lease.last_operation_stage = Some(CacheOperationStage::Rejected);
                lease.last_operation_id = Some(operation.to_string());
                lease.last_operation_purpose = Some(*purpose);
                lease.last_operation_outcome = Some(CacheOperationOutcome::Rejected);
                lease.last_operation_reason = Some(*reason);
                lease.last_operation_metrics.clear();
                CacheLifecycleEffect::LeaseUpdated
            }
            RuntimeEvent::CacheOperationStarted {
                operation,
                identity,
                purpose,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let lease = &mut self.leases[index];
                lease.last_operation_stage = Some(CacheOperationStage::Started);
                lease.last_operation_id = Some(operation.to_string());
                lease.last_operation_purpose = Some(*purpose);
                lease.last_operation_outcome = None;
                lease.last_operation_reason = None;
                lease.last_operation_metrics.clear();
                lease.record_cache_touch(at);
                if is_parked_maintenance(*purpose) {
                    lease.record_maintenance_call(at);
                }
                CacheLifecycleEffect::LeaseUpdated
            }
            RuntimeEvent::CacheOperationSuspended {
                identity,
                operation,
                reason,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let lease = &mut self.leases[index];
                lease.last_operation_stage = Some(CacheOperationStage::Suspended);
                if let Some(operation) = operation {
                    lease.last_operation_id = Some(operation.to_string());
                }
                lease.last_operation_reason = Some(*reason);
                lease.suspend((*reason).into(), None, at);
                CacheLifecycleEffect::Suspended
            }
            RuntimeEvent::CacheOperationCompleted {
                operation,
                identity,
                purpose,
                outcome,
                reason,
                metrics,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let lease = &mut self.leases[index];
                lease.last_operation_stage = Some(CacheOperationStage::Completed);
                lease.last_operation_id = Some(operation.to_string());
                lease.last_operation_purpose = Some(*purpose);
                lease.last_operation_outcome = Some(*outcome);
                lease.last_operation_reason = *reason;
                lease.last_operation_metrics = metrics.clone();
                if *outcome == CacheOperationOutcome::Suspended {
                    lease.suspend(
                        reason
                            .map(LeaseSuspensionReason::from)
                            .unwrap_or(LeaseSuspensionReason::PolicyBoundary),
                        None,
                        at,
                    );
                    CacheLifecycleEffect::Suspended
                } else {
                    CacheLifecycleEffect::LeaseUpdated
                }
            }
            RuntimeEvent::Usage { record } => self.apply_usage(record),
            RuntimeEvent::ContextCompacted { .. } => {
                self.retire_after_compaction(at);
                CacheLifecycleEffect::IdentityRetired
            }
            RuntimeEvent::SessionShutdown => {
                self.begin_shutdown();
                CacheLifecycleEffect::Shutdown
            }
            _ => CacheLifecycleEffect::Ignored,
        }
    }

    /// Replays canonical events idempotently.
    pub fn replay<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        for event in events {
            self.apply(&event);
        }
    }

    fn apply_usage(&mut self, record: &UsageRecord) -> CacheLifecycleEffect {
        let Some(identity) = record.provenance.cache_identity.as_ref() else {
            return CacheLifecycleEffect::Ignored;
        };
        let Some(purpose) = record.provenance.attempt_purpose else {
            return CacheLifecycleEffect::Ignored;
        };
        if !is_stream_maintenance(purpose) {
            return CacheLifecycleEffect::Ignored;
        }
        let Some(lease) = self.find_active_mut(identity) else {
            return CacheLifecycleEffect::Ignored;
        };
        lease.record_maintenance_usage(
            record.delta.input_tokens(),
            record.delta.get(CounterKind::Output),
        );
        CacheLifecycleEffect::UsageAttributed
    }

    fn ensure_current(
        &mut self,
        identity: &CacheIdentity,
        at: Timestamp,
        has_comparable_predecessor: bool,
    ) -> usize {
        let changed = self
            .current_identity
            .as_ref()
            .is_some_and(|current| current != identity);
        if changed
            && let Some(current) = self.current_identity.clone()
            && let Some(previous) = self.find_active_mut(&current)
        {
            previous.retire_for_identity_change(at);
        }
        self.current_identity = Some(identity.clone());
        if let Some(index) = self.find_active_index(identity) {
            return index;
        }
        self.push_lease(CacheLease::from_plan(
            identity.clone(),
            true,
            has_comparable_predecessor,
        ))
    }

    /// Late events for a retired exact identity must not switch the current
    /// lease back to historical state.  A never-seen identity may become
    /// current when Runtime emits its first event before a separate plan
    /// projection arrives.
    fn ensure_event_identity(
        &mut self,
        identity: &CacheIdentity,
        at: Timestamp,
        has_comparable_predecessor: bool,
    ) -> Option<usize> {
        if self
            .current_identity
            .as_ref()
            .is_some_and(|current| current != identity)
            && self.leases.iter().any(|lease| &lease.identity == identity)
        {
            return None;
        }
        Some(self.ensure_current(identity, at, has_comparable_predecessor))
    }

    fn find_active(&self, identity: &CacheIdentity) -> Option<&CacheLease> {
        self.leases
            .iter()
            .rev()
            .find(|lease| !lease.retired && &lease.identity == identity)
    }

    fn find_active_mut(&mut self, identity: &CacheIdentity) -> Option<&mut CacheLease> {
        self.leases
            .iter_mut()
            .rev()
            .find(|lease| !lease.retired && &lease.identity == identity)
    }

    fn find_active_index(&self, identity: &CacheIdentity) -> Option<usize> {
        self.leases
            .iter()
            .enumerate()
            .rev()
            .find(|(_, lease)| !lease.retired && &lease.identity == identity)
            .map(|(index, _)| index)
    }

    fn push_lease(&mut self, lease: CacheLease) -> usize {
        if self.leases.len() >= MAX_CACHE_LEASES {
            let evict = self
                .leases
                .iter()
                .position(|candidate| candidate.retired)
                .unwrap_or(0);
            self.leases.remove(evict);
        }
        self.leases.push(lease);
        self.leases.len() - 1
    }
}

fn is_stream_maintenance(purpose: ProviderAttemptPurpose) -> bool {
    matches!(
        purpose,
        ProviderAttemptPurpose::CacheKeepalive
            | ProviderAttemptPurpose::CacheHandoffCheckpoint
            | ProviderAttemptPurpose::IdleCompaction
    )
}

fn is_parked_maintenance(purpose: ProviderAttemptPurpose) -> bool {
    matches!(
        purpose,
        ProviderAttemptPurpose::CacheKeepalive | ProviderAttemptPurpose::CacheHandoffCheckpoint
    )
}
