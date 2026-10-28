//! The questionnaire, drawn inline above the composer like `/model`'s list.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::questionnaire::{QuestionnaireState, REVIEW_ROWS};
use crate::theme::{Theme, Tone, glyph};

use super::helpers::wrap_text;
use super::modal::deadline_text;

/// Indent shared with the compact pickers.
const INDENT: &str = "  ";

/// Rows the pane wants at `width`; the layout may grant fewer.
pub(super) fn questionnaire_rows(state: &QuestionnaireState, width: u16) -> u16 {
    let (lines, _) = questionnaire_lines(state, width, Theme::new());
    u16::try_from(lines.len()).unwrap_or(u16::MAX)
}

pub(super) fn draw_questionnaire(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &QuestionnaireState,
    theme: Theme,
) {
    if area.is_empty() {
        return;
    }
    frame.render_widget(Clear, area);
    let (lines, focus) = questionnaire_lines(state, area.width, theme);
    let height = usize::from(area.height);
    let visible = if lines.len() <= height || height < 2 {
        lines.into_iter().take(height).collect::<Vec<_>>()
    } else {
        // Short pane: pin the first row (the tabs or the question) and
        // scroll the rest so the highlighted row stays visible.
        let rest = height - 1;
        let offset = (focus + 1).saturating_sub(rest).max(1);
        let mut lines = lines.into_iter();
        let first = lines.next();
        first
            .into_iter()
            .chain(lines.skip(offset - 1).take(rest))
            .collect()
    };
    frame.render_widget(Paragraph::new(visible), area);
}

