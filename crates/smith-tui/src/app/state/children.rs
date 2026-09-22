//! Child-session presentation and inspection state.

use super::*;
use crate::status::SessionUsage;
use crate::transcript::{Block, safe_tool_name};
use agent_runtime_core::usage::UsageDelta;
use smith_runtime::client::SmithEventKind as RuntimeEvent;

impl App {
    /// The ticking or frozen elapsed time for one delegated child.
    ///
    /// Absent for children known only through durable recovery: their live
    /// runtime was in another process, so no honest wall-clock exists.
    pub fn child_elapsed(&self, child: &str) -> Option<Duration> {
        self.child_clocks.get(child).map(ChildClock::elapsed)
    }

    /// Recomputes a child's panel detail from its own most recent tool row.
    ///
    /// The reviewed display for that row resolves moments after the
    /// triggering event is folded — the host answers separately, from
    /// canonical history — so this is called both right after folding a
    /// tool event (to show an honest fallback immediately) and again once
    /// [`Self::set_child_tool_display`] resolves the projection, which
    /// overwrites the fallback before the next redraw. Reading the block's
    /// own `status` rather than trusting the triggering event's kind means a
    /// late-resolving completion still reports the right outcome.
    pub(super) fn refresh_child_tool_detail(&mut self, child: &str) {
        let Some(conversation) = self.child_conversations.get(child) else {
            return;
        };
        let Some(Block::Tool {
            name,
            display,
            protected_summary,
            status,
            ..
        }) = conversation.transcript.blocks().last()
        else {
            return;
        };
        // Matches the transcript's own unknown-tool fallback exactly: the
        // tool is named, and `protected_summary` — already computed from
        // the call's real argument keys, never its values — follows it.
        let label = display.as_ref().map_or_else(
            || format!("{}({protected_summary})", safe_tool_name(name)),
            ToolCallDisplay::invocation,
        );
        let detail = match status {
            ToolStatus::Running | ToolStatus::Unreported => label,
            ToolStatus::Ok => format!("ok {label}"),
            ToolStatus::Failed | ToolStatus::Denied => format!("failed {label}"),
        };
        if let Some(summary) = self.children.get_mut(child) {
            summary.detail = Some(detail);
        }
    }

    /// Replaces the coordinator's authoritative turn/token counts for every
    /// currently visible child.
    ///
    /// A wholesale replace on the same poll-on-redraw cadence as
    /// [`Self::set_inspected_detail`]: the coordinator is the counting
    /// authority, so this is the only path by which the panel's turn/token
    /// figures move at all.
    pub fn set_child_counts(&mut self, counts: BTreeMap<String, ChildCounts>) {
        self.child_counts = counts;
    }

    /// The coordinator's latest turn/token counts for one child, when the
    /// poll has answered for it.
    pub fn child_counts(&self, child: &str) -> Option<ChildCounts> {
        self.child_counts.get(child).copied()
    }

    /// Starts the linger countdown for a child that just finished cleanly.
    ///
    /// A child under inspection never retires out from under the reader; the
    /// countdown is armed again when they look away.
    pub(in crate::app) fn arm_child_dismissal(&mut self, child: &str) {
        if self.inspected_child.as_deref() == Some(child) {
            return;
        }
        if !self
            .children
            .get(child)
            .is_some_and(ChildSummary::retires_when_read)
        {
            return;
        }
        self.child_dismiss_at
            .insert(child.to_owned(), Instant::now() + COMPLETED_CHILD_LINGER);
    }

    /// Retires the panel rows whose linger window has closed.
    ///
    /// Returns whether anything left the screen, so the host can redraw once
    /// on the change instead of animating an idle panel.
    pub fn expire_child_rows(&mut self) -> bool {
        self.expire_child_rows_at(Instant::now())
    }

    pub(crate) fn expire_child_rows_at(&mut self, now: Instant) -> bool {
        let due: Vec<String> = self
            .child_dismiss_at
            .iter()
            .filter(|(_, at)| now >= **at)
            .map(|(child, _)| child.clone())
            .collect();
        for child in &due {
            self.child_dismiss_at.remove(child);
            self.retired_children.insert(child.clone());
        }
        !due.is_empty()
    }

    /// One child's retained transcript blocks, oldest first.
    pub fn child_blocks(&self, child: &str) -> &[Block] {
        self.child_conversations
            .get(child)
            .map_or(&[] as &[Block], |conversation| {
                conversation.transcript.blocks()
            })
    }

