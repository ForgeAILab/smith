//! Aligned inline report rows; other transcript text keeps its own wrapping.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::helpers::clip_line;
use super::wrap::wrap_lines;
use crate::theme::glyph;

pub(super) fn label_width<'a>(labels: impl Iterator<Item = &'a str>, width: u16) -> usize {
    // Leave at least half the pane for values at narrow widths. Long labels
    // can use another row without squeezing ordinary words out of the value.
    labels
        .map(|label| label.width())
        .max()
        .unwrap_or(0)
        .min((usize::from(width) / 2).saturating_sub(4))
        .max(1)
}

pub(super) fn field(
    label: Line<'static>,
    value: &str,
    value_style: Style,
    label_width: usize,
    width: u16,
) -> Vec<Line<'static>> {
    let value_width = usize::from(width).saturating_sub(label_width + 4).max(1);
    let labels = label_rows(label, label_width);
    let values = text_rows(value, value_width, value_style);
    (0..labels.len().max(values.len()))
        .map(|index| {
            let mut label = labels.get(index).cloned().unwrap_or_default();
            let padding = label_width.saturating_sub(label.width()) + 2;
            let mut spans = vec![Span::raw("  ")];
            spans.append(&mut label.spans);
            spans.push(Span::raw(" ".repeat(padding)));
            if let Some(value) = values.get(index) {
                spans.extend(value.spans.iter().cloned());
            }
            Line::from(spans)
        })
        .collect()
}

pub(super) fn text(content: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    text_rows(content, usize::from(width).saturating_sub(2).max(1), style)
        .into_iter()
        .map(|mut line| {
            line.spans.insert(0, Span::raw("  "));
            line
        })
        .collect()
}

fn text_rows(content: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    content
        .split('\n')
        .flat_map(|raw| {
            word_rows(
                Line::from(Span::styled(raw.trim_end_matches('\r').to_owned(), style)),
                width,
            )
        })
        .collect()
}

fn label_rows(mut line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if line.width() <= width {
        return word_rows(line, width);
    }
    let mut indentation = String::new();
    for span in &mut line.spans {
        let content = span.content.as_ref();
        let trimmed = content.trim_start_matches(char::is_whitespace);
        indentation.push_str(&content[..content.len() - trimmed.len()]);
        let has_text = !trimmed.is_empty();
        span.content = trimmed.to_owned().into();
        if has_text {
            break;
        }
    }
    let indent_width = indentation.width().min(width.saturating_sub(1));
    let mut rows = word_rows(line, width.saturating_sub(indent_width).max(1));
    if indent_width > 0 {
        // A nested label keeps its indentation on every continuation row.
        for row in &mut rows {
            row.spans.insert(0, Span::raw(" ".repeat(indent_width)));
        }
    }
    rows
}

fn word_rows(mut line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    // Identities and other unbroken tokens may exceed a whole column. Mark
    // that omission explicitly instead of splitting a token between rows.
    for span in &mut line.spans {
        span.content = span
            .content
            .split_inclusive(char::is_whitespace)
            .map(|part| {
                let word = part.trim_end_matches(char::is_whitespace);
                format!(
                    "{}{}",
                    clip_line(word.to_owned(), width),
                    &part[word.len()..]
                )
            })
            .collect::<String>()
            .into();
    }
    wrap_lines(&[line], u16::try_from(width).unwrap_or(u16::MAX))
}

pub(super) fn left_shorten(path: &str, width: usize) -> String {
    if path.width() <= width {
        return path.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut used = 1;
    let mut suffix = Vec::new();
    for character in path.chars().rev() {
        let cells = character.width().unwrap_or(0);
        if used + cells > width {
            break;
        }
        suffix.push(character);
        used += cells;
    }
    format!(
        "{}{}",
        glyph::ELIDED,
        suffix.into_iter().rev().collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    #[test]
    fn wrapped_field_labels_keep_their_leading_indentation() {
        let style = Style::default().add_modifier(Modifier::DIM);
        let rows = field(
            Line::from(vec![
                Span::raw(" "),
                Span::styled(" system instruction", style),
            ]),
            "value",
            Style::default(),
            18,
            44,
        );
        let labels = rows
            .iter()
            .map(|row| {
                row.to_string()
                    .chars()
                    .take(22)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(labels, ["    system", "    instruction"]);
        assert!(rows.iter().all(|row| row.width() <= 44));
        for (row, word) in rows.iter().zip(["system", "instruction"]) {
            assert!(
                row.spans
                    .iter()
                    .any(|span| span.content == word && span.style == style)
            );
        }
    }
}
