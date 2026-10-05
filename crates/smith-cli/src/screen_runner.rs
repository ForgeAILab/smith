//! One event loop for standalone screens and screens borrowing an idle session.

use std::cell::Cell;
use std::convert::Infallible;
use std::future::{Future, Pending, pending};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event as TermEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use smith_tui::{App, Screen, ScreenEvent, Step, Theme};
use tokio::time::Interval;

use crate::terminal::{self, Terminal};

/// Resolves presentation flags once so standalone and session surfaces agree.
pub(crate) fn theme_from_flags(no_color: bool, no_motion: bool) -> Theme {
    let mut theme = Theme::from_env();
    if no_color {
        theme = theme.without_color();
    }
    if no_motion {
        theme = theme.without_motion();
    }
    theme
}

/// Retains the existing error wording at each screen's input and draw boundary.
#[derive(Clone, Copy)]
pub(crate) struct ScreenContext {
    /// Preserve screen-specific draw failures without adding wording to generic pickers.
    pub(crate) draw: Option<&'static str>,
    /// Identify the existing input boundary when the event reader fails.
    pub(crate) input: &'static str,
}

/// Distinguishes screen steps, external completion, and input ending for callers.
pub(crate) enum ScreenResult<Outcome, Effect, Completion = Infallible> {
    /// Pass a selection or cancellation back to its owning flow.
    Outcome(Outcome),
    /// Return work without giving up the terminal or event reader.
    Effect(Effect),
    /// Pass through the raced future's output, including its own error type.
    Completed(Completion),
    /// Let each flow retain its existing behavior when terminal input ends.
    InputEnded,
}

/// Record cancellation without abandoning a transaction halfway through enrollment.
#[derive(Default)]
pub(crate) struct EffectCancellation(Cell<bool>);

impl EffectCancellation {
    pub(crate) fn requested(&self) -> bool {
        self.0.get()
    }
    fn request(&self) {
        self.0.set(true);
    }
}

/// Keeps a single event reader and terminal guard alive across a chain of screens.
pub(crate) struct ScreenSession<'a> {
    surface: Surface<'a>,
    events: Box<dyn futures_util::Stream<Item = std::io::Result<TermEvent>> + Unpin + Send>,
    tick: Option<Interval>,
    theme: Theme,
}

enum Surface<'a> {
    Standalone(Terminal),
    Embedded {
        terminal: &'a mut Terminal,
        app: &'a App,
    },
    #[cfg(test)]
    Testing {
        terminal: &'a mut ratatui::Terminal<ratatui::backend::TestBackend>,
        app: &'a App,
    },
}

impl ScreenSession<'static> {
    /// Enters once; the terminal guard also restores on errors, cancellation, or panic.
    pub(crate) fn enter(no_color: bool, no_motion: bool) -> Result<Self> {
        let terminal = terminal::enter()?;
        Ok(Self::new(
            Surface::Standalone(terminal),
            no_color,
            no_motion,
        ))
    }
}

impl<'a> ScreenSession<'a> {
    /// Borrows the idle session's terminal and backdrop without changing terminal modes.
    pub(crate) fn embedded(
        terminal: &'a mut Terminal,
        app: &'a App,
        no_color: bool,
        no_motion: bool,
    ) -> Self {
        Self::new(Surface::Embedded { terminal, app }, no_color, no_motion)
    }

    fn new(surface: Surface<'a>, no_color: bool, no_motion: bool) -> Self {
        Self {
            surface,
            events: Box::new(EventStream::new()),
            tick: None,
            theme: theme_from_flags(no_color, no_motion),
        }
    }

    /// Draws an effect's busy state without accepting input during external work.
    pub(crate) fn draw<S: Screen>(&mut self, screen: &S) -> std::io::Result<()> {
        let theme = self.theme;
        match &mut self.surface {
            Surface::Standalone(terminal) => terminal
                .draw(|frame| draw_screen(frame, screen, theme, None))
                .map(|_| ()),
            Surface::Embedded { terminal, app } => terminal
                .draw(|frame| draw_screen(frame, screen, theme, Some(app)))
                .map(|_| ()),
            #[cfg(test)]
            Surface::Testing { terminal, app } => terminal
                .draw(|frame| draw_screen(frame, screen, theme, Some(app)))
                .map(|_| ())
                // TestBackend cannot fail; keep its result compatible with I/O backends.
                .map_err(|never| match never {}),
        }
    }

    /// Runs an input-only screen without requiring a dummy completion future at call sites.
    pub(crate) async fn run<S: Screen>(
        &mut self,
        screen: &mut S,
        context: ScreenContext,
    ) -> Result<ScreenResult<S::Outcome, S::Effect>> {
        self.run_screen(screen, None::<Pending<Infallible>>, context)
            .await
    }

