//! Prepared submissions and bounded pending-input ownership.

use agent_runtime_core::ids::TurnId;
use agent_runtime_core::steer::SteerReceipt;

use crate::references::{ComposerReference, parse_references};
use crate::status::Activity;

use super::state::*;

impl App {
    /// Whether process-local user input still awaits runtime commitment or
    /// whole-turn dispatch.
    pub fn has_pending_input(&self) -> bool {
        !self.pending_input.accepted_steers.is_empty()
            || !self.pending_input.rejected_followups.is_empty()
            || !self.pending_input.queued_turns.is_empty()
            || self.pending_input.ready_submission.is_some()
    }

    /// Serving turn identity learned from the typed live event stream.
    pub fn active_turn(&self) -> Option<&TurnId> {
        self.active_turn.as_ref()
    }

    /// Whether automatic goal continuation must remain behind real-user work.
    pub fn should_defer_goal_continuation(&self) -> bool {
        self.has_pending_input()
    }

    /// Whether `Alt+Up` can restore one Smith-owned queued turn.
    pub fn has_editable_queued_turn(&self) -> bool {
        !self.pending_input.queued_turns.is_empty()
    }

    /// Bounded, text-labelled pending-input projections for the anchored pane.
    pub fn pending_input_previews(&self) -> Vec<PendingInputPreview> {
        let sections: [(&'static str, Vec<String>); 3] = [
            (
                "Pending for this turn",
                self.pending_input
                    .accepted_steers
                    .iter()
                    .map(|entry| entry.submission.display_text.clone())
                    .collect(),
            ),
            (
                "First follow-up",
                self.pending_input
                    .rejected_followups
                    .iter()
                    .map(|entry| entry.submission.display_text.clone())
                    .collect(),
            ),
            (
                "Queued turns",
                self.pending_input
                    .queued_turns
                    .iter()
                    .map(|entry| entry.display_text.clone())
                    .collect(),
            ),
        ];
        sections
            .into_iter()
            .filter_map(|(label, entries)| {
                if entries.is_empty() {
                    return None;
                }
                let overflow = entries.len().saturating_sub(MAX_PENDING_PREVIEW_ENTRIES);
                Some(PendingInputPreview {
                    label,
                    entries: entries
                        .into_iter()
                        .take(MAX_PENDING_PREVIEW_ENTRIES)
                        .collect(),
                    overflow,
                })
            })
            .collect()
    }

    /// Records that the runtime accepted one steer. No transcript row is
    /// appended until the matching committed disposition arrives.
    pub fn accept_steer(&mut self, receipt: SteerReceipt, submission: PreparedSubmission) {
        self.pending_input.accepted_steers.push_back(PendingSteer {
            receipt,
            submission,
        });
    }

    /// Preserves a runtime-declared non-steerable input as the first later
    /// whole-turn work.
    pub fn reject_steer_for_followup(
        &mut self,
        turn: Option<TurnId>,
        submission: PreparedSubmission,
    ) {
        self.push_rejected_followup(RejectedFollowup {
            turn,
            interrupt_eligible: false,
            submission,
        });
    }

    /// Restores an unaccepted prepared input exactly and reports the local
    /// failure without spending a provider request.
    pub fn restore_submission(&mut self, submission: PreparedSubmission, error: impl Into<String>) {
        self.restore_prepared_to_composer(submission);
        self.transcript.push_error(error);
    }

    /// Commits one accepted whole-turn submission to visible local history.
    /// Runtime turn acceptance is the boundary; merely pressing a key is not.
    pub fn whole_turn_dispatched(&mut self, turn: TurnId, submission: &PreparedSubmission) {
        self.transcript.push_user(submission.display_text());
        self.active_turn = Some(turn);
        self.status.activity = Activity::Working;
        self.follow_newest();
    }

    /// Takes the one future turn made eligible by the newest terminal event.
    pub fn take_ready_submission(&mut self) -> Option<PreparedSubmission> {
        self.pending_input.ready_submission.take()
    }

    pub(super) fn push_rejected_followup(&mut self, entry: RejectedFollowup) {
        if self.pending_input.rejected_followups.len() < MAX_REJECTED_FOLLOWUPS {
            self.pending_input.rejected_followups.push_back(entry);
            return;
        }
        // Preserve all text while bounding entry count. Rejected follow-ups are
        // dispatched as one FIFO batch anyway, so folding only the tail does
        // not change execution order.
        if let Some(tail) = self.pending_input.rejected_followups.pop_back() {
            let RejectedFollowup {
                turn: tail_turn,
                interrupt_eligible: tail_interrupt_eligible,
                submission: tail_submission,
            } = tail;
            let RejectedFollowup {
                turn,
                interrupt_eligible,
                submission,
            } = entry;
            let merged = PreparedSubmission::merge_fifo([tail_submission, submission])
                .expect("two rejected submissions merge");
            self.pending_input
                .rejected_followups
                .push_back(RejectedFollowup {
                    turn: turn.or(tail_turn),
                    interrupt_eligible: tail_interrupt_eligible || interrupt_eligible,
                    submission: merged,
                });
        }
    }

    pub(super) fn restore_prepared_to_composer(&mut self, submission: PreparedSubmission) {
        for paste in submission.pastes {
            if !self
                .pasted_chunks
                .iter()
                .any(|known| known.placeholder == paste.placeholder)
            {
                if self.pasted_chunks.len() == MAX_PASTED_CHUNKS {
                    self.pasted_chunks.remove(0);
                }
                self.pasted_chunks.push(paste);
            }
        }
        for image in submission.images {
            if !self
                .image_attachments
                .iter()
                .any(|known| known.placeholder == image.placeholder)
            {
                if self.image_attachments.len() == MAX_IMAGE_ATTACHMENTS {
                    self.image_attachments.remove(0);
                }
                self.image_attachments.push(image);
            }
        }
        let current = self.composer.text().to_owned();
        let restored = if current.trim().is_empty() {
            submission.display_text
        } else {
            format!("{}\n\n{current}", submission.display_text)
        };
        self.composer.replace(restored);
    }

    /// Folds one bracketed paste into whichever surface currently takes text.
    ///
    /// Large pastes into the composer collapse to an editable
    /// `[Pasted text #N +L lines]` placeholder; single-line surfaces receive
    /// the paste flattened onto one line.
    pub fn on_paste(&mut self, pasted: &str) {
        let normalized = pasted.replace("\r\n", "\n").replace('\r', "\n");
        if normalized.is_empty() {
            return;
        }
        match &mut self.overlay {
            None | Some(Overlay::Palette { .. }) => {
                let lines = normalized.lines().count();
                if lines >= PASTE_CHUNK_MIN_LINES
                    || normalized.chars().count() > PASTE_CHUNK_MIN_CHARS
                {
                    self.paste_counter += 1;
                    let placeholder = format!(
                        "[Pasted text #{} +{lines} line{}]",
                        self.paste_counter,
                        if lines == 1 { "" } else { "s" }
                    );
                    if self.pasted_chunks.len() == MAX_PASTED_CHUNKS {
                        self.pasted_chunks.remove(0);
                    }
                    self.pasted_chunks.push(PastedChunk {
                        placeholder: placeholder.clone(),
                        content: normalized,
                    });
                    self.composer.insert_str(&placeholder);
                } else {
                    self.composer.insert_str(&normalized);
                }
                if let Some(Overlay::Palette {
                    selected, error, ..
                }) = &mut self.overlay
                {
                    *selected = 0;
                    *error = None;
                }
            }
            Some(Overlay::Questionnaire { state }) => {
                state.paste(&normalized);
            }
            Some(Overlay::HistorySearch { query, .. }) => {
                query.push_str(&flatten_paste(&normalized));
                self.refresh_history_search(false);
            }
            Some(Overlay::ResourcePicker { picker, .. }) => {
                picker.paste(&flatten_paste(&normalized));
            }
            // Confirmation modals take no text.
            Some(_) => {}
        }
    }

    /// Replaces registered paste placeholders with their stored content.
    ///
    /// A single left-to-right pass over `text`: content brought in by one
    /// placeholder is never rescanned, so pasted text that happens to look
    /// like a placeholder cannot expand twice.
    pub fn expand_pasted(&self, text: &str) -> String {
        const MARKER: &str = "[Pasted text #";
        let mut expanded = String::with_capacity(text.len());
        let mut rest = text;
        'scan: while let Some(start) = rest.find(MARKER) {
            for chunk in &self.pasted_chunks {
                if rest[start..].starts_with(&chunk.placeholder) {
                    expanded.push_str(&rest[..start]);
                    expanded.push_str(&chunk.content);
                    rest = &rest[start + chunk.placeholder.len()..];
                    continue 'scan;
                }
            }
            let through_marker = start + MARKER.len();
            expanded.push_str(&rest[..through_marker]);
            rest = &rest[through_marker..];
        }
        expanded.push_str(rest);
        expanded
    }

