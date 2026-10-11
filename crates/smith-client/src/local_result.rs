//! Typed local command results, independent of their presentation.
//!
//! Terminal drawing belongs to `smith-tui`.

use crate::agent_report::{AgentReport, AgentResumeReport};
use crate::context_report::ContextReport;
use crate::diagnostics_report::DiagnosticsReport;
use crate::diff_report::{DiffOutcome, DiffReport};
use crate::goal_report::GoalReport;
use crate::help_report::HelpReport;
use crate::mcp_report::McpReport;
use crate::message_report::MessageReport;
use crate::recovery_report::RecoveryReport;
use crate::review_report::{ReviewReport, ReviewStartReport};
use crate::shell_report::{ShellOutput, ShellReport};
use crate::skills_report::SkillsReport;
use crate::status_report::StatusReport;
use crate::timeline_report::TimelineReport;

/// Semantic state of a local command result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalResultState {
    /// Informational output.
    Info,
    /// A successful command with no matching data.
    Empty,
    /// A local command that could not produce its result.
    Error,
}

/// A host or client command's local output, excluded from model history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalResult {
    /// The session's status snapshot.
    Status(Box<StatusReport>),
    /// The session's detailed cache, context, and recovery snapshot.
    Diagnostics(Box<DiagnosticsReport>),
    /// The session's context occupancy snapshot.
    Context(Box<ContextReport>),
    /// The command guide derived from the registry.
    Help(Box<HelpReport>),
    /// The session's root, child, and recovery timeline.
    Timeline(Box<TimelineReport>),
    /// The result of showing or changing the session's persistent goal.
    Goal(Box<GoalReport>),
    /// Child lists, inspection snapshots, and exact-resume outcomes.
    Agent(Box<AgentReport>),
    /// MCP server snapshots and local trust outcomes.
    Mcp(Box<McpReport>),
    /// Native module selection and mount outcomes.
    Modules(Box<crate::modules_report::ModulesReport>),
    /// Indexed skills, discovery problems, and local trust outcomes.
    Skills(Box<SkillsReport>),
    /// Classified Git patches or the last Smith turn's recovery preview.
    Diff(Box<DiffReport>),
    /// Read-only review scopes, inspection results, and dispatch outcomes.
    Review(Box<ReviewReport>),
    /// Undo, redo, and selective-revert previews and local outcomes.
    Recovery(Box<RecoveryReport>),
    /// Output from a local shell shortcut.
    Shell(Box<ShellReport>),
    /// A free-text notice, empty result, or local command failure.
    Message(Box<MessageReport>),
}

impl LocalResult {
    /// Command or result title, without inferring the report's type.
    pub fn title(&self) -> &str {
        match self {
            Self::Status(_) => "status",
            Self::Diagnostics(_) => "diagnostics",
            Self::Context(_) => "context",
            Self::Help(_) => "help",
            Self::Timeline(_) => "timeline",
            Self::Goal(_) => "goal",
            Self::Agent(report) => report.title(),
            Self::Mcp(_) => "mcp",
            Self::Modules(_) => "modules",
            Self::Skills(_) => "skills",
            Self::Diff(report) => &report.title,
            Self::Review(report) => report.title(),
            Self::Recovery(report) => report.title(),
            Self::Shell(_) => "shell",
            Self::Message(report) => report.title(),
        }
    }

    /// The result's semantic state.
    pub fn state(&self) -> LocalResultState {
        match self {
            Self::Status(_) | Self::Diagnostics(_) | Self::Context(_) | Self::Help(_) => {
                LocalResultState::Info
            }
            Self::Timeline(report) => match report.as_ref() {
                TimelineReport::Empty => LocalResultState::Empty,
                TimelineReport::Unavailable(_) => LocalResultState::Error,
                TimelineReport::Entries(_) => LocalResultState::Info,
            },
            Self::Goal(report) => match report.as_ref() {
                GoalReport::Empty => LocalResultState::Empty,
                GoalReport::Unavailable(_) => LocalResultState::Error,
                GoalReport::Cleared | GoalReport::Snapshot(_) => LocalResultState::Info,
            },
            Self::Agent(report) => match report.as_ref() {
                AgentReport::Empty => LocalResultState::Empty,
                AgentReport::Unavailable | AgentReport::Missing(_) => LocalResultState::Error,
                AgentReport::Resume(resume) => match resume {
                    AgentResumeReport::RequiresIdle | AgentResumeReport::Started { .. } => {
                        LocalResultState::Info
                    }
                    AgentResumeReport::Missing { .. }
                    | AgentResumeReport::Incompatible { .. }
                    | AgentResumeReport::Unavailable
                    | AgentResumeReport::Failed { .. } => LocalResultState::Error,
                },
                AgentReport::Parent | AgentReport::List(_) | AgentReport::Inspector(_) => {
                    LocalResultState::Info
                }
            },
            Self::Mcp(report) => match report.as_ref() {
                McpReport::Unavailable | McpReport::Error(_) => LocalResultState::Error,
                McpReport::Empty { .. } | McpReport::Servers(_) | McpReport::Trusted { .. } => {
                    LocalResultState::Info
                }
            },
            Self::Modules(_) => LocalResultState::Info,
            Self::Skills(report) => match report.as_ref() {
                SkillsReport::Error(_) => LocalResultState::Error,
                SkillsReport::Empty
                | SkillsReport::Indexed { .. }
                | SkillsReport::Trusted { .. } => LocalResultState::Info,
            },
            Self::Diff(report) => match &report.outcome {
                DiffOutcome::Empty => LocalResultState::Empty,
                DiffOutcome::Error(_) => LocalResultState::Error,
                DiffOutcome::Patch(_) => LocalResultState::Info,
            },
            Self::Review(report) => match report.as_ref() {
                ReviewReport::Empty => LocalResultState::Empty,
                ReviewReport::Error(_)
                | ReviewReport::Start(
                    ReviewStartReport::Unavailable
                    | ReviewStartReport::AtCapacity { .. }
                    | ReviewStartReport::Failed(_),
                ) => LocalResultState::Error,
                ReviewReport::Confirmation(_)
                | ReviewReport::Start(
                    ReviewStartReport::Started { .. } | ReviewStartReport::Queued { .. },
                ) => LocalResultState::Info,
            },
            Self::Recovery(report) => match report.as_ref() {
                RecoveryReport::PreviewError { .. }
                | RecoveryReport::RevertUsage
                | RecoveryReport::ApplyError { .. } => LocalResultState::Error,
                RecoveryReport::UndoConfirmation(_)
                | RecoveryReport::RedoConfirmation(_)
                | RecoveryReport::RevertConfirmation(_)
                | RecoveryReport::Applied(_)
                | RecoveryReport::Cancelled(_) => LocalResultState::Info,
            },
            Self::Shell(report) => match &report.output {
                ShellOutput::Empty => LocalResultState::Empty,
                ShellOutput::Output(_) if report.is_error => LocalResultState::Error,
                ShellOutput::Output(_) => LocalResultState::Info,
            },
            Self::Message(report) => match report.as_ref() {
                MessageReport::Notice { .. } => LocalResultState::Info,
                MessageReport::Empty { .. } => LocalResultState::Empty,
                MessageReport::Error { .. } => LocalResultState::Error,
            },
        }
    }
}
