//! Terminal-independent screens, so standalone and embedded hosts can share reducers.

use std::time::Duration;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::theme::Theme;

/// Input a host can deliver without giving a screen ownership of the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenEvent {
    /// Preserve the complete key, including modifiers and press/release kind.
    Key(KeyEvent),
    /// Deliver bracketed paste together so it cannot accidentally submit a field.
    Paste(String),
    /// Let a screen respond to a changed viewport before the next draw.
    Resize(u16, u16),
    /// Advance presentation state at the screen's requested cadence.
    Tick,
}

/// Keeps backing out distinct from cancellation so nested flows can resume their owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowOutcome<T> {
    /// All required steps finished and the owner can apply the result.
    Completed(T),
    /// Return to the previous screen with its state intact.
    Back,
    /// Abandon the entire flow without applying a result.
    Cancelled,
}

/// Separates completion from work the host must perform before running again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step<Outcome, Effect> {
    /// Keep accepting input and drawing the current state.
    Pending,
    /// Finish this screen, including user cancellation when its outcome allows it.
    Outcome(Outcome),
    /// Yield to the host while retaining the screen and entered terminal.
    Effect(Effect),
}

/// A pure screen value keeps rendering and input independent of terminal effects.
pub trait Screen {
    /// The completed value returned to the flow that owns this screen.
    type Outcome;
    /// Work returned to the host rather than performed by the reducer.
    type Effect;

    /// Draw within the host's viewport so the same state needs no terminal handle.
    fn draw(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme);

    /// Reduce one input while leaving external work to the host.
    fn on_event(&mut self, event: ScreenEvent) -> Step<Self::Outcome, Self::Effect>;

    /// Request animation ticks only for screens with time-dependent presentation.
    fn tick_interval(&self) -> Option<Duration> {
        None
    }
}
