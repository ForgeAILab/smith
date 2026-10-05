// Inline screens borrow the session pane without taking its composer or transcript.

struct EmbeddedFixture {
    rows: u16,
}

impl crate::screen::Screen for EmbeddedFixture {
    type Outcome = ();
    type Effect = std::convert::Infallible;
    fn draw(&self, frame: &mut ratatui::Frame<'_>, area: Rect, theme: Theme) {
        frame.render_widget(
            Paragraph::new("Connect OpenRouter\nCredential method")
                .style(theme.style(Tone::Heading)),
            area,
        );
    }
    fn content_height(&self, _width: u16) -> u16 {
        self.rows
    }
    fn footer(&self) -> Option<crate::picker::ScreenFooter> {
        Some(crate::picker::ScreenFooter::Progress { back: false })
    }
    fn on_event(
        &mut self,
        _event: crate::screen::ScreenEvent,
    ) -> crate::screen::Step<Self::Outcome, Self::Effect> {
        crate::screen::Step::Pending
    }
}

#[test]
fn embedded_screen_reserves_rows_without_covering_the_draft_or_transcript() {
    for (width, height) in [(44, 16), (100, 32)] {
        for rows in [2, 8, u16::MAX] {
            let mut app = App::new("gpt-5.3", "~/work/api");
            app.transcript.push_user("retained transcript");
            app.composer.insert_str("retained draft");
            let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
            let mut region = Rect::default();
            terminal
                .draw(|frame| {
                    region = draw_with_screen(
                        frame,
                        &app,
                        &EmbeddedFixture { rows },
                        Theme::new().without_color(),
                    );
                })
                .expect("draw");
            let text = screen_text(terminal.backend().buffer());
            for expected in [
                "retained transcript",
                "Connect OpenRouter",
                "retained draft",
                "gpt-5.3",
                "esc cancel",
            ] {
                assert!(text.contains(expected), "{text}");
            }
            let draft_row = text
                .lines()
                .position(|row| row.contains("retained draft"))
                .expect("draft row");
            assert!(
                usize::from(region.bottom()) <= draft_row,
                "{region:?}\n{text}"
            );
            assert!(region.y >= 3, "transcript minimum: {region:?}");
            assert!(region.height <= rows);
            assert_eq!(app.composer.text(), "retained draft");
        }
    }
}