    /// Visible text from one child's newest live provider attempt.
    ///
    /// The same presentation-only speculative state [`Self::speculative_text`]
    /// exposes for the root, so the inspector can show a child mid-sentence
    /// exactly as the root timeline shows itself mid-sentence.
    pub fn child_speculative_text(&self, child: &str) -> Option<&str> {
        self.child_conversations
            .get(child)
            .and_then(|conversation| conversation.speculative.visible_text())
    }

    /// Children in the order the delegated-work panel lists them: live work
    /// first, settled children after, retired rows not at all.
    ///
    /// The panel draws this and [`Self::inspectable_children`] reads it, so a
    /// child can never sort one way and select another — or, worse, be
    /// selectable while invisible.
    pub fn visible_children(&self) -> Vec<(&str, &ChildSummary)> {
        let (live, settled): (Vec<_>, Vec<_>) = self
            .children
            .iter()
            .filter(|(child, _)| !self.retired_children.contains(*child))
            .partition(|(_, summary)| summary.is_live());
        live.into_iter()
            .chain(settled)
            .map(|(child, summary)| (child.as_str(), summary))
            .collect()
    }

    /// Children the inspector can reach with the keyboard, in panel order.
    ///
    /// Background shell tasks are not children and are skipped — they have no
    /// child session, log, or follow-up.
    pub fn inspectable_children(&self) -> Vec<&str> {
        self.visible_children()
            .into_iter()
            .map(|(child, _)| child)
            .collect()
    }

    /// Moves the inspector one row down the panel: the root timeline first,
    /// then each child. Stops at the last child rather than wrapping, so a
    /// held key settles somewhere predictable.
    ///
    /// Returns whether the selection moved.
    pub fn inspect_next_child(&mut self) -> bool {
        let children = self.inspectable_children();
        let next = match &self.inspected_child {
            None => children.first().map(|child| (*child).to_owned()),
            Some(current) => children
                .iter()
                .position(|child| child == current)
                .and_then(|index| children.get(index + 1))
                .map(|child| (*child).to_owned()),
        };
        match next {
            Some(child) => {
                self.inspect_child(child);
                true
            }
            None => false,
        }
    }

    /// Opens the read-only inspector on one child.
    pub fn inspect_child(&mut self, child: impl Into<String>) {
        let child = child.into();
        if self.inspected_child.as_deref() != Some(child.as_str()) {
            // The card belongs to the child it was polled for; carrying it
            // across a selection would report one child's turns under
            // another's name until the next redraw.
            self.inspected_detail = None;
            if let Some(left) = self.inspected_child.take() {
                self.arm_child_dismissal(&left);
            }
        }
        // Reading a row is the opposite of ignoring it: whatever countdown it
        // was under stops here and restarts when the reader moves on.
        self.child_dismiss_at.remove(child.as_str());
        self.inspected_child = Some(child);
        // The two views scroll independently; carrying one's offset into the
        // other would open a log part-way up for no reason the user can see.
        self.follow_newest();
    }

    /// The host's latest coordinator card for the inspected child.
    pub fn inspected_detail(&self) -> Option<&str> {
        self.inspected_detail.as_deref()
    }

    /// Records the host's latest coordinator card for the inspected child.
    ///
    /// Ignored unless it names the child currently on screen: a poll that
    /// answered after the user moved on describes a view that is gone.
    pub fn set_inspected_detail(&mut self, child: &str, detail: Option<String>) {
        if self.inspected_child.as_deref() == Some(child) {
            self.inspected_detail = detail;
        }
    }

