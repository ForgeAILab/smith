use super::*;

pub(super) fn project_runtime_summary_persistence(
    capsule: &mut ResumeCapsule,
    persistence: RuntimeSummaryPersistence,
    summary_provider: Option<&str>,
    updated: agent_runtime_core::clock::Timestamp,
) -> Result<(), ResumeCapsuleError> {
    let runtime_state_unchanged = capsule
        .latest_summary_state_artifact
        .as_ref()
        .is_some_and(|current| current == &persistence.state_artifact);
    capsule.attach_runtime_summary_state_artifact(persistence.state_artifact)?;

    // Runtime's canonical snapshot can still contain the successful ordinary
    // summary that preceded a Smith handoff or a later failed idle attempt.
    // An identical protected-state artifact is exact evidence that Runtime
    // has not committed a newer summary, so retain the newer Smith projection
    // regardless of its purpose/outcome. A different state artifact is a
    // genuinely newer Runtime summary and replaces it below.
    if let Some(ordinary) = persistence.ordinary
        && !runtime_state_unchanged
    {
        let provider = summary_provider.ok_or(ResumeCapsuleError::InvalidSerializedForm)?;
        capsule.record_ordinary_summary(
            provider,
            ordinary.model,
            ordinary.revision,
            updated,
            ordinary.body,
            ordinary.coverage,
        )?;
        if let Some(summary) = capsule.semantic_summary.as_mut() {
            summary.provenance.usage = ordinary.usage;
        }
        let artifact_id = ordinary.artifact.id.to_string();
        capsule.attach_summary_artifact(ordinary.artifact.clone())?;
        if capsule.exact_state.artifacts.len() < MAX_ARTIFACTS
            && !capsule
                .exact_state
                .artifacts
                .iter()
                .any(|artifact| artifact.artifact == artifact_id)
        {
            capsule.exact_state.artifacts.push(ArtifactProjection {
                artifact: artifact_id,
                digest: Some(Fingerprint::of(ordinary.artifact.digest.hex.as_bytes())),
            });
        }
    }
    Ok(())
}

impl RedactingSessionStore {
    pub(super) fn new(
        inner: FileSessionStore,
        redactor: DefaultRedactor,
        reasoning: PersistedReasoningOverride,
        resume_capsule: Option<Arc<ResumeCapsuleSlot>>,
        artifact_store: Option<Arc<dyn ArtifactStore>>,
        summary_provider: Option<String>,
    ) -> Self {
        Self {
            inner: Arc::new(inner),
            writes: Arc::new(tokio::sync::Mutex::new(false)),
            #[cfg(test)]
            save_pause: Arc::new(Mutex::new(None)),
            redactor,
            reasoning,
            resume_capsule,
            artifact_store,
            summary_provider,
        }
    }

    /// Drains accepted writes and rejects later saves because Runtime
    /// delegation watchers can outlive the session shutdown future.
    pub(super) async fn close(&self) {
        *self.writes.lock().await = true;
    }

