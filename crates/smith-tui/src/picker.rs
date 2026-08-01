//! Shared keyboard-first resource picker.
//!
//! Setup, runtime selection, and pre-host resume all use this reducer. Entries
//! contain bounded local display metadata only; filtering never touches model
//! history or a provider.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::theme::{Theme, Tone};

/// Maximum number of resource matches shown beside the composer.
///
/// Runtime choice is deliberately a small Codex-style pane. Setup and the
/// standalone pre-host resume surface keep using the full bordered picker.
const COMPACT_VISIBLE_ENTRIES: usize = 5;

/// One locally selectable resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceEntry {
    /// Stable selection identity.
    pub id: String,
    /// Primary display label.
    pub label: String,
    /// Bounded one-line context.
    pub detail: String,
    /// Marks the currently active resource.
    pub active: bool,
    /// Why this entry cannot be selected.
    pub disabled_reason: Option<String>,
}

impl ResourceEntry {
    /// A selectable entry.
    pub fn new(id: impl Into<String>, label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail: detail.into(),
            active: false,
            disabled_reason: None,
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
    pub query: String,
    /// Complete bounded local inventory.
    pub entries: Vec<ResourceEntry>,
    /// Selected index within the filtered list.
    pub selected: usize,
    /// Guidance shown when nothing matches or exists.
    pub empty_guidance: String,
}

/// A completed picker interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    /// Keep the picker open.
    Pending,
    /// Leave without applying a value.
    Cancelled,
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
            query: String::new(),
            entries,
            selected: 0,
            empty_guidance: empty_guidance.into(),
        }
    }

    /// Indices of entries matching the current filter.
    pub fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_ascii_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query.is_empty()
                    || entry.id.to_ascii_lowercase().contains(&query)
                    || entry.label.to_ascii_lowercase().contains(&query)
                    || entry.detail.to_ascii_lowercase().contains(&query)
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
            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                PickerOutcome::Cancelled
            }
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
            (KeyCode::Backspace, _) => {
                self.query.pop();
                self.selected = 0;
                PickerOutcome::Pending
            }
            (KeyCode::Char(character), modifiers)
                if !modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                self.query.push(character);
                self.selected = 0;
                PickerOutcome::Pending
            }
            _ => PickerOutcome::Pending,
        }
    }
}

/// Draws a bordered picker within `area`.
pub fn draw_resource_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &ResourcePicker,
    theme: Theme,
) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", picker.title))
        .border_style(theme.style(Tone::Dim));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let footer_rows = if inner.width < 60 { 2 } else { 1 };
    let [body, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(footer_rows)]).areas(inner);
    frame.render_widget(
        Paragraph::new(picker_lines(picker, usize::from(body.height), theme))
            .wrap(Wrap { trim: false }),
        body,
    );
    frame.render_widget(
        Paragraph::new(if footer_rows == 1 {
            " ↑/↓ choose · Enter confirm · Esc cancel"
        } else {
            " ↑/↓ choose · Enter confirm\n Esc cancel"
        })
        .style(theme.style(Tone::Dim)),
        footer,
    );
}

/// Draws a bounded runtime picker directly above the fixed composer.
pub(crate) fn draw_compact_resource_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &ResourcePicker,
    theme: Theme,
) {
    if area.is_empty() {
        return;
    }

    let indices = picker.filtered_indices();
    let selected = picker.selected.min(indices.len().saturating_sub(1));
    let position = if indices.len() > COMPACT_VISIBLE_ENTRIES {
        format!(" · {}/{}", selected.saturating_add(1), indices.len())
    } else {
        String::new()
    };
    let filter = if picker.query.is_empty() {
        "type to filter".to_owned()
    } else {
        format!("filter: {}", picker.query)
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("  {}", picker.title), theme.style(Tone::Heading)),
        Span::styled(format!(" · {filter}{position}"), theme.style(Tone::Dim)),
    ])];
    lines.extend(picker_entry_lines(
        picker,
        usize::from(area.height).saturating_sub(1),
        theme,
    ));
    // Runtime entries stay one row each. Long metadata clips at the terminal
    // edge instead of making the compact pane grow or reflow.
    frame.render_widget(Paragraph::new(lines), area);
}

/// Rows requested by the compact runtime picker before terminal constraints.
pub(crate) fn compact_resource_picker_rows(picker: &ResourcePicker) -> u16 {
    let matches = picker.filtered_indices().len().max(1);
    u16::try_from(matches.min(COMPACT_VISIBLE_ENTRIES).saturating_add(1)).unwrap_or(u16::MAX)
}

fn picker_lines(picker: &ResourcePicker, height: usize, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(" filter: ", theme.style(Tone::Dim)),
        Span::styled(
            if picker.query.is_empty() {
                "type to search".to_owned()
            } else {
                picker.query.clone()
            },
            theme.style(if picker.query.is_empty() {
                Tone::Dim
            } else {
                Tone::Default
            }),
        ),
    ])];
    lines.extend(picker_entry_lines(picker, height.saturating_sub(1), theme));
    lines
}

fn picker_entry_lines(picker: &ResourcePicker, height: usize, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let indices = picker.filtered_indices();
    if indices.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(" {}", picker.empty_guidance),
            theme.style(Tone::Warning),
        )));
    } else {
        let capacity = height.max(1);
        let selected = picker.selected.min(indices.len().saturating_sub(1));
        let start = selected
            .saturating_sub(capacity / 2)
            .min(indices.len().saturating_sub(capacity));
        for (filtered_index, raw_index) in indices.iter().enumerate().skip(start).take(capacity) {
            let entry = &picker.entries[*raw_index];
            let marker = if filtered_index == selected {
                "›"
            } else {
                " "
            };
            let mut suffix = String::new();
            if entry.active {
                suffix.push_str(" · current");
            }
            if let Some(reason) = &entry.disabled_reason {
                suffix.push_str(" · unavailable: ");
                suffix.push_str(reason);
            }
            let tone = if entry.disabled_reason.is_some() {
                Tone::Dim
            } else if filtered_index == selected {
                Tone::Accent
            } else {
                Tone::Default
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {marker} {}", entry.label), theme.style(tone)),
                Span::styled(
                    format!("  {}{suffix}", entry.detail),
                    theme.style(Tone::Dim),
                ),
            ]));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
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
    fn narrow_no_color_picker_keeps_active_disabled_and_controls_textual() {
        let picker = ResourcePicker::new(
            "Models",
            vec![
                ResourceEntry::new("zai/glm", "zai/glm", "trusted").active(true),
                ResourceEntry::new("broken", "broken", "local").disabled("missing limits"),
                ResourceEntry::new("router/gpt", "router/gpt", "explicit"),
            ],
            "run setup",
        );
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).expect("terminal");
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
        let buffer = terminal.backend().buffer();
        let rendered = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("current"), "{rendered}");
        assert!(rendered.contains("unavailable:"), "{rendered}");
        assert!(rendered.contains("missing limits"), "{rendered}");
        assert!(rendered.contains("Enter confirm"), "{rendered}");
        assert!(rendered.contains('›'), "{rendered}");
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

        picker.query = "vision".to_owned();
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
            Theme::from_env().without_color().without_motion(),
        );
        assert_eq!(lines.len(), 6, "rendering is bounded to the viewport");
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
