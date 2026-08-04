## ADDED Requirements

### Requirement: Composer input history and reverse search

The interactive TUI SHALL maintain one bounded, process-local composer history
for accepted input and non-blank drafts cleared by the first `Ctrl+C`. It MUST
support lossless arrow-key navigation and local reverse search without
creating a provider request, canonical conversation entry, or durable session
record until the user explicitly accepts and submits recalled input.

#### Scenario: Accepted input enters bounded history
- **GIVEN** the composer contains a non-blank prompt, command, shell shortcut,
  or child-agent request that passes local validation
- **WHEN** Smith accepts the input for its provider, local action, or
  confirmation flow
- **THEN** Smith records the exact pre-normalization composer text in bounded
  process-local history
- **AND** suppresses an adjacent exact duplicate

#### Scenario: Rejected input remains unrecorded
- **GIVEN** composer input fails local parsing, reference resolution,
  availability, or busy-state validation
- **WHEN** the user attempts to submit it
- **THEN** Smith leaves the draft in the composer
- **AND** does not add a history entry

#### Scenario: First control-C stashes the current draft
- **GIVEN** the composer contains a non-blank unsent draft
- **WHEN** the user presses `Ctrl+C` once
- **THEN** Smith records the exact draft in the shared composer history and
  clears the composer
- **AND** retains the existing one-second second-press exit window

#### Scenario: Arrow keys browse without losing the current draft
- **GIVEN** no overlay owns input and composer history is non-empty
- **WHEN** the user presses `Up` from an empty or non-empty composer
- **THEN** Smith preserves the current text as a scratch draft and recalls the
  newest history entry
- **AND** repeated `Up` and `Down` traverse older and newer entries
- **AND** `Down` after the newest entry restores the scratch draft exactly

#### Scenario: Editing recalled input leaves navigation
- **GIVEN** the composer displays a recalled history entry
- **WHEN** the user edits that text
- **THEN** Smith keeps the edited text in the composer and exits active history
  navigation
- **AND** does not record the edit until it is accepted or stashed

#### Scenario: Control-R searches the same history
- **GIVEN** no other overlay owns input
- **WHEN** the user presses `Ctrl+R` and types a query
- **THEN** Smith shows the newest case-insensitive substring match from the
  shared composer history
- **AND** repeated `Ctrl+R` cycles through older matches with bounded wrapping
- **AND** the search performs no provider or persistence I/O

#### Scenario: Reverse search is accepted or cancelled losslessly
- **GIVEN** reverse search is open with an original composer draft
- **WHEN** the user presses `Enter` on a match
- **THEN** Smith places the exact match in the composer without submitting it
- **AND** when the user presses `Esc` instead, Smith restores the original
  draft exactly

#### Scenario: Existing overlays retain keyboard ownership
- **GIVEN** a picker, approval, questionnaire, or confirmation overlay is open
- **WHEN** the user presses `Up`, `Down`, or `Ctrl+R`
- **THEN** Smith preserves that overlay's documented keyboard behavior
- **AND** does not start composer history navigation or search
