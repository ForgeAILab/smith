//! Local `/agent` results and their plain-text rendering.
//!
//! Lists, inspector cards, and exact-resume outcomes carry data. Terminal
//! drawing belongs to `smith-tui`; lifecycle labels are shared across clients.

use std::borrow::Cow;

use serde::Serialize;
use smith_runtime::{
    ChildDurability as RuntimeChildDurability, ChildState as RuntimeChildState, ChildStatus,
};

/// Client-owned lifecycle data, independent of its display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildState {
    /// The child has an active turn.
    Running,
    /// The child has no active turn.
    Idle,
    /// The child's turn was interrupted.
    Interrupted {
        /// Whether the runtime retained an exact checkpoint.
        resumable: bool,
    },
    /// The child was stopped.
    Stopped {
        /// Readable stopping reason.
        reason: String,
    },
    /// The child failed.
    Failed,
    /// The child's retained session expired.
    Expired,
}

impl From<&RuntimeChildState> for ChildState {
    fn from(state: &RuntimeChildState) -> Self {
        match state {
            RuntimeChildState::Running => Self::Running,
            RuntimeChildState::Idle => Self::Idle,
            RuntimeChildState::Interrupted { resumable } => Self::Interrupted {
                resumable: *resumable,
            },
            RuntimeChildState::Stopped { reason } => Self::stopped(reason),
            RuntimeChildState::Failed => Self::Failed,
            RuntimeChildState::Expired => Self::Expired,
        }
    }
}

impl ChildState {
    /// Captures a typed runtime stopping reason as readable words.
    pub fn stopped(reason: &agent_runtime_core::cancel::CancelReason) -> Self {
        use agent_runtime_core::cancel::CancelReason;
        Self::Stopped {
            reason: match reason {
                CancelReason::UserRequested => "by request".to_owned(),
                CancelReason::Timeout => "deadline elapsed".to_owned(),
                CancelReason::LimitReached => "limit reached".to_owned(),
                CancelReason::Shutdown => "session ended".to_owned(),
                CancelReason::Host(reason) => reason.clone(),
            },
        }
    }

    /// Shared lifecycle label for human-readable child surfaces.
    pub fn label(&self) -> Cow<'static, str> {
        match self {
            Self::Running => "running".into(),
            Self::Idle => "idle".into(),
            Self::Interrupted { resumable: true } => "interrupted (resumable)".into(),
            Self::Interrupted { resumable: false } => "interrupted (not resumable)".into(),
            Self::Stopped { reason } => format!("stopped ({reason})").into(),
            Self::Failed => "failed".into(),
            Self::Expired => "expired".into(),
        }
    }

    fn machine_label(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Interrupted { .. } => "interrupted",
            Self::Stopped { .. } => "stopped",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }
}

/// Client-owned child persistence, independent of its display label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildDurability {
    /// The child belongs only to the current process.
    Ephemeral,
    /// The child's session is persisted.
    Durable,
}

impl From<&RuntimeChildDurability> for ChildDurability {
    fn from(durability: &RuntimeChildDurability) -> Self {
        match durability {
            RuntimeChildDurability::Ephemeral => Self::Ephemeral,
            RuntimeChildDurability::Durable => Self::Durable,
        }
    }
}

impl ChildDurability {
    /// Shared durability label for child surfaces.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ephemeral => "ephemeral",
            Self::Durable => "durable",
        }
    }
}

/// The result of listing, inspecting, navigating, or resuming a child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentReport {
    /// No children are retained in this session.
    Empty,
    /// Child delegation is unavailable.
    Unavailable,
    /// The requested child is absent from the coordinator.
    Missing(String),
    /// The inspector was closed in favor of the root timeline.
    Parent,
    /// Children in the coordinator's existing order.
    List(Vec<AgentSummary>),
    /// The authoritative card displayed in the child inspector.
    Inspector(AgentSnapshot),
    /// Exact-checkpoint resume validation or execution result.
    Resume(AgentResumeReport),
}

impl AgentReport {
    /// The existing guidance for a session without children.
    pub const EMPTY_MESSAGE: &str = "No child agents in this session.";
    /// The existing explanation when delegation is unavailable.
    pub const UNAVAILABLE_MESSAGE: &str = "Child delegation is unavailable for this session.";
    /// The existing acknowledgement after leaving child inspection.
    pub const PARENT_MESSAGE: &str =
        "Returned to the root timeline; the root composer remained focused.";

    /// Existing result title or notice source, selected from the result kind.
    pub fn title(&self) -> &str {
        match self {
            Self::Parent | Self::Inspector(_) => "agent",
            Self::Resume(AgentResumeReport::RequiresIdle) => "agent",
            Self::Resume(AgentResumeReport::Started { .. }) => "agents",
            Self::Resume(_) => "error",
            Self::Empty | Self::Unavailable | Self::Missing(_) | Self::List(_) => "agents",
        }
    }
}

