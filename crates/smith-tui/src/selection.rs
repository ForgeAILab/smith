//! Pointer text selection owned by Smith rather than the terminal.
//!
//! Terminal mouse reporting is global and all-or-nothing on the button: the
//! moment Smith asks for wheel notches, the terminal also hands over the
//! left-button press and stops performing native drag selection. There is no
//! wheel-only mouse mode in any terminal protocol, so keeping the wheel means
//! Smith must do the selecting itself.
//!
//! The model here is deliberately screen-space, not text-space. A selection is
//! a pair of cell coordinates over the *rendered* frame, and the text is read
//! back out of the frame buffer at copy time (see
//! [`text_from_buffer`]). That sidesteps mapping a click through word wrap,
//! scroll offset, and every widget's internal layout: what you drag across is
//! exactly what you get, including the composer and header.
//!
//! The cost is that a selection is only meaningful against the frame it was
//! drawn over, so anything that moves cells — scrolling, new output, a resize —
//! clears it. [`Selection::stale_after_redraw`] is the single place that rule
//! lives.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// An in-progress or completed pointer selection over rendered cells.
///
/// `anchor` is where the drag started and `head` is where the pointer is now;
/// either may be the earlier position on screen, so both ends are ordered at
/// use time rather than at insert time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    anchor: (u16, u16),
    head: (u16, u16),
    /// False once the button is released. A finished selection stays painted so
    /// the user can see what was copied.
    dragging: bool,
}

impl Selection {
    /// Starts a selection at a pressed cell.
    pub fn begin(column: u16, row: u16) -> Self {
        Self {
            anchor: (column, row),
            head: (column, row),
            dragging: true,
        }
    }

    /// Moves the loose end while the button is held.
    pub fn drag_to(&mut self, column: u16, row: u16) {
        self.head = (column, row);
    }

    /// Marks the button released, keeping the highlight painted.
    pub fn finish(&mut self) {
        self.dragging = false;
    }

    /// Whether the button is still held.
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// Whether the drag never left its starting cell.
    ///
    /// A bare click is how a user dismisses a previous highlight, so it must
    /// not be mistaken for a one-character selection and copied.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The two ends in screen order: earlier row first, then earlier column.
    fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        let (anchor, head) = (self.anchor, self.head);
        // Compare row before column: selection follows text flow, so a point
        // one row down is later even when its column is further left.
        if (anchor.1, anchor.0) <= (head.1, head.0) {
            (anchor, head)
        } else {
            (head, anchor)
        }
    }

    /// The half-open column span selected on `row`, if any.
    ///
    /// Selection is linear, not rectangular: it follows text flow, so interior
    /// rows span the full width and only the first and last rows are clipped by
    /// the drag's endpoints.
    pub fn span_on_row(&self, row: u16, area: Rect) -> Option<(u16, u16)> {
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let (left, right) = (area.x, area.x.saturating_add(area.width));
        let (start, end) = self.ordered();
        if row < start.1 || row > end.1 {
            return None;
        }
        let from = if row == start.1 {
            start.0.clamp(left, right)
        } else {
            left
        };
        // The head cell is included, which is what a user dragging rightward
        // expects: releasing over a character selects that character.
        let to = if row == end.1 {
            end.0.saturating_add(1).clamp(left, right)
        } else {
            right
        };
        (from < to).then_some((from, to))
    }

    /// Whether a redraw at `area` invalidates this selection.
    ///
    /// Screen-space coordinates only mean something against the frame they were
    /// drawn over. A selection that now falls outside the surface, or one drawn
    /// before the content moved underneath it, is discarded rather than
    /// silently highlighting the wrong text.
    pub fn stale_after_redraw(&self, area: Rect) -> bool {
        let bottom = area.y.saturating_add(area.height);
        let right = area.x.saturating_add(area.width);
        let outside = |(column, row): (u16, u16)| {
            row < area.y || row >= bottom || column < area.x || column >= right
        };
        outside(self.anchor) || outside(self.head)
    }
}

