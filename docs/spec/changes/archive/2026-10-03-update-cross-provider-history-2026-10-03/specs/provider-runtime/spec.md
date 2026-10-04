## MODIFIED Requirements

### Requirement: Safe provider and model switching

Smith SHALL change provider or model only between turns. It MUST retain the
canonical session, create a new cache identity, warn that the old remote cache
does not transfer, and reconstruct an immutable shared runtime when in-place
reconfiguration is unavailable. The session MUST remain usable on the new
provider and model whatever reasoning earlier turns produced: reasoning a
different provider or model produced SHALL NOT be sent to the new one, and
SHALL be sent again if the session returns to its producer.

#### Scenario: Switch provider during a session

- **GIVEN** a completed session turn used provider A
- **WHEN** the user confirms a switch to provider B
- **THEN** Smith saves and resumes the same session through a runtime configured
  for provider B
- **AND** cache state becomes unknown or unsupported for the new provider
- **AND** context counts remain labelled by their actual provenance

#### Scenario: Continue after a provider that signs its reasoning

- **GIVEN** a turn on a provider that returns signed or redacted reasoning
  (Gemini, Anthropic, xAI)
- **WHEN** the session continues on a different provider or model
- **THEN** the next turn succeeds
- **AND** the request to the new provider carries none of the earlier
  provider's reasoning
