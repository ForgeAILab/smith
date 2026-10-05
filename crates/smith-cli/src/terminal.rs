//! Terminal setup and teardown.
//!
//! Entering raw mode and the alternate screen changes global terminal state, so
//! restoring it is not optional: an early return, an error, or a panic that
//! skipped the restore would leave the user with a shell that does not echo.
//! [`enter`] installs a panic hook that restores first and then panics, so a
//! crash prints a readable backtrace instead of a scrambled one.

use std::io::{Stdout, Write, stdout};

use anyhow::Result;
use crossterm::cursor::MoveTo;
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;

type Inner = ratatui::Terminal<CrosstermBackend<Stdout>>;

/// An entered terminal that restores global shell state on every exit path.
pub(crate) struct Terminal {
    inner: Inner,
    restored: bool,
}

impl Terminal {
    /// Lets the session loop share its draw path with backend-driven input tests
    /// while this guard continues to own raw mode and alternate-screen restoration.
    pub(crate) fn inner_mut(&mut self) -> &mut Inner {
        &mut self.inner
    }

    /// Clear without querying the cursor and discard cell diffs so a new step's
    /// text is written whole, including an embedded session's retained backdrop.
    pub(crate) fn repaint(&mut self) -> std::io::Result<()> {
        clear_screen(&mut stdout())?;
        self.inner = ratatui::Terminal::new(CrosstermBackend::new(stdout()))?;
        invalidate_previous_frame(&mut self.inner);
        Ok(())
    }

    /// Draws one coalesced frame.
    pub(crate) fn draw<F>(&mut self, render: F) -> std::io::Result<ratatui::CompletedFrame<'_>>
    where
        F: FnOnce(&mut ratatui::Frame<'_>),
    {
        self.inner.draw(render)
    }

    /// Restores the normal screen and cooked input mode.
    pub(crate) fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        leave()?;
        self.restored = true;
        Ok(())
    }
}

/// A fresh blank buffer still lets ratatui skip spaces between words. Seed its
/// previous frame with empty symbols so every visible cell differs, then swap back
/// to an empty drawing buffer. These sentinels are never sent to the terminal;
/// the completed frame replaces them and ordinary frames resume normal diffs.
pub(crate) fn invalidate_previous_frame<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
) {
    for cell in &mut terminal.current_buffer_mut().content {
        cell.set_symbol("");
    }
    terminal.swap_buffers();
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Enters raw mode and the alternate screen, installing a restoring panic hook.
pub(crate) fn enter() -> Result<Terminal> {
    enable_raw_mode()?;
    // Bracketed paste turns a paste into one `Event::Paste` instead of a
    // storm of key events. Mouse reporting stays off: it is terminal-wide, so
    // enabling it to collect wheel events also swallows the drag the terminal
    // needs for native selection and copy. Keyboard scrolling covers the
    // transcript instead.
    if let Err(error) = enter_screen(&mut stdout()) {
        let _ = disable_raw_mode();
        return Err(error.into());
    }

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = leave();
        previous(info);
    }));

    let terminal = ratatui::Terminal::new(CrosstermBackend::new(stdout()));
    let terminal = match terminal {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = leave();
            return Err(error.into());
        }
    };
    if let Err(error) = clear_screen(&mut stdout()) {
        let _ = leave();
        return Err(error.into());
    }
    Ok(Terminal {
        inner: terminal,
        restored: false,
    })
}

fn clear_screen(writer: &mut impl Write) -> std::io::Result<()> {
    // A write-only clear, never `ratatui::Terminal::clear()`: its `ESC[6n`
    // query can lose its reply to crossterm's global reader after an
    // `EventStream` has existed, then time out under a multiplexer (cmux).
    // Entry and screen step boundaries clear; ordinary frames and host
    // rebuilds keep their buffers for an uninterrupted session surface.
    execute!(writer, Clear(ClearType::All), MoveTo(0, 0))
}

/// Restores the terminal. Safe to call more than once.
pub(crate) fn leave() -> Result<()> {
    leave_screen(&mut stdout())?;
    disable_raw_mode()?;
    Ok(())
}

