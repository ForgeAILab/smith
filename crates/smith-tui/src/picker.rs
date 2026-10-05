//! Shared keyboard-first resource picker.
//!
//! Setup, runtime selection, and pre-host resume all use this reducer. Entries
//! contain bounded local display metadata only; filtering never touches model
//! history or a provider.

use std::convert::Infallible;
use std::hash::{DefaultHasher, Hash, Hasher};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::line_input::LineInput;
use crate::render::lists::{clip_name, clip_words, list_row};
use crate::screen::{FlowOutcome, Screen, ScreenEvent, Step};
use crate::theme::{Theme, Tone};

/// Maximum number of resource matches shown beside the composer.
///
/// Session choices keep five matches beside the composer, including connection
/// steps. Standalone setup and pre-host resume use the available terminal height.
const COMPACT_VISIBLE_ENTRIES: usize = 5;

/// One locally selectable resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceEntry {
    /// Stable selection identity.
    pub id: String,
    /// Primary display label.
    pub label: String,
    /// Short context shown on every row.
    pub description: String,
    /// Additional context shown only beneath the selected row.
    pub detail: String,
    /// Marks the currently active resource.
    pub active: bool,
    /// Why this entry cannot be selected.
    pub disabled_reason: Option<String>,
}

impl ResourceEntry {
    /// A selectable entry.
    pub fn new(id: impl Into<String>, label: impl Into<String>, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            id: id.into(),
            label: label.into(),
            description: detail.clone(),
            detail,
            active: false,
            disabled_reason: None,
        }
    }

    /// Separates a short row description from the full selected-row context.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    fn selected_detail(&self) -> String {
        // Resource facts use the same separator in both columns. Compare
        // whole facts so a short description never removes part of a value.
        let detail = self
            .detail
            .split(" · ")
            .filter(|part| {
                !self
                    .description
                    .split(" · ")
                    .any(|description| description == *part)
            })
            .collect::<Vec<_>>()
            .join(" · ");
        match (&self.disabled_reason, detail.is_empty()) {
            (Some(reason), true) => reason.clone(),
            (Some(reason), false) => format!("{reason} · {detail}"),
            (None, _) => detail,
        }
    }

    /// Marks the active entry.
    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Makes the entry visible but non-selectable.
    #[must_use]
    pub fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }
}

/// Pure state of one resource picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePicker {
    /// Human-facing resource name.
    pub title: String,
    /// Current filter.
    pub query: LineInput,
    /// Complete bounded local inventory.
    pub entries: Vec<ResourceEntry>,
    /// Selected index within the filtered list.
    pub selected: usize,
    /// Guidance shown when the local inventory is empty.
    pub empty_guidance: String,
    // The owner explicitly supplies controls in the empty guidance.
    empty_guidance_names_keys: bool,
    // Fixed choices cannot accidentally consume letters as a filter.
    numbered: bool,
    // The owning flow supplies whether Escape has a previous step.
    back: bool,
}

/// A completed picker interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    /// Keep the picker open.
    Pending,
    /// Leave without applying a value.
    Cancelled,
    /// Return to the owning flow’s previous step without committing.
    Back,
    /// Apply the full stable entry ID.
    Selected(String),
}

