//! Terminal window-title management for the interactive TUI.
//!
//! The title is written as an OSC 0 sequence (`ESC ] 0 ; <title> BEL`) to the
//! real stdout, so two Smith sessions in different projects are
//! distinguishable from their window or tab titles instead of looking
//! identical. This module owns the write path and the sanitization that
//! happens immediately before it; the event loop decides when the title may
//! have changed.
//!
//! The payload is sanitized because its inputs are untrusted-derived text —
//! the project path name and the model id come from the filesystem and
//! configuration, not from Smith. Before the text enters the escape
//! sequence, control characters, Trojan-Source bidi/invisible formatting
//! codepoints, and redundant whitespace are removed, and the result is
//! bounded. The write itself is guarded by an `is_terminal` check so piped
//! or headless stdout never receives a title byte.
//!
//! On exit the caller clears the title this module last wrote. Restoring the
//! title the shell had before Smith started is deliberately not attempted:
//! reading a title back is not portable across terminals, and the shell's
//! own prompt hook reasserts its title on the next prompt.

use std::io::{IsTerminal, Write, stdout};

use crate::status::{Activity, Status};

/// Practical upper bound on title length, measured in Rust `char`s.
///
/// Most terminals silently truncate titles beyond a few hundred characters.
/// 240 leaves headroom for the OSC framing bytes while keeping titles
/// readable in tab bars and window managers.
const MAX_TERMINAL_TITLE_CHARS: usize = 240;

/// The OSC introducer and terminator bytes this module writes, verbatim.
const OSC_INTRODUCER: &str = "\x1b]0;";
/// BEL, not ST: some terminal integrations expose the ST terminator
/// (`ESC \`) in process decorations even when they accept the title update.
const OSC_TERMINATOR: &str = "\x07";

/// Outcome of writing a terminal title.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SetTerminalTitleResult {
    /// A sanitized title was written to the writer. Whether the writer is
    /// a terminal at all is the caller's guard, not this function's.
    Applied,
    /// Sanitization removed every visible character, so no title was
    /// emitted. Distinct from clearing: the caller decides what an empty
    /// title means.
    NoVisibleContent,
}

/// Renders one title from the current status and tracks the last title
/// actually written, so unchanged titles cost a string comparison instead
/// of an escape-sequence write.
#[derive(Debug, Default)]
pub struct TerminalTitleState {
    /// The last title successfully written, or `None` after a clear.
    last_written: Option<String>,
}

impl TerminalTitleState {
    /// A tracker that has not written any title yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The last title this tracker wrote, if any.
    pub fn last_written(&self) -> Option<&str> {
        self.last_written.as_deref()
    }

    /// Refreshes the managed title against the current status.
    ///
    /// Unchanged titles are not rewritten. A title whose sanitization
    /// removes every visible character clears the managed title instead —
    /// an honest empty rather than a stale wrong title.
    pub fn refresh(&mut self, status: &Status) -> std::io::Result<()> {
        self.refresh_guarded(stdout().is_terminal(), &mut stdout(), status)
    }

    /// Clears the managed title once, if one was written.
    pub fn clear(&mut self) -> std::io::Result<()> {
        self.clear_guarded(stdout().is_terminal(), &mut stdout())
    }

    fn refresh_guarded(
        &mut self,
        is_tty: bool,
        writer: &mut impl Write,
        status: &Status,
    ) -> std::io::Result<()> {
        if !is_tty {
            return Ok(());
        }
        let title = title_from_status(status);
        if self.last_written.as_deref() == Some(title.as_str()) {
            return Ok(());
        }
        match write_terminal_title(writer, &title)? {
            SetTerminalTitleResult::Applied => self.last_written = Some(title),
            SetTerminalTitleResult::NoVisibleContent => self.clear_guarded(true, writer)?,
        }
        Ok(())
    }

    fn clear_guarded(&mut self, is_tty: bool, writer: &mut impl Write) -> std::io::Result<()> {
        if self.last_written.take().is_some() && is_tty {
            write_osc(writer, "")?;
        }
        Ok(())
    }
}

/// Writes a sanitized OSC window-title sequence.
///
/// Returns [`SetTerminalTitleResult::NoVisibleContent`] when sanitization
/// removed every visible character, so the caller can distinguish "nothing
/// to show" from "title written".
pub fn write_terminal_title(
    writer: &mut impl Write,
    title: &str,
) -> std::io::Result<SetTerminalTitleResult> {
    let title = sanitize_terminal_title(title);
    if title.is_empty() {
        return Ok(SetTerminalTitleResult::NoVisibleContent);
    }
    write_osc(writer, &title)?;
    Ok(SetTerminalTitleResult::Applied)
}

