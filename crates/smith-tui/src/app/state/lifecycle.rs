use super::*;

impl App {
    /// A fresh client for `model` rooted at `project`.
    pub fn new(model: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            transcript: Transcript::new(),
            feedback: None,
            transcript_cache: RefCell::default(),
            status: Status::new(model, project),
            module_status: Vec::new(),
            cache_miss_notices: false,
            composer: Composer::new(),
            command_catalog: smith_client::file_commands::CommandCatalog::empty(),
            overlay: None,
            prompt_input_guard: crate::app::prompts::PromptInputGuard::default(),
            pending_prompts: VecDeque::new(),
            questionnaire_resolutions: VecDeque::new(),
            children: BTreeMap::new(),
            child_clocks: BTreeMap::new(),
            child_conversations: BTreeMap::new(),
            child_dismiss_at: BTreeMap::new(),
            retired_children: BTreeSet::new(),
            inspected_child: None,
            inspected_detail: None,
            child_counts: BTreeMap::new(),
            pending_spawns: VecDeque::new(),
            delegated_usage: BTreeMap::new(),
            child_usage_bindings: BTreeMap::new(),
            delegated_contributors: BTreeSet::new(),
            running_tasks: Vec::new(),
            task_clocks: BTreeMap::new(),
            plan: None,
            live_turn: LiveTurn::default(),
            work_details: false,
            approval_scroll: 0,
            approval_scroll_limit: 0,
            local_shell_turn: None,
            resources: RuntimeResources::default(),
            following: true,
            scroll_back: 0,
            scroll_limit: 0,
            scroll_to_block: None,
            result_scroll_revision: None,
            selection: None,
            tick: 0,
            should_quit: false,
            pasted_chunks: Vec::new(),
            paste_counter: 0,
            image_attachments: Vec::new(),
            image_counter: 0,
            turn_summary: None,
            turn_summary_revision: None,
            turn_block_revision: 0,
            turn_usage: crate::status::TurnUsage::default(),
            last_ctrl_c: None,
            last_event_seq: None,
            stream_gap: None,
            pending_recovered_events: 0,
            pending_lost_range: None,
            speculative: SpeculativeState::default(),
            pending_input: PendingInputState::default(),
            last_cache_notice_turn: None,
        }
    }

    /// Replaces module status with the host's latest items, removing absent ones.
    pub fn set_module_status(&mut self, items: Vec<ModuleStatusItem>) {
        self.module_status = items;
    }

    /// Current host-projected module items, in deterministic display order.
    pub fn module_status(&self) -> &[ModuleStatusItem] {
        &self.module_status
    }

    /// Routes a notice to the place fixed by its kind.
    pub fn push_notice(&mut self, kind: NoticeKind, text: impl Into<String>) {
        match kind.persistence() {
            NoticePersistence::Transcript => self.transcript.push_notice(kind, text),
            NoticePersistence::Feedback => {
                self.feedback = Some(Notice {
                    kind,
                    text: text.into(),
                });
            }
        }
    }

    /// The feedback currently replacing the hint row.
    pub fn feedback_notice(&self) -> Option<&Notice> {
        self.feedback.as_ref()
    }

    /// Clears feedback when the host handles a key outside `on_key`.
    pub fn clear_feedback(&mut self) {
        self.feedback = None;
    }

    /// Returns root turn identity, work, provider progress, and clocks to idle.
    /// Used at turn boundaries and when the host rebinds a retained app.
    /// Accounting, attempt deduplication, and shell attribution keep their
    /// existing lifetimes outside this value.
    pub fn reset_live_turn(&mut self) {
        self.live_turn.reset();
        self.status.activity = Activity::Idle;
    }

    /// Detaches idle host state while keeping the conversation and composer.
    pub fn rebind_host(&mut self) {
        self.reset_live_turn();
        self.overlay = None;
        self.prompt_input_guard = crate::app::prompts::PromptInputGuard::default();
        self.last_event_seq = None;
        self.stream_gap = None;
        self.pending_recovered_events = 0;
        self.pending_lost_range = None;
        self.pending_spawns.clear();
        self.local_shell_turn = None;
        self.speculative.clear();
        self.selection = None;
        // A change notice must not resume following a previously read result.
        self.scroll_to_block = None;
        self.result_scroll_revision = None;
    }

    /// Moves recall history, including stored attachments, to a new session.
    pub fn inherit_composer_history(&mut self, previous: &mut Self) {
        self.composer = std::mem::take(&mut previous.composer);
        self.composer.clear();
        self.pasted_chunks = std::mem::take(&mut previous.pasted_chunks);
        self.paste_counter = previous.paste_counter;
        self.image_attachments = std::mem::take(&mut previous.image_attachments);
        self.image_counter = previous.image_counter;
    }

    /// Replaces coordinator state, retaining logs only for listed children.
    pub fn replace_children<I>(&mut self, children: I)
    where
        I: IntoIterator<Item = (String, ChildState, Option<String>)>,
    {
        self.children.clear();
        for (child, state, detail) in children {
            self.restore_child(child, state, detail);
        }
        self.child_conversations
            .retain(|child, _| self.children.contains_key(child));
        for conversation in self.child_conversations.values_mut() {
            conversation.speculative.clear();
            conversation.live = false;
        }
        self.child_clocks.clear();
        self.child_counts.clear();
        self.child_dismiss_at.clear();
        self.retired_children.clear();
        if self
            .inspected_child
            .as_ref()
            .is_some_and(|child| !self.children.contains_key(child))
        {
            self.inspected_child = None;
        }
        self.inspected_detail = None;
    }

    /// Whether a runtime prompt still belongs to this host.
    pub fn has_pending_prompt(&self) -> bool {
        self.pending_approval_count() > 0
            || self.pending_questionnaire_count() > 0
            || matches!(&self.overlay, Some(Overlay::Confirm(dialog)) if dialog.rotation.is_some())
            || self.pending_prompts.iter().any(|prompt| {
                matches!(prompt, PendingPrompt::Confirm(dialog) if dialog.rotation.is_some())
            })
    }

    /// Enables or disables the layered `cache.miss_notices` presentation
    /// policy. Canonical cache state is collected regardless of this flag.
    pub fn set_cache_miss_notices(&mut self, enabled: bool) {
        self.cache_miss_notices = enabled;
    }

    /// Replays canonical cache events into status without rebuilding
    /// conversation blocks. Used when a persistent host restores its journal.
    pub fn restore_cache_events<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        self.status.replay_cache_events(events);
        if self.cache_miss_notices
            && let Some(summary) = self.status.cache_summary()
            && summary.significant()
            && self.last_cache_notice_turn.as_deref() != Some(summary.turn.as_str())
        {
            self.transcript
                .push_notice(NoticeKind::Cache, summary.render_notice());
            self.last_cache_notice_turn = Some(summary.turn);
        }
    }

    /// Replaces the local, credential-free picker inventory.
    pub fn set_resources(&mut self, resources: RuntimeResources) {
        self.status.context_window = (resources.context_windows.len() > 1)
            .then(|| resources.context_window.clone())
            .flatten();
        self.resources = resources;
    }

    /// Seeds one already-persisted child before live event subscription.
    ///
    /// Recovery events may be journaled before a terminal client attaches;
    /// this owner-supplied projection keeps inspection and `@child-id`
    /// continuation available without replaying or parsing journal prose.
    pub fn restore_child(
        &mut self,
        child_id: impl Into<String>,
        state: ChildState,
        detail: Option<String>,
    ) {
        self.children.insert(
            child_id.into(),
            ChildSummary {
                state,
                detail,
                // The coordinator's own status has no profile field, and a
                // restored child was never freshly spawned in this process,
                // so there is no spawn correlation to draw one from either.
                profile: None,
            },
        );
    }

    /// Replaces the running-background-task listing with the host's latest
    /// registry poll.
    ///
    /// A wholesale replace rather than an incremental diff: the registry
    /// already filters to non-terminal tasks, so a task's disappearance from
    /// `tasks` is itself the terminal signal — no separate removal path is
    /// needed.
    pub fn set_running_tasks(&mut self, tasks: Vec<RunningTaskSummary>) {
        // First sight starts a task's panel clock; disappearance ends it.
        // A re-used id restarting at zero is honest: the registry only
        // returns live tasks, so reappearance means a new run.
        self.task_clocks = tasks
            .iter()
            .map(|task| {
                let started = self
                    .task_clocks
                    .get(&task.task_id)
                    .copied()
                    .unwrap_or_else(Instant::now);
                (task.task_id.clone(), started)
            })
            .collect();
        self.running_tasks = tasks;
    }

    /// Elapsed time since one running background task was first seen.
    pub fn task_elapsed(&self, task_id: &str) -> Option<Duration> {
        self.task_clocks
            .get(task_id)
            .map(|started| started.elapsed())
    }

    /// Compact footer segment naming running background tasks, or `None`
    /// when none are running.
    ///
    /// Bounded to a few identities plus a remainder count so a chatty session
    /// running many tasks cannot grow the identity footer without limit.
    pub fn render_running_tasks_footer(&self) -> Option<String> {
        if self.running_tasks.is_empty() {
            return None;
        }
        const SHOWN: usize = 3;
        let mut label = self
            .running_tasks
            .iter()
            .take(SHOWN)
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let remaining = self.running_tasks.len().saturating_sub(SHOWN);
        if remaining > 0 {
            label.push_str(&format!(", +{remaining} more"));
        }
        Some(format!("bg {label}"))
    }

    /// Whether a turn is in flight.
    pub fn is_busy(&self) -> bool {
        matches!(
            self.status.activity,
            Activity::Working | Activity::Interrupting
        )
    }

    /// Whether anything would be lost by quitting now.
    pub fn has_live_work(&self) -> bool {
        self.is_busy()
            || self.has_pending_input()
            || !self.pending_prompts.is_empty()
            || !self.running_tasks.is_empty()
            || self.overlay.as_ref().is_some_and(Overlay::is_prompt)
    }

    /// Number of prompts waiting behind the visible one.
    pub fn queued_prompt_count(&self) -> usize {
        self.pending_prompts.len()
    }

    /// Number of approval prompts still awaiting one decision each.
    pub fn pending_approval_count(&self) -> usize {
        let visible = usize::from(matches!(self.overlay, Some(Overlay::Approval { .. })));
        visible.saturating_add(
            self.pending_prompts
                .iter()
                .filter(|prompt| matches!(prompt, PendingPrompt::Approval(..)))
                .count(),
        )
    }

    /// Number of questionnaire requests awaiting one terminal answer each.
    pub fn pending_questionnaire_count(&self) -> usize {
        let visible = usize::from(matches!(self.overlay, Some(Overlay::Questionnaire { .. })));
        visible.saturating_add(
            self.pending_prompts
                .iter()
                .filter(|prompt| matches!(prompt, PendingPrompt::Questionnaire(_)))
                .count(),
        )
    }

    /// Takes the next questionnaire result for the host interaction adapter.
    ///
    /// Every visible or queued request contributes at most one result. Taking
    /// removes it, so a host loop cannot answer one runtime responder twice.
    pub fn take_questionnaire_resolution(&mut self) -> Option<(String, QuestionnaireResolution)> {
        self.questionnaire_resolutions.pop_front()
    }

    /// Advances the animation clock.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.expire_prompts();
    }

    /// Whether the footer should ask for the confirming `Ctrl+C` press.
    pub fn ctrl_c_exit_hint_active(&self) -> bool {
        self.last_ctrl_c
            .is_some_and(|pressed| pressed.elapsed() < FORCE_QUIT_WINDOW)
    }

    /// Expires the first-press footer hint after the double-press window.
    ///
    /// Returns whether visible footer state changed so the host can request an
    /// idle redraw without continuously animating the status line.
    pub fn expire_ctrl_c_exit_hint(&mut self) -> bool {
        self.expire_ctrl_c_exit_hint_at(Instant::now())
    }

    pub(crate) fn expire_ctrl_c_exit_hint_at(&mut self, now: Instant) -> bool {
        let expired = self
            .last_ctrl_c
            .is_some_and(|pressed| now.duration_since(pressed) >= FORCE_QUIT_WINDOW);
        if expired {
            self.last_ctrl_c = None;
        }
        expired
    }

    /// Monotonic elapsed time for the active turn.
    pub fn turn_elapsed(&self) -> Option<Duration> {
        self.live_turn
            .turn_started_at
            .map(|started| started.elapsed())
    }

    /// The newest live success summary, only while its turn is still last.
    pub(crate) fn visible_turn_summary(&self) -> Option<&str> {
        self.turn_summary
            .as_deref()
            .filter(|_| self.turn_summary_revision == Some(self.transcript.append_revision()))
    }

    /// Takes the parked live-stream gap so the host can replay it from the
    /// canonical journal.
    ///
    /// While a gap is parked the envelope that revealed it has **not** been
    /// applied. The host replays the missing range with
    /// [`App::apply_recovered`], then applies the gap's `deferred` envelope
    /// the same way.
    pub fn take_stream_gap(&mut self) -> Option<StreamGap> {
        self.stream_gap.take()
    }

    /// The live provider round-trip stage and how long it has been in it.
    pub fn provider_phase(&self) -> Option<(ProviderPhase, Duration)> {
        self.live_turn
            .provider_phase
            .map(|(phase, since)| (phase, since.elapsed()))
    }

    /// The admitted root retry identity and any remaining backoff.
    pub fn provider_retry(&self) -> Option<ProviderRetryProgress> {
        self.live_turn
            .provider_retry
            .as_ref()
            .map(|retry| ProviderRetryProgress {
                next_attempt: retry.next_attempt,
                max_attempts: retry.max_attempts,
                backoff_remaining: (!retry.started)
                    .then(|| retry.delay.saturating_sub(retry.received_at.elapsed())),
            })
    }

    /// Visible text from the newest live provider attempt.
    ///
    /// This is presentation-only speculative state. It is never returned from
    /// [`Transcript::blocks`] and therefore cannot become canonical history or
    /// journal-replayed output without an explicit runtime commit event.
    pub fn speculative_text(&self) -> Option<&str> {
        self.speculative.visible_text()
    }

    /// Number of provider attempts with output awaiting an explicit terminal.
    pub fn speculative_attempt_count(&self) -> usize {
        self.speculative.in_flight()
    }

    /// Projects metadata-only process-exit reconciliation into the transcript.
    ///
    /// Child, monitor, and background-task identities remain in the
    /// protected recovery record; the UI only needs deterministic counts and
    /// the explicit fact that process-owned work was not restarted.
    pub fn present_recovered_ephemeral_work(
        &mut self,
        interrupted_children: usize,
        interrupted_monitors: usize,
        interrupted_tasks: usize,
    ) {
        let report = RestoreReport::EphemeralWork {
            reason: "process_exit".to_owned(),
            children: interrupted_children,
            monitors: interrupted_monitors,
            tasks: interrupted_tasks,
        };
        if let Some(text) = smith_client::recovery_report::render_restore_plain(&report) {
            self.transcript.push_notice(report.notice_kind(), text);
        }
    }

    /// Attributes the next shell call to an admitted local shortcut turn.
    pub fn track_shell_shortcut(&mut self, turn: TurnId, echo: u64) {
        self.local_shell_turn = Some((turn, echo));
    }

    /// Enriches a protected live tool event with a reviewed local projection.
    pub fn set_tool_display(&mut self, call_id: &str, display: ToolCallDisplay) {
        self.transcript.set_tool_display(call_id, display);
    }

    /// Attaches host-supplied, credential-redacted result lines to a tool row.
    pub fn set_tool_result_preview(&mut self, call_id: &str, preview: impl AsRef<str>) {
        self.transcript.set_tool_result_preview(call_id, preview);
        if let Some(status) = self.transcript.tool_status(call_id)
            && let Some(work) = &mut self.live_turn.work
            && let Some((_, work_status, _)) = work.tools.get_mut(call_id)
        {
            *work_status = status;
        }
    }

    /// Toggles bounded, redaction-safe transcript output and live work detail.
    pub fn toggle_work_details(&mut self) {
        self.work_details = !self.work_details;
        self.approval_scroll = 0;
    }

    /// Render-ready lines for explicitly requested active-work detail.
    pub(crate) fn work_detail_lines(&self) -> Vec<String> {
        let Some(work) = &self.live_turn.work else {
            return Vec::new();
        };
        if !self.work_details {
            return Vec::new();
        }
        work.tools
            .values()
            .take(12)
            .map(|(name, status, started_at)| match status {
                ToolStatus::Running => {
                    if let Some(started) = started_at {
                        format!(
                            "tool {name} · running {}",
                            render_elapsed(started.elapsed())
                        )
                    } else {
                        format!("tool {name} · {}", status.label())
                    }
                }
                _ => format!("tool {name} · {}", status.label()),
            })
            .collect()
    }
}
