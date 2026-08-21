//! Final neutral Agent Runtime composition stage.

use agent_runtime::runtime::RuntimeBuilder;

use super::{BuilderStage, FactoryError, build_runtime};

pub(super) fn runtime(builder: RuntimeBuilder) -> Result<BuilderStage, FactoryError> {
    build_runtime(builder)
}
