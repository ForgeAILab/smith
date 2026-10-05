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

mod rendering;

pub(crate) use rendering::{
    PickerContext, compact_resource_picker_rows, draw_compact_resource_picker, draw_picker_context,
    draw_picker_with_context, indented_words, picker_content_height,
};
pub use rendering::{ScreenFooter, draw_inline_screen, draw_inline_surface, draw_resource_picker};

#[cfg(test)]
use rendering::{entry_view, picker_entry_lines, picker_lines};
#[cfg(test)]
mod tests;
