use super::{
    Action, ApprovalPrompt, CancelReason, Context, InteractiveExit, KeyCode, KeyEventKind,
    KeyModifiers, LocalResult, LocalShellIdentity, MouseOutcome, NoticeKind, RecoveryAction,
    RecoveryReport, Result, RotationPrompt, SessionControl, SubmissionTarget, TermEvent, TuiLoop,
    account_entries, account_status, attach_from_clipboard, copy_selection_to_clipboard,
    dispatch_prepared_with_materialization, follow_up_agent, handle_local_command, local_command,
    reconfigure_exit, resume_agent, start_agent, start_local_shell, start_review, switch_account,
};

impl TuiLoop<'_> {
    pub(super) async fn on_terminal_event<B>(
        &mut self,
        terminal: &mut ratatui::Terminal<B>,
        key: std::io::Result<TermEvent>,
    ) -> Result<Option<InteractiveExit>>
    where
        B: ratatui::backend::Backend,
        B::Error: Send + Sync + 'static,
    {
        match key.context("reading a terminal event")? {
            // `Ctrl+V` is the explicit "attach from clipboard" chord:
            // terminals deliver ordinary pastes as bracketed text, but
            // an image on the clipboard can only be fetched by asking
            // the platform directly.
            TermEvent::Key(key)
                if key.kind != KeyEventKind::Release
                    && key.code == KeyCode::Char('v')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                attach_from_clipboard(&mut self.app);
                self.dirty = true;
            }
            TermEvent::Key(key) => {
                if let Some(action) = self.app.on_key(key)
                    && let Some(exit) = self.on_action(action).await
                {
                    return Ok(Some(exit));
                }
                self.dirty = true;
            }
            TermEvent::Paste(text) => {
                self.app.on_paste(&text);
                self.dirty = true;
            }
            TermEvent::Mouse(mouse) => match self.app.on_mouse(mouse) {
                MouseOutcome::Ignored => {}
                MouseOutcome::Redraw => self.dirty = true,
                MouseOutcome::CopySelection => {
                    // Drawn here rather than deferred to the frame
                    // tick: the selected text exists only in the frame
                    // buffer, and a runtime event arriving in between
                    // would clear the selection before it could be
                    // read — a release that silently copied nothing.
                    let mut selected = None;
                    terminal.draw(|frame| {
                        smith_tui::render::layout(frame.area(), &self.app, self.theme)
                            .apply(&mut self.app);
                        smith_tui::render::draw(frame, &self.app, self.theme);
                        selected = smith_tui::selected_text(frame, &self.app);
                    })?;
                    self.dirty = false;
                    // A drag across blank space yields nothing, and
                    // clobbering the clipboard with an empty string
                    // would lose whatever the user had there.
                    if let Some(text) = selected {
                        copy_selection_to_clipboard(&mut self.app, &text);
                    }
                }
            },
            TermEvent::Resize(_, _) => self.dirty = true,
            _ => {}
        }
        Ok(None)
    }

    async fn on_action(&mut self, action: Action) -> Option<InteractiveExit> {
        match action {
            Action::Submit { submission, target } => self.on_submit(submission, target).await,
            Action::FileCommand {
                typed,
                name,
                arguments,
                queue,
            } => match self.commands.prepare(&self.app, typed, &name, &arguments) {
                Ok(submission) if queue => {
                    self.host.set_goal_continuation_enabled(false);
                    self.app.queue_prepared(submission);
                }
                Ok(submission) => {
                    let Action::Submit { submission, target } =
                        self.app.submit_prepared(submission)
                    else {
                        unreachable!("prepared prompt produces a submission");
                    };
                    self.on_submit(submission, target).await;
                }
                Err(error) => self.app.transcript.push_error(error),
            },
            Action::TrustCommand { name, digest } => {
                let report = match self.commands.trust(&name, &digest) {
                    Ok(report) => {
                        self.app.set_command_catalog(self.commands.catalog());
                        report
                    }
                    Err(error) => smith_client::commands_report::CommandsReport::Error(error),
                };
                self.app
                    .show_local_report(LocalResult::Commands(Box::new(report)));
            }
            Action::RunShell { command } => self.on_run_shell(command).await,
            Action::Interrupt => self.on_interrupt(),
            Action::BackgroundShell => self.on_background_shell(),
            Action::Quit => return Some(self.on_quit()),
            // An account switch is live pool state, so it is
            // applied here rather than by tearing the session
            // down and rebuilding it around a new selection.
            Action::Reconfigure(command) => return self.on_reconfigure(command).await,
            Action::Command(command) => self.on_command(command).await,
            Action::TrustMcpServer { server } => self.on_trust_mcp_server(server),
            Action::TrustSkill { skill: name } => self.on_trust_skill(name),
            Action::ApplyUndo => self.on_apply_undo(),
            Action::CancelUndo => self.on_cancel_undo(),
            Action::ApplyRedo => self.on_apply_redo(),
            Action::CancelRedo => self.on_cancel_redo(),
            Action::ApplyRevert { scope, fingerprint } => self.on_apply_revert(scope, fingerprint),
            Action::CancelRevert { scope, fingerprint } => {
                self.on_cancel_revert(scope, fingerprint)
            }
            Action::StartReview { scope } => self.on_start_review(scope),
            Action::StartAgent { preset, task } => self.on_start_agent(preset, task),
            Action::FollowUpAgent { child_id, task } => self.on_follow_up_agent(child_id, task),
            Action::ResumeAgent { child_id } => self.on_resume_agent(child_id),
        }
        None
    }

    async fn on_submit(
        &mut self,
        submission: smith_tui::app::PreparedSubmission,
        target: SubmissionTarget,
    ) {
        self.host.set_goal_continuation_enabled(false);
        dispatch_prepared_with_materialization(
            &mut self.app,
            self.session,
            self.project,
            submission,
            target,
        )
        .await;
    }

    async fn on_run_shell(&mut self, command: String) {
        let echo = self
            .app
            .transcript
            .latest_shell_echo()
            .expect("submitted shell echo");
        self.shell_shortcuts.dispatched(self.host, echo);
        let identity = start_local_shell(
            echo,
            self.session.clone(),
            command,
            self.host
                .runtime()
                .policy()
                .turn_time_limit_ms
                .unwrap_or(600_000),
            self.local_shell_approvals.clone(),
            self.local_tx.clone(),
        )
        .await;
        match identity {
            Some(LocalShellIdentity::Turn(turn)) => self.app.track_shell_shortcut(turn, echo),
            Some(LocalShellIdentity::Call(call)) => {
                self.app.transcript.bind_shell_shortcut(echo, call.as_str())
            }
            None => {}
        }
    }

    fn on_interrupt(&mut self) {
        if let Err(error) = self
            .session
            .interrupt_current_turn(CancelReason::UserRequested)
        {
            self.app
                .transcript
                .push_error(format!("turn interruption failed: {error}"));
        }
    }

    fn on_background_shell(&mut self) {
        // Kept distinct from `Action::Interrupt`: this
        // never kills the group, it only asks the
        // registry to adopt whatever foreground call
        // is currently running, if any.
        if self
            .host
            .background_tasks()
            .trigger_manual_backgrounding(self.session.id())
        {
            self.app
                .transcript
                .push_notice(NoticeKind::Background, "command moved to the background");
        } else {
            self.app.push_notice(
                NoticeKind::BackgroundUnavailable,
                "no foreground shell command is running",
            );
        }
    }

    fn on_quit(&self) -> InteractiveExit {
        InteractiveExit::Quit(
            Box::new(self.app.session_usage()),
            self.app.status.price().cloned(),
            self.app.status.cache_summary().map(Box::new),
        )
    }

    async fn on_reconfigure(&mut self, command: SessionControl) -> Option<InteractiveExit> {
        match command {
            SessionControl::Account(position) => {
                match switch_account(self.credential_pool.as_ref(), &mut self.accounts, position)
                    .await
                {
                    Some(notice) => {
                        self.app.transcript.push_notice(NoticeKind::Account, notice);
                        self.app
                            .set_accounts(account_entries(self.credential_pool.as_ref()));
                        self.app.status.account = account_status(self.credential_pool.as_ref());
                    }
                    None => self
                        .app
                        .push_notice(NoticeKind::AccountUnchanged, "already using that account"),
                }
            }
            command => {
                if let Some(exit) = reconfigure_exit(&mut self.app, command) {
                    return Some(exit);
                }
            }
        }
        None
    }

    async fn on_command(&mut self, command: smith_client::commands::HostCommand) {
        handle_local_command(
            &mut self.app,
            self.host,
            self.project,
            self.mcp.as_deref(),
            &self.skills,
            &self.commands,
            command,
        )
        .await;
    }

    fn on_trust_mcp_server(&mut self, server: String) {
        self.app
            .show_local_report(smith_client::local_result::LocalResult::Mcp(Box::new(
                local_command::mcp::trust(self.mcp.as_deref(), &server),
            )));
    }

    fn on_trust_skill(&mut self, name: String) {
        let report = local_command::skills::trust(&self.skills, &name);
        if matches!(
            &report,
            smith_client::skills_report::SkillsReport::Trusted { .. }
        ) {
            self.trusted_skill_pending = true;
        }
        self.app
            .show_local_report(smith_client::local_result::LocalResult::Skills(Box::new(
                report,
            )));
    }

    fn on_apply_undo(&mut self) {
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(
                local_command::recovery::undo(self.host),
            )));
    }

    fn on_cancel_undo(&mut self) {
        self.host.changes().record_undo_cancelled();
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(RecoveryReport::Cancelled(
                RecoveryAction::Undo,
            ))));
    }

    fn on_apply_redo(&mut self) {
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(
                local_command::recovery::redo(self.host),
            )));
    }

    fn on_cancel_redo(&mut self) {
        self.host.changes().record_redo_cancelled();
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(RecoveryReport::Cancelled(
                RecoveryAction::Redo,
            ))));
    }

    fn on_apply_revert(&mut self, scope: String, fingerprint: String) {
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(
                local_command::recovery::revert(self.host, self.project, scope, &fingerprint),
            )));
    }

    fn on_cancel_revert(&mut self, scope: String, fingerprint: String) {
        self.host
            .changes()
            .record_revert_event(&scope, &fingerprint, "cancelled");
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(RecoveryReport::Cancelled(
                RecoveryAction::Revert,
            ))));
    }

    fn on_start_review(&mut self, scope: String) {
        start_review(self.host, self.project, scope, self.local_tx.clone());
    }

    fn on_start_agent(&mut self, preset: String, task: String) {
        start_agent(self.host, self.agents, preset, task, self.local_tx.clone());
    }

    fn on_follow_up_agent(&mut self, child_id: String, task: String) {
        follow_up_agent(self.host, child_id, task, self.local_tx.clone());
    }

    fn on_resume_agent(&mut self, child_id: String) {
        resume_agent(self.host, child_id, self.local_tx.clone());
    }

    pub(super) fn on_approval(&mut self, prompt: Option<ApprovalPrompt>) {
        match prompt {
            Some(prompt) => {
                if let Some(prompt) = self.local_shell_approvals.resolve(prompt) {
                    self.app.present_approval(prompt);
                    self.dirty = true;
                }
            }
            None => self.approvals = None,
        }
    }

    pub(super) fn on_rotation(&mut self, offer: Option<RotationPrompt>) {
        match offer {
            Some(prompt) => {
                self.app.present_rotation(prompt);
                self.dirty = true;
            }
            None => self.rotations = None,
        }
    }

    pub(super) fn on_interaction_notice(&mut self, notice: Option<smith_host::InteractionNotice>) {
        match notice {
            Some(notice) => {
                self.interactions.apply_notice(&mut self.app, notice);
                self.dirty = true;
            }
            None => self.interactions.close_receiver(),
        }
    }
}