    /// Copies the latest Sensitive Runtime summary extension into an
    /// owner-authorized artifact. Smith's ordinary JSON snapshot intentionally
    /// drops this namespace; the capsule keeps only the protected reference so
    /// a later host can hand the exact state back to Runtime.
    async fn persist_runtime_summary_state(
        &self,
        snapshot: &SessionSnapshot,
    ) -> Option<RuntimeSummaryPersistence> {
        let capsule = self.resume_capsule.as_ref()?;
        if snapshot.id != capsule.snapshot().session_id {
            return None;
        }
        let state = snapshot
            .extension_state
            .get(SEMANTIC_SUMMARY_COMPONENT_ID)?;
        if state.sensitivity != SessionStateSensitivity::Sensitive {
            return None;
        }
        let mut summary = protected_semantic_summary_from_state(state, UsageDelta::new()).ok()?;
        let summary_usage = snapshot
            .usage
            .records()
            .iter()
            .rev()
            .find(|record| {
                record.source == UsageSource::SemanticSummary
                    && record.provenance.purpose.as_deref() == Some(summary.purpose.as_str())
            })
            .map(|record| record.delta.clone())
            .unwrap_or_default();
        summary.usage = summary_usage.clone();
        if summary.source_artifact.provenance.session != snapshot.id {
            return None;
        }
        let bytes = serde_json::to_vec(state).ok()?;
        if bytes.is_empty() || bytes.len() > MAX_SERIALIZED_CAPSULE_BYTES {
            return None;
        }
        let artifacts = self.artifact_store.as_ref()?;
        let idempotency_key = Fingerprint::of(&bytes).as_str().to_owned();
        let state_artifact = artifacts
            .put(ArtifactWrite {
                bytes,
                media_type: RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE.to_owned(),
                sensitivity: ArtifactSensitivity::Sensitive,
                retention: ArtifactRetention::Session,
                provenance: ArtifactProvenance::new(
                    snapshot.id.clone(),
                    RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE,
                ),
                idempotency_key,
            })
            .await
            .ok()?;
        if state_artifact.provenance.session != snapshot.id
            || state_artifact.provenance.purpose != RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE
            || state_artifact.media_type != RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE
            || state_artifact.byte_length == 0
            || state_artifact.byte_length > MAX_SERIALIZED_CAPSULE_BYTES as u64
        {
            return None;
        }

        let ordinary = if summary.purpose == SEMANTIC_SUMMARY_PURPOSE {
            let body = summary.body.as_str();
            if body.is_empty() || body.len() > MAX_SUMMARY_BYTES {
                None
            } else {
                let artifact = artifacts
                    .put(ArtifactWrite {
                        bytes: body.as_bytes().to_vec(),
                        media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
                        sensitivity: summary.source_artifact.sensitivity,
                        retention: ArtifactRetention::Session,
                        provenance: ArtifactProvenance::new(
                            snapshot.id.clone(),
                            RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                        ),
                        idempotency_key: summary.summary_revision.as_str().to_owned(),
                    })
                    .await
                    .ok();
                artifact.and_then(|artifact| {
                    (artifact.provenance.session == snapshot.id
                        && artifact.provenance.purpose == RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE
                        && artifact.media_type == RESUME_SUMMARY_MEDIA_TYPE
                        && artifact.byte_length > 0
                        && artifact.byte_length <= MAX_SUMMARY_BYTES as u64)
                        .then(|| OrdinarySummaryPersistence {
                            model: summary.model_id,
                            revision: summary.summary_revision,
                            body: body.to_owned(),
                            usage: summary_usage_projection(&summary_usage),
                            artifact,
                            coverage: vec![SummaryCoverage::new(
                                "canonical_history",
                                0,
                                summary.omit_prefix as u64,
                            )],
                        })
                })
            }
        } else {
            None
        };
        Some(RuntimeSummaryPersistence {
            state_artifact,
            ordinary,
        })
    }
}

fn summary_usage_projection(usage: &UsageDelta) -> SummaryUsage {
    SummaryUsage {
        input_uncached: usage.get(CounterKind::InputUncached),
        input_cached: usage.get(CounterKind::InputCached),
        cache_write: usage.get(CounterKind::CacheWrite),
        output: usage.get(CounterKind::Output),
        reasoning: usage.get(CounterKind::Reasoning),
        cost_micro_usd: None,
        cost_is_estimate: false,
    }
}

#[async_trait]
impl SessionStore for RedactingSessionStore {
    async fn load(&self, id: &SessionId) -> Result<Option<SessionSnapshot>, RuntimeError> {
        // Capsule recovery is deliberately host-ordered after Runtime startup
        // has loaded canonical and protected state.  Loading must remain a
        // pure store operation here: mutating the slot would allow this call
        // to overwrite a cold-resumed projection during start_session.
        self.inner.load(id).await
    }

