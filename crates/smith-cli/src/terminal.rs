//! Terminal setup and teardown.
//!
//! Entering raw mode and the alternate screen changes global terminal state, so
//! restoring it is not optional: an early return, an error, or a panic that
//! skipped the restore would leave the user with a shell that does not echo.
//! [`enter`] installs a panic hook that restores first and then panics, so a
//! crash prints a readable backtrace instead of a scrambled one.
//!
//! Where the terminal speaks the kitty keyboard protocol, Smith also asks it
//! to disambiguate modified keys, which is what lets Shift+Enter arrive as
//! something other than Enter. That request is global terminal state too, so
//! it follows the same rule: every path that leaves the screen pops it.

use std::io::{Stdout, Write, stdout};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use crossterm::cursor::{MoveTo, Show};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;

type Inner = ratatui::Terminal<CrosstermBackend<Stdout>>;

/// Whether raw mode and the alternate screen are in effect, so exit paths that
/// cannot reach the [`Terminal`] restore only what Smith changed. A headless
/// run never enters, and must not have escape sequences written for it.
static ENTERED: AtomicBool = AtomicBool::new(false);

/// Whether keyboard enhancement flags are pushed and not yet popped. Swapped
/// to `false` by whichever exit path pops first, so a panic hook followed by
/// the guard's drop cannot pop twice: on a terminal with one stack, a second
/// pop would remove an entry the user's shell pushed.
static KEYBOARD_PUSHED: AtomicBool = AtomicBool::new(false);

/// The terminal's answer to the keyboard-protocol query, asked once.
///
/// The reply travels through crossterm's global reader, which can lose replies
/// once an `EventStream` has existed (see [`clear_screen`]). The first
/// [`enter`] always precedes the first reader, so it asks; a later entry, such
/// as the session after setup, reuses the answer.
static KEYBOARD_SUPPORTED: OnceLock<bool> = OnceLock::new();

/// Only disambiguation: modified keys such as Shift+Enter get their own
/// encoding, while unmodified text, Enter, Tab, and Backspace keep the legacy
/// bytes. Release and repeat reports stay off, so every key is still exactly
/// one press event.
const KEYBOARD_FLAGS: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;

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
    // In raw mode, so the reply is read rather than echoed. A terminal that
    // does not answer within crossterm's timeout counts as unsupported.
    let keyboard = *KEYBOARD_SUPPORTED
        .get_or_init(|| crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false));
    // Marked before writing, so a failure partway through still pops.
    KEYBOARD_PUSHED.store(keyboard, Ordering::SeqCst);
    // Bracketed paste turns a paste into one `Event::Paste` instead of a
    // storm of key events. Mouse reporting stays off: it is terminal-wide, so
    // enabling it to collect wheel events also swallows the drag the terminal
    // needs for native selection and copy. Keyboard scrolling covers the
    // transcript instead.
    if let Err(error) = enter_screen(&mut stdout(), keyboard) {
        let _ = leave_screen(&mut stdout(), take_keyboard_pop());
        let _ = disable_raw_mode();
        return Err(error.into());
    }
    ENTERED.store(true, Ordering::SeqCst);

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
    ENTERED.store(false, Ordering::SeqCst);
    leave_screen(&mut stdout(), take_keyboard_pop())?;
    disable_raw_mode()?;
    Ok(())
}

/// Avoid stdio locks when the exit backstop runs beside a blocked runtime.
pub(crate) fn restore_best_effort() {
    if !ENTERED.swap(false, Ordering::SeqCst) {
        return;
    }
    let _ = disable_raw_mode();
    #[cfg(unix)]
    use std::os::fd::AsFd;
    #[cfg(unix)]
    if let Ok(mut output) = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .or_else(|_| {
            std::io::stderr()
                .as_fd()
                .try_clone_to_owned()
                .map(std::fs::File::from)
        })
    {
        let _ = leave_screen(&mut output, take_keyboard_pop());
    }
    #[cfg(not(unix))]
    let _ = leave_screen(&mut std::io::stderr(), take_keyboard_pop());
}

