use super::*;

#[derive(Debug)]
struct ResumeCapsuleSlotState {
    capsule: ResumeCapsule,
    restored_source: Option<RecoverySource>,
}

/// Thread-safe live capsule projection shared by Runtime observers and the
/// existing session/checkpoint persistence adapters.
///
/// The slot applies the approved recovery precedence: the larger exact
/// watermark wins and protected state wins a tie. Semantic prose never
/// participates in that selection or authorizes work.
#[derive(Debug)]
pub struct ResumeCapsuleSlot {
    state: Mutex<ResumeCapsuleSlotState>,
}

impl ResumeCapsuleSlot {
    /// Creates an empty live slot for a root session.
    pub fn new(session: SessionId, created_at: Timestamp) -> Self {
        Self {
            state: Mutex::new(ResumeCapsuleSlotState {
                capsule: ResumeCapsule::new(session, created_at),
                restored_source: None,
            }),
        }
    }

    /// Returns the current exact live capsule without exposing protected
    /// summary text through debug or machine projections.
    pub fn snapshot(&self) -> ResumeCapsule {
        self.state
            .lock()
            .expect("resume capsule slot poisoned")
            .capsule
            .clone()
    }

    /// Mutates the live projection at one host-owned commit boundary.
    pub fn update<R>(&self, update: impl FnOnce(&mut ResumeCapsule) -> R) -> R {
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        update(&mut state.capsule)
    }

    /// Applies a fallible host projection as one atomic slot mutation.
    ///
    /// The callback edits a private candidate. If any validation step fails,
    /// the live capsule is left untouched. On success, the returned before and
    /// after images can be used with [`Self::restore_if_current`] to roll back
    /// a later persistence failure without erasing a concurrent newer event.
    pub(crate) fn try_update_atomic(
        &self,
        update: impl FnOnce(&mut ResumeCapsule) -> Result<(), ResumeCapsuleError>,
    ) -> Result<(ResumeCapsule, ResumeCapsule), ResumeCapsuleError> {
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        let previous = state.capsule.clone();
        let mut expected = previous.clone();
        update(&mut expected)?;
        state.capsule = expected.clone();
        Ok((previous, expected))
    }

    /// Encodes the redaction-safe capsule into the existing session extension
    /// state. Protected handoff text is stored separately as a sensitive
    /// session artifact and is skipped by this serialization.
    pub fn versioned_state(&self) -> Result<VersionedSessionState, ResumeCapsuleError> {
        Self::encode(&self.snapshot())
    }

    /// Prepares the current projection for one persistence attempt.
    ///
    /// The live slot can contain canonical state that has not reached a
    /// durable store yet.  Its `last_persisted_*` fields therefore cannot be
    /// stamped in `update_capsule`: doing so makes an unsuccessful save look
    /// successful to observers and recovery.  This method stamps a clone for
    /// the bytes being attempted; [`Self::commit_persisted`] publishes those
    /// markers only after the store confirms success.
    pub(crate) fn prepare_versioned_state(
        &self,
        persisted_at: Timestamp,
    ) -> Result<(ResumeCapsule, VersionedSessionState), ResumeCapsuleError> {
        let mut capsule = self.snapshot();
        capsule.last_persisted_watermark = capsule.exact_state.watermark;
        capsule.last_persisted_at = Some(persisted_at);
        let state = Self::encode(&capsule)?;
        Ok((capsule, state))
    }

    /// Publishes persistence markers after a store has accepted the prepared
    /// projection.  A concurrent event/summary update must not be marked as
    /// durable by an older save, so the commit is conditional on the live
    /// capsule matching the prepared bytes apart from its persistence fields.
    pub(crate) fn commit_persisted(&self, prepared: &ResumeCapsule) -> bool {
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        let mut comparable = state.capsule.clone();
        comparable.last_persisted_watermark = prepared.last_persisted_watermark;
        comparable.last_persisted_at = prepared.last_persisted_at;
        if comparable != *prepared {
            return false;
        }
        state.capsule.last_persisted_watermark = prepared.last_persisted_watermark;
        state.capsule.last_persisted_at = prepared.last_persisted_at;
        true
    }

    /// Rolls back one failed host-owned projection only when no newer update
    /// has entered the slot since the failed attempt began.  This protects a
    /// later canonical event from being erased by an older asynchronous save
    /// failure.
    pub(crate) fn restore_if_current(
        &self,
        expected: &ResumeCapsule,
        previous: ResumeCapsule,
    ) -> bool {
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        if state.capsule != *expected {
            return false;
        }
        state.capsule = previous;
        true
    }

    fn encode(capsule: &ResumeCapsule) -> Result<VersionedSessionState, ResumeCapsuleError> {
        capsule.validate()?;
        let value =
            serde_json::to_value(capsule).map_err(|_| ResumeCapsuleError::InvalidSerializedForm)?;
        Ok(VersionedSessionState {
            revision: RegistryRevision::new(RESUME_CAPSULE_STATE_REVISION),
            sensitivity: SessionStateSensitivity::RedactionSafe,
            value,
        })
    }