    async fn save(&self, snapshot: &SessionSnapshot) -> Result<(), RuntimeError> {
        // Keep the gate in an owned task: aborting a controller must not release
        // it while Tokio's blocking filesystem work can still rename a file.
        let store = self.clone();
        let snapshot = snapshot.clone();
        tokio::spawn(async move {
            let closed = store.writes.lock().await;
            if *closed {
                return Err(RuntimeError::conflict("the Smith snapshot store is closed"));
            }
            #[cfg(test)]
            {
                let pause = store
                    .save_pause
                    .lock()
                    .expect("save pause lock poisoned")
                    .take();
                if let Some(pause) = pause {
                    pause.started.notify_one();
                    pause.release.notified().await;
                }
            }
            store.save_open(&snapshot).await
        })
        .await
        .map_err(|error| RuntimeError::internal(format!("snapshot writer failed: {error}")))?
    }
}

impl RedactingSessionStore {
    async fn save_open(&self, snapshot: &SessionSnapshot) -> Result<(), RuntimeError> {
        let mut snapshot = snapshot.clone();
        if self.reasoning.is_empty() {
            snapshot.extension_state.remove(SESSION_STATE_NAMESPACE);
        } else {
            snapshot.extension_state.insert(
                SESSION_STATE_NAMESPACE.to_owned(),
                self.reasoning.versioned()?,
            );
        }
        let prepared_capsule = if let Some(slot) = &self.resume_capsule {
            // Protected artifact I/O happens before the atomic live-slot
            // projection, so a concurrent canonical event cannot be captured
            // in the rollback baseline and then erased by a failed save.
            let persistence = self.persist_runtime_summary_state(&snapshot).await;
            let projection = persistence.and_then(|persistence| {
                slot.try_update_atomic(|capsule| {
                    project_runtime_summary_persistence(
                        capsule,
                        persistence,
                        self.summary_provider.as_deref(),
                        snapshot.updated,
                    )
                })
                .ok()
            });
            let (prepared, state) = match slot.prepare_versioned_state(snapshot.updated) {
                Ok(prepared) => prepared,
                Err(error) => {
                    if let Some((previous, expected)) = projection {
                        let _ = slot.restore_if_current(&expected, previous);
                    }
                    return Err(RuntimeError::conflict(error.to_string()));
                }
            };
            snapshot
                .extension_state
                .insert(RESUME_CAPSULE_STATE_NAMESPACE.to_owned(), state);
            Some((slot, prepared, projection))
        } else {
            None
        };
        let mut value = match serde_json::to_value(&snapshot) {
            Ok(value) => value,
            Err(error) => {
                if let Some((slot, _, Some((previous, expected)))) = prepared_capsule.as_ref() {
                    let _ = slot.restore_if_current(expected, previous.clone());
                }
                return Err(RuntimeError::new(
                    ErrorKind::Serialization,
                    format!(
                        "session `{}` could not be prepared for redaction: {error}",
                        snapshot.id
                    ),
                ));
            }
        };
        self.redactor.redact(&mut value);
        let redacted = match serde_json::from_value(value) {
            Ok(redacted) => redacted,
            Err(error) => {
                if let Some((slot, _, Some((previous, expected)))) = prepared_capsule.as_ref() {
                    let _ = slot.restore_if_current(expected, previous.clone());
                }
                return Err(RuntimeError::new(
                    ErrorKind::Serialization,
                    format!(
                        "session `{}` could not be restored after redaction: {error}",
                        snapshot.id
                    ),
                ));
            }
        };
        let result = self.inner.save(&redacted).await;
        if let Some((slot, prepared, projection)) = prepared_capsule {
            if result.is_ok() {
                let _ = slot.commit_persisted(&prepared);
            } else if let Some((previous, expected)) = projection {
                let _ = slot.restore_if_current(&expected, previous);
            }
        }
        result
    }
}
