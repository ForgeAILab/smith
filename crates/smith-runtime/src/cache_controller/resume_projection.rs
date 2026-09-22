//! Reduction of canonical Runtime events into the bounded resume capsule.

use super::*;

pub(super) fn reduce_event(
    state: &Arc<Mutex<ControllerState>>,
    capsule: Option<&ResumeCapsuleSlot>,
    changes: &ChangeRecorder,
    envelope: &EventEnvelope,
) -> bool {
    let mut state = state.lock().expect("cache controller state poisoned");
    if state
        .snapshot
        .lifecycle
        .last_event_sequence
        .is_some_and(|last| envelope.seq <= last)
    {
        let _ = state.snapshot.lifecycle.apply(envelope);
        return false;
    }
    if state
        .snapshot
        .lifecycle
        .last_event_sequence
        .is_some_and(|last| envelope.seq > last.saturating_add(1))
    {
        state.event_gap = true;
        state.boundary_evaluated = true;
        state.snapshot.scheduled_for = None;
        state.snapshot.last_error = Some("runtime_event_gap".to_owned());
    }
    let synthetic_state_attempt = match &envelope.payload {
        RuntimeEvent::CacheStateChanged { attempt, .. } => {
            state.synthetic_attempts.contains(attempt)
        }
        _ => false,
    };
    state.snapshot.lifecycle.apply(envelope);
    let mut persist = false;
    match &envelope.payload {
        RuntimeEvent::TurnStarted | RuntimeEvent::InternalTurnStarted { .. } => {
            state.parent_turn_active = true;
            state.last_meaningful_activity_at = Some(envelope.timestamp);
            state
                .snapshot
                .lifecycle
                .record_parent_activity(envelope.timestamp);
        }
        RuntimeEvent::TurnSteerCommitted { .. }
        | RuntimeEvent::ToolCallRequested { .. }
        | RuntimeEvent::ToolCallCompleted { .. } => {
            state.last_meaningful_activity_at = Some(envelope.timestamp);
            state
                .snapshot
                .lifecycle
                .record_parent_activity(envelope.timestamp);
        }
        RuntimeEvent::ContextPlanned { input_tokens, .. } => {
            state.planned_input_tokens = *input_tokens;
        }
        RuntimeEvent::CachePlanChanged {
            preserved_prefix_tokens,
            ..
        } => {
            state.plan_has_comparable_predecessor = *preserved_prefix_tokens > 0;
        }
        RuntimeEvent::CacheStateChanged {
            cache_identity: Some(identity),
            ..
        } if !synthetic_state_attempt => {
            state
                .snapshot
                .lifecycle
                .record_parent_request(identity, envelope.timestamp);
            if let Some(activity) = state.last_meaningful_activity_at
                && let Some(lease) = state.snapshot.lifecycle.current_mut()
            {
                lease.record_meaningful_activity(activity);
            }
            persist = true;
        }
        RuntimeEvent::CacheOperationStarted {
            attempt: Some(attempt),
            ..
        } => {
            state.synthetic_attempts.insert(attempt.clone());
        }
        RuntimeEvent::CacheOperationCompleted {
            attempt: Some(attempt),
            ..
        } => {
            state.synthetic_attempts.remove(attempt);
            persist = true;
        }
        RuntimeEvent::GoalUpdated { goal, .. } => {
            state.goal_active = goal.as_ref().is_some_and(|goal| goal.status.is_active());
            state.goal_remaining_tokens = goal.as_ref().and_then(|goal| {
                goal.token_budget
                    .zip(goal.usage.charged_tokens)
                    .map(|(budget, used)| budget.saturating_sub(used))
            });
            persist = true;
        }
        RuntimeEvent::RateLimitObservation { snapshot, .. } => {
            state.provider_rate_limits = Some(snapshot.clone());
        }
        RuntimeEvent::PlanUpdated { .. }
        | RuntimeEvent::ChildSpawned { .. }
        | RuntimeEvent::ChildNeedsInput { .. }
        | RuntimeEvent::ChildCompleted { .. }
        | RuntimeEvent::ChildStopped { .. }
        | RuntimeEvent::ChildFailed { .. }
        | RuntimeEvent::InteractionRequested { .. }
        | RuntimeEvent::InteractionResolved { .. }
        | RuntimeEvent::ContextCompacted { .. } => persist = true,
        RuntimeEvent::TurnCompleted { .. } => {
            state.parent_turn_active = false;
            state.parent_idle_interval = state.parent_idle_interval.saturating_add(1);
            state.last_meaningful_activity_at = Some(envelope.timestamp);
            state
                .snapshot
                .lifecycle
                .record_parent_activity(envelope.timestamp);
            let interval_id = envelope
                .turn
                .as_ref()
                .map(|turn| format!("root-turn:{}", turn.as_str()))
                .unwrap_or_else(|| format!("root-event:{}", envelope.seq));
            state.snapshot.idle_compaction.reset(interval_id);
            persist = true;
        }
        RuntimeEvent::SessionShutdown => state.shutting_down = true,
        _ => {}
    }
    drop(state);
    if let Some(capsule) = capsule {
        update_capsule(capsule, changes, envelope, !synthetic_state_attempt);
    }
    persist
}

