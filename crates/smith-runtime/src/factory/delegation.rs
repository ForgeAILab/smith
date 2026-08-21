//! Root delegation assembly stage.

use super::{DelegationStage, SmithDelegation, assemble_delegation};

pub(super) fn assemble(delegation: Option<SmithDelegation>) -> DelegationStage {
    assemble_delegation(delegation)
}
