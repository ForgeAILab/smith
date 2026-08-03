## ADDED Requirements

### Requirement: Native Gemini Interactions adapter

Smith SHALL compose Google's Gemini API through an Agent Runtime native
Interactions adapter using the shared provider, transport, credential, event,
and capability contracts. It MUST NOT route the trusted Google provider through
OpenAI compatibility or a Smith-local parallel provider mechanism.

#### Scenario: Complete a native streamed tool turn

- **GIVEN** Google is selected with a valid key and catalog-backed model
- **WHEN** the native stream emits thoughts, a function call, a function result
  continuation, model output, and usage
- **THEN** the adapter maps them to the ordinary canonical runtime events in
  source order
- **AND** Smith executes tools only through its existing approval and tool loop
- **AND** TUI and headless surfaces observe equivalent behavior

#### Scenario: Cancel a native stream

- **GIVEN** a Gemini interaction is streaming
- **WHEN** the attempt is cancelled or its deadline expires
- **THEN** Smith drops the HTTP stream promptly
- **AND** emits one classified terminal outcome without a detached provider
  task

#### Scenario: Gemini rejects authentication

- **GIVEN** Google rejects an invalid, restricted, or obsolete key
- **WHEN** the request fails before semantic output
- **THEN** Smith emits the ordinary authentication classification
- **AND** no key, `x-goog-api-key` header, prompt, signature, or raw provider
  body enters logs, events, transcripts, snapshots, or diagnostics

### Requirement: Stateless exact Gemini continuation

The native adapter SHALL set `store=false` and reconstruct every request from
canonical local history. Required thought/signature and function-call state
MUST remain ordered, bounded, resumable, and opaque outside provider replay.

#### Scenario: Continue after a function call

- **GIVEN** Gemini emitted ordered thought/signature and function-call steps
- **WHEN** Smith sends the approved tool result
- **THEN** the adapter replays all required prior steps exactly and appends the
  correlated function result
- **AND** does not use `previous_interaction_id` or provider-side history

#### Scenario: Resume a saved Gemini session

- **GIVEN** a cleanly saved session contains native continuation state
- **WHEN** Smith resumes and starts the next attempt
- **THEN** the reconstructed stateless history is equivalent to the pre-save
  history
- **AND** provider signatures remain non-rendered and redaction-safe

#### Scenario: Required continuation is missing

- **GIVEN** a function-call history lacks required signed thought state
- **WHEN** the adapter prepares a continuation request
- **THEN** it fails before provider I/O with a bounded local compatibility
  error
- **AND** does not send a degraded history

### Requirement: Catalog-governed Gemini capabilities

The native adapter SHALL use the resolved model profile supplied by Smith and
MUST NOT guess provider-wide capabilities or limits. Catalog absence or invalid
metadata MUST fail before credential resolution or provider I/O.

#### Scenario: Apply catalog reasoning effort

- **GIVEN** the frozen model record advertises supported thinking levels
- **WHEN** the user selects one supported effort
- **THEN** the adapter sends the corresponding native `thinking_level`
- **AND** unsupported efforts fail during local preflight

#### Scenario: Model metadata is incomplete

- **GIVEN** the selected Google model lacks valid limits or tool capability in
  the frozen catalog and no explicit override supplies them
- **WHEN** Smith prepares the runtime
- **THEN** construction fails before credential lookup or Google I/O
- **AND** Smith does not infer values from the model name
