//! The `/skills` surface: what is indexed, what won a name, and what is withheld.
//!
//! A skill that quietly does nothing is the failure this surface exists to
//! prevent. Progressive trusted disclosure only protects a user who can find
//! out that something was withheld — a project skill sitting untrusted is
//! otherwise indistinguishable from one that was never written. So every
//! indexed entry appears here whatever state it is in: activatable, shadowed by
//! a higher layer, or held back with the reason attached; and every file
//! discovery refused appears beside them rather than in a log nobody opens.
//!
//! Nothing here activates a body. Listing reads the bounded index, which is
//! built from names, descriptions, and digests — never from instructions.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use smith_config::trust::{Executable, ExecutableKind, TrustDecision, TrustStatus, TrustStore};
use smith_runtime::skills::{SkillIndexEntry, SkillProblem, SmithSkillLayer, SmithSkillSources};

/// Everything `/skills` needs, owned for the life of one composed session.
///
/// Unlike the MCP context this is rebuilt at each composition rather than
/// carried across them. There is nothing live to preserve: a skill is a file
/// and a decision, and both are re-read from disk when the catalog is resolved
/// again.
pub(super) struct SkillContext {
    project: PathBuf,
    trust: Mutex<TrustStore>,
    /// Where each discovered workspace skill's body lives, by name.
    workspace: Vec<(String, PathBuf)>,
    /// Files discovery found and refused.
    problems: Vec<SkillProblem>,
}

impl std::fmt::Debug for SkillContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SkillContext")
            .field("project", &self.project)
            .field("workspace", &self.workspace.len())
            .field("problems", &self.problems.len())
            .finish()
    }
}

impl SkillContext {
    /// Discovers user and workspace skills and folds them onto `base`.
    ///
    /// Returns the context even when nothing was found: `/skills` still has the
    /// built-in reference set to show, and "there are no project skills" is an
    /// answer a user is entitled to get from the command rather than by
    /// listing directories themselves.
    pub(super) fn compose(
        base: SmithSkillSources,
        user_dir: &Path,
        project: &Path,
    ) -> anyhow::Result<(SmithSkillSources, Self)> {
        let trust = TrustStore::open(user_dir).map_err(|error| anyhow::anyhow!("{error}"))?;
        let workspace = smith_runtime::skills::discover(&project.join(".smith"))
            .skills
            .into_iter()
            .map(|found| (found.skill.name.clone(), found.path))
            .collect();
        let (sources, problems) =
            smith_runtime::skills::discover_into(base, user_dir, project, &trust);
        Ok((
            sources,
            Self {
                project: project.to_path_buf(),
                trust: Mutex::new(trust),
                workspace,
                problems,
            },
        ))
    }

    /// Every indexed skill grouped by source layer, plus every refused file.
    ///
    /// Shadowed entries stay visible: "which layer won this name" is the
    /// question a layered catalog exists to raise, and hiding the losers turns
    /// a deliberate override into a skill that mysteriously changed behavior.
    pub(super) fn render_list(&self, index: &[SkillIndexEntry]) -> String {
        let mut lines = Vec::new();
        for layer in [
            SmithSkillLayer::BuiltIn,
            SmithSkillLayer::User,
            SmithSkillLayer::Workspace,
            SmithSkillLayer::Session,
        ] {
            let entries = index
                .iter()
                .filter(|entry| entry.layer == layer)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                continue;
            }
            lines.push(layer.as_str().to_owned());
            for entry in entries {
                lines.push(format!(
                    "  {} · {} · {}",
                    entry.name(),
                    self.state(index, entry),
                    entry.description()
                ));
            }
        }
        if lines.is_empty() {
            lines.push("no skills are indexed".to_owned());
        }
        if !self.problems.is_empty() {
            lines.push("not loaded".to_owned());
            for problem in &self.problems {
                lines.push(format!(
                    "  {} · {} · {}",
                    problem.name,
                    problem.reason,
                    problem.path.display()
                ));
            }
        }
        lines.join("\n")
    }

    /// One entry's state, in the words that say what to do about it.
    fn state(&self, index: &[SkillIndexEntry], entry: &SkillIndexEntry) -> String {
        if !entry.activatable {
            return match entry.trust {
                _ if self.status(entry.name()) == Some(TrustStatus::Changed) => {
                    format!(
                        "withheld · its content changed — run `/skills trust {}`",
                        entry.name()
                    )
                }
                _ if self.status(entry.name()) == Some(TrustStatus::Denied) => format!(
                    "withheld · you declined it — run `/skills trust {}` to reconsider",
                    entry.name()
                ),
                _ => format!(
                    "withheld · needs approval — run `/skills trust {}`",
                    entry.name()
                ),
            };
        }
        // The resolver admits by layer order, so the winner for a name is its
        // highest activatable entry. Anything below that is present, correct,
        // and not what the agent would get.
        let winner = index
            .iter()
            .filter(|other| other.name() == entry.name() && other.activatable)
            .map(|other| other.layer)
            .max();
        match winner {
            Some(layer) if layer != entry.layer => {
                format!("shadowed by the {} skill of the same name", layer.as_str())
            }
            _ => "active".to_owned(),
        }
    }

    /// Renders what the user is being asked to admit, recording nothing.
    ///
    /// The path and the digest are the decision. A description would be the
    /// project telling the user what to think about the project's own file,
    /// which is the one sentence on screen that cannot be evidence.
    pub(super) fn confirmation(&self, name: &str) -> Result<String, String> {
        let (executable, status) = self.decide(name)?;
        Ok(format!(
            "skill `{name}`\n  {}\n  content {}\n  status {}\n\nActivating it writes this file's \
             instructions into the model's context. It grants no tool, permission, approval, or \
             credential, and the decision covers exactly this content.",
            executable.label(),
            executable.digest().as_hex(),
            match status {
                TrustStatus::Trusted => "already trusted; confirming again renews the decision",
                TrustStatus::Untrusted => "nothing has been decided about this content",
                TrustStatus::Changed => "approved earlier at different content",
                TrustStatus::Denied => "declined earlier",
            }
        ))
    }

    /// Records the user's decision so the next composition admits the skill.
    pub(super) fn trust(&self, name: &str) -> Result<String, String> {
        let (executable, _) = self.decide(name)?;
        self.trust
            .lock()
            .expect("trust store")
            .record(&self.project, &executable, TrustDecision::Allow)
            .map_err(|error| error.message)?;
        Ok(format!(
            "`{name}` is trusted at {}; it joins the catalog at the next idle boundary",
            executable.digest()
        ))
    }

    /// This project's decision about the named workspace skill, if it has one.
    fn status(&self, name: &str) -> Option<TrustStatus> {
        self.decide(name).ok().map(|(_, status)| status)
    }

    fn decide(&self, name: &str) -> Result<(Executable, TrustStatus), String> {
        let path = self
            .workspace
            .iter()
            .find(|(declared, _)| declared == name)
            .map(|(_, path)| path)
            .ok_or_else(|| {
                let known = self
                    .workspace
                    .iter()
                    .map(|(declared, _)| declared.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if known.is_empty() {
                    "this project declares no skills, so there is nothing to trust".to_owned()
                } else {
                    format!("`{name}` is not a project skill; this project declares: {known}")
                }
            })?;
        let executable = Executable::from_file(&self.project, ExecutableKind::Skill, path)
            .map_err(|error| error.message)?;
        let status = self
            .trust
            .lock()
            .expect("trust store")
            .status(&self.project, &executable)
            .map_err(|error| error.message)?;
        Ok((executable, status))
    }
}
