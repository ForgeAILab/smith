## ADDED Requirements

### Requirement: Model fallback is visible and explainable

Smith SHALL project canonical fallback decisions consistently to TUI and
headless clients, including outgoing and incoming provider/model, ordered
candidate position, reason, and elapsed/configured first-response budget when
relevant. Active-model reporting SHALL reflect the actual binding while the
selected profile name stays unchanged. Configuration explanation SHALL expose
the ordered chain, timeout overrides, winning provenance, and that switching
providers resends conversation context to a configured provider.

#### Scenario: Startup timeout switch

- **GIVEN** the configured 30-second first-response budget expires
- **WHEN** the shared runtime switches to the next candidate
- **THEN** the TUI shows a notice identifying the timed-out model and the model being tried
- **AND** headless structured events carry the same identities, timeout reason, and threshold
- **AND** active-model reporting follows the new candidate without renaming the profile

#### Scenario: All candidates fail

- **WHEN** the fallback chain is exhausted
- **THEN** both surfaces report the attempted/skipped bindings and their redaction-safe reasons
- **AND** failed-attempt latency and usage are not hidden by a final summary

#### Scenario: Legacy session replay

- **GIVEN** a saved session or event predates fallback metadata
- **WHEN** a current client replays it
- **THEN** deserialization succeeds without inventing fallback decisions
- **AND** historical usage keeps its original binding attribution
