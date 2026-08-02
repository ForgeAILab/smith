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

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Enters raw mode and the alternate screen, installing a restoring panic hook.
pub(crate) fn enter() -> Result<Terminal> {
    enable_raw_mode()?;
    // Bracketed paste turns a paste into one `Event::Paste` instead of a
    // storm of key events. Mouse reporting deliberately stays disabled so the
    // terminal retains native drag selection and copy behavior everywhere.
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
    // A write-only clear, never `ratatui::Terminal::clear()`: that one opens
    // with an `ESC[6n` cursor-position query whose reply can be swallowed by
    // crossterm's global event reader once an `EventStream` has existed — the
    // exact state a palette/slash reconfiguration re-enters this function in.
    // Under a multiplexer (observed in cmux) the query then times out and a
    // working session dies at the handshake. The screen still needs wiping
    // because a re-entered alternate screen can show the previous TUI frame,
    // and a fresh ratatui terminal redraws fully on its first frame anyway.
    if let Err(error) = execute!(stdout(), Clear(ClearType::All), MoveTo(0, 0)) {
        let _ = leave();
        return Err(error.into());
    }
    Ok(Terminal {
        inner: terminal,
        restored: false,
    })
}

/// Restores the terminal. Safe to call more than once.
pub(crate) fn leave() -> Result<()> {
    leave_screen(&mut stdout())?;
    disable_raw_mode()?;
    Ok(())
}

fn enter_screen(writer: &mut impl Write) -> std::io::Result<()> {
    execute!(writer, EnterAlternateScreen, EnableBracketedPaste)
}

fn leave_screen(writer: &mut impl Write) -> std::io::Result<()> {
    execute!(writer, DisableBracketedPaste, LeaveAlternateScreen)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUSE_MODE_CODES: [&str; 5] = ["1000", "1002", "1003", "1015", "1006"];

    #[test]
    fn terminal_screen_modes_preserve_native_pointer_selection() {
        let mut entered = Vec::new();
        enter_screen(&mut entered).expect("enter sequences");
        let entered = String::from_utf8(entered).expect("ANSI is UTF-8");
        assert!(entered.contains("\u{1b}[?1049h"), "{entered:?}");
        assert!(entered.contains("\u{1b}[?2004h"), "{entered:?}");
        for code in MOUSE_MODE_CODES {
            assert!(!entered.contains(&format!("?{code}h")), "{entered:?}");
        }

        let mut left = Vec::new();
        leave_screen(&mut left).expect("leave sequences");
        let left = String::from_utf8(left).expect("ANSI is UTF-8");
        assert!(left.contains("\u{1b}[?2004l"), "{left:?}");
        assert!(left.contains("\u{1b}[?1049l"), "{left:?}");
        for code in MOUSE_MODE_CODES {
            assert!(!left.contains(&format!("?{code}l")), "{left:?}");
        }
    }
}
