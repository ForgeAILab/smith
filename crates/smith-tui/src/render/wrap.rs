//! Word wrapping that never loses a character.
//!
//! Ratatui's own `Wrap` is not safe for text without spaces. Its word wrapper
//! measures a pending word *before* adding the grapheme that overflows it, so
//! an unbroken run — which is what every Chinese, Japanese, or Korean sentence
//! is — can produce a wrapped line one column wider than the area. The
//! paragraph renderer then writes that last glyph past the right edge, where
//! it lands in the wrong cell: the character is silently dropped from the
//! screen and half a glyph is left hanging at the margin.
//!
//! So Smith wraps its own lines and hands the paragraphs rows that already
//! fit. The rules are the ones `Wrap { trim: false }` implements for prose —
//! break between words, keep the whitespace a line starts with — plus the one
//! it lacks: a word wider than the area breaks at a glyph boundary instead of
//! overflowing it.

use ratatui::buffer::CellWidth;
use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span, StyledGrapheme};

/// A grapheme together with the display column it sits at in its source line.
///
/// The column travels with the glyph so a caller can map a position in the
/// unwrapped text — a cursor — onto the row it ends up drawn in.
struct Placed<'a> {
    grapheme: StyledGrapheme<'a>,
    width: usize,
    column: usize,
}

