//! What Smith spends before the user has said anything.
//!
//! Every request carries the unconditional instruction sections and the full
//! tool schemas, so their size is a per-request tax on every session for the
//! life of the release. It is also invisible: nothing fails when a section
//! grows, and the cost only shows up as a slightly larger bill spread across
//! every user.
//!
//! This is the one place that number is written down. The ceilings are authored
//! constants, not measurements — raising one is the intended way to accept
//! growth, and doing it in a diff is the point.
//!
//! For scale, LangChain's Deep Agents cut its own base harness from roughly
//! 6k to 2k tokens in v0.7 with no measured eval regression, which is the
//! evidence that a leaner harness is not a worse one.

use agent_runtime::context::{CharRatioSizer, ContextFragment, RequestSizer};
use agent_runtime_core::provider::ToolSchema;
use smith_runtime::prompt::{DynamicPromptContext, stable_fragments};

/// The unconditional instruction sections, in tokens.
///
/// These are sent on every request of every session regardless of posture,
/// surface, or workspace.
const INSTRUCTION_CEILING: u32 = 700;

/// The default coding tool schemas, in tokens.
///
/// Names, descriptions, and complete input schemas for `read`, `list`,
/// `search`, `edit`, and `shell`.
const TOOL_CEILING: u32 = 950;

/// Instructions plus tools: the floor of any Smith request.
const BASE_CEILING: u32 = 1_600;

fn sizer() -> CharRatioSizer {
    CharRatioSizer::default()
}

fn instruction_sizes() -> Vec<(String, u32)> {
    let sizer = sizer();
    stable_fragments()
        .iter()
        .map(|fragment: &ContextFragment| (fragment.id.to_string(), sizer.size_fragment(fragment)))
        .collect()
}

fn tool_sizes() -> Vec<(String, u32)> {
    let sizer = sizer();
    smith_tools::all()
        .iter()
        .map(|tool| {
            let spec = tool.spec();
            let schema = ToolSchema {
                name: spec.name.clone(),
                description: spec.description.clone(),
                input_schema: spec.input_schema.clone(),
            };
            (spec.name, sizer.size_tool_schema(&schema))
        })
        .collect()
}

fn report(label: &str, sizes: &[(String, u32)], total: u32, ceiling: u32) -> String {
    let mut lines = vec![format!(
        "{label} is {total} tokens against a ceiling of {ceiling}:"
    )];
    let mut sorted = sizes.to_vec();
    sorted.sort_by_key(|(_, tokens)| std::cmp::Reverse(*tokens));
    for (name, tokens) in sorted {
        lines.push(format!("  {tokens:>5}  {name}"));
    }
    lines.push(
        "Trim a section, or raise the ceiling in this file if the growth is intended.".to_owned(),
    );
    lines.join("\n")
}

#[test]
fn the_unconditional_instruction_prefix_stays_within_budget() {
    let sizes = instruction_sizes();
    let total: u32 = sizes.iter().map(|(_, tokens)| tokens).sum();
    assert!(
        total <= INSTRUCTION_CEILING,
        "{}",
        report("the instruction prefix", &sizes, total, INSTRUCTION_CEILING)
    );
}

#[test]
fn the_default_tool_schemas_stay_within_budget() {
    let sizes = tool_sizes();
    let total: u32 = sizes.iter().map(|(_, tokens)| tokens).sum();
    assert!(
        total <= TOOL_CEILING,
        "{}",
        report("the default tool schemas", &sizes, total, TOOL_CEILING)
    );
}

#[test]
fn the_whole_base_harness_stays_within_budget() {
    let mut sizes = instruction_sizes();
    sizes.extend(tool_sizes());
    let total: u32 = sizes.iter().map(|(_, tokens)| tokens).sum();
    assert!(
        total <= BASE_CEILING,
        "{}",
        report("the base harness", &sizes, total, BASE_CEILING)
    );
}

#[test]
fn a_capability_gated_section_costs_nothing_when_its_tool_is_absent() {
    // The gating is what keeps the base number honest: if the conditional
    // sections were still unconditional, this delta would be zero and the
    // ceilings above would have to absorb them.
    let sizer = sizer();
    let size = |context: &DynamicPromptContext| -> u32 {
        smith_runtime::prompt::fragments(context)
            .iter()
            .map(|fragment| sizer.size_fragment(fragment))
            .sum()
    };

    let bare = size(&DynamicPromptContext::default());
    let full = size(&DynamicPromptContext {
        todo_planning: true,
        questionnaire: true,
        delegation: true,
        ..DynamicPromptContext::default()
    });

    assert!(
        full > bare,
        "gating the capability sections saved nothing: {bare} vs {full}"
    );
    assert!(
        bare <= INSTRUCTION_CEILING,
        "a run with no optional capabilities must still fit the prefix budget"
    );
}