    /// Keep drawing while effects finish or unwind; cancellation is handed to the
    /// effect instead of dropping it between enrollment and configuration publication.
    pub(crate) async fn wait_effect<S: Screen, F: Future>(
        &mut self,
        screen: &S,
        completion: F,
        cancellation: &EffectCancellation,
        context: ScreenContext,
    ) -> Result<F::Output> {
        if matches!(self.surface, Surface::Standalone(_)) {
            // Standalone setup leaves input queued while effects run, so keys
            // for the next field or collision review are not discarded as busy input.
            return Ok(completion.await);
        }
        let mut working = Working(screen, cancellation);
        tokio::pin!(completion);
        match self
            .run_screen(&mut working, Some(&mut completion), context)
            .await
        {
            Ok(ScreenResult::Completed(result)) => Ok(result),
            Ok(ScreenResult::Outcome(never) | ScreenResult::Effect(never)) => match never {},
            Ok(ScreenResult::InputEnded) => {
                cancellation.request();
                Ok(completion.await)
            }
            Err(error) => {
                cancellation.request();
                let _ = completion.await;
                Err(error)
            }
        }
    }

    /// Yields outcomes or effects with the terminal entered, optionally racing external work.
    pub(crate) async fn run_screen<S, F>(
        &mut self,
        screen: &mut S,
        completion: Option<F>,
        context: ScreenContext,
    ) -> Result<ScreenResult<S::Outcome, S::Effect, F::Output>>
    where
        S: Screen,
        F: Future,
    {
        self.tick = screen.tick_interval().map(tokio::time::interval);
        let completion = async {
            match completion {
                Some(completion) => completion.await,
                None => pending().await,
            }
        };
        tokio::pin!(completion);
        loop {
            let draw = self.draw(screen);
            match context.draw {
                Some(message) => draw.context(message)?,
                None => draw?,
            }
            let event = tokio::select! {
                result = &mut completion => return Ok(ScreenResult::Completed(result)),
                event = self.events.next() => {
                    let Some(event) = event else {
                        return Ok(ScreenResult::InputEnded);
                    };
                    match event.context(context.input)? {
                        TermEvent::Key(key) => ScreenEvent::Key(key),
                        TermEvent::Paste(text) => ScreenEvent::Paste(text),
                        TermEvent::Resize(width, height) => ScreenEvent::Resize(width, height),
                        _ => continue,
                    }
                }
                _ = async {
                    match &mut self.tick {
                        Some(tick) => { tick.tick().await; }
                        None => pending().await,
                    }
                } => ScreenEvent::Tick,
            };
            match screen.on_event(event) {
                Step::Pending => {}
                Step::Outcome(outcome) => return Ok(ScreenResult::Outcome(outcome)),
                Step::Effect(effect) => return Ok(ScreenResult::Effect(effect)),
            }
        }
    }

    /// Leaves the alternate screen before a flow's existing normal-screen output.
    pub(crate) fn restore(&mut self) -> Result<()> {
        match &mut self.surface {
            Surface::Standalone(terminal) => terminal.restore(),
            Surface::Embedded { .. } => Ok(()),
            #[cfg(test)]
            Surface::Testing { .. } => Ok(()),
        }
    }

    /// Preserve standalone output after restoration; embedded login was reviewed on screen.
    pub(crate) fn login_messages(&self, preview: String, mut messages: Vec<String>) -> Vec<String> {
        if matches!(self.surface, Surface::Standalone(_)) {
            messages.insert(0, preview);
        }
        messages
    }

    /// Preserves explicit restore failures as well as the result of a standalone flow.
    pub(crate) fn finish<T>(mut self, result: Result<T>, context: &str) -> Result<T> {
        self.restore().with_context(|| context.to_owned())?;
        result
    }
}

struct Working<'a, S>(&'a S, &'a EffectCancellation);

impl<S: Screen> Screen for Working<'_, S> {
    type Outcome = Infallible;
    type Effect = Infallible;
    fn draw(&self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, theme: Theme) {
        self.0.draw(frame, area, theme);
    }
    fn draw_embedded(
        &self,
        frame: &mut ratatui::Frame<'_>,
        area: ratatui::layout::Rect,
        theme: Theme,
    ) {
        self.0.draw_embedded(frame, area, theme);
    }
    fn content_height(&self, width: u16) -> u16 {
        self.0.content_height(width)
    }
    fn footer(&self) -> Option<smith_tui::picker::ScreenFooter> {
        Some(smith_tui::picker::ScreenFooter::Busy { back: false })
    }
    fn on_event(&mut self, event: ScreenEvent) -> Step<Self::Outcome, Self::Effect> {
        if let ScreenEvent::Key(key) = event
            && key.kind != KeyEventKind::Release
            && (key.code == KeyCode::Esc
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL)))
        {
            self.1.request();
        }
        Step::Pending
    }
    fn tick_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(250))
    }
}

