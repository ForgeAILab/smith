//! Typed local commands and status/context rendering.

use super::*;
use smith_client::agent_report::AgentReport;
use smith_client::local_result::LocalResult;
use smith_client::mcp_report::McpReport;
use smith_client::recovery_report::RecoveryReport;
use smith_client::review_report::ReviewReport;
use smith_client::skills_report::SkillsReport;
use smith_client::status::{PriceReference, SessionCost, SessionUsage};

pub(crate) mod agent;
pub(super) mod context;
mod diff;
mod goal;
pub(super) mod mcp;
pub(super) mod recovery;
mod review;
pub(super) mod skills;
mod status;
pub(crate) mod timeline;

pub(super) fn tool_call_for_display(
    event: &RuntimeEvent,
) -> Option<agent_runtime_core::ids::ToolCallId> {
    match event {
        RuntimeEvent::ToolCallRequested { call, .. }
        | RuntimeEvent::ToolCallCompleted { call, .. } => Some(call.clone()),
        _ => None,
    }
}

pub(super) async fn handle_local_command(
    app: &mut App,
    host: &HostSession,
    project: &std::path::Path,
    mcp: Option<&crate::mcp::McpContext>,
    skills: &crate::skills::SkillContext,
    command: HostCommand,
) {
    match command {
        HostCommand::Skills(action) => match action {
            smith_client::commands::SkillsAction::List => {
                app.show_local_report(LocalResult::Skills(Box::new(skills::report(
                    skills,
                    host.runtime().skill_index(),
                ))));
            }
            // The path and the digest are what the decision binds, so the path
            // and the digest are what the confirmation shows.
            smith_client::commands::SkillsAction::Trust(skill) => match skills.confirmation(&skill)
            {
                Ok(content) => app.confirm_skill_trust(skill, content),
                Err(error) => {
                    app.show_local_report(LocalResult::Skills(Box::new(SkillsReport::Error(
                        error,
                    ))));
                }
            },
        },
        HostCommand::Mcp(action) => match (mcp, action) {
            (None, _) => app.show_local_report(LocalResult::Mcp(Box::new(mcp::report(None)))),
            (Some(context), smith_client::commands::McpAction::List) => {
                app.show_local_report(LocalResult::Mcp(Box::new(mcp::report(Some(context)))));
            }
            // Showing the resolved invocation and its content identity is the
            // whole point of the confirmation: the decision is about exactly
            // this content, so exactly this content is what gets displayed.
            (Some(context), smith_client::commands::McpAction::Trust(server)) => {
                match context.confirmation(&server) {
                    Ok(content) => app.confirm_mcp_trust(server, content),
                    Err(error) => {
                        app.show_local_report(LocalResult::Mcp(Box::new(McpReport::Error(error))));
                    }
                }
            }
        },
        HostCommand::Context => {
            app.show_local_report(LocalResult::Context(Box::new(context::report(
                &app.status,
                host.runtime().policy(),
            ))));
        }
        HostCommand::Timeline => {
            app.show_local_report(LocalResult::Timeline(Box::new(
                timeline::report(host).await,
            )));
        }
        HostCommand::Status => {
            app.show_local_report(LocalResult::Status(Box::new(status::report(
                app, host, project,
            ))));
        }
        HostCommand::Diagnostics => {
            let policy = host.runtime().policy();
            let git = GitChanges::discover(project)
                .and_then(|git| git.status_summary())
                .unwrap_or_else(|_| "unavailable (not a Git worktree)".to_owned());
            let child_count = host
                .runtime()
                .delegation()
                .and_then(|delegation| delegation.coordinator())
                .map_or(0, |coordinator| coordinator.list().len());
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
            let context = render_context_status(&app.status, policy);
            let cache_controller = host.cache_lifecycle().map_or_else(
                || "cache maintenance: unavailable".to_owned(),
                |snapshot| render_cache_controller_status(&snapshot),
            );
            let resume_capsule = host.resume_capsule().map_or_else(
                || "resume capsule: disabled".to_owned(),
                |capsule| render_resume_capsule_status(&capsule),
            );
            let harness = render_harness_status(&app.status);
            let reasoning = render_reasoning_status(policy);
            let goal = host.goal().map_or_else(
                |error| format!("unavailable ({error})"),
                |goal| goal.as_ref().map_or_else(|| "none".to_owned(), render_goal),
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
            // The same merged/root/agents shape the exit report carries,
            // named honestly when nothing has been spent yet — `/status` is
            // the one place a user checks usage mid-session rather than at
            // exit.
            let session_usage = app.session_usage();
            let usage = session_usage
                .render()
                .unwrap_or_else(|| "nothing spent yet".to_owned());
            // Unlike the exit report, `/status` reports cost as unknown
            // rather than omitting it when the catalog carries no price
            // entry for the active model (`usage-accounting`'s "Price is
            // unavailable"; `DESIGN.md` §7's `cost ?`) — a user checking
            // mid-session should never have to wonder whether "no cost line"
            // means "free" or "unpriced".
            let cost = render_status_cost(
                &session_usage,
                app.status.price(),
                (&policy.provider_name, policy.model.as_str()),
            );
            app.show_local_result(
                "diagnostics",
                format!(
                    "session: {}\nprofile: {} · posture {} · use {} · rev {} · source {}{}\n\
                     provider: {}\nmodel: {}\npermission: {:?}\n\
                     {reasoning}\n\
                     protected mid-turn recovery: {}\n\
                     {harness}\n{context}\n{cache_controller}\n{resume_capsule}\nproject: {}\nGit: {}\n\
                     goal: {goal}\nconnections: {connections}\nchildren: {}\nusage: {usage}\n\
                     cost: {cost}\n\
                     change attribution: {}",
                    host.session().id(),
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
                    policy.provider_name,
                    policy.model,
                    policy.approval_mode,
                    policy.mid_turn_durability.as_str(),
                    project.display(),
                    git,
                    child_count,
                    attribution,
                ),
            );
        }
        HostCommand::Goal(action) => {
            app.show_local_report(LocalResult::Goal(Box::new(
                goal::report(host, action).await,
            )));
        }
        HostCommand::Agent(selected) => {
            let report = agent::report(host, app.inspected_child.as_deref(), selected);
            match report {
                AgentReport::Inspector(snapshot) => {
                    // The inspector owns the card; leave no duplicate behind
                    // in the root timeline when the user returns with Esc.
                    let child = snapshot.summary.child.clone();
                    app.inspect_child(child.clone());
                    app.set_inspected_detail(&child, Some(snapshot));
                }
                report => {
                    if matches!(report, AgentReport::Parent) {
                        app.leave_child_inspection();
                    }
                    app.show_local_report(LocalResult::Agent(Box::new(report)));
                }
            }
        }
        HostCommand::Diff(scope) => {
            app.show_local_report(LocalResult::Diff(Box::new(diff::report(
                host, project, scope,
            ))));
        }
        HostCommand::Review(scope) => match review::report(project, scope) {
            ReviewReport::Confirmation(preview) => app.confirm_review(preview),
            report => app
                .transcript
                .push_local(LocalResult::Review(Box::new(report))),
        },
        HostCommand::Undo => match recovery::undo_preview(host) {
            RecoveryReport::UndoConfirmation(preview) => app.confirm_undo(preview),
            report => app
                .transcript
                .push_local(LocalResult::Recovery(Box::new(report))),
        },
        HostCommand::Redo => match recovery::redo_preview(host) {
            RecoveryReport::RedoConfirmation(preview) => app.confirm_redo(preview),
            report => app.show_local_report(LocalResult::Recovery(Box::new(report))),
        },
        HostCommand::Revert(scope) => match recovery::revert_preview(host, project, scope) {
            RecoveryReport::RevertConfirmation(preview) => app.confirm_revert(preview),
            report => app
                .transcript
                .push_local(LocalResult::Recovery(Box::new(report))),
        },
    }
}

/// The `/status` cost line.
///
/// Two honesty rules that only look alike from a distance: an empty session
/// has nothing to price (there is no "unknown" about zero tokens), while a
/// non-empty session against an unpriced model genuinely has an unknown
/// cost — `usage-accounting`'s "Price is unavailable" requires Smith to show
/// the counters and report cost as unknown rather than assuming a price, not
/// to omit the field the way the exit report does for the same case.
/// `binding` names the active provider/model even when unpriced, matching
/// `/status`'s own `provider:`/`model:` lines rather than naming nothing.
pub(super) fn render_status_cost(
    usage: &SessionUsage,
    price: Option<&PriceReference>,
    binding: (&str, &str),
) -> String {
    if usage.is_empty() {
        return "nothing spent yet".to_owned();
    }
    let Some(price) = price else {
        let (provider, model) = binding;
        return format!("unknown · no price reference for {provider}/{model}");
    };
    let cost = SessionCost::compute(usage, price);
    format!(
        "{} {} · {}/{}",
        cost.render(),
        cost.label.as_str(),
        price.provider,
        price.model,
    )
}

pub(super) fn render_goal(goal: &GoalProjection) -> String {
    let status = goal.status.as_str();
    let usage = goal
        .usage
        .charged_tokens
        .map_or_else(|| "unknown".to_owned(), |tokens| tokens.to_string());
    let budget = goal
        .token_budget
        .map_or_else(|| "none".to_owned(), |tokens| tokens.to_string());
    let provenance = goal.usage.provenance.as_str();
    let reason = goal.stopped_reason.as_ref().map_or_else(
        || "none".to_owned(),
        |reason| {
            reason.detail.as_ref().map_or_else(
                || reason.code.clone(),
                |detail| format!("{} · {detail}", reason.code),
            )
        },
    );
    format!(
        "{}\nstatus: {status}\ntokens: {usage} · {provenance}\nbudget: {budget}\nactive elapsed: {}\nreason: {reason}\nid: {} · generation {}",
        goal.objective,
        render_elapsed(Duration::from_millis(goal.usage.active_elapsed_ms)),
        goal.id,
        goal.generation,
    )
}

pub(super) fn render_harness_status(status: &Status) -> String {
    let capabilities = &status.capabilities;
    let mut lines = Vec::new();
    match &capabilities.registry {
        Some((fingerprint, entries)) => {
            lines.push(format!(
                "registry snapshot: {fingerprint} · {entries} entries"
            ));
        }
        None => lines.push("registry snapshot: waiting for live lifecycle".to_owned()),
    }
    if let Some((fingerprint, visible)) = &capabilities.view {
        lines.push(format!(
            "scoped capability view: {fingerprint} · {visible} visible"
        ));
    }
    if let Some((revision, candidates)) = &capabilities.retrieval {
        let candidates = if candidates.is_empty() {
            "(none)".to_owned()
        } else {
            candidates.join(", ")
        };
        lines.push(format!(
            "latest capability retrieval: {revision} · {candidates}"
        ));
    }
    if let Some((epoch, active)) = &capabilities.activation {
        let active = if active.is_empty() {
            "(none)".to_owned()
        } else {
            active.join(", ")
        };
        lines.push(format!("activation epoch: {epoch} · {active}"));
    }
    if let Some(plan) = &status.context_plan {
        lines.push(format!(
            "context provenance: {} · cache {}",
            plan.fingerprint, plan.cache_fingerprint
        ));
    }
    if capabilities.compactions > 0 {
        lines.push(format!(
            "context compaction: {} run(s) · {} tokens reclaimed",
            capabilities.compactions, capabilities.reclaimed_tokens
        ));
    }
    lines.join("\n")
}

pub(super) fn render_context_status(status: &Status, policy: &RuntimePolicy) -> String {
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

    let mut lines = Vec::new();
    if let Some(plan) = &status.context_plan {
        let percent_prefix = if plan.confidence == EstimationConfidence::Estimated {
            "~"
        } else {
            ""
        };
        lines.push(format!(
            "context window: {percent_prefix}{}% input left ({} used / {} budget)",
            plan.percent_left(),
            plan.render_input(),
            exact(plan.input_budget_tokens),
        ));
        lines.push(format!(
            "model window: {} total · {} reserved",
            exact(limits.context_tokens),
            exact(plan.reserved_tokens),
        ));
        lines.push(format!(
            "context plan: {} · {} segments",
            plan.confidence_label(),
            plan.segment_count,
        ));
        for (kind, tokens) in &plan.totals {
            lines.push(format!(
                "  {}: {}",
                kind.replace('_', " "),
                with_confidence(*tokens, plan.confidence),
            ));
        }
        let compaction_target = exact(policy.compaction_policy.low_watermark);
        if let Some(summary_tokens) = plan.totals.get("summary").filter(|tokens| **tokens > 0) {
            lines.push(format!(
                "compaction: applied · {} summary · {} recovery target",
                with_confidence(*summary_tokens, plan.confidence),
                compaction_target,
            ));
        } else {
            lines.push(format!(
                "compaction: enabled on overflow · {compaction_target} recovery target"
            ));
        }
    } else {
        lines.push(format!(
            "context window: not planned yet (? used / {} input budget)",
            exact(input_budget),
        ));
        lines.push(format!(
            "model window: {} total · {} reserved",
            exact(limits.context_tokens),
            exact(declared_reserve),
        ));
        lines.push("context plan: waiting for first turn".to_owned());
        lines.push(format!(
            "compaction: enabled on overflow · {} recovery target",
            exact(policy.compaction_policy.low_watermark),
        ));
    }
    lines.push(format!(
        "provider input (session): {}",
        status.context.render()
    ));
    lines.push(format!("cache read (session): {}", status.render_cache()));
    lines.push(render_cache_status(status));
    lines.push(render_reasoning_status(policy));
    lines.join("\n")
}

/// Renders the canonical cache state and derived miss diagnostics shared by
/// `/status` and the detailed context view. Unknown values remain `?`.
pub(super) fn render_cache_status(status: &Status) -> String {
    format!("cache: {}", cache_status_value(status))
}

fn cache_status_value(status: &Status) -> String {
    let usage = status.session_usage();
    let Some(summary) = status.cache_summary() else {
        return append_cache_lifecycle(
            format!(
                "state unknown · CH ? · misses {} · re-billed {}",
                usage.cache_miss_count, usage.cache_rebilled_tokens,
            ),
            status,
        );
    };
    let expected = summary
        .expected_read_tokens
        .map_or_else(|| "?".to_owned(), |tokens| tokens.to_string());
    let observed = summary
        .observed_read_tokens
        .map_or_else(|| "?".to_owned(), |tokens| tokens.to_string());
    let missed = summary
        .missed_tokens
        .map_or_else(|| "?".to_owned(), |tokens| tokens.to_string());
    let cost = summary.extra_cost_micro_usd.map_or_else(
        || "?".to_owned(),
        |micro| format!("${}.{:06} derived", micro / 1_000_000, micro % 1_000_000),
    );
    let confidence = match summary.confidence {
        Some(smith_runtime::client::EstimationConfidence::Exact) => "exact",
        Some(smith_runtime::client::EstimationConfidence::Estimated) => "estimated",
        None => "?",
    };
    append_cache_lifecycle(
        format!(
            "state {} · CH {} · expected {} · observed {} · missed {} · confidence {} · misses {} · re-billed {} · extra cost {}",
            summary.state.as_str(),
            summary.render_ch(),
            expected,
            observed,
            missed,
            confidence,
            usage.cache_miss_count,
            usage.cache_rebilled_tokens,
            cost,
        ),
        status,
    )
}

fn append_cache_lifecycle(mut line: String, status: &Status) -> String {
    let lifecycle = status.cache_lifecycle();
    if let Some(identity) = &lifecycle.cache_identity {
        line.push_str(&format!(" · identity {identity}"));
    }
    line.push_str(&format!(
        " · guarantee {} · maintenance calls {}",
        lifecycle
            .guaranteed_until_ms
            .map_or_else(|| "?".to_owned(), |timestamp| format!("{timestamp}ms")),
        lifecycle.maintenance_calls_used,
    ));
    if let Some(reason) = lifecycle.suspension_reason {
        line.push_str(&format!(" · suspended {}", cache_operation_reason(reason)));
    }
    line
}

/// Readable enum words. Never run this over route names, IDs, or user text.
fn diagnostic_label(value: impl std::fmt::Debug) -> String {
    let mut result = String::new();
    let mut previous_lowercase = false;
    for ch in format!("{value:?}").chars() {
        if ch.is_ascii_uppercase() && previous_lowercase {
            result.push(' ');
        }
        result.push(ch.to_ascii_lowercase());
        previous_lowercase = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    result
}

pub(super) fn render_cache_controller_summary(
    controller: &smith_runtime::cache_controller::CacheControllerSnapshot,
) -> String {
    format!(
        "cache maintenance: {}",
        cache_controller_summary_value(controller)
    )
}

fn cache_controller_summary_value(
    controller: &smith_runtime::cache_controller::CacheControllerSnapshot,
) -> String {
    use smith_runtime::cache_lifecycle::CacheMaintenanceMode;
    if !controller.synthetic_attempts.is_empty() {
        let usage = controller
            .synthetic_attempts
            .iter()
            .fold([0u64; 5], |mut total, attempt| {
                for (sum, value) in total.iter_mut().zip([
                    attempt.usage.input_uncached,
                    attempt.usage.input_cached,
                    attempt.usage.cache_write,
                    attempt.usage.output,
                    attempt.usage.reasoning,
                ]) {
                    *sum = sum.saturating_add(value);
                }
                total
            });
        return format!(
            "{} attempts · input {} · cached {} · writes {} · output {} · reasoning {}",
            controller.synthetic_attempts.len(),
            usage[0],
            usage[1],
            usage[2],
            usage[3],
            usage[4]
        );
    }
    if controller.requested_maintenance == CacheMaintenanceMode::Off {
        return "off".to_owned();
    }
    if controller.effective_maintenance == CacheMaintenanceMode::Off {
        return "unavailable under the current provider or policy; see /diagnostics".to_owned();
    }
    if controller.operation_in_flight {
        return "running".to_owned();
    }
    if controller.effective_maintenance == CacheMaintenanceMode::Observe {
        return "observe only (no background requests)".to_owned();
    }
    if let Some(at) = controller.scheduled_for {
        return format!(
            "scheduled for {}",
            smith_client::time_display::local_timestamp(at.0)
        );
    }
    "idle".to_owned()
}

pub(super) fn render_resume_summary(
    capsule: &smith_runtime::resume_capsule::RedactedResumeCapsule,
) -> String {
    format!("resume checkpoint: {}", resume_summary_value(capsule))
}

fn resume_summary_value(capsule: &smith_runtime::resume_capsule::RedactedResumeCapsule) -> String {
    capsule.last_persisted_at.map_or_else(
        || "not yet saved".to_owned(),
        |at| {
            format!(
                "saved {}",
                smith_client::time_display::local_timestamp(at.0)
            )
        },
    )
}

/// Renders Smith's bounded adaptive scheduler diagnostics.
pub(super) fn render_cache_controller_status(
    controller: &smith_runtime::cache_controller::CacheControllerSnapshot,
) -> String {
    let requested = format!("{:?}", controller.requested_maintenance).to_ascii_lowercase();
    let effective = format!("{:?}", controller.effective_maintenance).to_ascii_lowercase();
    let scheduled = controller.scheduled_for.map_or_else(
        || "none".to_owned(),
        |at| smith_client::time_display::local_timestamp(at.0),
    );
    let decision = controller.decision.as_ref().map_or_else(
        || "none".to_owned(),
        |decision| {
            let mut rendered = diagnostic_label(decision.disposition);
            if let Some(reason) = decision.reason {
                rendered.push_str(&format!(" / {}", diagnostic_label(reason)));
            }
            rendered
        },
    );
    let narrowing = controller
        .narrowing_reason
        .as_ref()
        .map(|reason| format!(" · narrowed {reason}"))
        .unwrap_or_default();
    let idle = render_idle_compaction_status(controller);
    let synthetic = render_synthetic_attempt_status(controller);
    let Some(lease) = controller.lifecycle.current() else {
        return format!(
            "cache maintenance: requested {requested} · effective {effective} · authority {} · lease none · calls {}/{} · scheduled {scheduled} · decision {decision}{narrowing}{idle}{synthetic}",
            if controller.synthetic_spend_authorized {
                "allowed"
            } else {
                "denied"
            },
            controller.interval_attempts,
            controller.policy.max_maintenance_calls,
        );
    };
    let guarantee = lease.guaranteed_until.map_or_else(
        || "?".to_owned(),
        |at| smith_client::time_display::local_timestamp(at.0),
    );
    let reads = lease
        .observed_read_tokens
        .map_or_else(|| "?".to_owned(), |tokens| tokens.to_string());
    let writes = lease
        .observed_write_tokens
        .map_or_else(|| "?".to_owned(), |tokens| tokens.to_string());
    let last = lease.last_operation_purpose.map_or_else(
        || "none".to_owned(),
        |purpose| format!("{purpose:?}").to_ascii_lowercase(),
    );
    let suspension = lease.suspension_reason.map_or_else(
        || "none".to_owned(),
        |reason| format!("{reason:?}").to_ascii_lowercase(),
    );
    format!(
        "cache maintenance: requested {requested} · effective {effective} · authority {} · lease {:?} · preserved {} · reads {reads} · writes {writes} · guarantee {guarantee} · calls {}/{} · synthetic in/out {}/{} · last {last} · scheduled {scheduled} · decision {decision} · suspension {suspension}{narrowing}{idle}{synthetic}",
        if controller.synthetic_spend_authorized {
            "allowed"
        } else {
            "denied"
        },
        lease.status,
        lease.structurally_preserved_prefix_tokens,
        controller.interval_attempts,
        controller.policy.max_maintenance_calls,
        lease.maintenance_input_tokens,
        lease.maintenance_output_tokens,
    )
}

fn render_synthetic_attempt_status(
    controller: &smith_runtime::cache_controller::CacheControllerSnapshot,
) -> String {
    let Some(attempt) = controller.synthetic_attempts.last() else {
        return " · synthetic attempts 0".to_owned();
    };
    let identity = attempt.cache_identity.as_deref().unwrap_or("none");
    let cost = attempt.cost_micro_usd.map_or_else(
        || "?".to_owned(),
        |micro| format!("${}.{:06}", micro / 1_000_000, micro % 1_000_000),
    );
    format!(
        " · synthetic attempts {} · latest {:?} {}/{} · identity {identity} · usage in/cached/write/out/reasoning {}/{}/{}/{}/{} · cost {cost}/{:?} · latency {}ms · status {}",
        controller.synthetic_attempts.len(),
        attempt.purpose,
        attempt.provider,
        attempt.model,
        attempt.usage.input_uncached,
        attempt.usage.input_cached,
        attempt.usage.cache_write,
        attempt.usage.output,
        attempt.usage.reasoning,
        attempt.cost_provenance,
        attempt.latency_ms,
        attempt.status,
    )
}

fn render_idle_compaction_status(
    controller: &smith_runtime::cache_controller::CacheControllerSnapshot,
) -> String {
    let decision = controller.idle_compaction_decision.as_ref().map_or_else(
        || "none".to_owned(),
        |decision| {
            let mut value = format!("{:?}", decision.disposition).to_ascii_lowercase();
            if let Some(reason) = decision.reason {
                value.push_str(&format!("/{reason:?}").to_ascii_lowercase());
            }
            value
        },
    );
    let outcome = controller.idle_compaction_outcome.map_or_else(
        || "none".to_owned(),
        |outcome| format!("{outcome:?}").to_ascii_lowercase(),
    );
    let reason = controller
        .idle_compaction_reason
        .as_deref()
        .unwrap_or("none");
    let latency = controller.idle_compaction_latency_ms.map_or_else(
        || "?".to_owned(),
        |milliseconds| format!("{milliseconds}ms"),
    );
    let provider = controller
        .idle_compaction_provider
        .as_deref()
        .unwrap_or("?");
    let model = controller.idle_compaction_model.as_deref().unwrap_or("?");
    let revision = controller
        .idle_compaction_revision
        .as_ref()
        .map_or_else(|| "?".to_owned(), ToString::to_string);
    let usage = &controller.idle_compaction_usage;
    format!(
        " · idle attempted {} · idle decision {decision} · idle outcome {outcome} · idle reason {reason} · idle latency {latency} · idle route {provider}/{model}/{revision} · idle usage in/cached/write/out/reasoning {}/{}/{}/{}/{}",
        controller.idle_compaction.attempted,
        usage.input_uncached,
        usage.input_cached,
        usage.cache_write,
        usage.output,
        usage.reasoning,
    )
}

pub(super) fn render_resume_capsule_status(
    capsule: &smith_runtime::resume_capsule::RedactedResumeCapsule,
) -> String {
    let summary = capsule.semantic_summary.as_ref().map_or_else(
        || "none".to_owned(),
        |summary| {
            format!(
                "{:?}/{}/{} · rev {} · coverage {} · {:?}",
                summary.provenance.purpose,
                summary.provenance.provider,
                summary.provenance.model,
                summary.provenance.revision,
                summary.provenance.source_coverage.len(),
                summary.provenance.outcome,
            )
        },
    );
    format!(
        "resume capsule: schema {} · watermark {} · persisted {} · summary {summary}",
        capsule.schema_version,
        capsule.last_persisted_watermark,
        capsule.last_persisted_at.map_or_else(
            || "not yet saved".to_owned(),
            |at| smith_client::time_display::local_timestamp(at.0)
        ),
    )
}

const fn cache_operation_reason(
    reason: smith_runtime::client::CacheOperationReason,
) -> &'static str {
    use smith_runtime::client::CacheOperationReason;
    match reason {
        CacheOperationReason::Unsupported => "unsupported",
        CacheOperationReason::MissingConformance => "missing_conformance",
        CacheOperationReason::MissingAuthority => "missing_authority",
        CacheOperationReason::InvalidIdentity => "invalid_identity",
        CacheOperationReason::BudgetExceeded => "budget_exceeded",
        CacheOperationReason::Cancelled => "cancelled",
        CacheOperationReason::DeadlineExceeded => "deadline_exceeded",
        CacheOperationReason::CapabilityChanged => "capability_changed",
        CacheOperationReason::IdentityChanged => "identity_changed",
        CacheOperationReason::CacheMiss => "cache_miss",
        CacheOperationReason::CacheExpired => "cache_expired",
        CacheOperationReason::ProtocolViolation => "protocol_violation",
        CacheOperationReason::Shutdown => "shutdown",
        CacheOperationReason::Conflict => "conflict",
    }
}

pub(super) fn render_reasoning_status(policy: &RuntimePolicy) -> String {
    let (reasoning, controls) = reasoning_status_values(policy);
    format!("reasoning: {reasoning}\nreasoning controls: {controls}")
}

fn reasoning_status_values(policy: &RuntimePolicy) -> (String, String) {
    let support = match policy.reasoning.support {
        ReasoningSupport::Unsupported => "unsupported",
        ReasoningSupport::Fixed => "fixed",
        ReasoningSupport::Controllable => "controllable",
    };
    let efforts = if policy.reasoning.efforts.is_empty() {
        "none".to_owned()
    } else {
        policy.reasoning.efforts.join(", ")
    };
    (
        format!(
            "{} · effort {} · {}",
            policy.reasoning.effective_state(),
            policy.reasoning.effective_effort(),
            policy.reasoning.selection_source,
        ),
        format!(
            "{support} · switch {} · efforts {efforts} · {}",
            policy.reasoning.switch.as_str(),
            policy.reasoning.capability_source,
        ),
    )
}

pub(super) enum LocalOutcome {
    Agent(Box<AgentReport>),
    Review(Box<ReviewReport>),
    Notice {
        /// The transcript block label for child lifecycle notices.
        source: &'static str,
        text: String,
    },
    Error(String),
    Shell {
        content: String,
        is_error: bool,
    },
}

#[cfg(test)]
mod diagnostic_label_tests {
    use super::diagnostic_label;
    #[test]
    fn enum_words_are_readable() {
        #[derive(Debug)]
        enum Reason {
            ProviderEvidenceUnavailable,
        }
        assert_eq!(
            diagnostic_label(Reason::ProviderEvidenceUnavailable),
            "provider evidence unavailable"
        );
    }
}
