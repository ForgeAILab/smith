//! Keyboard, mouse, composer history, exit, and scrolling transitions.

use std::time::Instant;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use smith_host::approval::PromptScope;

use crate::commands::{self, CommandAction};
use crate::questionnaire::QuestionnaireResolution;
use crate::references::{ComposerReference, parse_references};
use crate::selection::Selection;
use crate::status::Activity;

use super::state::*;

/// What the host loop must do after a mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseOutcome {
    /// Nothing changed on screen.
    Ignored,
    /// Visible state changed; redraw.
    Redraw,
    /// A drag finished. The host reads the highlighted text out of the frame
    /// buffer and puts it on the clipboard — `App` performs no I/O and cannot
    /// see the rendered frame, so it can only ask.
    CopySelection,
}

impl App {
    /// Handles a mouse event, reporting what the host must do next.
    ///
    /// Smith owns pointer selection here because enabling wheel reporting takes
    /// native terminal selection away; see [`crate::selection`]. Only the left
    /// button and the wheel are consumed.
    pub fn on_mouse(&mut self, mouse: MouseEvent) -> MouseOutcome {
        // Selection is screen-space, so it stays available under the palette
        // (which draws inline) but not under a modal that owns the surface.
        let selectable = matches!(self.overlay, None | Some(Overlay::Palette { .. }));
        match mouse.kind {
            MouseEventKind::ScrollUp if selectable => {
                // Scrolling slides the text under a highlight that addresses
                // fixed cells, so the selection cannot survive it.
                self.selection = None;
                self.scroll_up(MOUSE_SCROLL_LINES);
                MouseOutcome::Redraw
            }
            MouseEventKind::ScrollDown if selectable => {
                self.selection = None;
                self.scroll_down(MOUSE_SCROLL_LINES);
                MouseOutcome::Redraw
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // A press always clears the previous highlight, so a bare click
                // is how the user dismisses one.
                self.selection = Some(Selection::begin(mouse.column, mouse.row));
                MouseOutcome::Redraw
            }
            MouseEventKind::Drag(MouseButton::Left) => match &mut self.selection {
                Some(selection) if selection.dragging() => {
                    selection.drag_to(mouse.column, mouse.row);
                    MouseOutcome::Redraw
                }
                // A drag whose press we never saw (the button went down before
                // Smith owned the mouse) has no anchor to grow from.
                _ => MouseOutcome::Ignored,
            },
            MouseEventKind::Up(MouseButton::Left) => match &mut self.selection {
                Some(selection) if selection.dragging() => {
                    // The release position is authoritative, not just the last
                    // drag report: a quick flick can land `Up` well past the
                    // final `Drag` the terminal bothered to send, and reading
                    // only the drags would copy short — or, with none sent at
                    // all, copy nothing.
                    selection.drag_to(mouse.column, mouse.row);
                    selection.finish();
                    if selection.is_empty() {
                        // A click that never moved: dismiss, do not copy.
                        self.selection = None;
                        MouseOutcome::Redraw
                    } else {
                        MouseOutcome::CopySelection
                    }
                }
                _ => MouseOutcome::Ignored,
            },
            _ => MouseOutcome::Ignored,
        }
    }

