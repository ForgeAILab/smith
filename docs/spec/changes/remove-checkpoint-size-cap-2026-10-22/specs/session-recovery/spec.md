## MODIFIED Requirements

### Requirement: Exact mid-turn state uses protected checkpoints

Smith SHALL implement Agent Runtime's checkpoint contract with authenticated
encryption, integrity/version checks, atomic or transactional commits, and a
user-scoped protection key. It MUST NOT silently store exact pending actions,
answers, or tool state as plaintext. Protected checkpoint payloads and
envelopes SHALL have no Smith-imposed byte-size ceiling on save or load.
Existing envelope identity bounds, authentication, schema checks, and
checkpoint transition validation SHALL remain enforced.

#### Scenario: Protection key is unavailable
- **GIVEN** Smith cannot initialize protected checkpoint storage
- **WHEN** a persistent session starts
- **THEN** Smith reports that mid-turn crash recovery is unavailable
- **AND** does not claim pending approvals or questions will survive restart

#### Scenario: Image history exceeds the former checkpoint size limit
- **GIVEN** an otherwise valid checkpoint containing image history serializes to more than 32 MiB
- **WHEN** Smith saves it and a new store instance reopens it
- **THEN** the exact checkpoint is restored without a payload-size rejection
- **AND** authenticated encryption and owner-only atomic storage remain in use
- **AND** saving the same checkpoint again is idempotent
