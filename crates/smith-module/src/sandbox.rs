use std::sync::Mutex;

/// Authority vocabulary shared by module evidence and sandbox brokers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(missing_docs)]
pub enum ModuleCapability {
    WorkspaceRead,
    WorkspaceWrite,
    HostRead,
    HostWrite,
    Process,
    Network,
    ExternalRead,
    ExternalWrite,
    DataEgress,
    Credential,
    Approval,
    Provider,
}

#[derive(Debug, Default)]
struct HealthState {
    consecutive_failures: u8,
    failure: Option<String>,
    child_sessions: std::collections::BTreeMap<String, std::sync::Arc<ModuleHealth>>,
}

/// Live session failure state, retained across composition rebuilds.
#[derive(Debug, Default)]
pub struct ModuleHealth(Mutex<HealthState>);
impl ModuleHealth {
    /// Retains an independent child session counter across module rebuilds.
    /// The actual session identity comes from host invocation context, never the guest.
    pub fn child_session(
        &self,
        session: &agent_runtime_core::ids::SessionId,
    ) -> std::sync::Arc<Self> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .child_sessions
            .entry(session.as_str().into())
            .or_default()
            .clone()
    }

    /// Records permanent session disabling, for example a registration collision.
    pub fn fail(&self, reason: String) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.consecutive_failures = 3;
        state.failure = Some(reason);
    }
    /// Resets consecutive failures after a successful invocation, without reenabling.
    pub fn record_success(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.failure.is_none() {
            state.consecutive_failures = 0;
        }
    }
    /// Counts a failed invocation and disables after three consecutive failures.
    /// Returns whether this session module is now disabled.
    pub fn record_failure(&self, reason: String) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        if state.consecutive_failures >= 3 && state.failure.is_none() {
            state.failure = Some(reason);
        }
        state.failure.is_some()
    }
    /// Returns the safe diagnostic, if disabled.
    pub fn failure(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .failure
            .clone()
    }
}
impl PartialEq for ModuleHealth {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other) || self.failure() == other.failure()
    }
}
impl Eq for ModuleHealth {}
