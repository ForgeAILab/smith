## ADDED Requirements

### Requirement: Provider discovery before manual model entry

Add-model setup SHALL offer validated provider-discovered model choices before
requesting a manual model ID. It MUST resolve advertised limits before asking
for numeric input, preserve existing explicit values ahead of derived defaults,
and show actual changes through the existing reviewed transaction.

#### Scenario: Select Xiaoge's discovered model

- **GIVEN** Xiaoge publishes `selected-large` with complete limit metadata
- **WHEN** the user runs `smith setup add-model --provider xiaoge`
- **THEN** setup offers `selected-large` and prefills the published limits
- **AND** asks for no context, input-ceiling, or output-ceiling number
- **AND** review identifies the provider listing and its retrieval provenance

#### Scenario: API key is stored in owner-only config

- **GIVEN** an existing custom provider uses an inline user-config API key
- **WHEN** setup probes its listing
- **THEN** the request uses that provider's configured authentication
- **AND** a successful authenticated response prevents manual numeric fallback
- **AND** the key remains absent from review, diagnostics, and model caches

#### Scenario: Existing output differs from a derived fallback

- **GIVEN** an explicit or advertised output ceiling is `256000`
- **AND** the selected context would otherwise derive a smaller request budget
- **WHEN** setup prepares the model review
- **THEN** it preserves the output ceiling and labels the request budget separately
- **AND** it does not propose replacing `256000` with the derived budget

#### Scenario: Re-add an unchanged model

- **GIVEN** the provider/model and detected values already exist unchanged
- **WHEN** the user confirms add-model without changing them
- **THEN** the operation is a no-op for metadata
- **AND** it does not display an existing-values-differ replacement prompt

#### Scenario: Metadata is unavailable

- **GIVEN** no usable cached, endpoint, explicit, or trusted context is available
- **WHEN** bounded detection ends
- **THEN** setup offers manual model/context entry as recovery
- **AND** only missing numeric values are requested

### Requirement: Explicit model synchronization command

Smith SHALL provide `smith models sync [--provider NAME] [--force]` as a bounded
metadata-only operation and show secret-free per-source results, source age,
and whether a last-good snapshot was retained. `--force` MUST bypass freshness
only, retaining validation, authentication, and locking constraints.

#### Scenario: Refresh Xiaoge immediately

- **WHEN** the user runs `smith models sync --provider xiaoge --force`
- **THEN** Smith attempts a bounded authenticated listing refresh for Xiaoge
- **AND** reports success, unchanged, skipped, or failure with retained-cache age
- **AND** sends no inference request and changes no default or model override

#### Scenario: Model details explain cached metadata

- **GIVEN** a model is supplied by an endpoint snapshot
- **WHEN** the user inspects its `/model` details after safe snapshot adoption
- **THEN** details identify endpoint-advertised source, revision, retrieval age,
  and staleness, plus supported limits, capabilities, and any published prices
- **AND** the picker itself performs no network request or credential read
