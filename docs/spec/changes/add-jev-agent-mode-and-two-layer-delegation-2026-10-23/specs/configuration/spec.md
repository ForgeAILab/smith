## MODIFIED Requirements

### Requirement: Unified agent profile declarations

Smith SHALL use named profiles as the single declarative agent-preset type for
main-agent selection and explicit bounded child creation. A profile MAY contain
bounded description and instructions, an authority-narrowing posture,
main/child availability, provider/model preferences, an inherited delegation
availability switch, and existing profile policy fields; none of those fields
may grant trust, credentials, permissions, approval, workspace scope, or host
capabilities. Omitted delegation availability MUST preserve the existing
enabled behavior for main profiles.

Profiles MAY also declare inherited direct_interaction with a default of false.
Its effective value and delegation availability SHALL retain field-level source
provenance and participate in the profile policy revision. Delegation SHALL be
intersected with placement/depth: eligible Layer-1 profiles may delegate, while
Layer-2 profiles cannot. Direct interaction SHALL require a host-approved Layer-1
placement and broker; neither setting grants permissions or a wider workspace.

#### Scenario: Use one profile on the main agent
- **GIVEN** a valid profile is available for `main`
- **WHEN** startup, `--profile`, or `/profile` selects it
- **THEN** Smith resolves its instructions, posture, provider/model, and limits
  through the normal typed profile-precedence layer
- **AND** applies the effective profile atomically at a safe runtime boundary

#### Scenario: Expose one profile to both placements
- **GIVEN** a valid profile declares `use = ["main", "child"]`
- **WHEN** Smith builds its local profile inventory
- **THEN** the same named declaration is eligible for main selection and
  explicit child invocation
- **AND** each placement independently intersects the profile with host policy

#### Scenario: Existing profile omits availability
- **GIVEN** a pre-change profile contains only existing runtime fields
- **WHEN** the transition release resolves it
- **THEN** Smith treats it as a main-enabled build profile with delegation
  enabled
- **AND** does not expose it for child creation without an explicit child use

#### Scenario: Main profile disables delegation

- **GIVEN** a main-enabled profile declares `delegation = false`
- **WHEN** Smith resolves the effective profile through inheritance and
  precedence
- **THEN** provenance reports delegation as disabled from the winning source
- **AND** the field cannot add a child profile, permission, or authority

#### Scenario: Inherited interaction preference is source-explainable

- **GIVEN** a child-enabled lead extends a profile with direct_interaction=true and overrides delegation=false
- **WHEN** Smith resolves the effective profile
- **THEN** both values retain their winning sources and the lead remains non-delegating
- **AND** direct interaction still requires host placement and broker policy

## ADDED Requirements

### Requirement: Optional Jev advisory triage configuration

Smith SHALL resolve an optional top-level jev section with enabled, endpoint,
timeout_ms, actionability_threshold, profile_confidence_threshold,
clarifier_profile, and optional bearer_credential_ref through typed explainable
configuration. Defaults SHALL be disabled, the loopback SystemOne endpoint,
1500 ms, 0.5 actionability threshold, 0.8 normalized profile confidence threshold,
and clarifier profile name clarifier. Omission MUST preserve ordinary admission
without sidecar I/O. A named clarifier MUST be an eligible preflighted child
profile when enabled.

Timeout MUST be in 1..=10,000 ms and both thresholds finite in [0,1]. Invalid
static values, unknown fields, or unusable configured profile references SHALL
produce source-aware preflight diagnostics. A transient unavailable endpoint
SHALL instead follow the runtime fail-open contract.

#### Scenario: Existing configuration contains no jev table

- **GIVEN** a pre-change configuration has no jev settings
- **WHEN** Smith resolves and admits a user turn
- **THEN** ordinary behavior remains and no Jev request is made

#### Scenario: Explain an overridden threshold

- **GIVEN** a user-owned layer overrides a valid actionability threshold
- **WHEN** config explain inspects the effective jev settings
- **THEN** the value and winning source are reported without resolving a bearer secret into output
- **AND** the value is not described as a calibrated probability of correctness

#### Scenario: Reject invalid numeric settings

- **GIVEN** timeout_ms is zero or a threshold is non-finite or outside [0,1]
- **WHEN** preflight resolves the run
- **THEN** it reports the invalid path/source before provider or sidecar I/O

### Requirement: Jev endpoint and credentials do not self-authorize disclosure

Smith MUST require user-owned endpoint and credential authorization for Jev.
Enabling triage, selecting a bearer credential, and permitting non-loopback prompt
disclosure SHALL require user-owned effective configuration and host network
policy. Repository configuration MUST NOT grant that authority. Credential
references SHALL use the existing protected user credential mechanism and MUST
NOT place plaintext secrets in config, diagnostics, prompts, or journals. The
client MUST disable redirects and preserve the configured network boundary.

#### Scenario: Repository attempts to redirect triage remotely

- **GIVEN** a trusted user configuration allows only local advisory triage
- **WHEN** repository settings request an unrelated remote endpoint or credential
- **THEN** Smith refuses the authority-expanding override with provenance
- **AND** no user prompt or secret is sent to that endpoint

#### Scenario: Endpoint redirects to another host

- **GIVEN** an authorized local endpoint responds with a redirect
- **WHEN** the triage client handles the response
- **THEN** it does not follow the redirect
- **AND** ordinary root handling remains available without disclosing the prompt elsewhere

### Requirement: Profile routing descriptions are bounded advisory metadata

Jev profile choices SHALL be built from effective child-enabled, preflighted,
host-eligible profile identities with bounded one-line descriptions. The router
MUST NOT expose arbitrary model-generated profile names as executable routes or
interpret descriptions as new instructions, permissions, credentials, or tool
availability. Missing or ineligible choices SHALL fall back to ordinary handling.

#### Scenario: Model chooses an unregistered profile

- **GIVEN** the response names a profile outside the exact request roster
- **WHEN** Smith validates the response
- **THEN** it declines that route and takes ordinary handling
- **AND** no provider, capability, or child is created from the unknown name

#### Scenario: Profile text requests broader powers

- **GIVEN** profile metadata contains an authority-expanding instruction
- **WHEN** it is supplied as bounded advisory criteria
- **THEN** host-resolved policy remains the upper bound
- **AND** the metadata cannot change workspace, approval, tool, or credential authority
