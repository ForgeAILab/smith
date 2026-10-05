//! Shared public OAuth instructions, kept separate from tokens and polling work.

use std::convert::Infallible;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use smith_tui::picker::{ScreenFooter, draw_inline_screen};
use smith_tui::theme::Tone;
use smith_tui::{FlowOutcome, Screen, ScreenEvent, Step, Theme};

/// Only public authorization instructions enter this value; the runner owns polling.
pub(super) struct LoginProgress {
    title: String,
    instructions: Vec<String>,
    waiting: String,
    frame_number: usize,
    no_motion: bool,
}

impl LoginProgress {
    pub(super) fn new(
        title: &str,
        instructions: Vec<String>,
        waiting: &str,
        no_motion: bool,
    ) -> Self {
        Self {
            title: title.to_owned(),
            instructions,
            waiting: waiting.to_owned(),
            frame_number: 0,
            no_motion,
        }
    }

    fn lines(&self) -> Vec<String> {
        let mut lines = self.instructions.clone();
        let dots = if self.no_motion {
            "…".to_owned()
        } else {
            ".".repeat(self.frame_number % 3 + 1)
        };
        lines.push(format!("{}{dots}", self.waiting));
        lines
    }
}

impl Screen for LoginProgress {
    type Outcome = FlowOutcome<()>;
    type Effect = Infallible;

    /// Public instructions identify the login step, while animation stays incremental.
    fn step_key(&self) -> u64 {
        let mut key = DefaultHasher::new();
        (&self.title, &self.instructions).hash(&mut key);
        key.finish()
    }

    fn draw(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
        draw_progress(frame, area, &self.title, self.lines(), false, false, theme);
    }

    fn draw_embedded(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
        draw_progress(frame, area, &self.title, self.lines(), false, true, theme);
    }

    fn content_height(&self, width: u16) -> u16 {
        progress_height(self.lines(), width)
    }

    fn footer(&self) -> Option<ScreenFooter> {
        Some(ScreenFooter::Progress { back: false })
    }

    fn on_event(&mut self, event: ScreenEvent) -> Step<Self::Outcome, Self::Effect> {
        progress_event(event, &mut self.frame_number, false)
    }

    fn tick_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(250))
    }
}

/// Dropping the runner's raced future on either navigation outcome cancels polling.
pub(super) fn progress_event<T>(
    event: ScreenEvent,
    frame_number: &mut usize,
    back: bool,
) -> Step<FlowOutcome<T>, Infallible> {
    match event {
        ScreenEvent::Key(key) if key.kind != KeyEventKind::Release => {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                Step::Outcome(FlowOutcome::Cancelled)
            } else if key.code == KeyCode::Esc || key.code == KeyCode::BackTab {
                Step::Outcome(if back {
                    FlowOutcome::Back
                } else {
                    FlowOutcome::Cancelled
                })
            } else {
                Step::Pending
            }
        }
        ScreenEvent::Tick => {
            *frame_number = frame_number.wrapping_add(1);
            Step::Pending
        }
        _ => Step::Pending,
    }
}

fn progress_body(mut lines: Vec<String>, width: u16) -> (Paragraph<'static>, String, usize) {
    let waiting = lines.pop().unwrap_or_default();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let paragraph = Paragraph::new(lines.join("\n")).wrap(Wrap { trim: false });
    let rows = paragraph.line_count(width.saturating_sub(2));
    (paragraph, waiting, rows)
}

/// Use renderer wrapping to grow only as far as the public instructions need.
pub(super) fn progress_height(lines: Vec<String>, width: u16) -> u16 {
    let (_, _, rows) = progress_body(lines, width);
    u16::try_from(rows.saturating_add(4)).unwrap_or(u16::MAX)
}

/// Waiting is reserved independently, so a long URL never hides progress or cancel.
pub(super) fn draw_progress(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    lines: Vec<String>,
    back: bool,
    embedded: bool,
    theme: Theme,
) {
    let (paragraph, waiting, rows) = progress_body(lines, area.width);
    let body = draw_inline_screen(
        frame,
        area,
        title,
        rows + 2,
        (!embedded).then_some(ScreenFooter::Progress { back }),
        theme,
    );
    let width = area.width.saturating_sub(2);
    frame.render_widget(
        paragraph,
        Rect::new(
            body.x.saturating_add(2),
            body.y,
            width,
            body.height.saturating_sub(2),
        ),
    );
    if body.height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  {waiting}"),
                theme.style(Tone::Dim),
            ))),
            Rect::new(body.x, body.bottom().saturating_sub(1), body.width, 1),
        );
    }
}