/// Whether this exit path owes the pop. True at most once per push, whichever
/// of the guard, the panic hook, the exit backstop, or a second stop signal
/// leaves first.
fn take_keyboard_pop() -> bool {
    KEYBOARD_PUSHED.swap(false, Ordering::SeqCst)
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

/// `keyboard` pushes the enhancement flags after the alternate screen is
/// entered: kitty-protocol terminals keep one flag stack per screen, so the
/// push lands on the screen Smith owns.
fn enter_screen(writer: &mut impl Write, keyboard: bool) -> std::io::Result<()> {
    execute!(writer, EnterAlternateScreen, EnableBracketedPaste)?;
    if keyboard {
        execute!(writer, PushKeyboardEnhancementFlags(KEYBOARD_FLAGS))?;
    }
    // Selection is Smith's own once these are on: the terminal hands us the
    // left-button press instead of starting a native drag. See
    // `smith_tui::selection`, which paints the highlight and yields the text.
    write!(writer, "{ENABLE_MOUSE}")?;
    writer.flush()
}

/// `keyboard` pops the flags [`enter_screen`] pushed, first and while still on
/// the alternate screen, so the pop cannot reach the main screen's stack,
/// which belongs to the shell.
fn leave_screen(writer: &mut impl Write, keyboard: bool) -> std::io::Result<()> {
    if keyboard {
        execute!(writer, PopKeyboardEnhancementFlags)?;
    }
    write!(writer, "{DISABLE_MOUSE}\u{1b}[?1003l")?;
    execute!(writer, DisableBracketedPaste, LeaveAlternateScreen, Show)
}

/// Restores the legacy meaning of keys that disambiguation separates.
///
/// Without the keyboard protocol, Ctrl+[, Ctrl+I, and Ctrl+M send the same
/// bytes as Esc, Tab, and Enter, and people press them for that. With it, the
/// terminal reports Ctrl plus the character, which no binding handles. Every
/// other key the protocol reports already reaches the binding it reached
/// before, so nothing else is rewritten.
pub(crate) fn legacy_key_aliases(event: Event) -> Event {
    let Event::Key(key) = event else {
        return event;
    };
    if key.modifiers != KeyModifiers::CONTROL {
        return Event::Key(key);
    }
    let code = match key.code {
        KeyCode::Char('[') => KeyCode::Esc,
        KeyCode::Char('i') => KeyCode::Tab,
        KeyCode::Char('m') => KeyCode::Enter,
        _ => return Event::Key(key),
    };
    Event::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        ..key
    })
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
        for keyboard in [false, true] {
            let mut entered = Vec::new();
            enter_screen(&mut entered, keyboard).expect("enter sequences");
            let entered = String::from_utf8(entered).expect("ANSI is UTF-8");
            assert!(entered.contains("\u{1b}[?1049h"), "{entered:?}");
            assert!(entered.contains("\u{1b}[?2004h"), "{entered:?}");
            for code in ["1000", "1002", "1006"] {
                assert!(entered.contains(&format!("?{code}h")), "{entered:?}");
            }

            let mut left = Vec::new();
            leave_screen(&mut left, keyboard).expect("leave sequences");
            let left = String::from_utf8(left).expect("ANSI is UTF-8");
            assert!(left.contains("\u{1b}[?2004l"), "{left:?}");
            assert!(left.contains("\u{1b}[?1049l"), "{left:?}");
            assert!(left.contains("\u{1b}[?25h"), "{left:?}");
            assert!(left.contains("?1003l"), "{left:?}");
            for code in ["1000", "1002", "1006"] {
                assert!(left.contains(&format!("?{code}l")), "{left:?}");
            }
        }
    }

    #[test]
    fn all_motion_reporting_stays_off() {
        let mut entered = Vec::new();
        enter_screen(&mut entered, true).expect("enter sequences");
        let entered = String::from_utf8(entered).expect("ANSI is UTF-8");
        // `1003` would report a bare hover; nothing in Smith consumes one.
        assert!(!entered.contains("?1003h"), "{entered:?}");
    }

    const PUSH_DISAMBIGUATE: &str = "\u{1b}[>1u";
    const POP_KEYBOARD: &str = "\u{1b}[<1u";

    #[test]
    fn keyboard_flags_are_pushed_on_the_alternate_screen_and_popped_before_leaving_it() {
        let mut entered = Vec::new();
        enter_screen(&mut entered, true).expect("enter sequences");
        let entered = String::from_utf8(entered).expect("ANSI is UTF-8");
        // Disambiguation only: no release/repeat (2), alternate keys (4), or
        // all-keys-as-escapes (8) flags.
        assert_eq!(entered.matches(PUSH_DISAMBIGUATE).count(), 1, "{entered:?}");
        assert_eq!(entered.matches("\u{1b}[>").count(), 1, "{entered:?}");
        let alternate = entered.find("\u{1b}[?1049h").expect("alternate screen");
        assert!(
            alternate < entered.find(PUSH_DISAMBIGUATE).expect("push"),
            "the push must land on Smith's screen: {entered:?}"
        );

        let mut left = Vec::new();
        leave_screen(&mut left, true).expect("leave sequences");
        let left = String::from_utf8(left).expect("ANSI is UTF-8");
        assert_eq!(left.matches(POP_KEYBOARD).count(), 1, "{left:?}");
        assert!(
            left.find(POP_KEYBOARD).expect("pop") < left.find("\u{1b}[?1049l").expect("leave"),
            "the pop must not reach the shell's screen: {left:?}"
        );
    }

    #[test]
    fn an_unsupported_terminal_gets_neither_push_nor_pop() {
        let mut written = Vec::new();
        enter_screen(&mut written, false).expect("enter sequences");
        leave_screen(&mut written, false).expect("leave sequences");
        let written = String::from_utf8(written).expect("ANSI is UTF-8");
        assert!(!written.contains("\u{1b}[>"), "{written:?}");
        assert!(!written.contains("\u{1b}[<"), "{written:?}");
    }

    #[test]
    fn every_exit_path_shares_one_pop() {
        // The only test that touches the process-wide flag; real exits read it
        // through `take_keyboard_pop` exactly as below.
        KEYBOARD_PUSHED.store(true, Ordering::SeqCst);
        let mut first = Vec::new();
        leave_screen(&mut first, take_keyboard_pop()).expect("panic hook leave");
        let mut second = Vec::new();
        leave_screen(&mut second, take_keyboard_pop()).expect("guard leave");
        let mut third = Vec::new();
        leave_screen(&mut third, take_keyboard_pop()).expect("backstop leave");
        let [first, second, third] =
            [first, second, third].map(|bytes| String::from_utf8(bytes).expect("ANSI"));
        assert!(first.contains(POP_KEYBOARD), "{first:?}");
        assert!(!second.contains(POP_KEYBOARD), "{second:?}");
        assert!(!third.contains(POP_KEYBOARD), "{third:?}");
        // The remaining restore sequences are idempotent and still sent.
        assert!(second.contains("\u{1b}[?1049l"), "{second:?}");
    }

    fn pressed(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    #[test]
    fn disambiguated_control_aliases_keep_their_legacy_keys() {
        for (character, legacy) in [
            ('[', KeyCode::Esc),
            ('i', KeyCode::Tab),
            ('m', KeyCode::Enter),
        ] {
            assert_eq!(
                legacy_key_aliases(pressed(KeyCode::Char(character), KeyModifiers::CONTROL)),
                pressed(legacy, KeyModifiers::NONE),
                "Ctrl+{character}"
            );
        }
        // Lock state rides along unchanged.
        let locked = Event::Key(KeyEvent::new_with_kind_and_state(
            KeyCode::Char('['),
            KeyModifiers::CONTROL,
            crossterm::event::KeyEventKind::Press,
            crossterm::event::KeyEventState::CAPS_LOCK,
        ));
        let Event::Key(locked) = legacy_key_aliases(locked) else {
            panic!("a key stays a key");
        };
        assert_eq!(locked.code, KeyCode::Esc);
        assert_eq!(locked.state, crossterm::event::KeyEventState::CAPS_LOCK);
    }

    #[test]
    fn other_keys_pass_through_the_aliases_untouched() {
        for event in [
            // Ctrl+J is the newline key, never Enter.
            pressed(KeyCode::Char('j'), KeyModifiers::CONTROL),
            pressed(KeyCode::Char('c'), KeyModifiers::CONTROL),
            pressed(
                KeyCode::Char('m'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            pressed(KeyCode::Char('i'), KeyModifiers::ALT),
            pressed(KeyCode::Char('['), KeyModifiers::NONE),
            pressed(KeyCode::Enter, KeyModifiers::SHIFT),
            pressed(KeyCode::BackTab, KeyModifiers::SHIFT),
            Event::Paste("one\ntwo".to_owned()),
            Event::FocusGained,
        ] {
            assert_eq!(legacy_key_aliases(event.clone()), event);
        }
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
