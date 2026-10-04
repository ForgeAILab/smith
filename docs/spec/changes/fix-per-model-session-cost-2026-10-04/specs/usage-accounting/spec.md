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
- **THEN** the restored counters are priced by the binding the snapshot's
  activation manifests name, when they name exactly one
- **AND** when they name several, the restored counters are unpriced, the
  figure is labelled estimated, and the line says `price unknown for earlier
  models`

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

### Requirement: Delegated usage is accounted separately

Smith SHALL accumulate per-counter usage reported by delegated children from
the child event streams the host subscribes to, and SHALL keep those counters
distinguishable from the root session's own at every surface that reports them.
It MUST report the number of children that contributed usage, and MUST NOT
present delegated tokens as root tokens or omit them from a session total.
Delegated counters SHALL be kept per child binding, resolved from the child's
profile when it is spawned.

#### Scenario: Four children report usage

- **GIVEN** a session spawns four children and each reports provider usage
- **WHEN** the user quits
- **THEN** the exit report states a merged total across root and children
- **AND** an indented root line and an indented agents line break that total
  down
- **AND** the agents line names how many children contributed

#### Scenario: A session with no delegation

- **GIVEN** a session spawned no children
- **WHEN** the user quits
- **THEN** the report shows the root counters with no agents line and no
  breakdown
- **AND** the merged total equals the root total

#### Scenario: A dormant child reported nothing in this process

- **GIVEN** a resumed session recovers a durable child whose work happened in
  an earlier process
- **WHEN** Smith reports delegated usage
- **THEN** it counts only what this process observed
- **AND** does not invent counters for the recovered child or count it as a
  contributor

#### Scenario: Delegated counters keep their categories

- **GIVEN** a child reports cache-read input, uncached input, and output
- **WHEN** Smith accumulates it into the delegated totals
- **THEN** each counter lands in its own category
- **AND** the delegated counters are priced by the per-counter reference of
  the child's own binding

#### Scenario: A child runs another model

- **GIVEN** the root runs `zai/glm-5.3` and a child spawned with a profile
  bound to `chatgpt/gpt-6.1-sol`
- **WHEN** Smith prices the session
- **THEN** the child's counters are priced at the ChatGPT model's rates, or
  left unpriced and the figure labelled estimated if the catalog has none
- **AND** the cost line names both bindings

#### Scenario: A child's binding is unknown

- **GIVEN** a child whose profile cannot be resolved to a binding reports
  usage
- **WHEN** Smith prices the session
- **THEN** that child's counters are unpriced and the figure is labelled
  estimated

## ADDED Requirements

### Requirement: Usage log keeps per-binding counters

The usage log record SHALL carry each contributing binding's counters
separately (schema version 5), and SHALL keep its existing top-level provider
and model fields naming the last root binding. Readers MUST accept records of
versions 1 through 5.

#### Scenario: A two-model session is logged

- **GIVEN** a session that ran GLM and then Gemini
- **WHEN** Smith appends its usage record on exit
- **THEN** the record lists GLM's and Gemini's counters under their own
  bindings
- **AND** the top-level model is `gemini-3.8-flash`

#### Scenario: An older record is read

- **GIVEN** a version-4 record with no per-binding counters
- **WHEN** the log is read
- **THEN** it parses, with its totals attributed to its top-level binding