impl ResourcePicker {
    /// Creates a picker over bounded local entries.
    pub fn new(
        title: impl Into<String>,
        entries: Vec<ResourceEntry>,
        empty_guidance: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            query: LineInput::default(),
            entries,
            selected: 0,
            empty_guidance: empty_guidance.into(),
            empty_guidance_names_keys: false,
            numbered: false,
            back: false,
        }
    }

    /// Creates fixed choices so digits confirm and prose cannot hide an option.
    ///
    /// Flows should offer at most nine choices; longer inventories use `new`.
    pub fn choices(
        title: impl Into<String>,
        entries: Vec<ResourceEntry>,
        empty_guidance: impl Into<String>,
    ) -> Self {
        Self {
            numbered: true,
            ..Self::new(title, entries, empty_guidance)
        }
    }

    /// Makes Escape return one step while Ctrl+C still cancels the entire flow.
    #[must_use]
    pub fn with_back(mut self, back: bool) -> Self {
        self.back = back;
        self
    }

    /// Omits empty-list controls when the guidance already names its keys.
    #[must_use]
    pub fn with_empty_guidance_keys(mut self) -> Self {
        self.empty_guidance_names_keys = true;
        self
    }

    /// Supplies shared control wording to hosts whose footer is outside the list.
    pub fn footer(&self) -> ScreenFooter {
        if self.entries.is_empty() && self.empty_guidance_names_keys {
            return ScreenFooter::None;
        }
        if self.filtered_indices().is_empty() {
            return ScreenFooter::Empty {
                filtered: !self.entries.is_empty(),
                back: self.back,
            };
        }
        ScreenFooter::List {
            choices: self.numbered.then_some(self.entries.len().min(9)),
            back: self.back,
        }
    }

    /// Indices of entries matching the current filter.
    pub fn filtered_indices(&self) -> Vec<usize> {
        let query = if self.numbered {
            String::new()
        } else {
            self.query.text().trim().to_ascii_lowercase()
        };
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query.is_empty()
                    || entry.id.to_ascii_lowercase().contains(&query)
                    || entry.label.to_ascii_lowercase().contains(&query)
                    || entry.description.to_ascii_lowercase().contains(&query)
                    || entry.detail.to_ascii_lowercase().contains(&query)
                    || entry
                        .disabled_reason
                        .as_ref()
                        .is_some_and(|reason| reason.to_ascii_lowercase().contains(&query))
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// The selected filtered entry.
    pub fn selected_entry(&self) -> Option<&ResourceEntry> {
        let indices = self.filtered_indices();
        indices
            .get(self.selected.min(indices.len().saturating_sub(1)))
            .and_then(|index| self.entries.get(*index))
    }

    /// Reduces one key without performing effects.
    pub fn on_key(&mut self, key: KeyEvent) -> PickerOutcome {
        if key.kind == KeyEventKind::Release {
            return PickerOutcome::Pending;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                PickerOutcome::Cancelled
            }
            (KeyCode::Esc | KeyCode::BackTab, _) if self.back => PickerOutcome::Back,
            (KeyCode::Esc, _) => PickerOutcome::Cancelled,
            (KeyCode::Up | KeyCode::BackTab, _) => {
                let count = self.filtered_indices().len();
                if count > 0 {
                    self.selected = self.selected.checked_sub(1).unwrap_or(count - 1);
                }
                PickerOutcome::Pending
            }
            (KeyCode::Down | KeyCode::Tab, _) => {
                let count = self.filtered_indices().len();
                if count > 0 {
                    self.selected = (self.selected + 1) % count;
                }
                PickerOutcome::Pending
            }
            (KeyCode::Enter, _) => match self.selected_entry() {
                Some(entry) if entry.disabled_reason.is_none() => {
                    PickerOutcome::Selected(entry.id.clone())
                }
                _ => PickerOutcome::Pending,
            },
            (KeyCode::Char(character), modifiers)
                if !modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                if self.numbered {
                    if let Some(index) = character
                        .to_digit(10)
                        .filter(|digit| (1..=9).contains(digit))
                        .map(|digit| digit as usize - 1)
                        && let Some(entry) = self.entries.get(index)
                        && entry.disabled_reason.is_none()
                    {
                        self.selected = index;
                        return PickerOutcome::Selected(entry.id.clone());
                    }
                } else {
                    self.query.on_key(key);
                    self.selected = 0;
                }
                PickerOutcome::Pending
            }
            _ => {
                if !self.numbered && self.query.on_key(key) {
                    self.selected = 0;
                }
                PickerOutcome::Pending
            }
        }
    }

    /// Inserts pasted text to the filter query, control characters dropped.
    pub fn paste(&mut self, text: &str) {
        if self.numbered {
            return;
        }
        let cleaned = text
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>();
        if cleaned.is_empty() {
            return;
        }
        self.query.paste(&cleaned);
        self.selected = 0;
    }
}

