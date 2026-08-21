//! Immutable harness acceptance stage.

use std::sync::Arc;

use crate::factory::RuntimeRequest;
use crate::harness::{HarnessIdentity, HarnessResolutionReport, ResolvedHarness, ResolvedModule};

pub(super) struct Stage {
    pub(super) identity: HarnessIdentity,
    pub(super) modules: Arc<[ResolvedModule]>,
    pub(super) report: HarnessResolutionReport,
    pub(super) request: RuntimeRequest,
}

pub(super) fn accept(harness: ResolvedHarness) -> Stage {
    Stage {
        identity: harness.identity.clone(),
        modules: Arc::from(harness.modules.clone().into_boxed_slice()),
        report: harness.report.clone(),
        request: harness.into_request(),
    }
}