fn write_osc(writer: &mut impl Write, title: &str) -> std::io::Result<()> {
    writer.write_all(OSC_INTRODUCER.as_bytes())?;
    writer.write_all(title.as_bytes())?;
    writer.write_all(OSC_TERMINATOR.as_bytes())?;
    writer.flush()
}

/// Renders the terminal title from the current status.
///
/// Joins the available segments with ` · `: the product name, the project,
/// the model, and — only while a turn or child wait is actually in flight —
/// the activity. Empty segments are skipped rather than rendered as blanks.
/// The format is fixed; making segments configurable is future work.
pub fn title_from_status(status: &Status) -> String {
    let mut segments = vec!["smith".to_owned()];
    if !status.project.is_empty() {
        segments.push(status.project.clone());
    }
    if !status.model.is_empty() {
        segments.push(status.model.clone());
    }
    match status.activity {
        Activity::Idle | Activity::Ended => {}
        Activity::Working | Activity::Interrupting | Activity::ParkedAwaitingChild => {
            segments.push(status.activity.label().to_owned());
        }
    }
    segments.join(" · ")
}

/// Normalizes untrusted-derived title text into a single bounded display
/// line: control characters and invisible/bidi formatting codepoints are
/// dropped, whitespace runs collapse to one space, and the result truncates
/// after [`MAX_TERMINAL_TITLE_CHARS`] visible characters.
pub fn sanitize_terminal_title(title: &str) -> String {
    let mut sanitized = String::new();
    let mut chars_written = 0;
    let mut pending_space = false;

    for ch in title.chars() {
        if ch.is_whitespace() {
            // Only set pending once content exists: this strips leading
            // whitespace without an extra trim pass, and a trailing run is
            // dropped by never being flushed.
            pending_space = !sanitized.is_empty();
            continue;
        }

        if is_disallowed_terminal_title_char(ch) {
            continue;
        }

        if pending_space {
            let remaining = MAX_TERMINAL_TITLE_CHARS.saturating_sub(chars_written);
            if remaining > 1 {
                sanitized.push(' ');
                chars_written += 1;
                pending_space = false;
            }
        }

        if chars_written >= MAX_TERMINAL_TITLE_CHARS {
            break;
        }

        sanitized.push(ch);
        chars_written += 1;
    }

    sanitized
}

