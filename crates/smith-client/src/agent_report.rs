//! Local `/agent` results and their plain-text rendering.
//!
//! Lists, inspector cards, and exact-resume outcomes carry data. Terminal
//! drawing belongs to `smith-tui`; lifecycle labels retain their current text.

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
    /// Existing debug-formatted durability value.
    pub durability: String,
    /// Existing debug-formatted lifecycle value.
    pub state: String,
    /// Whether exact recovery is available.
    pub resumable: bool,
    /// Existing used/maximum turn value, without the field label.
    pub turns: String,
    /// Coordinator-reported token usage.
    pub tokens_used: u64,
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
                    child.durability,
                    child.state,
                    child.resumable,
                    child.turns,
                    child.tokens_used,
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        AgentReport::Inspector(child) => format!(
            "session {} · {} · {} · {} · {} tokens · {}\nresumable {}{}\ncontinue: type a follow-up below · exact recovery: /agent resume {}\nresult: {}",
            child.session,
            child.summary.durability,
            child.summary.state,
            child.summary.turns,
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
