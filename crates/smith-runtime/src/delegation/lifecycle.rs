use super::*;

/// Process-owned workers that project parking and request conditional child
/// completion admission. Exact payloads and cursor authority remain Runtime-
/// owned; this handle exists so Smith can freeze admission before shutdown.
#[derive(Debug)]
pub struct DelegationLifecycle {
    parking: Arc<std::sync::Mutex<DelegationParking>>,
    cancel: Cancellation,
    signal: Arc<tokio::sync::Notify>,
    tasks: std::sync::Mutex<Option<Vec<tokio::task::JoinHandle<()>>>>,
}

/// Cloneable notification seam used by the host cache controller. It exposes
/// only the identity-only parking projection, never child result content.
#[derive(Debug, Clone)]
pub(crate) struct DelegationParkingMonitor {
    parking: Arc<std::sync::Mutex<DelegationParking>>,
    signal: Arc<tokio::sync::Notify>,
}

impl DelegationParkingMonitor {
    pub(crate) fn snapshot(&self) -> ParkingSnapshot {
        self.parking
            .lock()
            .expect("delegation parking state poisoned")
            .snapshot()
    }

    pub(crate) async fn changed(&self) {
        self.signal.notified().await;
    }
}

impl DelegationLifecycle {
    /// Current identity-only parking projection for status and tests.
    pub fn snapshot(&self) -> ParkingSnapshot {
        self.parking
            .lock()
            .expect("delegation parking state poisoned")
            .snapshot()
    }

    pub(crate) fn monitor(&self) -> DelegationParkingMonitor {
        DelegationParkingMonitor {
            parking: self.parking.clone(),
            signal: self.signal.clone(),
        }
    }

