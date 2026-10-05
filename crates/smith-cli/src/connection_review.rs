//! Secret-free OAuth configuration review before either half of the transaction writes.

use std::cell::Cell;
use std::convert::Infallible;
use std::hash::{DefaultHasher, Hash, Hasher};

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Wrap};
use smith_tui::picker::{ScreenFooter, draw_inline_screen};
use smith_tui::{FlowOutcome, Screen, ScreenEvent, Step, Theme};

pub(super) struct ConnectionReview {
    title: String,
    preview: String,
    viewport: Cell<Viewport>,
}

#[derive(Clone, Copy, Default)]
struct Viewport {
    offset: usize,
    limit: usize,
    page: usize,
}

impl ConnectionReview {
    pub(super) fn new(title: &str, preview: String) -> Self {
        Self {
            title: format!("{title} · Review"),
            preview,
            viewport: Cell::new(Viewport::default()),
        }
    }

    pub(super) fn saving(&mut self) {
        self.title = self.title.replace(" · Review", " · Saving connection…");
    }

    fn paragraph(&self) -> Paragraph<'_> {
        Paragraph::new(self.preview.as_str()).wrap(Wrap { trim: false })
    }

    fn draw_surface(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme, embedded: bool) {
        let rows = self.paragraph().line_count(area.width.saturating_sub(2));
        let mut footer = ScreenFooter::Review {
            back: true,
            scroll: None,
        };
        let mut page = usize::from(area.height).saturating_sub(if embedded {
            2
        } else {
            3 + footer.rows(area.width).len()
        });
        if rows > page {
            footer = ScreenFooter::Review {
                back: true,
                scroll: Some((1, 1)),
            };
            page = usize::from(area.height).saturating_sub(if embedded {
                2
            } else {
                3 + footer.rows(area.width).len()
            });
        }
        let limit = rows.saturating_sub(page);
        let offset = self.viewport.get().offset.min(limit);
        self.viewport.set(Viewport {
            offset,
            limit,
            page,
        });
        let body = draw_inline_screen(
            frame,
            area,
            &self.title,
            rows,
            if embedded { None } else { self.footer() },
            theme,
        );
        let body = Rect::new(
            body.x.saturating_add(2),
            body.y,
            body.width.saturating_sub(2),
            body.height,
        );
        frame.render_widget(
            self.paragraph()
                .scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
            body,
        );
    }
}

impl Screen for ConnectionReview {
    type Outcome = FlowOutcome<()>;
    type Effect = Infallible;

    /// Saving replaces review as a step, but scrolling must not clear the surface.
    fn step_key(&self) -> u64 {
        let mut key = DefaultHasher::new();
        self.title.hash(&mut key);
        key.finish()
    }

    fn draw(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
        self.draw_surface(frame, area, theme, false);
    }
    fn draw_embedded(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
        self.draw_surface(frame, area, theme, true);
    }
    fn content_height(&self, width: u16) -> u16 {
        u16::try_from(
            self.paragraph()
                .line_count(width.saturating_sub(2))
                .saturating_add(2),
        )
        .unwrap_or(u16::MAX)
    }
    fn footer(&self) -> Option<ScreenFooter> {
        let viewport = self.viewport.get();
        Some(ScreenFooter::Review {
            back: true,
            scroll: (viewport.limit > 0).then_some((viewport.offset + 1, viewport.limit + 1)),
        })
    }

    fn on_event(&mut self, event: ScreenEvent) -> Step<Self::Outcome, Self::Effect> {
        let ScreenEvent::Key(key) = event else {
            return Step::Pending;
        };
        if key.kind == KeyEventKind::Release {
            return Step::Pending;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Step::Outcome(FlowOutcome::Cancelled);
        }
        let mut viewport = self.viewport.get();
        match key.code {
            KeyCode::Enter => return Step::Outcome(FlowOutcome::Completed(())),
            KeyCode::Esc | KeyCode::BackTab => return Step::Outcome(FlowOutcome::Back),
            KeyCode::Up => viewport.offset = viewport.offset.saturating_sub(1),
            KeyCode::Down => {
                viewport.offset = viewport.offset.saturating_add(1).min(viewport.limit)
            }
            KeyCode::PageUp => {
                viewport.offset = viewport.offset.saturating_sub(viewport.page.max(1))
            }
            KeyCode::PageDown => {
                viewport.offset = viewport
                    .offset
                    .saturating_add(viewport.page.max(1))
                    .min(viewport.limit)
            }
            KeyCode::Home => viewport.offset = 0,
            KeyCode::End => viewport.offset = viewport.limit,
            _ => {}
        }
        self.viewport.set(viewport);
        Step::Pending
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use smith_tui::{FlowOutcome, Screen, ScreenEvent, Step};

    #[test]
    fn connection_review_requires_confirmation_and_can_back_out() {
        let mut review = super::ConnectionReview::new(
            "Connect ChatGPT · experimental",
            "destination: ~/.smith/config.toml".into(),
        );
        let review_key = review.step_key();
        review.on_event(ScreenEvent::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert_eq!(review.step_key(), review_key);
        for (code, modifiers, expected) in [
            (KeyCode::Esc, KeyModifiers::NONE, FlowOutcome::Back),
            (
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
                FlowOutcome::Cancelled,
            ),
            (
                KeyCode::Enter,
                KeyModifiers::NONE,
                FlowOutcome::Completed(()),
            ),
        ] {
            assert!(
                matches!(review.on_event(ScreenEvent::Key(KeyEvent::new(code, modifiers))), Step::Outcome(outcome) if outcome == expected)
            );
        }
        review.saving();
        assert_ne!(review.step_key(), review_key);
    }
}
