## ADDED Requirements

### Requirement: Shrinking the context window compacts before the next request

Smith SHALL compact the transcript, or report that it cannot, before the
next provider request whenever a session switches to a window whose input
budget is smaller than the current transcript. It MUST NOT send a request
that the planner already knows exceeds the new window.

#### Scenario: Switch from 872k to 272k with a large transcript

- **GIVEN** a transcript of 400,000 tokens under the `872k` window
- **WHEN** the user selects `272k` and sends the next message
- **THEN** compaction runs before the provider request
- **AND** the request fits the `272k` input budget
