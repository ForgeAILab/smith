## ADDED Requirements

### Requirement: Endpoint-scoped versioned model prices

Smith SHALL cache validated endpoint-advertised per-counter rates with explicit
currency and units and attach their immutable source revision to usage pricing.
Only compatible rates for the exact serving provider/model/account binding may
price that binding. A missing price MUST remain unknown, and a later refresh
MUST NOT reprice usage already recorded under an earlier price reference.

#### Scenario: Xiaoge supplies no prices

- **GIVEN** Xiaoge advertises `selected-large` with limits and capabilities but
  no pricing fields
- **WHEN** Smith synchronizes its metadata and records usage
- **THEN** the model remains usable and token counters remain available
- **AND** pricing remains unknown without borrowing a price from another vendor

#### Scenario: Endpoint publishes partial rates

- **GIVEN** a reviewed schema publishes valid USD input/output rates but no
  cache-read/write rates
- **WHEN** Smith caches and uses the rates
- **THEN** valid counters retain explicit normalized units and provenance
- **AND** missing counters are unpriced and total confidence follows the
  existing exact/estimated/unknown rules

#### Scenario: Explicit zero and malformed prices

- **GIVEN** an endpoint publishes one zero rate and another invalid rate
- **WHEN** Smith validates prices
- **THEN** zero is retained as a known zero price and the invalid rate is unknown
- **AND** invalid pricing does not disable an otherwise valid model

#### Scenario: Same model changes price overnight

- **GIVEN** earlier attempts used revision A's price for model X
- **WHEN** daily refresh supplies revision B's different price for model X
- **THEN** earlier attempts retain A and later attempts use their frozen reference
- **AND** switching, children, exit reporting, and resume preserve attribution
- **AND** price metadata changes no routing, approval, context, or budget decision
