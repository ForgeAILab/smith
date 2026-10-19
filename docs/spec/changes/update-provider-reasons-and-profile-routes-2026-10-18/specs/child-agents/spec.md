## MODIFIED Requirements

### Requirement: Durable child profile identity

Smith SHALL include the effective profile name, revision, placement, provider/
model selection, and authority posture in durable child policy compatibility
evidence. Follow-up and resume MUST retain the same profile identity and MUST
fail closed when exact compatible composition is unavailable. A changed or
missing child profile MUST NOT stop the parent session from resuming: Smith
SHALL resolve a recorded route key whose revision is gone to the profile's
current route by name, and SHALL give a child it cannot route a policy
fingerprint that matches no recorded one, so that only the child is reported
incompatible.

#### Scenario: Resume with an unchanged profile
- **GIVEN** an interrupted child has a durable checkpoint and its effective
  profile revision remains available
- **WHEN** the user explicitly resumes that child
- **THEN** Smith continues the unfinished turn with the same profile identity
- **AND** does not consume a new task slot or repeat committed work

#### Scenario: Profile changed before resume
- **GIVEN** the selected profile's effective instructions, model, or posture
  changed after checkpoint creation
- **WHEN** the user requests resume
- **THEN** Smith reports an incompatible policy fingerprint
- **AND** does not silently spawn a replacement or run under mixed revisions

#### Scenario: Parent resumes after its child profile changed or was removed
- **GIVEN** a parent session ran a child under profile `audit`
- **AND** `audit`'s instructions then changed, or it is no longer a child
  profile
- **WHEN** the user resumes the parent
- **THEN** the parent resumes
- **AND** the child is listed with an incompatibility instead of failing the
  resume
