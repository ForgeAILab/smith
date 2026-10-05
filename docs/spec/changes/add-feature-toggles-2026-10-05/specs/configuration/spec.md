## ADDED Requirements

### Requirement: Built-in feature registry

Smith SHALL keep one registry of switchable built-in features. Each entry MUST
name a stable id, a one-line description, the existing configuration key that
controls it, its built-in default, and whether a change applies at the next
safe boundary or only to new sessions. An entry MUST NOT introduce a second key
for behavior that already has one, and the effective value MUST come from the
ordinary layered resolution with its provenance.

#### Scenario: Feature value is explained

- **GIVEN** `cache.idle_compaction = false` in the user configuration
- **WHEN** the feature registry is resolved
- **THEN** `idle-compaction` is off
- **AND** its provenance names the user file and the `cache.idle_compaction` key

#### Scenario: Registry key must exist

- **GIVEN** a registry entry whose key is not part of the configuration model
- **WHEN** the architecture tests run
- **THEN** they fail naming the entry
