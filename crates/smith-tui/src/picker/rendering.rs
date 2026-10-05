use super::*;

/// Shared lowercase controls keep every screen's navigation vocabulary consistent.
#[derive(Debug, Clone, Copy)]
pub enum ScreenFooter {
    /// The body guidance already supplies all relevant controls.
    None,
    /// Empty lists expose only actions that can recover or leave the screen.
    Empty {
        /// A nonempty inventory can recover by clearing its filter.
        filtered: bool,
        /// Whether the owner can resume an earlier step.
        back: bool,
    },
    /// Numbered choices advertise digits; inventories advertise arrow selection.
    List {
        /// Digit range advertised only for fixed choices.
        choices: Option<usize>,
        /// Whether the owner can resume an earlier step.
        back: bool,
    },
    /// Fields continue to another step without submitting the transaction.
    Field {
        /// Whether the owner can resume an earlier step.
        back: bool,
    },
    /// Review can include its existing wrapped-row scroll position.
    Review {
        /// Whether the owner can resume an earlier step.
        back: bool,
        /// Position and total of the review's wrapped-row viewport.
        scroll: Option<(usize, usize)>,
    },
    /// Login waits can return to their method list when owned by setup.
    Progress {
        /// Whether login is nested in setup rather than a standalone flow.
        back: bool,
    },
    /// Busy screens still expose whole-flow cancellation to the host.
    Busy {
        /// Whether there is an earlier editable step.
        back: bool,
    },
}

impl ScreenFooter {
    fn segments(self) -> Vec<String> {
        let escape = |back| if back { "esc back" } else { "esc cancel" }.to_owned();
        match self {
            Self::None => Vec::new(),
            Self::Empty { filtered, back } => {
                let mut segments = Vec::new();
                if filtered {
                    segments.push("ctrl+u clear filter".into());
                }
                segments.push(escape(back));
                segments
            }
            Self::List { choices, back } => vec![
                choices.filter(|count| *count > 0).map_or_else(
                    || "↑↓ choose".to_owned(),
                    |count| format!("↑↓ or 1–{count} choose"),
                ),
                "enter confirm".into(),
                escape(back),
            ],
            Self::Field { back } => vec!["enter continue".into(), escape(back)],
            Self::Review { back, scroll } => {
                let mut segments = vec!["enter confirm".into(), escape(back)];
                if let Some((position, total)) = scroll {
                    segments.push(format!("↑↓/PgUp/PgDn review · {position}/{total}"));
                }
                segments
            }
            Self::Progress { back: true } => vec![escape(true), "ctrl+c cancel".into()],
            Self::Progress { back: false } => vec![escape(false)],
            Self::Busy { back } => vec![escape(back), "ctrl+c cancel".into()],
        }
    }

    /// Fits controls as whole hints, prioritizing Enter and Escape when space is tight.
    pub fn rows(self, width: u16) -> Vec<String> {
        let mut segments = self.segments();
        let available = usize::from(width.saturating_sub(2));
        if matches!(self, Self::List { .. })
            && (width < 60 || segments.join(" · ").width() > available)
        {
            segments.rotate_left(1);
        }
        let mut rows = Vec::new();
        let mut row = String::new();
        for segment in segments {
            if !row.is_empty() && row.width() + 3 + segment.width() > available {
                rows.push(row);
                row = String::new();
            }
            if !row.is_empty() {
                row.push_str(" · ");
            }
            row.push_str(&segment);
        }
        if !row.is_empty() {
            rows.push(row);
        }
        rows
    }

    /// Session hosts have one hint row, so optional selection controls yield first.
    pub(crate) fn hint(self, width: u16) -> String {
        self.rows(width).into_iter().next().unwrap_or_default()
    }
}

/// Reserves the title, a blank row, and a content-sized footer before body rendering.
///
/// The body reaches the bottom only when it overflows; short screens have no
/// filler between their content and controls. Hosts can scroll the returned area.
pub fn draw_inline_surface(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    content_rows: usize,
    footer: ScreenFooter,
    theme: Theme,
) -> Rect {
    draw_inline_screen(frame, area, title, content_rows, Some(footer), theme)
}

