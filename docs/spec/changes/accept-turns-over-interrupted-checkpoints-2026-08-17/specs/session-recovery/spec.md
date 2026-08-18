## MODIFIED Requirements

### Requirement: Pending work resumes or reconciles safely

Smith SHALL restore checkpointed approvals and questionnaires exactly once,
resume partially completed tool batches without repeating committed results,
and mark prior ephemeral children or monitors interrupted without restarting
them. When a prior turn ended without a durable terminal boundary, Smith
SHALL accept newly submitted turns by finalizing the interrupted turn as an
explicit failed terminal without replaying its indeterminate work, rather
than rejecting the submission.

#### Scenario: Restart occurs with a question open
- **GIVEN** one questionnaire request was durably checkpointed
- **WHEN** the interactive TUI resumes the session
- **THEN** it presents the same request identity and accepts one response
- **AND** resumes the same turn without creating a synthetic new user turn

#### Scenario: New turn submitted over an interrupted turn
- **GIVEN** a prior turn ended with its protected checkpoint short of a
  terminal boundary
- **WHEN** the user submits a new turn on that session
- **THEN** the interrupted turn is finalized as a failed terminal without
  replaying its provider or tool work
- **AND** the new turn is accepted and runs normally
- **AND** the finalized chain preserves the checkpoint watermark for replay
