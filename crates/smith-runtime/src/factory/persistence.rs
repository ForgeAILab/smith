//! Checkpoint and persistence resolution stage.

use super::{DurabilityStage, FactoryError, RuntimeRequest, prepare_durability_stage};

pub(super) async fn prepare(request: &RuntimeRequest) -> Result<DurabilityStage, FactoryError> {
    prepare_durability_stage(request).await
}
