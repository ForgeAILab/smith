//! Local `/agent` results and their plain-text rendering.
//!
//! Lists, inspector cards, and exact-resume outcomes carry data. Terminal
//! drawing belongs to `smith-tui`; lifecycle labels are shared across clients.

use std::borrow::Cow;

use serde::Serialize;
use smith_runtime::{
    ChildDurability as RuntimeChildDurability, ChildState as RuntimeChildState, ChildStatus,
};

use crate::format::compact_tokens;

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
            Self::Resume(AgentResumeReport::Started { .. }) => "agent",
            Self::Resume(_) => "error",
            Self::Empty | Self::Unavailable | Self::Missing(_) | Self::List(_) => "agent",
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

    /// Labelled turn usage, with an optional finite ceiling.
    pub fn turns_label(&self) -> String {
        turns_label(self.turns_used, self.max_turns)
    }

    /// Coordinator-owned token usage in Smith's compact form.
    pub fn tokens_value(&self) -> String {
        compact_tokens(self.tokens_used)
    }
}

/// Labelled turn usage shared by child lists, inspectors, and panel rows.
pub fn turns_label(used: u32, maximum: Option<u32>) -> String {
    let value = maximum.map_or_else(|| used.to_string(), |max| format!("{used}/{max}"));
    let unit = if maximum.unwrap_or(used) == 1 {
        "turn"
    } else {
        "turns"
    };
    format!("{value} {unit}")
}

/// The extra coordinator fields shown when one child is inspected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSnapshot {
    /// Identity, lifecycle, and accounting shared with the list.
    pub summary: AgentSummary,
    /// Child session identity.
    pub session: String,
    /// Workspace in the reviewed spawn row's words.
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
            workspace: smith_runtime::delegation::agent_workspace_display(&status.workspace)
                .unwrap_or_else(|| "directory unknown".to_owned()),
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

