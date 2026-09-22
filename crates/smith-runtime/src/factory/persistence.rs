//! Checkpoint and persistence resolution stage.

use super::*;

pub(super) async fn prepare(request: &RuntimeRequest) -> Result<DurabilityStage, FactoryError> {
    if request.checkpoint_store.is_some() && request.checkpoint_setup.is_some() {
        return Err(FactoryError::Runtime(RuntimeError::config(
            "checkpoint_store and checkpoint_setup cannot both be supplied",
        )));
    }
    let store = match (
        request.checkpoint_store.clone(),
        request.checkpoint_setup.as_ref(),
    ) {
        (Some(store), None) => Some(store),
        (None, Some(setup)) => match setup.initialize().await {
            Ok(store) => Some(store),
            Err(error) => {
                tracing::warn!(
                    schema_version = CHECKPOINT_SCHEMA_VERSION,
                    %error,
                    "exact mid-turn durability is unavailable; completed-turn persistence remains enabled"
                );
                None
            }
        },
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!("conflicting checkpoint inputs returned above"),
    };
    // Child sessions share the protected store, never the root journal
    // barrier, which is scoped to the root writer.
    let child_store = store.clone();
    let root_store = match (store, request.checkpoint_barrier.clone()) {
        (Some(store), Some(barrier)) => {
            Some(Arc::new(BarrierCheckpointStore::new(store, barrier)) as Arc<dyn CheckpointStore>)
        }
        (store, None) => store,
        (None, Some(_)) => None,
    };
    let status = if root_store.is_some() {
        MidTurnDurability::Available
    } else {
        MidTurnDurability::Unavailable
    };
    Ok(DurabilityStage {
        root_store,
        child_store,
        status,
    })
}
