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

/// The glyph vocabulary. The reviewed transcript grammar is single-width
/// and never emoji-capable, so terminal presentation cannot shift its columns.
/// Other regions retain their own marks beside that table.
pub mod glyph {
    /// The reviewed transcript grammar, in speaker, result, work, selection,
    /// success, and elision order.
    pub const GRAMMAR: [&str; 7] = [">", "●", "⎿", "✻", "❯", "✓", "…"];
    /// Prefixes a user message.
    pub const USER: &str = GRAMMAR[0];
    /// Prefixes the composer, whose grammar is owned by its own surface.
    pub const INPUT: &str = "›";
    /// Prefixes assistant text and quiet informational rows.
    pub const BULLET: &str = GRAMMAR[1];
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
    pub const BRANCH: &str = GRAMMAR[2];
    /// Prefixes active work and an ephemeral successful-turn summary.
    pub const WORK: &str = GRAMMAR[3];
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
    pub const ELIDED: &str = GRAMMAR[6];
    /// The static stand-in for the spinner under reduced motion.
    pub const STILL: &str = BULLET;
    /// Marks the agent whose conversation the surface currently shows.
    pub const AGENT_CURRENT: &str = BULLET;
    /// Marks another agent in the delegated-agents panel. `⏺`/`◯` would
    /// match Claude's marks but are East-Asian Wide/Ambiguous and would
    /// misalign the panel by a column, as `⚡` would the header.
    pub const AGENT_OTHER: &str = "○";
    /// Separates header segments.
    pub const SEPARATOR: &str = "·";
    /// Marks system-instruction context.
    pub const CONTEXT_SYSTEM: &str = "■";
    /// Marks tool-schema context.
    pub const CONTEXT_TOOL: &str = "◆";
    /// Marks prior conversation context.
    pub const CONTEXT_HISTORY: &str = BULLET;
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
    /// Marks a provider request on its way up, with nothing back yet.
    pub const SENDING: &str = "↑";
    /// Marks streamed answer text arriving.
    pub const RECEIVING: &str = "↓";
    /// The spinner frames, 100 ms apart.
    pub const SPINNER: [&str; 4] = [WORK, "✼", "✽", "✼"];
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

    /// The style marking pointer-selected cells.
    ///
    /// Reversed video rather than a background color: it is the one highlight
    /// that must stay legible over every tone the surface already uses, and it
    /// survives `--no-color` unchanged because reversal is an attribute, not a
    /// hue. This is also what terminals use for their own selections, so the
    /// Smith-owned one looks like the native one it replaced.
    pub fn selection(self) -> Style {
        Style::default().add_modifier(Modifier::REVERSED)
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
            glyph::INPUT,
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
            glyph::SENDING,
            glyph::RECEIVING,
        ];
        glyphs.extend_from_slice(&glyph::GRAMMAR);
        glyphs.extend_from_slice(&glyph::SPINNER);
        for glyph in glyphs {
            assert_eq!(
                UnicodeWidthStr::width(glyph),
                1,
                "`{glyph}` is not single-width; it would break column alignment"
            );
        }
    }

    // Unicode 17 Emoji and Emoji_Presentation, plus the pictograph blocks.
    // https://www.unicode.org/Public/17.0.0/ucd/emoji/emoji-data.txt
    fn emoji_capable(character: char) -> bool {
        matches!(character as u32,
            0x23
            | 0x2A
            | 0x30..=0x39
            | 0xA9
            | 0xAE
            | 0x203C
            | 0x2049
            | 0x2122
            | 0x2139
            | 0x2194..=0x2199
            | 0x21A9..=0x21AA
            | 0x231A..=0x231B
            | 0x2328
            | 0x23CF
            | 0x23E9..=0x23F3
            | 0x23F8..=0x23FA
            | 0x24C2
            | 0x25AA..=0x25AB
            | 0x25B6
            | 0x25C0
            | 0x25FB..=0x25FE
            | 0x2600..=0x2604
            | 0x260E
            | 0x2611
            | 0x2614..=0x2615
            | 0x2618
            | 0x261D
            | 0x2620
            | 0x2622..=0x2623
            | 0x2626
            | 0x262A
            | 0x262E..=0x262F
            | 0x2638..=0x263A
            | 0x2640
            | 0x2642
            | 0x2648..=0x2653
            | 0x265F..=0x2660
            | 0x2663
            | 0x2665..=0x2666
            | 0x2668
            | 0x267B
            | 0x267E..=0x267F
            | 0x2692..=0x2697
            | 0x2699
            | 0x269B..=0x269C
            | 0x26A0..=0x26A1
            | 0x26A7
            | 0x26AA..=0x26AB
            | 0x26B0..=0x26B1
            | 0x26BD..=0x26BE
            | 0x26C4..=0x26C5
            | 0x26C8
            | 0x26CE..=0x26CF
            | 0x26D1
            | 0x26D3..=0x26D4
            | 0x26E9..=0x26EA
            | 0x26F0..=0x26F5
            | 0x26F7..=0x26FA
            | 0x26FD
            | 0x2702
            | 0x2705
            | 0x2708..=0x270D
            | 0x270F
            | 0x2712
            | 0x2714
            | 0x2716
            | 0x271D
            | 0x2721
            | 0x2728
            | 0x2733..=0x2734
            | 0x2744
            | 0x2747
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2763..=0x2764
            | 0x2795..=0x2797
            | 0x27A1
            | 0x27B0
            | 0x27BF
            | 0x2934..=0x2935
            | 0x2B05..=0x2B07
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0x1F000..=0x1FAFF
        )
    }

    fn grammar_glyph_is_safe(glyph: &str) -> bool {
        glyph.chars().count() == 1
            && UnicodeWidthStr::width(glyph) == 1
            && !glyph.chars().any(emoji_capable)
    }

    #[test]
    fn grammar_glyphs_are_single_width_and_never_emoji_capable() {
        for glyph in glyph::GRAMMAR.into_iter().chain(glyph::SPINNER) {
            assert!(
                grammar_glyph_is_safe(glyph),
                "unsafe grammar glyph: {glyph}"
            );
        }
    }

    #[test]
    fn grammar_rejects_emoji_even_when_it_has_text_presentation() {
        for glyph in ["⏺", "✳", "✔", "⚠", "#", "1", "😀", "\u{1faff}"] {
            assert!(
                !grammar_glyph_is_safe(glyph),
                "accepted emoji-capable glyph: {glyph}"
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
        assert_eq!(
            moving.spinner(0),
            moving.spinner(glyph::SPINNER.len() as u64)
        );
    }
}