/// Shared human-readable exact-checkpoint availability for child surfaces.
pub fn exact_resume_label(resumable: bool) -> &'static str {
    if resumable {
        "exact resume available"
    } else {
        "no exact checkpoint"
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
                    "{} · {} · {} · {} · {} · {} tokens",
                    child.child,
                    child.durability.label(),
                    child.state.label(),
                    exact_resume_label(child.resumable),
                    child.turns_label(),
                    child.tokens_value(),
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        AgentReport::Inspector(child) => {
            let mut text = format!(
                "session {} · {} · {} · {} · {} tokens · {}\n{}{}\ncontinue: type a follow-up below",
                child.session,
                child.summary.durability.label(),
                child.summary.state.label(),
                child.summary.turns_label(),
                child.summary.tokens_value(),
                child.workspace,
                exact_resume_label(child.summary.resumable),
                child
                    .incompatibility
                    .as_deref()
                    .map(|reason| format!(" · incompatible: {reason}"))
                    .unwrap_or_default(),
            );
            if child.summary.resumable {
                text.push_str(&format!(
                    "\nexact recovery: /agent resume {}",
                    child.summary.child
                ));
            }
            if let Some(result) = &child.last_result {
                text.push_str(&format!("\nresult\n{result}"));
            }
            text
        }
        AgentReport::Resume(resume) => resume.render_value(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::cancel::CancelReason;

    fn live_findings_snapshot(resumable: bool) -> AgentSnapshot {
        AgentSnapshot {
            summary: AgentSummary {
                child: "child-1".to_owned(),
                durability: ChildDurability::Durable,
                state: ChildState::Idle,
                resumable,
                turns_used: 1,
                max_turns: None,
                tokens_used: 3_100,
            },
            session: "child-session-1".to_owned(),
            workspace: "read only".to_owned(),
            incompatibility: None,
            last_result: Some("**4 entries** in `src/lib.rs`.".to_owned()),
        }
    }

    #[test]
    fn live_findings_agent_reports_name_the_command_and_label_compact_counts() {
        for (used, max, expected) in [
            (0, None, "0 turns"),
            (1, None, "1 turn"),
            (2, None, "2 turns"),
            (1, Some(1), "1/1 turn"),
            (1, Some(5), "1/5 turns"),
        ] {
            let mut snapshot = live_findings_snapshot(false);
            snapshot.summary.turns_used = used;
            snapshot.summary.max_turns = max;
            let list = AgentReport::List(vec![snapshot.summary]);
            assert_eq!(list.title(), "agent");
            let plain = render_plain(&list);
            assert!(
                plain.contains(&format!(" · {expected} · 3.1k tokens")),
                "{plain}"
            );
        }
        for report in [
            AgentReport::Empty,
            AgentReport::Unavailable,
            AgentReport::Missing("child-1".to_owned()),
            AgentReport::Resume(AgentResumeReport::Started {
                child: "child-1".to_owned(),
            }),
        ] {
            assert_eq!(report.title(), "agent");
        }
    }

    #[test]
    fn live_findings_plain_inspector_states_facts_once_and_gates_exact_recovery() {
        for resumable in [false, true] {
            let snapshot = live_findings_snapshot(resumable);
            let text = render_plain(&AgentReport::Inspector(snapshot));
            for fact in [
                "session child-session-1",
                "durable",
                "idle",
                "1 turn",
                "3.1k tokens",
                "read only",
                exact_resume_label(resumable),
                "continue: type a follow-up below",
                "result\n**4 entries** in `src/lib.rs`.",
            ] {
                assert_eq!(text.matches(fact).count(), 1, "{fact}: {text}");
            }
            assert_eq!(
                text.contains("exact recovery: /agent resume child-1"),
                resumable,
                "{text}"
            );
            assert!(!text.contains("no activity"), "{text}");
        }
    }

    #[test]
    fn child_details_use_spawn_workspace_words_and_exact_resume_words() {
        use agent_runtime_core::clock::Timestamp;
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::ids::{ChildId, SessionId};

        for (workspace, expected_workspace) in [
            (WorkspacePolicy::SharedProject, "shared"),
            (
                WorkspacePolicy::ExplicitDirectory {
                    path: "/repo/child".to_owned(),
                },
                "/repo/child",
            ),
            (WorkspacePolicy::IsolatedWorktree, "isolated worktree"),
            (WorkspacePolicy::ReadOnlyView, "read only"),
        ] {
            for (resumable, expected_resume) in [
                (true, "exact resume available"),
                (false, "no exact checkpoint"),
            ] {
                let status = ChildStatus {
                    child: ChildId::new("child"),
                    parent: SessionId::new("parent"),
                    session: SessionId::new("session"),
                    durability: RuntimeChildDurability::Durable,
                    state: RuntimeChildState::Interrupted { resumable },
                    workspace: workspace.clone(),
                    turns_used: 1,
                    max_turns: 5,
                    tokens_used: 2,
                    last_result: None,
                    last_artifacts: Vec::new(),
                    updated_at: Timestamp(0),
                    incompatibility: None,
                    last_error: None,
                };
                let snapshot = AgentSnapshot::from(&status);
                assert_eq!(snapshot.workspace, expected_workspace);
                let inspector = render_plain(&AgentReport::Inspector(snapshot.clone()));
                assert!(
                    inspector
                        .lines()
                        .next()
                        .unwrap()
                        .ends_with(expected_workspace)
                );
                assert_eq!(inspector.lines().nth(1), Some(expected_resume));
                let list = render_plain(&AgentReport::List(vec![snapshot.summary]));
                assert!(list.contains(&format!(" · {expected_resume} · ")), "{list}");
                for rendered in [&inspector, &list] {
                    assert!(!rendered.contains("resumable true"), "{rendered}");
                    assert!(!rendered.contains("resumable false"), "{rendered}");
                }
            }
        }
    }

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
                workspace: "read only".to_owned(),
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