pub(super) fn update_capsule(
    slot: &ResumeCapsuleSlot,
    changes: &ChangeRecorder,
    envelope: &EventEnvelope,
    real_parent_cache_state: bool,
) {
    slot.update(|capsule| {
        capsule.exact_state.watermark = capsule.exact_state.watermark.max(envelope.seq);
        if let Some(turn) = &envelope.turn {
            capsule.parent_turn_id = Some(turn.clone());
            capsule.exact_state.parent_turn_id = Some(turn.clone());
        }
        if matches!(
            &envelope.payload,
            RuntimeEvent::TurnStarted
                | RuntimeEvent::InternalTurnStarted { .. }
                | RuntimeEvent::TurnSteerCommitted { .. }
                | RuntimeEvent::ToolCallRequested { .. }
                | RuntimeEvent::ToolCallCompleted { .. }
        ) {
            capsule.cache.last_meaningful_activity_at = Some(envelope.timestamp);
        }
        match &envelope.payload {
            RuntimeEvent::GoalUpdated { goal, .. } => {
                capsule.exact_state.goal = goal.as_ref().map(|goal| ExactGoalProjection {
                    goal_id: Some(goal.id.to_string()),
                    generation: goal.generation,
                    state: Some(goal.status.as_str().to_owned()),
                });
            }
            RuntimeEvent::PlanUpdated {
                revision, counts, ..
            } => {
                capsule.exact_state.plan = Some(ExactPlanProjection {
                    revision: *revision,
                    pending: count(counts, "pending"),
                    completed: count(counts, "completed"),
                    failed: count(counts, "failed").saturating_add(count(counts, "cancelled")),
                });
            }
            RuntimeEvent::ChildSpawned { child, .. } => {
                if metadata_fits(child.as_str())
                    && (capsule.exact_state.children.contains_key(child)
                        || capsule.exact_state.children.len() < MAX_CHILDREN)
                {
                    capsule.exact_state.children.insert(
                        child.clone(),
                        ChildResumeProjection {
                            child: child.clone(),
                            task_digest: None,
                            state: ChildLifecycleState::Running,
                            terminal_outcome: None,
                            watermark: envelope.seq,
                        },
                    );
                }
            }
            RuntimeEvent::ChildNeedsInput { child, request, .. } => {
                update_child(
                    &mut capsule.exact_state.children,
                    child,
                    ChildLifecycleState::NeedsInput,
                    Some(Fingerprint::of(request.as_str())),
                    envelope.seq,
                );
            }
            RuntimeEvent::ChildCompleted { child, result } => {
                update_child(
                    &mut capsule.exact_state.children,
                    child,
                    ChildLifecycleState::Completed,
                    Some(Fingerprint::of(result.as_bytes())),
                    envelope.seq,
                );
            }
            RuntimeEvent::ChildStopped { child, .. } => update_child(
                &mut capsule.exact_state.children,
                child,
                ChildLifecycleState::Stopped,
                None,
                envelope.seq,
            ),
            RuntimeEvent::ChildFailed { child, .. } => update_child(
                &mut capsule.exact_state.children,
                child,
                ChildLifecycleState::Failed,
                None,
                envelope.seq,
            ),
            RuntimeEvent::InteractionRequested { .. } => {
                capsule.exact_state.unresolved_decisions =
                    capsule.exact_state.unresolved_decisions.saturating_add(1);
            }
            RuntimeEvent::InteractionResolved { .. } => {
                capsule.exact_state.unresolved_decisions =
                    capsule.exact_state.unresolved_decisions.saturating_sub(1);
            }
            RuntimeEvent::ToolCallCompleted {
                call,
                name,
                is_error,
            } if matches!(name.as_str(), "shell" | "task_output") => {
                let key = call.to_string();
                if metadata_fits(&key)
                    && (capsule.exact_state.validations.contains_key(&key)
                        || capsule.exact_state.validations.len() < MAX_VALIDATIONS)
                {
                    capsule.exact_state.validations.insert(
                        key.clone(),
                        ValidationProjection {
                            validation: key,
                            exit_status: Some(i32::from(*is_error)),
                            watermark: envelope.seq,
                        },
                    );
                }
            }
            RuntimeEvent::CacheStateChanged {
                cache_identity,
                state,
                ..
            } => {
                if real_parent_cache_state {
                    capsule.retire_handoff_if_identity_changed(cache_identity.as_ref());
                    capsule.cache.prior_identity = cache_identity.clone();
                }
                capsule.cache.provider_warmth = match state {
                    agent_runtime_core::event::CacheState::WarmObserved => {
                        ResumeCacheWarmth::WarmObserved
                    }
                    agent_runtime_core::event::CacheState::MissObserved => {
                        ResumeCacheWarmth::MissObserved
                    }
                    agent_runtime_core::event::CacheState::Expired => {
                        ResumeCacheWarmth::ExpiredObserved
                    }
                    _ => ResumeCacheWarmth::Unknown,
                };
                if real_parent_cache_state {
                    capsule.cache.cold_resume = false;
                }
            }
            RuntimeEvent::CacheAvailabilityEvidenceRecorded { evidence } => {
                if let Some(guaranteed_until) = evidence.guaranteed_until {
                    capsule.cache.guaranteed_until = Some(guaranteed_until);
                }
            }
            RuntimeEvent::TurnCompleted { .. } => {
                capsule.cache.last_meaningful_activity_at = Some(envelope.timestamp);
                capsule.cache.idle_compaction_interval_id = Some(
                    envelope
                        .turn
                        .as_ref()
                        .map(|turn| format!("root-turn:{}", turn.as_str()))
                        .unwrap_or_else(|| format!("root-event:{}", envelope.seq)),
                );
                capsule.cache.idle_compaction_attempted = false;
                if let Some(turn) = &envelope.turn {
                    let _ = capsule.push_recent_turn(RecentTurnProjection {
                        turn: turn.clone(),
                        role: RecentTurnRole::Assistant,
                        content_digest: None,
                    });
                }
                if let Some(change_set) = changes.latest() {
                    for mutation in change_set.mutations {
                        if let ToolMutation::Exact(edit) = mutation {
                            let path = edit.path.to_string_lossy().into_owned();
                            if metadata_fits(&path)
                                && capsule.exact_state.changed_files.len() < MAX_CHANGED_FILES
                                && !capsule
                                    .exact_state
                                    .changed_files
                                    .iter()
                                    .any(|changed| changed.path == path)
                            {
                                capsule
                                    .exact_state
                                    .changed_files
                                    .push(ChangedFileProjection {
                                        path,
                                        additions: 0,
                                        deletions: 0,
                                        digest: Some(Fingerprint::of(edit.after_hash)),
                                    });
                            }
                        }
                    }
                }
            }
            RuntimeEvent::ContextCompacted { .. } => {
                capsule.retire_handoff_if_identity_changed(None);
                capsule.cache.prior_identity = None;
                capsule.cache.provider_warmth = ResumeCacheWarmth::Unknown;
                capsule.cache.guaranteed_until = None;
                capsule.cache.cold_resume = true;
            }
            _ => {}
        }
    });
}

pub(super) fn update_child(
    children: &mut BTreeMap<ChildId, ChildResumeProjection>,
    child: &ChildId,
    state: ChildLifecycleState,
    digest: Option<Fingerprint>,
    watermark: u64,
) {
    if !metadata_fits(child.as_str())
        || (!children.contains_key(child) && children.len() >= MAX_CHILDREN)
    {
        return;
    }
    let projection = children
        .entry(child.clone())
        .or_insert_with(|| ChildResumeProjection {
            child: child.clone(),
            task_digest: None,
            state,
            terminal_outcome: None,
            watermark,
        });
    projection.state = state;
    projection.watermark = watermark;
    projection.terminal_outcome = Some(ChildTerminalOutcome {
        state,
        result_digest: digest,
        watermark,
    });
}

pub(super) fn metadata_fits(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_METADATA_BYTES
}
pub(super) fn count(counts: &BTreeMap<String, u32>, key: &str) -> u32 {
    counts.get(key).copied().unwrap_or_default()
}