/// Session hosts omit the local footer because controls live below their composer.
pub fn draw_inline_screen(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    content_rows: usize,
    footer: Option<ScreenFooter>,
    theme: Theme,
) -> Rect {
    frame.render_widget(Clear, area);
    let footer_lines = footer
        .map(|footer| footer.rows(area.width))
        .unwrap_or_default();
    let footer_rows = u16::try_from(footer_lines.len())
        .unwrap_or(u16::MAX)
        .min(area.height.saturating_sub(1));
    let height = u16::try_from(content_rows).unwrap_or(u16::MAX).min(
        area.height
            .saturating_sub(if footer.is_some() { 3 + footer_rows } else { 2 }),
    );
    let body = Rect::new(area.x, area.y.saturating_add(2), area.width, height);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {title}"),
            theme.style(Tone::Heading),
        ))),
        Rect::new(area.x, area.y, area.width, area.height.min(1)),
    );
    let footer_y = body
        .bottom()
        .saturating_add(1)
        .min(area.bottom().saturating_sub(footer_rows));
    frame.render_widget(
        Paragraph::new(
            footer_lines
                .into_iter()
                .map(|line| Line::from(format!("  {line}")))
                .collect::<Vec<_>>(),
        )
        .style(theme.style(Tone::Dim)),
        Rect::new(area.x, footer_y, area.width, footer_rows),
    );
    body
}

/// Draws the same unframed list before a session exists.
pub fn draw_resource_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &ResourcePicker,
    theme: Theme,
) {
    super::draw_picker_with_context(frame, area, picker, &picker.title, None, None, theme);
}

/// Setup adds its existing note and validation text without owning a second list renderer.
pub(crate) fn draw_picker_with_context(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &ResourcePicker,
    title: &str,
    note: Option<&str>,
    error: Option<&str>,
    theme: Theme,
) {
    draw_picker_context(
        frame,
        area,
        picker,
        PickerContext {
            title,
            note,
            error,
            embedded: false,
        },
        theme,
    );
}

/// Measure with the same wrapped rows as drawing, so inline screens need no filler.
pub(crate) fn picker_content_height(
    picker: &ResourcePicker,
    width: u16,
    note: Option<&str>,
    error: Option<&str>,
) -> u16 {
    let theme = Theme::new();
    let rows = entry_view_capped(
        picker,
        usize::MAX,
        width,
        theme,
        false,
        COMPACT_VISIBLE_ENTRIES,
    )
    .lines
    .len()
        + note
            .map(|text| indented_words(text, width, 2, Tone::Dim, theme).len())
            .unwrap_or(0)
        + error
            .map(|text| {
                indented_words(&format!("error: {text}"), width, 2, Tone::Danger, theme).len()
            })
            .unwrap_or(0);
    u16::try_from(
        rows.saturating_add(2)
            .saturating_sub(usize::from(note.is_some())),
    )
    .unwrap_or(u16::MAX)
}

pub(crate) struct PickerContext<'a> {
    pub(crate) title: &'a str,
    pub(crate) note: Option<&'a str>,
    pub(crate) error: Option<&'a str>,
    pub(crate) embedded: bool,
}

pub(crate) fn draw_picker_context(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &ResourcePicker,
    context: PickerContext<'_>,
    theme: Theme,
) {
    let PickerContext {
        title,
        note,
        error,
        embedded,
    } = context;
    let mut prefix = note
        .map(|note| indented_words(note, area.width, 2, Tone::Dim, theme))
        .unwrap_or_default();
    if let Some(error) = error {
        prefix.extend(indented_words(
            &format!("error: {error}"),
            area.width,
            2,
            Tone::Danger,
            theme,
        ));
    }
    let entry_cap = if embedded {
        COMPACT_VISIBLE_ENTRIES
    } else {
        usize::MAX
    };
    let all = entry_view_capped(picker, usize::MAX, area.width, theme, false, entry_cap);
    let mut body = draw_inline_screen(
        frame,
        area,
        title,
        (prefix.len() + all.lines.len()).saturating_sub(usize::from(note.is_some())),
        (!embedded).then(|| picker.footer()),
        theme,
    );
    // The intro occupies the title's usual blank row so it precedes choices
    // even when it wraps; other picker screens keep their existing spacing.
    if note.is_some() && area.height > 1 {
        body.y = body.y.saturating_sub(1);
        body.height = body.height.saturating_add(1);
    }
    let prefix_rows = prefix.len().min(usize::from(body.height).saturating_sub(1));
    let view = entry_view_capped(
        picker,
        usize::from(body.height).saturating_sub(prefix_rows),
        area.width,
        theme,
        false,
        entry_cap,
    );
    draw_picker_heading(frame, area, picker, title, view.scrolling, theme);
    prefix.truncate(prefix_rows);
    prefix.extend(view.lines);
    frame.render_widget(Paragraph::new(prefix), body);
}

/// Draws a bounded runtime picker above the composer; the host draws its shared footer.
pub(crate) fn draw_compact_resource_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &ResourcePicker,
    theme: Theme,
) {
    if area.is_empty() {
        return;
    }
    let view = entry_view(
        picker,
        usize::from(area.height).saturating_sub(1),
        area.width,
        theme,
        true,
    );
    draw_picker_heading(frame, area, picker, &picker.title, view.scrolling, theme);
    frame.render_widget(
        Paragraph::new(view.lines),
        Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        ),
    );
}

