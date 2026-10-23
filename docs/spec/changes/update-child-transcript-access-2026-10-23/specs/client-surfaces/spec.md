## ADDED Requirements

### Requirement: Child inspection renders the retained conversation

The selected child view SHALL use the main conversation's rendering and
visibility rules for the delegated task, assistant messages, permitted
provider-exposed reasoning, tool calls/results, and final reply. Retained
history SHALL remain readable through scrolling or paging; a short activity
label or bounded lifecycle log SHALL NOT substitute for available conversation
content. Existing composer focus and root navigation behavior SHALL remain.

#### Scenario: Inspect a child after it has already performed work

- **GIVEN** a child committed assistant text, a tool call, and its result before
  the TUI subscribed to the child's stream
- **WHEN** the user selects that child
- **THEN** its delegated task and committed conversation are hydrated from
  canonical history and shown in order
- **AND** subsequent live events join that view without gaps or duplicate rows

#### Scenario: Read older work beyond the client memory window

- **GIVEN** a child's retained conversation exceeds the in-memory row limit
- **WHEN** the user scrolls or pages back
- **THEN** earlier retained content remains accessible from canonical storage
- **AND** large tool results have the same access behavior as in the main view

#### Scenario: Inspect a recovered child

- **GIVEN** the parent restarted and a retained child has no live handle
- **WHEN** the user opens that child's view
- **THEN** its retained transcript is displayed without executing the child
- **AND** unavailable historical segments are labeled explicitly

#### Scenario: Return to the main conversation

- **GIVEN** the user has read a child's conversation
- **WHEN** the user dismisses child inspection
- **THEN** the root transcript, scroll position, and composer draft are restored

### Requirement: Child activity describes observations without inventing work

The child view SHALL distinguish observed provider waiting, provider-exposed
reasoning, tool execution, and terminal state using available canonical
evidence, and SHALL expose the age of the last observed progress. It MUST NOT
claim that a silent child is thinking, fetching, or stalled solely because its
lifecycle remains running. Provider-hidden content MUST NOT be fabricated.

#### Scenario: The provider exposes search activity without prose

- **GIVEN** the child stream reports hosted search but no assistant text or
  permitted reasoning
- **WHEN** the child view renders
- **THEN** it shows the observed search activity and available prior transcript
- **AND** it does not invent an assistant explanation or hidden reasoning

#### Scenario: A provider request has produced no recent progress

- **GIVEN** the latest observed activity is a provider request
- **WHEN** time passes without additional visible events
- **THEN** the view identifies the last observed activity and its progress age
- **AND** it does not keep presenting a previous completed tool as active
