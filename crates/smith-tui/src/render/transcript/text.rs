use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{Theme, Tone, glyph};

use super::super::helpers::wrap_text;
use super::super::reports;

pub(in crate::render) fn render_inline_markdown(
    raw: &str,
    base: Style,
    theme: Theme,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = raw;

    while !rest.is_empty() {
        if let Some(link) = rest.strip_prefix('[')
            && let Some(label_end) = link.find("](")
            && let Some(target_end) = link[label_end + 2..].find(')')
        {
            spans.push(Span::styled(
                link[..label_end].to_owned(),
                base.patch(theme.style(Tone::Link)),
            ));
            rest = &link[label_end + 2 + target_end + 1..];
            continue;
        }
        if let Some(strong) = rest.strip_prefix("**")
            && let Some(end) = strong.find("**")
        {
            spans.push(Span::styled(
                strong[..end].to_owned(),
                base.add_modifier(Modifier::BOLD),
            ));
            rest = &strong[end + 2..];
            continue;
        }
        if let Some(code) = rest.strip_prefix('`')
            && let Some(end) = code.find('`')
        {
            spans.push(Span::styled(
                code[..end].to_owned(),
                base.patch(theme.style(Tone::Code)),
            ));
            rest = &code[end + 1..];
            continue;
        }
        if let Some(emphasis) = rest.strip_prefix('*')
            && let Some(end) = emphasis.find('*')
        {
            spans.push(Span::styled(
                emphasis[..end].to_owned(),
                base.add_modifier(Modifier::ITALIC),
            ));
            rest = &emphasis[end + 1..];
            continue;
        }

        let next = ["[", "**", "`", "*"]
            .into_iter()
            .filter_map(|delimiter| rest.find(delimiter))
            .filter(|index| *index > 0)
            .min()
            .unwrap_or(rest.len());
        if next == 0 {
            let first = rest.chars().next().expect("rest was checked as non-empty");
            spans.push(Span::styled(first.to_string(), base));
            rest = &rest[first.len_utf8()..];
        } else {
            spans.push(Span::styled(rest[..next].to_owned(), base));
            rest = &rest[next..];
        }
    }

    spans
}

pub(super) fn report_title(command: &str, theme: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{} ", glyph::BULLET), theme.style(Tone::Dim)),
        Span::styled(command.to_owned(), theme.style(Tone::Command)),
    ])
}

pub(super) fn report_field(
    label: &str,
    value: &str,
    tone: Tone,
    label_width: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    reports::field(
        Line::from(Span::styled(label.to_owned(), theme.style(Tone::Dim))),
        value,
        theme.style(tone),
        label_width,
        width,
    )
}

/// Wraps free text before applying inline Markdown.
pub(super) fn render_inline_text_lines(
    content: &str,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    content
        .lines()
        .flat_map(|raw| {
            wrap_text(raw, usize::from(width).max(1))
                .into_iter()
                .map(|wrapped| {
                    Line::from(render_inline_markdown(
                        &wrapped,
                        theme.style(Tone::Default),
                        theme,
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(super) fn context_field(label: &str, value: String, tone: Tone, theme: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}:"), theme.style(Tone::Dim)),
        Span::styled(format!(" {value}"), theme.style(tone)),
    ])
}

/// Keeps the existing character wrapping, carrying styles from typed fields.
/// Byte ranges only split spans; no text selects a presentation branch.
pub(super) fn wrap_context_line(line: Line<'static>, available: usize) -> Vec<Line<'static>> {
    let raw = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    if raw.is_empty() {
        return vec![Line::default()];
    }
    let mut rows = Vec::new();
    let mut line_start = 0;
    for raw_line in raw.split_inclusive('\n') {
        let content = raw_line
            .strip_suffix('\n')
            .map(|text| text.strip_suffix('\r').unwrap_or(text))
            .unwrap_or(raw_line);
        let mut start = line_start;
        for wrapped in wrap_text(content, available) {
            let end = start + wrapped.len();
            let mut offset = 0;
            let spans = line
                .spans
                .iter()
                .filter_map(|span| {
                    let span_start = offset;
                    offset += span.content.len();
                    let from = start.max(span_start);
                    let to = end.min(offset);
                    if from < to {
                        Some(Span::styled(
                            span.content[from - span_start..to - span_start].to_owned(),
                            span.style,
                        ))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            rows.push(Line::from(spans));
            start = end;
        }
        line_start += raw_line.len();
    }
    rows
}

pub(in crate::render) fn render_prefixed_local_state(
    marker: &str,
    content: &str,
    width: u16,
    style: Style,
) -> Vec<Line<'static>> {
    let available = usize::from(width.saturating_sub(2)).max(1);
    let mut lines = Vec::new();
    for raw in content.lines() {
        for (index, wrapped) in wrap_text(raw, available).into_iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(
                    if index == 0 {
                        format!("{marker} ")
                    } else {
                        "  ".to_owned()
                    },
                    style,
                ),
                Span::styled(wrapped, style),
            ]));
        }
    }
    lines
}
