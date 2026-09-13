## ADDED Requirements

### Requirement: Installed coding agents enumerate without model declarations

The selection inventory SHALL enumerate an installed coding-agent model id
(`cli/<kind>/<model>`) referenced by any profile using Smith's built-in
bookkeeping limits when no explicit, trusted, or catalog limit exists. The
enumerated entry MUST remain provider-qualified exactly as the profile
declares it, its limits MUST carry provenance identifying them as built-in
bookkeeping rather than advertised capability, and an explicit
`[models."provider/cli/<kind>/<model>"]` table MUST override the built-ins.
A profile that resolves MUST NOT be reported as failing to resolve to a
usable provider/model pair merely because it selects an installed agent.

#### Scenario: Profile selects an installed agent with no declaration

- **GIVEN** a profile declares `provider = "google"` and
  `model = "cli/claude-code/sonnet"` with no `[models]` entry for that pair
- **WHEN** Smith builds the selection inventory
- **THEN** the pair is enumerated with built-in bookkeeping limits
- **AND** the profile is selectable, not marked unavailable

#### Scenario: Explicit limits override built-in bookkeeping

- **GIVEN** the same profile and an explicit
  `[models."google/cli/claude-code/sonnet"]` table
- **WHEN** Smith builds the selection inventory
- **THEN** the explicit limits win and retain configured provenance
- **AND** the built-in values are not merged in alongside them

### Requirement: Candidate previews derive per-candidate output budgets

When the selection inventory previews a provider/model candidate other than
the active one, request-output and reserve values whose provenance is scoped
to the active profile MUST NOT be applied to that candidate; the preview
SHALL derive the candidate's automatic budget from the candidate's own
limits. Explicit values from layers that persist across a model switch
(user-global configuration, environment variables, command-line flags, and
session overrides) MUST still apply, and a candidate whose ceiling such a
persistent value exceeds MUST remain non-selectable with a bounded reason.
The effective budget of the active candidate MUST NOT change because other
candidates were previewed.

#### Scenario: Active profile request does not disable smaller-ceiling candidates

- **GIVEN** the active profile configures `max_output_tokens = 32768`
- **AND** another candidate's output ceiling is 32,000
- **WHEN** Smith previews that candidate in the inventory
- **THEN** the candidate is selectable
- **AND** its preview budget is its own automatic value bounded by 32,000

#### Scenario: Persistent explicit request still conflicts honestly

- **GIVEN** a user-global configuration layer sets
  `max_output_tokens = 40000`
- **AND** a candidate's output ceiling is 32,000
- **WHEN** Smith previews that candidate
- **THEN** the candidate is non-selectable with a bounded reason naming the
  request and the ceiling
- **AND** the explicit value is not clamped

#### Scenario: Active candidate budget is unaffected by previews

- **GIVEN** the active model resolves an effective request of 32,768 tokens
- **WHEN** Smith previews other candidates in the same inventory
- **THEN** the active model's effective request remains 32,768 with its
  configured reserve
