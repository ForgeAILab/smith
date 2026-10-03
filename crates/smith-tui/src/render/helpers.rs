//! Small bounded text helpers shared by multiple visual regions.

use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::{Theme, Tone, glyph};

/// The tone one child's lifecycle label reads in, wherever it is shown.
///
/// The panel row and the inspector heading name the same state, so they read
/// it through the same function: a child cannot be green in one place and
/// grey in the other. Work that needs the user stands out, a clean finish
/// reads as success, and a settled outcome nobody has to act on recedes.
pub(super) fn child_state_tone(state: &str) -> Tone {
    match state {
        "failed" => Tone::Danger,
        "needs input" => Tone::Warning,
        "completed" | "idle" => Tone::Success,
        "running" | "resuming" => Tone::Default,
        _ => Tone::Dim,
    }
}

/// Clips text to a display-cell budget, ending with `…` when it was cut.
pub(super) fn clip_line(text: String, budget: usize) -> String {
    if text.width() <= budget {
        return text;
    }
    let mut kept = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let width = ch.width().unwrap_or(0);
        if used + width > budget.saturating_sub(1) {
            break;
        }
        used += width;
        kept.push(ch);
    }
    kept.push_str(glyph::ELIDED);
    kept
}

pub(super) fn push_segment(spans: &mut Vec<Span<'static>>, theme: Theme, segment: Span<'static>) {
    spans.push(Span::styled(
        format!(" {} ", glyph::SEPARATOR),
        theme.style(Tone::Dim),
    ));
    spans.push(segment);
}

pub(super) fn truncate(spans: Vec<Span<'static>>, width: u16) -> Vec<Span<'static>> {
    let limit = usize::from(width);
    let mut used = 0;
    let mut kept = Vec::new();
    for span in spans {
        let span_width = span.content.width();
        if used + span_width > limit {
            break;
        }
        used += span_width;
        kept.push(span);
    }
    if kept
        .last()
        .is_some_and(|span| span.content.as_ref() == " · ")
    {
        kept.pop();
    }
    kept
}

pub(super) fn wrap_text(raw: &str, available: usize) -> Vec<String> {
    if raw.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for character in raw.chars() {
        let character_width = character.width().unwrap_or(0);
        if used > 0 && used + character_width > available {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push(character);
        used += character_width;
    }
    lines.push(line);
    lines
}

/// Reflows a transcript line beneath its text rather than beneath its marker.
pub(super) fn hanging_lines(line: Line<'static>, width: u16, indent: usize) -> Vec<Line<'static>> {
    if line.width() <= usize::from(width) || usize::from(width) <= indent {
        return vec![line];
    }
    let mut prefix = Vec::new();
    let mut body = Vec::new();
    let mut remaining = indent;
    for span in line.spans {
        let mut split = 0;
        for (index, character) in span.content.char_indices() {
            let cells = character.width().unwrap_or(0);
            if cells > remaining || remaining == 0 {
                break;
            }
            remaining -= cells;
            split = index + character.len_utf8();
        }
        if split > 0 {
            prefix.push(Span::styled(span.content[..split].to_owned(), span.style));
        }
        if split < span.content.len() {
            body.push(Span::styled(span.content[split..].to_owned(), span.style));
        }
    }
    let rows = super::wrap::wrap_lines(&[Line::from(body)], width - indent as u16);
    rows.into_iter()
        .enumerate()
        .map(|(index, mut row)| {
            let mut spans = if index == 0 {
                prefix.clone()
            } else {
                vec![Span::raw(" ".repeat(indent))]
            };
            spans.append(&mut row.spans);
            Line::from(spans)
        })
        .collect()
}
