## MODIFIED Requirements

### Requirement: Labelled cost calculation

Smith SHALL calculate cost only from a versioned price reference and compatible
usage counters. Calculated values MUST be labelled exact, estimated, or unknown
according to their inputs. The catalog snapshot's per-model price entry is one
such reference, and it MUST carry the same revision and retrieval provenance as
every other catalog field. Cost MUST remain presentation only: it MUST NOT
enter routing, approval, context, or budget decisions, and MUST NOT reach the
model.

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

#### Scenario: An estimated counter downgrades the label

- **GIVEN** any contributing counter is tokenizer-estimated,
  character-estimated, derived from a provider total, or unknown
- **WHEN** Smith renders the session cost
- **THEN** the figure is labelled estimated
- **AND** Smith does not present it as exact because the price reference was
  exact

#### Scenario: A model the catalog does not price

- **GIVEN** the active model's catalog record carries no price entry
- **WHEN** Smith renders the exit report
- **THEN** it prints the token lines and no cost line
- **AND** does not substitute a price from another model, provider, or
  hard-coded default

#### Scenario: Cost changes no decision

- **GIVEN** a session with a known price and any accumulated cost
- **WHEN** Smith plans a request, evaluates an approval, or trims context
- **THEN** the computed cost is not an input to any of them

## ADDED Requirements

### Requirement: Delegated usage is accounted separately

Smith SHALL accumulate per-counter usage reported by delegated children from
the child event streams the host subscribes to, and SHALL keep those counters
distinguishable from the root session's own at every surface that reports them.
It MUST report the number of children that contributed usage, and MUST NOT
present delegated tokens as root tokens or omit them from a session total.

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
- **AND** the delegated totals are priced by the same per-counter reference the
  root totals are
