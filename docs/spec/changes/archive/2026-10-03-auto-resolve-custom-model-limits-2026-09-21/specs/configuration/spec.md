## MODIFIED Requirements

### Requirement: Trusted setup descriptors and model limits

Provider choices exposed by setup SHALL map to Smith-owned descriptors whose
adapter kind is present in the pinned Agent Runtime. A selected model MUST
resolve its context window from one explicit input, a provider-advertised
OpenAI-compatible `/models` listing read during interactive setup, or a
versioned trusted catalog; setup MUST NOT finish for a model whose total
context remains unknown, and MUST NOT route an unavailable adapter through a
different provider family. Automatically resolved values are prefilled for
review with their source named, never silently committed. When a source does
not publish narrower ceilings, or the context was entered manually, maximum
input and output are derived without additional prompts (input = context,
output = the automatic percentage rule).

#### Scenario: Catalog-backed model is selected

- **GIVEN** a setup descriptor offers a model with complete trusted catalog
  metadata
- **WHEN** the user selects it
- **THEN** review shows the model and catalog provenance of its context,
  maximum-input, and maximum-output limits
- **AND** full runtime preflight resolves the same immutable profile

#### Scenario: GLM quick start is selected

- **GIVEN** the pinned runtime includes the OpenAI-compatible adapter
- **WHEN** the user selects the GLM quick start
- **THEN** setup proposes provider `zai` at
  `https://api.z.ai/api/coding/paas/v4` and model `glm-4.7`
- **AND** the proposed model profile declares 200000 context tokens, 196000
  maximum input tokens, and 131072 maximum output tokens
- **AND** the selected profile requests and reserves at most 8192 output tokens
- **AND** the proposed provider response policy treats a non-redacted
  reasoning-only completion as visible assistant text without disabling GLM
  thinking

#### Scenario: The endpoint advertises limits for the entered model

- **GIVEN** the user enters a model ID in the add-provider or add-model flow
- **AND** the endpoint's `GET /models` listing carries limit fields for that
  ID
- **WHEN** the listing resolves within the bounded probe
- **THEN** setup prefills the advertised limits and names the endpoint listing
  as their source in review
- **AND** no inference request is sent and nothing is written before the
  reviewed commit

#### Scenario: A same-name trusted catalog entry pre-fills limits

- **GIVEN** the endpoint publishes no limits but the entered model ID matches
  a trusted catalog entry exactly, case-insensitively, by final path segment,
  or by a final segment whose `.`, `_`, and `-` separators differ
- **WHEN** resolution runs
- **THEN** setup prefills that entry's limits and names the catalog match in
  review
- **AND** the user can still edit the context window before committing

#### Scenario: A gateway rewrites a version separator

- **GIVEN** a gateway advertises model `claude-fable-5-1` without limit fields
- **AND** the trusted catalog contains `anthropic/claude-fable-5.1` with
  complete limits
- **WHEN** setup falls back from the endpoint listing to the catalog
- **THEN** the two IDs match without prompting for the context window
- **AND** setup does not drop the `1` version component or match
  `claude-fable-5`

#### Scenario: Derived ceilings complete a partial resolution

- **GIVEN** resolution produced a context window without published input or
  output ceilings
- **WHEN** the values are prefilled
- **THEN** maximum input defaults to the context window and maximum output to
  the automatic percentage rule, labeled as derived in review

#### Scenario: Unknown custom model resolves nothing

- **GIVEN** the custom model is absent from every trusted catalog source and
  the endpoint advertises nothing for it
- **WHEN** the user tries to continue from model setup
- **THEN** Smith requires only the explicit context window and derives the
  maximum-input and maximum-output limits without showing separate fields
- **AND** it cannot finish setup while any enforceable limit is absent or
  invalid

#### Scenario: Descriptor names an unavailable adapter

- **GIVEN** a setup descriptor's adapter is not present in the pinned runtime
- **WHEN** Smith builds the provider choices
- **THEN** that descriptor is not selectable
- **AND** Smith does not silently substitute an OpenAI-compatible or other
  adapter
