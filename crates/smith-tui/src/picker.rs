//! Shared keyboard-first resource picker.
//!
//! Setup, runtime selection, and pre-host resume all use this reducer. Entries
//! contain bounded local display metadata only; filtering never touches model
//! history or a provider.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::render::lists::{clip_words, detail_line, list_row};
use crate::theme::{Theme, Tone};

/// Maximum number of resource matches shown beside the composer.
///
/// Runtime choice keeps five choices beside the composer. Setup and the
/// standalone pre-host resume surface keep using the full bordered picker.
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
    pub query: String,
    /// Complete bounded local inventory.
    pub entries: Vec<ResourceEntry>,
    /// Selected index within the filtered list.
    pub selected: usize,
    /// Guidance shown when the local inventory is empty.
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
            (KeyCode::Char('u' | 'U'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.clear();
                self.selected = 0;
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

    /// Appends pasted text to the filter query, control characters dropped.
    pub fn paste(&mut self, text: &str) {
        let cleaned = text
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>();
        if cleaned.is_empty() {
            return;
        }
        self.query.push_str(&cleaned);
        self.selected = 0;
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
    let lines = picker_lines(picker, usize::from(body.height), body.width, theme);
    // Empty-inventory guidance may be longer than a narrow pane and should
    // still expose its setup command. Choices and their selected detail, by
    // contrast, each stay on one row so the next state label keeps its place.
    let lines = if picker.filtered_indices().is_empty() && lines.len() > 1 {
        let mut wrapped = vec![lines[0].clone()];
        wrapped.extend(crate::render::wrap::wrap_lines(&lines[1..], body.width));
        wrapped
    } else {
        lines
    };
    // Choice descriptions and selected detail are already bounded by width;
    // wrapping here would consume the next resource's reserved row.
    frame.render_widget(Paragraph::new(lines), body);
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
    let position = if !indices.is_empty() {
        format!("{}/{}", selected.saturating_add(1), indices.len())
    } else {
        String::new()
    };
    let filter = if picker.query.is_empty() {
        "type to filter".to_owned()
    } else {
        format!("filter: {}", picker.query)
    };
    let heading_budget = usize::from(area.width).saturating_sub(position.width() + 4);
    let title = clip_words(&picker.title, heading_budget);
    let filter = clip_words(&filter, heading_budget.saturating_sub(title.width() + 3));
    let filter = if filter.is_empty() {
        filter
    } else {
        format!(" · {filter}")
    };
    let heading_width = 2 + title.width() + filter.width();
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("  {title}"), theme.style(Tone::Heading)),
        Span::styled(filter, theme.style(Tone::Dim)),
        Span::styled(
            format!(
                "{}{position}",
                " ".repeat(
                    usize::from(area.width).saturating_sub(heading_width + position.width())
                )
            ),
            theme.style(Tone::Dim),
        ),
    ])];
    lines.extend(picker_entry_lines(
        picker,
        usize::from(area.height).saturating_sub(1),
        area.width,
        theme,
    ));
    // Choices and the selected detail have separate reserved rows. Wrapping
    // them here would make the compact pane grow into the composer.
    frame.render_widget(Paragraph::new(lines), area);
}

/// Rows requested by the compact runtime picker before terminal constraints.
pub(crate) fn compact_resource_picker_rows(picker: &ResourcePicker) -> u16 {
    let matches = picker.filtered_indices().len().max(1);
    let detail = picker
        .selected_entry()
        .is_some_and(|entry| !entry.selected_detail().is_empty());
    u16::try_from(matches.min(COMPACT_VISIBLE_ENTRIES) + 1 + usize::from(detail))
        .unwrap_or(u16::MAX)
}

fn picker_lines(
    picker: &ResourcePicker,
    height: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
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
    lines.extend(picker_entry_lines(
        picker,
        height.saturating_sub(1),
        width,
        theme,
    ));
    lines
}

fn picker_entry_lines(
    picker: &ResourcePicker,
    height: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let indices = picker.filtered_indices();
    if indices.is_empty() {
        let guidance = if picker.entries.is_empty() {
            picker.empty_guidance.clone()
        } else {
            "No matches · Ctrl+U clear filter".to_owned()
        };
        lines.push(Line::from(Span::styled(
            format!(" {guidance}"),
            theme.style(Tone::Warning),
        )));
    } else {
        let detail = picker
            .selected_entry()
            .map(|entry| entry.selected_detail())
            .unwrap_or_default();
        let capacity = height
            .saturating_sub(usize::from(!detail.is_empty()))
            .max(1);
        let selected = picker.selected.min(indices.len().saturating_sub(1));
        let start = selected
            .saturating_sub(capacity / 2)
            .min(indices.len().saturating_sub(capacity));
        let visible = indices
            .iter()
            .skip(start)
            .take(capacity)
            .map(|index| &picker.entries[*index])
            .collect::<Vec<_>>();
        let name_width = visible
            .iter()
            .map(|entry| entry.label.width())
            .max()
            .unwrap_or(0);
        // Keep the state docked even when an identity or unavailable reason is
        // too long. The complete reason remains on the selected detail line.
        let states = visible
            .iter()
            .map(|entry| {
                let current = if entry.active { "✓ current" } else { "" };
                match &entry.disabled_reason {
                    Some(reason) => {
                        let prefix = if current.is_empty() {
                            String::new()
                        } else {
                            format!("{current} · ")
                        };
                        let full = format!("{prefix}unavailable: {reason}");
                        if 2 + name_width + 2 + full.width() <= usize::from(width) {
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
        let name_width = name_width
            .min(usize::from(width).saturating_sub(4 + state_width))
            .min(usize::from(width) / 2);
        for (offset, entry) in visible.iter().enumerate() {
            let filtered_index = start + offset;
            let tone = if entry.disabled_reason.is_some() {
                Tone::Dim
            } else if filtered_index == selected {
                Tone::Accent
            } else {
                Tone::Default
            };
            lines.push(list_row(
                &entry.label,
                &entry.description,
                &states[offset],
                filtered_index == selected,
                name_width,
                width,
                tone,
                theme,
            ));
            if filtered_index == selected && !detail.is_empty() && height > capacity {
                lines.push(detail_line(&detail, name_width, width, theme));
            }
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
            picker.query = query.to_owned();
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
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(empty_text.contains("run smith setup add-model"));
        assert!(!empty_text.contains("No matches"));

        let mut filtered = ResourcePicker::new(
            "Models",
            vec![ResourceEntry::new("local/model", "local/model", "local")],
            "No local model is selectable · run smith setup add-model",
        );
        filtered.query = "does-not-exist".to_owned();
        filtered.selected = 4;
        let filtered_lines = picker_lines(
            &filtered,
            3,
            44,
            Theme::from_env().without_color().without_motion(),
        );
        let filtered_text = filtered_lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
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

        let mut terminal = Terminal::new(TestBackend::new(44, 10)).expect("terminal");
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

        picker.query = "broken".to_owned();
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
                .any(|line| line.contains("broken") && line.ends_with("unavailable│")),
            "{rendered}"
        );
        assert!(!rendered.contains("missing limits"), "{rendered}");
        assert!(rendered.contains("Enter confirm"), "{rendered}");
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
            80,
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