/// Returns whether `ch` should be dropped from terminal-title output.
///
/// Covers plain control characters plus the Trojan-Source-style bidi
/// controls and common non-rendering formatting codepoints, so title text
/// cannot smuggle terminal control semantics or render misleadingly
/// relative to its bytes.
fn is_disallowed_terminal_title_char(ch: char) -> bool {
    if ch.is_control() {
        return true;
    }

    matches!(
        ch,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_TERMINAL_TITLE_CHARS, SetTerminalTitleResult, TerminalTitleState,
        sanitize_terminal_title, title_from_status, write_terminal_title,
    };
    use crate::status::{Activity, Status, TokenCount};

    fn status(project: &str, model: &str, activity: Activity) -> Status {
        let mut status = Status::new(model, project);
        status.activity = activity;
        status
    }

    #[test]
    fn sanitizes_terminal_title() {
        let sanitized =
            sanitize_terminal_title("  Project\t|\nWorking\x1b\x07\u{009D}\u{009C} |  Model  ");
        assert_eq!(sanitized, "Project | Working | Model");
    }

    #[test]
    fn strips_invisible_format_chars_from_terminal_title() {
        let sanitized = sanitize_terminal_title(
            "Pro\u{202E}j\u{2066}e\u{200F}c\u{061C}t\u{200B} \u{FEFF}T\u{2060}itle",
        );
        assert_eq!(sanitized, "Project Title");
    }

    #[test]
    fn truncates_terminal_title() {
        let input = "a".repeat(MAX_TERMINAL_TITLE_CHARS + 10);
        let sanitized = sanitize_terminal_title(&input);
        assert_eq!(sanitized.chars().count(), MAX_TERMINAL_TITLE_CHARS);
    }

    #[test]
    fn truncation_prefers_visible_char_over_pending_space() {
        let input = format!("{} b", "a".repeat(MAX_TERMINAL_TITLE_CHARS - 1));
        let sanitized = sanitize_terminal_title(&input);
        assert_eq!(sanitized.chars().count(), MAX_TERMINAL_TITLE_CHARS);
        assert_eq!(sanitized.chars().last(), Some('b'));
    }

    #[test]
    fn all_invisible_input_sanitizes_to_empty() {
        assert_eq!(sanitize_terminal_title("\u{200B}\u{202E}\u{FEFF} \t"), "");
    }

    #[test]
    fn writes_exact_osc_bytes_with_bel_terminator() {
        let mut out = Vec::new();
        let result = write_terminal_title(&mut out, "smith · tui").expect("write title");
        assert_eq!(result, SetTerminalTitleResult::Applied);
        assert_eq!(out, b"\x1b]0;smith \xc2\xb7 tui\x07");
    }

    #[test]
    fn invisible_only_title_reports_no_visible_content_without_writing() {
        let mut out = Vec::new();
        let result = write_terminal_title(&mut out, "\u{202E}").expect("write title");
        assert_eq!(result, SetTerminalTitleResult::NoVisibleContent);
        assert!(out.is_empty());
    }

    #[test]
    fn title_from_status_joins_available_segments() {
        assert_eq!(
            title_from_status(&status("proj", "model-x", Activity::Idle)),
            "smith · proj · model-x"
        );
    }

    #[test]
    fn title_from_status_appends_activity_only_while_work_is_in_flight() {
        assert_eq!(
            title_from_status(&status("proj", "model-x", Activity::Working)),
            "smith · proj · model-x · working"
        );
        assert_eq!(
            title_from_status(&status("proj", "model-x", Activity::Interrupting)),
            "smith · proj · model-x · interrupting"
        );
        assert_eq!(
            title_from_status(&status("proj", "model-x", Activity::ParkedAwaitingChild)),
            "smith · proj · model-x · waiting for child"
        );
        assert_eq!(
            title_from_status(&status("proj", "model-x", Activity::Ended)),
            "smith · proj · model-x"
        );
    }

    #[test]
    fn title_from_status_skips_empty_segments() {
        let mut empty_model = Status::new("", "proj");
        empty_model.activity = Activity::Idle;
        empty_model.context = TokenCount::UNKNOWN;
        assert_eq!(title_from_status(&empty_model), "smith · proj");
    }

    #[test]
    fn tracker_writes_only_on_change_and_clears_once() {
        let mut tracker = TerminalTitleState::new();
        let mut out = Vec::new();

        tracker
            .refresh_guarded(true, &mut out, &status("proj", "m", Activity::Idle))
            .expect("refresh");
        tracker
            .refresh_guarded(true, &mut out, &status("proj", "m", Activity::Idle))
            .expect("refresh");
        assert_eq!(out, b"\x1b]0;smith \xc2\xb7 proj \xc2\xb7 m\x07");

        tracker
            .refresh_guarded(true, &mut out, &status("proj", "m", Activity::Working))
            .expect("refresh");
        assert_eq!(
            out,
            b"\x1b]0;smith \xc2\xb7 proj \xc2\xb7 m\x07\x1b]0;smith \xc2\xb7 proj \xc2\xb7 m \xc2\xb7 working\x07"
        );

        tracker.clear_guarded(true, &mut out).expect("clear");
        let bytes_after_clear = out.len();
        tracker.clear_guarded(true, &mut out).expect("clear");
        assert_eq!(out.len(), bytes_after_clear);
        assert_eq!(
            &out[bytes_after_clear - 5..],
            b"\x1b]0;\x07",
            "the trailing write is exactly one empty OSC title"
        );
        assert_eq!(tracker.last_written(), None);
    }

    #[test]
    fn tracker_clear_after_no_write_emits_nothing() {
        let mut tracker = TerminalTitleState::new();
        let mut out = Vec::new();
        tracker.clear_guarded(true, &mut out).expect("clear");
        assert!(out.is_empty());
    }

    #[test]
    fn non_terminal_stdout_is_never_touched() {
        let mut tracker = TerminalTitleState::new();
        let mut out = Vec::new();
        tracker
            .refresh_guarded(false, &mut out, &status("proj", "m", Activity::Working))
            .expect("refresh");
        assert!(out.is_empty());
        assert_eq!(tracker.last_written(), None);
        // A later clear on the same non-terminal target stays a no-op.
        tracker.clear_guarded(false, &mut out).expect("clear");
        assert!(out.is_empty());
    }
}
