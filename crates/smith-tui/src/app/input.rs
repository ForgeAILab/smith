//! Keyboard, mouse, composer history, exit, and scrolling transitions.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use smith_host::approval::PromptScope;

use crate::commands::{self, CommandAction};
use crate::questionnaire::QuestionnaireResolution;
use crate::references::{ComposerReference, parse_references};
use crate::status::Activity;

use super::state::*;

impl App {
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
            (KeyCode::Up, _) => {
                if self.composer.recall_previous() {
                    None
                } else {
                    self.on_scroll_key(key)
                }
            }
            (KeyCode::Down, _) if self.composer.is_recalling() => {
                self.composer.recall_next();
                None
            }
            (KeyCode::Down | KeyCode::Home | KeyCode::End, _) => self.on_scroll_key(key),
            _ => self.on_composer_key(key),
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
                            self.transcript.push_error(format!(
                                "`{agent}` is {}; use `/agent {agent}` to inspect it{}",
                                existing.state,
                                if existing.state == "interrupted" {
                                    " and `/agent resume <id>` for exact continuation"
                                } else {
                                    ""
                                }
                            ));
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
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(1),
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