    /// Handles a key press, returning an action for the host loop.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        let action = self.reduce_key(key);
        if !matches!(action, Some(Action::Quit | Action::Reconfigure(_))) {
            self.present_next_prompt();
        }
        action
    }

    pub(super) fn reduce_key(&mut self, key: KeyEvent) -> Option<Action> {
        // Terminals that report both press and release would otherwise act on
        // every keystroke twice.
        if key.kind == KeyEventKind::Release {
            return None;
        }

        // Ctrl+C is checked before overlays: two consecutive presses must
        // always be able to leave, even while a prompt owns input.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.on_ctrl_c();
        }
        self.last_ctrl_c = None;

        match &self.overlay {
            Some(Overlay::Approval { .. }) => return self.on_approval_key(key),
            Some(Overlay::Questionnaire { .. }) => return self.on_questionnaire_key(key),
            Some(Overlay::Palette { .. }) => return self.on_palette_key(key),
            Some(Overlay::ResourcePicker { .. }) => {
                return self.on_resource_picker_key(key);
            }
            Some(Overlay::HistorySearch { .. }) => return self.on_history_search_key(key),
            Some(Overlay::RotationConfirm { .. }) => {
                return match key.code {
                    // The first eligible member is what `y` takes, which is
                    // pool order: the account the user listed next.
                    KeyCode::Char('y') => self.answer_rotation(Some(0)),
                    KeyCode::Char('n') | KeyCode::Esc => self.answer_rotation(None),
                    // A pool wider than two accounts is chosen by the number
                    // the modal printed beside each one.
                    KeyCode::Char(digit @ '1'..='9') => {
                        let listed = digit.to_digit(10).unwrap_or(0) as usize;
                        self.select_offered_account(listed)
                    }
                    _ => None,
                };
            }
            Some(Overlay::UndoConfirm { .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        self.overlay = None;
                        Some(Action::ApplyUndo)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        Some(Action::CancelUndo)
                    }
                    _ => None,
                };
            }
            Some(Overlay::RedoConfirm { .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        self.overlay = None;
                        Some(Action::ApplyRedo)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        Some(Action::CancelRedo)
                    }
                    _ => None,
                };
            }
            Some(Overlay::RevertConfirm {
                scope, fingerprint, ..
            }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        let action = Action::ApplyRevert {
                            scope: scope.clone(),
                            fingerprint: fingerprint.clone(),
                        };
                        self.overlay = None;
                        Some(action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        let action = Action::CancelRevert {
                            scope: scope.clone(),
                            fingerprint: fingerprint.clone(),
                        };
                        self.overlay = None;
                        Some(action)
                    }
                    _ => None,
                };
            }
            Some(Overlay::ReviewConfirm { scope, .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        let action = Action::StartReview {
                            scope: scope.clone(),
                        };
                        self.overlay = None;
                        Some(action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        None
                    }
                    _ => None,
                };
            }
            Some(Overlay::AgentConfirm { preset, task, .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        let action = Action::StartAgent {
                            preset: preset.clone(),
                            task: task.clone(),
                        };
                        self.overlay = None;
                        self.composer.clear();
                        Some(action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        None
                    }
                    _ => None,
                };
            }
            Some(Overlay::AgentFollowUpConfirm { child_id, task, .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        let action = Action::FollowUpAgent {
                            child_id: child_id.clone(),
                            task: task.clone(),
                        };
                        self.overlay = None;
                        self.composer.clear();
                        Some(action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        None
                    }
                    _ => None,
                };
            }
            Some(Overlay::AgentResumeConfirm { child_id, .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        let action = Action::ResumeAgent {
                            child_id: child_id.clone(),
                        };
                        self.overlay = None;
                        self.composer.clear();
                        Some(action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        None
                    }
                    _ => None,
                };
            }
            Some(Overlay::ExitConfirm { .. }) => return self.on_exit_confirm_key(key),
            None => {}
        }

        match (key.code, key.modifiers) {
            (KeyCode::Tab, _) if self.is_busy() && !self.composer.is_blank() => {
                self.queue_current_ordinary_submission();
                None
            }
            // At the empty idle point of action, Tab cycles only the
            // configured main-agent profiles. Overlay-specific Tab behavior was
            // handled above and a non-empty draft is never changed.
            (KeyCode::Tab, _) if !self.is_busy() && self.composer.is_empty() => {
                self.cycle_agent_profile(false)
            }
            (KeyCode::BackTab, _) if !self.is_busy() && self.composer.is_empty() => {
                self.cycle_agent_profile(true)
            }
            (KeyCode::Tab | KeyCode::BackTab, _) => None,
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                let original = self.composer.text().to_owned();
                if !self.composer.text().starts_with('/') {
                    self.composer.replace("/");
                }
                self.overlay = Some(Overlay::Palette {
                    selected: 0,
                    error: None,
                    restore_on_escape: Some(original),
                });
                None
            }
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                self.follow_newest();
                None
            }
            (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                self.open_history_search();
                None
            }
            // The host decides whether a foreground shell call exists to
            // adopt; the app has no runtime visibility to gate this itself,
            // and always emitting the action keeps the mapping trivial and
            // testable without a live host.
            (KeyCode::Char('b'), KeyModifiers::CONTROL) => Some(Action::BackgroundShell),
            (KeyCode::Char('?'), KeyModifiers::NONE) if self.composer.is_empty() => {
                self.follow_newest();
                self.dispatch_command(CommandAction::Help)
            }
            (KeyCode::Esc, _) => self.on_escape(),
            (KeyCode::PageUp, _) => {
                self.scroll_up(10);
                None
            }
            (KeyCode::PageDown, _) => {
                self.scroll_down(10);
                None
            }
            (KeyCode::Up, modifiers) if modifiers.contains(KeyModifiers::ALT) => {
                self.edit_newest_queued_submission();
                None
            }
            // The arrows serve two lists that never overlap: composer history
            // above the prompt, delegated agents below it. History wins while
            // it has somewhere to go, so recall behavior is unchanged for a
            // session with no children.
            (KeyCode::Up, _) => {
                if !self.inspect_previous_child() {
                    self.composer.recall_previous();
                }
                None
            }
            (KeyCode::Down, _) => {
                if !self.composer.recall_next() {
                    self.inspect_next_child();
                }
                None
            }
            (KeyCode::Home | KeyCode::End, _) => self.on_scroll_key(key),
            _ => self.on_composer_key(key),
        }
    }

    /// Answers a rotation offer.
    ///
    /// `offered` is an index into the offer's eligible list, or `None` to
    /// stay. Either way the prompt is consumed here: leaving it in the overlay
    /// would keep the runtime blocked on an answer already given.
    pub(super) fn answer_rotation(&mut self, offered: Option<usize>) -> Option<Action> {
        let Some(Overlay::RotationConfirm { prompt, .. }) = self.overlay.take() else {
            return None;
        };
        let request = prompt.request().clone();
        let outgoing = request.outgoing.label.clone();

        match offered.and_then(|index| request.eligible.get(index)) {
            Some(member) => {
                let notice = crate::accounts::switch_notice(&outgoing, &member.label, false);
                prompt.switch_to(member.position);
                self.transcript.push_notice("account", &notice);
            }
            None => {
                let notice = crate::accounts::declined_notice(
                    &outgoing,
                    request.outgoing_resets_at_ms,
                    crate::accounts::now_ms(),
                );
                // Dropping would decline too, but saying so explicitly keeps
                // the refusal a decision rather than an accident.
                prompt.decline();
                self.transcript.push_notice("account", &notice);
            }
        }
        None
    }

    /// Selects the account the modal printed as `listed` (1-based).
    ///
    /// An out-of-range number is ignored rather than treated as a refusal: a
    /// mistyped digit must not spend the turn.
    pub(super) fn select_offered_account(&mut self, listed: usize) -> Option<Action> {
        let Some(Overlay::RotationConfirm { prompt, .. }) = &self.overlay else {
            return None;
        };
        let position = listed.checked_sub(1)?;
        let index = prompt
            .request()
            .eligible
            .iter()
            .position(|member| member.position == position)?;
        self.answer_rotation(Some(index))
    }

    pub(super) fn on_ctrl_c(&mut self) -> Option<Action> {
        let now = Instant::now();
        let second_press = self
            .last_ctrl_c
            .is_some_and(|previous| now.duration_since(previous) < FORCE_QUIT_WINDOW);

        if second_press {
            self.cancel_pending_prompts();
            self.should_quit = true;
            return Some(Action::Quit);
        }

        self.last_ctrl_c = Some(now);
        match self.overlay.take() {
            Some(Overlay::Palette {
                restore_on_escape, ..
            }) => {
                if let Some(original) = restore_on_escape {
                    self.composer.replace(original);
                }
            }
            Some(Overlay::ResourcePicker {
                restore_on_escape, ..
            }) => self.composer.replace(restore_on_escape),
            Some(Overlay::HistorySearch { original, .. }) => self.composer.replace(original),
            other => self.overlay = other,
        }
        self.composer.stash_for_recall();
        None
    }

    pub(super) fn open_history_search(&mut self) {
        self.overlay = Some(Overlay::HistorySearch {
            original: self.composer.text().to_owned(),
            query: String::new(),
            selected: None,
            matched: None,
        });
    }

    pub(super) fn on_history_search_key(&mut self, key: KeyEvent) -> Option<Action> {
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                if let Some(Overlay::HistorySearch { original, .. }) = self.overlay.take() {
                    self.composer.replace(original);
                }
            }
            (KeyCode::Enter, _) => {
                let matched = match &self.overlay {
                    Some(Overlay::HistorySearch { matched, .. }) => matched.clone(),
                    _ => None,
                };
                if let Some(matched) = matched {
                    self.overlay = None;
                    self.composer.replace(matched);
                }
            }
            (KeyCode::Backspace, _) => {
                if let Some(Overlay::HistorySearch { query, .. }) = &mut self.overlay {
                    query.pop();
                }
                self.refresh_history_search(false);
            }
            (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                self.refresh_history_search(true);
            }
            (KeyCode::Char(character), modifiers)
                if !modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                if let Some(Overlay::HistorySearch { query, .. }) = &mut self.overlay {
                    query.push(character);
                }
                self.refresh_history_search(false);
            }
            _ => {}
        }
        None
    }

    pub(super) fn refresh_history_search(&mut self, cycle: bool) {
        let (query, after) = match &self.overlay {
            Some(Overlay::HistorySearch {
                query, selected, ..
            }) => (query.clone(), cycle.then_some(*selected).flatten()),
            _ => return,
        };
        let found = self.composer.search_history(&query, after);
        if let Some(Overlay::HistorySearch {
            selected, matched, ..
        }) = &mut self.overlay
        {
            (*selected, *matched) =
                found.map_or((None, None), |(index, entry)| (Some(index), Some(entry)));
        }
    }

    pub(super) fn request_exit(&mut self) -> Option<Action> {
        if !self.has_live_work() {
            self.cancel_pending_prompts();
            self.should_quit = true;
            return Some(Action::Quit);
        }

        let (approval, questionnaire) = match self.overlay.take() {
            Some(Overlay::Approval { prompt, review }) => (Some((prompt, review)), None),
            Some(Overlay::Questionnaire { state }) => (None, Some(state)),
            _ => (None, None),
        };
        self.overlay = Some(Overlay::ExitConfirm {
            approval,
            questionnaire,
        });
        None
    }

    pub(super) fn cancel_pending_prompts(&mut self) {
        self.overlay = match self.overlay.take() {
            Some(Overlay::Approval { prompt, .. }) => {
                prompt.cancel();
                None
            }
            Some(Overlay::Questionnaire { state }) => {
                self.resolve_questionnaire(state, QuestionnaireResolution::Cancelled);
                None
            }
            Some(Overlay::ExitConfirm {
                approval,
                questionnaire,
            }) => {
                if let Some((prompt, _)) = approval {
                    prompt.cancel();
                }
                if let Some(state) = questionnaire {
                    self.resolve_questionnaire(state, QuestionnaireResolution::Cancelled);
                }
                None
            }
            other => other,
        };
        while let Some(prompt) = self.pending_prompts.pop_front() {
            match prompt {
                PendingPrompt::Approval(prompt, _) => prompt.cancel(),
                PendingPrompt::Questionnaire(state) => {
                    self.resolve_questionnaire(state, QuestionnaireResolution::Cancelled);
                }
            }
        }
    }

    pub(super) fn on_escape(&mut self) -> Option<Action> {
        // Leaving a read-only view is the cheapest thing Esc can mean, so it
        // goes first: a user reading a child's log must not interrupt the root
        // turn by pressing Esc to get back.
        if self.leave_child_inspection() {
            return None;
        }
        if self.is_busy() {
            self.pending_input.interrupt_for_steer = !self.pending_input.accepted_steers.is_empty();
            self.status.activity = Activity::Interrupting;
            return Some(Action::Interrupt);
        }
        if !self.composer.is_empty() {
            self.composer.clear();
        }
        None
    }

    pub(super) fn on_approval_key(&mut self, key: KeyEvent) -> Option<Action> {
        // No default action: an approval modal must not be answerable by a
        // stray Enter arriving from the composer.
        match key.code {
            KeyCode::Char('y') => self.answer_approval(Some(PromptScope::Once)),
            KeyCode::Char('a') => self.answer_approval(Some(PromptScope::Session)),
            KeyCode::Char('n') | KeyCode::Esc => self.answer_approval(None),
            _ => {}
        }
        None
    }

    pub(super) fn on_questionnaire_key(&mut self, key: KeyEvent) -> Option<Action> {
        let resolution = match &mut self.overlay {
            Some(Overlay::Questionnaire { state }) => state.on_key(key),
            _ => None,
        };
        if let Some(resolution) = resolution
            && let Some(Overlay::Questionnaire { state }) = self.overlay.take()
        {
            self.resolve_questionnaire(state, resolution);
            self.present_next_prompt();
        }
        None
    }

    pub(super) fn on_exit_confirm_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('y') => {
                self.cancel_pending_prompts();
                self.should_quit = true;
                Some(Action::Quit)
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                let (approval, questionnaire) = match self.overlay.take() {
                    Some(Overlay::ExitConfirm {
                        approval,
                        questionnaire,
                    }) => (approval, questionnaire),
                    _ => (None, None),
                };
                self.overlay = match (approval, questionnaire) {
                    (Some((prompt, review)), None) => Some(Overlay::Approval { prompt, review }),
                    (None, Some(state)) => Some(Overlay::Questionnaire { state }),
                    _ => None,
                };
                self.present_next_prompt();
                None
            }
            _ => None,
        }
    }

    pub(super) fn on_palette_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => {
                if let Some(Overlay::Palette {
                    restore_on_escape, ..
                }) = self.overlay.take()
                    && let Some(original) = restore_on_escape
                {
                    self.composer.replace(original);
                }
                None
            }
            KeyCode::Backspace => {
                self.composer_backspace_over_attachment();
                if self.composer.is_empty() {
                    self.overlay = None;
                    return None;
                }
                if let Some(Overlay::Palette {
                    selected, error, ..
                }) = &mut self.overlay
                {
                    *selected = 0;
                    *error = None;
                }
                None
            }
            KeyCode::Tab | KeyCode::Down => {
                let count = commands::matches(self.composer.text()).len();
                if count > 0
                    && let Some(Overlay::Palette { selected, .. }) = &mut self.overlay
                {
                    // Tab completes the highlighted entry — the same one Enter
                    // acts on — while Down only moves the highlight.
                    if key.code == KeyCode::Tab {
                        let command = commands::matches(self.composer.text())[*selected % count];
                        self.composer.replace(commands::completion(command));
                        self.overlay = None;
                    } else {
                        *selected = (*selected + 1) % count;
                    }
                }
                None
            }
            KeyCode::BackTab | KeyCode::Up => {
                let matches = commands::matches(self.composer.text());
                if !matches.is_empty()
                    && let Some(Overlay::Palette { selected, .. }) = &mut self.overlay
                {
                    *selected = selected.checked_sub(1).unwrap_or(matches.len() - 1);
                }
                None
            }
            KeyCode::Enter => {
                if self.composer.text().starts_with("//") {
                    self.overlay = None;
                    return self.on_composer_key(key);
                }
                let matches = commands::matches(self.composer.text());
                let selected = match &self.overlay {
                    Some(Overlay::Palette { selected, .. }) => *selected,
                    _ => return None,
                };
                let input = self.composer.text().to_owned();
                match commands::parse(&input) {
                    Ok(command) => self.dispatch_command(command),
                    Err(message) => {
                        let needs_value = matches.get(selected).is_some_and(|command| {
                            matches!(
                                command.name,
                                "resume" | "profile" | "provider" | "model" | "think" | "effort"
                            )
                        });
                        if needs_value && self.composer.text().split_whitespace().count() == 1 {
                            let command = matches[selected];
                            self.composer.replace(commands::completion(command));
                            self.overlay = None;
                            return None;
                        }
                        if let Some(Overlay::Palette { error, .. }) = &mut self.overlay {
                            *error = Some(message);
                        }
                        None
                    }
                }
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                self.composer.insert(character);
                if let Some(Overlay::Palette {
                    selected, error, ..
                }) = &mut self.overlay
                {
                    *selected = 0;
                    *error = None;
                }
                None
            }
            _ => None,
        }
    }

    pub(super) fn on_composer_key(&mut self, key: KeyEvent) -> Option<Action> {
        match (key.code, key.modifiers) {
            (KeyCode::Enter, m)
                if m.contains(KeyModifiers::SHIFT) || m.contains(KeyModifiers::ALT) =>
            {
                self.composer.insert('\n');
                None
            }
            (KeyCode::Enter, _) => {
                if self.composer.is_blank() {
                    return None;
                }
                let text = self.composer.text().trim().to_owned();
                // While a child is inspected, an ordinary submission addresses
                // that child: the user is reading its log, and sending the
                // root a message from there would answer the wrong agent. A
                // local command, a shell shortcut, and an explicit `@` target
                // still mean exactly what they say.
                let text = match &self.inspected_child {
                    Some(child)
                        if !text.starts_with('/')
                            && !text.starts_with('!')
                            && !text.starts_with('@') =>
                    {
                        format!("@{child} {text}")
                    }
                    _ => text,
                };
                match self.prepare_ordinary_submission(&text) {
                    Ok(Some(submission)) => {
                        let target = if self.is_busy() {
                            SubmissionTarget::Steer {
                                expected_turn: self.active_turn.clone(),
                            }
                        } else {
                            SubmissionTarget::WholeTurn
                        };
                        self.composer.record_current();
                        self.composer.clear();
                        self.follow_newest();
                        return Some(Action::Submit { submission, target });
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.transcript.push_error(error);
                        return None;
                    }
                }
                // One leading marker is an explicit local prepared shell
                // action. Keep the draft on validation failure so no command
                // disappears before it has been accepted.
                if let Some(command) = text.strip_prefix('!') {
                    let command = command.trim();
                    if command.is_empty() {
                        self.transcript
                            .push_error("a shell shortcut requires a command after `!`");
                        return None;
                    }
                    let command = command.to_owned();
                    self.composer.record_current();
                    self.composer.clear();
                    self.transcript.push_notice("shell", format!("$ {command}"));
                    self.follow_newest();
                    return Some(Action::RunShell {
                        command: self.expand_pasted(&command),
                    });
                }
                // `/…` is a local command and never reaches the provider.
                if text.starts_with('/') {
                    self.follow_newest();
                    return match commands::parse(&text) {
                        Ok(command) => self.dispatch_command(command),
                        Err(error) => {
                            self.transcript.push_error(error);
                            None
                        }
                    };
                }
                let files = self
                    .resources
                    .files
                    .iter()
                    .filter_map(|entry| entry.id.strip_prefix("file:"))
                    .map(str::to_owned)
                    .collect();
                let agents = self
                    .resources
                    .child_agents
                    .iter()
                    .filter(|entry| entry.disabled_reason.is_none())
                    .filter_map(|entry| entry.id.strip_prefix("agent:"))
                    .map(str::to_owned)
                    .chain(self.children.keys().cloned())
                    .collect();
                let parsed = match parse_references(&text, &files, &agents) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        self.transcript.push_error(error);
                        return None;
                    }
                };
                let attached_files = parsed
                    .references
                    .iter()
                    .filter_map(|reference| match reference {
                        ComposerReference::File(path) => Some(path.clone()),
                        ComposerReference::Agent(_) => None,
                    })
                    .collect::<Vec<_>>();
                let referenced_agents = parsed
                    .references
                    .iter()
                    .filter_map(|reference| match reference {
                        ComposerReference::Agent(agent) => Some(agent.clone()),
                        ComposerReference::File(_) => None,
                    })
                    .collect::<Vec<_>>();
                if let Some(agent) = referenced_agents.first() {
                    let trimmed = parsed.text.trim_start();
                    let plain = format!("@{agent}");
                    let typed = format!("@agent:{agent}");
                    let task = trimmed
                        .strip_prefix(&typed)
                        .or_else(|| trimmed.strip_prefix(&plain));
                    let Some(task) = task else {
                        self.transcript.push_error(
                            "a child-enabled profile or existing child must be the first token, for example `@review inspect the diff` or `@child-1 check that edge case`",
                        );
                        return None;
                    };
                    if referenced_agents.len() != 1 || !attached_files.is_empty() {
                        self.transcript.push_error(
                            "one explicit child profile must be submitted without file attachments",
                        );
                        return None;
                    }
                    let task = task.trim();
                    if task.is_empty() {
                        self.transcript.push_error(format!(
                            "`@{agent}` requires a bounded task after the child identity"
                        ));
                        return None;
                    }
                    if let Some(existing) = self.children.get(agent).cloned() {
                        if !matches!(
                            existing.state.as_str(),
                            "idle" | "completed" | "needs input"
                        ) {
                            // The error goes to the root transcript, which the
                            // inspector is covering, so it also has to be
                            // visible where the user typed: the panel row and
                            // the child's own view carry the same state.
                            self.transcript.push_error(format!(
                                "`{agent}` is {}; it takes a follow-up once it settles{}",
                                existing.state,
                                if existing.state == "interrupted" {
                                    ", and `/agent resume <id>` continues its exact checkpoint"
                                } else {
                                    ""
                                }
                            ));
                            self.push_child_log(
                                agent,
                                format!("follow-up refused while {}", existing.state),
                            );
                            return None;
                        }
                        let model = match &self.status.provider {
                            Some(provider) => format!("{provider}/{}", self.status.model),
                            None => self.status.model.clone(),
                        };
                        self.composer.record_current();
                        self.overlay = Some(Overlay::AgentFollowUpConfirm {
                            child_id: agent.clone(),
                            task: self.expand_pasted(task),
                            content: format!(
                                "child: {agent}\noperation: new follow-up turn\ncontinuity: reuse prior child history and cumulative limits\nprovider/model: {model}\nprovider spend: yes\ncheckpoint replay: no"
                            ),
                        });
                        return None;
                    }
                    let profile_detail = self
                        .resources
                        .child_agents
                        .iter()
                        .find(|entry| entry.id.strip_prefix("agent:") == Some(agent.as_str()))
                        .map_or("read-only child profile", |entry| entry.detail.as_str());
                    self.composer.record_current();
                    self.overlay = Some(Overlay::AgentConfirm {
                        preset: agent.clone(),
                        task: self.expand_pasted(task),
                        content: format!(
                            "profile: {agent}\nconfiguration: {profile_detail}\nworkspace: read-only\nturn limit: 1\nprovider spend: yes\nresult: bounded child summary"
                        ),
                    });
                    return None;
                }
                unreachable!("ordinary input without a child reference was prepared above")
            }
            (KeyCode::Backspace, _) => {
                self.composer_backspace_over_attachment();
                None
            }
            (KeyCode::Delete, _) => {
                self.composer_delete_over_attachment();
                None
            }
            (KeyCode::Left, _) => {
                self.composer_move_left_over_attachment();
                None
            }
            (KeyCode::Right, _) => {
                self.composer_move_right_over_attachment();
                None
            }
            (KeyCode::Home, _) => {
                self.composer.move_home();
                None
            }
            (KeyCode::End, _) => {
                self.composer.move_end();
                None
            }
            (KeyCode::Char(ch), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                if ch == '@' && self.composer_at_token_boundary() {
                    let restore = self.composer.text().to_owned();
                    self.open_target_picker(ResourceTarget::Reference, restore);
                    return None;
                }
                self.composer.insert(ch);
                if self.composer.text().starts_with('/') {
                    self.overlay = Some(Overlay::Palette {
                        selected: 0,
                        error: None,
                        restore_on_escape: None,
                    });
                }
                None
            }
            _ => None,
        }
    }

    pub(super) fn composer_at_token_boundary(&self) -> bool {
        let cursor = self.composer.cursor();
        cursor == 0
            || self
                .composer
                .text()
                .chars()
                .nth(cursor.saturating_sub(1))
                .is_some_and(|character| {
                    character.is_whitespace()
                        || matches!(character, '(' | '[' | '{' | ',' | ';' | ':')
                })
    }

    pub(super) fn accept_composer_input(&mut self) {
        self.composer.record_current();
        self.composer.clear();
    }

    pub(super) fn on_scroll_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Home => self.scroll_up(u16::MAX),
            KeyCode::End => self.follow_newest(),
            _ => {}
        }
        None
    }

    /// Scrolls up, which pauses following.
    pub fn scroll_up(&mut self, lines: u16) {
        if self.scroll_limit == 0 || lines == 0 {
            return;
        }
        self.scroll_back = self
            .scroll_back
            .saturating_add(lines)
            .min(self.scroll_limit);
        self.following = self.scroll_back == 0;
    }

    /// Scrolls down, resuming following at the bottom.
    pub fn scroll_down(&mut self, lines: u16) {
        self.scroll_back = self
            .scroll_back
            .min(self.scroll_limit)
            .saturating_sub(lines);
        if self.scroll_back == 0 {
            self.following = true;
        }
    }

    /// Jumps to newest output and resumes following.
    pub fn follow_newest(&mut self) {
        self.scroll_back = 0;
        self.following = true;
    }

    /// Synchronizes scroll state with the viewport computed by the renderer.
    ///
    /// Keeping the visible offset stable while paused prevents streaming output
    /// or a resize from pulling the reader toward the newest content.
    pub(crate) fn sync_scroll_limit(&mut self, limit: u16) {
        if self.following {
            self.scroll_limit = limit;
            self.scroll_back = 0;
            return;
        }

        let visible_offset = self
            .scroll_limit
            .saturating_sub(self.scroll_back.min(self.scroll_limit));
        self.scroll_limit = limit;
        if limit == 0 {
            self.follow_newest();
            return;
        }

        self.scroll_back = limit.saturating_sub(visible_offset.min(limit));
        if self.scroll_back == 0 {
            self.following = true;
        }
    }
}
