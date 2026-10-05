use super::*;

impl Screen for SetupApp {
    type Outcome = ();
    type Effect = SetupEffect;

    fn step_key(&self) -> u64 {
        self.step_generation
    }

    fn draw(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
        draw_setup_in_area(frame, area, self, theme);
    }

    fn draw_embedded(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
        draw_setup_surface(frame, area, self, theme, true);
    }

    fn content_height(&self, width: u16) -> u16 {
        if let Some(picker) = &self.picker {
            return picker_content_height(
                picker,
                width,
                (self.step == Step::Action)
                    .then_some("Nothing is sent to a provider until setup finishes."),
                self.error.as_deref(),
            );
        }
        u16::try_from(
            setup_content_rows(self, width, Theme::new())
                .len()
                .saturating_add(2),
        )
        .unwrap_or(u16::MAX)
    }

    fn footer(&self) -> Option<ScreenFooter> {
        let back = !self.history.is_empty();
        Some(match self.step {
            Step::Review => {
                let scroll = self.review_scroll.get();
                ScreenFooter::Review {
                    back,
                    scroll: (scroll.limit > 0).then_some((scroll.offset + 1, scroll.limit + 1)),
                }
            }
            Step::Busy => ScreenFooter::Busy { back },
            _ => self
                .picker
                .as_ref()
                .map(ResourcePicker::footer)
                .unwrap_or(ScreenFooter::Field { back }),
        })
    }

    fn on_event(&mut self, event: ScreenEvent) -> ScreenStep<Self::Outcome, Self::Effect> {
        match event {
            ScreenEvent::Key(key) => match self.on_key(key) {
                SetupEffect::None => ScreenStep::Pending,
                SetupEffect::Cancel => ScreenStep::Outcome(()),
                effect => ScreenStep::Effect(effect),
            },
            ScreenEvent::Paste(text) => {
                self.on_paste(&text);
                ScreenStep::Pending
            }
            ScreenEvent::Resize(_, _) | ScreenEvent::Tick => ScreenStep::Pending,
        }
    }
}

/// Draws the complete setup surface.
pub fn draw_setup(frame: &mut Frame<'_>, app: &SetupApp, theme: Theme) {
    let area = frame.area();
    app.draw(frame, area, theme);
}

fn draw_setup_in_area(frame: &mut Frame<'_>, area: Rect, app: &SetupApp, theme: Theme) {
    draw_setup_surface(frame, area, app, theme, false);
}

fn draw_setup_surface(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &SetupApp,
    theme: Theme,
    embedded: bool,
) {
    let back = !app.history.is_empty();
    let base_title = app.title.as_deref().unwrap_or("Smith setup");
    if let Some(picker) = &app.picker {
        let title = if app.step == Step::Action {
            format!("{base_title} · Welcome · choose how to connect a model")
        } else {
            format!("{base_title} · {}", picker.title)
        };
        draw_picker_context(
            frame,
            area,
            picker,
            PickerContext {
                title: &title,
                note: (app.step == Step::Action)
                    .then_some("Nothing is sent to a provider until setup finishes."),
                error: app.error.as_deref(),
                embedded,
            },
            theme,
        );
        return;
    }
    let heading = match app.step {
        Step::Review => "Review · nothing is written until you confirm",
        Step::Busy => "Applying setup",
        _ => app.prompt().0,
    };
    let mut rows = setup_content_rows(app, area.width, theme);
    let mut footer = match app.step {
        Step::Review => ScreenFooter::Review { back, scroll: None },
        Step::Busy => ScreenFooter::Busy { back },
        _ => ScreenFooter::Field { back },
    };
    let base_page = usize::from(area.height).saturating_sub(if embedded {
        2
    } else {
        3 + footer.rows(area.width).len()
    });
    if app.step == Step::Review {
        if rows.len() > base_page {
            footer = ScreenFooter::Review {
                back,
                scroll: Some((1, 1)),
            };
        }
        let page = rows
            .len()
            .min(usize::from(area.height).saturating_sub(if embedded {
                2
            } else {
                3 + footer.rows(area.width).len()
            }));
        let limit = rows.len().saturating_sub(page);
        let offset = app.review_scroll.get().offset.min(limit);
        app.review_scroll.set(ReviewScroll {
            offset,
            limit,
            page,
        });
        footer = ScreenFooter::Review {
            back,
            scroll: (limit > 0).then_some((offset + 1, limit + 1)),
        };
        let body = draw_inline_screen(
            frame,
            area,
            &format!("{base_title} · {heading}"),
            rows.len(),
            (!embedded).then_some(footer),
            theme,
        );
        frame.render_widget(
            Paragraph::new(
                rows.into_iter()
                    .skip(offset)
                    .take(usize::from(body.height))
                    .collect::<Vec<_>>(),
            ),
            body,
        );
    } else {
        let body = draw_inline_screen(
            frame,
            area,
            &format!("{base_title} · {heading}"),
            rows.len(),
            (!embedded).then_some(footer),
            theme,
        );
        if app.step != Step::Busy
            && let Some(row) = rows
                .iter()
                .position(|line| line.to_string().starts_with("› "))
        {
            // The editable value must remain reachable when help exceeds the
            // small-terminal body; discard only the offscreen help prefix.
            let row = if row >= usize::from(body.height) && body.height > 0 {
                let skipped = row + 1 - usize::from(body.height);
                rows.drain(..skipped);
                row - skipped
            } else {
                row
            };
            let field = if app.prompt().2 {
                &app.secret.0
            } else {
                &app.input
            };
            let (_, cursor) = field.viewport(usize::from(body.width).saturating_sub(2));
            if row < usize::from(body.height) && body.width > 2 {
                frame.set_cursor_position((body.x + 2 + cursor as u16, body.y + row as u16));
            }
        }
        frame.render_widget(Paragraph::new(rows), body);
    }
}

fn setup_content_rows(app: &SetupApp, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match app.step {
        Step::Review | Step::Busy => {
            if app.step == Step::Busy {
                lines.extend(indented_words(
                    app.busy_note()
                        .unwrap_or("Writing your choices and checking the connection…"),
                    width,
                    2,
                    Tone::Accent,
                    theme,
                ));
                lines.push(Line::default());
            }
            lines.extend(app.wrapped_review_lines(width));
        }
        _ => {
            let (_, help, masked) = app.prompt();
            if app.error.is_none() {
                lines.extend(indented_words(&help, width, 2, Tone::Dim, theme));
                lines.push(Line::default());
            }
            let field = if masked { &app.secret.0 } else { &app.input };
            let (visible, _) = field.viewport(usize::from(width).saturating_sub(2));
            let value = if masked {
                visible.clone()
            } else if app.input.is_empty() {
                "type a value".to_owned()
            } else {
                visible
            };
            lines.push(Line::from(vec![
                Span::styled("› ", theme.style(Tone::Accent)),
                Span::styled(
                    value,
                    theme.style(if !masked && app.input.is_empty() {
                        Tone::Dim
                    } else {
                        Tone::Default
                    }),
                ),
            ]));
        }
    }
    if let Some(error) = &app.error {
        lines.push(Line::default());
        lines.extend(indented_words(
            &format!("error: {error}"),
            width,
            2,
            Tone::Danger,
            theme,
        ));
    }
    crate::render::wrap::wrap_lines(&lines, width)
}
