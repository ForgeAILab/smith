//! The visual vocabulary from `DESIGN.md` §3 and §4.
//!
//! Two rules drive everything here:
//!
//! - **Named ANSI colors only.** Smith asks the terminal for "cyan" and lets
//!   the theme decide what cyan is. That is what makes light and dark terminals
//!   both work without detecting either.
//! - **Color is never the only channel.** Every [`Tone`] pairs with a glyph or
//!   a word at the call site, so `--no-color` and monochrome screenshots stay
//!   fully legible.

use ratatui::style::{Color, Modifier, Style};

/// A semantic color token. Call sites name intent, not color, so the palette
/// can change in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Transcript body and assistant text.
    Default,
    /// Timestamps, hints, and secondary detail.
    Dim,
    /// The user marker, focus edge, and selection.
    Accent,
    /// A local slash-command label.
    Command,
    /// Confirmed, succeeded, cache hit.
    Success,
    /// Estimated values, degraded capability, unread background work.
    Warning,
    /// Errors, denials, destructive targets.
    Danger,
    /// Compact in-flight model progress.
    Reasoning,
    /// Inline and fenced code.
    Code,
    /// A rendered hyperlink.
    Link,
    /// The active model in the compact footer.
    StatusModel,
    /// The working directory in the compact footer.
    StatusPath,
    /// Structural headings and modal titles.
    Heading,
}

/// The glyph vocabulary. Every entry is ASCII or a verified single-width
/// codepoint — no emoji, whose width varies by terminal and would corrupt
/// column alignment.
pub mod glyph {
    /// Prefixes a user message.
    pub const USER: &str = "›";
    /// Prefixes assistant text and quiet informational rows.
    pub const BULLET: &str = "•";
    /// Prefixes model reasoning.
    pub const REASONING: &str = BULLET;
    /// Prefixes a tool call.
    pub const TOOL: &str = BULLET;
    /// Prefixes an error.
    pub const ERROR: &str = "■";
    /// Prefixes a warning.
    pub const WARNING: &str = "⚠";
    /// Prefixes a background notification.
    pub const NOTICE: &str = BULLET;
    /// Prefixes tool output or other detail belonging to the prior row.
    pub const BRANCH: &str = "└";
    /// Prefixes a wrapped command continuation.
    pub const CONTINUATION: &str = "│";
    /// Prefixes an approval request.
    pub const APPROVAL: &str = "?";
    /// Marks an observed cache read. Narrow by design: `⚡` (U+26A1) is
    /// East-Asian Wide and would misalign the header by a column.
    pub const CACHE: &str = "⌁";
    /// Marks a line an edit removes.
    pub const REMOVED: &str = "-";
    /// Marks a line an edit adds.
    pub const ADDED: &str = "+";
    /// Marks output that was left out: collapsed context, or a review the
    /// modal had no room for.
    pub const ELIDED: &str = "…";
    /// The static stand-in for the spinner under reduced motion.
    pub const STILL: &str = "●";
    /// Separates header segments.
    pub const SEPARATOR: &str = "·";
    /// Marks system-instruction context.
    pub const CONTEXT_SYSTEM: &str = "■";
    /// Marks tool-schema context.
    pub const CONTEXT_TOOL: &str = "◆";
    /// Marks prior conversation context.
    pub const CONTEXT_HISTORY: &str = "●";
    /// Marks compacted summary context.
    pub const CONTEXT_SUMMARY: &str = "▲";
    /// Marks the current user input.
    pub const CONTEXT_INPUT: &str = "✦";
    /// Marks another runtime-defined context category.
    pub const CONTEXT_OTHER: &str = "+";
    /// Marks unused input capacity.
    pub const CONTEXT_FREE: &str = "·";
    /// Marks output and reasoning capacity reserved outside the input budget.
    pub const CONTEXT_RESERVE: &str = "□";
    /// The spinner frames, 100 ms apart.
    pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
}

/// Resolves [`Tone`]s to styles, honoring the no-color and reduced-motion
/// contracts from `DESIGN.md` §4 and §6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    color: bool,
    motion: bool,
}

impl Theme {
    /// The full-color, full-motion theme.
    pub fn new() -> Self {
        Self {
            color: true,
            motion: true,
        }
    }

