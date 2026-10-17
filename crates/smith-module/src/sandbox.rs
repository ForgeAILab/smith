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
    Model,
    SessionNotify,
}

#[derive(Debug, Default)]
struct HealthState {
    consecutive_failures: u8,
    failure: Option<String>,
    diagnostics: std::collections::BTreeMap<String, String>,
    model_calls: std::collections::BTreeMap<
        agent_runtime_core::ids::SessionId,
        (agent_runtime_core::ids::TurnId, u32),
    >,
    child_sessions: std::collections::BTreeMap<String, std::sync::Arc<ModuleHealth>>,
}

/// Live session failure state, retained across composition rebuilds.
#[derive(Debug, Default)]
pub struct ModuleHealth(Mutex<HealthState>);
impl ModuleHealth {
    /// Records a bounded, non-fatal contribution diagnostic for module listings.
    pub fn contribution_diagnostic(&self, name: &str, reason: &str) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.diagnostics.len() < 256 || state.diagnostics.contains_key(name) {
            state.diagnostics.insert(
                name.chars().take(128).collect(),
                reason.chars().take(512).collect(),
            );
        }
    }

    /// Clears diagnostics when a fresh composition mounts this module.
    pub fn clear_contribution_diagnostics(&self) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .diagnostics
            .clear();
    }

    /// Safe explanations for refused or stopped contributions, in stable order.
    pub fn contribution_diagnostics(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .diagnostics
            .iter()
            .map(|(name, reason)| format!("{name}: {reason}"))
            .collect()
    }

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

    /// Reserves a model attempt in the host's turn, surviving composition rebuilds.
    /// Refusals do not affect invocation health. Missing turns fail closed.
    pub fn reserve_model_call(
        &self,
        session: &agent_runtime_core::ids::SessionId,
        turn: Option<&agent_runtime_core::ids::TurnId>,
        maximum: u32,
    ) -> Option<u32> {
        let turn = turn?;
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let (current, count) = state
            .model_calls
            .entry(session.clone())
            .or_insert_with(|| (turn.clone(), 0));
        if current != turn {
            *current = turn.clone();
            *count = 0;
        }
        if *count >= maximum.min(4) {
            return None;
        }
        *count += 1;
        Some(*count)
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