impl Screen for ResourcePicker {
    /// Titles distinguish chooser steps; filtering and selection keep the same key.
    fn step_key(&self) -> u64 {
        let mut key = DefaultHasher::new();
        self.title.hash(&mut key);
        key.finish()
    }

    type Outcome = FlowOutcome<String>;
    type Effect = Infallible;

    fn draw(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
        draw_resource_picker(frame, area, self, theme);
    }

    fn draw_embedded(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
        draw_picker_context(
            frame,
            area,
            self,
            PickerContext {
                title: &self.title,
                note: None,
                error: None,
                embedded: true,
            },
            theme,
        );
    }

    fn content_height(&self, width: u16) -> u16 {
        picker_content_height(self, width, None, None)
    }

    fn footer(&self) -> Option<ScreenFooter> {
        Some(ResourcePicker::footer(self))
    }

    fn on_event(&mut self, event: ScreenEvent) -> Step<Self::Outcome, Self::Effect> {
        match event {
            ScreenEvent::Key(key) => match self.on_key(key) {
                PickerOutcome::Pending => Step::Pending,
                PickerOutcome::Cancelled => Step::Outcome(FlowOutcome::Cancelled),
                PickerOutcome::Back => Step::Outcome(FlowOutcome::Back),
                PickerOutcome::Selected(id) => Step::Outcome(FlowOutcome::Completed(id)),
            },
            ScreenEvent::Paste(text) => {
                self.paste(&text);
                Step::Pending
            }
            ScreenEvent::Resize(_, _) | ScreenEvent::Tick => Step::Pending,
        }
    }
}

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
    draw_picker_with_context(frame, area, picker, &picker.title, None, None, theme);
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

struct EntryView {
    lines: Vec<Line<'static>>,
    scrolling: bool,
}

