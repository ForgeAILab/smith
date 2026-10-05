//! The input composer.
//!
//! A small multi-line buffer with a character cursor. It indexes by `char`
//! rather than by byte, so a cursor never lands inside a multi-byte codepoint
//! and panics the renderer — which is exactly what happens the first time
//! someone types an accented character into a byte-indexed buffer.

use std::collections::VecDeque;
use std::ops::Range;

use crate::line_input::{LineEdit, edit_text, insert_text};
use unicode_width::UnicodeWidthChar;

/// Composer history is intentionally bounded, process-local UI state.
const MAX_HISTORY_ENTRIES: usize = 100;

/// A multi-line text buffer with a cursor.
#[derive(Debug, Clone, Default)]
pub struct Composer {
    text: String,
    /// Cursor position, counted in characters from the start.
    cursor: usize,
    /// Retain the intended display column across shorter draft lines.
    vertical_column: Option<usize>,
    /// Accepted inputs and interrupted drafts, oldest first.
    history: VecDeque<String>,
    /// Entry currently selected while navigating [`Self::history`].
    history_cursor: Option<usize>,
    /// Draft to restore after navigating beyond the newest history entry.
    history_scratch: Option<String>,
}

impl Composer {
    /// An empty composer.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The leading shell shortcut character is drawn as the prompt itself.
    pub fn is_bash_mode(&self) -> bool {
        self.text.starts_with('!')
    }

    /// Editable text after the prompt; submission still uses [`Self::text`].
    /// An escaped `!!x` is displayed as `! !x` and still submits literal `!x`.
    pub fn visible_text(&self) -> &str {
        self.text.strip_prefix('!').unwrap_or(&self.text)
    }

    /// Cursor coordinates in the visible draft, excluding the shell prompt.
    pub fn visible_cursor_position(&self) -> (usize, usize) {
        let (line, column) = self.cursor_position();
        (
            line,
            column.saturating_sub(usize::from(line == 0 && self.is_bash_mode())),
        )
    }

    /// Maps a visible draft position back to the stored character cursor.
    pub fn move_to_visible_position(&mut self, line: usize, column: usize) {
        self.move_to_position(
            line,
            column.saturating_add(usize::from(line == 0 && self.is_bash_mode())),
        );
    }

