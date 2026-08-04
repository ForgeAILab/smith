## ADDED Requirements

### Requirement: Account usage visibility and manual switching

When the active provider has a credential pool, the TUI SHALL show which pool
member is active, offer an accessible picker listing every member with its
usage meter, cooldown state, and provenance-safe display name, and accept an
explicit manual switch to any member not currently in cooldown. A rotation
offered after limit exhaustion SHALL be presented as a modal the user answers,
stating the cache cost of switching, and its outcome SHALL be recorded in the
transcript. Headless runs SHALL select their member once at session start and
keep it for the whole run, projecting the active member and the typed
exhaustion outcome through the versioned machine output without ever prompting
or rotating.

#### Scenario: Inspect pool usage

- **GIVEN** the active provider declares a two-member pool
- **WHEN** the user opens the account picker
- **THEN** both members appear in pool order with usage percentage or unknown,
  cooldown state, and which one is active
- **AND** no credential value or secret fragment is displayed

#### Scenario: Manually switch the active account

- **GIVEN** the picker is open and the second member is eligible
- **WHEN** the user selects it
- **THEN** subsequent attempts use the second member
- **AND** the sticky selection persists for future sessions
- **AND** the transcript records the manual switch

#### Scenario: Rotation is offered as a modal and announced

- **GIVEN** an attempt hits limit exhaustion mid-task with an eligible member
  available
- **WHEN** the runtime offers rotation
- **THEN** a modal names the outgoing and incoming members, the outgoing
  member's reset time, and warns that switching resends the turn without the
  provider-side prompt cache
- **AND** confirming it replays the attempt and writes a rotation notice to the
  transcript
- **AND** declining it writes the exhaustion outcome to the transcript instead

#### Scenario: A headless run never rotates

- **GIVEN** `smith -p` starts with a two-member pool and exhausts its member
  mid-run
- **WHEN** the attempt fails with the typed limit-exhaustion error
- **THEN** the run fails with that error and the earliest reset time
- **AND** no prompt is rendered and no member switch occurs
- **AND** machine output names the member the run started on
