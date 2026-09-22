//! Final neutral Agent Runtime composition stage.

use super::*;

pub(super) fn runtime(builder: RuntimeBuilder) -> Result<BuilderStage, FactoryError> {
    Ok(BuilderStage {
        runtime: builder.build().map_err(FactoryError::Runtime)?,
    })
}