fn entry_view(
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
fn picker_entry_lines(
    picker: &ResourcePicker,
    height: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    entry_view(picker, height, width, theme, true).lines
}

#[cfg(test)]
fn picker_lines(
    picker: &ResourcePicker,
    height: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    entry_view(picker, height, width, theme, false).lines
}

#[cfg(test)]
mod tests {
    #[test]
    fn step_key_tracks_the_title_without_repainting_filter_or_selection_changes() {
        use crate::Screen;

        let mut picker = ResourcePicker::new(
            "Choose model",
            vec![ResourceEntry::new("model", "Model", "model detail")],
            "No models",
        );
        let initial = picker.step_key();
        picker.on_key(key(KeyCode::Down));
        picker.paste("model");
        assert_eq!(picker.step_key(), initial);
        picker.title = "Choose provider".into();
        assert_ne!(picker.step_key(), initial);
    }

    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn empty_resume_guidance_replaces_the_footer_but_filters_keep_recovery_keys() {
        let mut picker = ResourcePicker::new(
            "Resume session",
            Vec::new(),
            "No sessions to resume in this project · esc exits",
        )
        .with_empty_guidance_keys();
        for width in [44, 80, 100] {
            assert!(picker.footer().rows(width).is_empty());
            let rendered = render_picker(&picker, width, 16);
            assert!(rendered.contains("esc exits"), "{rendered}");
            assert!(!rendered.contains("esc cancel"), "{rendered}");
            assert!(!rendered.contains("enter confirm"), "{rendered}");
            assert!(!rendered.contains("↑↓ choose"), "{rendered}");
            let rows = rendered.lines().collect::<Vec<_>>();
            let guidance_end = rows
                .iter()
                .position(|row| row.contains("esc exits"))
                .expect("empty resume guidance");
            assert!(
                rows[guidance_end + 1..]
                    .iter()
                    .all(|row| row.trim().is_empty()),
                "empty resume must have no footer row: {rendered}"
            );
        }
        assert_eq!(picker.on_key(key(KeyCode::Esc)), PickerOutcome::Cancelled);
        picker
            .entries
            .push(ResourceEntry::new("session", "Prompt", "session-id"));
        assert!(
            picker
                .footer()
                .rows(80)
                .join(" · ")
                .contains("enter confirm")
        );
        picker.query = "no matches".into();
        for width in [44, 100] {
            assert_eq!(
                picker.footer().rows(width).join(" · "),
                "ctrl+u clear filter · esc cancel"
            );
            let rendered = render_picker(&picker, width, 16);
            assert!(!rendered.contains("enter confirm"), "{rendered}");
            assert!(!rendered.contains("↑↓ choose"), "{rendered}");
        }
    }

    #[test]
    fn empty_session_pickers_keep_the_cancel_hint_without_an_explicit_flag() {
        let picker = ResourcePicker::new(
            "Resume session",
            Vec::new(),
            "No sessions to resume · esc cancel",
        );
        for width in [44, 80] {
            assert_eq!(picker.footer().hint(width), "esc cancel");
        }
    }

    #[test]
    fn selected_session_id_stays_on_one_row_at_44_columns() {
        for id in [
            "session-6465-42d4-be69-180feb01a926",
            "session-2042b4df-6465-42d4-be69-180feb01a926",
            "会話-2042b4df-6465-42d4-be69-180feb01a926-too-long",
        ] {
            let picker = ResourcePicker::new(
                "Resume session",
                vec![
                    ResourceEntry::new(id, "Explain lib.rs", id).description("2 min ago · 1 turn"),
                ],
                "empty",
            );
            let rows = picker_lines(&picker, 10, 44, Theme::new());
            assert_eq!(rows.len(), 2, "{rows:?}");
            let detail = rows[1].to_string();
            if id.width() <= 42 {
                assert_eq!(detail, format!("  {id}"));
            } else {
                assert!(detail.starts_with("  "), "{detail}");
                assert!(detail.ends_with('…'), "{detail}");
                assert!(
                    id.starts_with(detail.trim_start().trim_end_matches('…')),
                    "{detail}"
                );
                assert!(detail.width() <= 44, "{detail}");
            }
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn render_picker(picker: &ResourcePicker, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| picker.draw(frame, frame.area(), Theme::new().without_color()))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn fixed_choices_confirm_digits_and_ignore_filter_input() {
        let mut picker = ResourcePicker::choices(
            "Method",
            vec![
                ResourceEntry::new("one", "One", "detail"),
                ResourceEntry::new("two", "Two", "detail").disabled("unavailable"),
                ResourceEntry::new("three", "Three", "detail"),
            ],
            "empty",
        );
        for character in ['a', '2', '4', '9', '0'] {
            assert_eq!(
                picker.on_key(key(KeyCode::Char(character))),
                PickerOutcome::Pending
            );
        }
        picker.paste("letters and digits 3");
        assert!(picker.query.is_empty());
        assert_eq!(picker.filtered_indices(), vec![0, 1, 2]);
        assert_eq!(
            picker.selected, 0,
            "invalid and disabled digits leave selection alone"
        );
        assert_eq!(
            picker.on_key(key(KeyCode::Char('3'))),
            PickerOutcome::Selected("three".into())
        );
        assert_eq!(picker.selected, 2);
        let screen = render_picker(&picker, 44, 16);
        assert!(screen.contains("❯ 3. Three"), "{screen}");
        assert!(!screen.contains("type to filter"), "{screen}");
        let mut inventory = ResourcePicker::new("Inventory", picker.entries, "empty");
        inventory.on_key(key(KeyCode::Char('3')));
        assert_eq!(inventory.query, "3", "inventory digits remain filter text");
    }

    #[test]
    fn standalone_title_and_footer_are_unframed_and_content_sized() {
        let picker = ResourcePicker::choices(
            "Choose a method",
            vec![ResourceEntry::new("one", "First method", "selected detail")],
            "empty",
        );
        for (width, height) in [(44, 16), (100, 32)] {
            let screen = render_picker(&picker, width, height);
            let rows = screen.lines().collect::<Vec<_>>();
            assert!(rows[0].starts_with("  Choose a method"), "{screen}");
            assert!(rows[1].trim().is_empty());
            assert!(rows[2].starts_with("❯ 1. First method"));
            assert!(rows[3].starts_with("     selected detail"));
            for glyph in ['┌', '┐', '└', '┘', '│', '╭', '╮', '╰', '╯'] {
                assert!(!screen.contains(glyph), "{screen}");
            }
            let footer = rows
                .iter()
                .position(|row| row.contains("enter confirm"))
                .expect("footer");
            assert_eq!(footer, 5, "one blank row separates the content and footer");
            assert!(rows[footer].starts_with("  "));
            assert!(rows[footer..].join("\n").contains("esc cancel"));
            assert!(rows[footer..].join("\n").contains("↑↓ or 1–1 choose"));
        }
    }

    #[test]
    fn embedded_inventory_keeps_five_choices_and_reaches_the_last_entry() {
        let mut picker = ResourcePicker::new(
            "Connect OpenRouter · Choose model",
            (0..30)
                .map(|index| {
                    ResourceEntry::new(index.to_string(), format!("resource-{index}"), "metadata")
                })
                .collect(),
            "empty",
        );
        for (width, height) in [(44, 16), (100, 32)] {
            let mut app = crate::App::new("gpt-5.3", "~/work/api");
            app.transcript.push_user("retained transcript");
            app.composer.insert_str("retained draft");
            for selected in [0, 29] {
                picker.selected = selected;
                let mut terminal =
                    Terminal::new(TestBackend::new(width, height)).expect("terminal");
                terminal
                    .draw(|frame| {
                        crate::render::draw_with_screen(
                            frame,
                            &app,
                            &picker,
                            Theme::new().without_color(),
                        );
                    })
                    .expect("draw");
                let buffer = terminal.backend().buffer();
                let text = (0..height)
                    .map(|y| {
                        (0..width)
                            .map(|x| buffer[(x, y)].symbol())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert_eq!(
                    text.lines().filter(|row| row.contains("resource-")).count(),
                    COMPACT_VISIBLE_ENTRIES,
                    "{text}"
                );
                for expected in [
                    "retained transcript".to_owned(),
                    "retained draft".to_owned(),
                    format!("❯ resource-{selected}"),
                    format!("{}/30", selected + 1),
                    "enter confirm".to_owned(),
                    "esc cancel".to_owned(),
                ] {
                    assert!(text.contains(&expected), "{text}");
                }
            }
        }
    }

    #[test]
    fn scroll_position_and_description_column_use_the_complete_filtered_list() {
        let entries = (0..20)
            .map(|index| {
                ResourceEntry::new(
                    index.to_string(),
                    if index == 19 {
                        "longer label".to_owned()
                    } else {
                        format!("row-{index}")
                    },
                    "metadata",
                )
            })
            .collect();
        let mut picker = ResourcePicker::new("Inventory", entries, "empty");
        for compact in [false, true] {
            let before = entry_view(&picker, 5, 80, Theme::new(), compact);
            assert!(before.scrolling);
            let column = before.lines[0]
                .to_string()
                .find("metadata")
                .expect("description");
            picker.selected = 19;
            let after = entry_view(&picker, 5, 80, Theme::new(), compact);
            let last = after
                .lines
                .iter()
                .find(|line| line.to_string().contains("longer label"))
                .expect("last entry")
                .to_string();
            assert_eq!(
                last.find("metadata"),
                Some(column),
                "scrolling keeps columns stable"
            );
            let screen = render_picker(&picker, 44, 8);
            assert!(
                screen
                    .lines()
                    .next()
                    .expect("heading")
                    .trim_end()
                    .ends_with("20/20"),
                "{screen}"
            );
            assert!(
                screen.contains("enter confirm") && screen.contains("esc cancel"),
                "{screen}"
            );
            picker.selected = 0;
        }
    }

    #[test]
    fn shared_footer_prioritizes_enter_and_escape_at_44_columns() {
        for choices in [None, Some(4)] {
            let footer = ScreenFooter::List {
                choices,
                back: true,
            };
            let rows = footer.rows(44);
            assert!(rows[0].starts_with("enter confirm · esc back"));
            assert!(rows.iter().all(|row| row.width() + 2 <= 44));
            assert!(footer.rows(100)[0].starts_with("↑↓"));
        }
        assert_eq!(
            ScreenFooter::Field { back: true }.rows(44),
            ["enter continue · esc back"]
        );
        assert_eq!(
            ScreenFooter::Review {
                back: true,
                scroll: None
            }
            .rows(44),
            ["enter confirm · esc back"]
        );
        assert_eq!(
            ScreenFooter::Progress { back: true }.rows(44),
            ["esc back · ctrl+c cancel"]
        );
        assert_eq!(
            ScreenFooter::Progress { back: false }.rows(44),
            ["esc cancel"]
        );
    }

    #[test]
    fn picker_distinguishes_back_from_whole_flow_cancel_and_ignores_releases() {
        let mut picker = ResourcePicker::choices("Method", Vec::new(), "empty").with_back(true);
        assert_eq!(
            picker.on_event(ScreenEvent::Key(key(KeyCode::Esc))),
            Step::Outcome(FlowOutcome::Back)
        );
        assert_eq!(
            picker.on_event(ScreenEvent::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Step::Outcome(FlowOutcome::Cancelled)
        );
        assert_eq!(
            picker.on_key(KeyEvent::new_with_kind(
                KeyCode::Esc,
                KeyModifiers::NONE,
                KeyEventKind::Release
            )),
            PickerOutcome::Pending
        );
        picker = picker.with_back(false);
        assert_eq!(
            picker.on_event(ScreenEvent::Key(key(KeyCode::Esc))),
            Step::Outcome(FlowOutcome::Cancelled)
        );
    }

    #[test]
    fn filtering_selection_and_cancellation_are_pure() {
        let mut picker = ResourcePicker::new(
            "Models",
            vec![
                ResourceEntry::new("zai/glm", "zai/glm", "GLM"),
                ResourceEntry::new("router/gpt", "router/gpt", "OpenRouter"),
            ],
            "run setup",
        );
        assert_eq!(
            picker.on_key(key(KeyCode::Char('g'))),
            PickerOutcome::Pending
        );
        assert_eq!(picker.filtered_indices(), vec![0, 1]);
        assert_eq!(
            picker.on_key(key(KeyCode::Char('l'))),
            PickerOutcome::Pending
        );
        assert_eq!(picker.filtered_indices(), vec![0]);
        assert_eq!(
            picker.on_key(key(KeyCode::Enter)),
            PickerOutcome::Selected("zai/glm".into())
        );
        assert_eq!(picker.on_key(key(KeyCode::Esc)), PickerOutcome::Cancelled);
    }

    #[test]
    fn disabled_and_empty_entries_cannot_be_selected() {
        let mut picker = ResourcePicker::new(
            "Providers",
            vec![ResourceEntry::new("broken", "broken", "").disabled("missing model")],
            "run setup",
        );
        assert_eq!(picker.on_key(key(KeyCode::Enter)), PickerOutcome::Pending);
        picker.query = "absent".into();
        assert_eq!(picker.on_key(key(KeyCode::Enter)), PickerOutcome::Pending);
    }

    #[test]
    fn filtering_keeps_selected_detail_and_short_description_searchable() {
        let mut picker = ResourcePicker::new(
            "Choose model",
            vec![
                ResourceEntry::new("local/model", "model", "project config · input 124k")
                    .description("local · 128k context"),
            ],
            "run setup",
        );
        for query in ["project config", "input 124k", "128k context"] {
            picker.query = query.to_owned().into();
            assert_eq!(picker.filtered_indices(), [0]);
            assert_eq!(
                picker.on_key(key(KeyCode::Enter)),
                PickerOutcome::Selected("local/model".to_owned())
            );
        }
    }

    #[test]
    fn selected_detail_omits_facts_already_in_the_description() {
        for (entry, expected, repeated) in [
            (
                ResourceEntry::new("0", "1", "env:FIRST · 25% used").description("25% used"),
                "env:FIRST",
                vec!["25% used"],
            ),
            (
                ResourceEntry::new(
                    "dev",
                    "dev",
                    "build · use main · zai/glm-5.3 · coding · rev r1",
                )
                .description("build · coding"),
                "use main · zai/glm-5.3 · rev r1",
                vec!["build", "coding"],
            ),
        ] {
            assert_eq!(entry.selected_detail(), expected);
            let picker = ResourcePicker::new("Choose resource", vec![entry], "no resources");
            for width in [44, 100] {
                for theme in [Theme::new(), Theme::new().without_color()] {
                    let lines = picker_entry_lines(&picker, 2, width, theme);
                    assert_eq!(lines.len(), 2);
                    let text = lines
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n");
                    for fact in &repeated {
                        assert_eq!(text.matches(*fact).count(), 1, "{text}");
                    }
                    assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
                }
            }
        }
        let same = ResourceEntry::new("one", "one", "short · description");
        assert!(same.selected_detail().is_empty());
        assert_eq!(
            same.disabled("missing credential").selected_detail(),
            "missing credential"
        );
    }

    #[test]
    fn empty_inventory_and_unmatched_filter_have_distinct_guidance() {
        let empty = ResourcePicker::new(
            "Models",
            Vec::new(),
            "No local model is selectable · run smith setup add-model",
        );
        let empty_lines = picker_lines(
            &empty,
            3,
            44,
            Theme::from_env().without_color().without_motion(),
        );
        let empty_text = empty_lines
            .iter()
            .map(|line| line.to_string().trim().to_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(empty_text.contains("run smith setup add-model"));
        assert!(!empty_text.contains("No matches"));

        let mut filtered = ResourcePicker::new(
            "Models",
            vec![ResourceEntry::new("local/model", "local/model", "local")],
            "No local model is selectable · run smith setup add-model",
        );
        filtered.query = "does-not-exist".to_owned().into();
        filtered.selected = 4;
        let filtered_lines = picker_lines(
            &filtered,
            3,
            44,
            Theme::from_env().without_color().without_motion(),
        );
        let filtered_text = filtered_lines
            .iter()
            .map(|line| line.to_string().trim().to_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(filtered_text.contains("No matches"), "{filtered_text}");
        assert!(
            filtered_text.contains("Ctrl+U clear filter"),
            "{filtered_text}"
        );

        assert_eq!(
            filtered.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL,)),
            PickerOutcome::Pending
        );
        assert!(filtered.query.is_empty());
        assert_eq!(filtered.selected, 0);
        assert_eq!(
            filtered.selected_entry().map(|entry| entry.id.as_str()),
            Some("local/model")
        );
    }

    #[test]
    fn state_labels_stay_at_the_right_and_unavailable_stays_disabled() {
        let long_detail =
            "advertised capabilities, context window, output ceiling, and request budget "
                .repeat(4);
        let mut picker = ResourcePicker::new(
            "Models",
            vec![
                ResourceEntry::new("local/model", "model", long_detail.clone())
                    .description("local")
                    .active(true),
                ResourceEntry::new("broken/model", "broken", long_detail)
                    .description("local")
                    .disabled("missing limits"),
            ],
            "run setup",
        );

        let mut terminal = Terminal::new(TestBackend::new(44, 30)).expect("terminal");
        terminal
            .draw(|frame| {
                draw_resource_picker(
                    frame,
                    frame.area(),
                    &picker,
                    Theme::from_env().without_color().without_motion(),
                );
            })
            .expect("draw");
        let rendered = (0..terminal.backend().buffer().area.height)
            .map(|y| {
                (0..terminal.backend().buffer().area.width)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let current = rendered.find("current").expect("current state is visible");
        let metadata = rendered.find("advertised").expect("metadata is visible");
        assert!(current < metadata, "{rendered}");
        assert!(
            rendered.contains("unavailable: missing limits"),
            "{rendered}"
        );

        picker.query = "broken".to_owned().into();
        assert_eq!(picker.on_key(key(KeyCode::Enter)), PickerOutcome::Pending);
    }

    #[test]
    fn narrow_no_color_picker_keeps_active_disabled_and_controls_textual() {
        let mut picker = ResourcePicker::new(
            "Models",
            vec![
                ResourceEntry::new("zai/glm", "zai/glm", "trusted").active(true),
                ResourceEntry::new("broken", "broken", "local").disabled("missing limits"),
                ResourceEntry::new("router/gpt", "router/gpt", "explicit"),
            ],
            "run setup",
        );
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).expect("terminal");
        let mut render_picker = |picker: &ResourcePicker| {
            terminal
                .draw(|frame| {
                    draw_resource_picker(
                        frame,
                        frame.area(),
                        picker,
                        Theme::from_env().without_color().without_motion(),
                    );
                })
                .expect("draw");
            let buffer = terminal.backend().buffer();
            (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let rendered = render_picker(&picker);
        assert!(rendered.contains("✓ current"), "{rendered}");
        assert!(
            rendered
                .lines()
                .any(|line| line.contains("broken") && line.trim_end().ends_with("unavailable")),
            "{rendered}"
        );
        assert!(!rendered.contains("missing limits"), "{rendered}");
        assert!(rendered.contains("enter confirm"), "{rendered}");
        assert!(rendered.contains('❯'), "{rendered}");

        picker.on_key(key(KeyCode::Down));
        let rendered = render_picker(&picker);
        assert!(
            rendered.lines().collect::<Vec<_>>().windows(2).any(|rows| {
                rows[0].contains("❯ broken") && rows[1].contains("missing limits")
            }),
            "{rendered}"
        );
    }

    #[test]
    fn hundreds_of_catalog_entries_remain_bounded_searchable_and_deterministic() {
        let entries = (0..600)
            .map(|index| {
                let id = format!("router/vendor/model-{index:04}");
                let detail = if index == 599 {
                    "OpenRouter · tools+reasoning+vision"
                } else {
                    "OpenRouter · tools"
                };
                let entry = ResourceEntry::new(&id, format!("Model {index:04}"), detail);
                if index == 400 {
                    entry.disabled("catalog model does not support tool calling")
                } else {
                    entry
                }
            })
            .collect();
        let mut picker = ResourcePicker::new("Models", entries, "run setup");

        picker.query = "vision".to_owned().into();
        assert_eq!(picker.filtered_indices(), [599]);
        assert_eq!(
            picker.on_key(key(KeyCode::Enter)),
            PickerOutcome::Selected("router/vendor/model-0599".to_owned())
        );

        picker.query.clear();
        picker.selected = 599;
        let lines = picker_lines(
            &picker,
            6,
            80,
            Theme::from_env().without_color().without_motion(),
        );
        assert!(lines.len() <= 6, "rendering is bounded to the viewport");
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Model 0599"), "{rendered}");
        assert!(!rendered.contains("Model 0000"), "{rendered}");
        assert_eq!(picker.on_key(key(KeyCode::Esc)), PickerOutcome::Cancelled);
    }
}
