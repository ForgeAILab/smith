//! Captures grouped facts for the local `/diagnostics` report.

use std::path::Path;
use std::time::Duration;

use smith_client::diagnostics_report::{DiagnosticsReport, DiagnosticsRow, DiagnosticsSection};
use smith_client::status::{Confidence, SessionCost, Status, TokenCount, render_elapsed};
use smith_host::GitChanges;
use smith_runtime::client::EstimationConfidence;
use smith_runtime::factory::RuntimePolicy;
use smith_runtime::host::HostSession;
use smith_tui::App;

use super::{cache_controller_summary_value, diagnostic_label};
use crate::resources::bounded_text;

pub(crate) fn report(app: &App, host: &HostSession, project: &Path) -> DiagnosticsReport {
    let policy = host.runtime().policy();
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

    let reasoning = &policy.reasoning;
    let mut session = vec![
        field("session", host.session().id().to_string()),
        field("profile", &policy.agent_profile),
        field("posture", policy.agent_posture.as_str()),
        field(
            "profile uses",
            policy
                .agent_profile_uses
                .iter()
                .map(|placement| placement.as_str())
                .collect::<Vec<_>>()
                .join("+"),
        ),
        field(
            "profile revision",
            bounded_text(&policy.agent_profile_revision, 12),
        ),
        field(
            "profile source",
            bounded_text(&policy.agent_profile_source, 80),
        ),
    ];
    if policy.agent_profile_legacy {
        session.push(field("legacy adapter", "migrate to [profiles]"));
    }
    session.extend([
        field("provider", &policy.provider_name),
        field("model", policy.model.to_string()),
        field("permission", diagnostic_label(policy.approval_mode)),
        field("reasoning", reasoning.effective_state()),
        field("reasoning effort", reasoning.effective_effort()),
        field("reasoning source", &reasoning.selection_source),
        field("reasoning controls", diagnostic_label(reasoning.support)),
        field("reasoning switch", reasoning.switch.as_str()),
        field(
            "reasoning efforts",
            if reasoning.efforts.is_empty() {
                "none".to_owned()
            } else {
                reasoning.efforts.join(", ")
            },
        ),
        field("capability source", &reasoning.capability_source),
        field("project", project.display().to_string()),
        field(
            "Git",
            GitChanges::discover(project)
                .and_then(|git| git.status_summary())
                .unwrap_or_else(|_| "unavailable (not a Git worktree)".to_owned()),
        ),
    ]);
    session.extend(goal_rows(host));
    session.extend([
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
            if session_usage.is_empty() {
                "nothing spent yet".to_owned()
            } else if let Some(price) = app.status.price() {
                let cost = SessionCost::compute(&session_usage, price);
                format!(
                    "{} {} ({})",
                    cost.render(),
                    cost.label.as_str(),
                    price.render_sources(&session_usage)
                )
            } else {
                format!(
                    "unknown (no price reference for {}/{})",
                    policy.provider_name, policy.model
                )
            },
        ),
    ]);
    let mut context = context_section(&app.status, policy);
    context.rows.extend(harness_section(&app.status).rows);
    let mut cache = cache_rows(&app.status);
    if let Some(controller) = host.cache_lifecycle() {
        cache.extend(cache_controller_rows(&controller));
    } else {
        cache.push(field("maintenance", "unavailable"));
    }
    let mut recovery = vec![field(
        "mid-turn recovery",
        policy.mid_turn_durability.as_str(),
    )];
    if let Some(capsule) = host.resume_capsule() {
        recovery.extend(resume_capsule_rows(&capsule));
    } else {
        recovery.push(field("resume capsule", "disabled"));
    }
    recovery.push(field("change attribution", attribution));
    DiagnosticsReport {
        sections: vec![
            DiagnosticsSection {
                heading: "Session".to_owned(),
                rows: session,
            },
            context,
            DiagnosticsSection {
                heading: "Cache".to_owned(),
                rows: cache,
            },
            DiagnosticsSection {
                heading: "Recovery".to_owned(),
                rows: recovery,
            },
        ],
    }
}

