## Context

`Overlay` (`smith-tui/src/app/state.rs`) has 17 variants: Shortcuts,
Approval, Questionnaire, Palette, ResourcePicker, HistorySearch, UndoConfirm,
RedoConfirm, RevertConfirm, McpTrustConfirm, SkillTrustConfirm,
ReviewConfirm, AgentConfirm, AgentFollowUpConfirm, AgentResumeConfirm,
RotationConfirm, ExitConfirm. The confirm renderers in `render/modal.rs`
share one shape — a warning line, a body cut at `MAX_BODY_LINES`, and a
`y <action>   n/esc cancel` line inside `draw_modal` — with per-kind titles,
tones, and actions. Approvals and questionnaires queue in `pending_prompts`
and take a 500 ms keystroke guard; the other 15 assignments overwrite the
slot. Notices are `push_notice(source: &str, text)`.

## Goals / Non-Goals

- Goals: one confirmation component; no prompt lost to another; notice kinds
  that cannot drift; feedback that does not pile up in the transcript.
- Non-Goals: changing approval or questionnaire presentation; keymap-driven
  dispatch; changing confirmation wording beyond what scrolling needs.

## Decisions

- Decision: `Overlay::Confirm(ConfirmDialog)` carries title, tone, optional
  warning line, body lines, accept label, and the `Action`s for accept and
  cancel, built where the dialog is opened. `y` accepts; `n` and Esc cancel;
  Enter does nothing; arrows, PageUp, and PageDown scroll the body; the
  500 ms guard applies. The exit confirmation uses the same component.
- Decision: `App::open_overlay` is the only way to set the slot. A prompt
  (approval, questionnaire, confirmation) opens if the slot is empty or holds
  a transient overlay (which closes), and otherwise joins the FIFO queue that
  approvals and questionnaires already use; answering one opens the next. A
  transient overlay opens only when no prompt is open or queued.
- Decision: `NoticeKind` lives in `smith-client` (client-neutral) and fixes
  each kind's label and persistence: `Transcript` or `Feedback`. Labels keep
  today's wording. Feedback shows in the hint row, replacing the hints, until
  the next keypress, and never enters the transcript.
- Decision: a notice is Feedback only if it answers the user's own keypress
  and changes nothing (refused commands, already-current selections, an
  empty or unreadable clipboard, a no-op). Anything the runtime, a provider,
  a child, a monitor, or a recovery reports stays in the transcript.

## Risks / Trade-offs

- A feedback line that disappears on the next key is easier to miss than a
  transcript row. It sits directly under the composer where the key was
  pressed, which is where Claude Code shows the same feedback.
- Queuing a confirmation behind an approval delays it; answering in arrival
  order is the rule approvals already follow.