/// Coordinator-owned identity, lifecycle, and accounting for one child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSummary {
    /// Stable child identity.
    pub child: String,
    /// Child persistence, rendered with shared labels.
    pub durability: ChildDurability,
    /// Child lifecycle, rendered with shared labels.
    pub state: ChildState,
    /// Whether exact recovery is available.
    pub resumable: bool,
    /// Coordinator-reported turn usage.
    pub turns_used: u32,
    /// Finite turn limit; absent for an unbounded child.
    pub max_turns: Option<u32>,
    /// Coordinator-reported token usage.
    pub tokens_used: u64,
}

impl From<&ChildStatus> for AgentSummary {
    fn from(status: &ChildStatus) -> Self {
        Self {
            child: status.child.to_string(),
            durability: ChildDurability::from(&status.durability),
            state: ChildState::from(&status.state),
            resumable: status.resumable(),
            turns_used: status.turns_used,
            max_turns: (status.max_turns != u32::MAX).then_some(status.max_turns),
            tokens_used: status.tokens_used,
        }
    }
}

impl AgentSummary {
    /// Existing used/maximum turn value, without the field label.
    pub fn turns_value(&self) -> String {
        self.max_turns.map_or_else(
            || self.turns_used.to_string(),
            |max| format!("{}/{max}", self.turns_used),
        )
    }
}

/// The extra coordinator fields shown when one child is inspected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSnapshot {
    /// Identity, lifecycle, and accounting shared with the list.
    pub summary: AgentSummary,
    /// Child session identity.
    pub session: String,
    /// Existing debug-formatted workspace policy.
    pub workspace: String,
    /// Exact-resume incompatibility, if present.
    pub incompatibility: Option<String>,
    /// Last child result; absent when no result is available.
    pub last_result: Option<String>,
}

impl From<&ChildStatus> for AgentSnapshot {
    fn from(status: &ChildStatus) -> Self {
        Self {
            summary: AgentSummary::from(status),
            session: status.session.to_string(),
            workspace: format!("{:?}", status.workspace),
            incompatibility: status.incompatibility.clone(),
            last_result: status.last_result.clone(),
        }
    }
}

impl AgentSnapshot {
    /// Renders the redaction-safe headless serialization view of this report.
    /// Headless has no child text line; workspace and result content stay out
    /// of its machine lifecycle metadata.
    pub fn into_headless_output(self) -> HeadlessAgentOutput {
        HeadlessAgentOutput {
            child_id: self.summary.child,
            child_session_id: self.session,
            durability: self.summary.durability.label().to_owned(),
            state: self.summary.state.machine_label().to_owned(),
            resumable: self.summary.resumable,
            turns_used: self.summary.turns_used,
            max_turns: self.summary.max_turns,
            tokens_used: self.summary.tokens_used,
            incompatibility: self.incompatibility,
        }
    }
}

/// Existing headless child fields rendered from an inspector snapshot.
#[derive(Debug, Serialize)]
pub struct HeadlessAgentOutput {
    /// Stable child identity.
    pub child_id: String,
    /// Child session identity.
    pub child_session_id: String,
    /// Headless durability label.
    pub durability: String,
    /// Headless lifecycle label.
    pub state: String,
    /// Whether exact recovery is available.
    pub resumable: bool,
    /// Coordinator-reported turn usage.
    pub turns_used: u32,
    /// Finite turn limit; omitted for an unbounded child.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// Coordinator-reported token usage.
    pub tokens_used: u64,
    /// Exact-resume incompatibility, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incompatibility: Option<String>,
}

/// Validation and execution outcomes for `/agent resume`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentResumeReport {
    /// The root must become idle before confirmation can be shown.
    RequiresIdle,
    /// The requested child is absent from the client's retained children.
    Missing {
        /// Requested child identity.
        child: String,
    },
    /// The child has no compatible interrupted checkpoint.
    Incompatible {
        /// Requested child identity.
        child: String,
    },
    /// The host has no coordinator to execute the confirmed resume.
    Unavailable,
    /// The coordinator accepted the exact-checkpoint resume.
    Started {
        /// Resumed child identity.
        child: String,
    },
    /// The coordinator rejected the resume.
    Failed {
        /// Requested child identity.
        child: String,
        /// Coordinator-owned explanation.
        error: String,
    },
}

impl AgentResumeReport {
    /// The existing outcome wording, without its notice or error marker.
    pub fn render_value(&self) -> String {
        match self {
            Self::RequiresIdle => {
                "exact child resume requires an idle root turn; draft preserved".to_owned()
            }
            Self::Missing { child } => {
                format!("No child named `{child}`; use `/agent` to list retained children.")
            }
            Self::Incompatible { child } => format!(
                "`{child}` has no compatible interrupted checkpoint; inspect it with `/agent {child}`"
            ),
            Self::Unavailable => {
                "child resume is unavailable because the coordinator is not wired".to_owned()
            }
            Self::Started { child } => {
                format!("{child} exact checkpoint resume started · no new child task")
            }
            Self::Failed { child, error } => format!("{child} did not resume: {error}"),
        }
    }
}