    /// Reads `NO_COLOR`, `NO_MOTION`, and `TERM=dumb` from the environment.
    ///
    /// Per the `NO_COLOR` convention, the variable disables color when it is
    /// present and non-empty, whatever its value.
    pub fn from_env() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        let dumb = std::env::var("TERM").is_ok_and(|term| term == "dumb");
        Self {
            color: !set("NO_COLOR") && !dumb,
            motion: !set("NO_MOTION") && !dumb,
        }
    }

    /// Disables hue while preserving typographic structure.
    pub fn without_color(mut self) -> Self {
        self.color = false;
        self
    }

    /// Disables the spinner and sub-second timer updates.
    pub fn without_motion(mut self) -> Self {
        self.motion = false;
        self
    }

    /// Whether color attributes are emitted.
    pub fn uses_color(self) -> bool {
        self.color
    }

    /// Whether animation is permitted.
    pub fn uses_motion(self) -> bool {
        self.motion
    }

    /// The style for a tone.
    pub fn style(self, tone: Tone) -> Style {
        // Typographic modifiers survive `--no-color`: they carry structure,
        // not hue, and remain readable when the palette is unavailable.
        let plain = match tone {
            Tone::Dim => Style::default().add_modifier(Modifier::DIM),
            Tone::Reasoning => Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC),
            Tone::Accent => Style::default().add_modifier(Modifier::BOLD),
            Tone::Link => Style::default().add_modifier(Modifier::UNDERLINED),
            Tone::Heading => Style::default().add_modifier(Modifier::BOLD),
            _ => Style::default(),
        };
        if !self.color {
            return plain;
        }
        match tone {
            Tone::Default => Style::default(),
            Tone::Dim => plain,
            Tone::Accent => plain.fg(Color::Cyan),
            Tone::Command => Style::default().fg(Color::Magenta),
            Tone::Success => Style::default().fg(Color::Green),
            Tone::Warning => Style::default().fg(Color::Yellow),
            Tone::Danger => Style::default().fg(Color::Red),
            Tone::Reasoning => plain,
            Tone::Code => Style::default().fg(Color::Cyan),
            Tone::Link => plain.fg(Color::Cyan),
            Tone::StatusModel => Style::default().fg(Color::Cyan),
            Tone::StatusPath => Style::default().fg(Color::Green),
            Tone::Heading => plain,
        }
    }

    /// The spinner frame for a tick, or the static glyph under reduced motion.
    pub fn spinner(self, tick: u64) -> &'static str {
        if !self.motion {
            return glyph::STILL;
        }
        let frames = glyph::SPINNER;
        frames[(tick as usize) % frames.len()]
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn every_glyph_occupies_exactly_one_column() {
        let mut glyphs = vec![
            glyph::USER,
            glyph::BULLET,
            glyph::REASONING,
            glyph::TOOL,
            glyph::ERROR,
            glyph::WARNING,
            glyph::NOTICE,
            glyph::BRANCH,
            glyph::CONTINUATION,
            glyph::APPROVAL,
            glyph::CACHE,
            glyph::REMOVED,
            glyph::ADDED,
            glyph::ELIDED,
            glyph::STILL,
            glyph::SEPARATOR,
            glyph::CONTEXT_SYSTEM,
            glyph::CONTEXT_TOOL,
            glyph::CONTEXT_HISTORY,
            glyph::CONTEXT_SUMMARY,
            glyph::CONTEXT_INPUT,
            glyph::CONTEXT_OTHER,
            glyph::CONTEXT_FREE,
            glyph::CONTEXT_RESERVE,
        ];
        glyphs.extend_from_slice(&glyph::SPINNER);
        for glyph in glyphs {
            assert_eq!(
                UnicodeWidthStr::width(glyph),
                1,
                "`{glyph}` is not single-width; it would break column alignment"
            );
        }
    }

    #[test]
    fn no_color_keeps_dim_and_bold_but_drops_hue() {
        let theme = Theme::new().without_color();
        assert_eq!(theme.style(Tone::Danger), Style::default());
        assert_eq!(
            theme.style(Tone::Dim),
            Style::default().add_modifier(Modifier::DIM)
        );
        assert_eq!(
            theme.style(Tone::Heading),
            Style::default().add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            theme.style(Tone::Reasoning),
            Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC)
        );
        assert_eq!(
            theme.style(Tone::Link),
            Style::default().add_modifier(Modifier::UNDERLINED)
        );
    }

    #[test]
    fn color_mode_assigns_distinct_hues_to_distinct_meanings() {
        let theme = Theme::new();
        assert_eq!(theme.style(Tone::Success).fg, Some(Color::Green));
        assert_eq!(theme.style(Tone::Danger).fg, Some(Color::Red));
        assert_eq!(theme.style(Tone::Warning).fg, Some(Color::Yellow));
        assert_eq!(theme.style(Tone::Command).fg, Some(Color::Magenta));
        assert_eq!(theme.style(Tone::Code).fg, Some(Color::Cyan));
        assert_eq!(theme.style(Tone::StatusModel).fg, Some(Color::Cyan));
        assert_eq!(theme.style(Tone::StatusPath).fg, Some(Color::Green));
        assert_ne!(theme.style(Tone::Success), theme.style(Tone::Danger));
    }

    #[test]
    fn reduced_motion_freezes_the_spinner() {
        let still = Theme::new().without_motion();
        assert_eq!(still.spinner(0), glyph::STILL);
        assert_eq!(still.spinner(7), glyph::STILL);

        let moving = Theme::new();
        assert_ne!(moving.spinner(0), moving.spinner(1));
        assert_eq!(moving.spinner(0), moving.spinner(10));
    }
}
