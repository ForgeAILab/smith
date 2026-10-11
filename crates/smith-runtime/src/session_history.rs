//! Read-only host access to canonical active-session history.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use agent_runtime::delegation::DelegationCoordinator;
use agent_runtime::runtime::SessionHandle;
use agent_runtime_core::content::Message;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::SessionId;

/// Maps active host sessions to their canonical history without copying image
/// data into a second cache. Resumed sessions register their restored handle.
#[derive(Debug, Default)]
pub(crate) struct LiveSessionHistory {
    sessions: Mutex<HashMap<String, SessionHandle>>,
    delegation: Mutex<Weak<OnceLock<DelegationCoordinator>>>,
}

impl LiveSessionHistory {
    /// Uses the existing coordinator for child history without owning its lifecycle.
    pub(crate) fn set_delegation(&self, slot: &Arc<OnceLock<DelegationCoordinator>>) {
        *self
            .delegation
            .lock()
            .expect("session-history delegation lock poisoned") = Arc::downgrade(slot);
    }

    /// Registers one live or resumed session until the returned lease drops or
    /// is explicitly unregistered during host shutdown.
    pub(crate) fn register(self: &Arc<Self>, session: SessionHandle) -> SessionHistoryRegistration {
        let key = session.id().as_str().to_owned();
        self.sessions
            .lock()
            .expect("session-history registry lock poisoned")
            .insert(key.clone(), session);
        SessionHistoryRegistration {
            registry: Arc::downgrade(self),
            key,
            active: AtomicBool::new(true),
        }
    }

    fn unregister(&self, key: &str) {
        self.sessions
            .lock()
            .expect("session-history registry lock poisoned")
            .remove(key);
    }
}

impl smith_module::SessionHistory for LiveSessionHistory {
    fn with_history(
        &self,
        session: &SessionId,
        visitor: &mut dyn FnMut(&[Message]),
    ) -> Result<(), RuntimeError> {
        let handle = self
            .sessions
            .lock()
            .expect("session-history registry lock poisoned")
            .get(session.as_str())
            .cloned();
        if let Some(handle) = handle {
            handle.with_history(visitor);
            return Ok(());
        }
        let slot = self
            .delegation
            .lock()
            .expect("session-history delegation lock poisoned")
            .upgrade();
        if let Some(coordinator) = slot.as_ref().and_then(|slot| slot.get())
            && let Some(child) = coordinator
                .list()
                .into_iter()
                .find(|child| child.session == *session)
            && coordinator
                .with_child_history(&child.child, visitor)
                .is_some()
        {
            return Ok(());
        }
        Err(RuntimeError::not_found(
            "this session has no active image history",
        ))
    }
}

/// Removes one session from recent-history lookup on shutdown or drop.
#[derive(Debug)]
pub(crate) struct SessionHistoryRegistration {
    registry: Weak<LiveSessionHistory>,
    key: String,
    active: AtomicBool,
}

impl SessionHistoryRegistration {
    /// Removes the registered history source. Safe to call more than once.
    pub(crate) fn unregister(&self) {
        if self.active.swap(false, Ordering::AcqRel)
            && let Some(registry) = self.registry.upgrade()
        {
            registry.unregister(&self.key);
        }
    }
}

impl Drop for SessionHistoryRegistration {
    fn drop(&mut self) {
        self.unregister();
    }
}