pub(crate) fn harness_section(status: &Status) -> DiagnosticsSection {
    let capabilities = &status.capabilities;
    let mut rows = vec![field(
        "registry snapshot",
        capabilities
            .registry
            .as_ref()
            .map_or("unknown", |(fingerprint, _)| fingerprint.as_str()),
    )];
    if let Some((_, entries)) = &capabilities.registry {
        rows.push(field("  entries", entries.to_string()));
    }
    if let Some((fingerprint, visible)) = &capabilities.view {
        rows.extend([
            field("capability view", fingerprint),
            field("  visible", visible.to_string()),
        ]);
    }
    if let Some((revision, candidates)) = &capabilities.retrieval {
        rows.extend([
            field("retrieval", revision),
            field(
                "  candidates",
                if candidates.is_empty() {
                    "none".to_owned()
                } else {
                    candidates.join(", ")
                },
            ),
        ]);
    }
    if let Some((epoch, active)) = &capabilities.activation {
        rows.extend([
            field("activation epoch", epoch.to_string()),
            field(
                "  active",
                if active.is_empty() {
                    "none".to_owned()
                } else {
                    active.join(", ")
                },
            ),
        ]);
    }
    if let Some(plan) = &status.context_plan {
        rows.extend([
            field("context provenance", &plan.fingerprint),
            field("  cache", &plan.cache_fingerprint),
        ]);
    }
    rows.extend([
        field("compaction runs", capabilities.compactions.to_string()),
        field(
            "tokens reclaimed",
            capabilities.reclaimed_tokens.to_string(),
        ),
    ]);
    DiagnosticsSection {
        heading: "Context".to_owned(),
        rows,
    }
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
        rows.extend([
            field("context plan", plan.confidence_label()),
            field("plan segments", plan.segment_count.to_string()),
        ]);
        for (kind, tokens) in &plan.totals {
            rows.push(context_segment_row(
                kind,
                with_confidence(*tokens, plan.confidence),
            ));
        }
        if plan.totals.get("summary").is_some_and(|tokens| *tokens > 0) {
            rows.push(field("compaction", "applied"));
        } else {
            rows.push(field("compaction", "enabled on overflow"));
        }
    } else {
        rows.extend([
            field(
                "context window",
                format!(
                    "unknown (not planned yet; unknown used / {} input budget)",
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
            field("context plan", "unknown (waiting for first turn)"),
            field("compaction", "enabled on overflow"),
        ]);
    }
    rows.extend([
        field(
            "recovery target",
            exact(policy.compaction_policy.low_watermark),
        ),
        field(
            "provider input",
            if status.context.confidence == Confidence::Unknown {
                "unknown".to_owned()
            } else {
                format!("{} (session)", status.context.render())
            },
        ),
    ]);
    DiagnosticsSection {
        heading: "Context".to_owned(),
        rows,
    }
}

fn context_segment_row(kind: &str, value: String) -> DiagnosticsRow {
    let label = match kind {
        "system_instruction" => "system".to_owned(),
        "developer_instruction" => "developer".to_owned(),
        "ability_instruction" => "ability".to_owned(),
        other => other.replace('_', " "),
    };
    if ratatui::text::Line::from(label.as_str()).width() <= 16 {
        field(format!("  {label}"), value)
    } else {
        field("  segment", format!("{label}: {value}"))
    }
}

fn goal_rows(host: &HostSession) -> Vec<DiagnosticsRow> {
    let goal = match host.goal() {
        Err(error) => return vec![field("goal", format!("unavailable ({error})"))],
        Ok(None) => return vec![field("goal", "none")],
        Ok(Some(goal)) => goal,
    };
    let reason = goal.stopped_reason.as_ref().map_or_else(
        || "none".to_owned(),
        |reason| {
            reason.detail.as_ref().map_or_else(
                || reason.code.clone(),
                |detail| format!("{} ({detail})", reason.code),
            )
        },
    );
    vec![
        field("goal", goal.objective),
        field("goal status", goal.status.as_str()),
        field(
            "goal tokens",
            format!(
                "{} ({})",
                unknown(goal.usage.charged_tokens),
                goal.usage.provenance.as_str()
            ),
        ),
        field(
            "goal budget",
            goal.token_budget
                .map_or_else(|| "none".to_owned(), |tokens| tokens.to_string()),
        ),
        field(
            "goal active time",
            render_elapsed(Duration::from_millis(goal.usage.active_elapsed_ms)),
        ),
        field("goal stop reason", reason),
        field(
            "goal id",
            format!("{} (generation {})", goal.id, goal.generation),
        ),
    ]
}

fn cache_rows(status: &Status) -> Vec<DiagnosticsRow> {
    let usage = status.session_usage();
    let summary = status.cache_summary();
    let summary = summary.as_ref();
    let lifecycle = status.cache_lifecycle();
    vec![
        field(
            "session read",
            status.cache_read.map_or_else(
                || "unknown".to_owned(),
                |tokens| TokenCount::reported(tokens).render(),
            ),
        ),
        field(
            "state",
            summary.map_or("unknown", |summary| summary.state.as_str()),
        ),
        field(
            "cache hit",
            summary
                .and_then(|summary| summary.cache_read_percent)
                .map_or_else(|| "unknown".to_owned(), |percent| format!("{percent}%")),
        ),
        field(
            "expected",
            unknown(summary.and_then(|summary| summary.expected_read_tokens)),
        ),
        field(
            "observed",
            unknown(summary.and_then(|summary| summary.observed_read_tokens)),
        ),
        field(
            "missed",
            unknown(summary.and_then(|summary| summary.missed_tokens)),
        ),
        field(
            "confidence",
            summary
                .and_then(|summary| summary.confidence)
                .map_or_else(|| "unknown".to_owned(), diagnostic_label),
        ),
        field("misses", usage.cache_miss_count.to_string()),
        field("re-billed", usage.cache_rebilled_tokens.to_string()),
        field(
            "extra cost",
            summary
                .and_then(|summary| summary.extra_cost_micro_usd)
                .map_or_else(
                    || "unknown".to_owned(),
                    |micro| format!("{} derived", micro_usd(micro)),
                ),
        ),
        field(
            "identity",
            lifecycle.cache_identity.as_deref().unwrap_or("unknown"),
        ),
        field(
            "guarantee",
            lifecycle.guaranteed_until_ms.map_or_else(
                || "unknown".to_owned(),
                |timestamp| format!("{timestamp}ms"),
            ),
        ),
        field(
            "maintenance calls",
            lifecycle.maintenance_calls_used.to_string(),
        ),
        field(
            "suspension",
            lifecycle
                .suspension_reason
                .map_or_else(|| "none".to_owned(), diagnostic_label),
        ),
    ]
}

pub(crate) fn cache_controller_rows(
    controller: &smith_runtime::cache_controller::CacheControllerSnapshot,
) -> Vec<DiagnosticsRow> {
    let lease = controller.lifecycle.current();
    let decision = controller.decision.as_ref().map_or_else(
        || "none".to_owned(),
        |decision| {
            decision.reason.map_or_else(
                || diagnostic_label(decision.disposition),
                |reason| {
                    format!(
                        "{} / {}",
                        diagnostic_label(decision.disposition),
                        diagnostic_label(reason)
                    )
                },
            )
        },
    );
    let mut rows = vec![
        field("maintenance", cache_controller_summary_value(controller)),
        field(
            "  requested",
            diagnostic_label(controller.requested_maintenance),
        ),
        field(
            "  effective",
            diagnostic_label(controller.effective_maintenance),
        ),
        field(
            "  authority",
            if controller.synthetic_spend_authorized {
                "allowed"
            } else {
                "denied"
            },
        ),
        field(
            "  lease",
            lease.map_or_else(|| "none".to_owned(), |lease| diagnostic_label(lease.status)),
        ),
    ];
    if let Some(lease) = lease {
        rows.extend([
            field(
                "  preserved",
                lease.structurally_preserved_prefix_tokens.to_string(),
            ),
            field("  reads", unknown(lease.observed_read_tokens)),
            field("  writes", unknown(lease.observed_write_tokens)),
            field(
                "  guarantee",
                lease.guaranteed_until.map_or_else(
                    || "unknown".to_owned(),
                    |at| smith_client::time_display::local_timestamp(at.0),
                ),
            ),
            field(
                "  synthetic in/out",
                format!(
                    "{}/{}",
                    lease.maintenance_input_tokens, lease.maintenance_output_tokens
                ),
            ),
            field(
                "  last",
                lease
                    .last_operation_purpose
                    .map_or_else(|| "none".to_owned(), diagnostic_label),
            ),
            field(
                "  suspension",
                lease
                    .suspension_reason
                    .map_or_else(|| "none".to_owned(), diagnostic_label),
            ),
        ]);
    }
    rows.extend([
        field(
            "  calls",
            format!(
                "{}/{}",
                controller.interval_attempts, controller.policy.max_maintenance_calls
            ),
        ),
        field(
            "  scheduled",
            controller.scheduled_for.map_or_else(
                || "none".to_owned(),
                |at| smith_client::time_display::local_timestamp(at.0),
            ),
        ),
        field("  decision", decision),
    ]);
    if let Some(reason) = &controller.narrowing_reason {
        rows.push(field("  narrowing", reason));
    }
    rows.extend(idle_compaction_rows(controller));
    rows.extend(synthetic_attempt_rows(controller));
    rows
}

fn idle_compaction_rows(
    controller: &smith_runtime::cache_controller::CacheControllerSnapshot,
) -> Vec<DiagnosticsRow> {
    let decision = controller.idle_compaction_decision.as_ref().map_or_else(
        || "none".to_owned(),
        |decision| {
            decision.reason.map_or_else(
                || diagnostic_label(decision.disposition),
                |reason| {
                    format!(
                        "{} / {}",
                        diagnostic_label(decision.disposition),
                        diagnostic_label(reason)
                    )
                },
            )
        },
    );
    let route = match (
        controller.idle_compaction_provider.as_deref(),
        controller.idle_compaction_model.as_deref(),
        controller.idle_compaction_revision.as_ref(),
    ) {
        (None, None, None) => "unknown".to_owned(),
        (provider, model, revision) => format!(
            "{}/{}/{}",
            provider.unwrap_or("unknown"),
            model.unwrap_or("unknown"),
            unknown(revision)
        ),
    };
    vec![
        field(
            "idle compaction",
            if controller.idle_compaction.attempted {
                "attempted yes"
            } else {
                "attempted no"
            },
        ),
        field("  decision", decision),
        field(
            "  outcome",
            controller
                .idle_compaction_outcome
                .map_or_else(|| "none".to_owned(), diagnostic_label),
        ),
        field(
            "  reason",
            controller
                .idle_compaction_reason
                .as_deref()
                .unwrap_or("none"),
        ),
        field(
            "  latency",
            controller
                .idle_compaction_latency_ms
                .map_or_else(|| "unknown".to_owned(), |ms| format!("{ms}ms")),
        ),
        field("  route", route),
        field("  usage", summary_usage(&controller.idle_compaction_usage)),
    ]
}

fn synthetic_attempt_rows(
    controller: &smith_runtime::cache_controller::CacheControllerSnapshot,
) -> Vec<DiagnosticsRow> {
    let mut rows = vec![field(
        "synthetic attempts",
        controller.synthetic_attempts.len().to_string(),
    )];
    let Some(attempt) = controller.synthetic_attempts.last() else {
        return rows;
    };
    rows.extend([
        field("  purpose", diagnostic_label(attempt.purpose)),
        field("  route", format!("{}/{}", attempt.provider, attempt.model)),
        field(
            "  identity",
            attempt.cache_identity.as_deref().unwrap_or("none"),
        ),
        field("  usage", summary_usage(&attempt.usage)),
        field(
            "  cost",
            format!(
                "{} ({})",
                attempt
                    .cost_micro_usd
                    .map_or_else(|| "unknown".to_owned(), micro_usd),
                diagnostic_label(attempt.cost_provenance)
            ),
        ),
        field("  latency", format!("{}ms", attempt.latency_ms)),
        field("  status", &attempt.status),
    ]);
    rows
}

fn resume_capsule_rows(
    capsule: &smith_runtime::resume_capsule::RedactedResumeCapsule,
) -> Vec<DiagnosticsRow> {
    let mut rows = vec![
        field("capsule schema", capsule.schema_version.to_string()),
        field(
            "capsule watermark",
            capsule.last_persisted_watermark.to_string(),
        ),
        field(
            "capsule persisted",
            capsule.last_persisted_at.map_or_else(
                || "not yet saved".to_owned(),
                |at| smith_client::time_display::local_timestamp(at.0),
            ),
        ),
    ];
    if let Some(summary) = &capsule.semantic_summary {
        let provenance = &summary.provenance;
        rows.extend([
            field(
                "capsule summary",
                format!(
                    "{} {}/{}/{}",
                    diagnostic_label(provenance.purpose),
                    provenance.provider,
                    provenance.model,
                    provenance.revision
                ),
            ),
            field(
                "summary coverage",
                provenance.source_coverage.len().to_string(),
            ),
            field("summary outcome", diagnostic_label(provenance.outcome)),
        ]);
    } else {
        rows.push(field("capsule summary", "none"));
    }
    rows
}

fn unknown(value: Option<impl std::fmt::Display>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
}

fn micro_usd(micro: u128) -> String {
    format!("${}.{:06}", micro / 1_000_000, micro % 1_000_000)
}

fn summary_usage(usage: &smith_runtime::resume_capsule::SummaryUsage) -> String {
    format!(
        "input {} · cached {} · writes {} · output {} · reasoning {}",
        usage.input_uncached, usage.input_cached, usage.cache_write, usage.output, usage.reasoning
    )
}

fn field(label: impl Into<String>, value: impl Into<String>) -> DiagnosticsRow {
    DiagnosticsRow::Field {
        label: label.into(),
        value: value.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_segment_labels_fit_eighteen_display_columns() {
        for kind in [
            "system_instruction",
            "developer_instruction",
            "ability_instruction",
            "history",
            "tool_schema",
            "future_context_segment_kind",
            "宽宽宽宽宽宽宽宽宽",
        ] {
            let DiagnosticsRow::Field { label, value } =
                context_segment_row(kind, "~200".to_owned())
            else {
                panic!("context segment must be a field");
            };
            assert!(label.starts_with("  "));
            assert!(
                ratatui::text::Line::from(label.as_str()).width() <= 18,
                "{label}"
            );
            assert!(value.ends_with("~200"));
            if label == "  segment" {
                assert!(value.starts_with(&kind.replace('_', " ")), "{value}");
            }
        }
    }
}
