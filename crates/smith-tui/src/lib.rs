//! The Smith terminal client.
//!
//! The crate is split so that everything except the final draw call is
//! testable without a terminal:
//!
//! - [`app`] — state, the reducer over runtime events, and the key map.
//! - [`transcript`] — conversation history as addressable blocks.
//! - [`status`] — header status and the estimated/unknown honesty rules.
//! - [`composer`] — the input buffer.
//! - [`diff`] — the line differ behind the approval modal's edit review.
//! - [`theme`] — colors and glyphs.
//! - [`selection`] — pointer selection, which Smith owns because enabling the
//!   wheel takes the terminal's own selection away.
//! - [`render`] — layout computation, scroll synchronization, and drawing.
//!
//! `App` owns presentation state, elapsed-time clocks, and approval and
//! questionnaire replies. The host loop in `smith-cli` feeds it events and
//! performs the returned [`Action`](app::Action)s, including runtime, filesystem,
//! and terminal operations. State and rendering tests can run without a live
//! terminal or provider; timing-sensitive paths still consult clocks.
//!
//! The visual contract these modules implement is `DESIGN.md` at the repository
//! root; section references in the code point there.

pub mod accounts;
pub mod app;
pub mod composer;
pub mod diff;
mod hyperlink;
pub mod line_input;
pub mod picker;
pub mod questionnaire;
pub mod references;
pub mod render;
pub mod screen;
pub mod selection;
pub mod setup;
pub mod status;
pub mod terminal_title;
pub mod theme;
pub mod transcript;

// Shared session projections; terminal rendering remains in this crate.
pub use smith_client::{cache, commands, time_display, usage_log};

pub use app::{
    Action, App, ConfirmDialog, ConfirmOutcome, MouseOutcome, Overlay, PendingInputPreview,
    PreparedSubmission, ResourceTarget, RuntimeResources, SubmissionTarget,
};
pub use cache::{
    CacheLifecycleSummary, CacheOperationDisposition, CacheOperationSummary, CachePrice,
    CacheProjection, CacheTurnSummary, CacheVisibilityState, MISS_NOTICE_COST_MICRO_USD,
    MISS_NOTICE_TOKENS,
};
pub use commands::{
    AdvisorChoice, COMMANDS, Command, CommandSpec, ConfirmCommand, GoalAction, HostCommand,
    McpAction, SelectionCommand, SessionControl, SkillsAction, UiCommand,
};
pub use composer::Composer;
pub use diff::{Change, EditReview, diff_lines};
pub use line_input::LineInput;
pub use picker::{PickerOutcome, ResourceEntry, ResourcePicker, draw_resource_picker};
pub use questionnaire::{
    QuestionnaireAnswer, QuestionnaireAnswerValue, QuestionnaireChoice, QuestionnaireFocus,
    QuestionnaireForm, QuestionnaireQuestion, QuestionnaireResolution, QuestionnaireState,
    QuestionnaireValidationError,
};
pub use references::{
    ComposerReference, MAX_COMPOSER_REFERENCES, ParsedReferences, parse_references,
};
pub use render::{draw, draw_synced, selected_text};
pub use screen::{FlowOutcome, Screen, ScreenEvent, Step};
pub use setup::{
    ResolveModelLimits, ResolvedModelLimits, SetupApp, SetupCredential, SetupEffect, SetupMode,
    SetupModelLimits, SetupSubmission, draw_setup,
};
pub use status::{
    Activity, Confidence, ContextPlanStatus, ContextPlanUpdate, McpStatus, Status, TokenCount,
};
pub use theme::{Theme, Tone};
pub use transcript::{Block, LocalResult, LocalResultState, ToolStatus, Transcript};
