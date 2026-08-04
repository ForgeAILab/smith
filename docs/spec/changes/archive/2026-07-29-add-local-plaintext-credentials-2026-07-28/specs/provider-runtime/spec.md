## MODIFIED Requirements

### Requirement: Host-bound credential resolution

Provider configuration MUST contain either a credential reference or an inline
`api_key` accepted only from owner-only user configuration. Smith SHALL
construct the shared provider with a redaction-safe `Secret` only after
endpoint and profile validation succeeds.

#### Scenario: Configure an OpenAI-compatible endpoint with a reference

- **GIVEN** a profile supplies a base URL, model, credential reference, and
  enforceable model limits
- **WHEN** Smith constructs the shared OpenAI-compatible provider
- **THEN** authorization is attached only at the transport boundary
- **AND** the raw secret is absent from Smith runtime events, snapshots, tool
  arguments, and logs

#### Scenario: Configure an inline user key

- **GIVEN** owner-only user config supplies a provider `api_key`
- **WHEN** Smith constructs the provider
- **THEN** it wraps and registers the value with the persistence redactor
  before provider construction
- **AND** it performs no Keychain, Secret Service, or environment lookup
- **AND** every runtime and persistence surface remains secret-free
