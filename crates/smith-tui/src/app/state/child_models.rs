use super::*;

/// Coordinator-owned turn and token counts for one visible child.
///
/// Kept as its own type so the client can say precisely what it does and
/// does not know: these numbers are never derived from the event stream,
/// only replaced wholesale from the delegation coordinator's own accounting
/// on the host's poll-on-redraw — see `usage-accounting`'s "Counts come
/// from the coordinator".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildCounts {
    /// Tasks (spawn plus follow-ups) the child has consumed.
    pub turns_used: u32,
    /// The task cap, or `u32::MAX` when the child is unbounded.
    pub max_turns: u32,
    /// Cumulative provider tokens attributed to the child.
    pub tokens_used: u64,
}

/// Child lifecycle data, including states observed only on a live stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildState {
    /// The child has an active turn.
    Running,
    /// The child is continuing its exact checkpoint.
    Resuming,
    /// The child is waiting for parent input.
    NeedsInput,
    /// The child's latest task completed.
    Completed,
    /// The child has no active turn.
    Idle,
    /// The child's turn was interrupted.
    Interrupted {
        /// Whether an exact checkpoint exists.
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
    /// Recovery is blocked.
    Blocked,
    /// The recovered child is terminal.
    Terminal,
}

impl From<ReportChildState> for ChildState {
    fn from(state: ReportChildState) -> Self {
        match state {
            ReportChildState::Running => Self::Running,
            ReportChildState::Idle => Self::Idle,
            ReportChildState::Interrupted { resumable } => Self::Interrupted { resumable },
            ReportChildState::Stopped { reason } => Self::Stopped { reason },
            ReportChildState::Failed => Self::Failed,
            ReportChildState::Expired => Self::Expired,
        }
    }
}

impl ChildState {
    /// Captures the shared readable stopping reason.
    pub fn stopped(reason: &agent_runtime_core::cancel::CancelReason) -> Self {
        ReportChildState::stopped(reason).into()
    }

    /// Existing lifecycle wording, shared with local reports where applicable.
    pub fn label(&self) -> Cow<'static, str> {
        match self {
            Self::Running => ReportChildState::Running.label(),
            Self::Resuming => "resuming".into(),
            Self::NeedsInput => "needs input".into(),
            Self::Completed => "completed".into(),
            Self::Idle => ReportChildState::Idle.label(),
            Self::Interrupted { resumable } => ReportChildState::Interrupted {
                resumable: *resumable,
            }
            .label(),
            Self::Stopped { reason } => ReportChildState::Stopped {
                reason: reason.clone(),
            }
            .label(),
            Self::Failed => ReportChildState::Failed.label(),
            Self::Expired => ReportChildState::Expired.label(),
            Self::Blocked => "blocked".into(),
            Self::Terminal => "terminal".into(),
        }
    }

    /// Whether the child is running or resuming, rather than waiting on input.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running | Self::Resuming)
    }

    /// Whether the child has in-flight work or a pending input request.
    pub fn is_live(&self) -> bool {
        self.is_running() || matches!(self, Self::NeedsInput)
    }

    /// Whether the child finished cleanly with nothing left to decide.
    pub fn retires_when_read(&self) -> bool {
        matches!(self, Self::Completed | Self::Idle)
    }

    /// Whether the child can accept a new follow-up turn.
    pub fn accepts_follow_up(&self) -> bool {
        self.retires_when_read() || matches!(self, Self::NeedsInput)
    }

    /// Whether an interrupted child has an exact checkpoint to continue.
    pub fn is_resumable(&self) -> bool {
        matches!(self, Self::Interrupted { resumable: true })
    }

    /// The shared tone for the panel row and inspector heading.
    pub fn tone(&self) -> Tone {
        match self {
            Self::Failed => Tone::Danger,
            Self::NeedsInput => Tone::Warning,
            Self::Completed | Self::Idle => Tone::Success,
            Self::Running | Self::Resuming => Tone::Default,
            _ => Tone::Dim,
        }
    }
}

/// The latest user-visible state of one child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSummary {
    /// Current lifecycle state.
    pub state: ChildState,
    /// Latest bounded result or detail.
    pub detail: Option<String>,
    /// The child's agent profile, once the spawn correlation resolves it.
    ///
    /// `None` for a child recovered or resumed into this process without
    /// ever being freshly spawned here: the coordinator's own `ChildStatus`
    /// carries no profile field, so a recovered child's profile is honestly
    /// unknown rather than guessed at.
    pub profile: Option<String>,
}

impl ChildSummary {
    /// Whether this child's lifecycle describes in-flight work.
    ///
    /// Both the panel's row order and the inspector's keyboard order read
    /// this, so a live child can never sort one way and select another.
    pub fn is_live(&self) -> bool {
        self.state.is_live()
    }

    /// Whether this child's lifecycle describes work that finished
    /// cleanly, with nothing left for the user to decide.
    ///
    /// Only these read as success and only these retire on their own. A
    /// failure, a stop, or an interrupted checkpoint is a row the user has
    /// not dealt with yet, and it stays until they do.
    pub fn retires_when_read(&self) -> bool {
        self.state.retires_when_read()
    }
}
