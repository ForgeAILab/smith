## MODIFIED Requirements

### Requirement: Turn-scoped change attribution

Smith SHALL journal a versioned change set for completed turns that perform
authorized mutations. Each change set MUST distinguish exact attributable
patches from observed or ambiguous deltas and MUST retain bounded pre/post
evidence sufficient for safe conflict checks without exposing protected
arguments or credentials. An ambiguous delta MUST NOT withdraw the exact
patches recorded in the same turn.

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
- **AND** the change set marks that delta ineligible for automatic `/undo`
  without withdrawing the exact patches recorded beside it

#### Scenario: Historical journal lacks attribution

- **GIVEN** a resumed session predates change-set records
- **WHEN** the user inspects or attempts to undo a historical turn
- **THEN** Smith keeps the transcript resumable
- **AND** labels that turn non-undoable rather than synthesizing ownership

### Requirement: Safe last-turn undo

`/undo` SHALL target only the newest completed, not-yet-undone Smith turn that
recorded at least one exact attributable patch. Smith MUST show the complete
reverse patch, require explicit confirmation with no default action, and apply
it only when every affected path matches the recorded post-image. When the same
turn also produced an ambiguous delta, `/undo` SHALL reverse the exact patches
only, MUST name the unattributable tools in the preview, and MUST leave every
path it cannot attribute untouched.

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

#### Scenario: Mixed turn undoes Smith's own edits and names the rest

- **GIVEN** the newest turn contains both an exact Smith edit and an ambiguous
  shell or extension delta
- **WHEN** the user invokes `/undo`, reviews the preview, and confirms
- **THEN** Smith reverses the exact edit after its post-image check
- **AND** the preview names the unattributable tools and no path outside the
  exact patches is modified

#### Scenario: Turn with no exact patch has nothing to undo

- **GIVEN** the newest turn changed the workspace only through ambiguous shell
  or extension deltas
- **WHEN** the user invokes `/undo`
- **THEN** Smith refuses without modifying any path
- **AND** points the user to `/diff` and selective `/revert`

#### Scenario: Concurrent write blocks the exact half of a mixed turn

- **GIVEN** a mixed turn whose Smith-edited path no longer matches its recorded
  post-image
- **WHEN** the user invokes `/undo`
- **THEN** Smith refuses without modifying any affected path
- **AND** does not fall back to reversing the ambiguous delta

### Requirement: Safe exact redo

`/redo` SHALL target only the newest successful undo or selective revert whose
recorded forward patch is exact and whose current paths match the expected
pre-image. Smith MUST preview the complete patch, require explicit non-default
confirmation, apply atomically, and journal the result. Redo SHALL cover
exactly what the matching undo reversed, so an undo limited to the exact
patches of a mixed turn stays reversible on the same terms.

#### Scenario: Redo an exact undo
- **GIVEN** Smith successfully undid one fully attributable edit turn and no
  affected path changed afterward
- **WHEN** the user invokes `/redo`, reviews, and confirms
- **THEN** Smith reapplies the exact forward patch atomically
- **AND** records a redo transaction linked to the original turn and undo

#### Scenario: Concurrent edit blocks redo
- **GIVEN** an affected path changed after the undo
- **WHEN** the user attempts `/redo`
- **THEN** Smith refuses without modifying any path
- **AND** points to `/diff` and `/timeline` with a structured conflict

#### Scenario: Redo restores the exact half of a mixed turn
- **GIVEN** Smith undid the exact patches of a turn that also produced an
  ambiguous delta, and no affected path changed afterward
- **WHEN** the user invokes `/redo`, reviews, and confirms
- **THEN** Smith reapplies exactly those patches
- **AND** the ambiguous delta is neither reapplied nor reversed

#### Scenario: Ambiguous shell delta is not redoable
- **GIVEN** a prior recovery record holds no exact patch, only an
  unattributable shell delta
- **WHEN** the user invokes `/redo`
- **THEN** Smith reports that no exact redo candidate exists
