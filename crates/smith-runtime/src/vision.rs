//! Session accounting for tool-free vision requests, including interrupted streams.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agent_runtime::harness::{
    ComponentDescriptor, TurnCommitHook, TurnCommitPatch, TurnCommitView,
};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::store::{SessionSnapshot, SessionStore};
use agent_runtime_core::usage::UsageRecord;
use async_trait::async_trait;
use smith_module::VisionUsageRecorder;

#[derive(Debug, Clone)]
pub(crate) struct VisionRoute {
    pub(crate) binding: smith_module::VisionBinding,
    pub(crate) price: Option<smith_config::catalog::CatalogModelCost>,
}

#[derive(Debug, Default)]
pub(crate) struct VisionAccounting {
    pending: Mutex<BTreeMap<SessionId, Vec<UsageRecord>>>,
}

impl VisionAccounting {
    pub(crate) fn account_snapshot(&self, snapshot: &mut SessionSnapshot) {
        let pending = self.pending.lock().expect("vision usage lock poisoned");
        if let Some(records) = pending.get(&snapshot.id) {
            for record in records {
                if !snapshot.usage.records().contains(record) {
                    snapshot.usage.record(record.clone());
                }
            }
        }
    }

    pub(crate) fn store(self: &Arc<Self>, inner: Arc<dyn SessionStore>) -> Arc<dyn SessionStore> {
        Arc::new(VisionSessionStore {
            inner,
            accounting: self.clone(),
        })
    }
}

impl VisionUsageRecorder for VisionAccounting {
    fn record(&self, session: &SessionId, record: UsageRecord) {
        self.pending
            .lock()
            .expect("vision usage lock poisoned")
            .entry(session.clone())
            .or_default()
            .push(record);
    }
}

#[async_trait]
impl TurnCommitHook for VisionAccounting {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(
            "smith.vision.usage",
            RegistryRevision::new("smith-vision-usage-1"),
        )
    }

    async fn after_commit(&self, view: &TurnCommitView) -> Result<TurnCommitPatch, RuntimeError> {
        Ok(TurnCommitPatch {
            usage: self
                .pending
                .lock()
                .expect("vision usage lock poisoned")
                .remove(&view.session)
                .unwrap_or_default(),
            ..TurnCommitPatch::default()
        })
    }
}

#[derive(Debug)]
struct VisionSessionStore {
    inner: Arc<dyn SessionStore>,
    accounting: Arc<VisionAccounting>,
}

#[async_trait]
impl SessionStore for VisionSessionStore {
    async fn load(&self, session: &SessionId) -> Result<Option<SessionSnapshot>, RuntimeError> {
        self.inner.load(session).await
    }

    async fn save(&self, snapshot: &SessionSnapshot) -> Result<(), RuntimeError> {
        let mut snapshot = snapshot.clone();
        self.accounting.account_snapshot(&mut snapshot);
        self.inner.save(&snapshot).await
    }
}
