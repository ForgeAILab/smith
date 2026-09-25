## ADDED Requirements

### Requirement: Native Anthropic adaptive-effort requests

Smith SHALL preserve a validated neutral effort selection through the
`anthropic-effort` dialect so Agent Runtime's Anthropic Messages adapter is the
single owner of native request serialization. For an effort selection, the
adapter SHALL emit adaptive thinking and the selected output effort, and Smith
MUST NOT add an OpenAI, OpenRouter, Z.AI, or Gemini reasoning extension.

#### Scenario: Selected effort reaches the Anthropic adapter

- **GIVEN** the exact binding uses the Anthropic Messages adapter and trusted
  `anthropic-effort` metadata
- **WHEN** a turn is accepted at `low` effort
- **THEN** Smith passes the typed `low` selection unchanged to the adapter
- **AND** the native request contains `thinking.type = "adaptive"`
- **AND** the native request contains `output_config.effort = "low"`

#### Scenario: Retry and continuation retain Anthropic effort

- **GIVEN** a turn is accepted with a supported Anthropic effort
- **WHEN** its provider request retries or continues after a tool call
- **THEN** every request in that turn uses the identical typed effort snapshot
- **AND** no retry or continuation downgrades it to a provider default

#### Scenario: Upstream route remains unavailable

- **GIVEN** a correctly encoded Anthropic request reaches a proxy route that
  returns `503 Service Unavailable`
- **WHEN** the existing provider retry policy is exhausted
- **THEN** Smith reports the upstream failure through its retry/error contract
- **AND** reasoning metadata does not hide, reclassify, or claim to repair it
