//! One terminal owner and event loop for screens shown outside a session.

use std::convert::Infallible;
use std::future::{Future, Pending, pending};

use anyhow::{Context, Result};
use crossterm::event::{Event as TermEvent, EventStream};
use futures_util::StreamExt;
use smith_tui::{Screen, ScreenEvent, Step, Theme};
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

/// Keeps a single event reader and terminal guard alive across a chain of screens.
pub(crate) struct ScreenSession {
    terminal: Terminal,
    events: EventStream,
    tick: Option<Interval>,
    theme: Theme,
}

impl ScreenSession {
    /// Enters once; the terminal guard also restores on errors, cancellation, or panic.
    pub(crate) fn enter(no_color: bool, no_motion: bool) -> Result<Self> {
        let terminal = terminal::enter()?;
        Ok(Self {
            terminal,
            events: EventStream::new(),
            tick: None,
            theme: theme_from_flags(no_color, no_motion),
        })
    }

    /// Draws an effect's busy state without accepting input during external work.
    pub(crate) fn draw<S: Screen>(&mut self, screen: &S) -> std::io::Result<()> {
        let theme = self.theme;
        self.terminal
            .draw(|frame| {
                let area = frame.area();
                screen.draw(frame, area, theme);
            })
            .map(|_| ())
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
        self.terminal.restore()
    }

    /// Preserves explicit restore failures as well as the result of a standalone flow.
    pub(crate) fn finish<T>(mut self, result: Result<T>, context: &str) -> Result<T> {
        self.restore().with_context(|| context.to_owned())?;
        result
    }
}
