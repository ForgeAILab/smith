//! Smith's host-policy layer over the shared agent runtime.
//!
//! The runtime in `../agent-runtime` owns mechanism — the provider/tool loop,
//! events, usage, cancellation — and deliberately ships none of the policy a
//! product needs. This crate supplies Smith's half of that contract:
//!
//! - [`approval`] — the interactive approval gate a terminal user answers.
//! - [`workspace`] — the project-root write boundary.
//!
//! The dependency is one-way by design: Smith depends on the runtime; the
//! runtime never depends on Smith.

pub mod approval;
pub mod changes;
pub mod workspace;

pub use approval::{
    ApprovalPrompt, ApprovalRequests, ApprovalRequired, HeadlessApproval, InteractiveApproval,
    PromptScope,
};
pub use changes::{AppliedRevert, ChangeView, GitChanges, RevertPreview};
pub use workspace::ProjectWorkspace;
