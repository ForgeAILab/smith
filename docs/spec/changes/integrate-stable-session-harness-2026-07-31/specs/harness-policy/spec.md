## ADDED Requirements

### Requirement: Versioned Smith prompt sections

Smith SHALL author identity, workflow, trust, inspection, tool, verification,
approval, questionnaire, delegation, response-style, activated-skill, memory,
and project-context instructions as independently versioned context fragments.
Dynamic content MUST remain separately budgeted and cacheable.

#### Scenario: Activated skills change
- **GIVEN** static Smith workflow instructions are unchanged
- **WHEN** a new authorized skill activates
- **THEN** only the relevant activation and dynamic prompt fingerprints change
- **AND** the stable instruction prefix remains independently identifiable

### Requirement: Evidence-backed coding workflow

Smith's default workflow SHALL guide the agent through understanding,
inspection, planning for multi-step work, modification, verification, and an
evidence-backed report. It MUST prohibit claims that commands or tests
succeeded unless committed tool results show they ran successfully.

#### Scenario: Verification command fails
- **GIVEN** Smith edited code and the verification command returned an error
- **WHEN** the agent reports the result
- **THEN** it states that verification failed with relevant evidence
- **AND** does not claim the change passed

### Requirement: Progressive trusted skill disclosure

Smith SHALL index bounded skill metadata before bodies and use deterministic
source precedence of built-in, user, trusted workspace, then session override.
Workspace skill bodies MUST NOT enter privileged context without project trust,
provenance, revision, and activation policy.

#### Scenario: Untrusted workspace contains a skill file
- **GIVEN** a repository supplies skill metadata and instructions
- **WHEN** the project is not trusted for that executable content
- **THEN** Smith does not activate the body as privileged instructions
- **AND** file-authored claims cannot grant permissions

### Requirement: Reusable todo state for multi-step work

Smith SHALL compose the generic typed todo component with versioned
checkpointed state and plan-update events. Product guidance SHOULD use it for
multi-step work and SHOULD NOT require it for trivial requests.

#### Scenario: Multi-step edit plan changes
- **GIVEN** the agent has an active multi-step todo list
- **WHEN** verification reveals a new required step
- **THEN** the plan update is checkpointed and rendered inline
- **AND** replay reconstructs the same todo state without a permanent pane

### Requirement: Smith-owned memory policy over generic contributors

Memory contributors SHALL be bounded, versioned, sensitivity-aware harness
components, while Smith owns what is stored, source precedence, retrieval,
retention, and user controls. Memory MUST NOT silently become canonical user
history or tool authority.

#### Scenario: Relevant project memory is retrieved
- **GIVEN** Smith policy permits one bounded memory record
- **WHEN** it contributes to a turn
- **THEN** the context plan identifies its source, revision, sensitivity, and
  token cost
- **AND** removing it does not rewrite canonical conversation history

### Requirement: Recoverable artifacts and semantic summaries

Smith SHALL store oversized originals in a session-private artifact store and
provide bounded previews/references and authorized reads. Model-assisted
summaries MUST retain provenance and originals, use explicit purpose/spend
policy, and be revalidated by Agent Runtime's deterministic planner.

#### Scenario: Large shell output is offloaded
- **GIVEN** shell output exceeds the inline context budget
- **WHEN** Smith stores it as an artifact
- **THEN** the model and transcript receive a bounded preview and reference
- **AND** an authorized paginated read can recover the original

### Requirement: Questionnaire use is bounded by product guidance

Smith SHALL tell the agent to request user input only for a material choice or
missing fact that cannot be inferred safely. Routine reversible implementation
details SHOULD be handled autonomously.

#### Scenario: Routine naming choice is reversible
- **GIVEN** the agent can choose a conventional local variable name safely
- **WHEN** it plans the edit
- **THEN** it proceeds without opening a questionnaire
- **AND** reserves the interaction channel for consequential ambiguity
