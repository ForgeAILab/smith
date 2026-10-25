## MODIFIED Requirements

### Requirement: Source-explainable child-agent wait policy

The resolved child-agent wait policy SHALL expose
`profiles.<name>.child_agents.wait_default_timeout_ms` and
`profiles.<name>.child_agents.wait_max_timeout_ms`. The accepted ranges SHALL
be `0..=300_000` and `1..=300_000` milliseconds respectively, with built-in
defaults of 300,000 milliseconds for both values. The default SHALL NOT exceed
the maximum. A zero default is an immediate status check when explicitly
configured; omitted `agent.wait.timeout_ms` uses the resolved default wait
timer duration, and a requested value above the maximum is clamped to it.

These values bound only the wait timer after which an idle parent is woken.
They MUST NOT hold a parent tool call open, and MUST NOT grant a child
lifetime, token, cancellation, or provider limit; timer expiry MUST leave the
child running.

#### Scenario: Default wait policy is five minutes

- **GIVEN** a profile omits both child-agent wait settings
- **WHEN** Smith resolves the profile
- **THEN** the default and maximum are each 300,000 milliseconds
- **AND** the values retain built-in provenance

#### Scenario: Repository narrows the wait timer

- **GIVEN** a profile sets `wait_max_timeout_ms = 10000` and a compatible
  default
- **WHEN** Smith resolves the profile
- **THEN** the configured value bounds the wait timer only
- **AND** it cannot stop or expire a child when the timer fires

### Requirement: Layered adaptive cache lifecycle configuration

Smith SHALL expose the following source-explainable cache lifecycle policy:

```toml
[profiles.<name>.context.cache]
maintenance = "adaptive"             # off | observe | adaptive
inactivity_limit_ms = 3600000
max_hold_while_child_ms = 3600000
max_maintenance_calls = 1
max_maintenance_input_tokens = 0      # 0 = exact resolved model/plan budget
max_maintenance_output_tokens = 256
maintenance_deadline_ms = 30000
keepalive_margin_ms = 120000
keepalive_jitter_percent = 10
handoff_checkpoint = true
idle_compaction = true
resume_capsule = true

[profiles.<name>.child_agents]
wait_default_timeout_ms = 300000
wait_max_timeout_ms = 300000
```

`off` SHALL disable synthetic cache maintenance while preserving ordinary
correctness and provider caching. `observe` SHALL record plans, leases, and
provider observations but send no synthetic request. `adaptive` SHALL permit
only actions that also satisfy provider/model capability, adapter conformance,
host authority, lifecycle, continuation-source, and budget policy.

Invalid types, enum values, ranges, or unsupported action combinations MUST
fail closed during configuration resolution. The accepted ranges are
`inactivity_limit_ms: 1_000..=86_400_000` (zero invalid),
`max_hold_while_child_ms: 0..=86_400_000` (zero disables child hold),
`max_maintenance_calls: 0..=8` (zero disables synthetic maintenance),
`max_maintenance_input_tokens: 0..=resolved_model_input_limit` (zero uses the
exact resolved plan/model budget), `max_maintenance_output_tokens: 1..=4096`,
`maintenance_deadline_ms: 1..=120_000`, `keepalive_margin_ms:
0..=inactivity_limit_ms` (zero means no early margin), and
`keepalive_jitter_percent: 0..=50` (zero is deterministic). The wait default
is `0..=300_000` milliseconds and the wait maximum is `1..=300_000`; their
built-in defaults are 300,000 milliseconds, and the default MUST NOT exceed the
maximum. `agent.wait.timeout_ms = 0` is an immediate status check that arms
no timer; a requested timeout above the resolved maximum is clamped to it and
reported as clamped. A wait on a running child returns immediately and arms a
single-use wait timer; the child keeps running in the background for automatic
terminal delivery.
An unsupported provider SHALL remain usable for ordinary turns with cache
status `unsupported`; it MUST NOT be made invalid solely because the requested
maintenance mode was `adaptive`.

#### Scenario: Unsupported provider with adaptive configuration

- **GIVEN** resolved configuration requests `maintenance = "adaptive"`
- **AND** the selected model declares caching unsupported
- **WHEN** Smith resolves the runtime
- **THEN** the ordinary provider runtime remains valid
- **AND** cache status is `unsupported`
- **AND** no synthetic maintenance is scheduled

#### Scenario: Observe mode is selected

- **GIVEN** configuration resolves `maintenance = "observe"`
- **WHEN** a parked parent crosses a known retention boundary
- **THEN** Smith may emit an observation-only scheduling projection
- **AND** sends no keepalive or handoff checkpoint

#### Scenario: Invalid jitter is declared

- **GIVEN** a configuration layer declares a jitter percentage outside the
  accepted bounded range
- **WHEN** Smith resolves the cache policy
- **THEN** preflight fails with the exact key and source
- **AND** no provider or cache request is constructed
