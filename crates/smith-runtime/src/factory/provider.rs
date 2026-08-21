//! Provider, credential, profile, and context-policy resolution stage.

use super::{FactoryError, PreparedFactoryInputs, RuntimeRequest, prepare_factory_inputs};

pub(super) async fn prepare(
    request: &RuntimeRequest,
) -> Result<PreparedFactoryInputs, FactoryError> {
    prepare_factory_inputs(request).await
}