    /// Moves the inspector one row up the panel, returning to the root
    /// timeline from the first child.
    ///
    /// Returns whether the selection moved.
    pub fn inspect_previous_child(&mut self) -> bool {
        let Some(current) = self.inspected_child.clone() else {
            return false;
        };
        let children = self.inspectable_children();
        match children
            .iter()
            .position(|child| *child == current)
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| children.get(index))
            .map(|child| (*child).to_owned())
        {
            Some(previous) => self.inspect_child(previous),
            None => {
                self.leave_child_inspection();
            }
        }
        true
    }

    /// Leaves the child inspector for the root timeline.
    ///
    /// Returns whether a child was being inspected.
    pub fn leave_child_inspection(&mut self) -> bool {
        let left = self.inspected_child.take();
        self.inspected_detail = None;
        if let Some(left) = &left {
            self.arm_child_dismissal(left);
            self.follow_newest();
        }
        left.is_some()
    }

    /// Delegated children whose panel clock is still ticking.
    pub fn live_child_count(&self) -> usize {
        self.child_clocks
            .values()
            .filter(|clock| clock.is_live())
            .count()
    }

    /// The child counterpart of [`Self::set_tool_display`].
    ///
    /// A child's events withhold argument values exactly as the root's do, so
    /// its rows need the same host-supplied projection. Enrichment arrives for
    /// a child that already has a row, so this must not resurrect a retired
    /// one: a projection landing just after a child settled is the host
    /// answering an earlier question, not new work.
    pub fn set_child_tool_display(&mut self, child: &str, call_id: &str, display: ToolCallDisplay) {
        if let Some(conversation) = self.child_conversations.get_mut(child) {
            conversation.transcript.set_tool_display(call_id, display);
        }
        // The panel detail mirrors whatever the reviewed projection now
        // says about the child's most recent tool row.
        self.refresh_child_tool_detail(child);
    }

    /// The child counterpart of [`Self::set_tool_result_preview`].
    pub fn set_child_tool_result_preview(
        &mut self,
        child: &str,
        call_id: &str,
        preview: impl AsRef<str>,
    ) {
        if let Some(conversation) = self.child_conversations.get_mut(child) {
            conversation
                .transcript
                .set_tool_result_preview(call_id, preview);
        }
    }

    /// One child's conversation, ready to be written to.
    ///
    /// Everything the client records about a child goes through here, which
    /// makes it the one honest place to say "this child is not finished being
    /// interesting": a retired row comes back, and a pending retirement is
    /// called off.
    pub(in crate::app) fn child_conversation_mut(&mut self, child: &str) -> &mut Conversation {
        self.child_dismiss_at.remove(child);
        self.retired_children.remove(child);
        self.child_conversations
            .entry(child.to_owned())
            .or_default()
    }

    /// Records one child lifecycle milestone, sourced to the phase it names.
    ///
    /// Lifecycle is the parent's knowledge — a child never narrates its own
    /// spawn — so these come from the parent stream even though everything
    /// else in the log comes from the child's.
    pub(in crate::app) fn push_child_notice(
        &mut self,
        child: &str,
        source: &str,
        text: impl Into<String>,
    ) {
        let conversation = self.child_conversation_mut(child);
        conversation.transcript.push_notice(source, text);
        conversation.transcript.retain_newest(MAX_CHILD_BLOCKS);
    }

    /// Settles the child's still-open tool rows when the child itself is done.
    pub(in crate::app) fn settle_child_tool_calls(&mut self, child: &str) {
        if let Some(conversation) = self.child_conversations.get_mut(child) {
            conversation.as_mut().settle(ToolStatus::Unreported);
        }
    }

    /// Records the child's answer as the assistant prose it is.
    ///
    /// Only for a child the client never heard from directly: a live child
    /// streamed this answer into its own transcript already, and repeating the
    /// parent's copy beneath it would show the same reply twice.
    pub(in crate::app) fn push_child_answer(&mut self, child: &str, text: &str) {
        if self
            .child_conversations
            .get(child)
            .is_some_and(|conversation| conversation.live)
        {
            return;
        }
        let bounded: String = if text.chars().count() > MAX_CHILD_ANSWER_CHARS {
            text.chars()
                .take(MAX_CHILD_ANSWER_CHARS)
                .chain(std::iter::once('…'))
                .collect()
        } else {
            text.to_owned()
        };
        let conversation = self.child_conversation_mut(child);
        conversation.transcript.push_text_delta(&bounded);
        conversation.transcript.close_open();
        conversation.transcript.retain_newest(MAX_CHILD_BLOCKS);
    }

    /// Records a failure the child reported.
    pub(in crate::app) fn push_child_error(&mut self, child: &str, message: impl Into<String>) {
        let conversation = self.child_conversation_mut(child);
        conversation.transcript.push_error(message);
        conversation.transcript.retain_newest(MAX_CHILD_BLOCKS);
    }

    /// Folds one event from a child's own stream into that child's
    /// conversation.
    ///
    /// This is the same fold the root session gets, against a different
    /// transcript. Nothing here touches session status, the plan, or the turn
    /// clock: a child working is not the user's session working.
    pub fn apply_child(&mut self, child: &str, envelope: &EventEnvelope) {
        let conversation = self.child_conversation_mut(child);
        conversation.live = true;
        let handled = conversation.as_mut().apply(&envelope.payload);
        conversation.transcript.retain_newest(MAX_CHILD_BLOCKS);

        // The panel row answers "what is that agent doing right now" in one
        // line, using the same reviewed tool display projection the
        // transcript uses; see `refresh_child_tool_detail`.
        match &envelope.payload {
            RuntimeEvent::ToolCallRequested { .. } | RuntimeEvent::ToolCallCompleted { .. } => {
                self.refresh_child_tool_detail(child);
            }
            // Delegated usage is accounted separately from the root's own
            // counters — see `usage-accounting`'s "Delegated usage is
            // accounted separately" — and only for what this live stream
            // actually reported, so a dormant recovered child (no stream,
            // no call here) contributes nothing.
            RuntimeEvent::Usage { record } => {
                if let Some(purpose) = record.provenance.attempt_purpose
                    && purpose.is_synthetic_cache()
                {
                    self.status.record_synthetic_usage(purpose, &record.delta);
                } else {
                    self.record_delegated_usage(child, &record.delta);
                }
            }
            _ => {}
        }

        if handled && self.inspected_child.as_deref() == Some(child) {
            // The reader is looking at this child; a new block below the fold
            // is why they are looking.
            self.follow_newest();
        }
    }

    /// Folds one child's own provider-usage record into the delegated
    /// totals, counting the reporting child as a contributor exactly once.
    ///
    /// Mirrors [`Status::record_usage`](crate::status::Status::record_usage)'s
    /// own rule that an input-free record is not usable evidence: an
    /// output-only record says nothing about context consumption, so — like
    /// the root path — it contributes no counters and does not mark the
    /// child a contributor on its own.
    fn record_delegated_usage(&mut self, child: &str, delta: &UsageDelta) {
        if delta.input_tokens() == 0 {
            return;
        }
        self.delegated_contributors.insert(child.to_owned());
        for kind in [
            CounterKind::InputUncached,
            CounterKind::InputCached,
            CounterKind::CacheWrite,
            CounterKind::Output,
            CounterKind::Reasoning,
        ] {
            let value = delta.get(kind);
            if value > 0 {
                *self.delegated_usage.entry(kind).or_insert(0) += value;
            }
        }
    }

    /// This session's whole usage: the root's own counters, plus whatever
    /// delegated children reported on their own streams this process
    /// observed, kept distinguishable per `usage-accounting`'s "Delegated
    /// usage is accounted separately" rather than blended into the root
    /// figures.
    pub fn session_usage(&self) -> SessionUsage {
        let mut usage = self.status.session_usage();
        usage.delegated_totals = self.delegated_usage.clone();
        usage.delegated_contributors =
            u32::try_from(self.delegated_contributors.len()).unwrap_or(u32::MAX);
        usage
    }

    /// The profile a lifecycle transition that replaces a child's whole
    /// summary should carry forward, so only `ChildSpawned` — the one event
    /// that resolves it — ever changes it.
    pub(in crate::app) fn carried_child_profile(&self, child: &str) -> Option<String> {
        self.children
            .get(child)
            .and_then(|summary| summary.profile.clone())
    }

    /// Notes a live root tool call as a pending delegation spawn awaiting
    /// its child's identity, when the call's resolved display says it is
    /// one.
    ///
    /// `RuntimeEvent::ChildSpawned` carries no originating tool-call id, and
    /// Smith's runtime is never configured to emit raw tool arguments on the
    /// event stream, so the one place a live spawn call's action is actually
    /// known is the reviewed projection the host resolves from canonical,
    /// credential-redacted history. Queuing keys strictly on that resolved
    /// `label`/`target` — the same controlled vocabulary
    /// `is_redundant_tool_row` reads — never on the tool name's shape or a
    /// parsed free-text scan.
    ///
    /// This must only ever be called for the root's own event stream. It is
    /// called from exactly one place: `tui_driver::run_tui`'s root-events
    /// branch, right after the host resolves a live `ToolCallRequested`'s
    /// display — a branch of the host loop's `tokio::select!` that is
    /// structurally distinct from the child-events branch, which folds a
    /// child's own `agent`-shaped calls through
    /// [`Self::set_child_tool_display`] instead and never reaches this
    /// method. A child could not reach its own queue even if it tried:
    /// delegation forbids a child from spawning a grandchild in the first
    /// place.
    pub fn note_pending_spawn(&mut self, call_id: &str, display: &ToolCallDisplay) {
        if display.label() != "Agent" || display.target() != "spawn" {
            return;
        }
        let profile = display
            .qualifiers()
            .iter()
            .find_map(|qualifier| qualifier.strip_prefix("profile ").map(str::to_owned));
        self.pending_spawns.push_back(PendingSpawn {
            call_id: call_id.to_owned(),
            profile,
        });
    }
}
