## ADDED Requirements

### Requirement: Server-reported limit windows surface

Smith SHALL retain the latest normalized rate-limit snapshot per provider pool
member and present it consistently across the TUI and versioned machine
output, alongside — never mixed into — the disjoint token counters. A member
with no snapshot MUST present as unknown rather than zero or full, and a
stale snapshot MUST remain attributable to the attempt that produced it.

#### Scenario: Meter reflects the latest snapshot

- **GIVEN** the active member's last attempt reported a primary window 82%
  used
- **WHEN** usage is displayed in the TUI or emitted in machine output
- **THEN** the member shows 82% with its reset time
- **AND** token counters and their provenance labels are unchanged by the
  snapshot

#### Scenario: Unknown usage stays unknown

- **GIVEN** a pool member has never produced a rate-limit snapshot
- **WHEN** usage is displayed
- **THEN** that member's window state presents as unknown
- **AND** it is not rendered as 0% used or as exhausted
