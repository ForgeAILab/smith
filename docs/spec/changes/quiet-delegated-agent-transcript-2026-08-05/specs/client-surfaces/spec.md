## MODIFIED Requirements

### Requirement: Single-focus conversational interaction

The interactive TUI SHALL keep the composer as its only persistent focus
target. Transcript navigation SHALL work through global shortcuts, background
activity SHALL render inline, and absent or hidden regions MUST NOT participate
in focus order. A temporary read-only view MAY borrow the transcript region,
but MUST be dismissible with one key and MUST NOT take focus.

#### Scenario: Tab does not leave the composer

- **GIVEN** no modal or command menu is open
- **WHEN** the user presses `Tab` or `Shift+Tab`
- **THEN** Smith does not move focus to the transcript, inbox, or another
  persistent region
- **AND** the composer remains ready for input

#### Scenario: Transcript scroll is global

- **GIVEN** the composer is active and the transcript has older content
- **WHEN** the user presses a transcript scroll shortcut
- **THEN** the transcript scrolls without entering a separate transcript mode
- **AND** sending a prompt restores follow-newest behavior

#### Scenario: Background activity remains visible

- **GIVEN** a child or monitor emits progress while the user is composing
- **WHEN** Smith renders the event
- **THEN** the event reaches its surface without stealing focus — a monitor as
  a concise attributed transcript notice, a child as panel activity and an
  inspectable log entry
- **AND** detailed child state remains available through `/agent` and panel
  selection

#### Scenario: A read-only child view borrows the transcript region

- **GIVEN** a delegated child is selected from the panel
- **WHEN** Smith renders its read-only view over the transcript region
- **THEN** the composer keeps focus and its draft
- **AND** one dismissal key restores the root timeline unchanged
- **AND** the view participates in no focus order
