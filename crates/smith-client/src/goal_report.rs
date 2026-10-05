//! The local `/goal` snapshot and its plain-text rendering.
//!
//! Availability and goal fields travel as data. Transcript and headless text
//! draw the same snapshot with their existing presentation.

use std::time::Duration;

use agent_runtime_core::goal::GoalProjection;

use crate::format::plural;
use crate::status::render_elapsed;

/// The result of showing or changing a persistent goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalReport {
    /// A persisted root session has no goal.
    Empty,
    /// The goal was successfully cleared.
    Cleared,
    /// The host could not read or change the goal.
    Unavailable(String),
    /// The goal after the requested operation.
    Snapshot(GoalSnapshot),
}

impl GoalReport {
    /// The existing guidance for a persisted session without a goal.
    pub const EMPTY_MESSAGE: &str = "No persistent goal. Create one with `/goal <objective>`.";
    /// The existing acknowledgement after clearing a goal.
    pub const CLEARED_MESSAGE: &str = "Goal cleared.";
    /// The existing explanation when persistent goals are unavailable.
    pub const UNAVAILABLE_MESSAGE: &str = "Persistent goals require a persisted root session; they are unavailable in ephemeral and child sessions.";
}

/// Goal information captured when `/goal` is invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalSnapshot {
    /// User-supplied objective.
    pub objective: String,
    /// Runtime-owned lifecycle label.
    pub status: String,
    /// Charged tokens; absent when usage is unknown.
    pub charged_tokens: Option<u64>,
    /// Token budget; absent for an unbudgeted goal.
    pub token_budget: Option<u64>,
    /// Runtime-owned token usage provenance label.
    pub usage_provenance: String,
    /// Active elapsed time in the existing display format.
    pub active_elapsed: String,
    /// Stopping reason; absent when the goal has no reason.
    pub stopped_reason: Option<GoalStoppedReason>,
    /// Stable goal identity.
    pub id: String,
    /// Monotonic goal generation.
    pub generation: u64,
}

impl From<&GoalProjection> for GoalSnapshot {
    fn from(goal: &GoalProjection) -> Self {
        Self {
            objective: goal.objective.clone(),
            status: goal.status.as_str().to_owned(),
            charged_tokens: goal.usage.charged_tokens,
            token_budget: goal.token_budget,
            usage_provenance: goal.usage.provenance.as_str().to_owned(),
            active_elapsed: render_elapsed(Duration::from_millis(goal.usage.active_elapsed_ms)),
            stopped_reason: goal
                .stopped_reason
                .as_ref()
                .map(|reason| GoalStoppedReason {
                    code: reason.code.clone(),
                    detail: reason.detail.clone(),
                }),
            id: goal.id.to_string(),
            generation: goal.generation,
        }
    }
}

impl GoalSnapshot {
    /// Charged usage without a field label or provenance.
    pub fn charged_tokens_value(&self) -> String {
        self.charged_tokens
            .map_or_else(|| "unknown".to_owned(), |tokens| tokens.to_string())
    }

    /// Token budget without a field label.
    pub fn budget_value(&self) -> String {
        self.token_budget
            .map_or_else(|| "none".to_owned(), |tokens| tokens.to_string())
    }

    /// The existing stopping reason value, including its availability.
    pub fn reason_value(&self) -> String {
        self.stopped_reason
            .as_ref()
            .map_or_else(|| "none".to_owned(), GoalStoppedReason::render_value)
    }
}

/// A goal's stopping reason, preserving the code and optional detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalStoppedReason {
    /// Runtime-owned reason code.
    pub code: String,
    /// Optional explanation accompanying the code.
    pub detail: Option<String>,
}

impl GoalStoppedReason {
    /// The existing stopping reason value, without a field label.
    pub fn render_value(&self) -> String {
        self.detail.as_ref().map_or_else(
            || self.code.clone(),
            |detail| format!("{} · {detail}", self.code),
        )
    }
}

/// Renders the transcript body for the existing plain-text capture surface.
/// No terminal libraries or title/label parsing are involved.
pub fn render_plain(report: &GoalReport) -> String {
    let goal = match report {
        GoalReport::Empty => return GoalReport::EMPTY_MESSAGE.to_owned(),
        GoalReport::Cleared => return GoalReport::CLEARED_MESSAGE.to_owned(),
        GoalReport::Unavailable(error) => return error.clone(),
        GoalReport::Snapshot(goal) => goal,
    };
    format!(
        "{}\nstatus: {}\ntokens: {} · {}\nbudget: {}\nactive elapsed: {}\nreason: {}\nid: {} · generation {}",
        goal.objective,
        goal.status,
        goal.charged_tokens_value(),
        goal.usage_provenance,
        goal.budget_value(),
        goal.active_elapsed,
        goal.reason_value(),
        goal.id,
        goal.generation,
    )
}

/// Renders headless metadata lines, without the stderr source prefix.
/// Headless exposes snapshots only; absent goals produce no metadata lines.
/// Each returned string keeps its original line boundaries, including any
/// newlines in a runtime-owned stopping reason.
pub fn render_headless_plain(report: &GoalReport, continuation_turns: u32) -> Vec<String> {
    let GoalReport::Snapshot(goal) = report else {
        return Vec::new();
    };
    let mut lines = vec![format!(
        "goal: {} · {} tokens · budget {} · {}",
        goal.status,
        goal.charged_tokens_value(),
        goal.budget_value(),
        plural(
            continuation_turns,
            "continuation turn",
            "continuation turns"
        ),
    )];
    if let Some(reason) = &goal.stopped_reason {
        lines.push(format!("goal reason: {}", reason.render_value()));
    }
    lines
}
