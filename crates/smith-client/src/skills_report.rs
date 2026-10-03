//! Local `/skills` results and their plain-text rendering.
//!
//! Source layers, admission states, and discovery problems travel as data.
//! Trust confirmation remains a separate approval surface.

use smith_runtime::skills::SmithSkillLayer;

/// The result of listing indexed skills or recording a trust decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsReport {
    /// No skills are indexed and discovery reported no refused files.
    Empty,
    /// The host could not prepare or record a trust decision.
    /// This includes unavailable skill files and trust-store failures.
    Error(String),
    /// Indexed skills by source layer, followed by refused files.
    Indexed {
        /// Nonempty layers in the resolver's existing precedence order.
        groups: Vec<SkillGroup>,
        /// Discovery problems in their existing order.
        problems: Vec<SkillLoadProblem>,
    },
    /// The exact skill content was trusted for the next composition.
    Trusted {
        /// Workspace skill name.
        skill: String,
        /// Content identity covered by the recorded decision.
        digest: String,
    },
}

impl SkillsReport {
    /// The existing message when no skills are indexed.
    pub const EMPTY_MESSAGE: &str = "no skills are indexed";

    /// The existing acknowledgement of a recorded trust decision.
    pub fn trusted_value(skill: &str, digest: &str) -> String {
        format!("`{skill}` is trusted at {digest}; it joins the catalog at the next idle boundary")
    }
}

/// One source layer's indexed skills, including shadowed or withheld entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillGroup {
    /// Source layer, independent of its heading text.
    pub layer: SmithSkillLayer,
    /// Entries in the index's existing order.
    pub entries: Vec<SkillEntry>,
}

/// Bounded skill metadata and the host's admission decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry {
    /// Indexed name.
    pub name: String,
    /// Routing description; never the instruction body.
    pub description: String,
    /// Activation, shadowing, or trust state.
    pub state: SkillState,
}

/// Activation and trust outcomes, independent of display labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillState {
    /// This entry wins its name in the activation catalog.
    Active,
    /// A higher activatable layer wins this name.
    Shadowed {
        /// The winning source layer.
        layer: SmithSkillLayer,
    },
    /// The skill content changed since the last trust decision.
    Changed,
    /// The user refused the skill content.
    Denied,
    /// The skill needs approval.
    Untrusted,
}

impl SkillState {
    /// The existing state value, including any trust guidance.
    pub fn render_value(&self, skill: &str) -> String {
        match self {
            Self::Active => "active".to_owned(),
            Self::Shadowed { layer } => {
                format!("shadowed by the {} skill of the same name", layer.as_str())
            }
            Self::Changed => {
                format!("withheld · its content changed — run `/skills trust {skill}`")
            }
            Self::Denied => {
                format!("withheld · you declined it — run `/skills trust {skill}` to reconsider")
            }
            Self::Untrusted => {
                format!("withheld · needs approval — run `/skills trust {skill}`")
            }
        }
    }
}

/// A file discovery refused, with its existing bounded explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillLoadProblem {
    /// Skill directory name.
    pub name: String,
    /// Discovery-owned refusal explanation.
    pub reason: String,
    /// Rejected path's existing display value.
    pub path: String,
}

/// Renders the transcript body for the existing plain-text capture surface.
/// No terminal libraries or title/label parsing are involved.
pub fn render_plain(report: &SkillsReport) -> String {
    let (groups, problems) = match report {
        SkillsReport::Empty => return SkillsReport::EMPTY_MESSAGE.to_owned(),
        SkillsReport::Error(error) => return error.clone(),
        SkillsReport::Trusted { skill, digest } => {
            return SkillsReport::trusted_value(skill, digest);
        }
        SkillsReport::Indexed { groups, problems } => (groups, problems),
    };
    let mut lines = Vec::new();
    for group in groups {
        lines.push(group.layer.as_str().to_owned());
        for entry in &group.entries {
            lines.push(format!(
                "  {} · {} · {}",
                entry.name,
                entry.state.render_value(&entry.name),
                entry.description,
            ));
        }
    }
    if lines.is_empty() {
        lines.push(SkillsReport::EMPTY_MESSAGE.to_owned());
    }
    if !problems.is_empty() {
        lines.push("not loaded".to_owned());
        for problem in problems {
            lines.push(format!(
                "  {} · {} · {}",
                problem.name, problem.reason, problem.path,
            ));
        }
    }
    lines.join("\n")
}
