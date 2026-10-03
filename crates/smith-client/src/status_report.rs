//! The local `/status` snapshot and its plain-text rendering.
//!
//! Values carry no field labels. The terminal client draws these fields
//! directly; the existing transcript fixture recorder uses [`render_plain`].

/// Session information captured when `/status` is invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    /// Root session identity.
    pub session: String,
    /// Active profile name.
    pub profile: String,
    /// Active provider name.
    pub provider: String,
    /// Active model name.
    pub model: String,
    /// Approval mode's display value.
    pub permission: String,
    /// Effective reasoning state, effort, and selection source.
    pub reasoning: String,
    /// Supported reasoning controls and their source.
    pub reasoning_controls: String,
    /// Last root turn's cache usage, or its availability.
    pub prompt_cache: String,
    /// Cache maintenance summary.
    pub cache_maintenance: String,
    /// Resume checkpoint summary.
    pub resume_checkpoint: String,
    /// Project path as displayed by the host.
    pub project: String,
    /// Git worktree summary.
    pub git: String,
    /// Persistent goal snapshot, including availability.
    pub goal: StatusGoal,
    /// Number of delegated children.
    pub children: usize,
    /// Session usage summary.
    pub usage: String,
    /// Session cost summary, preserving unknown versus unspent.
    pub cost: String,
}

impl StatusReport {
    /// The existing hint at the end of the status card.
    pub const DIAGNOSTICS_HINT: &str = "/diagnostics shows detailed cache and recovery information";
}

/// The goal details embedded in `/status`; `/goal` remains a text command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusGoal {
    /// No persistent goal is present.
    None,
    /// The host could not read the goal.
    Unavailable(String),
    /// The goal's current display values.
    Active(StatusGoalReport),
}

/// Display values for the goal embedded in the status snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusGoalReport {
    /// User-supplied objective.
    pub objective: String,
    /// Current goal state.
    pub status: String,
    /// Charged tokens and their provenance.
    pub tokens: String,
    /// Token budget, or its absence.
    pub budget: String,
    /// Active elapsed time.
    pub active_elapsed: String,
    /// Stopping reason, or its absence.
    pub reason: String,
    /// Goal identity and generation.
    pub id: String,
}

/// Renders the transcript body for the existing plain-text capture surface.
/// No terminal libraries or title/label parsing are involved.
pub fn render_plain(report: &StatusReport) -> String {
    let goal = match &report.goal {
        StatusGoal::None => "none".to_owned(),
        StatusGoal::Unavailable(error) => format!("unavailable ({error})"),
        StatusGoal::Active(goal) => format!(
            "{}\nstatus: {}\ntokens: {}\nbudget: {}\nactive elapsed: {}\nreason: {}\nid: {}",
            goal.objective,
            goal.status,
            goal.tokens,
            goal.budget,
            goal.active_elapsed,
            goal.reason,
            goal.id,
        ),
    };
    format!(
        "session: {}\nprofile: {}\nprovider: {} · model: {}\npermission: {}\n\
         reasoning: {}\nreasoning controls: {}\nprompt cache: {}\ncache maintenance: {}\n\
         resume checkpoint: {}\nproject: {}\nGit: {}\ngoal: {goal}\nchildren: {}\n\
         usage: {}\ncost: {}\n{}",
        report.session,
        report.profile,
        report.provider,
        report.model,
        report.permission,
        report.reasoning,
        report.reasoning_controls,
        report.prompt_cache,
        report.cache_maintenance,
        report.resume_checkpoint,
        report.project,
        report.git,
        report.children,
        report.usage,
        report.cost,
        StatusReport::DIAGNOSTICS_HINT,
    )
}
