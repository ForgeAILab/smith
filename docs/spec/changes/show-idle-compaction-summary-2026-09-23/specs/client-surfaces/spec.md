## ADDED Requirements

### Requirement: Completed idle compaction is visible in the transcript

When automatic idle compaction completes, Smith SHALL append one distinct
bounded local transcript notice that names the compaction, identifies where
the durable summary lives, and states that the next prompt continues from the
summarized context. The notice is local presentation only: it SHALL NOT enter
canonical history or any provider request.

#### Scenario: Idle compaction completes while the user is away

- **GIVEN** idle compaction is enabled and the inactivity limit fires
- **WHEN** the ordinary summary attempt completes
- **THEN** the transcript shows one idle-compaction notice naming the summary
- **AND** the notice does not change canonical user or assistant history

#### Scenario: Compaction fails

- **GIVEN** the idle summary attempt fails or its persistence fails
- **WHEN** Smith records the failure
- **THEN** the transcript shows one bounded failure notice
- **AND** no summary view is offered for that interval

### Requirement: Resuming a compacted session shows the compacted state

Smith SHALL show one bounded on-return presentation when a session resumes
after its last idle interval completed compaction, stating that the context
was compacted during idle and that continuing starts from the summary. The
presentation SHALL appear once per resume and SHALL NOT replay for sessions
without a completed idle compaction.

#### Scenario: User resumes the next day

- **GIVEN** idle compaction completed before the process exited
- **WHEN** the user resumes the session
- **THEN** one on-return presentation states the context was compacted
- **AND** it identifies the summary view command

#### Scenario: Session without compaction

- **GIVEN** no idle compaction ever completed for the session
- **WHEN** the user resumes
- **THEN** no on-return compaction presentation appears

### Requirement: `/summary` renders the durable summary with provenance

Smith SHALL provide an interactive `/summary` command that renders the current
durable semantic summary body from its protected session-owned artifact,
bounded in size, together with its provenance: purpose, provider and model,
revision, source coverage watermark, and creation time. The rendering SHALL be
labeled as non-authoritative presentation and SHALL NOT modify exact state,
schedule work, or enter canonical history. When the body is missing, corrupt,
or incomplete, the command SHALL render a bounded failure without falling back
to untrusted prose.

#### Scenario: View after an idle compaction

- **GIVEN** a completed idle-compaction summary exists in the protected store
- **WHEN** the user runs `/summary`
- **THEN** the summary body is rendered with its provenance
- **AND** the rendering is labeled non-authoritative

#### Scenario: Summary artifact unavailable

- **GIVEN** the summary reference is missing or fails verification
- **WHEN** the user runs `/summary`
- **THEN** a bounded failure notice is rendered
- **AND** no unverified text is displayed

### Requirement: Headless output carries only summary metadata

Headless text mode SHALL write at most one bounded stderr pointer after a
completed turn that included idle compaction, naming the session and artifact
without the body. JSON and stream-JSON results SHALL carry only bounded
redaction-safe metadata such as `idle_compaction_completed` and
`summary_available`; machine output SHALL NOT contain summary text.

#### Scenario: Headless turn after compaction

- **GIVEN** a completed root turn included an idle compaction
- **WHEN** the headless result is emitted
- **THEN** text mode writes one stderr pointer without the summary body
- **AND** JSON output contains only the bounded metadata fields
