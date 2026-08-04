## ADDED Requirements

### Requirement: Parent resume restores the child coordinator without execution

Smith SHALL load and validate a resumed parent's durable child catalog,
reconcile lifecycle leases and checkpoint watermarks, and wire those records
into the delegation coordinator before accepting child operations. Recovery
MUST be lazy and MUST NOT construct a provider request, invoke a tool, or
silently restart child work merely to list, inspect, or render the session.

#### Scenario: Parent resumes with two children

- **GIVEN** a saved parent owns one idle child and one child interrupted by
  process exit
- **WHEN** Smith resumes the parent and renders `/agent`
- **THEN** both stable child IDs and accurate states are available
- **AND** provider and tool invocation counts remain zero until an explicit
  operation is submitted

### Requirement: Child recovery is exact, protected, and no-prompt compatible

Smith SHALL store exact child turn state through the authenticated protected
checkpoint path and SHALL reconcile it exactly once with the durable child
record. When the configured checkpoint key comes from the existing inline or
environment source, child persistence and resume MUST NOT open or query an
operating-system credential service and MUST NOT fall back to plaintext.

#### Scenario: Resume under an environment checkpoint key

- **GIVEN** root and child checkpoints were protected by the configured
  environment key
- **WHEN** Smith restarts and explicitly resumes the child
- **THEN** it authenticates and continues the exact checkpoint without an OS
  credential prompt
- **AND** repeats no committed provider, approval, interaction, or tool effect

#### Scenario: Child checkpoint fails authentication

- **GIVEN** the child record references a checkpoint that does not authenticate
- **WHEN** Smith recovers the parent
- **THEN** the child is shown blocked/non-resumable with a redacted integrity
  reason
- **AND** Smith does not replace, delete, or execute it

### Requirement: Legacy ephemeral children are never fabricated as durable

Smith SHALL preserve presentation evidence for historical journal-only child
runs, but MUST label them legacy ephemeral when no protected child record and
session state exist. It MUST NOT reconstruct raw history from redacted events,
offer resume, or bind the old child ID to a new session.

#### Scenario: Resume an older Smith session

- **GIVEN** the parent journal contains a completed or unresolved child from a
  schema predating durable child records
- **WHEN** Smith resumes that parent
- **THEN** timeline inspection retains the bounded historical child evidence
- **AND** follow-up/resume reports that the legacy child is unavailable
- **AND** no provider request or replacement spawn occurs
