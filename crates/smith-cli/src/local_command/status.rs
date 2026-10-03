//! Captures the host and client values for the local `/status` report.

use std::path::Path;
use std::time::Duration;

use smith_client::status_report::{StatusGoal, StatusGoalReport, StatusReport};
use smith_host::GitChanges;
use smith_runtime::host::HostSession;
use smith_tui::App;

use super::{
    cache_controller_summary_value, reasoning_status_values, render_elapsed, render_status_cost,
    resume_summary_value,
};

pub(super) fn report(app: &App, host: &HostSession, project: &Path) -> StatusReport {
    let policy = host.runtime().policy();
    let (reasoning, reasoning_controls) = reasoning_status_values(policy);
    let session_usage = app.session_usage();
    let goal = match host.goal() {
        Err(error) => StatusGoal::Unavailable(error.to_string()),
        Ok(None) => StatusGoal::None,
        Ok(Some(goal)) => {
            let usage = goal
                .usage
                .charged_tokens
                .map_or_else(|| "unknown".to_owned(), |tokens| tokens.to_string());
            let reason = goal.stopped_reason.as_ref().map_or_else(
                || "none".to_owned(),
                |reason| {
                    reason.detail.as_ref().map_or_else(
                        || reason.code.clone(),
                        |detail| format!("{} · {detail}", reason.code),
                    )
                },
            );
            StatusGoal::Active(StatusGoalReport {
                objective: goal.objective,
                status: goal.status.as_str().to_owned(),
                tokens: format!("{usage} · {}", goal.usage.provenance.as_str()),
                budget: goal
                    .token_budget
                    .map_or_else(|| "none".to_owned(), |tokens| tokens.to_string()),
                active_elapsed: render_elapsed(Duration::from_millis(goal.usage.active_elapsed_ms)),
                reason,
                id: format!("{} · generation {}", goal.id, goal.generation),
            })
        }
    };

    StatusReport {
        session: host.session().id().to_string(),
        profile: policy.agent_profile.clone(),
        provider: policy.provider_name.clone(),
        model: policy.model.to_string(),
        permission: format!("{:?}", policy.approval_mode),
        reasoning,
        reasoning_controls,
        prompt_cache: app
            .status
            .cache_summary()
            .and_then(|summary| summary.render_usage_value())
            .unwrap_or_else(|| "usage not reported".to_owned()),
        cache_maintenance: host
            .cache_lifecycle()
            .map(|controller| cache_controller_summary_value(&controller))
            .unwrap_or_else(|| "off".to_owned()),
        resume_checkpoint: host
            .resume_capsule()
            .map(|capsule| resume_summary_value(&capsule))
            .unwrap_or_else(|| "not available".to_owned()),
        project: project.display().to_string(),
        git: GitChanges::discover(project)
            .and_then(|git| git.status_summary())
            .unwrap_or_else(|_| "unavailable (not a Git worktree)".to_owned()),
        goal,
        children: host
            .runtime()
            .delegation()
            .and_then(|delegation| delegation.coordinator())
            .map_or(0, |coordinator| coordinator.list().len()),
        usage: session_usage
            .render()
            .unwrap_or_else(|| "nothing spent yet".to_owned()),
        cost: render_status_cost(
            &session_usage,
            app.status.price(),
            (&policy.provider_name, policy.model.as_str()),
        ),
    }
}
