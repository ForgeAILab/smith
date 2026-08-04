## ADDED Requirements

### Requirement: Completed turns persist immediately

Smith SHALL persist a compatible redacted session snapshot after every
completed turn when persistence is enabled. The snapshot MUST retain canonical
history, usage, identity, all ordered manifests, and the latest durable
checkpoint watermark.

#### Scenario: Process exits before explicit shutdown
- **GIVEN** a turn completed and its snapshot save succeeded
- **WHEN** the Smith process exits unexpectedly
- **THEN** resuming retains that turn and all earlier manifests
- **AND** does not depend on orderly session shutdown

### Requirement: Exact mid-turn state uses protected checkpoints

Smith SHALL implement Agent Runtime's checkpoint contract with authenticated
encryption, integrity/version checks, atomic or transactional commits, and a
user-scoped protection key. It MUST NOT silently store exact pending actions,
answers, or tool state as plaintext.

#### Scenario: Protection key is unavailable
- **GIVEN** Smith cannot initialize protected checkpoint storage
- **WHEN** a persistent session starts
- **THEN** Smith reports that mid-turn crash recovery is unavailable
- **AND** does not claim pending approvals or questions will survive restart

### Requirement: Journal and checkpoint have explicit watermarks

The redacted event journal and protected checkpoint SHALL record compatible
watermarks so startup can distinguish durable execution state from later
presentation-only events. Redacted events MUST NOT be expanded into guessed
raw arguments or answers.

#### Scenario: Journal is ahead of checkpoint
- **GIVEN** complete journal records exist after the latest checkpoint
- **WHEN** Smith resumes
- **THEN** it may replay those records for observability according to policy
- **AND** does not repeat provider or tool side effects absent an idempotent
  checkpointed transition

### Requirement: Pending work resumes or reconciles safely

Smith SHALL restore checkpointed approvals and questionnaires exactly once,
resume partially completed tool batches without repeating committed results,
and mark prior ephemeral children or monitors interrupted without restarting
them.

#### Scenario: Restart occurs with a question open
- **GIVEN** one questionnaire request was durably checkpointed
- **WHEN** the interactive TUI resumes the session
- **THEN** it presents the same request identity and accepts one response
- **AND** resumes the same turn without creating a synthetic new user turn
