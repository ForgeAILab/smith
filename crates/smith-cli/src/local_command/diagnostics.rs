//! Captures the existing sections and rows for the local `/diagnostics` report.

use std::path::Path;
use std::time::Duration;

use smith_client::diagnostics_report::{DiagnosticsReport, DiagnosticsRow, DiagnosticsSection};
use smith_client::status::{Status, TokenCount};
use smith_host::GitChanges;
use smith_runtime::client::EstimationConfidence;
use smith_runtime::factory::RuntimePolicy;
use smith_runtime::host::HostSession;
use smith_tui::App;

use super::{
    cache_controller_status_value, cache_status_value, reasoning_status_values, render_elapsed,
    render_status_cost, resume_capsule_status_value,
};
use crate::resources::bounded_text;

pub(super) fn report(app: &App, host: &HostSession, project: &Path) -> DiagnosticsReport {
    let policy = host.runtime().policy();
    let (reasoning, reasoning_controls) = reasoning_status_values(policy);
    let attribution = host.changes().latest().map_or_else(
        || {
            if host.changes().has_historical_records() {
                "historical metadata only; not automatically undoable".to_owned()
            } else {
                "no attributable turn recorded".to_owned()
            }
        },
        |set| {
            if set.undone || !set.has_exact_mutations() {
                format!("Smith turn {} · automatic undo unavailable", set.turn)
            } else if set.is_fully_attributable() {
                format!("Smith turn {} · undo available", set.turn)
            } else {
                format!(
                    "Smith turn {} · undo covers Smith's own edits; \
                     ambiguous changes need /diff",
                    set.turn
                )
            }
        },
    );
    let connections = if app.resources.disconnections.is_empty() {
        "none".to_owned()
    } else {
        app.resources
            .disconnections
            .iter()
            .map(|entry| format!("{} ({})", entry.label, entry.detail))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let session_usage = app.session_usage();

    DiagnosticsReport {
        sections: vec![
            DiagnosticsSection {
                rows: vec![
                    field("session", host.session().id().to_string()),
                    field(
                        "profile",
                        format!(
                            "{} · posture {} · use {} · rev {} · source {}{}",
                            policy.agent_profile,
                            policy.agent_posture.as_str(),
                            policy
                                .agent_profile_uses
                                .iter()
                                .map(|placement| placement.as_str())
                                .collect::<Vec<_>>()
                                .join("+"),
                            bounded_text(&policy.agent_profile_revision, 12),
                            bounded_text(&policy.agent_profile_source, 80),
                            if policy.agent_profile_legacy {
                                " · legacy adapter; migrate to [profiles]"
                            } else {
                                ""
                            },
                        ),
                    ),
                    field("provider", &policy.provider_name),
                    field("model", policy.model.to_string()),
                    field("permission", format!("{:?}", policy.approval_mode)),
                    field("reasoning", reasoning),
                    field("reasoning controls", reasoning_controls),
                    field(
                        "protected mid-turn recovery",
                        policy.mid_turn_durability.as_str(),
                    ),
                ],
            },
            harness_section(&app.status),
            context_section(&app.status, policy),
            DiagnosticsSection {
                rows: vec![
                    field(
                        "cache maintenance",
                        host.cache_lifecycle().map_or_else(
                            || "unavailable".to_owned(),
                            |snapshot| cache_controller_status_value(&snapshot),
                        ),
                    ),
                    field(
                        "resume capsule",
                        host.resume_capsule().map_or_else(
                            || "disabled".to_owned(),
                            |capsule| resume_capsule_status_value(&capsule),
                        ),
                    ),
                ],
            },
            DiagnosticsSection {
                rows: vec![
                    field("project", project.display().to_string()),
                    field(
                        "Git",
                        GitChanges::discover(project)
                            .and_then(|git| git.status_summary())
                            .unwrap_or_else(|_| "unavailable (not a Git worktree)".to_owned()),
                    ),
                ],
            },
            goal_section(host),
            DiagnosticsSection {
                rows: vec![
                    field("connections", connections),
                    field(
                        "children",
                        host.runtime()
                            .delegation()
                            .and_then(|delegation| delegation.coordinator())
                            .map_or(0, |coordinator| coordinator.list().len())
                            .to_string(),
                    ),
                    field(
                        "usage",
                        session_usage
                            .render()
                            .unwrap_or_else(|| "nothing spent yet".to_owned()),
                    ),
                    field(
                        "cost",
                        render_status_cost(
                            &session_usage,
                            app.status.price(),
                            (&policy.provider_name, policy.model.as_str()),
                        ),
                    ),
                    field("change attribution", attribution),
                ],
            },
        ],
    }
}

pub(crate) fn harness_section(status: &Status) -> DiagnosticsSection {
    let capabilities = &status.capabilities;
    let mut rows = Vec::new();
    rows.push(field(
        "registry snapshot",
        match &capabilities.registry {
            Some((fingerprint, entries)) => format!("{fingerprint} · {entries} entries"),
            None => "waiting for live lifecycle".to_owned(),
        },
    ));
    if let Some((fingerprint, visible)) = &capabilities.view {
        rows.push(field(
            "scoped capability view",
            format!("{fingerprint} · {visible} visible"),
        ));
    }
    if let Some((revision, candidates)) = &capabilities.retrieval {
        let candidates = if candidates.is_empty() {
            "(none)".to_owned()
        } else {
            candidates.join(", ")
        };
        rows.push(field(
            "latest capability retrieval",
            format!("{revision} · {candidates}"),
        ));
    }
    if let Some((epoch, active)) = &capabilities.activation {
        let active = if active.is_empty() {
            "(none)".to_owned()
        } else {
            active.join(", ")
        };
        rows.push(field("activation epoch", format!("{epoch} · {active}")));
    }
    if let Some(plan) = &status.context_plan {
        rows.push(field(
            "context provenance",
            format!("{} · cache {}", plan.fingerprint, plan.cache_fingerprint),
        ));
    }
    if capabilities.compactions > 0 {
        rows.push(field(
            "context compaction",
            format!(
                "{} run(s) · {} tokens reclaimed",
                capabilities.compactions, capabilities.reclaimed_tokens
            ),
        ));
    }
    DiagnosticsSection { rows }
}

pub(crate) fn context_section(status: &Status, policy: &RuntimePolicy) -> DiagnosticsSection {
    let limits = policy.model_profile.limits;
    let declared_reserve = policy
        .context_policy
        .output_reserve
        .saturating_add(policy.context_policy.reasoning_reserve);
    let input_budget = limits.input_budget(declared_reserve);
    let exact = |tokens: u32| TokenCount::reported(u64::from(tokens)).render();
    let with_confidence = |tokens: u32, confidence: EstimationConfidence| match confidence {
        EstimationConfidence::Exact => TokenCount::reported(u64::from(tokens)).render(),
        EstimationConfidence::Estimated => TokenCount::estimated(u64::from(tokens)).render(),
    };

    let mut rows = Vec::new();
    if let Some(plan) = &status.context_plan {
        let percent_prefix = if plan.confidence == EstimationConfidence::Estimated {
            "~"
        } else {
            ""
        };
        rows.push(field(
            "context window",
            format!(
                "{percent_prefix}{}% input left ({} used / {} budget)",
                plan.percent_left(),
                plan.render_input(),
                exact(plan.input_budget_tokens),
            ),
        ));
        rows.push(field(
            "model window",
            format!(
                "{} total · {} reserved",
                exact(limits.context_tokens),
                exact(plan.reserved_tokens)
            ),
        ));
        rows.push(field(
            "context plan",
            format!(
                "{} · {} segments",
                plan.confidence_label(),
                plan.segment_count
            ),
        ));
        for (kind, tokens) in &plan.totals {
            rows.push(field(
                format!("  {}", kind.replace('_', " ")),
                with_confidence(*tokens, plan.confidence),
            ));
        }
        let compaction_target = exact(policy.compaction_policy.low_watermark);
        rows.push(field(
            "compaction",
            if let Some(summary_tokens) = plan.totals.get("summary").filter(|tokens| **tokens > 0) {
                format!(
                    "applied · {} summary · {} recovery target",
                    with_confidence(*summary_tokens, plan.confidence),
                    compaction_target,
                )
            } else {
                format!("enabled on overflow · {compaction_target} recovery target")
            },
        ));
    } else {
        rows.extend([
            field(
                "context window",
                format!(
                    "not planned yet (? used / {} input budget)",
                    exact(input_budget)
                ),
            ),
            field(
                "model window",
                format!(
                    "{} total · {} reserved",
                    exact(limits.context_tokens),
                    exact(declared_reserve)
                ),
            ),
            field("context plan", "waiting for first turn"),
            field(
                "compaction",
                format!(
                    "enabled on overflow · {} recovery target",
                    exact(policy.compaction_policy.low_watermark)
                ),
            ),
        ]);
    }
    let (reasoning, reasoning_controls) = reasoning_status_values(policy);
    rows.extend([
        field("provider input (session)", status.context.render()),
        field("cache read (session)", status.render_cache()),
        field("cache", cache_status_value(status)),
        field("reasoning", reasoning),
        field("reasoning controls", reasoning_controls),
    ]);
    DiagnosticsSection { rows }
}

fn goal_section(host: &HostSession) -> DiagnosticsSection {
    let goal = match host.goal() {
        Err(error) => {
            return DiagnosticsSection {
                rows: vec![field("goal", format!("unavailable ({error})"))],
            };
        }
        Ok(None) => {
            return DiagnosticsSection {
                rows: vec![field("goal", "none")],
            };
        }
        Ok(Some(goal)) => goal,
    };
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
    DiagnosticsSection {
        rows: vec![
            field("goal", goal.objective),
            field("status", goal.status.as_str()),
            field(
                "tokens",
                format!("{usage} · {}", goal.usage.provenance.as_str()),
            ),
            field(
                "budget",
                goal.token_budget
                    .map_or_else(|| "none".to_owned(), |tokens| tokens.to_string()),
            ),
            field(
                "active elapsed",
                render_elapsed(Duration::from_millis(goal.usage.active_elapsed_ms)),
            ),
            field("reason", reason),
            field(
                "id",
                format!("{} · generation {}", goal.id, goal.generation),
            ),
        ],
    }
}

fn field(label: impl Into<String>, value: impl Into<String>) -> DiagnosticsRow {
    DiagnosticsRow::Field {
        label: label.into(),
        value: value.into(),
    }
}
