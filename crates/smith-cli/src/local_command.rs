//! Typed local commands and status/context rendering.

use agent_runtime_core::provider::ReasoningSupport;
use smith_client::agent_report::{AgentReport, AgentSnapshot};
use smith_client::commands::HostCommand;
use smith_client::local_result::LocalResult;
use smith_client::recovery_report::{RecoveryPreview, RevertPreview};
use smith_client::review_report::{ReviewPreview, ReviewReport};
use smith_client::status::{PriceReference, SessionCost, SessionUsage, Status};
use smith_runtime::client::SmithEventKind as RuntimeEvent;
use smith_runtime::factory::RuntimePolicy;
use smith_runtime::host::HostSession;
use smith_tui::app::App;

pub(crate) mod agent;
pub(super) mod context;
pub(super) mod diagnostics;
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
    let report = match command {
        HostCommand::Skills(action) => {
            skills::command(skills, host.runtime().skill_index(), action)
        }
        HostCommand::Mcp(action) => mcp::command(mcp, action),
        HostCommand::Context => CommandReport::Show(LocalResult::Context(Box::new(
            context::report(&app.status, host.runtime().policy()),
        ))),
        HostCommand::Timeline => CommandReport::Show(LocalResult::Timeline(Box::new(
            timeline::report(host).await,
        ))),
        HostCommand::Status => CommandReport::Show(LocalResult::Status(Box::new(status::report(
            app, host, project,
        )))),
        HostCommand::Diagnostics => CommandReport::Show(LocalResult::Diagnostics(Box::new(
            diagnostics::report(app, host, project),
        ))),
        HostCommand::Goal(action) => CommandReport::Show(LocalResult::Goal(Box::new(
            goal::report(host, action).await,
        ))),
        HostCommand::Agent(selected) => {
            agent::command(host, app.inspected_child.as_deref(), selected)
        }
        HostCommand::Diff(scope) => CommandReport::Show(LocalResult::Diff(Box::new(diff::report(
            host, project, scope,
        )))),
        HostCommand::Review(scope) => review::command(project, scope),
        HostCommand::Undo => recovery::undo_command(host),
        HostCommand::Redo => recovery::redo_command(host),
        HostCommand::Revert(scope) => recovery::revert_command(host, project, scope),
    };
    report.present(app);
}

/// A command's report and the surface action that presents it.
/// Kept in the CLI because focus and confirmation belong to this surface.
pub(super) enum CommandReport {
    Show(LocalResult),
    Append(LocalResult),
    Inspect(Box<AgentSnapshot>),
    Parent,
    SkillTrust { skill: String, content: String },
    McpTrust { server: String, content: String },
    ReviewConfirmation(ReviewPreview),
    UndoConfirmation(RecoveryPreview),
    RedoConfirmation(RecoveryPreview),
    RevertConfirmation(RevertPreview),
}

impl CommandReport {
    fn present(self, app: &mut App) {
        match self {
            Self::Show(report) => app.show_local_report(report),
            Self::Append(report) => app.transcript.push_local(report),
            Self::Inspect(snapshot) => {
                // The inspector owns the card; leave no duplicate behind
                // in the root timeline when the user returns with Esc.
                let child = snapshot.summary.child.clone();
                app.inspect_child(child.clone());
                app.set_inspected_detail(&child, Some(*snapshot));
            }
            Self::Parent => {
                app.leave_child_inspection();
                app.show_local_report(LocalResult::Agent(Box::new(AgentReport::Parent)));
            }
            Self::SkillTrust { skill, content } => app.confirm_skill_trust(skill, content),
            Self::McpTrust { server, content } => app.confirm_mcp_trust(server, content),
            Self::ReviewConfirmation(preview) => app.confirm_review(preview),
            Self::UndoConfirmation(preview) => app.confirm_undo(preview),
            Self::RedoConfirmation(preview) => app.confirm_redo(preview),
            Self::RevertConfirmation(preview) => app.confirm_revert(preview),
        }
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
        "{} {} · {}",
        cost.render(),
        cost.label.as_str(),
        price.render_sources(usage),
    )
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
        echo: u64,
        call: Option<agent_runtime_core::ids::ToolCallId>,
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
