## MODIFIED Requirements

### Requirement: Labelled cost calculation

Smith SHALL calculate cost only from a versioned price reference and compatible
usage counters. Calculated values MUST be labelled exact, estimated, or unknown
according to their inputs. The catalog snapshot's per-model price entry is one
such reference, and it MUST carry the same revision and retrieval provenance as
every other catalog field. Counters MUST be priced by the reference of the
provider/model binding that produced them: root counters by the binding active
when they were reported, delegated counters by the child's own binding.
Counters from one binding MUST NOT be priced at another binding's rates. Cost
MUST remain presentation only: it MUST NOT enter routing, approval, context,
or budget decisions, and MUST NOT reach the model.

#### Scenario: Price is unavailable

- **GIVEN** a custom compatible endpoint reports tokens but has no configured
  price
- **WHEN** Smith renders usage
- **THEN** it shows the token counters
- **AND** reports cost as unknown rather than assuming an OpenAI price

#### Scenario: A priced model with reported counters

- **GIVEN** the active model's catalog record prices every counter the session
  accumulated
- **AND** every one of those counters is provider-reported
- **WHEN** Smith renders the session cost
- **THEN** it reports one USD figure labelled exact
- **AND** names the provider and model the price came from

#### Scenario: Two models in one session

- **GIVEN** a session ran turns on `zai/glm-5.3`, then switched with `/model`
  to `google/gemini-3.8-flash` and ran more turns
- **AND** the catalog prices both and every counter is provider-reported
- **WHEN** Smith renders the exit report or `/status`
- **THEN** the GLM counters are priced at GLM's rates and the Gemini counters
  at Gemini's
- **AND** the line gives the total labelled exact, then each binding with its
  share, for example `$0.034 exact · zai/glm-5.3 $0.022 ·
  google/gemini-3.8-flash $0.012`

#### Scenario: One binding has no price

- **GIVEN** a session used a priced model and a model the catalog does not
  price
- **WHEN** Smith renders the session cost
- **THEN** the figure covers only the priced binding and is labelled estimated
- **AND** the line names the unpriced binding as `price unknown for
  <provider>/<model>`

#### Scenario: Usage restored on resume

- **GIVEN** a resumed session whose snapshot restores usage records, which
  carry no model identity
- **WHEN** Smith prices the session
- **THEN** when the project's usage log holds a record for this session whose
  totals equal the restored totals and whose counters are all attributed to
  named models, the restored counters are priced by the latest such record's
  per-binding counters; a later record that could not attribute them does not
  hide an earlier one that did
- **AND** otherwise they are priced by the binding the snapshot's activation
  manifests name, when they name exactly one
- **AND** otherwise the restored counters are unpriced, the figure is labelled
  estimated, and the line says `price unknown for earlier models`

#### Scenario: An estimated counter downgrades the label

- **GIVEN** any contributing counter is tokenizer-estimated,
  character-estimated, derived from a provider total, or unknown
- **WHEN** Smith renders the session cost
- **THEN** the figure is labelled estimated
- **AND** Smith does not present it as exact because the price reference was
  exact

#### Scenario: A model the catalog does not price

- **GIVEN** no binding that contributed counters has a catalog price entry
- **WHEN** Smith renders the exit report
- **THEN** it prints the token lines and no cost line
- **AND** does not substitute a price from another model, provider, or
  hard-coded default

#### Scenario: Cost changes no decision

- **GIVEN** a session with a known price and any accumulated cost
- **WHEN** Smith plans a request, evaluates an approval, or trims context
- **THEN** the computed cost is not an input to any of them

### Requirement: Consistent usage surfaces

Smith SHALL expose Agent Runtime's versioned usage schema through the TUI,
final non-interactive JSON result, streaming JSON events, and embedding
boundary. The TUI MUST show current-turn and session totals with cache and
provenance labels. The session total's turn count SHALL count the
conversation turns the user started, not provider requests, and SHALL be
written `1 turn` / `N turns`.

#### Scenario: Compare CLI and runtime usage

- **GIVEN** one deterministic session is run through the headless host
- **WHEN** its runtime events and final JSON output are inspected
- **THEN** both expose equivalent counters, provenance, and attribution

#### Scenario: A turn with tool calls

- **GIVEN** one user prompt that the model answered after three tool calls
- **WHEN** the user quits
- **THEN** the exit report's usage line starts `1 turn ·`
- **AND** never `turn(s)`
