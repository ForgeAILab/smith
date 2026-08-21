//! Workspace and approval-authority resolution stage.

use std::sync::Arc;

use agent_runtime_core::approval::ApprovalPolicy;
use agent_runtime_core::workspace::Workspace;

use super::{FactoryError, RuntimeRequest, approval, require_workspace};

pub(super) struct Stage {
    pub(super) workspace: Arc<dyn Workspace>,
    pub(super) approval: Arc<dyn ApprovalPolicy>,
}

pub(super) fn prepare(request: &RuntimeRequest) -> Result<Stage, FactoryError> {
    Ok(Stage {
        workspace: require_workspace(request)?,
        approval: approval(request)?,
    })
}
