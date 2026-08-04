## ADDED Requirements

### Requirement: Git-aware change inspection

Smith SHALL provide a read-only `/diff` view over the current Git checkout,
including staged, unstaged, and untracked changes. The view MUST support
all-uncommitted, last-Smith-turn, file, and hunk scopes and MUST bound binary
or oversized content without hiding that it exists.

#### Scenario: Inspect all current changes

- **GIVEN** a Git checkout contains staged, unstaged, and untracked changes
- **WHEN** the user invokes `/diff`
- **THEN** Smith shows every category with explicit state labels
- **AND** does not modify the checkout or spend provider tokens

#### Scenario: Diff is unavailable outside Git

- **GIVEN** the project is not inside a Git worktree
- **WHEN** the user invokes `/diff`
- **THEN** Smith explains locally that Git-backed change inspection is
  unavailable
- **AND** does not initialize Git or mutate the project

### Requirement: Read-only change review

Smith SHALL provide `/review` for the last Smith turn, all uncommitted changes,
a selected commit, or a base-branch comparison. Review execution MUST use a
read-only tool/workspace surface and MUST report prioritized findings with
file and line evidence without modifying the checkout.

#### Scenario: Review uncommitted changes

- **GIVEN** the checkout has uncommitted changes
- **WHEN** the user selects the uncommitted scope in `/review`
- **THEN** a read-only reviewer evaluates the bounded diff
- **AND** findings appear in the transcript without mutation authority

#### Scenario: Review spend is explicit

- **GIVEN** review requires a provider turn
- **WHEN** the user selects a review scope
- **THEN** Smith identifies the provider-backed action before dispatch
- **AND** does not begin review until the user confirms

### Requirement: Turn-scoped change attribution

Smith SHALL journal a versioned change set for completed turns that perform
authorized mutations. Each change set MUST distinguish exact attributable
patches from observed or ambiguous deltas and MUST retain bounded pre/post
evidence sufficient for safe conflict checks without exposing protected
arguments or credentials.

#### Scenario: Exact edit is attributable

- **GIVEN** a Smith `edit` invocation succeeds during a turn
- **WHEN** the turn completes
- **THEN** its change set records the exact reversible patch and pre/post-image
  hashes
- **AND** marks that patch eligible for automatic undo

#### Scenario: Ambiguous shell write is not guessed

- **GIVEN** a shell command changes project files but Smith cannot prove exact
  ownership of the complete delta
- **WHEN** the turn completes
- **THEN** the observed change is visible in `/diff`
- **AND** the change set marks the turn ineligible for automatic `/undo`

#### Scenario: Historical journal lacks attribution

- **GIVEN** a resumed session predates change-set records
- **WHEN** the user inspects or attempts to undo a historical turn
- **THEN** Smith keeps the transcript resumable
- **AND** labels that turn non-undoable rather than synthesizing ownership

### Requirement: Safe last-turn undo

`/undo` SHALL target only the newest completed, not-yet-undone Smith turn whose
change set is fully attributable. Smith MUST show the complete reverse patch,
require explicit confirmation with no default action, and apply it only when
every affected path matches the recorded post-image.

#### Scenario: Undo an attributable turn

- **GIVEN** the last completed Smith turn contains only attributable changes
- **AND** every affected path still matches its recorded post-image
- **WHEN** the user invokes `/undo`, reviews the reverse patch, and confirms
- **THEN** Smith applies the reverse atomically
- **AND** journals the undo outcome

#### Scenario: Concurrent edit blocks undo

- **GIVEN** a path changed after Smith recorded the turn post-image
- **WHEN** the user attempts `/undo`
- **THEN** Smith refuses without modifying any affected path
- **AND** points the user to `/diff` and selective `/revert`

#### Scenario: Mixed attributable and ambiguous turn blocks undo

- **GIVEN** the newest turn contains an ambiguous shell or extension delta
- **WHEN** the user invokes `/undo`
- **THEN** Smith previews the known state but refuses automatic reversal
- **AND** does not partially undo the turn

### Requirement: Explicit selective revert

`/revert` SHALL let the user select an exact current file or hunk, preview the
reverse patch, and confirm with no default action. Smith MUST label change
origin as Smith, user, or unknown when evidence permits, MUST fail closed on
stale patches, and MUST NOT implement recovery through broad reset or checkout
operations.

#### Scenario: Revert a selected hunk

- **GIVEN** the current Git diff contains multiple files and hunks
- **WHEN** the user selects one hunk in `/revert`, reviews it, and confirms
- **THEN** Smith reverses only that hunk
- **AND** leaves every unselected change untouched

#### Scenario: Revert a user-authored change explicitly

- **GIVEN** a selected hunk is labelled user-authored or origin unknown
- **WHEN** the user explicitly confirms its reverse patch
- **THEN** Smith may reverse that exact hunk
- **AND** records that the operation was user-selected rather than automatic

#### Scenario: Remove an unchanged untracked file recoverably

- **GIVEN** the user selects an unchanged untracked file for revert
- **WHEN** the user confirms
- **THEN** Smith moves its content into recoverable session storage before
  removing it from the workspace
- **AND** records how to restore it

#### Scenario: Modified untracked file fails closed

- **GIVEN** an untracked file changed after the revert preview
- **WHEN** the user confirms the stale preview
- **THEN** Smith refuses without removing or overwriting the file

### Requirement: Recovery operations are auditable and recoverable

Smith SHALL journal every undo and revert request, selected scope,
confirmation, forward/reverse patch identity, and terminal outcome. A
successful revert MUST remain recoverable during the active session, and
failed recovery MUST leave the workspace unchanged.

#### Scenario: Recover a successful revert

- **GIVEN** a revert completed successfully in the current session
- **WHEN** the user chooses to restore that recovery operation
- **THEN** Smith previews the corresponding forward patch
- **AND** applies it only after the same conflict and confirmation checks

#### Scenario: Atomic recovery failure

- **GIVEN** any selected path fails validation or patch application
- **WHEN** an undo or revert is attempted
- **THEN** Smith rolls back the recovery transaction
- **AND** reports a structured failure without partial workspace mutation
