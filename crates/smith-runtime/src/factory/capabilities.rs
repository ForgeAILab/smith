//! Built-in, trusted-native, MCP, and delegation-tool capability stage.

use super::{
    AgentToolProfile, CapabilityStage, FactoryError, RuntimeRequest, prepare_capability_stage,
};

pub(super) fn prepare(
    request: &RuntimeRequest,
    profiles: Vec<AgentToolProfile>,
) -> Result<CapabilityStage, FactoryError> {
    prepare_capability_stage(request, profiles)
}
