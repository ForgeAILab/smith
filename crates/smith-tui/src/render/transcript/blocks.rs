use std::borrow::Cow;

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use smith_client::NoticeKind;
use smith_tools::{ToolCallDisplay, tool_display_label};

use crate::status::render_elapsed;
use crate::theme::{Theme, Tone, glyph};
use crate::transcript::{Block, LocalResult, ToolStatus};

use super::super::helpers::hanging_lines;
use super::super::markdown::render_assistant_lines;
use super::agents::render_agent_report;
use super::changes::{render_diff_report, render_recovery_report, render_review_report};
use super::integrations::{render_mcp_report, render_skills_report};
use super::local_reports::{
    render_diagnostics_report, render_goal_report, render_message_report, render_shell_report,
    render_timeline_report,
};
use super::text::report_title;
use super::{render_context_report, render_help_report, render_status_card, safe_tool_name};

pub(super) fn conversation_lines(
    blocks: &[Block],
    speculative: Option<&str>,
    theme: Theme,
    width: u16,
    expanded: bool,
) -> Vec<Line<'static>> {
    let Some(text) = speculative else {
        return block_lines(blocks, theme, width, expanded);
    };
    // A commit can extend the current assistant block. Preview that same
    // boundary without promoting an attempt into the canonical transcript.
    let (preceding, text) = if let Some(Block::Assistant {
        text: body,
        open: true,
    }) = blocks.last()
    {
        (
            &blocks[..blocks.len() - 1],
            Cow::Owned(format!("{body}{text}")),
        )
    } else {
        (blocks, Cow::Borrowed(text))
    };
    let mut lines = block_lines(preceding, theme, width, expanded);
    if !lines.is_empty() {
        lines.push(Line::default());
    }
    lines.extend(render_assistant_lines(&text, theme, width, true));
    lines
}

pub(super) fn suppressed_block(block: &Block, expanded: bool) -> bool {
    // Reasoning is canonical model state, not a second assistant answer.
    // The anchored working row represents progress without
    // exposing raw provider reasoning as transcript prose.
    if matches!(block, Block::Reasoning { .. }) {
        return true;
    }
    if !expanded
        && matches!(
            block,
            Block::Notice {
                kind: NoticeKind::Capabilities,
                ..
            }
        )
    {
        return true;
    }
    // A row whose effect a better surface already reports is dropped
    // whole, before the blank-line separator below ever considers it —
    // that is what keeps a suppression from leaving a doubled or
    // leading blank line, and it drops the result preview for free,
    // since the preview lives inside this same block.
    if let Block::Tool {
        name,
        display,
        status,
        ..
    } = block
        && is_redundant_tool_row(name, *status, display.as_deref())
    {
        return true;
    }
    false
}