/// Every pane line, already wrapped to `width`, and the index of the
/// highlighted row.
fn questionnaire_lines(
    state: &QuestionnaireState,
    width: u16,
    theme: Theme,
) -> (Vec<Line<'static>>, usize) {
    let form = state.form();
    let text_width = usize::from(width).saturating_sub(INDENT.len() * 2).max(1);
    let mut lines = Vec::new();
    let mut focus = 0;

    if state.has_review() {
        let mut tabs = vec![Span::styled(format!("{INDENT}← "), theme.style(Tone::Dim))];
        for (index, question) in form.questions.iter().enumerate() {
            let mark = if state.is_answered(index) {
                "■"
            } else {
                "□"
            };
            tabs.push(tab_span(
                format!("{mark} {}", question.header),
                index == state.current_index(),
                theme,
            ));
            tabs.push(Span::raw(" "));
        }
        tabs.push(tab_span(
            format!("{} Submit", glyph::GRAMMAR[5]),
            state.is_review(),
            theme,
        ));
        tabs.push(Span::styled(" →", theme.style(Tone::Dim)));
        lines.push(Line::from(tabs));
    }

    if state.is_review() {
        lines.push(heading("Review your answers", theme));
        for (index, question) in form.questions.iter().enumerate() {
            for (row, text) in wrap_text(&question.prompt, text_width.saturating_sub(2))
                .into_iter()
                .enumerate()
            {
                let lead = if row == 0 { glyph::BULLET } else { " " };
                lines.push(Line::from(vec![
                    Span::styled(format!("{INDENT}{lead} "), theme.style(Tone::Dim)),
                    Span::raw(text),
                ]));
            }
            lines.push(Line::from(match state.answer_text(index) {
                Some(answer) => {
                    Span::styled(format!("{INDENT}  → {answer}"), theme.style(Tone::Success))
                }
                None => Span::styled(
                    format!("{INDENT}  → not answered"),
                    theme.style(Tone::Warning),
                ),
            }));
        }
        for (index, label) in REVIEW_ROWS.iter().enumerate() {
            let selected = state.cursor() == index;
            if selected {
                focus = lines.len();
            }
            lines.push(choice_row(index, label, selected, false, theme));
        }
    } else {
        let question = state.question();
        let mut prompt = wrap_text(&question.prompt, text_width).into_iter();
        if !state.has_review() {
            // A lone question carries its header on the prompt's first row.
            let first = prompt.next().unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{INDENT}{} · ", question.header),
                    theme.style(Tone::Accent),
                ),
                Span::styled(first, theme.style(Tone::Heading)),
            ]));
        }
        lines.extend(prompt.map(|text| heading(&text, theme)));
        if state.is_discussing() {
            lines.push(heading("Chat about this question", theme));
            let draft = state.displayed_draft();
            let text = if draft.is_empty() {
                "Type your message…"
            } else {
                &draft
            };
            for text in wrap_text(text, text_width) {
                lines.push(Line::from(Span::styled(
                    format!("{INDENT}{text}"),
                    theme.style(Tone::Accent),
                )));
            }
            focus = lines.len();
            lines.push(Line::from(Span::styled(
                format!("{INDENT}enter send message · esc back"),
                theme.style(Tone::Dim),
            )));
        } else {
            for (index, choice) in question.choices.iter().enumerate() {
                let selected = state.cursor() == index;
                if selected {
                    focus = lines.len();
                }
                let staged = state.staged_choice() == Some(choice.id.as_str());
                lines.push(choice_row(index, &choice.label, selected, staged, theme));
                if let Some(description) = &choice.description {
                    lines.extend(
                        wrap_text(description, text_width.saturating_sub(5))
                            .into_iter()
                            .map(|text| {
                                Line::from(Span::styled(
                                    format!("{INDENT}     {text}"),
                                    theme.style(Tone::Dim),
                                ))
                            }),
                    );
                }
            }
            if question.allows_free_form {
                let selected = state.on_free_form_row();
                if selected {
                    focus = lines.len();
                }
                let draft = state.displayed_draft();
                let mut row = choice_row(question.choices.len(), "", selected, false, theme);
                row.spans.pop();
                if draft.is_empty() {
                    row.spans.push(Span::styled(
                        "Other · Type an answer",
                        theme.style(if selected { Tone::Accent } else { Tone::Dim }),
                    ));
                } else {
                    row.spans.push(Span::styled(
                        draft,
                        theme.style(if selected {
                            Tone::Accent
                        } else {
                            Tone::Default
                        }),
                    ));
                    if question.sensitive {
                        row.spans
                            .push(Span::styled(" (masked)", theme.style(Tone::Dim)));
                    }
                }
                if selected {
                    row.spans.push(Span::styled("▏", theme.style(Tone::Accent)));
                }
                lines.push(row);
            }
            let index = question.choices.len() + usize::from(question.allows_free_form);
            if state.on_discussion_row() {
                focus = lines.len();
            }
            lines.push(choice_row(
                index,
                "Chat about this question",
                state.on_discussion_row(),
                false,
                theme,
            ));
        }
    }

    let mut notes = Vec::new();
    if form.restored {
        notes.push(("restored pending question".to_owned(), Tone::Warning));
    }
    if form.deadline.instant().is_some() {
        notes.push((
            format!("deadline {}", deadline_text(form.deadline)),
            Tone::Dim,
        ));
    }
    if let Some(error) = state.error() {
        notes.push((format!("{} {error}", glyph::ERROR), Tone::Danger));
    }
    for (text, tone) in notes {
        lines.extend(
            wrap_text(&text, text_width)
                .into_iter()
                .map(|text| Line::from(Span::styled(format!("{INDENT}{text}"), theme.style(tone)))),
        );
    }
    // Wrap line by line so the highlighted row's index survives wrapping.
    let mut wrapped = Vec::with_capacity(lines.len());
    let mut wrapped_focus = 0;
    for (index, line) in lines.into_iter().enumerate() {
        if index == focus {
            wrapped_focus = wrapped.len();
        }
        wrapped.extend(super::wrap::wrap_lines(&[line], width));
    }
    (wrapped, wrapped_focus)
}

fn heading(text: &str, theme: Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!("{INDENT}{text}"),
        theme.style(Tone::Heading),
    ))
}

fn tab_span(text: String, active: bool, theme: Theme) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        if active {
            theme
                .style(Tone::Accent)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            theme.style(Tone::Dim)
        },
    )
}

/// One numbered list row: `❯ 1. ✓ Label`. The label is the last span so the
/// free-form row can replace it.
pub(super) fn choice_row(
    index: usize,
    label: &str,
    selected: bool,
    staged: bool,
    theme: Theme,
) -> Line<'static> {
    let tone = if selected {
        Tone::Accent
    } else {
        Tone::Default
    };
    let mut spans = vec![
        Span::styled(
            if selected { "❯ " } else { "  " },
            theme.style(Tone::Accent),
        ),
        Span::styled(format!("{}. ", index + 1), theme.style(tone)),
    ];
    if staged {
        spans.push(Span::styled(
            format!("{} ", glyph::GRAMMAR[5]),
            theme.style(Tone::Success),
        ));
    }
    spans.push(Span::styled(label.to_owned(), theme.style(tone)));
    Line::from(spans)
}