fn draw_picker_heading(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &ResourcePicker,
    title: &str,
    scrolling: bool,
    theme: Theme,
) {
    let count = picker.filtered_indices().len();
    let position = if scrolling && count > 0 {
        format!("{}/{}", picker.selected.min(count - 1) + 1, count)
    } else {
        String::new()
    };
    let filter = if picker.numbered {
        String::new()
    } else if picker.query.is_empty() {
        "type to filter".to_owned()
    } else {
        format!("filter: {}", picker.query.text())
    };
    let budget = usize::from(area.width)
        .saturating_sub(2 + position.width() + usize::from(!position.is_empty()));
    let title_budget = if picker.query.is_empty() {
        budget
    } else {
        budget.saturating_sub(12)
    };
    let title = clip_words(title, title_budget);
    let filter_budget = budget.saturating_sub(title.width() + 3);
    let (visible_query, query_cursor) = picker.query.viewport(filter_budget.saturating_sub(8));
    let filter = if !picker.numbered && !picker.query.is_empty() && filter_budget > 8 {
        format!("filter: {visible_query}")
    } else {
        clip_words(&filter, filter_budget)
    };
    let filter = if filter.is_empty() {
        filter
    } else {
        format!(" · {filter}")
    };
    let gap = usize::from(area.width)
        .saturating_sub(2 + title.width() + filter.width() + position.width());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("  {title}"), theme.style(Tone::Heading)),
            Span::styled(filter.clone(), theme.style(Tone::Dim)),
            Span::styled(
                format!("{}{position}", " ".repeat(gap)),
                theme.style(Tone::Dim),
            ),
        ])),
        Rect::new(area.x, area.y, area.width, area.height.min(1)),
    );
    if !picker.numbered && area.width > 0 && area.height > 0 {
        let prefix = if picker.query.is_empty() { 3 } else { 11 };
        let column = (2 + title.width() + prefix + query_cursor).min(usize::from(area.width - 1));
        frame.set_cursor_position((area.x + column as u16, area.y));
    }
}

/// Row measurement uses the production viewport logic, preserving the five-choice cap.
pub(crate) fn compact_resource_picker_rows(picker: &ResourcePicker) -> u16 {
    let view = entry_view(picker, usize::MAX, u16::MAX, Theme::new(), true);
    u16::try_from(view.lines.len() + 1).unwrap_or(u16::MAX)
}

pub(super) struct EntryView {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) scrolling: bool,
}

pub(super) fn entry_view(
    picker: &ResourcePicker,
    height: usize,
    width: u16,
    theme: Theme,
    compact: bool,
) -> EntryView {
    entry_view_capped(
        picker,
        height,
        width,
        theme,
        compact,
        if compact {
            COMPACT_VISIBLE_ENTRIES
        } else {
            usize::MAX
        },
    )
}