/// Button presses (`1000`), drag motion while a button is held (`1002`), and
/// SGR coordinates (`1006`), which lift the 223-column limit of the original
/// encoding.
///
/// Deliberately not crossterm's `EnableMouseCapture`: that also sends `1003`,
/// which reports every pointer move across the terminal even with no button
/// down. Smith has nothing to do with a hovering pointer, and the events cost a
/// wakeup each.
const ENABLE_MOUSE: &str = "\u{1b}[?1000h\u{1b}[?1002h\u{1b}[?1006h";
/// The exact inverse of [`ENABLE_MOUSE`], innermost mode first.
const DISABLE_MOUSE: &str = "\u{1b}[?1006l\u{1b}[?1002l\u{1b}[?1000l";

fn enter_screen(writer: &mut impl Write) -> std::io::Result<()> {
    execute!(writer, EnterAlternateScreen, EnableBracketedPaste)?;
    // Selection is Smith's own once these are on: the terminal hands us the
    // left-button press instead of starting a native drag. See
    // `smith_tui::selection`, which paints the highlight and yields the text.
    write!(writer, "{ENABLE_MOUSE}")?;
    writer.flush()
}

fn leave_screen(writer: &mut impl Write) -> std::io::Result<()> {
    write!(writer, "{DISABLE_MOUSE}")?;
    execute!(writer, DisableBracketedPaste, LeaveAlternateScreen)
}

#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod tests {
    use super::*;

    #[test]
    fn repaint_writes_spaces_in_whole_phrases_and_then_resumes_cell_diffs() {
        let mut output = Vec::new();
        {
            let mut terminal = ratatui::Terminal::with_options(
                CrosstermBackend::new(&mut output),
                ratatui::TerminalOptions {
                    viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 60, 2)),
                },
            )
            .expect("fixed terminal");
            invalidate_previous_frame(&mut terminal);
            let render = |frame: &mut ratatui::Frame<'_>| {
                frame.render_widget(
                    ratatui::widgets::Paragraph::new("Quick start with GLM"),
                    frame.area(),
                );
            };
            terminal.draw(render).expect("whole repaint");
            // A marker in the captured byte stream separates the two draws.
            terminal
                .backend_mut()
                .write_all(b"FRAME_BOUNDARY")
                .expect("marker");
            terminal.draw(render).expect("unchanged frame");
        }
        let output = String::from_utf8(output).expect("ANSI is UTF-8");
        let (first, second) = output.split_once("FRAME_BOUNDARY").expect("two frames");
        assert!(first.contains("Quick start with GLM"), "{first:?}");
        assert!(!second.contains("Quick"), "{second:?}");
        assert!(!output.contains('\0'), "sentinels must never be emitted");
        assert!(
            !output.contains("\u{1b}[6n"),
            "cursor queries must never be emitted"
        );
    }

    #[test]
    fn terminal_screen_modes_enable_and_restore_button_and_drag_reporting() {
        let mut entered = Vec::new();
        enter_screen(&mut entered).expect("enter sequences");
        let entered = String::from_utf8(entered).expect("ANSI is UTF-8");
        assert!(entered.contains("\u{1b}[?1049h"), "{entered:?}");
        assert!(entered.contains("\u{1b}[?2004h"), "{entered:?}");
        for code in ["1000", "1002", "1006"] {
            assert!(entered.contains(&format!("?{code}h")), "{entered:?}");
        }

        let mut left = Vec::new();
        leave_screen(&mut left).expect("leave sequences");
        let left = String::from_utf8(left).expect("ANSI is UTF-8");
        assert!(left.contains("\u{1b}[?2004l"), "{left:?}");
        assert!(left.contains("\u{1b}[?1049l"), "{left:?}");
        for code in ["1000", "1002", "1006"] {
            assert!(left.contains(&format!("?{code}l")), "{left:?}");
        }
    }

    #[test]
    fn all_motion_reporting_stays_off() {
        let mut entered = Vec::new();
        enter_screen(&mut entered).expect("enter sequences");
        let entered = String::from_utf8(entered).expect("ANSI is UTF-8");
        // `1003` would report a bare hover; nothing in Smith consumes one.
        assert!(!entered.contains("?1003h"), "{entered:?}");
    }

    #[test]
    fn screen_clear_never_queries_the_cursor_position() {
        let mut cleared = Vec::new();
        clear_screen(&mut cleared).expect("clear sequences");
        let cleared = String::from_utf8(cleared).expect("ANSI is UTF-8");
        assert!(cleared.contains("\u{1b}[2J"), "{cleared:?}");
        assert!(cleared.contains("\u{1b}[1;1H"), "{cleared:?}");
        assert!(!cleared.contains("\u{1b}[6n"), "{cleared:?}");
    }
}
