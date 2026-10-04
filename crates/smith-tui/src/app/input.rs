//! Keyboard, mouse, composer history, exit, and scrolling transitions.

use std::time::Instant;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use smith_client::NoticeKind;
use smith_host::approval::PromptScope;

use crate::commands;
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
        let scrollable = selectable || matches!(self.overlay, Some(Overlay::Approval { .. }));
        match mouse.kind {
            MouseEventKind::ScrollUp if scrollable => {
                // Scrolling slides the text under a highlight that addresses
                // fixed cells, so the selection cannot survive it.
                self.selection = None;
                self.scroll_up(MOUSE_SCROLL_LINES);
                MouseOutcome::Redraw
            }
            MouseEventKind::ScrollDown if scrollable => {
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
        self.clear_feedback();

        let closing_shortcuts = matches!(self.overlay, Some(Overlay::Shortcuts));
        if closing_shortcuts {
            self.overlay = None;
            if key.code == KeyCode::Esc {
                self.last_ctrl_c = None;
                return None;
            }
            // All other closing keys keep their normal meaning, including
            // Ctrl+C, Tab, and a literal second question mark.
        }

        let ignore_prompt_key = matches!(
            self.overlay,
            Some(Overlay::Approval { .. } | Overlay::Confirm(_))
        ) && self.prompt_input_guard.ignore_key();

        // Ctrl+C is checked before overlays: two consecutive presses must
        // always be able to leave, even while a prompt owns input.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.on_ctrl_c();
        }
        self.last_ctrl_c = None;
        if key.code == KeyCode::Char('o') && key.modifiers == KeyModifiers::CONTROL {
            self.toggle_work_details();
            return None;
        }
        // Navigation never resolves a prompt. It remains available during the
        // quiet window, after the guard has seen and accounted for the key.
        if matches!(self.overlay, Some(Overlay::Approval { .. })) {
            match key.code {
                KeyCode::PageUp => self.scroll_up(10),
                KeyCode::PageDown => self.scroll_down(10),
                KeyCode::Home | KeyCode::End => {
                    self.selection = None;
                    return self.on_scroll_key(key);
                }
                KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                    self.follow_newest()
                }
                KeyCode::Up => self.approval_scroll = self.approval_scroll.saturating_sub(1),
                KeyCode::Down => {
                    self.approval_scroll = self
                        .approval_scroll
                        .saturating_add(1)
                        .min(self.approval_scroll_limit);
                }
                _ => {
                    if ignore_prompt_key {
                        return None;
                    }
                    return self.on_approval_key(key);
                }
            }
            self.selection = None;
            return None;
        }
        if matches!(self.overlay, Some(Overlay::Confirm(_))) {
            return self.on_confirm_key(key, ignore_prompt_key);
        }
        if ignore_prompt_key {
            return None;
        }

        match &self.overlay {
            Some(Overlay::Approval { .. }) => return self.on_approval_key(key),
            Some(Overlay::Questionnaire { .. }) => return self.on_questionnaire_key(key),
            Some(Overlay::Palette { .. }) => return self.on_palette_key(key),
            Some(Overlay::ResourcePicker { .. }) => {
                return self.on_resource_picker_key(key);
            }
            Some(Overlay::HistorySearch { .. }) => return self.on_history_search_key(key),
            Some(Overlay::Confirm(_)) => unreachable!("confirmations are handled above"),
            Some(Overlay::Shortcuts) | None => {}
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
                self.open_overlay(Overlay::Palette {
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
            (KeyCode::Char('?'), modifiers)
                if !closing_shortcuts
                    && self.composer.is_empty()
                    && (modifiers == KeyModifiers::NONE || modifiers == KeyModifiers::SHIFT) =>
            {
                self.selection = None;
                self.open_overlay(Overlay::Shortcuts);
                None
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
            // Draft lines own the arrows until their edge. Beyond that, keep
            // the existing history and delegated-agent navigation order.
            (KeyCode::Up, _) => {
                let ranges = self
                    .composer
                    .registered_ranges(self.attachment_placeholders());
                if !self.composer.move_up_over(&ranges) && !self.inspect_previous_child() {
                    self.composer.recall_previous();
                }
                None
            }
            (KeyCode::Down, _) => {
                let ranges = self
                    .composer
                    .registered_ranges(self.attachment_placeholders());
                if !self.composer.move_down_over(&ranges) && !self.composer.recall_next() {
                    self.inspect_next_child();
                }
                None
            }
            (KeyCode::Home | KeyCode::End, _) if self.composer.is_empty() => {
                self.on_scroll_key(key)
            }
            _ => self.on_composer_key(key),
        }
    }

    pub(super) fn on_confirm_key(&mut self, key: KeyEvent, ignore_key: bool) -> Option<Action> {
        let Some(Overlay::Confirm(dialog)) = &mut self.overlay else {
            return None;
        };
        match key.code {
            KeyCode::Up => dialog.scroll = dialog.scroll.saturating_sub(1),
            KeyCode::Down => {
                dialog.scroll = dialog.scroll.saturating_add(1).min(dialog.scroll_limit)
            }
            KeyCode::PageUp => dialog.scroll = dialog.scroll.saturating_sub(10),
            KeyCode::PageDown => {
                dialog.scroll = dialog.scroll.saturating_add(10).min(dialog.scroll_limit)
            }
            _ => {
                if ignore_key {
                    return None;
                }
                let (accept, offered) = match key.code {
                    KeyCode::Char('y') => (true, Some(0)),
                    KeyCode::Char('n') | KeyCode::Esc => (false, None),
                    // Retain the account numbers already printed by rotation.
                    KeyCode::Char(digit @ '1'..='9') if dialog.rotation.is_some() => {
                        let position = digit.to_digit(10)? as usize - 1;
                        let index = dialog
                            .rotation
                            .as_ref()?
                            .request()
                            .eligible
                            .iter()
                            .position(|member| member.position == position)?;
                        (true, Some(index))
                    }
                    _ => return None,
                };
                let Some(Overlay::Confirm(dialog)) = self.overlay.take() else {
                    return None;
                };
                let outcome = if accept {
                    *dialog.accept
                } else {
                    *dialog.cancel
                };
                return match outcome {
                    ConfirmOutcome::Action(action) => {
                        match &action {
                            Action::Quit => {
                                self.cancel_pending_prompts();
                                self.should_quit = true;
                            }
                            Action::StartAgent { .. }
                            | Action::FollowUpAgent { .. }
                            | Action::ResumeAgent { .. } => {
                                self.composer.clear();
                            }
                            _ => {}
                        }
                        Some(action)
                    }
                    ConfirmOutcome::Dismiss => None,
                    ConfirmOutcome::SwitchAccount | ConfirmOutcome::StayAccount => {
                        if let Some(prompt) = dialog.rotation {
                            self.answer_rotation(*prompt, if accept { offered } else { None });
                        }
                        None
                    }
                };
            }
        }
        self.selection = None;
        None
    }

    /// Consumes the owned responder and records the account decision.
    fn answer_rotation(
        &mut self,
        prompt: smith_host::rotation::RotationPrompt,
        offered: Option<usize>,
    ) {
        let request = prompt.request().clone();
        let outgoing = request.outgoing.label.clone();
        match offered.and_then(|index| request.eligible.get(index)) {
            Some(member) => {
                let notice = crate::accounts::switch_notice(&outgoing, &member.label, false);
                prompt.switch_to(member.position);
                self.transcript.push_notice(NoticeKind::Account, &notice);
            }
            None => {
                let notice = crate::accounts::declined_notice(
                    &outgoing,
                    request.outgoing_resets_at_ms,
                    crate::accounts::now_ms(),
                );
                prompt.decline();
                self.transcript.push_notice(NoticeKind::Account, &notice);
            }
        }
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
        if matches!(
            self.overlay,
            Some(
                Overlay::Palette { .. }
                    | Overlay::ResourcePicker { .. }
                    | Overlay::HistorySearch { .. }
            )
        ) {
            match self.overlay.take() {
                Some(Overlay::Palette {
                    restore_on_escape: Some(original),
                    ..
                })
                | Some(Overlay::ResourcePicker {
                    restore_on_escape: original,
                    ..
                })
                | Some(Overlay::HistorySearch { original, .. }) => self.composer.replace(original),
                _ => {}
            }
        }
        self.composer.stash_for_recall();
        None
    }

    pub(super) fn open_history_search(&mut self) {
        self.open_overlay(Overlay::HistorySearch {
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

        let mut body = vec![String::new()];
        if self.is_busy() {
            body.push("· a turn is still running".to_owned());
        }
        if self.pending_approval_count() > 0 {
            body.push("· an approval is pending".to_owned());
        }
        if self.pending_questionnaire_count() > 0 {
            body.push("· a questionnaire is pending".to_owned());
        }
        if !self.running_tasks.is_empty() {
            let ids = self
                .running_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            body.push(format!(
                "· {} background {} running: {ids}",
                self.running_tasks.len(),
                if self.running_tasks.len() == 1 {
                    "task"
                } else {
                    "tasks"
                }
            ));
        }
        let mut dialog = ConfirmDialog::new(
            "exit",
            crate::theme::Tone::Warning,
            body,
            "quit",
            ConfirmOutcome::Action(Action::Quit),
            ConfirmOutcome::Dismiss,
        );
        dialog.warning = Some((
            "quit with work in progress?".to_owned(),
            crate::theme::Tone::Heading,
        ));
        dialog.accept_tone = crate::theme::Tone::Danger;
        dialog.cancel_key = "n";
        dialog.cancel_label = "keep working".to_owned();
        dialog.hint = "y quit · n keep working".to_owned();
        self.open_overlay(Overlay::Confirm(dialog));
        None
    }

    pub(super) fn cancel_pending_prompts(&mut self) {
        if self.overlay.as_ref().is_some_and(Overlay::is_prompt) {
            match self.overlay.take() {
                Some(Overlay::Approval { prompt, .. }) => prompt.cancel(),
                Some(Overlay::Questionnaire { state }) => {
                    self.resolve_questionnaire(state, QuestionnaireResolution::Cancelled);
                }
                Some(Overlay::Confirm(_)) => {}
                _ => unreachable!("only prompts are cancelled"),
            }
        }
        while let Some(prompt) = self.pending_prompts.pop_front() {
            match prompt {
                PendingPrompt::Approval(prompt, _) => prompt.cancel(),
                PendingPrompt::Questionnaire(state) => {
                    self.resolve_questionnaire(state, QuestionnaireResolution::Cancelled);
                }
                PendingPrompt::Confirm(_) => {}
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
                    Err(message) if !commands::has_exact_name(&input) => {
                        let Some(command) = matches.get(selected).copied() else {
                            if let Some(Overlay::Palette { error, .. }) = &mut self.overlay {
                                *error = Some(message);
                            }
                            return None;
                        };
                        let completed = commands::completion(command);
                        match commands::parse(&completed) {
                            Ok(command) => {
                                // Tab's argument separator does not belong in accepted history.
                                let completed = completed.trim_end().to_owned();
                                // Dispatch the selected command as the text
                                // the user chose, so accepted history records
                                // `/status` or `/effort`, rather than the
                                // search query. If an idle guard or a direct
                                // resource validation rejects it, restore the
                                // original query so the draft remains intact.
                                let original = input.clone();
                                self.composer.replace(completed.clone());
                                let action = self.dispatch_command(command);
                                if self.composer.text() == completed {
                                    self.composer.replace(original);
                                } else if let Some(Overlay::ResourcePicker {
                                    restore_on_escape,
                                    ..
                                }) = &mut self.overlay
                                {
                                    *restore_on_escape = original;
                                }
                                action
                            }
                            Err(completion_error) => {
                                if let Some(Overlay::Palette { error, .. }) = &mut self.overlay {
                                    *error = Some(completion_error);
                                }
                                None
                            }
                        }
                    }
                    Err(message) => {
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
        let ranges = self
            .composer
            .registered_ranges(self.attachment_placeholders());
        match (key.code, key.modifiers) {
            (KeyCode::Enter, m)
                if m.contains(KeyModifiers::SHIFT) || m.contains(KeyModifiers::ALT) =>
            {
                self.composer.insert('\n');
                None
            }
            (KeyCode::Enter, _) => {
                if self.composer.cursor() > 0
                    && self.composer.text().chars().nth(self.composer.cursor() - 1) == Some('\\')
                {
                    self.composer.backspace();
                    self.composer.insert('\n');
                    return None;
                }
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
                                expected_turn: self.live_turn.active_turn.clone(),
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
                    self.transcript.push_shell_shortcut(&command);
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
                        if !existing.state.accepts_follow_up() {
                            // The error goes to the root transcript, which the
                            // inspector is covering, so it also has to be
                            // visible where the user typed: the panel row and
                            // the child's own view carry the same state.
                            self.transcript.push_error(format!(
                                "`{agent}` is {}; it takes a follow-up once it settles{}",
                                existing.state.label(),
                                if existing.state.is_resumable() {
                                    ", and `/agent resume <id>` continues its exact checkpoint"
                                } else {
                                    ""
                                }
                            ));
                            self.push_child_error(
                                agent,
                                format!("follow-up refused while {}", existing.state.label()),
                            );
                            return None;
                        }
                        let model = match &self.status.provider {
                            Some(provider) => format!("{provider}/{}", self.status.model),
                            None => self.status.model.clone(),
                        };
                        self.composer.record_current();
                        let content = format!(
                            "child: {agent}\noperation: new follow-up turn\ncontinuity: reuse prior child history and cumulative limits\nprovider/model: {model}\nprovider spend: yes\ncheckpoint replay: no"
                        );
                        self.confirm_child(
                            "existing child follow-up",
                            "start follow-up and spend provider tokens",
                            "y start follow-up turn · n/esc cancel",
                            content,
                            Action::FollowUpAgent {
                                child_id: agent.clone(),
                                task: self.expand_pasted(task),
                            },
                        );
                        return None;
                    }
                    let profile_detail = self
                        .resources
                        .child_agents
                        .iter()
                        .find(|entry| entry.id.strip_prefix("agent:") == Some(agent.as_str()))
                        .map_or("read-only child profile", |entry| entry.detail.as_str());
                    self.composer.record_current();
                    let content = format!(
                        "profile: {agent}\nconfiguration: {profile_detail}\nworkspace: read-only\nturn limit: 1\nprovider spend: yes\nresult: bounded child summary"
                    );
                    self.confirm_child(
                        "read-only child agent",
                        "start child and spend provider tokens",
                        "y start read-only child · n/esc cancel",
                        content,
                        Action::StartAgent {
                            preset: agent.clone(),
                            task: self.expand_pasted(task),
                        },
                    );
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
                self.composer.move_to_start();
                None
            }
            (KeyCode::End, _) => {
                self.composer.move_to_end();
                None
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.composer.move_home();
                None
            }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.composer.move_end();
                None
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                self.composer.delete_word_left_over(&ranges);
                None
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.composer.delete_to_line_start();
                None
            }
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                self.composer.delete_to_line_end();
                None
            }
            (KeyCode::Char('b'), KeyModifiers::ALT) => {
                self.composer.move_word_left_over(&ranges);
                None
            }
            (KeyCode::Char('f'), KeyModifiers::ALT) => {
                self.composer.move_word_right_over(&ranges);
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
                    self.open_overlay(Overlay::Palette {
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
            KeyCode::Home => self.scroll_up(usize::MAX),
            KeyCode::End => self.follow_newest(),
            _ => {}
        }
        None
    }

    /// Scrolls up, which pauses following.
    pub fn scroll_up(&mut self, lines: usize) {
        if lines == 0 {
            return;
        }
        self.scroll_to_block = None;
        if self.scroll_limit == 0 {
            return;
        }
        self.scroll_back = self
            .scroll_back
            .saturating_add(lines)
            .min(self.scroll_limit);
        self.following = self.scroll_back == 0;
    }

    /// Scrolls down, resuming following at the bottom unless reading a result.
    pub fn scroll_down(&mut self, lines: usize) {
        if lines == 0 {
            return;
        }
        self.scroll_to_block = None;
        self.scroll_back = self
            .scroll_back
            .min(self.scroll_limit)
            .saturating_sub(lines);
        if self.scroll_back == 0 && self.result_scroll_revision.is_none() {
            self.following = true;
        }
    }

    /// Jumps to newest output and resumes following.
    pub fn follow_newest(&mut self) {
        self.scroll_to_block = None;
        self.result_scroll_revision = None;
        self.scroll_back = 0;
        self.following = true;
    }

    pub(crate) fn output_after_result(&self) -> bool {
        self.result_scroll_revision
            .is_some_and(|revision| self.transcript.append_revision() != revision)
    }

    /// Synchronizes scroll state with the viewport computed by the renderer.
    ///
    /// Keeping the visible offset stable while paused prevents streaming output
    /// or a resize from pulling the reader toward the newest content.
    #[cfg(test)]
    pub(crate) fn sync_scroll_limit(&mut self, limit: usize) {
        if self.following {
            self.scroll_limit = limit;
            self.scroll_back = 0;
            return;
        }

        let visible_offset = self
            .scroll_limit
            .saturating_sub(self.scroll_back.min(self.scroll_limit));
        self.scroll_limit = limit;
        if limit == 0 && self.result_scroll_revision.is_none() {
            self.follow_newest();
            return;
        }

        self.scroll_back = limit.saturating_sub(visible_offset.min(limit));
        if self.scroll_back == 0 && self.result_scroll_revision.is_none() {
            self.following = true;
        }
    }
}
