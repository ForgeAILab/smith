## ADDED Requirements

### Requirement: Completed summary bodies remain user-inspectable

The resume capsule SHALL retain the completed semantic summary body through
the existing protected session-owned artifact with its provenance, and Smith
SHALL expose one bounded verification-checked read of that body for
user-facing rendering. The view SHALL return only verified summary text and
provenance; it SHALL NOT expose raw credentials, unredacted private prompt
bodies, exact protected interaction content, or provider cache contents, and
summary text SHALL remain non-authoritative against exact state.

#### Scenario: Inspecting a stored idle-compaction summary

- **GIVEN** the capsule records a completed summary with a valid artifact
- **WHEN** the bounded view is requested
- **THEN** verified summary text and provenance are returned
- **AND** the capsule's exact state is unchanged

#### Scenario: Artifact reference is stale or corrupt

- **GIVEN** the summary artifact fails identity or bound checks
- **WHEN** the bounded view is requested
- **THEN** the view reports the summary as missing
- **AND** no unverified text is returned or rendered