/// Reads the selected text out of a rendered frame buffer.
///
/// Returns `None` when the selection covers nothing but blank cells, so a stray
/// drag across empty transcript space does not clobber the clipboard.
///
/// Trailing blanks are trimmed per row because the buffer is padded to the full
/// surface width: without this, every copied line would carry the spaces
/// between the text and the right edge.
pub fn text_from_buffer(selection: &Selection, buffer: &Buffer, area: Rect) -> Option<String> {
    if selection.is_empty() {
        return None;
    }
    let mut rows = Vec::new();
    for row in area.y..area.y.saturating_add(area.height) {
        let Some((from, to)) = selection.span_on_row(row, area) else {
            continue;
        };
        let mut text = String::new();
        for column in from..to {
            // A wide glyph occupies two cells: the second carries an empty
            // symbol, and skipping it keeps the copy free of padding while
            // preserving the character itself from the first cell.
            let symbol = buffer[(column, row)].symbol();
            if symbol.is_empty() {
                continue;
            }
            text.push_str(symbol);
        }
        rows.push(text.trim_end().to_owned());
    }
    // Interior blank rows are real content — a paragraph break — but leading
    // and trailing ones are just the drag overshooting the text.
    while rows.first().is_some_and(String::is_empty) {
        rows.remove(0);
    }
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    (!rows.is_empty()).then(|| rows.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn area() -> Rect {
        Rect::new(0, 0, 10, 4)
    }

    fn buffer(rows: [&str; 4]) -> Buffer {
        let mut buffer = Buffer::empty(area());
        for (row, text) in rows.iter().enumerate() {
            buffer.set_string(
                0,
                u16::try_from(row).expect("small"),
                text,
                Style::default(),
            );
        }
        buffer
    }

    #[test]
    fn a_selection_within_one_row_spans_only_the_dragged_cells() {
        let mut selection = Selection::begin(2, 1);
        selection.drag_to(5, 1);

        assert_eq!(selection.span_on_row(1, area()), Some((2, 6)));
        assert_eq!(selection.span_on_row(0, area()), None);
        assert_eq!(selection.span_on_row(2, area()), None);
    }

    #[test]
    fn a_backward_drag_selects_the_same_cells_as_a_forward_one() {
        let mut forward = Selection::begin(2, 1);
        forward.drag_to(7, 2);
        let mut backward = Selection::begin(7, 2);
        backward.drag_to(2, 1);

        for row in 0..4 {
            assert_eq!(
                forward.span_on_row(row, area()),
                backward.span_on_row(row, area()),
                "row {row}"
            );
        }
    }

    #[test]
    fn a_multi_row_selection_fills_interior_rows_to_the_edges() {
        let mut selection = Selection::begin(6, 0);
        selection.drag_to(3, 2);

        assert_eq!(selection.span_on_row(0, area()), Some((6, 10)));
        assert_eq!(selection.span_on_row(1, area()), Some((0, 10)));
        assert_eq!(selection.span_on_row(2, area()), Some((0, 4)));
    }

    #[test]
    fn copied_text_joins_rows_and_drops_the_padding_to_the_right_edge() {
        let buffer = buffer(["hello", "second", "third", ""]);
        let mut selection = Selection::begin(0, 0);
        selection.drag_to(4, 1);

        let text = text_from_buffer(&selection, &buffer, area()).expect("text");
        assert_eq!(text, "hello\nsecon");
    }

    #[test]
    fn a_selection_over_blank_cells_copies_nothing() {
        let buffer = buffer(["", "", "", ""]);
        let mut selection = Selection::begin(0, 0);
        selection.drag_to(9, 3);

        assert_eq!(text_from_buffer(&selection, &buffer, area()), None);
    }

    #[test]
    fn a_click_that_never_moved_copies_nothing() {
        let buffer = buffer(["hello", "", "", ""]);
        let selection = Selection::begin(1, 0);

        assert!(selection.is_empty());
        assert_eq!(text_from_buffer(&selection, &buffer, area()), None);
    }

    #[test]
    fn blank_rows_survive_inside_a_selection_but_not_at_its_edges() {
        let buffer = buffer(["", "second", "", "fourth"]);
        let mut selection = Selection::begin(0, 0);
        selection.drag_to(9, 3);

        let text = text_from_buffer(&selection, &buffer, area()).expect("text");
        assert_eq!(text, "second\n\nfourth");
    }

    #[test]
    fn a_selection_off_the_resized_surface_is_stale() {
        let mut selection = Selection::begin(2, 3);
        selection.drag_to(8, 3);

        assert!(!selection.stale_after_redraw(area()));
        assert!(selection.stale_after_redraw(Rect::new(0, 0, 10, 2)));
    }
}
