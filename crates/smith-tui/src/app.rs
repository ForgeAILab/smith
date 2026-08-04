//! Application state facade.
//!
//! The implementation is split by state-transition ownership under `app/`;
//! the public paths remain rooted at `smith_tui::app`.

mod input;
mod pending_input;
mod prompts;
mod reducer;
mod resources;
mod state;

pub(crate) use state::MAX_PENDING_PREVIEW_ENTRIES;
pub use state::{
    Action, App, ChildSummary, LEGACY_AGENT_PROFILE_PREFIX, Overlay, PaletteCommand,
    PendingInputPreview, PlanSummary, PreparedSubmission, ProviderPhase, ResourceTarget,
    RuntimeResources, StreamGap, SubmissionTarget,
};
