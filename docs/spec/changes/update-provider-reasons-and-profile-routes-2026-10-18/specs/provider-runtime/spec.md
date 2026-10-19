## MODIFIED Requirements

### Requirement: Smith-owned production transport

Smith SHALL supply the concrete streaming HTTP transport injected into shared
provider adapters. The transport MUST enforce cancellation, deadlines, bounded
request/response handling, TLS validation, status/error classification, and
redaction of headers, bodies, credentials, and prompt content from debug and
diagnostic surfaces. A rejected request SHALL carry the provider's stated
reason, `status` (or a string `code`) and `message`, read from the documented
error object of the body or of the first SSE `data:` line that holds one, and
MUST drop the whole reason when it contains any credential value the request
sent, including the bare token of a `Bearer` header.

#### Scenario: Provider stream is cancelled

- **GIVEN** the shared provider adapter is waiting on Smith's HTTP byte stream
- **WHEN** the session cancellation token fires
- **THEN** the transport request and byte stream are dropped promptly
- **AND** the shared runtime emits a structured cancelled attempt

#### Scenario: Error body contains a credential

- **GIVEN** a provider error body reflects an authorization value
- **WHEN** Smith maps the transport failure
- **THEN** the value is absent from visible and persisted diagnostics
- **AND** only bounded redaction-safe metadata reaches the shared provider error

#### Scenario: Gemini rejects a streamed request

- **GIVEN** Gemini answers with `event: error` and a `data:` line whose error
  has code `invalid_request` and message "Request contains an invalid
  argument."
- **WHEN** the user, a headless result, or a parent agent reads the failure
- **THEN** it reads `Provider: Gemini provider rejected the request:
  invalid_request: Request contains an invalid argument.`
