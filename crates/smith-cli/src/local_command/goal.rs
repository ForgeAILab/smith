//! Captures the host values after showing or changing a local `/goal`.

use agent_runtime_core::goal::GoalCommand;
use smith_client::commands::GoalAction;
use smith_client::goal_report::{GoalReport, GoalSnapshot};
use smith_runtime::host::HostSession;

pub(super) async fn report(host: &HostSession, action: GoalAction) -> GoalReport {
    let result = match action {
        GoalAction::Show => match host.goal() {
            Ok(Some(goal)) => return GoalReport::Snapshot(GoalSnapshot::from(&goal)),
            Ok(None) if host.runtime().goal_component().is_some() => return GoalReport::Empty,
            Ok(None) => return GoalReport::Unavailable(GoalReport::UNAVAILABLE_MESSAGE.to_owned()),
            Err(error) => Err(error),
        },
        GoalAction::Create(objective) => {
            host.control_goal(GoalCommand::Create {
                objective,
                token_budget: None,
            })
            .await
        }
        GoalAction::Edit(objective) => match host.goal() {
            Ok(Some(goal)) => {
                host.control_goal(GoalCommand::Edit {
                    id: goal.id,
                    generation: goal.generation,
                    objective,
                })
                .await
            }
            Ok(None) => {
                return GoalReport::Unavailable(
                    "No goal to edit; use `/goal <objective>`.".to_owned(),
                );
            }
            Err(error) => Err(error),
        },
        GoalAction::Budget(token_budget) => match host.goal() {
            Ok(Some(goal)) => {
                host.control_goal(GoalCommand::SetBudget {
                    id: goal.id,
                    generation: goal.generation,
                    token_budget,
                })
                .await
            }
            Ok(None) => {
                return GoalReport::Unavailable(
                    "No goal budget to change; use `/goal <objective>` first.".to_owned(),
                );
            }
            Err(error) => Err(error),
        },
        GoalAction::Pause => match host.goal() {
            Ok(Some(goal)) => {
                host.control_goal(GoalCommand::Pause {
                    id: goal.id,
                    generation: goal.generation,
                })
                .await
            }
            Ok(None) => return GoalReport::Unavailable("No active goal to pause.".to_owned()),
            Err(error) => Err(error),
        },
        GoalAction::Resume => match host.goal() {
            Ok(Some(goal)) => {
                host.control_goal(GoalCommand::Resume {
                    id: goal.id,
                    generation: goal.generation,
                })
                .await
            }
            Ok(None) => return GoalReport::Unavailable("No stopped goal to resume.".to_owned()),
            Err(error) => Err(error),
        },
        GoalAction::Clear => match host.goal() {
            Ok(Some(goal)) => {
                host.control_goal(GoalCommand::Clear {
                    id: goal.id,
                    generation: goal.generation,
                })
                .await
            }
            Ok(None) => return GoalReport::Unavailable("No goal to clear.".to_owned()),
            Err(error) => Err(error),
        },
    };
    match result {
        Ok(result) => match result.goal {
            Some(goal) => GoalReport::Snapshot(GoalSnapshot::from(&goal)),
            None => GoalReport::Cleared,
        },
        Err(error) => GoalReport::Unavailable(error.to_string()),
    }
}