/// Re-flows `lines` into rows at most `width` columns wide.
///
/// Every input line yields at least one row, so a blank line stays a paragraph
/// break rather than disappearing.
pub(crate) fn wrap_lines(lines: &[Line<'static>], width: u16) -> Vec<Line<'static>> {
    let limit = usize::from(width.max(1));
    let mut rows = Vec::with_capacity(lines.len());
    for line in lines {
        wrap_line(line, limit, &mut rows, &mut Vec::new());
    }
    rows
}

/// The number of rows `lines` occupy once wrapped to `width`.
pub(crate) fn wrapped_row_count(lines: &[Line<'static>], width: u16) -> usize {
    if lines.is_empty() {
        return 0;
    }
    wrap_lines(lines, width).len()
}

/// One line's rows, with the source display column each row begins at.
///
/// The offsets are what turn a cursor's position in the typed text into the
/// cell it is drawn over once the line has wrapped.
pub(crate) fn wrap_line_with_offsets(
    line: &Line<'static>,
    width: u16,
) -> (Vec<Line<'static>>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut offsets = Vec::new();
    wrap_line(line, usize::from(width.max(1)), &mut rows, &mut offsets);
    (rows, offsets)
}

fn wrap_line(
    line: &Line<'static>,
    limit: usize,
    rows: &mut Vec<Line<'static>>,
    offsets: &mut Vec<usize>,
) {
    let mut row = Row::new(line);
    // Whitespace is held back rather than placed: a run that falls at a break
    // belongs to neither row, and padding out to the right edge is invisible
    // against a buffer that is already blank there.
    let mut spaces: Vec<Placed<'_>> = Vec::new();
    let mut spaces_width = 0usize;
    let mut word: Vec<Placed<'_>> = Vec::new();
    let mut column = 0usize;

    for grapheme in line.styled_graphemes(Style::default()) {
        let width = usize::from(grapheme.symbol.cell_width());
        let placed = Placed {
            grapheme,
            width,
            column,
        };
        column += width;
        if placed.grapheme.is_whitespace() {
            if !word.is_empty() {
                place(
                    &mut row,
                    rows,
                    offsets,
                    limit,
                    &mut spaces,
                    spaces_width,
                    &word,
                );
                spaces_width = 0;
                word.clear();
            }
            spaces_width += width;
            spaces.push(placed);
        } else {
            word.push(placed);
        }
    }
    if !word.is_empty() {
        place(
            &mut row,
            rows,
            offsets,
            limit,
            &mut spaces,
            spaces_width,
            &word,
        );
    }
    // Whatever whitespace is left trails the line; only the part that fits is
    // kept, since the rest is indistinguishable from the blank cells beyond it.
    fill(&mut row, &mut spaces, limit);
    offsets.push(row.column);
    rows.push(row.finish());
}

/// Places one word, breaking the row before it when it does not fit, and
/// breaking the word itself when no row could hold it whole.
fn place(
    row: &mut Row,
    rows: &mut Vec<Line<'static>>,
    offsets: &mut Vec<usize>,
    limit: usize,
    spaces: &mut Vec<Placed<'_>>,
    spaces_width: usize,
    word: &[Placed<'_>],
) {
    let word_width: usize = word.iter().map(|placed| placed.width).sum();
    // A word no row could hold is broken where it stands rather than pushed to
    // a line of its own: moving it first would strand the marker or bullet
    // that opens the line on an otherwise empty row, which is what a Chinese
    // sentence — one unbroken word from the wrapper's point of view — does to
    // every transcript entry.
    let breakable = word_width <= limit;
    if breakable && row.width > 0 && row.width + spaces_width + word_width > limit {
        offsets.push(row.column);
        rows.push(row.take());
        spaces.clear();
    }
    fill(row, spaces, limit);
    for placed in word {
        // The `row.width > 0` guard is what guarantees progress: a glyph wider
        // than the whole area still takes a row of its own rather than looping.
        if row.width + placed.width > limit && row.width > 0 {
            offsets.push(row.column);
            rows.push(row.take());
        }
        row.push(placed);
    }
}

/// Moves as much held-back whitespace onto the row as still fits.
fn fill(row: &mut Row, spaces: &mut Vec<Placed<'_>>, limit: usize) {
    for placed in spaces.drain(..) {
        if row.width + placed.width > limit {
            break;
        }
        row.push(&placed);
    }
    spaces.clear();
}

/// One output row under construction, merging runs of one style into spans.
struct Row {
    spans: Vec<Span<'static>>,
    text: String,
    style: Style,
    width: usize,
    /// Display column in the source line this row starts at.
    column: usize,
    alignment: Option<Alignment>,
}

impl Row {
    fn new(line: &Line<'static>) -> Self {
        Self {
            spans: Vec::new(),
            text: String::new(),
            style: Style::default(),
            width: 0,
            column: 0,
            alignment: line.alignment,
        }
    }

    fn push(&mut self, placed: &Placed<'_>) {
        if self.width == 0 && self.spans.is_empty() && self.text.is_empty() {
            self.column = placed.column;
        }
        if !self.text.is_empty() && placed.grapheme.style != self.style {
            self.flush();
        }
        self.style = placed.grapheme.style;
        self.text.push_str(placed.grapheme.symbol);
        self.width += placed.width;
    }

    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.spans
                .push(Span::styled(std::mem::take(&mut self.text), self.style));
        }
    }

    fn line(&mut self) -> Line<'static> {
        self.flush();
        let mut line = Line::from(std::mem::take(&mut self.spans));
        line.alignment = self.alignment;
        line
    }

    fn finish(mut self) -> Line<'static> {
        self.line()
    }

    /// Emits the row and leaves an empty one in its place.
    ///
    /// The empty row inherits the column just past what was emitted, so a line
    /// that ends exactly on a break still reports where its end sits.
    fn take(&mut self) -> Line<'static> {
        let end = self.column + self.width;
        let line = self.line();
        self.width = 0;
        self.column = end;
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn text(rows: &[Line<'static>]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                row.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    fn widths(rows: &[Line<'static>]) -> Vec<usize> {
        rows.iter()
            .map(|row| {
                row.spans
                    .iter()
                    .flat_map(|span| span.content.chars())
                    .map(|character| usize::from(character.to_string().as_str().cell_width()))
                    .sum()
            })
            .collect()
    }

    #[test]
    fn prose_breaks_between_words() {
        let rows = wrap_lines(&[Line::from("the retry policy classifies failures")], 12);

        assert_eq!(
            text(&rows),
            vec!["the retry", "policy", "classifies", "failures"]
        );
    }

    #[test]
    fn a_chinese_sentence_wraps_without_losing_a_character() {
        // Ratatui's own wrapper drops the glyph that straddles the right edge
        // here, which is what put a hole in every Chinese transcript line.
        let sentence = "重试策略会把失败分为可恢复和不可恢复两类,可恢复的失败会重试。";
        for width in 3..40u16 {
            let rows = wrap_lines(&[Line::from(sentence)], width);
            assert_eq!(
                text(&rows).concat(),
                sentence,
                "width {width} changed the text"
            );
            for (row, drawn) in widths(&rows).into_iter().enumerate() {
                assert!(
                    drawn <= usize::from(width),
                    "width {width} row {row} is {drawn} columns wide"
                );
            }
        }
    }

    #[test]
    fn a_wide_glyph_is_never_split_across_rows() {
        let rows = wrap_lines(&[Line::from("一二三")], 3);

        // Three columns hold one whole character and not half of a second.
        assert_eq!(text(&rows), vec!["一", "二", "三"]);
    }

    #[test]
    fn a_word_no_row_could_hold_breaks_where_it_stands() {
        // The marker that opens the line keeps its place instead of being
        // stranded on a row of its own ahead of the unbreakable run.
        let rows = wrap_lines(&[Line::from("• 一二三四五六")], 8);

        assert_eq!(text(&rows), vec!["• 一二三", "四五六"]);
    }

    #[test]
    fn a_word_that_fits_a_row_of_its_own_moves_to_the_next_row() {
        let rows = wrap_lines(&[Line::from("ab classifies")], 10);

        assert_eq!(text(&rows), vec!["ab", "classifies"]);
    }

    #[test]
    fn a_blank_line_stays_a_paragraph_break() {
        let rows = wrap_lines(&[Line::from("one"), Line::default(), Line::from("two")], 10);

        assert_eq!(text(&rows), vec!["one", "", "two"]);
    }

    #[test]
    fn leading_whitespace_survives_but_trailing_padding_does_not() {
        let rows = wrap_lines(&[Line::from("    indented code")], 10);

        assert_eq!(text(&rows), vec!["    indent", "ed code"]);
    }

    #[test]
    fn styles_carry_across_a_break() {
        let accent = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        let rows = wrap_lines(
            &[Line::from(vec![
                Span::raw("• "),
                Span::styled("一二三四", accent),
            ])],
            6,
        );

        assert_eq!(text(&rows), vec!["• 一二", "三四"]);
        assert_eq!(rows[0].spans[1].style, accent);
        assert_eq!(rows[1].spans[0].style, accent);
    }

    #[test]
    fn row_offsets_locate_a_column_after_the_wrap() {
        let (rows, offsets) = wrap_line_with_offsets(&Line::from("› 一二三四五"), 8);

        assert_eq!(text(&rows), vec!["› 一二三", "四五"]);
        // The first row covers columns 0..8 and the second picks up at 8.
        assert_eq!(offsets, vec![0, 8]);
    }
}
