## MODIFIED Requirements

### Requirement: Reusable setup commands

Smith SHALL expose `smith setup add-provider` and
`smith setup add-model --provider <name>` as reusable interactive entry points.
Running `smith setup` without an action SHALL present equivalent choices for
GLM quick start, adding a provider, adding a model to an existing provider, and
changing the default profile/model.

#### Scenario: Add provider command

- **GIVEN** Smith already has a usable default configuration
- **WHEN** the user runs `smith setup add-provider`
- **THEN** the flow collects a distinct provider, authentication, and first
  usable model
- **AND** reviews the additive user-config change without starting a session

#### Scenario: Add model command

- **GIVEN** provider `acme` exists
- **WHEN** the user runs `smith setup add-model --provider acme`
- **THEN** the flow skips provider creation and collects a model plus its
  enforceable limit provenance
- **AND** lets the user choose whether to make it the default

#### Scenario: Limits resolve automatically after the model is entered

- **GIVEN** the user has entered the model ID in either flow
- **WHEN** the endpoint's model listing or a same-name trusted catalog entry
  supplies a context window
- **THEN** numeric limit entry is skipped and the resolved values and source
  are shown in review
- **AND** the resolution probe sends no inference request and stays within its
  bounded time and size

#### Scenario: Resolution fails or finds nothing

- **GIVEN** the endpoint is unreachable, lists nothing for the model, and no
  catalog name matches
- **WHEN** the bounded resolution attempt ends
- **THEN** the flow asks only for the total context window and derives the
  input and output ceilings without showing more numeric fields
- **AND** the failed attempt is not an error the user must dismiss
