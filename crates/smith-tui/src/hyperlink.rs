//! OSC 8 annotations stay separate from visible glyphs until the buffer is drawn.

use ratatui::buffer::{Buffer, CellDiffOption, CellWidth};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

const OPEN: &str = "\x1b]8;;";
const END: &str = "\x1b\\";
const CLOSE: &str = "\x1b]8;;\x1b\\";

/// Controls in destinations could terminate OSC and inject terminal commands.
pub(crate) fn annotate(text: &str, target: &str) -> String {
    if target.chars().any(char::is_control) || target.is_empty() {
        return text.to_owned();
    }
    format!("{OPEN}{target}{END}{text}{CLOSE}")
}

/// Reads our annotations as text runs so wrapping never measures escape bytes.
pub(crate) fn runs(mut text: &str) -> Vec<(&str, Option<&str>)> {
    let mut result = Vec::new();
    let mut target = None;
    while !text.is_empty() {
        if let Some(tail) = text.strip_prefix(OPEN)
            && let Some(end) = tail.find(END)
        {
            target = (!tail[..end].is_empty()).then_some(&tail[..end]);
            text = &tail[end + END.len()..];
        } else {
            let end = text.find(OPEN).unwrap_or(text.len());
            // Malformed annotations are ordinary text, not an infinite loop.
            let end = if end == 0 { text.len() } else { end };
            result.push((&text[..end], target));
            text = &text[end..];
        }
    }
    result
}

/// Selection reads visible text, never terminal control payloads from cell symbols.
pub(crate) fn visible_text(text: &str) -> String {
    let mut visible = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            if !ch.is_control() {
                visible.push(ch);
            }
            continue;
        }
        match chars.next() {
            Some(']') => {
                while let Some(next) = chars.next() {
                    if next == '\x07' {
                        break;
                    }
                    if next == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some('[') => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    visible
}

pub(crate) fn plain_line(line: &Line<'static>) -> Line<'static> {
    let mut plain = line.clone();
    for span in &mut plain.spans {
        if span.content.contains('\x1b') {
            span.content = visible_text(&span.content).into();
        }
    }
    plain
}

pub(crate) fn line_width(line: &Line<'static>) -> usize {
    plain_line(line).width()
}

/// Normal rendering owns positions; annotations rewrite only the cells it covered.
///
/// Each glyph is a self-contained link so a partial diff cannot omit an opener or
/// leave a link active over subsequent text. Ratatui 0.30.2's ForcedWidth is the
/// supported replacement for the older width-1/skip workaround: its diff uses
/// CellWidth, including for CJK and combining glyphs.
pub(crate) fn apply(buffer: &mut Buffer, area: Rect, rows: &[Line<'static>]) {
    for (row, line) in rows.iter().take(usize::from(area.height)).enumerate() {
        let y = area.y + row as u16;
        let mut column = area.x;
        for span in &line.spans {
            for (text, target) in runs(&span.content) {
                let plain = Span::raw(text);
                for glyph in plain.styled_graphemes(ratatui::style::Style::default()) {
                    let width = glyph.symbol.cell_width();
                    let Some(forced_width) = std::num::NonZeroU16::new(width) else {
                        continue;
                    };
                    if column.saturating_add(width) > area.right() {
                        break;
                    }
                    if let Some(target) = target {
                        let cell = &mut buffer[(column, y)];
                        let symbol = annotate(cell.symbol(), target);
                        cell.set_symbol(&symbol)
                            .set_diff_option(CellDiffOption::ForcedWidth(forced_width));
                        // Hidden trailing cells must never overwrite a wide linked glyph.
                        for x in column + 1..column + width {
                            buffer[(x, y)].set_diff_option(CellDiffOption::Skip);
                        }
                    }
                    column += width;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::{Selection, text_from_buffer};
    use ratatui::style::Style;
    use ratatui::widgets::{Paragraph, Widget};

    #[test]
    fn copy_strips_osc_and_csi_and_snaps_wide_linked_glyphs() {
        let area = Rect::new(0, 0, 20, 1);
        let rows = vec![Line::from(vec![
            Span::raw("a"),
            Span::raw(annotate("中é docs", "https://example.com")),
            Span::raw(" z"),
        ])];
        let mut buffer = Buffer::empty(area);
        Paragraph::new(rows.iter().map(plain_line).collect::<Vec<_>>()).render(area, &mut buffer);
        apply(&mut buffer, area, &rows);
        let mut selection = Selection::begin(0, 0);
        selection.drag_to(19, 0);
        assert_eq!(
            text_from_buffer(&selection, &buffer, area).as_deref(),
            Some("a中é docs z")
        );
        let mut wide = Selection::begin(2, 0);
        wide.drag_to(3, 0);
        assert_eq!(
            text_from_buffer(&wide, &buffer, area).as_deref(),
            Some("中é")
        );
        assert_eq!(
            visible_text("\x1b[31mred\x1b[0m \x1b]8;;url\x07link\x1b]8;;\x07"),
            "red link"
        );
    }

    #[test]
    fn diffs_keep_positions_and_each_update_closes_its_link() {
        let area = Rect::new(0, 0, 12, 1);
        let buffer = |label: &str| {
            let rows = vec![Line::from(Span::raw(annotate(
                label,
                "https://example.com",
            )))];
            let mut buffer = Buffer::empty(area);
            Paragraph::new(rows.iter().map(plain_line).collect::<Vec<_>>())
                .render(area, &mut buffer);
            apply(&mut buffer, area, &rows);
            buffer
        };
        let before = buffer("a中 café");
        let after = buffer("a中 cafe");
        let diff = before.diff(&after);
        assert_eq!(
            diff.len(),
            1,
            "a partial diff must redraw only the changed glyph"
        );
        assert_eq!((diff[0].0, diff[0].1), (7, 0));
        assert!(diff[0].2.symbol().starts_with(OPEN));
        assert!(diff[0].2.symbol().ends_with(CLOSE));
        assert_eq!(diff[0].2.cell_width(), 1);
        let mut cleared = Buffer::empty(area);
        cleared.set_string(0, 0, "plain text", Style::default());
        assert!(
            after
                .diff(&cleared)
                .iter()
                .any(|(x, _, cell)| *x == 2 && cell.symbol() == "a")
        );
    }

    #[test]
    fn unsafe_destinations_cannot_emit_terminal_commands() {
        assert_eq!(annotate("label", "https://example.com\x1b]evil"), "label");
    }
}
