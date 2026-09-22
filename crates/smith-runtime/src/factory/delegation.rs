//! Root delegation assembly stage.

use super::*;

pub(super) fn assemble(delegation: Option<SmithDelegation>) -> DelegationStage {
    DelegationStage { delegation }
}
