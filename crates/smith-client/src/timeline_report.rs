//! The local `/timeline` snapshot and its plain-text rendering.
//!
//! Entry kinds and result availability travel as data. Terminal drawing
//! belongs to `smith-tui`, without recovering structure from display text.

/// Timeline information captured when `/timeline` is invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineReport {
    /// No root turns, children, or recovery actions have been recorded.
    Empty,
    /// The host could not read the session's timeline.
    Unavailable(String),
    /// Ordered entries, retaining the host's existing hundred-entry limit.
    Entries(Vec<TimelineEntry>),
}

impl TimelineReport {
    /// The existing message for a session without timeline entries.
    pub const EMPTY_MESSAGE: &str = "No turns, children, or recovery actions yet.";
}

/// One timeline entry, with its identity and display values kept separate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineEntry {
    /// A root turn completed in the runtime event history.
    RootTurn {
        /// Stable root turn identity.
        turn: String,
        /// Existing finish value, including any reason or request.
        finish: String,
        /// Last plan counts recorded for this turn, if available.
        plan: Option<TimelinePlan>,
        /// Successful shell gates for this turn.
        passed_gates: u32,
        /// Failed shell gates for this turn.
        failed_gates: u32,
    },
    /// A committed manifest used when runtime entries are unavailable.
    RootManifest {
        /// Stable root turn identity.
        turn: String,
        /// Manifest provider name.
        provider: String,
        /// Manifest model name.
        model: String,
        /// Number of activated capabilities.
        activated_capabilities: usize,
    },
    /// A child lifecycle event from the runtime history.
    ChildEvent {
        /// Stable child identity.
        child: String,
        /// Lifecycle event and its existing display values.
        event: TimelineChildEvent,
    },
    /// A coordinator snapshot for a child absent from the runtime history.
    ChildSnapshot {
        /// Stable child identity.
        child: String,
        /// Child session identity.
        session: String,
        /// Existing debug-formatted durability value.
        durability: String,
        /// Existing debug-formatted state value.
        state: String,
        /// Whether exact recovery is available.
        resumable: bool,
        /// Existing used/maximum turn value.
        turns: String,
    },
    /// A recovery action supplied by the change registry.
    Recovery {
        /// One-based position in the registry's recovery history.
        number: usize,
        /// Registry-owned action description; renderers do not parse it.
        detail: String,
    },
}

/// Terminal plan counts associated with a completed root turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelinePlan {
    /// In-progress items.
    pub active: u32,
    /// Pending items.
    pub pending: u32,
    /// Completed items.
    pub done: u32,
    /// Cancelled items.
    pub cancelled: u32,
}

impl TimelinePlan {
    /// The existing plan summary, without inspecting status labels.
    pub fn render_value(&self) -> String {
        format!(
            "plan {} active/{} pending/{} done/{} cancelled",
            self.active, self.pending, self.done, self.cancelled,
        )
    }
}

/// A recorded child lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineChildEvent {
    /// The child was started with its workspace and optional turn limit.
    Started {
        /// Existing debug-formatted workspace policy.
        workspace: String,
        /// Finite limit; absent for an unlimited child.
        turn_limit: Option<u32>,
    },
    /// The child requested input.
    NeedsInput,
    /// The child completed its task.
    Completed,
    /// The child was stopped.
    Stopped {
        /// Existing debug-formatted stopping reason.
        reason: String,
    },
    /// The child failed.
    Failed,
}

impl TimelineChildEvent {
    /// The existing event description, without the child's identity.
    pub fn render_value(&self) -> String {
        match self {
            Self::Started {
                workspace,
                turn_limit,
            } => match turn_limit {
                Some(limit) => format!("started · {workspace} · {limit} turn limit"),
                None => format!("started · {workspace}"),
            },
            Self::NeedsInput => "needs input".to_owned(),
            Self::Completed => "task completed".to_owned(),
            Self::Stopped { reason } => format!("stopped ({reason})"),
            Self::Failed => "failed".to_owned(),
        }
    }
}

/// Renders the transcript body for the existing plain-text capture surface.
/// No terminal libraries or title/label parsing are involved.
pub fn render_plain(report: &TimelineReport) -> String {
    let entries = match report {
        TimelineReport::Empty => return TimelineReport::EMPTY_MESSAGE.to_owned(),
        TimelineReport::Unavailable(error) => return format!("timeline unavailable: {error}"),
        TimelineReport::Entries(entries) => entries,
    };
    entries
        .iter()
        .map(|entry| match entry {
            TimelineEntry::RootTurn {
                turn,
                finish,
                plan,
                passed_gates,
                failed_gates,
            } => format!(
                "root {turn} · {finish} · {} · gates {passed_gates} passed/{failed_gates} failed",
                plan.as_ref()
                    .map_or_else(|| "plan none".to_owned(), TimelinePlan::render_value),
            ),
            TimelineEntry::RootManifest {
                turn,
                provider,
                model,
                activated_capabilities,
            } => format!(
                "root {turn} · committed · {provider}/{model} · {activated_capabilities} activated capability/capabilities",
            ),
            TimelineEntry::ChildEvent { child, event } => {
                format!("child {child} · {}", event.render_value())
            }
            TimelineEntry::ChildSnapshot {
                child,
                session,
                durability,
                state,
                resumable,
                turns,
            } => format!(
                "child {child} · session {session} · {durability} · {state} · {} · {turns} turns",
                crate::agent_report::exact_resume_label(*resumable),
            ),
            TimelineEntry::Recovery { number, detail } => {
                format!("recovery recovery-{number} · {detail}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
