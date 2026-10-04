//! Client-neutral notice labels and placement.

use std::borrow::Cow;

/// Whether a notice records an event or only answers a keypress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticePersistence {
    /// Retained in the conversation transcript.
    Transcript,
    /// Shown in the hint row until the next keypress.
    Feedback,
}

/// A notice kind fixes its existing label and persistence.
///
/// Feedback is reserved for user keypresses that change nothing. Runtime,
/// provider, child, monitor, and recovery reports remain transcript notices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeKind {
    /// Transcript notice labelled `monitor`.
    Monitor,
    /// Transcript notice labelled `marker`.
    Marker,
    /// Transcript notice labelled `account`.
    Account,
    /// Transcript notice labelled `sub-agent`.
    SubAgent,
    /// Transcript notice labelled `local`.
    Local,
    /// Transcript notice labelled `reasoning`.
    Reasoning,
    /// Transcript notice labelled `cache`.
    Cache,
    /// Transcript notice labelled `turn`.
    Turn,
    /// Transcript notice labelled `stale`.
    Stale,
    /// Transcript notice labelled `review`.
    Review,
    /// Transcript notice labelled `resume`.
    Resume,
    /// Transcript notice labelled `questionnaire`.
    Questionnaire,
    /// Transcript notice labelled `provider`.
    Provider,
    /// Transcript notice labelled `limit`.
    Limit,
    /// Transcript notice labelled `downgrade`.
    Downgrade,
    /// Transcript notice labelled `changes`.
    Changes,
    /// Transcript notice labelled `approval`.
    Approval,
    /// Transcript notice labelled `smith`.
    Smith,
    /// Transcript notice labelled `goal`.
    Goal,
    /// Transcript notice labelled `context`.
    Context,
    /// Transcript notice labelled `mcp`.
    Mcp,
    /// Transcript notice labelled `profile`.
    Profile,
    /// Transcript notice labelled `background`.
    Background,
    /// Transcript notice labelled `details`.
    Details,
    /// Transcript notice labelled `retry`.
    Retry,
    /// Transcript notice labelled `integrity`.
    Integrity,
    /// Transcript notice labelled `stream`.
    Stream,
    /// Transcript notice labelled `capabilities`.
    Capabilities,
    /// Transcript notice labelled `started`.
    Started,
    /// Transcript notice labelled `recovered`.
    Recovered,
    /// Transcript notice labelled `resuming`.
    Resuming,
    /// Transcript notice labelled `interrupted`.
    Interrupted,
    /// Transcript notice labelled `needs input`.
    NeedsInput,
    /// Transcript notice labelled `stopped`.
    Stopped,
    /// Transcript notice labelled `agents`.
    Agents,
    /// Transcript notice labelled `session restored`.
    SessionRestored,
    /// Transcript notice labelled `recovery`.
    Recovery,
    /// Transcript notice labelled `help`.
    Help,
    /// Transcript notice labelled `undo`.
    Undo,
    /// Transcript notice labelled `redo`.
    Redo,
    /// Transcript notice labelled `revert`.
    Revert,
    /// A named monitor, retaining the `monitor:<name>` label.
    NamedMonitor(String),
    /// Keypress feedback labelled `smith`.
    CommandRefused,
    /// Keypress feedback labelled `goal`.
    GoalRefused,
    /// Refused exact child resume, labelled `agent`.
    AgentResumeRefused,
    /// Keypress feedback labelled `resume`.
    SessionUnchanged,
    /// Keypress feedback labelled `account`.
    AccountUnchanged,
    /// Keypress feedback labelled `clipboard`.
    Clipboard,
    /// Keypress feedback labelled `background`.
    BackgroundUnavailable,
    /// Keypress feedback labelled `smith`.
    OverlayBlocked,
}

impl NoticeKind {
    /// The existing source wording, independent of the client rendering it.
    pub fn label(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            Self::Monitor => "monitor",
            Self::Marker => "marker",
            Self::Account | Self::AccountUnchanged => "account",
            Self::SubAgent => "sub-agent",
            Self::Local => "local",
            Self::Reasoning => "reasoning",
            Self::Cache => "cache",
            Self::Turn => "turn",
            Self::Stale => "stale",
            Self::Review => "review",
            Self::Resume | Self::SessionUnchanged => "resume",
            Self::Questionnaire => "questionnaire",
            Self::Provider => "provider",
            Self::Limit => "limit",
            Self::Downgrade => "downgrade",
            Self::Changes => "changes",
            Self::Approval => "approval",
            Self::Smith | Self::CommandRefused | Self::OverlayBlocked => "smith",
            Self::Goal | Self::GoalRefused => "goal",
            Self::AgentResumeRefused => "agent",
            Self::Context => "context",
            Self::Mcp => "mcp",
            Self::Profile => "profile",
            Self::Background | Self::BackgroundUnavailable => "background",
            Self::Details => "details",
            Self::Retry => "retry",
            Self::Integrity => "integrity",
            Self::Stream => "stream",
            Self::Capabilities => "capabilities",
            Self::Started => "started",
            Self::Recovered => "recovered",
            Self::Resuming => "resuming",
            Self::Interrupted => "interrupted",
            Self::NeedsInput => "needs input",
            Self::Stopped => "stopped",
            Self::Agents => "agents",
            Self::SessionRestored => "session restored",
            Self::Recovery => "recovery",
            Self::Help => "help",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Revert => "revert",
            Self::Clipboard => "clipboard",
            Self::NamedMonitor(name) => return Cow::Owned(format!("monitor:{name}")),
        })
    }

    /// Where this kind belongs; callers cannot override it.
    pub fn persistence(&self) -> NoticePersistence {
        match self {
            Self::CommandRefused
            | Self::GoalRefused
            | Self::AgentResumeRefused
            | Self::SessionUnchanged
            | Self::AccountUnchanged
            | Self::Clipboard
            | Self::BackgroundUnavailable
            | Self::OverlayBlocked => NoticePersistence::Feedback,
            _ => NoticePersistence::Transcript,
        }
    }
}

/// A labelled notice body, without terminal types or styles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Fixes both the label and placement.
    pub kind: NoticeKind,
    /// Existing notice wording.
    pub text: String,
}
