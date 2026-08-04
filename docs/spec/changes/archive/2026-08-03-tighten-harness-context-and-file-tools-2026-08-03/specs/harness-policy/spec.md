## ADDED Requirements

### Requirement: Instruction sections follow registered capabilities

Smith SHALL contribute an instruction section describing a capability only when
that capability is registered for the run. The delegation section, the
questionnaire section, and the todo-planning guidance MUST each be contributed
conditionally, and MUST be absent from the assembled context when the
corresponding tool is not part of the run's tool surface. Sections that remain
unconditional MUST keep their existing identities and revisions so an
unaffected run's cached prefix is unchanged.

Every conditional section MUST be positioned after all unconditional
instruction sections, so that no conditional content falls inside the leading
run of cache-stable segments.

#### Scenario: Child surface receives no questionnaire instructions
- **GIVEN** a run whose surface is a child agent, for which Smith does not
  register the questionnaire tool
- **WHEN** the context is assembled for a provider request
- **THEN** the assembled instructions contain no questionnaire section
- **AND** they contain no instruction to invoke a user-facing question tool

#### Scenario: Read-only profile receives no delegation instructions
- **GIVEN** an active agent profile that does not permit delegation
- **WHEN** the context is assembled for a provider request
- **THEN** the assembled instructions contain no delegation section

#### Scenario: A fully capable run is unchanged
- **GIVEN** a root run that registers the questionnaire, delegation, and todo
  tools
- **WHEN** the context is assembled
- **THEN** every conditional section is present, after the unconditional ones,
  in the authored order
- **AND** each unconditional section carries the same revision it carried
  before this change

#### Scenario: Workflow prose does not name an unregistered tool
- **GIVEN** a run for which the todo tool is not registered
- **WHEN** the context is assembled
- **THEN** the workflow section does not instruct the model to use
  `write_todos`

### Requirement: The stable instruction prefix survives a posture switch

The unconditional instruction sections SHALL be byte-identical across every
run, posture, and turn, and SHALL form an unbroken leading run of cache-stable
segments. No cache-stable segment MAY follow a segment that is not cache-stable
in canonical order, so no variable content can shorten the leading stable run.

#### Scenario: Switching posture mid-session preserves the head
- **GIVEN** a session running under a read-only posture
- **WHEN** the user switches to a build posture and the session resumes
- **THEN** every unconditional instruction section is byte-identical to the
  one sent before the switch
- **AND** the leading run of cache-stable segments is the same length

#### Scenario: A variable section cannot be placed inside the stable run
- **GIVEN** the assembled instruction fragments
- **WHEN** their cache classifications are read in canonical position order
- **THEN** no cache-stable segment follows a segment that is not cache-stable

### Requirement: Todo planning follows the posture

Smith SHALL register the todo-planning tool for postures whose output is the
work itself, and SHALL NOT register it for a read-only posture whose deliverable
is already a plan or a review. When the tool is not registered, no todo state is
projected into the context.

#### Scenario: Plan posture omits the planning tool
- **GIVEN** an agent profile with the plan posture
- **WHEN** the run's tool surface is composed
- **THEN** the todo tool is absent
- **AND** no todo plan fragment is contributed

#### Scenario: Build posture keeps the planning tool
- **GIVEN** an agent profile with the build posture
- **WHEN** the run's tool surface is composed
- **THEN** the todo tool is present

### Requirement: Bounded base harness size

The assembled base harness — the unconditional instruction sections together
with the default tool specifications — SHALL stay within an authored token
ceiling enforced by an automated test. The test MUST report the per-section
contribution when the ceiling is exceeded, and raising the ceiling MUST be an
explicit source change.

#### Scenario: Growth beyond the ceiling fails the build
- **GIVEN** the authored base harness ceiling
- **WHEN** a change increases the unconditional instruction sections or the
  default tool specifications beyond it
- **THEN** the workspace test suite fails
- **AND** the failure names the sections and their individual sizes
