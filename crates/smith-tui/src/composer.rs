//! The input composer.
//!
//! A small multi-line buffer with a character cursor. It indexes by `char`
//! rather than by byte, so a cursor never lands inside a multi-byte codepoint
//! and panics the renderer — which is exactly what happens the first time
//! someone types an accented character into a byte-indexed buffer.

/// A multi-line text buffer with a cursor.
#[derive(Debug, Clone, Default)]
pub struct Composer {
    text: String,
    /// Cursor position, counted in characters from the start.
    cursor: usize,
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
        let offset = self.byte_offset(self.cursor);
        self.text.insert(offset, ch);
        self.cursor += 1;
    }

    /// Inserts a string at the cursor, as a paste would.
    pub fn insert_str(&mut self, value: &str) {
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
    }

    /// Deletes the character at the cursor.
    pub fn delete(&mut self) {
        if self.cursor >= self.len() {
            return;
        }
        let offset = self.byte_offset(self.cursor);
        self.text.remove(offset);
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
    }

    /// Replaces the draft and leaves the cursor at its end.
    pub fn replace(&mut self, value: impl Into<String>) {
        self.text = value.into();
        self.cursor = self.text.chars().count();
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
}
