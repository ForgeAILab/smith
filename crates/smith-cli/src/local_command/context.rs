//! Captures the host and client values for the local `/context` report.

use std::collections::BTreeMap;

use smith_client::context_report::{
    ContextCapacity, ContextCategory, ContextCategoryKind, ContextCompaction, ContextReport,
    ContextUsage, ContextWindow,
};
use smith_client::status::{Status, TokenCount};
use smith_runtime::client::EstimationConfidence;
use smith_runtime::factory::RuntimePolicy;

use super::{cache_status_value, reasoning_status_values};

pub(crate) fn report(status: &Status, policy: &RuntimePolicy) -> ContextReport {
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
    let recovery_target = exact(policy.compaction_policy.low_watermark);
    let (reasoning, reasoning_controls) = reasoning_status_values(policy);
    let mut report = ContextReport {
        available_windows: policy
            .context_windows
            .iter()
            .map(|name| ContextWindow {
                name: name.clone(),
                active: policy.context_window.as_deref() == Some(name.as_str()),
            })
            .collect(),
        summary: format!("{} · usage unavailable until the first turn", policy.model,),
        usage: ContextUsage::Unavailable,
        categories: vec![
            ContextCategory {
                kind: ContextCategoryKind::System,
                label: "system instructions".to_owned(),
                tokens: 0,
                value: "? (not counted yet)".to_owned(),
            },
            ContextCategory {
                kind: ContextCategoryKind::Tool,
                label: "tool schemas".to_owned(),
                tokens: 0,
                value: "? (not counted yet)".to_owned(),
            },
        ],
        free_input: ContextCapacity {
            tokens: input_budget,
            value: exact(input_budget),
        },
        reserve: ContextCapacity {
            tokens: declared_reserve,
            value: exact(declared_reserve),
        },
        model_window: format!(
            "{} total · {} input budget",
            exact(limits.context_tokens),
            exact(input_budget),
        ),
        counting: "waiting for first context plan".to_owned(),
        compaction: ContextCompaction::Enabled { recovery_target },
        tool_context: if policy.artifact_offloading {
            format!(
                "offload above {} serialized bytes · artifact pages up to {} bytes",
                policy.tool_output_context.inline_bytes,
                policy.tool_output_context.artifact_page_bytes,
            )
        } else {
            "artifact storage unavailable; ordinary output limits still apply".to_owned()
        },
        provider_input: status.context.render(),
        cache_read: status.render_cache(),
        cache: cache_status_value(status),
        reasoning,
        reasoning_controls,
    };

    if let Some(plan) = &status.context_plan {
        let percent_prefix = if plan.confidence == EstimationConfidence::Estimated {
            "~"
        } else {
            ""
        };
        report.summary = format!(
            "{} · {} / {} input tokens · {percent_prefix}{}% left",
            policy.model,
            plan.render_input(),
            exact(plan.input_budget_tokens),
            plan.percent_left(),
        );
        report.usage = match plan.confidence {
            EstimationConfidence::Exact => ContextUsage::Exact,
            EstimationConfidence::Estimated => ContextUsage::Estimated,
        };

        let mut categories = display_categories(&plan.totals);
        let categorized = categories.iter().fold(0u32, |total, category| {
            total.saturating_add(category.tokens)
        });
        if plan.input_tokens > categorized {
            categories.push(DisplayCategory {
                label: "other context".to_owned(),
                kind: ContextCategoryKind::Other,
                tokens: plan.input_tokens - categorized,
                rank: u8::MAX,
            });
        }
        report.categories = categories
            .into_iter()
            .map(|category| ContextCategory {
                kind: category.kind,
                label: category.label,
                tokens: category.tokens,
                value: format!(
                    "{} ({})",
                    with_confidence(category.tokens, plan.confidence),
                    render_percent(category.tokens, plan.input_budget_tokens),
                ),
            })
            .collect();
        let free_tokens = plan.remaining_tokens();
        report.free_input = ContextCapacity {
            tokens: free_tokens,
            value: format!(
                "{} ({})",
                with_confidence(free_tokens, plan.confidence),
                render_percent(free_tokens, plan.input_budget_tokens),
            ),
        };
        report.reserve = ContextCapacity {
            tokens: plan.reserved_tokens,
            value: format!(
                "{} ({})",
                exact(plan.reserved_tokens),
                render_percent(
                    plan.reserved_tokens,
                    plan.input_budget_tokens
                        .saturating_add(plan.reserved_tokens),
                ),
            ),
        };
        report.model_window = format!(
            "{} total · {} input budget",
            exact(limits.context_tokens),
            exact(plan.input_budget_tokens),
        );
        report.counting = format!(
            "{} · {} segments",
            plan.confidence_label(),
            plan.segment_count,
        );
        if let Some(summary_tokens) = plan.totals.get("summary").filter(|tokens| **tokens > 0) {
            report.compaction = ContextCompaction::Applied {
                summary: with_confidence(*summary_tokens, plan.confidence),
                recovery_target: exact(policy.compaction_policy.low_watermark),
            };
        }
    }
    report
}