/// Every transcript block as rendered rows.
///
/// The root timeline and a delegated child's history both come through
/// here. A child is an agent that reports back, not a different kind of
/// thing, so it must not get a second, thinner renderer that drifts from
/// this one — whatever the runtime chooses to report about it lands in the
/// same blocks and draws the same way.
pub(super) fn block_lines(
    blocks: &[Block],
    theme: Theme,
    width: u16,
    expanded: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for block in blocks {
        if suppressed_block(block, expanded) {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        let start = lines.len();
        match block {
            Block::User { text } => {
                for (index, raw) in text.lines().enumerate() {
                    let marker = if index == 0 { glyph::USER } else { " " };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{marker} "),
                            theme.style(Tone::Dim).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(raw.to_owned(), theme.style(Tone::Default)),
                    ]));
                }
            }
            Block::Assistant { text, open } => {
                lines.extend(render_assistant_lines(text, theme, width, *open));
            }
            Block::Reasoning { .. } => {}
            Block::Tool {
                name,
                display,
                protected_summary,
                status,
                result_preview,
                user_command,
                started_at,
                enrichment,
                ..
            } => {
                let tone = match status {
                    ToolStatus::WaitingForApproval
                    | ToolStatus::Running
                    | ToolStatus::Unreported => Tone::Dim,
                    ToolStatus::Ok => Tone::Success,
                    ToolStatus::Failed | ToolStatus::Denied => Tone::Danger,
                };
                let mut call = if let Some(command) = user_command {
                    vec![Span::styled(
                        format!("! {command}"),
                        theme.style(Tone::Default),
                    )]
                } else {
                    vec![
                        Span::styled(
                            format!("{} ", glyph::TOOL),
                            theme.style(tone).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            tool_invocation(
                                name,
                                display.as_deref(),
                                enrichment,
                                protected_summary,
                            ),
                            theme.style(Tone::Heading),
                        ),
                    ]
                };
                if !matches!(status, ToolStatus::Ok) {
                    let status_text = match status {
                        ToolStatus::Running => started_at
                            .map(|started| format!("running {}", render_elapsed(started.elapsed())))
                            .unwrap_or_else(|| status.label().to_owned()),
                        _ => status.label().to_owned(),
                    };
                    call.push(Span::styled(format!(" {status_text}"), theme.style(tone)));
                }
                lines.push(Line::from(call));
                if !matches!(status, ToolStatus::Running | ToolStatus::WaitingForApproval) {
                    if let Some(preview) = result_preview {
                        let summary =
                            (status == &ToolStatus::Ok && !expanded && user_command.is_none())
                                .then(|| {
                                    display
                                        .as_ref()
                                        .and_then(|display| display.result_summary(preview))
                                })
                                .flatten();
                        lines.extend(nested_result_lines(
                            summary.as_deref().unwrap_or(preview),
                            expanded,
                            width,
                            theme,
                        ));
                    } else if matches!(status, ToolStatus::Ok) {
                        lines.extend(nested_result_lines("Completed", expanded, width, theme));
                    }
                }
            }

            Block::Error { message } => {
                for (index, raw) in message.lines().enumerate() {
                    let marker = if index == 0 { glyph::ERROR } else { " " };
                    lines.push(Line::from(Span::styled(
                        format!("{marker} {raw}"),
                        theme.style(Tone::Danger),
                    )));
                }
            }
            // Turn boundaries read as quiet punctuation — "Worked for 5s" —
            // not as a sourced notice row.
            Block::Notice { kind: source, text } if source.label() == "turn" => {
                for raw in text.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  {raw}"),
                        theme.style(Tone::Dim),
                    )));
                }
            }
            Block::Notice { kind: source, text } => {
                for (index, raw) in text.lines().enumerate() {
                    if index == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(format!("{} ", glyph::NOTICE), theme.style(Tone::Dim)),
                            Span::styled(source.label().into_owned(), theme.style(Tone::Heading)),
                            Span::styled(" · ", theme.style(Tone::Dim)),
                            Span::styled(raw.to_owned(), theme.style(Tone::Default)),
                        ]));
                    } else {
                        lines.push(Line::from(Span::styled(
                            format!("  {raw}"),
                            theme.style(Tone::Dim),
                        )));
                    }
                }
            }
            Block::Local(LocalResult::Status(report)) => {
                lines.push(report_title("/status", theme));
                lines.extend(render_status_card(report, width, theme));
            }
            Block::Local(LocalResult::Diagnostics(report)) => {
                lines.push(report_title("/diagnostics", theme));
                lines.extend(render_diagnostics_report(report, width, theme));
            }
            Block::Local(LocalResult::Context(report)) => {
                lines.push(report_title("/context", theme));
                lines.extend(render_context_report(report, width, theme));
            }
            Block::Local(LocalResult::Help(report)) => {
                lines.push(report_title("/help", theme));
                lines.extend(render_help_report(report, width, theme));
            }
            Block::Local(LocalResult::Timeline(report)) => {
                lines.push(Line::from(Span::styled(
                    "/timeline",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_timeline_report(report, width, theme));
            }
            Block::Local(LocalResult::Goal(report)) => {
                lines.push(Line::from(Span::styled(
                    "/goal",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_goal_report(report, width, theme));
            }
            Block::Local(LocalResult::Agent(report)) => {
                lines.extend(render_agent_report(report, width, theme));
            }
            Block::Local(LocalResult::Mcp(report)) => {
                lines.push(Line::from(Span::styled("/mcp", theme.style(Tone::Command))));
                lines.extend(render_mcp_report(report, width, theme));
            }
            Block::Local(LocalResult::Modules(report)) => {
                lines.push(report_title("/modules", theme));
                lines.extend(super::local_reports::render_modules_report(
                    report, width, theme,
                ));
            }
            Block::Local(LocalResult::Skills(report)) => {
                lines.push(Line::from(Span::styled(
                    "/skills",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_skills_report(report, width, theme));
            }
            Block::Local(LocalResult::Diff(report)) => {
                lines.push(Line::from(Span::styled(
                    format!("/{}", report.title),
                    theme.style(Tone::Command),
                )));
                lines.extend(render_diff_report(report, width, theme));
            }
            Block::Local(LocalResult::Review(report)) => {
                lines.extend(render_review_report(report, width, theme));
            }
            Block::Local(LocalResult::Recovery(report)) => {
                lines.extend(render_recovery_report(report, width, theme));
            }
            Block::Local(LocalResult::Shell(report)) => {
                lines.push(Line::from(Span::styled(
                    "/shell",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_shell_report(report, width, theme));
            }
            Block::Local(LocalResult::Message(report)) => {
                lines.push(Line::from(Span::styled(
                    format!("/{}", report.title()),
                    theme.style(Tone::Command),
                )));
                lines.extend(render_message_report(report, width, theme));
            }
        }
        if matches!(block, Block::User { .. } | Block::Notice { .. }) {
            let rendered = lines
                .drain(start..)
                .flat_map(|line| hanging_lines(line, width, 2))
                .collect::<Vec<_>>();
            lines.extend(rendered);
        } else if matches!(block, Block::Tool { .. }) {
            let call = lines.remove(start);
            let wrapped = hanging_lines(call, width, 2);
            drop(lines.splice(start..start, wrapped));
        }
    }
    lines
}

fn nested_result_lines(
    output: &str,
    expanded: bool,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    for (index, raw) in output.lines().enumerate() {
        let prefix = if index == 0 {
            format!("  {}  ", glyph::BRANCH)
        } else {
            "     ".to_owned()
        };
        rows.extend(hanging_lines(
            Line::from(vec![
                Span::styled(prefix, theme.style(Tone::Dim)),
                Span::styled(raw.to_owned(), theme.style(Tone::Dim)),
            ]),
            width,
            5,
        ));
    }
    if !expanded && rows.len() > 4 {
        let remaining = rows.len() - 4;
        rows.truncate(4);
        rows.extend(hanging_lines(
            Line::from(Span::styled(
                format!(
                    "     {} +{remaining} lines (ctrl+o to expand)",
                    glyph::ELIDED
                ),
                theme.style(Tone::Dim),
            )),
            width,
            5,
        ));
    }
    rows
}

/// Whether a successful tool call's row is already reported in full by a
/// named non-transcript surface, and can therefore be dropped.
///
/// The set is enumerated explicitly here, never inferred from the call's
/// name shape, argument count, or result size — see `tool-call-display`'s
/// "Reviewed redundant-row suppression". Only a successful call ever
/// qualifies: a failure, a denial, or a call whose outcome never arrived is
/// not redundant with anything, so those statuses always render. The
/// host-local calls that finish within a frame or two are also hidden while
/// running; otherwise their row is drawn only to vanish on completion, which
/// reads as a glitch. `agent`'s action is read from the
/// projector's own `target()` (`"spawn"`, `"wait"`, …) — a call with no
/// reviewed display cannot be matched against that vocabulary at all, so it
/// renders rather than being guessed at.
fn is_redundant_tool_row(
    name: &str,
    status: ToolStatus,
    display: Option<&ToolCallDisplay>,
) -> bool {
    if status == ToolStatus::Running {
        return matches!(name, "write_todos" | "registry.search");
    }
    if status != ToolStatus::Ok {
        return false;
    }
    match name {
        "write_todos" => true,
        "registry.search" => true,
        // Delegation's own lifecycle line reports `wait`, `result`,
        // `resume`, and `stop`. `spawn` is the one row that announces the
        // spawn, so it is never suppressed, and nothing else reports
        // `follow_up` or `list`, so they render too.
        "agent" => matches!(
            display.map(ToolCallDisplay::target),
            Some("wait" | "result" | "resume" | "stop")
        ),
        _ => false,
    }
}

/// The compact invocation portion of a tool row, with any host-confirmed
/// enrichment appended after the projector's own qualifiers.
///
/// Enrichment is kept in a field of its own on `Block::Tool` rather than
/// folded into `display`'s qualifiers, specifically so a later
/// re-projection at tool completion cannot silently drop it; this is where
/// the two are joined back together when the block's rendered rows are
/// refreshed.
fn tool_invocation(
    name: &str,
    display: Option<&ToolCallDisplay>,
    enrichment: &[String],
    protected_summary: &str,
) -> String {
    let Some(display) = display else {
        let label = tool_display_label(name)
            .map(str::to_owned)
            .unwrap_or_else(|| safe_tool_name(name));
        return format!("{label}({protected_summary})");
    };
    if enrichment.is_empty() {
        return display.invocation();
    }
    let mut details = Vec::with_capacity(display.qualifiers().len() + enrichment.len() + 1);
    details.push(display.target());
    details.extend(display.qualifiers().iter().map(String::as_str));
    details.extend(enrichment.iter().map(String::as_str));
    format!("{}({})", display.label(), details.join(" · "))
}
