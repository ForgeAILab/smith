//! Character-indexed editing shared by every text field so fixes have one home.

use std::fmt;
use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_width::UnicodeWidthStr;

/// An edit independent of a field's navigation and submission keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEdit {
    /// Insert at the cursor so typing can correct the middle of a value.
    Insert(char),
    /// Move by one character without crossing a UTF-8 byte boundary.
    Left,
    /// Move by one character, bounded by the stored text.
    Right,
    /// Reach the current line start without deleting its newline.
    Home,
    /// Reach the current line end without entering the next line.
    End,
    /// Remove the preceding character or atomic composer placeholder.
    Backspace,
    /// Remove the following character or atomic composer placeholder.
    Delete,
    /// Skip preceding whitespace and the preceding word.
    WordLeft,
    /// Skip following whitespace and the following word.
    WordRight,
    /// Use the same word boundary as backward word movement.
    DeleteWordLeft,
    /// Preserve a multiline draft while clearing its line prefix.
    DeleteToStart,
    /// Preserve a multiline draft while clearing its line suffix.
    DeleteToEnd,
}

/// One mapping keeps slash drafts, filters, and secret fields in agreement.
pub fn line_edit(key: KeyEvent) -> Option<LineEdit> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    Some(match (key.code, key.modifiers) {
        (KeyCode::Left, _) => LineEdit::Left,
        (KeyCode::Right, _) => LineEdit::Right,
        (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => LineEdit::Home,
        (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => LineEdit::End,
        (KeyCode::Backspace, _) => LineEdit::Backspace,
        (KeyCode::Delete, _) => LineEdit::Delete,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => LineEdit::DeleteToStart,
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => LineEdit::DeleteToEnd,
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => LineEdit::DeleteWordLeft,
        (KeyCode::Char('b'), KeyModifiers::ALT) => LineEdit::WordLeft,
        (KeyCode::Char('f'), KeyModifiers::ALT) => LineEdit::WordRight,
        (KeyCode::Char(ch), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
            LineEdit::Insert(ch)
        }
        _ => return None,
    })
}

/// Text and its character cursor; masking is presentation only, never stored text.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    text: String,
    cursor: usize,
    masked: bool,
}

impl fmt::Debug for LineInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LineInput")
            .field(
                "text",
                &if self.masked {
                    "[redacted]"
                } else {
                    &self.text
                },
            )
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl From<String> for LineInput {
    fn from(text: String) -> Self {
        let mut input = Self::default();
        input.replace(text);
        input
    }
}

impl From<&str> for LineInput {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

impl PartialEq<&str> for LineInput {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl LineInput {
    /// Secrets share editing while keeping debug output and display redacted.
    pub fn masked() -> Self {
        Self {
            masked: true,
            ..Self::default()
        }
    }
    /// Stored text stays separate from masking for submission.
    pub fn text(&self) -> &str {
        &self.text
    }
    /// Character offsets let every field share Unicode-safe edits.
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    /// Field owners use emptiness for placeholders and validation.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    /// Reset text and cursor together so navigation cannot retain an invalid cursor.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }
    /// Restored non-secret values start with the cursor at their end.
    pub fn replace(&mut self, text: impl Into<String>) {
        self.text = text.into().chars().filter(|ch| !ch.is_control()).collect();
        self.cursor = self.text.chars().count();
    }
    /// Paste inserts at the cursor, dropping controls so it cannot submit a form.
    pub fn paste(&mut self, text: &str) {
        let clean: String = text.chars().filter(|ch| !ch.is_control()).collect();
        insert_text(&mut self.text, &mut self.cursor, &clean);
    }
    /// Report recognized edits so field owners can refresh matching choices.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        if let Some(edit) = line_edit(key) {
            self.edit(edit);
            true
        } else {
            false
        }
    }
    /// Apply a mapped edit without duplicating a field-specific key table.
    pub fn edit(&mut self, edit: LineEdit) {
        if !matches!(edit, LineEdit::Insert(ch) if ch.is_control()) {
            edit_text(&mut self.text, &mut self.cursor, edit, &[], 0);
        }
    }
    /// Presentation masks every stored character while retaining the real value.
    pub fn display_text(&self) -> String {
        if self.masked {
            "•".repeat(self.text.chars().count())
        } else {
            self.text.clone()
        }
    }
    /// Display columns account for wide characters in plain fields.
    pub fn display_cursor(&self) -> usize {
        if self.masked {
            self.cursor
        } else {
            self.text
                .chars()
                .take(self.cursor)
                .collect::<String>()
                .width()
        }
    }
    /// A horizontal window keeps the insertion point visible in narrow fields.
    pub fn viewport(&self, width: usize) -> (String, usize) {
        let display = self.display_text();
        let mut column = self.display_cursor();
        let mut chars = display.chars();
        while column >= width.max(1) {
            let Some(ch) = chars.next() else {
                break;
            };
            column = column.saturating_sub(ch.to_string().width());
        }
        let mut visible = String::new();
        for ch in chars {
            if visible.width() + ch.to_string().width() > width {
                break;
            }
            visible.push(ch);
        }
        (visible, column.min(width.saturating_sub(1)))
    }
}

