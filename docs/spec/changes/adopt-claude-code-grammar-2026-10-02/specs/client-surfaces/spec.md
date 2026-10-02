## ADDED Requirements

### Requirement: One transcript row per tool call

The interactive TUI SHALL render each tool call as one row that names the
tool by its reviewed label and reviewed argument summary, with the result
nested beneath it. Smith MUST NOT render a second row that restates the same
call, and MUST NOT print argument names in place of protected values.

#### Scenario: A shell call completes

- **GIVEN** the model runs a shell command that prints twenty lines
- **WHEN** the call completes successfully
- **THEN** the transcript shows one call row with a success marker
- **AND** at most four result lines beneath it followed by a count of the
  remaining lines and the expand key

#### Scenario: A user shell shortcut

- **GIVEN** the user submits `!ls`
- **WHEN** the command completes
- **THEN** the transcript shows the command once, marked as the user's, with
  its result nested beneath
- **AND** no change notice appears when no file changed

#### Scenario: Protected arguments

- **GIVEN** a tool call whose arguments are not reviewed for display
- **WHEN** its row renders
- **THEN** the row shows the tool label and reviewed summary only
- **AND** it does not list argument names or a "details unavailable" note

### Requirement: Folded detail has one expand key

The TUI SHALL fold long tool output, approval detail, and long diffs to a
bounded preview, and SHALL expand and fold them with one documented key that
works whether or not a prompt is open.

#### Scenario: Expanding a result

- **GIVEN** a tool row shows a bounded preview and a remaining-line count
- **WHEN** the user presses the expand key
- **THEN** the bounded full detail is shown in place
- **AND** pressing it again folds the detail

### Requirement: Progress is one row outside the transcript

While a turn is active the TUI SHALL show exactly one progress row directly
above the composer, carrying elapsed time, token flow when the provider has
reported it, and the interrupt key. The row MUST NOT be a transcript entry.

#### Scenario: A turn is running

- **GIVEN** a turn has been running for twelve seconds
- **WHEN** the screen is drawn
- **THEN** one progress row shows the elapsed time and the interrupt key
- **AND** appending transcript content does not move or duplicate it

#### Scenario: Token flow is estimated

- **GIVEN** the only token count available is an estimate
- **WHEN** the progress row renders it
- **THEN** the number carries the estimate marker

### Requirement: Streamed and committed text render identically

The TUI SHALL render assistant text through one Markdown renderer while it
streams and after it commits. Committing a block MUST NOT change the layout
of text already on screen. Links SHALL keep a visible target.

#### Scenario: A streamed answer commits

- **GIVEN** an answer containing bold text, a list, and a code fence is
  streaming
- **WHEN** the attempt commits
- **THEN** the rendered lines are the same as immediately before the commit

#### Scenario: A link

- **GIVEN** assistant text contains a Markdown link whose label is not its
  URL
- **WHEN** it renders
- **THEN** both the label and the URL are visible or selectable

### Requirement: Approvals lead with the action

An approval SHALL present, in order: what will run or change, where and under
what deadline, the applicable warning, the question, and the choices. The
identity hash, full permission list, and raw arguments MUST remain available
through the expand key and MUST NOT precede the action. The transcript SHALL
remain scrollable while the prompt is open.

#### Scenario: A shell approval

- **GIVEN** a prepared shell call needs approval
- **WHEN** the prompt renders
- **THEN** the command is the first content line
- **AND** each choice is on its own line with its key
- **AND** the identity hash and raw arguments appear only after expanding

#### Scenario: Reviewing the work behind a prompt

- **GIVEN** an approval is open
- **WHEN** the user presses a transcript scroll key
- **THEN** the transcript scrolls and the prompt stays open and unanswered

### Requirement: Lists show the name first and metadata last

Command menus, the command palette, and resource pickers SHALL render each
row as a name column, then one short description, then state. Argument
grammar, provenance, and limits SHALL appear only for the selected row.
Rows MUST be cut at a word boundary with an ellipsis.

#### Scenario: Command menu

- **GIVEN** the command menu is open
- **WHEN** it renders five rows
- **THEN** descriptions start in the same column
- **AND** only the selected row shows its argument grammar

#### Scenario: Model picker at 44 columns

- **GIVEN** the model picker is open in a 44-column terminal
- **WHEN** it renders
- **THEN** every row shows the model name and its current or unavailable
  state
- **AND** provenance appears only on the selected row's detail line

### Requirement: Informational results are aligned and word-wrapped

`/help`, `/status`, `/context`, and `/diagnostics` SHALL render labels and
values in aligned columns with word wrapping and a hanging indent. A result
MUST NOT break a word across lines, and a result longer than the pane SHALL
open at its beginning.

#### Scenario: Help in a narrow terminal

- **GIVEN** a 44-column terminal
- **WHEN** the user invokes `/help`
- **THEN** no command name or word is split across lines
- **AND** the first visible line is the start of the result

#### Scenario: Diagnostics with unknown values

- **GIVEN** cache state has not been reported
- **WHEN** the user invokes `/diagnostics`
- **THEN** each fact is on its own line under a group heading
- **AND** an unreported value reads as unknown in words

### Requirement: The composer supports line editing

The composer SHALL move the cursor between the lines of a multi-line draft
with Up and Down, reaching history only from the first or last line, and
SHALL support start-of-line, end-of-line, delete-word, delete-to-start, and
delete-to-end keys. A leading `!` SHALL switch the prompt marker and hint to
shell mode.

#### Scenario: Moving within a draft

- **GIVEN** a three-line draft with the cursor on the last line
- **WHEN** the user presses Up
- **THEN** the cursor moves to the second line
- **AND** history is not recalled

#### Scenario: Shell mode

- **GIVEN** an empty composer
- **WHEN** the user types `!`
- **THEN** the prompt marker and the hint row indicate shell mode
