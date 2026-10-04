## ADDED Requirements

### Requirement: Child state is typed data in the client

The interactive TUI SHALL hold each child agent's lifecycle state as a typed
value, not as a display label, and SHALL derive the label, its tone, whether
the child is live, whether it retires once read, and whether an exact resume
is available from that value. Smith MUST NOT decide a child's state or
resumability by comparing or searching display text.

#### Scenario: Resuming an interrupted child

- **GIVEN** a child was interrupted with an exact checkpoint
- **WHEN** the user runs `/agent resume` for it while idle
- **THEN** Smith opens the resume confirmation
- **AND** this holds whether the child arrived through a live event or was
  restored from the coordinator

#### Scenario: A child without a checkpoint

- **GIVEN** a child was interrupted without an exact checkpoint
- **WHEN** the user runs `/agent resume` for it
- **THEN** Smith reports that the child cannot be resumed exactly

#### Scenario: Wording is unchanged

- **GIVEN** children in each lifecycle state
- **WHEN** the delegated-work panel, the transcript, and `/agents` render
- **THEN** every state reads exactly as it did before this change

### Requirement: The live turn resets in one place

The interactive TUI SHALL keep the state that describes a live turn in one
value and SHALL return it to idle through one reset, used by every turn
boundary and by any rebuild that keeps the application.

#### Scenario: A turn ends

- **GIVEN** a turn with provider progress, a retry notice, and token flow
- **WHEN** the turn completes, fails, or is interrupted
- **THEN** the progress row is gone and the next turn starts from idle state
