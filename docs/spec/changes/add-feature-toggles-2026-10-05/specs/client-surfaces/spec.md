## ADDED Requirements

### Requirement: Feature list and toggle

The TUI SHALL provide `/features`, listing every registry entry with its
on/off state, controlling key, and provenance. `/features <id> on|off` MUST
write only the user configuration file, through the previewed and
rollback-capable user-config edit, after confirmation, and MUST apply the new
value through the existing safe-boundary reconfigure. When a higher-precedence
layer overrides the user value, Smith MUST report that instead of claiming the
change took effect. `smith config features` SHALL print the same listing
without starting a session.

#### Scenario: Turn a feature off

- **GIVEN** `idle-compaction` is on from its built-in default
- **WHEN** the user runs `/features idle-compaction off` and confirms
- **THEN** `~/.smith/config.toml` gains `cache.idle_compaction = false`
- **AND** the change applies at the next safe boundary
- **AND** `/features` shows it off with the user file as its source

#### Scenario: Project layer overrides the user value

- **GIVEN** a project file sets `cache.idle_compaction = true`
- **WHEN** the user turns `idle-compaction` off
- **THEN** Smith reports that the project file still turns it on
- **AND** does not claim the feature is off

#### Scenario: Unknown feature id

- **WHEN** the user runs `/features nope on`
- **THEN** Smith refuses and lists the valid ids