fn draw_screen<S: Screen>(
    frame: &mut ratatui::Frame<'_>,
    screen: &S,
    theme: Theme,
    app: Option<&App>,
) {
    match app {
        Some(app) => {
            smith_tui::render::draw_with_screen(frame, app, screen, theme);
        }
        None => screen.draw(frame, frame.area(), theme),
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use smith_tui::setup::{SetupApp, SetupMode};
    use smith_tui::{App, FlowOutcome, Theme};

    use super::{ScreenContext, ScreenResult, ScreenSession, Surface};

    fn input(
        codes: Vec<KeyEvent>,
    ) -> Box<dyn futures_util::Stream<Item = std::io::Result<Event>> + Unpin + Send> {
        Box::new(futures_util::stream::iter(
            codes.into_iter().map(|key| Ok(Event::Key(key))),
        ))
    }

    fn screen_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn embedded_runner_keeps_the_backdrop_through_credentials_and_cancel() {
        for (width, height) in [(44, 16), (100, 32)] {
            let mut app = App::new("gpt-5.3", "~/work/api");
            app.transcript.push_user("retained transcript");
            app.composer.insert_str("retained draft");
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                    .expect("terminal");
            let mut setup = SetupApp::new(
                SetupMode::Credential {
                    provider: "openrouter".into(),
                },
                Vec::new(),
                Vec::new(),
                crate::setup::glm_quick_start(),
                Vec::new(),
                crate::setup::setup_prompts(),
            )
            .with_title("Connect OpenRouter");
            let context = ScreenContext {
                draw: None,
                input: "test input",
            };
            {
                let mut session = ScreenSession {
                    surface: Surface::Testing {
                        terminal: &mut terminal,
                        app: &app,
                    },
                    events: input(vec![KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE)]),
                    tick: None,
                    theme: Theme::new().without_color().without_motion(),
                };
                assert!(matches!(
                    session
                        .run(&mut setup, context)
                        .await
                        .expect("credential field"),
                    ScreenResult::InputEnded
                ));
                if let Surface::Testing { terminal, .. } = &session.surface {
                    let text = screen_text(terminal);
                    for expected in [
                        "retained transcript",
                        "retained draft",
                        "Connect OpenRouter",
                        "enter continue",
                    ] {
                        assert!(text.contains(expected), "{text}");
                    }
                    assert!(!text.contains("Smith setup"), "{text}");
                }
                session.events = input(vec![
                    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                ]);
                assert!(matches!(
                    session.run(&mut setup, context).await.expect("cancel"),
                    ScreenResult::Outcome(())
                ));
                session.restore().expect("borrowed restore is inert");
            }
            assert_eq!(app.composer.text(), "retained draft");
            assert_eq!(app.transcript.blocks().len(), 1);
        }
    }

    #[tokio::test]
    async fn embedded_xai_escape_drops_the_poll_future() {
        let app = App::new("gpt-5.3", "~/work/api");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(44, 16)).expect("terminal");
        let mut session = ScreenSession {
            surface: Surface::Testing {
                terminal: &mut terminal,
                app: &app,
            },
            events: input(vec![KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)]),
            tick: None,
            theme: Theme::new().without_color().without_motion(),
        };
        let mut progress =
            crate::xai::login_progress("ABCD-1234", "https://auth.x.ai/activate", true, true);
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct Guard(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let guard = Guard(dropped.clone());
        let completion = async move {
            let _guard = guard;
            std::future::pending::<()>().await
        };
        assert!(matches!(
            session
                .run_screen(
                    &mut progress,
                    Some(completion),
                    ScreenContext {
                        draw: None,
                        input: "test input"
                    }
                )
                .await
                .expect("progress"),
            ScreenResult::Outcome(FlowOutcome::Cancelled)
        ));
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
        if let Surface::Testing { terminal, .. } = &session.surface {
            let text = screen_text(terminal);
            for expected in [
                "Connect xAI",
                "ABCD-1234",
                "https://auth.x.ai",
                "Waiting for xAI",
                "esc cancel",
            ] {
                assert!(text.contains(expected), "{text}");
            }
        }
    }
}
