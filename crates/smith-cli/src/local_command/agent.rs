//! Captures the coordinator values for the local `/agent` report.

use smith_client::agent_report::{AgentReport, AgentSnapshot, AgentSummary};
use smith_client::commands::AgentAction;
use smith_runtime::host::HostSession;

pub(super) fn report(
    host: &HostSession,
    inspected_child: Option<&str>,
    action: AgentAction,
) -> AgentReport {
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
    else {
        return AgentReport::Unavailable;
    };
    let children = coordinator.list();
    let selected = match action {
        AgentAction::Parent => return AgentReport::Parent,
        AgentAction::Next | AgentAction::Previous if children.is_empty() => None,
        direction @ (AgentAction::Next | AgentAction::Previous) => {
            let current = inspected_child
                .and_then(|current| {
                    children
                        .iter()
                        .position(|status| status.child.as_str() == current)
                })
                .unwrap_or(0);
            let index = if direction == AgentAction::Next {
                (current + 1) % children.len()
            } else {
                current.checked_sub(1).unwrap_or(children.len() - 1)
            };
            Some(children[index].child.as_str().to_owned())
        }
        AgentAction::Inspect(selected) => Some(selected),
        AgentAction::List => None,
    };
    if let Some(selected) = selected {
        children
            .iter()
            .find(|status| status.child.as_str() == selected)
            .map_or_else(
                || AgentReport::Missing(selected),
                |status| AgentReport::Inspector(AgentSnapshot::from(status)),
            )
    } else if children.is_empty() {
        AgentReport::Empty
    } else {
        AgentReport::List(children.iter().map(AgentSummary::from).collect())
    }
}
