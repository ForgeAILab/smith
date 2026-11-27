## ADDED Requirements

### Requirement: Catalog key connections write without a summary review

A built-in catalog key connection in a session SHALL write its user-scope
transaction as soon as the user answers the default choice, with no summary
review. This covers `/connect anthropic` and `/connect google`. When the write
would replace an existing user-config value, Smith SHALL still show the
collision preview and SHALL NOT replace anything until the user confirms it.
Custom endpoint, gateway, and OAuth connections keep their reviews.

#### Scenario: Connect Anthropic with only new values

- **GIVEN** a session whose config has no `anthropic` provider
- **WHEN** the user runs `/connect anthropic`, stores an API key, picks a
  model, and chooses Keep current default
- **THEN** Smith writes the provider without showing a review step
- **AND** the outcome appears as a notice in the session

#### Scenario: Connect Anthropic over existing values

- **GIVEN** a connection whose write would change `default_profile`
- **WHEN** the user chooses Make this the default
- **THEN** Smith shows the collision preview before writing
- **AND** nothing is written until the user confirms it

### Requirement: In-session setup confirmation is a numbered choice

A setup review shown inside a session SHALL end with numbered choices to
save or go back, so the screen offers a choice rather than text that only
accepts Enter. Arrow keys move between the choices, digits choose, Enter
confirms the selected choice, and Page Up and Page Down scroll the review.
The choices SHALL stay visible when the review is taller than the pane.
Standalone `smith setup` keeps its existing review controls.

#### Scenario: Confirm a collision in a session

- **GIVEN** `/connect` shows a collision preview
- **WHEN** the review renders above the composer
- **THEN** it ends with `1. Replace these values` and `2. Go back`
- **AND** choosing 2 or pressing Escape returns to the default choice without
  writing

#### Scenario: Empty API-key field explains itself

- **GIVEN** a connection's API-key step is open
- **WHEN** no key has been entered
- **THEN** the field shows a dim placeholder asking the user to paste the key
- **AND** the placeholder is never part of the submitted secret

## MODIFIED Requirements

### Requirement: Minimal native Anthropic connection

Smith SHALL expose Anthropic as a trusted API-key provider in guided setup and
`/connect anthropic`, using the existing reviewed credential methods. The
normal connection and model-selection flow MUST NOT ask for or write model
limits, modalities, capabilities, or reasoning options. It SHALL write the
fixed official endpoint so the catalog binding is visible.

#### Scenario: Connect and select Claude

- **GIVEN** the user has an Anthropic API key and Smith is idle
- **WHEN** they connect Anthropic, accept the recommended
  `claude-opus-5-5`, and make it the default
- **THEN** Smith records provider `anthropic` with kind `anthropic-messages`,
  the official `base_url`, the reviewed credential reference, and a `claude`
  profile selecting that model
- **AND** sends no inference request during connection
- **AND** writes no explicit model metadata

#### Scenario: Connect Claude without changing the default

- **GIVEN** the user has an Anthropic API key and another default profile
- **WHEN** they connect Anthropic and choose Keep current default
- **THEN** Smith records only the `anthropic` provider, its official
  `base_url`, and the reviewed credential reference
- **AND** writes no profile, so the Claude models are chosen later through
  `/model`

#### Scenario: Choose another catalog model

- **GIVEN** the frozen Anthropic catalog lists several valid models
- **WHEN** the user opens the model choice during connection or `/model`
- **THEN** Smith lists those models with catalog provenance and any disabled
  reasons
- **AND** selecting one changes only the profile's model

#### Scenario: Recommended model is unavailable

- **GIVEN** the frozen catalog lacks valid metadata for `claude-opus-5-5`
- **WHEN** the user opens Anthropic setup or `/model`
- **THEN** Smith keeps the model unavailable with a local reason
- **AND** does not ask the user to guess limits to bypass preflight

#### Scenario: Reconnect with a replacement key

- **GIVEN** Anthropic is configured with a selected catalog model
- **WHEN** the user reconnects it with a replacement key
- **THEN** Smith changes only its authentication source
- **AND** preserves profiles, model identity, and unrelated configuration