    /// Restores one canonical or protected candidate while enforcing exact
    /// watermark precedence. Returns whether the candidate became current.
    pub fn restore_versioned_state(
        &self,
        persisted: &VersionedSessionState,
        source: RecoverySource,
    ) -> Result<bool, ResumeCapsuleError> {
        if persisted.revision != RegistryRevision::new(RESUME_CAPSULE_STATE_REVISION) {
            return Err(ResumeCapsuleError::UnsupportedSchemaVersion);
        }
        let candidate = ResumeCapsule::from_json_value(persisted.value.clone())?;
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        if candidate.session_id != state.capsule.session_id {
            return Err(ResumeCapsuleError::InvalidSerializedForm);
        }
        let candidate_watermark = candidate.exact_state.watermark;
        let current_watermark = state.capsule.exact_state.watermark;
        let wins = state.restored_source.is_none()
            || candidate_watermark > current_watermark
            || (candidate_watermark == current_watermark
                && source == RecoverySource::ProtectedCheckpoint
                && state.restored_source != Some(RecoverySource::ProtectedCheckpoint));
        if !wins {
            return Ok(false);
        }
        let mut candidate = candidate;
        candidate.diagnostics = candidate.summary_conflicts(source);
        state.capsule = candidate;
        state.restored_source = Some(source);
        Ok(true)
    }

    /// Applies cold-process reconciliation once after persisted candidates
    /// have been selected.
    pub fn cold_resume(&self) -> ColdResumeResult {
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        let result = state.capsule.cold_resume();
        state.capsule = result.capsule.clone();
        result
    }
}

/// Restores a completed summary body from its protected, session-owned
/// artifact. The reference and returned page are verified before UTF-8 text
/// enters live memory; failures never fall back to untrusted prose.
pub async fn restore_summary_artifact(
    slot: &ResumeCapsuleSlot,
    store: &dyn ArtifactStore,
) -> Result<bool, ResumeCapsuleError> {
    let capsule = slot.snapshot();
    let Some(summary) = capsule.semantic_summary.as_ref() else {
        return Ok(false);
    };
    if summary.provenance.outcome != ResumeSummaryOutcome::Completed {
        return Ok(false);
    }
    let Some(reference) = summary.provenance.summary_artifact.clone() else {
        slot.update(ResumeCapsule::mark_summary_missing);
        return Ok(false);
    };
    if reference.provenance.session != capsule.session_id
        || reference.provenance.purpose != summary_artifact_purpose(summary.provenance.purpose)
        || reference.media_type != RESUME_SUMMARY_MEDIA_TYPE
        || reference.byte_length == 0
        || reference.byte_length > MAX_SUMMARY_BYTES as u64
    {
        slot.update(ResumeCapsule::mark_summary_missing);
        return Ok(false);
    }
    let request = ArtifactRead {
        session: capsule.session_id,
        id: reference.id.clone(),
        offset: 0,
        limit: u32::try_from(MAX_SUMMARY_BYTES)
            .expect("summary bound fits u32")
            .min(MAX_ARTIFACT_READ_BYTES),
    };
    let chunk = match store.read(request.clone()).await {
        Ok(chunk) => chunk,
        Err(_) => {
            slot.update(ResumeCapsule::mark_summary_missing);
            return Ok(false);
        }
    };
    if chunk.validate_for(&request).is_err()
        || chunk.reference != reference
        || chunk.next_offset.is_some()
    {
        slot.update(ResumeCapsule::mark_summary_missing);
        return Ok(false);
    }
    let body = match String::from_utf8(chunk.bytes) {
        Ok(body) => body,
        Err(_) => {
            slot.update(ResumeCapsule::mark_summary_missing);
            return Ok(false);
        }
    };
    if slot
        .update(|capsule| capsule.restore_summary_body(body))
        .is_err()
    {
        slot.update(ResumeCapsule::mark_summary_missing);
        return Ok(false);
    }
    Ok(true)
}

/// Reads and validates the latest protected Runtime semantic-summary state.
/// The returned value is still Sensitive extension state; callers must pass
/// it only to Agent Runtime's protected restore seam and never to redacted
/// status or ordinary JSON output.
pub async fn restore_runtime_summary_state(
    slot: &ResumeCapsuleSlot,
    store: &dyn ArtifactStore,
) -> Result<Option<VersionedSessionState>, ResumeCapsuleError> {
    let capsule = slot.snapshot();
    let Some(reference) = capsule.latest_summary_state_artifact.clone() else {
        return Ok(None);
    };
    if reference.provenance.session != capsule.session_id
        || reference.provenance.purpose != RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE
        || reference.media_type != RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE
        || reference.sensitivity != ArtifactSensitivity::Sensitive
        || reference.byte_length == 0
        || reference.byte_length > MAX_SERIALIZED_CAPSULE_BYTES as u64
    {
        return Ok(None);
    }
    let request = ArtifactRead {
        session: capsule.session_id,
        id: reference.id.clone(),
        offset: 0,
        limit: u32::try_from(MAX_SERIALIZED_CAPSULE_BYTES)
            .expect("capsule bound fits u32")
            .min(MAX_ARTIFACT_READ_BYTES),
    };
    let chunk = match store.read(request.clone()).await {
        Ok(chunk) => chunk,
        Err(_) => return Ok(None),
    };
    if chunk.validate_for(&request).is_err()
        || chunk.reference != reference
        || chunk.next_offset.is_some()
    {
        return Ok(None);
    }
    let state: VersionedSessionState = match serde_json::from_slice(&chunk.bytes) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };
    if state.sensitivity != SessionStateSensitivity::Sensitive {
        return Ok(None);
    }
    Ok(Some(state))
}
