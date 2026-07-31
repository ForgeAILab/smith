## ADDED Requirements

### Requirement: Single-focus conversational interaction

The interactive TUI SHALL keep the composer as its only persistent focus
target. Transcript navigation SHALL work through global shortcuts, background
activity SHALL render inline, and absent or hidden regions MUST NOT participate
in focus order.

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
- **THEN** a concise attributed notice appears inline without stealing focus
- **AND** detailed child state remains available through `/agent`

### Requirement: Unified command discovery

Smith SHALL expose one typed command registry shared by slash completion,
`/help`, and `Ctrl+P`. Typing `/` at the start of a composer draft MUST open a
filterable menu; `Tab` MUST complete without executing, and `Enter` MUST execute
the selected command locally when permitted.

#### Scenario: Slash opens filtered completion

- **GIVEN** the composer is empty
- **WHEN** the user types `/rev`
- **THEN** the command menu filters to matching registered commands with
  descriptions and argument hints
- **AND** no provider request is issued

#### Scenario: Tab completes without execution

- **GIVEN** a command-menu result is selected
- **WHEN** the user presses `Tab`
- **THEN** Smith completes the command text in the composer
- **AND** does not execute a host action or send a provider request

#### Scenario: Ctrl-P shares the registry

- **GIVEN** the user opens command discovery with `Ctrl+P`
- **WHEN** the menu appears
- **THEN** it uses the same commands, descriptions, parser, and host actions as
  slash completion

#### Scenario: Busy command fails locally

- **GIVEN** a host command requires an idle runtime
- **WHEN** the user invokes it during an active model turn
- **THEN** Smith keeps the draft and reports the idle requirement locally
- **AND** does not queue, execute, or send the command to the provider

### Requirement: Focused built-in command set

Smith SHALL initially register only commands backed by implemented product
capabilities: help, status, session/runtime selection, child inspection,
change inspection/review/recovery, and exit. `/help` MUST group primary and
advanced commands without advertising unavailable capabilities.

#### Scenario: Help exposes the complete implemented set

- **WHEN** the user invokes `/help`
- **THEN** Smith lists `/help`, `/status`, `/new`, `/resume`, `/model`,
  `/provider`, `/profile`, `/agent`, `/diff`, `/review`, `/undo`, `/revert`,
  and `/quit`
- **AND** every listed command has a one-line description

#### Scenario: Status is local and honest

- **WHEN** the user invokes `/status`
- **THEN** Smith reports resolved model/profile, permission mode, context
  provenance, session, child, Git, and change-attribution state
- **AND** unknown or unavailable values are labelled rather than guessed
- **AND** no provider request is issued