/// Embedded flows retain full descriptions while yielding list height to the transcript.
fn entry_view_capped(
    picker: &ResourcePicker,
    height: usize,
    width: u16,
    theme: Theme,
    compact: bool,
    entry_cap: usize,
) -> EntryView {
    let indices = picker.filtered_indices();
    if indices.is_empty() {
        let guidance = if picker.entries.is_empty() {
            picker.empty_guidance.as_str()
        } else {
            "No matches · Ctrl+U clear filter"
        };
        let mut lines = indented_words(guidance, width, 2, Tone::Warning, theme);
        if compact {
            lines = vec![Line::from(Span::styled(
                format!(
                    "  {}",
                    clip_words(guidance, usize::from(width.saturating_sub(2)))
                ),
                theme.style(Tone::Warning),
            ))];
        }
        lines.truncate(height);
        return EntryView {
            lines,
            scrolling: false,
        };
    }
    let selected = picker.selected.min(indices.len() - 1);
    // Both column budgets use the complete filtered list, so neither changes on scroll.
    let max_name = indices
        .iter()
        .map(|index| picker.entries[*index].label.width())
        .max()
        .unwrap_or(0);
    let states = indices
        .iter()
        .map(|index| {
            let entry = &picker.entries[*index];
            let current = if entry.active { "✓ current" } else { "" };
            match &entry.disabled_reason {
                Some(reason) => {
                    let prefix = if current.is_empty() {
                        String::new()
                    } else {
                        format!("{current} · ")
                    };
                    let full = format!("{prefix}unavailable: {reason}");
                    if 2 + max_name + 2 + full.width() + 2 + entry.description.width().min(8)
                        <= usize::from(width)
                    {
                        full
                    } else {
                        format!("{prefix}unavailable")
                    }
                }
                None => current.to_owned(),
            }
        })
        .collect::<Vec<_>>();
    let state_width = states.iter().map(|state| state.width()).max().unwrap_or(0);
    let name_width = max_name
        .min(usize::from(width).saturating_sub(4 + state_width))
        .min(usize::from(width) / 2);
    let groups = indices
        .iter()
        .enumerate()
        .map(|(offset, index)| {
            let entry = &picker.entries[*index];
            let chosen = offset == selected;
            let tone = if entry.disabled_reason.is_some() {
                Tone::Dim
            } else if chosen {
                Tone::Accent
            } else {
                Tone::Default
            };
            let mut lines = if picker.numbered {
                let label = format!(
                    "{}. {}{}",
                    offset + 1,
                    entry.label,
                    if states[offset].is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", states[offset])
                    }
                );
                let mut lines = indented_words(&label, width, 2, tone, theme);
                if let Some(line) = lines.first_mut() {
                    line.spans[0] = Span::styled(
                        if chosen { "❯ " } else { "  " },
                        theme.style(if chosen { Tone::Accent } else { Tone::Dim }),
                    );
                }
                lines
            } else {
                vec![list_row(
                    &entry.label,
                    &entry.description,
                    &states[offset],
                    chosen,
                    name_width,
                    width,
                    tone,
                    theme,
                )]
            };
            if chosen {
                let detail = if picker.numbered {
                    let extra = entry.selected_detail();
                    if extra.is_empty() {
                        entry.description.clone()
                    } else if entry.description.is_empty() {
                        extra
                    } else {
                        format!("{} · {extra}", entry.description)
                    }
                } else {
                    entry.selected_detail()
                };
                if !detail.is_empty() {
                    let indent = if picker.numbered {
                        4 + (offset + 1).to_string().len()
                    } else {
                        2
                    };
                    if !detail.chars().any(char::is_whitespace) || compact {
                        lines.push(Line::from(vec![
                            Span::raw(" ".repeat(indent)),
                            Span::styled(
                                if detail.chars().any(char::is_whitespace) {
                                    clip_words(&detail, usize::from(width).saturating_sub(indent))
                                } else {
                                    clip_name(&detail, usize::from(width).saturating_sub(indent))
                                },
                                theme.style(Tone::Dim),
                            ),
                        ]));
                    } else {
                        lines.extend(indented_words(&detail, width, indent, Tone::Dim, theme));
                    }
                }
            }
            lines
        })
        .collect::<Vec<_>>();
    let cap = entry_cap.min(indices.len());
    let mut start = selected;
    let mut used = groups[selected].len();
    while start > 0
        && selected - start + 1 < cap
        && used.saturating_add(groups[start - 1].len()) <= height
    {
        start -= 1;
        used += groups[start].len();
    }
    let mut lines = Vec::new();
    let mut visible = 0;
    for group in groups.into_iter().skip(start).take(cap) {
        if visible > 0 && lines.len().saturating_add(group.len()) > height {
            break;
        }
        visible += 1;
        lines.extend(group);
    }
    lines.truncate(height);
    EntryView {
        lines,
        scrolling: visible < indices.len(),
    }
}

/// Wraps prose at whole words with a stable hanging indent, including wide glyphs.
/// Tokens wider than a whole row use glyph boundaries so IDs and URLs remain readable.
pub(crate) fn indented_words(
    text: &str,
    width: u16,
    indent: usize,
    tone: Tone,
    theme: Theme,
) -> Vec<Line<'static>> {
    let available = width
        .saturating_sub(u16::try_from(indent).unwrap_or(u16::MAX))
        .max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut row = String::new();
        for word in paragraph.split_whitespace() {
            if !row.is_empty() && row.width() + 1 + word.width() > usize::from(available) {
                lines.push(Line::from(Span::styled(row, theme.style(tone))));
                row = String::new();
            }
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(word);
        }
        lines.push(Line::from(Span::styled(row, theme.style(tone))));
    }
    let mut rows = crate::render::wrap::wrap_lines(&lines, available);
    for row in &mut rows {
        row.spans.insert(0, Span::raw(" ".repeat(indent)));
    }
    rows
}

#[cfg(test)]
pub(super) fn picker_entry_lines(
    picker: &ResourcePicker,
    height: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    entry_view(picker, height, width, theme, true).lines
}

#[cfg(test)]
pub(super) fn picker_lines(
    picker: &ResourcePicker,
    height: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    entry_view(picker, height, width, theme, false).lines
}