    /// Whether the buffer holds nothing but whitespace.
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// The cursor position in characters.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The number of characters held.
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// Whether the buffer is completely empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Character ranges occupied by registered placeholder labels.
    ///
    /// The composer remains a plain string; callers provide only labels that
    /// still own out-of-band material. Returning character offsets keeps every
    /// later edit on the same Unicode-safe coordinate system as the cursor.
    pub fn registered_ranges<'a>(
        &self,
        placeholders: impl IntoIterator<Item = &'a str>,
    ) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        for placeholder in placeholders {
            if placeholder.is_empty() {
                continue;
            }
            let length = placeholder.chars().count();
            for (byte_start, _) in self.text.match_indices(placeholder) {
                let start = self.text[..byte_start].chars().count();
                ranges.push(start..start + length);
            }
        }
        ranges.sort_unstable_by_key(|range| (range.start, range.end));
        ranges.dedup();
        ranges
    }

    /// Shared edits retain atomic attachments and the shell prompt's protected start.
    pub fn edit_line(&mut self, edit: LineEdit, ranges: &[Range<usize>]) {
        self.vertical_column = None;
        let protected = usize::from(self.is_bash_mode());
        if edit_text(&mut self.text, &mut self.cursor, edit, ranges, protected) {
            self.leave_history_navigation();
        }
    }

    /// Inserts a character at the cursor.
    pub fn insert(&mut self, ch: char) {
        self.edit_line(LineEdit::Insert(ch), &[]);
    }

    /// Inserts a string at the cursor, as a paste would.
    pub fn insert_str(&mut self, value: &str) {
        self.vertical_column = None;
        self.leave_history_navigation();
        insert_text(&mut self.text, &mut self.cursor, value);
    }

    /// Deletes the character before the cursor.
    pub fn backspace(&mut self) {
        self.edit_line(LineEdit::Backspace, &[]);
    }

    /// Deletes the character or registered atomic range before the cursor.
    pub fn backspace_over(&mut self, atomic_ranges: &[Range<usize>]) {
        self.edit_line(LineEdit::Backspace, atomic_ranges);
    }

    /// Deletes the character at the cursor.
    pub fn delete(&mut self) {
        self.edit_line(LineEdit::Delete, &[]);
    }

    /// Deletes the character or registered atomic range after the cursor.
    pub fn delete_over(&mut self, atomic_ranges: &[Range<usize>]) {
        self.edit_line(LineEdit::Delete, atomic_ranges);
    }

    /// Moves the cursor one character left.
    pub fn move_left(&mut self) {
        self.edit_line(LineEdit::Left, &[]);
    }

    /// Moves left by one character or across one registered atomic range.
    pub fn move_left_over(&mut self, atomic_ranges: &[Range<usize>]) {
        self.edit_line(LineEdit::Left, atomic_ranges);
    }

    /// Moves the cursor one character right.
    pub fn move_right(&mut self) {
        self.edit_line(LineEdit::Right, &[]);
    }

    /// Moves right by one character or across one registered atomic range.
    pub fn move_right_over(&mut self, atomic_ranges: &[Range<usize>]) {
        self.edit_line(LineEdit::Right, atomic_ranges);
    }

    /// Moves the cursor to the start of the current line.
    pub fn move_home(&mut self) {
        self.edit_line(LineEdit::Home, &[]);
    }

    /// Moves the cursor to the end of the current line.
    pub fn move_end(&mut self) {
        self.edit_line(LineEdit::End, &[]);
    }

    /// Moves to the start of the whole draft.
    pub fn move_to_start(&mut self) {
        self.vertical_column = None;
        self.cursor = usize::from(self.is_bash_mode());
    }

    /// Moves to the end of the whole draft.
    pub fn move_to_end(&mut self) {
        self.vertical_column = None;
        self.cursor = self.len();
    }

    /// Moves up a draft line, returning false only on the first line.
    pub fn move_up_over(&mut self, atomic_ranges: &[Range<usize>]) -> bool {
        let (line, column) = self.visible_cursor_position();
        let Some(previous) = line.checked_sub(1) else {
            return false;
        };
        self.move_vertical(previous, column, atomic_ranges);
        true
    }

    /// Moves down a draft line, returning false only on the last line.
    pub fn move_down_over(&mut self, atomic_ranges: &[Range<usize>]) -> bool {
        let (line, column) = self.visible_cursor_position();
        if line + 1 >= self.lines().len() {
            return false;
        }
        self.move_vertical(line + 1, column, atomic_ranges);
        true
    }

    fn move_vertical(&mut self, line: usize, column: usize, atomic_ranges: &[Range<usize>]) {
        let column = self.vertical_column.unwrap_or(column);
        self.move_to_visible_position(line, column);
        if let Some(range) = atomic_ranges
            .iter()
            .find(|range| range.start < self.cursor && self.cursor < range.end)
        {
            self.cursor = if self.cursor - range.start <= range.end - self.cursor {
                range.start
            } else {
                range.end
            };
        }
        self.vertical_column = Some(column);
    }

    /// Moves left over whitespace and the preceding word or registered label.
    pub fn move_word_left_over(&mut self, atomic_ranges: &[Range<usize>]) {
        self.edit_line(LineEdit::WordLeft, atomic_ranges);
    }

    /// Moves right over whitespace and the next word or registered label.
    pub fn move_word_right_over(&mut self, atomic_ranges: &[Range<usize>]) {
        self.edit_line(LineEdit::WordRight, atomic_ranges);
    }

    /// Deletes the word or registered label to the left of the cursor.
    pub fn delete_word_left_over(&mut self, atomic_ranges: &[Range<usize>]) {
        self.edit_line(LineEdit::DeleteWordLeft, atomic_ranges);
    }

    /// Deletes to the current line's start without removing its newline.
    pub fn delete_to_line_start(&mut self) {
        self.edit_line(LineEdit::DeleteToStart, &[]);
    }

    /// Deletes to the current line's end without removing its newline.
    pub fn delete_to_line_end(&mut self) {
        self.edit_line(LineEdit::DeleteToEnd, &[]);
    }

    /// Moves the cursor to a `(line, column)` position, both zero-based and
    /// counted in display columns, clamping to the nearest real position.
    ///
    /// This is the mouse-click inverse of [`Self::cursor_position`]: a click
    /// past the end of a line lands at that line's end, and a click below the
    /// last line lands at the end of the buffer.
    pub fn move_to_position(&mut self, line: usize, column: usize) {
        self.vertical_column = None;
        let mut index = 0usize;
        let mut current_line = 0usize;
        let mut chars = self.text.chars();
        while current_line < line {
            match chars.next() {
                Some('\n') => {
                    current_line += 1;
                    index += 1;
                }
                Some(_) => index += 1,
                None => {
                    self.cursor = index;
                    return;
                }
            }
        }
        let mut current_col = 0usize;
        for character in chars {
            if character == '\n' || current_col >= column {
                break;
            }
            let char_width = UnicodeWidthChar::width(character).unwrap_or(0);
            current_col += char_width;
            index += 1;
        }
        self.cursor = index;
    }

    /// Empties the buffer.
    pub fn clear(&mut self) {
        self.vertical_column = None;
        self.text.clear();
        self.cursor = 0;
        self.leave_history_navigation();
    }

    /// Replaces the draft and leaves the cursor at its end.
    pub fn replace(&mut self, value: impl Into<String>) {
        self.vertical_column = None;
        self.text = value.into();
        self.cursor = self.text.chars().count();
        self.leave_history_navigation();
    }

    /// Records the exact current input in bounded local history.
    pub fn record_current(&mut self) -> bool {
        self.record_history(self.text.clone())
    }

    /// Clears the current draft and keeps it in bounded local history.
    pub fn stash_for_recall(&mut self) {
        self.vertical_column = None;
        let draft = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.record_history(draft);
    }

    /// Recalls the previous composer-history entry.
    pub fn recall_previous(&mut self) -> bool {
        self.vertical_column = None;
        let Some(last) = self.history.len().checked_sub(1) else {
            return false;
        };
        let index = match self.history_cursor {
            Some(current) => current.saturating_sub(1),
            None => {
                self.history_scratch = Some(self.text.clone());
                last
            }
        };
        self.history_cursor = Some(index);
        self.text.clone_from(&self.history[index]);
        self.cursor = self.text.chars().count();
        true
    }

    /// Moves toward newer history and restores the pre-navigation draft.
    pub fn recall_next(&mut self) -> bool {
        self.vertical_column = None;
        let Some(current) = self.history_cursor else {
            return false;
        };
        if current + 1 < self.history.len() {
            let index = current + 1;
            self.history_cursor = Some(index);
            self.text.clone_from(&self.history[index]);
            self.cursor = self.text.chars().count();
        } else {
            self.text = self.history_scratch.take().unwrap_or_default();
            self.cursor = self.text.chars().count();
            self.history_cursor = None;
        }
        true
    }

    /// Finds a case-insensitive substring match, newest first.
    ///
    /// When `after` identifies the current match, the next older match is
    /// returned and the search wraps after the oldest match.
    pub fn search_history(&self, query: &str, after: Option<usize>) -> Option<(usize, String)> {
        if query.is_empty() {
            return None;
        }
        let query = query.to_lowercase();
        let matches = self
            .history
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, entry)| {
                entry
                    .to_lowercase()
                    .contains(&query)
                    .then_some((index, entry))
            })
            .collect::<Vec<_>>();
        let selected = after
            .and_then(|current| {
                matches
                    .iter()
                    .position(|(index, _)| *index == current)
                    .map(|position| (position + 1) % matches.len())
            })
            .unwrap_or(0);
        matches
            .get(selected)
            .map(|(index, entry)| (*index, (*entry).clone()))
    }

    /// Whether a composer-history entry is currently selected.
    pub fn is_recalling(&self) -> bool {
        self.history_cursor.is_some()
    }

    fn record_history(&mut self, entry: String) -> bool {
        self.leave_history_navigation();
        if entry.trim().is_empty() || self.history.back() == Some(&entry) {
            return false;
        }
        if self.history.len() == MAX_HISTORY_ENTRIES {
            self.history.pop_front();
        }
        self.history.push_back(entry);
        true
    }

    fn leave_history_navigation(&mut self) {
        self.history_cursor = None;
        self.history_scratch = None;
    }

    /// The buffer split into display lines.
    pub fn lines(&self) -> Vec<&str> {
        self.text.split('\n').collect()
    }

    /// The cursor as a `(line, column)` pair, both zero-based and counted in
    /// display columns (taking character display width into account).
    pub fn cursor_position(&self) -> (usize, usize) {
        let mut line = 0;
        let mut column = 0;
        for (index, ch) in self.text.chars().enumerate() {
            if index == self.cursor {
                return (line, column);
            }
            if ch == '\n' {
                line += 1;
                column = 0;
            } else {
                column += UnicodeWidthChar::width(ch).unwrap_or(0);
            }
        }
        (line, column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_from_inside_an_atomic_label_removes_the_whole_label() {
        for edit in [LineEdit::Backspace, LineEdit::Delete] {
            let mut composer = Composer::new();
            composer.replace("a [chunk] z");
            let ranges = composer.registered_ranges(["[chunk]"]);
            composer.cursor = 5;
            composer.edit_line(edit, &ranges);
            assert_eq!(composer.text(), "a  z");
            assert_eq!(composer.cursor(), 2);
        }
    }

    #[test]
    fn typing_advances_the_cursor() {
        let mut composer = Composer::new();
        for ch in "fix".chars() {
            composer.insert(ch);
        }
        assert_eq!(composer.text(), "fix");
        assert_eq!(composer.cursor(), 3);
    }

    #[test]
    fn editing_multibyte_text_stays_on_character_boundaries() {
        let mut composer = Composer::new();
        composer.insert_str("café ☕");
        assert_eq!(composer.cursor(), 6);

        composer.backspace();
        assert_eq!(composer.text(), "café ");

        composer.move_left();
        composer.move_left();
        composer.insert('!');
        assert_eq!(composer.text(), "caf!é ");
    }

    #[test]
    fn registered_ranges_use_character_offsets_and_ignore_other_text() {
        let mut composer = Composer::new();
        let paste = "[Pasted text #1 +3 lines]";
        let image = "[Image #1 32×32]";
        composer.insert_str(&format!("é{paste}{image}[Image #9]終"));

        let paste_start = 1;
        let image_start = paste_start + paste.chars().count();
        assert_eq!(
            composer.registered_ranges([paste, image]),
            [
                paste_start..image_start,
                image_start..image_start + image.chars().count()
            ]
        );
    }

    #[test]
    fn horizontal_movement_crosses_adjacent_registered_ranges() {
        let mut composer = Composer::new();
        let paste = "[Pasted text #1 +3 lines]";
        let image = "[Image #1 32×32]";
        composer.insert_str(&format!("{paste}{image}"));
        let ranges = composer.registered_ranges([paste, image]);

        composer.move_left_over(&ranges);
        assert_eq!(composer.cursor(), paste.chars().count());
        composer.move_left_over(&ranges);
        assert_eq!(composer.cursor(), 0);
        composer.move_right_over(&ranges);
        assert_eq!(composer.cursor(), paste.chars().count());
        composer.move_right_over(&ranges);
        assert_eq!(composer.cursor(), composer.len());
    }

    #[test]
    fn deletion_removes_adjacent_registered_ranges_in_key_direction() {
        let mut composer = Composer::new();
        let paste = "[Pasted text #1 +3 lines]";
        let image = "[Image #1 32×32]";
        composer.insert_str(&format!("{paste}{image}"));

        let ranges = composer.registered_ranges([paste, image]);
        composer.backspace_over(&ranges);
        assert_eq!(composer.text(), paste);
        assert_eq!(composer.cursor(), paste.chars().count());
        let ranges = composer.registered_ranges([paste, image]);
        composer.backspace_over(&ranges);
        assert!(composer.is_empty());

        composer.insert_str(&format!("{paste}{image}"));
        composer.move_home();
        let ranges = composer.registered_ranges([paste, image]);
        composer.delete_over(&ranges);
        assert_eq!(composer.text(), image);
        assert_eq!(composer.cursor(), 0);
        let ranges = composer.registered_ranges([paste, image]);
        composer.delete_over(&ranges);
        assert!(composer.is_empty());
    }

    #[test]
    fn the_cursor_cannot_leave_the_buffer() {
        let mut composer = Composer::new();
        composer.move_left();
        composer.backspace();
        assert_eq!(composer.cursor(), 0);
        assert!(composer.is_empty());

        composer.insert_str("hi");
        composer.move_right();
        composer.move_right();
        composer.move_right();
        assert_eq!(composer.cursor(), 2);
        composer.delete();
        assert_eq!(composer.text(), "hi");
    }

    #[test]
    fn positional_moves_clamp_to_real_lines_and_columns() {
        let mut composer = Composer::new();
        composer.insert_str("first\nsecond café");

        composer.move_to_position(0, 2);
        assert_eq!(composer.cursor_position(), (0, 2));

        // Past the end of a line lands at that line's end.
        composer.move_to_position(0, 99);
        assert_eq!(composer.cursor_position(), (0, 5));

        composer.move_to_position(1, 8);
        assert_eq!(composer.cursor_position(), (1, 8));
        composer.insert('!');
        assert_eq!(composer.text(), "first\nsecond c!afé");

        // Below the last line lands at the end of the buffer.
        composer.move_to_position(9, 0);
        assert_eq!(composer.cursor(), composer.len());
    }

    #[test]
    fn home_and_end_stay_within_the_current_line() {
        let mut composer = Composer::new();
        composer.insert_str("first\nsecond");
        composer.move_home();
        assert_eq!(composer.cursor(), 6);
        composer.move_end();
        assert_eq!(composer.cursor(), 12);
    }

    #[test]
    fn vertical_moves_keep_the_display_column_across_short_and_empty_lines() {
        let mut composer = Composer::new();
        composer.replace("abcdef\n中\nabcdef\n");
        composer.move_to_position(0, 4);
        assert!(!composer.move_up_over(&[]));
        assert!(composer.move_down_over(&[]));
        assert_eq!(composer.cursor_position(), (1, 2));
        assert!(composer.move_down_over(&[]));
        assert_eq!(composer.cursor_position(), (2, 4));
        assert!(composer.move_down_over(&[]));
        assert_eq!(composer.cursor_position(), (3, 0));
        assert!(!composer.move_down_over(&[]));
        assert!(composer.move_up_over(&[]));
        assert_eq!(composer.cursor_position(), (2, 4));
        assert!(composer.move_up_over(&[]));
        composer.move_home();
        assert!(composer.move_down_over(&[]));
        assert_eq!(composer.cursor_position(), (2, 0));
    }

    #[test]
    fn vertical_moves_snap_to_registered_placeholder_edges() {
        let mut composer = Composer::new();
        let label = "[Image #1 32×32]";
        composer.replace(format!("abcdefghijklmno\n{label}\nabcdefghijklmno"));
        let ranges = composer.registered_ranges([label]);
        composer.move_to_position(0, 4);
        composer.move_down_over(&ranges);
        assert_eq!(composer.cursor(), ranges[0].start);
        composer.move_down_over(&ranges);
        assert_eq!(composer.cursor_position(), (2, 4));
        composer.move_to_position(2, 13);
        composer.move_up_over(&ranges);
        assert_eq!(composer.cursor(), ranges[0].end);
    }

    #[test]
    fn word_moves_and_deletion_stop_at_labels_adjacent_to_ordinary_text() {
        let mut composer = Composer::new();
        let label = "[Pasted text #1 +3 lines]";
        composer.replace(format!("café{label}tail  "));
        let ranges = composer.registered_ranges([label]);
        composer.move_word_left_over(&ranges);
        assert_eq!(composer.cursor(), ranges[0].end);
        composer.move_word_left_over(&ranges);
        assert_eq!(composer.cursor(), ranges[0].start);
        composer.move_word_left_over(&ranges);
        assert_eq!(composer.cursor(), 0);
        composer.move_word_right_over(&ranges);
        assert_eq!(composer.cursor(), ranges[0].start);
        composer.move_word_right_over(&ranges);
        assert_eq!(composer.cursor(), ranges[0].end);
        composer.delete_word_left_over(&ranges);
        assert_eq!(composer.text(), "cafétail  ");
        assert_eq!(composer.cursor(), 4);
    }

    #[test]
    fn cursor_position_tracks_lines_and_columns() {
        let mut composer = Composer::new();
        composer.insert_str("ab\ncd");
        assert_eq!(composer.cursor_position(), (1, 2));
        composer.move_home();
        assert_eq!(composer.cursor_position(), (1, 0));
    }

    #[test]
    fn cursor_position_counts_display_width_for_wide_characters() {
        let mut composer = Composer::new();
        composer.insert_str("中文测试");
        assert_eq!(composer.cursor_position(), (0, 8));

        composer.move_home();
        assert_eq!(composer.cursor_position(), (0, 0));

        composer.move_right();
        assert_eq!(composer.cursor_position(), (0, 2));

        composer.move_right();
        assert_eq!(composer.cursor_position(), (0, 4));

        composer.move_to_position(0, 6);
        assert_eq!(composer.cursor(), 3);
        assert_eq!(composer.cursor_position(), (0, 6));
    }

    #[test]
    fn whitespace_only_input_counts_as_blank() {
        let mut composer = Composer::new();
        composer.insert_str("  \n ");
        assert!(composer.is_blank());
        assert!(!composer.is_empty());
    }

    #[test]
    fn accepted_input_and_stashed_drafts_share_bounded_exact_history() {
        let mut composer = Composer::new();
        composer.insert_str("first café\nline");
        assert!(composer.record_current());
        composer.clear();
        composer.insert_str("first café\nline");
        assert!(!composer.record_current());
        composer.clear();
        composer.stash_for_recall();
        composer.insert_str("second\ndraft");
        composer.stash_for_recall();

        assert!(composer.is_empty());
        assert!(composer.recall_previous());
        assert_eq!(composer.text(), "second\ndraft");
        assert!(composer.recall_previous());
        assert_eq!(composer.text(), "first café\nline");
        assert!(composer.recall_next());
        assert_eq!(composer.text(), "second\ndraft");
        assert!(composer.recall_next());
        assert!(composer.is_empty());
        assert!(!composer.is_recalling());
    }

    #[test]
    fn editing_a_recalled_draft_leaves_history_navigation() {
        let mut composer = Composer::new();
        composer.insert_str("recover me");
        composer.stash_for_recall();
        assert!(composer.recall_previous());

        composer.insert('!');
        assert_eq!(composer.text(), "recover me!");
        assert!(!composer.is_recalling());
        assert!(!composer.recall_next());
    }

    #[test]
    fn navigation_restores_the_non_empty_scratch_draft() {
        let mut composer = Composer::new();
        composer.insert_str("first");
        composer.record_current();
        composer.replace("second");
        composer.record_current();
        composer.replace("work in progress");

        assert!(composer.recall_previous());
        assert_eq!(composer.text(), "second");
        assert!(composer.recall_previous());
        assert_eq!(composer.text(), "first");
        assert!(composer.recall_next());
        assert_eq!(composer.text(), "second");
        assert!(composer.recall_next());
        assert_eq!(composer.text(), "work in progress");
        assert!(!composer.is_recalling());
    }

    #[test]
    fn history_capacity_drops_the_oldest_entry() {
        let mut composer = Composer::new();
        for index in 0..=MAX_HISTORY_ENTRIES {
            composer.replace(format!("entry {index}"));
            assert!(composer.record_current());
        }

        assert_eq!(composer.history.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(
            composer.history.front().map(String::as_str),
            Some("entry 1")
        );
        assert_eq!(
            composer.history.back().map(String::as_str),
            Some("entry 100")
        );
    }

    #[test]
    fn reverse_search_is_case_insensitive_newest_first_and_wraps() {
        let mut composer = Composer::new();
        for entry in ["Fix CAFÉ", "unrelated", "fix tests", "prefix café suffix"] {
            composer.replace(entry);
            composer.record_current();
        }

        let newest = composer.search_history("CAFÉ", None).unwrap();
        assert_eq!(newest.1, "prefix café suffix");
        let older = composer.search_history("café", Some(newest.0)).unwrap();
        assert_eq!(older.1, "Fix CAFÉ");
        let wrapped = composer.search_history("café", Some(older.0)).unwrap();
        assert_eq!(wrapped, newest);

        assert_eq!(composer.search_history("missing", None), None);
        assert_eq!(composer.search_history("", None), None);
    }
}