fn byte_offset(text: &str, cursor: usize) -> usize {
    text.char_indices()
        .nth(cursor)
        .map_or(text.len(), |(offset, _)| offset)
}

pub(crate) fn insert_text(text: &mut String, cursor: &mut usize, value: &str) {
    text.insert_str(byte_offset(text, *cursor), value);
    *cursor += value.chars().count();
}

/// Atomic ranges and the shell prompt belong to the composer; editing stays shared.
pub(crate) fn edit_text(
    text: &mut String,
    cursor: &mut usize,
    edit: LineEdit,
    atomic: &[Range<usize>],
    protected: usize,
) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let old = *cursor;
    let mut index = old;
    let mut remove = None;
    match edit {
        LineEdit::Insert(ch) => {
            insert_text(text, cursor, &ch.to_string());
            return true;
        }
        LineEdit::Left | LineEdit::Backspace => {
            index = atomic
                .iter()
                .find(|r| r.start < old && old <= r.end)
                .map_or(old.saturating_sub(1), |r| r.start);
            if edit == LineEdit::Backspace && old > 0 {
                remove = Some(
                    atomic
                        .iter()
                        .find(|r| r.start < old && old <= r.end)
                        .cloned()
                        .unwrap_or(index..old),
                );
            }
        }
        LineEdit::Right | LineEdit::Delete => {
            index = atomic
                .iter()
                .find(|r| r.start <= old && old < r.end)
                .map_or((old + 1).min(chars.len()), |r| r.end);
            if edit == LineEdit::Delete {
                if old < chars.len() {
                    remove = Some(
                        atomic
                            .iter()
                            .find(|range| range.start <= old && old < range.end)
                            .cloned()
                            .unwrap_or(old..index),
                    );
                }
                index = old;
            }
        }
        LineEdit::Home | LineEdit::DeleteToStart => {
            while index > 0 && chars[index - 1] != '\n' {
                index -= 1;
            }
            index = index.max(protected);
            if edit == LineEdit::DeleteToStart && index < old {
                remove = Some(index..old);
            }
        }
        LineEdit::End | LineEdit::DeleteToEnd => {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            if edit == LineEdit::DeleteToEnd {
                if old < index {
                    remove = Some(old..index);
                }
                index = old;
            }
        }
        LineEdit::WordLeft | LineEdit::DeleteWordLeft => {
            while index > 0 && chars[index - 1].is_whitespace() {
                index -= 1;
            }
            if let Some(range) = atomic.iter().find(|r| r.start < index && index <= r.end) {
                index = range.start;
            } else {
                while index > 0 && !chars[index - 1].is_whitespace() {
                    if atomic.iter().any(|r| r.end == index) {
                        break;
                    }
                    index -= 1;
                }
            }
            if edit == LineEdit::DeleteWordLeft && index < old {
                remove = Some(index..old);
            }
        }
        LineEdit::WordRight => {
            while index < chars.len() && chars[index].is_whitespace() {
                index += 1;
            }
            if let Some(range) = atomic.iter().find(|r| r.start <= index && index < r.end) {
                index = range.end;
            } else {
                while index < chars.len() && !chars[index].is_whitespace() {
                    if atomic.iter().any(|r| r.start == index) {
                        break;
                    }
                    index += 1;
                }
            }
        }
    }
    let changed = remove.is_some();
    if let Some(range) = remove {
        text.replace_range(
            byte_offset(text, range.start)..byte_offset(text, range.end),
            "",
        );
        index = range.start;
    }
    *cursor = index;
    changed
}
