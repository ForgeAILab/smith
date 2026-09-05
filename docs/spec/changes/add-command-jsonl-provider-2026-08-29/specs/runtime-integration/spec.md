## ADDED Requirements

### Requirement: One composition path for command and native providers

Smith SHALL construct `command-jsonl` through the same
`smith-runtime::factory` boundary and shared `Provider` contract used by native
HTTP providers. Provider transport differences MUST NOT create a second
context, tool/MCP, approval, retry, event, persistence, child, TUI, or headless
composition path.

#### Scenario: Compare command-provider surfaces

- **GIVEN** identical resolved policy and a deterministic command-provider
  fixture
- **WHEN** the TUI and `smith -p` execute the same turn
- **THEN** their canonical messages, attempts, tool results, usage, and
  terminal lifecycle are equivalent
- **AND** only declared presentation metadata differs

#### Scenario: Switch between native and command providers

- **GIVEN** an idle persisted session used a native provider on its previous
  turn
- **WHEN** the user selects a compatible command provider at the existing safe
  turn boundary
- **THEN** Smith rebuilds the same session through the one factory with the new
  `Arc<dyn Provider>`
- **AND** canonical history and Smith policy remain local
- **AND** prior provider cache state does not transfer

### Requirement: Compatible command-provider dependency gate

Smith SHALL enable command-provider composition only from an exact immutable
Agent Runtime revision containing the approved bounded process framework.
Smith's factory integration and Agent Runtime's Smith consumer-conformance
suite MUST both pass before the adapter becomes available.

#### Scenario: Pinned runtime lacks the feature

- **GIVEN** Smith's exact Agent Runtime revision does not expose the approved
  `command-provider` facade feature and contracts
- **WHEN** a build or configuration requests `command-jsonl`
- **THEN** the adapter remains unavailable rather than falling back to a Smith-
  local process implementation
- **AND** existing native providers remain buildable and unchanged

#### Scenario: Compatible runtime revision is adopted

- **GIVEN** the candidate runtime includes the approved command framework
- **WHEN** Smith enables the feature and runs both compatibility gates
- **THEN** direct argv, bounds, terminal validation, cancellation, and process-
  tree semantics match the reviewed upstream contract
- **AND** the exact revision is recorded in Smith's manifest and lockfile
