## ADDED Requirements

### Requirement: Addressable child continuation UX

Smith SHALL expose durable existing children separately from new child presets
through typed completion, `/agent`, and `/timeline`. The surface MUST show the
stable child ID, bounded role/task label, durability, model, workspace/tool
posture, state, task usage, and resumability. Selecting or inspecting a child
MUST preserve the root composer draft and MUST NOT itself spend provider tokens.

#### Scenario: Target an idle existing child

- **GIVEN** a durable idle child is relevant to the user's next task
- **WHEN** the user selects its `@child-id` reference and confirms the
  follow-up
- **THEN** Smith sends the task to that exact child as a new turn
- **AND** clearly distinguishes it from selecting a preset that would spawn a
  new child

#### Scenario: Inspect an interrupted child

- **GIVEN** a durable child is interrupted and has a compatible checkpoint
- **WHEN** the user opens it through `/agent`
- **THEN** Smith shows an explicit resume action and the renewed provider/tool
  spend boundary
- **AND** does not resume until that action is confirmed

### Requirement: Live, replay, and headless child state are equivalent

Smith SHALL keep the interactive reducer, persisted journal replay, and
versioned headless projection equivalent for durable child identity, lifecycle, resumability,
cumulative limits, and latest bounded outcome. Protected prompts, tool
arguments, answers, and checkpoints MUST NOT be reconstructed into these
presentation surfaces.

#### Scenario: Restart and continue a child in stream JSON mode

- **GIVEN** a headless consumer observed a child complete before process exit
- **WHEN** a resumed run follows up that child
- **THEN** additive machine events retain the same child and session identity
  with an explicit follow-up transition
- **AND** replay produces the same committed state without protected content
