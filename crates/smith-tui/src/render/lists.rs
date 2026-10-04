//! Fixed-height list rows shared by commands and resource pickers.

use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::{Theme, Tone, glyph};

/// Descriptions yield whole words so clipping never changes a word's meaning.
pub(crate) fn clip_words(text: &str, budget: usize) -> String {
    if budget == 0 {
        return String::new();
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.width() <= budget {
        return text;
    }
    let mut kept = String::new();
    for word in text.split_whitespace() {
        let separator = usize::from(!kept.is_empty());
        if kept.width() + separator + word.width() + 1 > budget {
            break;
        }
        if separator > 0 {
            kept.push(' ');
        }
        kept.push_str(word);
    }
    kept.push_str(glyph::ELIDED);
    kept
}

/// Keeps identity prefixes readable without splitting selected IDs across rows.
pub(crate) fn clip_name(name: &str, budget: usize) -> String {
    if name.width() <= budget {
        return name.to_owned();
    }
    if name.chars().any(char::is_whitespace) {
        return clip_words(name, budget);
    }
    if budget == 0 {
        return String::new();
    }
    let mut kept = String::new();
    let mut used = 0;
    for character in name.chars() {
        let cells = character.width().unwrap_or(0);
        if used + cells + 1 > budget {
            break;
        }
        kept.push(character);
        used += cells;
    }
    kept.push_str(glyph::ELIDED);
    kept
}

pub(crate) fn description_column(name_width: usize) -> usize {
    2 + name_width + 2
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn list_row(
    name: &str,
    description: &str,
    state: &str,
    selected: bool,
    name_width: usize,
    width: u16,
    tone: Tone,
    theme: Theme,
) -> Line<'static> {
    let width = usize::from(width);
    let name = clip_name(name, name_width);
    let column = description_column(name_width);
    let reserved = if state.is_empty() {
        0
    } else {
        state.width() + 2
    };
    let description = clip_words(description, width.saturating_sub(column + reserved));
    let mut spans = vec![
        Span::styled(
            if selected { "❯ " } else { "  " },
            theme.style(if selected { Tone::Accent } else { Tone::Dim }),
        ),
        Span::styled(name.clone(), theme.style(tone)),
        Span::styled(
            " ".repeat(name_width.saturating_sub(name.width()) + 2),
            theme.style(Tone::Dim),
        ),
        Span::styled(description.clone(), theme.style(Tone::Dim)),
    ];
    if !state.is_empty() {
        spans.push(Span::raw(" ".repeat(
            width.saturating_sub(column + description.width() + state.width()),
        )));
        spans.push(Span::styled(state.to_owned(), theme.style(tone)));
    }
    Line::from(spans)
}

pub(crate) fn detail_line(
    detail: &str,
    name_width: usize,
    width: u16,
    theme: Theme,
) -> Line<'static> {
    let column = description_column(name_width).min(usize::from(width));
    Line::from(vec![
        Span::raw(" ".repeat(column)),
        Span::styled(
            clip_words(detail, usize::from(width).saturating_sub(column)),
            theme.style(Tone::Dim),
        ),
    ])
}