    /// Registered placeholder labels, for renderer highlighting.
    pub(crate) fn attachment_placeholders(&self) -> impl Iterator<Item = &str> {
        self.pasted_chunks
            .iter()
            .map(|chunk| chunk.placeholder.as_str())
            .chain(
                self.image_attachments
                    .iter()
                    .map(|attachment| attachment.placeholder.as_str()),
            )
    }

    /// Whether the composer surface currently accepts an image attachment.
    pub fn can_attach_image(&self) -> bool {
        matches!(self.overlay, None | Some(Overlay::Palette { .. }))
    }

    /// Registers a clipboard image and inserts its composer placeholder.
    ///
    /// The host reads and encodes the clipboard — [`App`] stays free of I/O —
    /// and hands over a finished PNG data URI.
    pub fn attach_image(&mut self, data_uri: String, width: u32, height: u32) {
        self.image_counter += 1;
        let placeholder = format!("[Image #{} {width}×{height}]", self.image_counter);
        if self.image_attachments.len() == MAX_IMAGE_ATTACHMENTS {
            self.image_attachments.remove(0);
        }
        self.image_attachments.push(ImageAttachment {
            placeholder: placeholder.clone(),
            data_uri,
        });
        self.composer.insert_str(&placeholder);
    }

    pub(super) fn prepare_ordinary_submission(
        &self,
        text: &str,
    ) -> Result<Option<PreparedSubmission>, String> {
        let (display_text, parsed_text, files) = if let Some(literal) = text.strip_prefix("//") {
            let literal = format!("/{literal}");
            (literal.clone(), literal, Vec::new())
        } else if let Some(literal) = text.strip_prefix("!!") {
            let literal = format!("!{literal}");
            (literal.clone(), literal, Vec::new())
        } else if text.starts_with('/') || text.starts_with('!') {
            return Ok(None);
        } else {
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
            let parsed = parse_references(text, &files, &agents)?;
            if parsed
                .references
                .iter()
                .any(|reference| matches!(reference, ComposerReference::Agent(_)))
            {
                return Ok(None);
            }
            let attached_files = parsed
                .references
                .iter()
                .filter_map(|reference| match reference {
                    ComposerReference::File(path) => Some(path.clone()),
                    ComposerReference::Agent(_) => None,
                })
                .collect::<Vec<_>>();
            (parsed.text.clone(), parsed.text, attached_files)
        };

        let expanded_text = self.expand_pasted(&parsed_text);
        let mut images = self
            .image_attachments
            .iter()
            .filter_map(|attachment| {
                expanded_text
                    .find(&attachment.placeholder)
                    .map(|position| (position, attachment.clone()))
            })
            .collect::<Vec<_>>();
        images.sort_by_key(|(position, _)| *position);
        let images = images
            .into_iter()
            .map(|(_, attachment)| attachment)
            .collect();
        let pastes = self
            .pasted_chunks
            .iter()
            .filter(|chunk| display_text.contains(&chunk.placeholder))
            .cloned()
            .collect();
        Ok(Some(PreparedSubmission {
            display_text,
            expanded_text,
            files,
            images,
            pastes,
        }))
    }

    pub(super) fn queue_current_ordinary_submission(&mut self) {
        let text = self.composer.text().trim().to_owned();
        let submission = match self.prepare_ordinary_submission(&text) {
            Ok(Some(submission)) => submission,
            Ok(None) => {
                self.transcript.push_error(
                    "only an ordinary user prompt can be queued; commands, shell actions, and child actions keep their explicit paths",
                );
                return;
            }
            Err(error) => {
                self.transcript.push_error(error);
                return;
            }
        };
        if self.pending_input.queued_turns.len() == MAX_EXPLICIT_QUEUED_TURNS {
            self.transcript.push_error(format!(
                "the explicit turn queue is full ({MAX_EXPLICIT_QUEUED_TURNS} entries)"
            ));
            return;
        }
        self.composer.record_current();
        self.composer.clear();
        self.pending_input.queued_turns.push_back(submission);
        self.follow_newest();
    }

    pub(super) fn edit_newest_queued_submission(&mut self) {
        let Some(submission) = self.pending_input.queued_turns.pop_back() else {
            return;
        };
        self.restore_prepared_to_composer(submission);
    }
}
