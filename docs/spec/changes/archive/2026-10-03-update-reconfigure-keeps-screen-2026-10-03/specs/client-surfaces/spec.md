## ADDED Requirements

### Requirement: Changing the session's configuration keeps the screen

The interactive TUI SHALL keep its screen and application when a model,
profile, effort, thinking, or context-window change, an MCP recomposition,
or a provider connection rebuilds the host for the same session: the
transcript as shown, folded and expanded state, scroll position, the
composer, and composer history. It SHALL re-derive status, resources,
children, and usage from the new host, return the live turn to idle, and
append one notice that names what changed. It MUST NOT rebuild the
transcript from history for the same session, and MUST NOT leave the
alternate screen except while another flow draws its own screen.

#### Scenario: Switching model

- **GIVEN** a session with a finished turn, a `/status` result on screen,
  and an earlier prompt in composer history
- **WHEN** the user picks another model with `/model`
- **THEN** the earlier turn and the `/status` result are still in the
  transcript, followed by one notice naming the old and new model
- **AND** Up in the empty composer recalls the earlier prompt
- **AND** the footer names the new model

#### Scenario: Cycling profiles with Tab

- **GIVEN** an idle session with an empty draft and several main profiles
- **WHEN** the user presses Tab
- **THEN** the transcript and scroll position are unchanged apart from one
  profile notice

#### Scenario: Switching to another session

- **GIVEN** a session with composer history
- **WHEN** the user resumes a different session with `/resume`
- **THEN** the transcript shows that session's history
- **AND** composer history is kept

#### Scenario: Connecting a provider

- **GIVEN** an idle session with a transcript
- **WHEN** the user connects a provider through `/connect` and finishes or
  cancels its flow
- **THEN** Smith returns to the same transcript and composer

#### Scenario: A rebuild while a turn runs

- **GIVEN** a turn is running
- **WHEN** a session reconfiguration is requested
- **THEN** Smith refuses it with a notice that it requires an idle turn
- **AND** the running turn continues