    /// Freezes new admission first, then cancels and boundedly drains both
    /// process-owned workers.
    pub async fn shutdown(&self) {
        self.parking
            .lock()
            .expect("delegation parking state poisoned")
            .shutdown();
        self.cancel.cancel(CancelReason::Shutdown);
        self.signal.notify_waiters();
        let tasks = self
            .tasks
            .lock()
            .expect("delegation lifecycle tasks poisoned")
            .take()
            .unwrap_or_default();
        for mut task in tasks {
            if tokio::time::timeout(std::time::Duration::from_millis(250), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

impl Drop for DelegationLifecycle {
    fn drop(&mut self) {
        self.cancel.cancel(CancelReason::Shutdown);
        self.signal.notify_waiters();
        for task in self
            .tasks
            .lock()
            .expect("delegation lifecycle tasks poisoned")
            .take()
            .unwrap_or_default()
        {
            task.abort();
        }
    }
}

/// Shortest delay before a transiently refused child-completion admission is
/// retried, and the ceiling that repeated refusals back off to.
///
/// `Busy` and `Conflict` are races, not answers: Runtime refused *this*
/// attempt because the parent turn boundary or the protected cursor was
/// momentarily occupied. The admission worker is otherwise woken only by
/// runtime events, so a refusal arriving after the last event of a run would
/// never be retried and the ready outcome would never be delivered.
const ADMISSION_RETRY_MIN: Duration = Duration::from_millis(20);
const ADMISSION_RETRY_MAX: Duration = Duration::from_millis(500);

/// Doubles a transient-refusal retry delay up to [`ADMISSION_RETRY_MAX`].
///
/// `None` is the first refusal since the last progress, so the worker retries
/// quickly; a parent that stays occupied backs off instead of spinning.
pub fn next_admission_retry_delay(previous: Option<Duration>) -> Duration {
    match previous {
        None => ADMISSION_RETRY_MIN,
        Some(delay) => delay.saturating_mul(2).min(ADMISSION_RETRY_MAX),
    }
}

/// Starts the local parking projection and Runtime-backed admission worker.
///
/// Runtime's coordinator owns exact outcome payloads, protected cursor
/// advancement, and conditional idle admission. Smith only projects
/// lifecycle state and never fabricates a user-role message or local
/// canonical event for a child result.
pub(super) fn start_delegation_lifecycle_tasks(
    session: &SessionHandle,
    coordinator: DelegationCoordinator,
) -> DelegationLifecycle {
    let parking = Arc::new(std::sync::Mutex::new(DelegationParking::new()));
    let signal = Arc::new(tokio::sync::Notify::new());
    let cancel = Cancellation::new();
    let handle_parking = parking.clone();
    let handle_signal = signal.clone();

    let event_parking = parking.clone();
    let event_signal = signal.clone();
    let event_coordinator = coordinator.clone();
    let event_session = session.clone();
    let event_cancel = cancel.clone();
    let event_task = tokio::spawn(async move {
        let mut events = event_session.subscribe();
        loop {
            let envelope = tokio::select! {
                _ = event_cancel.cancelled() => break,
                envelope = events.next() => match envelope {
                    Some(envelope) => envelope,
                    None => break,
                },
            };
            let mut state = event_parking
                .lock()
                .expect("delegation parking state poisoned");
            match envelope.payload {
                RuntimeEvent::TurnStarted | RuntimeEvent::InternalTurnStarted { .. } => {
                    state.parent_turn_started();
                }
                RuntimeEvent::TurnCompleted { .. } => {
                    let pending = event_coordinator
                        .list()
                        .into_iter()
                        .filter(|status| matches!(status.state, ChildState::Running))
                        .map(|status| status.child.as_str().to_owned())
                        .collect::<Vec<_>>();
                    state.parent_turn_completed(pending);
                }
                RuntimeEvent::ChildSpawned { child, .. } => {
                    state.child_spawned(child.as_str());
                }
                RuntimeEvent::ChildNeedsInput { child, .. }
                | RuntimeEvent::ChildCompleted { child, .. }
                | RuntimeEvent::ChildStopped { child, .. }
                | RuntimeEvent::ChildFailed { child, .. } => {
                    state.child_terminal(child.as_str());
                }
                RuntimeEvent::SessionShutdown => state.shutdown(),
                _ => {}
            }
            drop(state);
            event_signal.notify_waiters();
        }
        event_parking
            .lock()
            .expect("delegation parking state poisoned")
            .shutdown();
        event_signal.notify_waiters();
    });

    let admission_parking = parking;
    let admission_signal = signal;
    let admission_cancel = cancel.clone();
    let admission_task = tokio::spawn(async move {
        let mut initial_snapshot = true;
        let mut retry_delay: Option<Duration> = None;
        loop {
            if admission_cancel.is_cancelled() {
                break;
            }
            let notified = admission_signal.notified();
            tokio::pin!(notified);
            // Register before reading the protected snapshot. `notify_waiters`
            // does not retain a permit for a future that has not been polled,
            // so omitting this creates a lost-wakeup window between the
            // snapshot and the select below.
            notified.as_mut().enable();
            let mut progressed = false;
            // Set only by an admission Runtime refused for a transient reason.
            let mut transient_refusal = false;

            // This is an idempotent protected snapshot, not an acknowledgement.
            // Runtime's child-completion admission remains the only operation
            // that advances the canonical cursor.
            progressed |= reconcile_ready_outcome_snapshot(
                &admission_parking,
                coordinator.take_ready_task_outcomes(),
                initial_snapshot,
            );
            initial_snapshot = false;

            let should_admit = admission_parking
                .lock()
                .expect("delegation parking state poisoned")
                .begin_child_completion_admission();
            if should_admit {
                let cursor = coordinator.child_outcome_cursor();
                let request = ChildCompletionAdmissionRequest::new(cursor.parent().clone(), cursor);
                match coordinator
                    .try_admit_child_completion_if_idle(request)
                    .await
                {
                    Ok(ChildCompletionAdmission::Accepted { turn, cursor }) => {
                        progressed = true;
                        admission_parking
                            .lock()
                            .expect("delegation parking state poisoned")
                            .admission_accepted(cursor.revision());
                        // Waiting for the ordinary turn boundary ensures a
                        // second attributed continuation cannot be started
                        // concurrently by this Smith worker.
                        turn.completed().await;
                        progressed |= reconcile_ready_outcome_snapshot(
                            &admission_parking,
                            coordinator.take_ready_task_outcomes(),
                            false,
                        );
                    }
                    Ok(ChildCompletionAdmission::Busy) => {
                        transient_refusal = true;
                        admission_parking
                            .lock()
                            .expect("delegation parking state poisoned")
                            .admission_busy();
                    }
                    Ok(ChildCompletionAdmission::Stale) => {
                        progressed = true;
                        let revision = coordinator.child_outcome_cursor().revision();
                        admission_parking
                            .lock()
                            .expect("delegation parking state poisoned")
                            .admission_stale(revision);
                        progressed |= reconcile_ready_outcome_snapshot(
                            &admission_parking,
                            coordinator.take_ready_task_outcomes(),
                            false,
                        );
                    }
                    Ok(ChildCompletionAdmission::Shutdown) => {
                        admission_parking
                            .lock()
                            .expect("delegation parking state poisoned")
                            .shutdown();
                        break;
                    }
                    Ok(ChildCompletionAdmission::Conflict { .. }) | Err(_) => {
                        transient_refusal = true;
                        admission_parking
                            .lock()
                            .expect("delegation parking state poisoned")
                            .admission_conflict();
                    }
                }
            }

            if admission_parking
                .lock()
                .expect("delegation parking state poisoned")
                .is_shutdown_frozen()
            {
                break;
            }
            if progressed {
                retry_delay = None;
            } else if transient_refusal {
                // A refused attempt must not wait on an event that a quiet
                // session will never emit.
                let delay = next_admission_retry_delay(retry_delay);
                retry_delay = Some(delay);
                tokio::select! {
                    _ = &mut notified => {}
                    _ = tokio::time::sleep(delay) => {}
                    _ = admission_cancel.cancelled() => break,
                }
            } else {
                retry_delay = None;
                tokio::select! {
                    _ = &mut notified => {}
                    _ = admission_cancel.cancelled() => break,
                }
            }
        }
        admission_parking
            .lock()
            .expect("delegation parking state poisoned")
            .shutdown();
    });
    DelegationLifecycle {
        parking: handle_parking,
        cancel,
        signal: handle_signal,
        tasks: std::sync::Mutex::new(Some(vec![event_task, admission_task])),
    }
}

fn reconcile_ready_outcome_snapshot(
    parking: &Arc<std::sync::Mutex<DelegationParking>>,
    outcomes: Vec<ChildTaskOutcome>,
    recovered: bool,
) -> bool {
    let keys = outcomes
        .iter()
        .map(terminal_outcome_key)
        .collect::<Vec<_>>();
    let mut state = parking.lock().expect("delegation parking state poisoned");
    for key in &keys {
        // The lossless Runtime snapshot closes the local live-child
        // projection even if a bounded presentation subscriber lagged.
        state.child_terminal(&key.child_id);
    }
    let changed = state.reconcile_ready_outcomes(keys);
    if recovered {
        state.enable_idle_wakeup_for_recovered_outcomes();
    }
    changed
}

fn terminal_outcome_key(outcome: &ChildTaskOutcome) -> TerminalOutcomeKey {
    match outcome {
        ChildTaskOutcome::Completed { child, result } => {
            TerminalOutcomeKey::new(child.as_str(), result.turn.as_str())
        }
        ChildTaskOutcome::NeedsInput { child, request } => {
            TerminalOutcomeKey::new(child.as_str(), request.id().as_str())
        }
    }
}
