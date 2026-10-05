use super::*;

impl Transcript {
    /// Replaces the transcript with one rebuilt from canonical history.
    ///
    /// Used when resuming a session: history is the source of truth, and any
    /// live-only blocks (notices, in-flight tools) are intentionally dropped.
    pub fn replace_from_history(&mut self, history: &[Message]) {
        self.replace_from_history_with_shell_shortcuts(history, &[]);
    }

    /// Rebuilds history with finished shortcuts at their original boundaries.
    /// Anchors beyond the restored history are discarded, never repositioned.
    pub fn replace_from_history_with_shell_shortcuts(
        &mut self,
        history: &[Message],
        saved: &[RestoredShellShortcut],
    ) {
        self.append_revision = self.append_revision.wrapping_add(1);
        self.blocks.clear();
        self.block_revisions.clear();
        let mut shortcuts = saved
            .iter()
            .filter(|shortcut| shortcut.anchor <= history.len())
            .collect::<Vec<_>>();
        shortcuts.sort_by_key(|shortcut| shortcut.anchor);
        let mut shortcuts = shortcuts.into_iter().peekable();
        for (anchor, message) in history.iter().enumerate() {
            while shortcuts
                .peek()
                .is_some_and(|shortcut| shortcut.anchor == anchor)
            {
                self.push_saved_shell_shortcut(shortcuts.next().expect("anchored shortcut"));
            }
            match message.role {
                Role::User => self.push_user(user_display_text(message)),
                Role::System => {}
                Role::Assistant => {
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text } => {
                                self.push_block(Block::Assistant {
                                    text: text.clone(),
                                    open: false,
                                });
                            }
                            ContentPart::Reasoning { text, redacted, .. } => {
                                self.push_block(Block::Reasoning {
                                    text: text.clone(),
                                    redacted: *redacted,
                                    open: false,
                                });
                            }
                            ContentPart::ToolCall(call) => {
                                let argument_keys = argument_keys(&call.arguments);
                                self.push_block(Block::Tool {
                                    call_id: call.id.as_str().to_owned(),
                                    name: call.name.clone(),
                                    // Canonical history is intentionally not
                                    // projected here. The host supplies a
                                    // credential-redacted display clone after
                                    // rebuilding the transcript.
                                    display: None,
                                    protected_summary: summarize_unavailable_arguments(
                                        &call.name,
                                        &argument_keys,
                                    ),
                                    // History records the call; the matching
                                    // result below supplies the outcome.
                                    user_command: None,
                                    shell_echo: None,
                                    status: ToolStatus::Running,
                                    // As with `display`, the host supplies a
                                    // redacted preview after rebuilding.
                                    result_preview: None,
                                    started_at: None,
                                    // A resumed call has no enrichment to
                                    // recover: the correlation that would
                                    // have produced it is process-local, not
                                    // part of canonical history.
                                    enrichment: Vec::new(),
                                });
                            }
                            // An assistant message does not carry results;
                            // those arrive under the tool role below.
                            ContentPart::Image { .. } | ContentPart::ToolResult(_) => {}
                        }
                    }
                }
                Role::Tool => {
                    for part in &message.content {
                        if let ContentPart::ToolResult(result) = part {
                            let status = if result.is_error {
                                ToolStatus::Failed.with_result_preview(
                                    result
                                        .content
                                        .iter()
                                        .find_map(ContentPart::as_text)
                                        .unwrap_or(""),
                                )
                            } else {
                                ToolStatus::Ok
                            };
                            self.complete_tool_call(result.call_id.as_str(), status);
                        }
                    }
                }
            }
        }
        for shortcut in shortcuts {
            self.push_saved_shell_shortcut(shortcut);
        }
    }

    fn push_saved_shell_shortcut(&mut self, shortcut: &RestoredShellShortcut) {
        let echo = self.next_shell_echo;
        self.next_shell_echo = self.next_shell_echo.wrapping_add(1);
        self.close_open();
        self.push_block(Block::Tool {
            call_id: shortcut.call.clone().unwrap_or_default(),
            name: "shell".to_owned(),
            display: None,
            protected_summary: String::new(),
            user_command: Some(shortcut.command.clone()),
            shell_echo: Some(echo),
            status: if shortcut.is_error {
                ToolStatus::Failed.with_result_preview(shortcut.result.as_deref().unwrap_or(""))
            } else {
                ToolStatus::Ok
            },
            result_preview: shortcut.result.clone(),
            started_at: None,
            enrichment: Vec::new(),
        });
    }
}
/// Text projection of a user message, marking image parts in place so a
/// resumed transcript still shows that an image travelled with the turn.
fn user_display_text(message: &Message) -> String {
    let mut text = String::new();
    for part in &message.content {
        let rendered = match part {
            ContentPart::Text { text } => text.as_str(),
            ContentPart::Image { .. } => "[image]",
            _ => continue,
        };
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(rendered);
    }
    text
}
