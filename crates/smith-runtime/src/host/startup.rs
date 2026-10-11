use super::*;

/// Starts a standard Smith session through the one runtime factory.
///
/// The journal observer is attached to the builder through a deferred seam,
/// then opened only after provider/runtime preflight succeeds and before the
/// first session event is emitted. A bad provider configuration therefore
/// cannot leave an empty session journal behind.
pub async fn start(mut request: HostSessionRequest) -> Result<HostSession, HostSessionError> {
    if request.runtime.system_prompt.is_none() && request.runtime.project_instructions.is_none() {
        request.runtime.project_instructions =
            discover_project_instructions(&request.project_root)?;
    }
    let mut config = request.runtime.config.clone();
    let summary_provider = request.runtime.semantic_summary.as_ref().map(|summary| {
        summary
            .provider
            .clone()
            .unwrap_or_else(|| config.provider.name.value.clone())
    });
    let surface = request.runtime.surface;
    reject_project_granted_authority(&config, &request.project_root)?;
    reject_project_controlled_persistence(&config, &request.project_root)?;
    let persistence = config.persistence.enabled.value;
    let session_id = request.session_id.clone().unwrap_or_else(mint_session_id);
    let host_clock: Arc<dyn Clock> = request
        .runtime
        .clock
        .clone()
        .unwrap_or_else(|| Arc::new(SystemClock));
    request.runtime.clock = Some(host_clock.clone());
    let resume_capsule = config
        .context
        .cache
        .resume_capsule
        .value
        .then(|| Arc::new(ResumeCapsuleSlot::new(session_id.clone(), host_clock.now())));

    if request.session_id.is_some() && !persistence {
        return Err(HostSessionError::ResumeDisabled {
            session: session_id,
        });
    }

    let persistence_redactor = request
        .runtime
        .persistence_redactor
        .clone()
        .unwrap_or_default();
    // The standard factory also registers provider credentials with this
    // shared redactor, even when durable session persistence is disabled.
    request.runtime.persistence_redactor = Some(persistence_redactor.clone());

    let mut snapshot_store = None;
    let mut resume_snapshot_exists = false;
    let (paths, journal_slot, checkpoint_barrier, ring) = if persistence {
        let paths = paths(&config, &request.project_root)?;
        if request.runtime.artifact_store.is_none() {
            request.runtime.artifact_store = Some(Arc::new(SmithArtifactStore::new(paths.clone())));
        }
        let inner = FileSessionStore::new(paths.clone());
        // A saved effort a higher layer answered for this run, kept so the
        // run's own selection cannot erase the session's choice on save.
        let mut shadowed_effort = None;
        let mut shadowed_context_window = None;
        if request.session_id.is_some() {
            let snapshot = inner.load(&session_id).await?;
            resume_snapshot_exists = snapshot.is_some();
            // Prime the persistence projection before Runtime startup so a
            // recovered non-terminal checkpoint cannot save an empty capsule
            // while the final recovery selection is still pending.  This is
            // only a write-safety baseline; the authoritative candidate
            // selection is repeated after start_session below.
            if let (Some(slot), Some(persisted)) = (
                resume_capsule.as_ref(),
                snapshot.as_ref().and_then(|snapshot| {
                    snapshot.extension_state.get(RESUME_CAPSULE_STATE_NAMESPACE)
                }),
            ) {
                slot.restore_versioned_state(persisted, RecoverySource::CanonicalSnapshot)
                    .map_err(|error| RuntimeError::conflict(error.to_string()))?;
            }
            if let Some(state) = snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.extension_state.get(SESSION_STATE_NAMESPACE))
            {
                let restored = PersistedReasoningOverride::restore(state)?;
                restored.apply_with_context_window(
                    &mut config,
                    request.reasoning_reset_enabled,
                    request.reasoning_reset_effort || request.reasoning_effort_shadowed,
                    request.context_window_reset,
                    request.context_window_shadowed,
                );
                if request.reasoning_effort_shadowed && !request.reasoning_reset_effort {
                    shadowed_effort = restored.effort.clone();
                }
                if request.context_window_shadowed && !request.context_window_reset {
                    shadowed_context_window = restored.context_window.clone();
                }
                request.runtime.config = config.clone();
            }
        }
        let mut reasoning_override = PersistedReasoningOverride::from_config(&config);
        if reasoning_override.effort.is_none() {
            reasoning_override.effort = shadowed_effort;
        }
        if reasoning_override.context_window.is_none() {
            reasoning_override.context_window = shadowed_context_window;
        }
        let store = Arc::new(RedactingSessionStore::new(
            inner,
            persistence_redactor.clone(),
            reasoning_override,
            resume_capsule.clone(),
            request.runtime.artifact_store.clone(),
            summary_provider.clone(),
        ));
        snapshot_store = Some(store.clone());
        request.runtime.session_store = Some(store);
        if let Some(store) = request.runtime.checkpoint_store.take() {
            request.runtime.checkpoint_store =
                Some(with_resume_capsule(store, resume_capsule.clone()));
        }
        if let Some(setup) = request.runtime.checkpoint_setup.take() {
            request.runtime.checkpoint_setup =
                Some(setup.with_resume_capsule(resume_capsule.clone()));
        }
        if request.runtime.checkpoint_store.is_none() && request.runtime.checkpoint_setup.is_none()
        {
            let provider = match request.checkpoint_keys.clone() {
                Some(provider) => Some(provider),
                None => {
                    if let Some(key) = &config.persistence.checkpoint_key {
                        Some(Arc::new(
                            ConfiguredCheckpointKeyProvider::new(&key.value)
                                .map_err(RuntimeError::from)?,
                        ) as Arc<dyn CheckpointKeyProvider>)
                    } else if let Some(reference) = &config.persistence.checkpoint_key_credential {
                        let resolver = request.runtime.credentials.clone().ok_or_else(|| {
                            RuntimeError::config(
                                "a checkpoint key credential reference requires a credential resolver",
                            )
                        })?;
                        Some(Arc::new(
                            CredentialCheckpointKeyProvider::new(resolver, &reference.value)
                                .map_err(RuntimeError::from)?,
                        ) as Arc<dyn CheckpointKeyProvider>)
                    } else {
                        None
                    }
                }
            };
            request.runtime.checkpoint_setup = Some(
                match provider {
                    Some(provider) => SmithCheckpointSetup::with_provider(paths.clone(), provider),
                    None => SmithCheckpointSetup::platform(paths.clone()),
                }
                .with_resume_capsule(resume_capsule.clone()),
            );
        }

        let (slot, barrier, ring) = if config.persistence.journal_events.value {
            let slot = Arc::new(DeferredObserver::default());
            request.runtime.observers.push(slot.clone());
            let barrier = Arc::new(JournalCheckpointBarrier::default());
            request.runtime.checkpoint_barrier =
                Some(barrier.clone() as Arc<dyn CheckpointBarrier>);
            // Unlike the journal, the ring has no later resource to bind: it
            // is its own complete observer from the moment it is built, so it
            // attaches directly instead of through the deferred slot above.
            let ring = Arc::new(EventRing::default());
            request.runtime.observers.push(ring.clone());
            (Some(slot), Some(barrier), Some(ring))
        } else {
            (None, None, None)
        };
        (Some(paths), slot, barrier, ring)
    } else {
        (None, None, None, None)
    };

    let change_journal = paths
        .as_ref()
        .map(|paths| paths.changes(&session_id))
        .transpose()?;
    let changes = Arc::new(
        smith_tools::ChangeRecorder::new(change_journal).with_project_root(&request.project_root),
    );
    request.runtime.change_recorder = Some(changes.clone());
    request
        .runtime
        .observers
        .push(Arc::new(ChangeTurnObserver(changes.clone())));

    let harness = crate::harness::resolve(crate::harness::HarnessSpec::trusted(request.runtime))
        .map_err(FactoryError::from)?;
    let runtime = crate::factory::build(harness).await?;

    // Probe existence before creating the lifecycle lock file. The reads are
    // atomic and side-effect free; an arbitrary missing resume id must not
    // leave a user-state directory or lock artifact behind.
    let checkpoint_probe = if request.session_id.is_some() {
        match runtime.checkpoint_store() {
            Some(store) => store.load_latest(&session_id).await?,
            None => None,
        }
    } else {
        None
    };
    if request.session_id.is_some() && !resume_snapshot_exists && checkpoint_probe.is_none() {
        return Err(HostSessionError::SessionNotFound {
            session: session_id,
        });
    }

    let lifecycle_lease = match &paths {
        Some(paths) => {
            let path = paths.lifecycle_lock(&session_id)?;
            Some(try_acquire_private_lock(&path).await?)
        }
        None => None,
    };

    // The prior owner may have advanced after the existence probe but before
    // releasing its lifecycle lease. Reload both durable records under our
    // lease and reconcile only against this fresh checkpoint watermark.
    let checkpoint = if request.session_id.is_some() {
        resume_snapshot_exists = match &paths {
            Some(paths) => FileSessionStore::new(paths.clone())
                .load(&session_id)
                .await?
                .is_some(),
            None => false,
        };
        let checkpoint = match runtime.checkpoint_store() {
            Some(store) => store.load_latest(&session_id).await?,
            None => None,
        };
        if !resume_snapshot_exists && checkpoint.is_none() {
            return Err(HostSessionError::SessionNotFound {
                session: session_id,
            });
        }
        if let (Some(slot), Some(persisted)) = (
            resume_capsule.as_ref(),
            checkpoint.as_ref().and_then(|checkpoint| {
                checkpoint
                    .snapshot
                    .extension_state
                    .get(RESUME_CAPSULE_STATE_NAMESPACE)
            }),
        ) {
            slot.restore_versioned_state(persisted, RecoverySource::ProtectedCheckpoint)
                .map_err(|error| RuntimeError::conflict(error.to_string()))?;
        }
        checkpoint
    } else {
        None
    };

    let mut resume_identity_floor = None;
    let mut recovered_ephemeral_work = None;
    let mut restored_interaction = checkpoint.as_ref().and_then(|checkpoint| {
        if let TurnState::AwaitingInteraction {
            request,
            response: None,
            ..
        } = &checkpoint.state
        {
            Some(RestoredInteraction {
                request_id: request.id().clone(),
                turn_id: checkpoint.turn.clone(),
                question_count: request.questionnaire_payload().questions().len(),
            })
        } else {
            None
        }
    });
    let journal = match (&paths, journal_slot, checkpoint_barrier) {
        (Some(paths), Some(slot), Some(barrier)) => {
            let journal_path = paths.journal(&session_id)?;
            if request.session_id.is_some() {
                let recovery = read_journal(&journal_path).await?;
                recovered_ephemeral_work = unresolved_ephemeral_work(&recovery);
                match checkpoint.as_ref() {
                    Some(checkpoint) if !checkpoint.state.is_terminal() => {
                        let reconciled = reconcile_nonterminal_journal(
                            &journal_path,
                            checkpoint.watermark.event_sequence,
                        )
                        .await?;
                        if reconciled.retained_gap {
                            tracing::warn!(
                                session = %session_id,
                                "the retained journal prefix contains an explicit gap; exact presentation replay is unavailable"
                            );
                        }
                        resume_identity_floor = Some(reconciled.identity_floor);
                        if reconciled.truncated_records > 0 {
                            tracing::info!(
                                session = %session_id,
                                records = reconciled.truncated_records,
                                boundary = checkpoint.watermark.event_sequence,
                                "discarded presentation-only journal tail before checkpoint resume"
                            );
                        }
                    }
                    _ => {
                        if recovery.records.iter().any(|line| {
                            matches!(
                                line.record,
                                crate::journal::JournalRecord::Dropped { .. }
                                    | crate::journal::JournalRecord::Oversized { .. }
                            )
                        }) {
                            tracing::warn!(
                                session = %session_id,
                                "the journal contains an explicit gap; exact presentation replay is unavailable"
                            );
                        }
                        resume_identity_floor = Some(recovery.identity_floor());
                    }
                };
            }
            let journal = Arc::new(
                EventJournal::for_session(
                    paths,
                    &session_id,
                    request.journal,
                    Arc::new(persistence_redactor.clone()),
                )
                .await?,
            );
            barrier.install(journal.clone())?;
            slot.install(journal.clone())?;
            Some(journal)
        }
        (None, None, None) | (Some(_), None, None) => None,
        _ => {
            return Err(RuntimeError::config(
                "journal observer, checkpoint barrier, and session paths must be configured together",
            )
            .into());
        }
    };

    let mut start = StartSession::new()
        .with_id(session_id.clone())
        .with_checkpoint_recovery(CheckpointRecoveryPolicy::ResumeOrInterrupt);
    if let Some(floor) = resume_identity_floor {
        start = start.with_resume_identity_floor(floor);
    }
    if surface == crate::factory::HostSurface::Headless && restored_interaction.is_some() {
        start = start.with_checkpoint_recovery(CheckpointRecoveryPolicy::DeferPendingInteraction);
    }
    let session = match runtime.runtime().start_session(start).await {
        Ok(session) => session,
        Err(error) => {
            if let Some(journal) = &journal {
                let _ = journal.shutdown().await;
            }
            return Err(error.into());
        }
    };

    if session.interrupted_on_resume().is_some() {
        restored_interaction = None;
    }

    // Agent Runtime has now loaded its canonical and protected startup
    // candidates through the wrapped stores.  Select the capsule candidates
    // only after that boundary, then restore the optional protected summary
    // body and apply cold-process reconciliation last.  In particular, the
    // RedactingSessionStore load above must never run after cold_resume and
    // restore a pre-cold canonical projection back into the live slot.
    if request.session_id.is_some() {
        let canonical = match &paths {
            Some(paths) => {
                FileSessionStore::new(paths.clone())
                    .load(&session_id)
                    .await?
            }
            None => None,
        };
        let protected = match runtime.checkpoint_store() {
            Some(store) => store.load_latest(&session_id).await?,
            None => None,
        };
        if let Some(slot) = &resume_capsule {
            if let Some(persisted) = canonical
                .as_ref()
                .and_then(|snapshot| snapshot.extension_state.get(RESUME_CAPSULE_STATE_NAMESPACE))
            {
                slot.restore_versioned_state(persisted, RecoverySource::CanonicalSnapshot)
                    .map_err(|error| RuntimeError::conflict(error.to_string()))?;
            }
            if let Some(persisted) = protected.as_ref().and_then(|checkpoint| {
                checkpoint
                    .snapshot
                    .extension_state
                    .get(RESUME_CAPSULE_STATE_NAMESPACE)
            }) {
                slot.restore_versioned_state(persisted, RecoverySource::ProtectedCheckpoint)
                    .map_err(|error| RuntimeError::conflict(error.to_string()))?;
            }
            if let Some(store) = runtime.artifact_store() {
                if let Some(summary_state) = restore_runtime_summary_state(slot, store.as_ref())
                    .await
                    .map_err(|error| RuntimeError::conflict(error.to_string()))?
                    && session
                        .restore_semantic_summary_if_absent(summary_state)
                        .is_err()
                {
                    // Summary state is optional recovery acceleration. A
                    // changed route, incompatible revision, or stale source
                    // prefix must never block canonical cold continuation.
                    slot.update(|capsule| capsule.latest_summary_state_artifact = None);
                }
                restore_summary_artifact(slot, store.as_ref())
                    .await
                    .map_err(|error| RuntimeError::conflict(error.to_string()))?;
            }
            let _ = slot.cold_resume();
        }
    }

    // Root sessions get their delegation coordinator now that the session
    // exists: the `agent` tool starts answering, and completed child results
    // are routed into the session's safe-boundary inbox.
    runtime.wire_advisor(&session)?;
    let mut delegation_lifecycle = None;
    if let Some(delegation) = runtime.delegation() {
        let wait_policy = DelegationWaitPolicy::new(
            config.child_agents.wait_default_timeout_ms.value,
            config.child_agents.wait_max_timeout_ms.value,
        )?;
        delegation_lifecycle = Some(
            crate::delegation::wire_delegation_with_wait_policy(&session, delegation, wait_policy)
                .await?,
        );
        let durable_children = delegation
            .coordinator()
            .expect("a successfully wired delegation has a coordinator")
            .list()
            .into_iter()
            .filter(|status| status.durability == ChildDurability::Durable)
            .map(|status| status.child)
            .collect::<BTreeSet<_>>();
        if let Some(interruption) = &mut recovered_ephemeral_work {
            interruption
                .children
                .retain(|child| !durable_children.contains(child));
            if interruption.is_empty() {
                recovered_ephemeral_work = None;
            }
        }
    }
    if let (Some(journal), Some(interruption)) = (&journal, recovered_ephemeral_work.clone()) {
        journal.record_ephemeral_interruption(interruption).await?;
    }

    // Every session — persisted or not — gets a background-task context so a
    // `run_in_background` shell call always has somewhere to notify and spool
    // to. Without persistence there is no session directory to spool under,
    // so a process-scoped temp directory stands in; it is still cleaned up by
    // shutdown killing every task before the process exits.
    let task_spool_dir = match &paths {
        Some(paths) => paths.tasks_dir(session.id())?,
        None => std::env::temp_dir().join(format!("smith-tasks-{}", session.id())),
    };
    runtime
        .background_services()
        .expect("a standard HostSession always resolves background services")
        .registry()
        .register_session_context(
            session.id(),
            Some(session.clone()),
            journal.clone(),
            task_spool_dir,
        );

    if session.interrupted_on_resume().is_some()
        && let Some(component) = runtime.goal_component()
        && let Some(goal) = session.goal(component)?
        && goal.status == agent_runtime_core::goal::GoalStatus::Active
    {
        session
            .control_goal(
                component,
                GoalCommand::Pause {
                    id: goal.id,
                    generation: goal.generation,
                },
            )
            .await?;
    }

    let goal_admission_gate = runtime
        .goal_component()
        .map(|_| GoalAdmissionGate::new(true));
    let goal_controller = runtime
        .goal_component()
        .zip(goal_admission_gate.clone())
        .map(|(component, admission_gate)| {
            session.start_goal_controller(
                (**component).clone(),
                GoalControllerConfig::new(
                    "Continue the current persistent goal from its canonical state. Stop only by completing it or recording a genuine blocker.",
                )
                .with_sensitivity(agent_runtime_core::content::InternalTurnSensitivity::Public)
                .with_admission_gate(admission_gate),
            )
        })
        .transpose()?;

    let cache_config = CacheControllerConfig::from_resolved(
        &config.context.cache,
        CacheControllerResolvedInputs {
            synthetic_spend: config.synthetic_cache_spend,
            contract: runtime.policy().model_profile.capabilities.cache_contract(),
            model_input_limit: runtime.policy().model_profile.limits.max_input_tokens,
            model_output_limit: runtime.policy().model_profile.limits.max_output_tokens,
            provider: runtime.policy().provider_name.clone(),
            model: runtime.policy().model.as_str().to_owned(),
            endpoint_identity: runtime.policy().cache_endpoint_identity.clone(),
            profile_identity: runtime.policy().model_profile.fingerprint(),
            semantic_summary_provider: runtime
                .policy()
                .semantic_summary
                .as_ref()
                .map(|summary| summary.provider.clone()),
            semantic_summary_model: runtime
                .policy()
                .semantic_summary
                .as_ref()
                .map(|summary| summary.model.clone()),
            attempt_marker_available: resume_capsule.is_some(),
        },
    )
    .map_err(RuntimeError::config)?;
    let parking_monitor = delegation_lifecycle
        .as_ref()
        .map(DelegationLifecycle::monitor);
    let cache_controller = CacheLifecycleController::start(
        session.clone(),
        cache_config,
        host_clock,
        parking_monitor,
        resume_capsule.clone(),
        runtime.artifact_store().cloned(),
        changes.clone(),
    );

    let session_history_registration = runtime.session_history().register(session.clone());
    let client = crate::client::SmithSession::new(session.clone());
    Ok(HostSession {
        runtime,
        session,
        session_history_registration,
        client,
        display_redactor: persistence_redactor,
        journal,
        paths,
        snapshot_store,
        shutdown_result: tokio::sync::Mutex::new(None),
        ring,
        changes,
        lifecycle_lease: Mutex::new(lifecycle_lease),
        restored_interaction,
        recovered_ephemeral_work,
        goal_controller: Mutex::new(goal_controller),
        goal_admission_gate,
        delegation_lifecycle: Mutex::new(delegation_lifecycle),
        cache_controller: Mutex::new(Some(cache_controller)),
        final_cache_lifecycle: Mutex::new(None),
        resume_capsule,
    })
}
