## ADDED Requirements

### Requirement: Advisor on a harness session

Smith SHALL offer the `advisor` tool through the bridge on a root harness
session whose profile resolves an advisor, and it SHALL behave as it does in
a native session: no arguments, no approval prompt, the answer returned as
the tool result, and usage recorded under the advisor attribution.

The transcript sent to the advisor SHALL be built from what Smith observed
of the conversation: earlier turns, and for the current turn the CLI's text,
the bridged calls and their results, and the tools the CLI ran itself with
their outcomes. A tool the CLI ran itself MUST be labelled as run by the
CLI, not presented as a Smith tool call. The existing input-budget omission
rule applies.

#### Scenario: Mid-turn call from a harness

- **GIVEN** the CLI read a file itself and spawned a child through the
  bridge earlier in the current turn
- **WHEN** it calls `advisor`
- **THEN** the advisor request contains the user's task, the CLI's read
  labelled as run by the CLI, and the bridged spawn with its result

#### Scenario: The CLI's own tools are off

- **GIVEN** a harness running without its own tools
- **WHEN** the CLI calls `advisor`
- **THEN** the transcript contains the conversation and the bridged calls

#### Scenario: No advisor configured

- **GIVEN** a harness session whose profile resolves no advisor
- **WHEN** a turn runs
- **THEN** `advisor` is not bridged and no advisor guidance is delivered