/// Renders the transcript or inspector body for the plain-text capture surface.
/// No terminal libraries or title/label parsing are involved.
pub fn render_plain(report: &AgentReport) -> String {
    match report {
        AgentReport::Empty => AgentReport::EMPTY_MESSAGE.to_owned(),
        AgentReport::Unavailable => AgentReport::UNAVAILABLE_MESSAGE.to_owned(),
        AgentReport::Missing(child) => format!("No child named `{child}`."),
        AgentReport::Parent => AgentReport::PARENT_MESSAGE.to_owned(),
        AgentReport::List(children) => children
            .iter()
            .map(|child| {
                format!(
                    "{} · {} · {} · resumable {} · {} turns · {} tokens",
                    child.child,
                    child.durability.label(),
                    child.state.label(),
                    child.resumable,
                    child.turns_value(),
                    child.tokens_used,
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        AgentReport::Inspector(child) => format!(
            "session {} · {} · {} · {} · {} tokens · {}\nresumable {}{}\ncontinue: type a follow-up below · exact recovery: /agent resume {}\nresult: {}",
            child.session,
            child.summary.durability.label(),
            child.summary.state.label(),
            child.summary.turns_value(),
            child.summary.tokens_used,
            child.workspace,
            child.summary.resumable,
            child
                .incompatibility
                .as_deref()
                .map(|reason| format!(" · incompatible: {reason}"))
                .unwrap_or_default(),
            child.summary.child,
            child.last_result.as_deref().unwrap_or("not available"),
        ),
        AgentReport::Resume(resume) => resume.render_value(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::cancel::CancelReason;

    #[test]
    fn child_labels_are_readable_and_preserve_host_reason_text() {
        for (state, label) in [
            (RuntimeChildState::Running, "running"),
            (RuntimeChildState::Idle, "idle"),
            (
                RuntimeChildState::Interrupted { resumable: true },
                "interrupted (resumable)",
            ),
            (
                RuntimeChildState::Interrupted { resumable: false },
                "interrupted (not resumable)",
            ),
            (
                RuntimeChildState::Stopped {
                    reason: CancelReason::UserRequested,
                },
                "stopped (by request)",
            ),
            (
                RuntimeChildState::Stopped {
                    reason: CancelReason::Timeout,
                },
                "stopped (deadline elapsed)",
            ),
            (
                RuntimeChildState::Stopped {
                    reason: CancelReason::LimitReached,
                },
                "stopped (limit reached)",
            ),
            (
                RuntimeChildState::Stopped {
                    reason: CancelReason::Shutdown,
                },
                "stopped (session ended)",
            ),
            (
                RuntimeChildState::Stopped {
                    reason: CancelReason::Host("Keep CASE and IDs".to_owned()),
                },
                "stopped (Keep CASE and IDs)",
            ),
            (RuntimeChildState::Failed, "failed"),
            (RuntimeChildState::Expired, "expired"),
        ] {
            assert_eq!(ChildState::from(&state).label(), label);
        }
        assert_eq!(ChildDurability::Durable.label(), "durable");
        assert_eq!(ChildDurability::Ephemeral.label(), "ephemeral");
    }

    #[test]
    fn child_machine_output_keeps_unqualified_state_and_existing_fields() {
        for (state, machine, resumable) in [
            (ChildState::Running, "running", false),
            (ChildState::Idle, "idle", false),
            (
                ChildState::Interrupted { resumable: true },
                "interrupted",
                true,
            ),
            (
                ChildState::Interrupted { resumable: false },
                "interrupted",
                false,
            ),
            (
                ChildState::stopped(&CancelReason::LimitReached),
                "stopped",
                false,
            ),
            (ChildState::Failed, "failed", false),
            (ChildState::Expired, "expired", false),
        ] {
            let snapshot = AgentSnapshot {
                summary: AgentSummary {
                    child: "child".to_owned(),
                    durability: ChildDurability::Durable,
                    state,
                    resumable,
                    turns_used: 1,
                    max_turns: None,
                    tokens_used: 2,
                },
                session: "session".to_owned(),
                workspace: "ReadOnlyView".to_owned(),
                incompatibility: None,
                last_result: None,
            };
            assert_eq!(
                serde_json::to_string(&snapshot.into_headless_output()).expect("machine output"),
                format!(
                    r#"{{"child_id":"child","child_session_id":"session","durability":"durable","state":"{machine}","resumable":{resumable},"turns_used":1,"tokens_used":2}}"#
                ),
            );
        }
    }
}
