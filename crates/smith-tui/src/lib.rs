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
//! - [`render`] — the pure draw function.
//!
//! `App` performs no I/O and owns no runtime handle. The host loop in
//! `smith-cli` feeds it events and performs the [`Action`](app::Action)s it
//! returns, so the same state machine can be driven by a test with no
//! terminal, no provider, and no clock.
//!
//! The visual contract these modules implement is `DESIGN.md` at the repository
//! root; section references in the code point there.

pub mod accounts;
pub mod app;
pub mod commands;
pub mod composer;
pub mod diff;
pub mod picker;
pub mod questionnaire;
pub mod references;
pub mod render;
pub mod selection;
pub mod setup;
pub mod status;
pub mod theme;
pub mod transcript;
pub mod usage_log;

pub use app::{
    Action, App, MouseOutcome, Overlay, PaletteCommand, PendingInputPreview, PreparedSubmission,
    ResourceTarget, RuntimeResources, SubmissionTarget,
};
pub use commands::{COMMANDS, CommandAction, CommandSpec, GoalAction, McpAction};
pub use composer::Composer;
pub use diff::{Change, EditReview, diff_lines};
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
pub use setup::{
    SetupApp, SetupCredential, SetupEffect, SetupMode, SetupModelLimits, SetupSubmission,
    draw_setup,
};
pub use status::{
    Activity, Confidence, ContextPlanStatus, ContextPlanUpdate, McpStatus, Status, TokenCount,
};
pub use theme::{Theme, Tone};
pub use transcript::{Block, LocalResultState, ToolStatus, Transcript};
