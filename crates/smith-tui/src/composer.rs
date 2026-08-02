//! The input composer.
//!
//! A small multi-line buffer with a character cursor. It indexes by `char`
//! rather than by byte, so a cursor never lands inside a multi-byte codepoint
//! and panics the renderer — which is exactly what happens the first time
//! someone types an accented character into a byte-indexed buffer.

use std::collections::VecDeque;

/// Composer history is intentionally bounded, process-local UI state.
const MAX_HISTORY_ENTRIES: usize = 100;

/// A multi-line text buffer with a cursor.
#[derive(Debug, Clone, Default)]
pub struct Composer {
    text: String,
    /// Cursor position, counted in characters from the start.
    cursor: usize,
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

    fn byte_offset(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map_or(self.text.len(), |(offset, _)| offset)
    }

    /// Inserts a character at the cursor.
    pub fn insert(&mut self, ch: char) {
        self.leave_history_navigation();
        let offset = self.byte_offset(self.cursor);
        self.text.insert(offset, ch);
        self.cursor += 1;
    }

    /// Inserts a string at the cursor, as a paste would.
    pub fn insert_str(&mut self, value: &str) {
        self.leave_history_navigation();
        let offset = self.byte_offset(self.cursor);
        self.text.insert_str(offset, value);
        self.cursor += value.chars().count();
    }

    /// Deletes the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let offset = self.byte_offset(self.cursor - 1);
        self.text.remove(offset);
        self.cursor -= 1;
        self.leave_history_navigation();
    }

    /// Deletes the character at the cursor.
    pub fn delete(&mut self) {
        if self.cursor >= self.len() {
            return;
        }
        let offset = self.byte_offset(self.cursor);
        self.text.remove(offset);
        self.leave_history_navigation();
    }

    /// Moves the cursor one character left.
    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Moves the cursor one character right.
    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    /// Moves the cursor to the start of the current line.
    pub fn move_home(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut index = self.cursor;
        while index > 0 && chars[index - 1] != '\n' {
            index -= 1;
        }
        self.cursor = index;
    }

    /// Moves the cursor to the end of the current line.
    pub fn move_end(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut index = self.cursor;
        while index < chars.len() && chars[index] != '\n' {
            index += 1;
        }
        self.cursor = index;
    }

    /// Empties the buffer.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.leave_history_navigation();
    }

    /// Replaces the draft and leaves the cursor at its end.
    pub fn replace(&mut self, value: impl Into<String>) {
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
        let draft = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.record_history(draft);
    }

    /// Recalls the previous composer-history entry.
    pub fn recall_previous(&mut self) -> bool {
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

    /// Whether Up/Down currently navigate composer history.
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

    /// Empties the buffer and returns its trimmed contents.
    pub fn take(&mut self) -> String {
        let taken = self.text.trim().to_owned();
        self.clear();
        taken
    }

    /// The buffer split into display lines.
    pub fn lines(&self) -> Vec<&str> {
        self.text.split('\n').collect()
    }

    /// The cursor as a `(line, column)` pair, both zero-based and counted in
    /// characters.
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
                column += 1;
            }
        }
        (line, column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn home_and_end_stay_within_the_current_line() {
        let mut composer = Composer::new();
        composer.insert_str("first\nsecond");
        composer.move_home();
        assert_eq!(composer.cursor(), 6);
        composer.move_end();
        assert_eq!(composer.cursor(), 12);
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
    fn taking_trims_and_empties() {
        let mut composer = Composer::new();
        composer.insert_str("  run the tests  ");
        assert!(!composer.is_blank());
        assert_eq!(composer.take(), "run the tests");
        assert!(composer.is_empty());
        assert_eq!(composer.cursor(), 0);
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
