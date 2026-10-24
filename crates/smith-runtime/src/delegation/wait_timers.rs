use super::*;
use agent_runtime::runtime::InternalTurnAdmission;
use agent_runtime_core::content::{InternalTurnInput, InternalTurnSensitivity, InternalTurnSource};
use std::sync::Mutex;
use tokio::time::Instant as TimerInstant;

/// Process-local wake information for one child, also used by client panels.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChildWaitMonitor {
    /// Stable direct-child identity.
    pub child_id: String,
    /// Scheduled wake time as Unix milliseconds.
    pub deadline: u64,
    /// Effective timer duration in milliseconds.
    pub wake_after_ms: u64,
}

#[derive(Debug)]
struct WaitTimer {
    monitor: ChildWaitMonitor,
    expires: TimerInstant,
    expiry_observed: bool,
}

#[derive(Debug, Default)]
struct TimerState {
    timers: BTreeMap<String, WaitTimer>,
    children: BTreeMap<String, (String, TimerInstant)>,
    shutdown: bool,
}

/// Single-use wait timers shared by the root tool and session lifecycle worker.
/// They contain no child output and are never persisted.
#[derive(Debug)]
pub struct AgentWaitTimers {
    state: Mutex<TimerState>,
    changed: tokio::sync::Notify,
    default_profile: String,
}

impl Default for AgentWaitTimers {
    fn default() -> Self {
        Self::new("inherited")
    }
}

impl AgentWaitTimers {
    /// Creates an empty process-local registry for the root's profile.
    pub fn new(default_profile: impl Into<String>) -> Self {
        Self {
            state: Mutex::new(TimerState::default()),
            changed: tokio::sync::Notify::new(),
            default_profile: default_profile.into(),
        }
    }

    /// Live monitor tags in deterministic child-ID order.
    pub fn snapshot(&self) -> Vec<ChildWaitMonitor> {
        self.state
            .lock()
            .expect("wait timers poisoned")
            .timers
            .values()
            .map(|timer| timer.monitor.clone())
            .collect()
    }

    pub(super) fn task_started(&self, child: &str, profile: Option<&str>) {
        let mut state = self.state.lock().expect("wait timers poisoned");
        if !state.shutdown {
            state.children.insert(
                child.to_owned(),
                (
                    profile
                        .unwrap_or(&self.default_profile)
                        .chars()
                        .take(128)
                        .collect(),
                    TimerInstant::now(),
                ),
            );
        }
    }

    pub(super) fn arm(&self, child: &str, timeout: Duration) -> Option<ChildWaitMonitor> {
        if timeout.is_zero() {
            return None;
        }
        let mut state = self.state.lock().expect("wait timers poisoned");
        if state.shutdown {
            return None;
        }
        state
            .children
            .entry(child.to_owned())
            .or_insert_with(|| ("unknown".to_owned(), TimerInstant::now()));
        let now = TimerInstant::now();
        let wall_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let monitor = ChildWaitMonitor {
            child_id: child.to_owned(),
            deadline: wall_ms.saturating_add(timeout.as_millis() as u64),
            wake_after_ms: timeout.as_millis() as u64,
        };
        state.timers.insert(
            child.to_owned(),
            WaitTimer {
                monitor: monitor.clone(),
                expires: now + timeout,
                expiry_observed: false,
            },
        );
        drop(state);
        self.changed.notify_one();
        Some(monitor)
    }

    /// The timer still armed for `child`, reporting its remaining duration.
    pub(super) fn armed(&self, child: &str) -> Option<ChildWaitMonitor> {
        let state = self.state.lock().expect("wait timers poisoned");
        state.timers.get(child).map(|timer| ChildWaitMonitor {
            wake_after_ms: timer
                .expires
                .saturating_duration_since(TimerInstant::now())
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            ..timer.monitor.clone()
        })
    }

    pub(super) fn task_restarted(&self, child: &str) {
        let mut state = self.state.lock().expect("wait timers poisoned");
        if !state.shutdown {
            state.timers.remove(child);
            let task = state
                .children
                .entry(child.to_owned())
                .or_insert_with(|| ("unknown".to_owned(), TimerInstant::now()));
            task.1 = TimerInstant::now();
        }
        drop(state);
        self.changed.notify_one();
    }

    pub(super) fn cancel(&self, child: &str) {
        let removed = self
            .state
            .lock()
            .expect("wait timers poisoned")
            .timers
            .remove(child)
            .is_some();
        if removed {
            self.changed.notify_one();
        }
    }

    pub(super) fn shutdown(&self) {
        let mut state = self.state.lock().expect("wait timers poisoned");
        state.shutdown = true;
        state.timers.clear();
        state.children.clear();
        drop(state);
        self.changed.notify_one();
    }

    pub(super) async fn changed(&self) {
        self.changed.notified().await;
    }

    pub(super) fn next_deadline(&self) -> Option<TimerInstant> {
        let now = TimerInstant::now();
        let mut state = self.state.lock().expect("wait timers poisoned");
        for timer in state.timers.values_mut() {
            if timer.expires <= now {
                timer.expiry_observed = true;
            }
        }
        state
            .timers
            .values()
            .filter(|timer| !timer.expiry_observed)
            .map(|timer| timer.expires)
            .min()
    }

    // Called by the same worker that admits completions. No await separates
    // the final coordinator check from Runtime's atomic idle admission.
    pub(super) fn try_admit(
        &self,
        session: &SessionHandle,
        coordinator: &DelegationCoordinator,
    ) -> Result<Option<InternalTurnAdmission>, RuntimeError> {
        let mut state = self.state.lock().expect("wait timers poisoned");
        if state.shutdown {
            return Ok(None);
        }
        state.timers.retain(|child, _| {
            coordinator
                .status(&agent_runtime_core::ids::ChildId::new(child))
                .is_ok_and(|status| status.state == ChildState::Running)
        });
        if !coordinator.take_ready_task_outcomes().is_empty() {
            return Ok(None);
        }
        let now = TimerInstant::now();
        let expired = state
            .timers
            .iter()
            .filter(|(_, timer)| timer.expires <= now)
            .map(|(child, timer)| {
                let (profile, started) = &state.children[child];
                json!({"child_id": child, "profile": profile, "state": "running",
                    "elapsed_ms": now.saturating_duration_since(*started).as_millis() as u64,
                    "wake_after_ms": timer.monitor.wake_after_ms})
            })
            .collect::<Vec<_>>();
        if expired.is_empty() {
            return Ok(None);
        }
        let input = InternalTurnInput::new(
            format!(
                "The single-use agent wait deadline elapsed. These children are still running:\n{}\nEach child's outcome will arrive as a host message in a later turn. Do any independent work, then end your response; do not predict, announce, or write an outcome yourself. Arm wait again only if you need another deadline; inspect or stop a child only when useful. No partial child output is included.",
                Value::Array(expired.clone())
            ),
            InternalTurnSource {
                kind: "delegation.wait-timeout".into(),
                id: "wait-timers".into(),
                revision: RegistryRevision::new("1"),
                sensitivity: InternalTurnSensitivity::Public,
                goal: None,
            },
        )?;
        let admission = session.try_send_internal_if_idle(input)?;
        if matches!(admission, InternalTurnAdmission::Accepted(_)) {
            for child in expired.iter().filter_map(|row| row["child_id"].as_str()) {
                state.timers.remove(child);
            }
        }
        Ok(Some(admission))
    }
}
