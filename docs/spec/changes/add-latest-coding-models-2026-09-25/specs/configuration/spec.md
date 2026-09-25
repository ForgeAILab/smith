## MODIFIED Requirements

### Requirement: Named context windows

Smith SHALL ship reviewed direct-ChatGPT metadata for every GPT-6 model
advertised by the installed Codex catalog. GPT-6 Astra, GPT-6 Sol, and GPT-6
Luna SHALL each use the published 272,000-token default window and expose the
published 872,000-token extended window without requiring a hand-written
`[models]` block.

#### Scenario: New GPT-6 variants resolve on ChatGPT

- **GIVEN** the direct ChatGPT provider is configured without explicit model
  limits
- **WHEN** Smith resolves `chatgpt/gpt-6-sol` or `chatgpt/gpt-6-luna`
- **THEN** the model resolves with the `272k` window by default
- **AND** the `872k` window is selectable
- **AND** the model appears in Smith's model inventory

### Requirement: Embedded provider catalog fallback

Smith SHALL bundle a normalized Models.dev snapshot recent enough to include
the coding models supported at release time, including GPT-6 Sol, GPT-6 Luna,
and Claude Opus 5.5. A valid last-good user cache MAY supersede that snapshot.

#### Scenario: Offline startup sees the release catalog

- **GIVEN** Smith 0.2.14 starts without a user catalog cache and cannot refresh
  Models.dev
- **WHEN** it loads the embedded catalog
- **THEN** GPT-6 Sol, GPT-6 Luna, and Claude Opus 5.5 metadata is available to
  compatible endpoint-bound providers and exact model setup review
