## ADDED Requirements

### Requirement: Fork command and flag

The TUI SHALL provide `/fork`, which forks the current session, switches to the
fork, and reports both session ids. `--fork-session` SHALL combine with
`--resume [<id>]` to start from a fork of that session instead of continuing
it, in the TUI and in `smith -p`, and MUST be rejected without `--resume`. A forked session MUST show which session it was forked from in its
header and in `/resume`. `/fork` MUST refuse, with the reason, on a session
that runs on an installed coding agent.

#### Scenario: User forks the current chat

- **GIVEN** an idle session
- **WHEN** the user runs `/fork`
- **THEN** the TUI continues in a new session with the same transcript
- **AND** reports the new id and the original id
- **AND** the original appears in `/resume`

#### Scenario: Headless fork of a saved session

- **WHEN** the user runs `smith -p --resume <id> --fork-session "try another way"`
- **THEN** the turn runs in a new forked session
- **AND** session `<id>` is unchanged

#### Scenario: Flag without a session to fork

- **WHEN** the user runs `smith --fork-session`
- **THEN** argument parsing fails naming `--resume`
