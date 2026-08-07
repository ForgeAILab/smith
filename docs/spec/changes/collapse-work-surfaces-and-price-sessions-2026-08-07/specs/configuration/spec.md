## MODIFIED Requirements

### Requirement: Catalog model validation and precedence

Smith SHALL normalize only schema-valid catalog metadata into provider-scoped
model records. Explicit Smith model configuration MUST retain field-level
precedence over catalog metadata, and every winning catalog field MUST identify
its catalog revision and retrieval provenance. A catalog entry MAY carry an
optional per-counter price in USD per million tokens; an entry whose price
block is absent, incomplete, or ill-typed MUST remain a valid selectable model
record with no price rather than a rejected or partially-priced one.

#### Scenario: Complete catalog limits are normalized

- **GIVEN** a catalog model publishes positive context and output limits within
  Smith's integer bounds
- **AND** its optional separate input limit is absent
- **WHEN** Smith normalizes the model record
- **THEN** `context_tokens` is the published context limit
- **AND** `max_output_tokens` is the published output limit
- **AND** `max_input_tokens` is the published total context limit
- **AND** runtime context policy still holds back declared output and reasoning
  reserves before admitting input

#### Scenario: Separate input limit is published

- **GIVEN** a catalog model publishes a positive input limit no greater than its
  total context limit
- **WHEN** Smith normalizes the model record
- **THEN** `max_input_tokens` is that separate published input limit
- **AND** no larger input ceiling is inferred

#### Scenario: Invalid catalog limits

- **GIVEN** a catalog entry has a zero or out-of-range limit, output above
  context, or a separate input limit above context
- **WHEN** Smith validates the snapshot
- **THEN** that entry cannot become a selectable model record
- **AND** Smith does not clamp or guess a replacement limit

#### Scenario: A published price is normalized

- **GIVEN** a catalog model publishes finite non-negative input, output, cache
  read, and cache write costs
- **WHEN** Smith normalizes the model record
- **THEN** each cost is retained per counter in USD per million tokens
- **AND** it carries the same catalog revision and retrieval provenance as the
  entry's other fields

#### Scenario: A partial or ill-typed price

- **GIVEN** a catalog model publishes only an input cost, or publishes a
  negative, infinite, or non-numeric cost
- **WHEN** Smith normalizes the model record
- **THEN** the model stays selectable
- **AND** only the individually valid costs are retained, with no value
  inferred for a counter the source did not price

#### Scenario: A price cannot disable a model

- **GIVEN** a catalog model publishes no price at all
- **WHEN** Smith prepares runtime choices
- **THEN** the model is selectable exactly as it is today
- **AND** the absence of a price is not a validation diagnostic

#### Scenario: A snapshot at the prior schema revision

- **GIVEN** a cached catalog snapshot was written at the schema revision before
  prices existed
- **WHEN** Smith loads it
- **THEN** it is rejected as stale by the existing revision check
- **AND** Smith falls back to the embedded seed and schedules a refresh, as it
  does for any stale revision

#### Scenario: Effective reserves leave no input budget

- **GIVEN** a catalog model has internally valid published limits
- **BUT** the effective Smith output and reasoning reserves equal or exceed its
  context window
- **WHEN** Smith prepares runtime choices
- **THEN** the model is visible but disabled with a local reserve diagnostic
- **AND** Smith does not lower the published output ceiling or configured
  reserve to make it selectable

#### Scenario: Explicit limit overrides catalog metadata

- **GIVEN** a catalog record and an explicit
  `[models."<provider>/<model>"]` record both supply a limit field
- **WHEN** Smith resolves the model profile
- **THEN** the explicit field wins through the existing catalog precedence
- **AND** catalog and explicit contributions remain available for provenance
  diagnostics

#### Scenario: Catalog cannot grant provider entitlement

- **GIVEN** a valid catalog model is associated with a configured provider
- **WHEN** Smith adds it to the inventory
- **THEN** Smith labels it as catalog-advertised rather than account-verified
- **AND** does not claim that the credential, subscription, region, or account
  can use that model
