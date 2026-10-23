//! Final neutral Agent Runtime composition stage.

use super::*;

pub(super) async fn runtime(builder: RuntimeBuilder) -> Result<BuilderStage, FactoryError> {
    // Cold schema compilation is CPU-bound and can exhaust a deeply nested
    // async startup stack before the registry's shared validators are warm.
    let runtime = tokio::task::spawn_blocking(move || builder.build())
        .await
        .map_err(|error| {
            FactoryError::Runtime(RuntimeError::config(format!(
                "runtime validation worker failed: {error}"
            )))
        })?
        .map_err(FactoryError::Runtime)?;
    Ok(BuilderStage { runtime })
}
