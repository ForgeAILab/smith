//! Captures indexed skill metadata and local skill trust outcomes.

use smith_client::commands::SkillsAction;
use smith_client::local_result::LocalResult;
use smith_client::skills_report::{
    SkillEntry, SkillGroup, SkillLoadProblem, SkillState, SkillsReport,
};
use smith_config::trust::TrustStatus;
use smith_runtime::skills::{SkillIndexEntry, SmithSkillLayer};

use crate::skills::SkillContext;

use super::CommandReport;

pub(super) fn command(
    context: &SkillContext,
    index: &[SkillIndexEntry],
    action: SkillsAction,
) -> CommandReport {
    match action {
        SkillsAction::List => {
            CommandReport::Show(LocalResult::Skills(Box::new(report(context, index))))
        }
        // The path and the digest are what the decision binds, so the path
        // and the digest are what the confirmation shows.
        SkillsAction::Trust(skill) => match context.confirmation(&skill) {
            Ok(content) => CommandReport::SkillTrust { skill, content },
            Err(error) => {
                CommandReport::Show(LocalResult::Skills(Box::new(SkillsReport::Error(error))))
            }
        },
    }
}

pub(crate) fn report(context: &SkillContext, index: &[SkillIndexEntry]) -> SkillsReport {
    let groups = [
        SmithSkillLayer::BuiltIn,
        SmithSkillLayer::User,
        SmithSkillLayer::Workspace,
        SmithSkillLayer::Session,
    ]
    .into_iter()
    .filter_map(|layer| {
        let entries = index
            .iter()
            .filter(|entry| entry.layer == layer)
            .map(|entry| SkillEntry {
                name: entry.name().to_owned(),
                description: entry.description().to_owned(),
                state: state(context, index, entry),
            })
            .collect::<Vec<_>>();
        (!entries.is_empty()).then_some(SkillGroup { layer, entries })
    })
    .collect::<Vec<_>>();
    let problems = context
        .problems()
        .iter()
        .map(|problem| SkillLoadProblem {
            name: problem.name.clone(),
            reason: problem.reason.clone(),
            path: problem.path.display().to_string(),
        })
        .collect::<Vec<_>>();
    if groups.is_empty() && problems.is_empty() {
        SkillsReport::Empty
    } else {
        SkillsReport::Indexed { groups, problems }
    }
}

fn state(context: &SkillContext, index: &[SkillIndexEntry], entry: &SkillIndexEntry) -> SkillState {
    if !entry.activatable {
        return match context.status(entry.name()) {
            Some(TrustStatus::Changed) => SkillState::Changed,
            Some(TrustStatus::Denied) => SkillState::Denied,
            _ => SkillState::Untrusted,
        };
    }
    // The resolver admits by layer order, so the winner for a name is its
    // highest activatable entry. Lower entries remain visible in the report.
    let winner = index
        .iter()
        .filter(|other| other.name() == entry.name() && other.activatable)
        .map(|other| other.layer)
        .max();
    match winner {
        Some(layer) if layer != entry.layer => SkillState::Shadowed { layer },
        _ => SkillState::Active,
    }
}

pub(crate) fn trust(context: &SkillContext, skill: &str) -> SkillsReport {
    match context.trust(skill) {
        Ok(digest) => SkillsReport::Trusted {
            skill: skill.to_owned(),
            digest: digest.to_string(),
        },
        Err(error) => SkillsReport::Error(error),
    }
}