#[derive(Debug, Clone)]
pub(crate) struct DisplayCategory {
    pub(crate) label: String,
    pub(crate) kind: ContextCategoryKind,
    pub(crate) tokens: u32,
    pub(crate) rank: u8,
}

pub(crate) fn display_categories(totals: &BTreeMap<String, u32>) -> Vec<DisplayCategory> {
    const INSTRUCTION_KINDS: [&str; 3] = [
        "system_instruction",
        "developer_instruction",
        "ability_instruction",
    ];

    let instruction_tokens = INSTRUCTION_KINDS.iter().fold(0u32, |sum, kind| {
        sum.saturating_add(totals.get(*kind).copied().unwrap_or_default())
    });
    let mut categories = vec![
        DisplayCategory {
            label: "system instructions".to_owned(),
            kind: ContextCategoryKind::System,
            tokens: instruction_tokens,
            rank: 0,
        },
        DisplayCategory {
            label: "tool schemas".to_owned(),
            kind: ContextCategoryKind::Tool,
            tokens: totals.get("tool_schema").copied().unwrap_or_default(),
            rank: 1,
        },
    ];
    categories.extend(
        totals
            .iter()
            .filter(|(kind, tokens)| {
                **tokens > 0
                    && !INSTRUCTION_KINDS.contains(&kind.as_str())
                    && kind.as_str() != "tool_schema"
            })
            .map(|(kind, tokens)| display_category(kind, *tokens)),
    );
    categories.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| left.label.cmp(&right.label))
    });
    categories
}

pub(crate) fn display_category(kind: &str, tokens: u32) -> DisplayCategory {
    use ContextCategoryKind::{History, Input, Other, Summary, System, Tool};
    let (label, kind, rank) = match kind {
        "system_instruction" => ("system instructions".to_owned(), System, 0),
        "developer_instruction" => ("developer instructions".to_owned(), System, 1),
        "ability_instruction" => ("ability instructions".to_owned(), System, 2),
        "tool_schema" => ("tool schemas".to_owned(), Tool, 3),
        "memory" => ("memory".to_owned(), History, 4),
        "history" => ("history".to_owned(), History, 5),
        "tool_result" => ("tool results".to_owned(), Tool, 6),
        "retrieval" => ("retrieved context".to_owned(), History, 7),
        "continuation" => ("continuation".to_owned(), Other, 8),
        "summary" => ("summary".to_owned(), Summary, 9),
        "user_input" => ("user input".to_owned(), Input, 10),
        other => (other.replace('_', " "), Other, u8::MAX - 1),
    };
    DisplayCategory {
        label,
        kind,
        tokens,
        rank,
    }
}

fn render_percent(tokens: u32, total: u32) -> String {
    if total == 0 {
        return "0.0%".to_owned();
    }
    let tenths = u64::from(tokens)
        .saturating_mul(1_000)
        .checked_div(u64::from(total))
        .unwrap_or(0);
    format!("{}.{:01}%", tenths / 10, tenths % 10)
}
